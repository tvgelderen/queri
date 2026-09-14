//! Real route: the PG DNS names exist only on the bastion's Docker network.
use anyhow::{Context, Result, ensure};
use queri::{
    database::{Session, runtime},
    model::*,
    ssh::{Connector, HostDecision, Prompt},
    storage::Store,
};
use russh::keys::{self, HashAlg};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use uuid::Uuid;

fn env(name: &str) -> String {
    std::env::var(name)
        .expect("Run scripts/ssh-fixture/run.sh cargo test --test ssh -- --ignored --nocapture")
}
fn profile() -> ConnectionProfile {
    ConnectionProfile {
        id: Uuid::new_v4(),
        name: "SSH fixture".into(),
        environment: None,
        policy: ExecutionPolicy::Normal,
        notes: String::new(),
        color: String::new(),
        remember_password: false,
        ssh: Some(SshConfig {
            host: env("QUERI_SSH_HOST"),
            port: env("QUERI_SSH_PORT").parse().unwrap(),
            user: env("QUERI_SSH_USER"),
            auth: SshAuth::PrivateKey {
                path: env("QUERI_SSH_KEY").into(),
            },
            remember_passphrase: false,
        }),
        config: ConnectionConfig::Postgres {
            host: env("QUERI_SSH_DATABASE_HOST"),
            port: 5432,
            database: "fixture".into(),
            user: "postgres".into(),
            tls: TlsMode::Disable,
            timeout_seconds: 5,
        },
    }
}
async fn prompt(
    prompts: &mut mpsc::UnboundedReceiver<Prompt>,
    decision: HostDecision,
    changed: bool,
) -> Result<()> {
    match tokio::time::timeout(Duration::from_secs(20), prompts.recv())
        .await?
        .context("Missing host prompt")?
    {
        Prompt::Host {
            endpoint,
            algorithm,
            fingerprint,
            previous,
            reply,
            ..
        } => {
            assert_eq!(endpoint, profile().ssh.unwrap().endpoint());
            assert_eq!(algorithm, "ssh-ed25519");
            assert_eq!(
                fingerprint,
                keys::load_public_key(env("QUERI_SSH_HOST_PUBLIC_KEY"))?
                    .fingerprint(HashAlg::Sha256)
                    .to_string()
            );
            assert_eq!(previous.is_some(), changed);
            reply
                .send(decision)
                .map_err(|_| anyhow::anyhow!("Prompt abandoned"))?;
        }
        _ => anyhow::bail!("Unexpected credential prompt"),
    }
    Ok(())
}
fn connect(
    connector: &Arc<Connector>,
    profile: ConnectionProfile,
) -> tokio::task::JoinHandle<Result<Arc<Session>>> {
    let connector = connector.clone();
    tokio::spawn(async move {
        connector
            .connect(profile, env("QUERI_SSH_DATABASE_PASSWORD"))
            .await
    })
}
async fn scalar(session: &Arc<Session>, sql: &str) -> Result<String> {
    Ok(session.query(sql.into()).await?.sets[0].rows[0][0].text())
}

async fn restart_bastion() -> Result<()> {
    ensure!(
        std::process::Command::new("docker")
            .args(["restart", &env("QUERI_SSH_CONTAINER")])
            .status()?
            .success()
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if tokio::net::TcpStream::connect((
                env("QUERI_SSH_HOST"),
                env("QUERI_SSH_PORT").parse::<u16>().unwrap(),
            ))
            .await
            .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await?;
    Ok(())
}

fn authentication_count() -> Result<usize> {
    let logs = std::process::Command::new("docker")
        .args(["logs", &env("QUERI_SSH_CONTAINER")])
        .output()?;
    ensure!(logs.status.success());
    Ok(String::from_utf8_lossy(&logs.stderr)
        .matches("Accepted publickey")
        .count())
}

fn transport_releases() -> Result<usize> {
    let logs = std::process::Command::new("docker")
        .args(["logs", &env("QUERI_SSH_CONTAINER")])
        .output()?;
    ensure!(logs.status.success());
    Ok(String::from_utf8_lossy(&logs.stderr)
        .matches("Queri released transport")
        .count())
}

#[test]
#[ignore = "Requires disposable OpenSSH/PostgreSQL fixture; scripts/ssh-fixture/run.sh"]
fn ssh_full_route() -> Result<()> {
    runtime()?.block_on(async {
        let directory = tempfile::tempdir()?;
        let store = Store::open(directory.path().join("app.db"))?;
        let connector = Connector::new(store.clone());
        let mut prompts = connector.take_prompts().unwrap();
        let p = profile();
        assert!(Session::connect(p.clone(), String::new()).await.is_err(), "SSH must never fall back to a direct route");

        let attempt = connect(&connector, p.clone());
        prompt(&mut prompts, HostDecision::Reject, false).await?;
        ensure!(attempt.await?.is_err(), "Rejected host must fail");
        let accepted_before = authentication_count()?;
        assert_eq!(accepted_before, 0, "Rejected host was not authenticated");
        let abandoned_waiter = connect(&connector, p.clone());
        let attempt = connect(&connector, p.clone());
        // A cancelled waiter must not abandon the shared verification prompt.
        let host_prompt = tokio::time::timeout(Duration::from_secs(10), prompts.recv()).await?.context("Missing shared host prompt")?;
        // Human verification can outlive the 15-second SSH network deadline.
        tokio::time::sleep(Duration::from_secs(16)).await;
        abandoned_waiter.abort();
        let _ = abandoned_waiter.await;
        match host_prompt { Prompt::Host { reply, cancelled, .. } => { assert!(!cancelled.is_cancelled()); let _ = reply.send(HostDecision::Session); }, _ => anyhow::bail!("Expected host prompt") }
        let first = attempt.await??;
        assert_eq!(scalar(&first, "SELECT 42").await?, "42");
        assert!(store.ssh_host(&p.ssh.as_ref().unwrap().endpoint())?.is_none());
        let second = connect(&connector, p.clone()).await??;
        assert_eq!(authentication_count()?, accepted_before + 1, "Tabs share exactly one authenticated SSH transport");
        let releases=transport_releases()?;
        let mut temporary=p.clone(); temporary.id=Uuid::new_v4();
        let consumer_one=connect(&connector,temporary.clone()).await??;
        let consumer_two=connect(&connector,temporary).await??;
        drop(consumer_one);
        assert_eq!(scalar(&consumer_two,"SELECT 15").await?,"15");
        assert_eq!(transport_releases()?,releases,"One tab closing must preserve another tab's transport");
        drop(consumer_two);
        tokio::time::timeout(Duration::from_secs(5),async {
            loop { if transport_releases()? > releases { break Ok::<_,anyhow::Error>(()); } tokio::time::sleep(Duration::from_millis(10)).await; }
        }).await.context("Last consumer did not disconnect SSH transport")??;
        let shared_authentications=authentication_count()?;

        first.query("BEGIN; CREATE TEMP TABLE tab_private(value int); INSERT INTO tab_private VALUES (1)".into()).await?;
        assert_eq!(first.transaction_status().await?, TransactionStatus::Open);
        assert_eq!(second.transaction_status().await?, TransactionStatus::Idle);
        assert_eq!(scalar(&second, "SELECT to_regclass('pg_temp.tab_private') IS NULL").await?, "t");
        assert_ne!(scalar(&first, "SELECT pg_backend_pid()").await?, scalar(&second, "SELECT pg_backend_pid()").await?);
        first.query("ROLLBACK".into()).await?;
        let maintenance = connect(&connector, p.for_database("postgres")).await??;
        maintenance.database_action(DatabaseAction::Create { name:"ssh_admin_route".into() }).await?;
        maintenance.database_action(DatabaseAction::Delete { name:"ssh_admin_route".into(), force:false }).await?;
        assert!(!maintenance.schemas().await?.is_empty());
        assert_eq!(authentication_count()?,shared_authentications,"Maintenance databases share the profile transport");
        let mut policy = p.clone(); policy.id=Uuid::new_v4(); policy.policy=ExecutionPolicy::ReadOnlyByDefault;
        let readonly=connect(&connector,policy).await??;
        assert_eq!(scalar(&readonly,"SHOW default_transaction_read_only").await?,"on");
        readonly.set_statement_timeout(StatementTimeout::custom(30)?)?;
        let timeout=readonly.query("SELECT pg_sleep(1)".into()).await.err().context("SSH query did not respect statement timeout")?;
        assert!(queri::database::is_statement_timeout(&timeout));
        drop(readonly);

        let query = { let first = first.clone(); tokio::spawn(async move { first.query("SELECT pg_sleep(30)".into()).await }) };
        // Observe pg_sleep from the independent tab before issuing cancellation.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if scalar(&second, "SELECT count(*) FROM pg_stat_activity WHERE application_name='Queri' AND wait_event='PgSleep'").await? != "0" { break Ok::<_,anyhow::Error>(()); }
                tokio::task::yield_now().await;
            }
        }).await??;
        tokio::time::timeout(Duration::from_secs(3), first.cancel()).await??;
        let error = tokio::time::timeout(Duration::from_secs(3), query).await??.err().context("pg_sleep did not cancel")?;
        ensure!(format!("{error:#}").contains("cancel"), "{error:#}");
        assert_eq!(scalar(&first, "SELECT 7").await?, "7");

        // Trust disappears across app instances unless explicitly persisted.
        let independent = Connector::new(store.clone());
        let mut independent_prompts = independent.take_prompts().unwrap();
        let attempt = connect(&independent, p.clone());
        prompt(&mut independent_prompts, HostDecision::Persist, false).await?;
        drop(attempt.await??);
        let restarted = Connector::new(Store::open(directory.path().join("app.db"))?);
        let _restarted_prompts = restarted.take_prompts().unwrap();
        assert_eq!(scalar(&connect(&restarted, p.clone()).await??, "SELECT 8").await?, "8");

        let mut encrypted = p.clone(); encrypted.id = Uuid::new_v4();
        encrypted.ssh.as_mut().unwrap().auth = SshAuth::PrivateKey { path: env("QUERI_SSH_ENCRYPTED_KEY").into() };
        connector.supply_passphrase(encrypted.id, env("QUERI_SSH_PASSPHRASE"));
        assert_eq!(scalar(&connect(&connector, encrypted.clone()).await??, "SELECT 9").await?, "9");
        connector.supply_passphrase(encrypted.id, "wrong".into());
        assert!(connect(&connector, encrypted.clone()).await?.is_err());
        // Interactive unlock path uses the same worker as a supplied passphrase.
        let attempt = connect(&connector, encrypted.clone());
        match tokio::time::timeout(Duration::from_secs(10), prompts.recv()).await?.context("Missing passphrase prompt")? {
            Prompt::Passphrase { reply, .. } => { let _ = reply.send(Some(env("QUERI_SSH_PASSPHRASE"))); },
            _ => anyhow::bail!("Expected passphrase prompt"),
        }
        drop(attempt.await??);
        for key in ["QUERI_SSH_RSA_KEY","QUERI_SSH_ECDSA_KEY","QUERI_SSH_PKCS8_KEY"] {
            let mut keyed = p.clone(); keyed.id=Uuid::new_v4(); keyed.ssh.as_mut().unwrap().auth=SshAuth::PrivateKey { path:env(key).into() };
            if key=="QUERI_SSH_PKCS8_KEY" { connector.supply_passphrase(keyed.id,env("QUERI_SSH_PASSPHRASE")); }
            assert_eq!(scalar(&connect(&connector,keyed).await??,"SELECT 14").await?,"14");
        }
        let mut agent = p.clone(); agent.id = Uuid::new_v4();
        let fingerprint = keys::load_secret_key(env("QUERI_SSH_KEY"), None)?.public_key().fingerprint(HashAlg::Sha256).to_string();
        assert!(queri::ssh::agent_identities().await?.contains(&fingerprint));
        agent.ssh.as_mut().unwrap().auth = SshAuth::Agent { fingerprint };
        let signing_started = std::time::Instant::now();
        assert_eq!(scalar(&connect(&connector, agent).await??, "SELECT 10").await?, "10");
        assert!(signing_started.elapsed() >= Duration::from_secs(16), "Fixture must exercise delayed agent confirmation");

        let mut tls = p.clone(); tls.id = Uuid::new_v4();
        if let ConnectionConfig::Postgres { tls: mode, .. } = &mut tls.config { *mode = TlsMode::Verify; }
        let verified = connect(&connector, tls.clone()).await??;
        assert_eq!(scalar(&verified, "SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()").await?, "t");
        let mut mismatch = tls.clone(); mismatch.id = Uuid::new_v4();
        if let ConnectionConfig::Postgres { host, .. } = &mut mismatch.config { *host = "wrong-database".into(); }
        let error = connect(&connector, mismatch.clone()).await?.err().context("TLS hostname mismatch was accepted")?;
        ensure!(format!("{error:#}").contains("TLS"), "{error:#}");
        if let ConnectionConfig::Postgres { tls, .. } = &mut mismatch.config { *tls = TlsMode::Require; }
        connector.invalidate(mismatch.id);
        assert_eq!(scalar(&connect(&connector, mismatch).await??, "SELECT 11").await?, "11");
        let before_test = authentication_count()?;
        connector.test(tls, env("QUERI_SSH_DATABASE_PASSWORD"), String::new()).await?;
        assert_eq!(authentication_count()?,before_test+1,"Connection test establishes an isolated SSH transport");

        // Kill the transport during a query. No SQL or temporary state is replayed on reconnect.
        first.query("CREATE TABLE IF NOT EXISTS ssh_no_replay(value int); TRUNCATE ssh_no_replay; INSERT INTO ssh_no_replay VALUES(1)".into()).await?;
        first.query("CREATE TEMP TABLE do_not_replay(value int); BEGIN; INSERT INTO ssh_no_replay VALUES (2)".into()).await?;
        let lost = first.connection_lost().unwrap();
        let old = first.clone();
        let old_query = tokio::spawn(async move { old.query("COMMIT; SELECT pg_sleep(30)".into()).await });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if scalar(&second,"SELECT count(*) FROM ssh_no_replay").await? == "2" { break Ok::<_,anyhow::Error>(()); }
                tokio::task::yield_now().await;
            }
        }).await??;
        restart_bastion().await?;
        tokio::time::timeout(Duration::from_secs(5), lost.cancelled()).await.context("Dependent PG session did not observe transport loss")?;
        assert!(tokio::time::timeout(Duration::from_secs(5), old_query).await??.is_err());
        assert_eq!(first.transaction_status().await?, TransactionStatus::Unknown);
        tokio::time::timeout(Duration::from_secs(5), second.connection_lost().unwrap().cancelled()).await?;
        assert!(first.cancel().await.is_err());
        assert!(connect(&connector, p.clone()).await?.is_err(), "Lost transports require explicit reconnect");
        connector.prepare_reconnect(p.id);
        let replacement = connect(&connector, p.clone()).await??;
        assert_eq!(replacement.transaction_status().await?, TransactionStatus::Idle);
        assert_eq!(scalar(&replacement, "SELECT count(*) FROM ssh_no_replay").await?, "2");
        assert_eq!(scalar(&replacement, "SELECT to_regclass('pg_temp.do_not_replay') IS NULL").await?, "t");
        replacement.query("DROP TABLE ssh_no_replay".into()).await?;
        // Old tokens cannot target the replacement transport.
        assert!(first.cancel().await.is_err());
        assert_eq!(scalar(&replacement, "SELECT 12").await?, "12");

        // Rotate the real server handshake key. The old fingerprint remains until explicit replacement.
        eprintln!("Checking actual host-key rotation");
        let old_fingerprint = store.ssh_host(&p.ssh.as_ref().unwrap().endpoint())?.unwrap().1;
        let host_key = env("QUERI_SSH_HOST_KEY");
        let new_key = directory.path().join("new_host_key");
        ensure!(std::process::Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&new_key).status()?.success());
        std::fs::copy(&new_key, &host_key)?;
        std::fs::copy(new_key.with_extension("pub"), env("QUERI_SSH_HOST_PUBLIC_KEY"))?;
        restart_bastion().await?;
        connector.invalidate(p.id);
        let attempt = connect(&connector, p.clone());
        prompt(&mut prompts, HostDecision::Session, true).await?;
        assert!(attempt.await?.is_err(), "Changed host key must reject session acceptance");
        assert_eq!(store.ssh_host(&p.ssh.as_ref().unwrap().endpoint())?.unwrap().1, old_fingerprint);
        let attempt = connect(&connector, p.clone());
        prompt(&mut prompts, HostDecision::Replace, true).await?;
        assert_eq!(scalar(&attempt.await??, "SELECT 13").await?, "13");
        assert_ne!(store.ssh_host(&p.ssh.as_ref().unwrap().endpoint())?.unwrap().1, old_fingerprint);
        // All waiters abandoning an unknown-host attempt cleans up its prompt and socket.
        eprintln!("Checking lifecycle cancellation and profile edits");
        let isolated_store = Store::open(directory.path().join("lifecycle.db"))?;
        let lifecycle = Connector::new(isolated_store.clone());
        let mut lifecycle_prompts = lifecycle.take_prompts().unwrap();
        let attempt = connect(&lifecycle, p.clone());
        let pending = tokio::time::timeout(Duration::from_secs(10), lifecycle_prompts.recv()).await?.context("No lifecycle prompt")?;
        let Prompt::Host { reply, cancelled, .. } = pending else { anyhow::bail!("Expected lifecycle host prompt") };
        attempt.abort(); let _ = attempt.await;
        tokio::time::timeout(Duration::from_secs(3), cancelled.cancelled()).await?;
        assert!(reply.is_closed());
        assert!(isolated_store.ssh_host(&p.ssh.as_ref().unwrap().endpoint())?.is_none());
        // Editing while a prompt is pending invalidates that exact attempt.
        let attempt = connect(&lifecycle, p.clone());
        let pending = tokio::time::timeout(Duration::from_secs(10), lifecycle_prompts.recv()).await?.context("No edit-race prompt")?;
        let Prompt::Host { reply, cancelled, .. } = pending else { anyhow::bail!("Expected edit-race host prompt") };
        lifecycle.invalidate(p.id);
        assert!(attempt.await?.is_err());
        tokio::time::timeout(Duration::from_secs(3), cancelled.cancelled()).await?;
        assert!(reply.send(HostDecision::Persist).is_err());
        assert!(isolated_store.ssh_host(&p.ssh.as_ref().unwrap().endpoint())?.is_none());
        eprintln!("Verified real SSH: keys, agent, trust, forwarding DNS, TLS names, cancel, independent tabs, transport loss and explicit reconnect.");
        Ok(())
    })
}

#[test]
#[ignore = "Requires disposable OpenSSH/PostgreSQL fixture"]
fn ssh_test_connection_does_not_save_passphrase() -> Result<()> {
    runtime()?.block_on(async {
        let directory = tempfile::tempdir()?;
        let store = Store::open(directory.path().join("app.db"))?;
        let connector = Connector::new(store.clone());
        let mut prompts = connector.take_prompts().unwrap();
        let mut profile = profile();
        let ssh = profile.ssh.as_mut().unwrap();
        ssh.auth = SshAuth::PrivateKey {
            path: env("QUERI_SSH_ENCRYPTED_KEY").into(),
        };
        ssh.remember_passphrase = true;
        store.save_profile(&profile)?;
        let id = profile.id;
        let attempt = tokio::spawn(async move {
            connector
                .test(profile, env("QUERI_SSH_DATABASE_PASSWORD"), String::new())
                .await
        });
        match tokio::time::timeout(Duration::from_secs(10), prompts.recv())
            .await?
            .context("Missing passphrase prompt")?
        {
            Prompt::Passphrase { reply, .. } => {
                let _ = reply.send(Some(env("QUERI_SSH_PASSPHRASE")));
            }
            _ => anyhow::bail!("Expected passphrase prompt"),
        }
        prompt(&mut prompts, HostDecision::Session, false).await?;
        attempt.await??;
        assert!(
            store.load_ssh_passphrase(id)?.is_none(),
            "Test connection must not save credentials"
        );
        Ok(())
    })
}
