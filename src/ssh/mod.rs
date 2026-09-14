//! App-owned SSH trust and transport coordination. No GPUI types cross this boundary.
use crate::{database::Session, model::*, storage::Store};
use anyhow::{Context, Result, bail, ensure};
use russh::{
    client,
    keys::{
        self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate,
        agent::{AgentIdentity, client::AgentClient},
    },
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const NETWORK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostDecision {
    Reject,
    Session,
    Persist,
    Replace,
}

pub enum Prompt {
    Host {
        endpoint: String,
        algorithm: String,
        fingerprint: String,
        previous: Option<String>,
        reply: oneshot::Sender<HostDecision>,
        cancelled: CancellationToken,
    },
    Passphrase {
        name: String,
        reply: oneshot::Sender<Option<String>>,
        cancelled: CancellationToken,
    },
}

struct Trust {
    store: Store,
    prompts: mpsc::UnboundedSender<Prompt>,
    session: Mutex<HashMap<String, (String, String)>>,
    // Serialize verification decisions across profiles sharing a bastion.
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Trust {
    async fn verify(&self, endpoint: &str, algorithm: String, fingerprint: String) -> Result<bool> {
        let gate = self
            .gates
            .lock()
            .unwrap()
            .entry(endpoint.to_owned())
            .or_default()
            .clone();
        let _guard = gate.lock().await;
        let store = self.store.clone();
        let host = endpoint.to_owned();
        let saved = tokio::task::spawn_blocking(move || store.ssh_host(&host)).await??;
        let session = self.session.lock().unwrap().get(endpoint).cloned();
        let disk_fingerprint = saved.as_ref().map(|(_, fp)| fp.clone());
        let trusted = saved.or(session);
        if trusted.as_ref() == Some(&(algorithm.clone(), fingerprint.clone())) {
            return Ok(true);
        }
        let previous = trusted.map(|(_, fp)| fp);
        let (reply, response) = oneshot::channel();
        let cancelled = CancellationToken::new();
        let _cancel_prompt = cancelled.clone().drop_guard();
        self.prompts
            .send(Prompt::Host {
                endpoint: endpoint.to_owned(),
                algorithm: algorithm.clone(),
                fingerprint: fingerprint.clone(),
                previous: previous.clone(),
                reply,
                cancelled,
            })
            .map_err(|_| anyhow::anyhow!("SSH host verification UI unavailable"))?;
        let decision = response.await.context("SSH host verification cancelled")?;
        if decision == HostDecision::Reject {
            return Ok(false);
        }
        if previous.is_some() {
            ensure!(
                decision == HostDecision::Replace,
                "SSH host key changed; explicit replacement is required"
            );
        } else {
            ensure!(
                decision != HostDecision::Replace,
                "No SSH host key to replace"
            );
        }
        if matches!(decision, HostDecision::Persist | HostDecision::Replace) {
            let store = self.store.clone();
            let host = endpoint.to_owned();
            let alg = algorithm.clone();
            let fp = fingerprint.clone();
            // Compare against disk, which may differ from a session-only trust entry.
            let expected = disk_fingerprint;
            tokio::task::spawn_blocking(move || {
                store.save_ssh_host(&host, &alg, &fp, expected.as_deref())
            })
            .await??;
        }
        self.session
            .lock()
            .unwrap()
            .insert(endpoint.to_owned(), (algorithm, fingerprint));
        Ok(true)
    }
}

pub struct HostHandler {
    endpoint: String,
    trust: Arc<Trust>,
    prompting: watch::Sender<bool>,
    closed: CancellationToken,
    attempt_cancel: CancellationToken,
}

impl Drop for HostHandler {
    fn drop(&mut self) {
        self.closed.cancel();
    }
}

impl client::Handler for HostHandler {
    type Error = anyhow::Error;
    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool> {
        ensure!(
            key.certificate().is_none(),
            "SSH host certificates are unsupported; use an ordinary host key"
        );
        let key = key.public_key();
        self.prompting.send_replace(true);
        let result = tokio::select! {
            biased;
            _ = self.attempt_cancel.cancelled() => Err(anyhow::anyhow!("SSH verification attempt cancelled")),
            _ = self.closed.cancelled() => Err(anyhow::anyhow!("SSH transport closed")),
            result = self.trust.verify(&self.endpoint, key.algorithm().to_string(), key.fingerprint(HashAlg::Sha256).to_string()) => result,
        };
        self.prompting.send_replace(false);
        result
    }
}

pub struct Transport {
    handle: Arc<client::Handle<HostHandler>>,
    pub closed: CancellationToken,
}

impl Transport {
    pub async fn channel(
        &self,
        host: &str,
        port: u16,
    ) -> Result<russh::ChannelStream<client::Msg>> {
        ensure!(
            !self.closed.is_cancelled(),
            "SSH transport lost. Reconnect explicitly; the previous transaction outcome may be unknown."
        );
        let channel = tokio::time::timeout(
            NETWORK_TIMEOUT,
            self.handle
                .channel_open_direct_tcpip(host, u32::from(port), "127.0.0.1", 0),
        )
        .await
        .context("SSH forwarding timed out")?
        .context("SSH forwarding failed (database host resolves on the bastion)")?;
        Ok(channel.into_stream())
    }

    pub fn shutdown(&self) {
        self.closed.cancel();
        let handle = self.handle.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    handle.disconnect(
                        russh::Disconnect::ByApplication,
                        "Queri released transport",
                        "en",
                    ),
                )
                .await;
            });
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.shutdown();
    }
}

type Establishment = std::result::Result<Arc<Transport>, Arc<str>>;
struct Pending {
    result: watch::Sender<Option<Establishment>>,
    waiters: AtomicUsize,
    cancel: CancellationToken,
}
struct Waiter(Arc<Pending>);
impl Drop for Waiter {
    fn drop(&mut self) {
        if self.0.waiters.fetch_sub(1, Ordering::AcqRel) == 1 && self.0.result.borrow().is_none() {
            self.0.cancel.cancel();
        }
    }
}
struct Slot {
    generation: String,
    ready: Weak<Transport>,
    pending: Weak<Pending>,
}

/// Owned by Services. Cache entries never keep unused SSH connections alive.
pub struct Connector {
    trust: Arc<Trust>,
    slots: Mutex<HashMap<Uuid, Slot>>,
    // One attempt's explicitly entered passphrase; never retained after use.
    supplied: Mutex<HashMap<Uuid, String>>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Prompt>>>,
    progress: Option<watch::Sender<&'static str>>,
}

impl Connector {
    pub fn new(store: Store) -> Arc<Self> {
        let (prompts, receiver) = mpsc::unbounded_channel();
        Arc::new(Self {
            trust: Arc::new(Trust {
                store,
                prompts,
                session: Mutex::new(HashMap::new()),
                gates: Mutex::new(HashMap::new()),
            }),
            slots: Mutex::new(HashMap::new()),
            supplied: Mutex::new(HashMap::new()),
            progress: None,
            receiver: Mutex::new(Some(receiver)),
        })
    }
    pub fn take_prompts(&self) -> Option<mpsc::UnboundedReceiver<Prompt>> {
        self.receiver.lock().unwrap().take()
    }
    pub fn supply_passphrase(&self, id: Uuid, value: String) {
        self.supplied.lock().unwrap().insert(id, value);
    }
    pub fn clear_passphrase(&self, id: Uuid) {
        self.supplied.lock().unwrap().remove(&id);
    }
    pub fn invalidate(&self, id: Uuid) {
        self.supplied.lock().unwrap().remove(&id);
        if let Some(slot) = self.slots.lock().unwrap().remove(&id) {
            if let Some(pending) = slot.pending.upgrade() {
                pending.cancel.cancel();
            }
            if let Some(transport) = slot.ready.upgrade() {
                transport.shutdown();
            }
        }
    }
    pub fn shutdown(&self) {
        let ids = self
            .slots
            .lock()
            .unwrap()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for id in ids {
            self.invalidate(id);
        }
        self.supplied.lock().unwrap().clear();
    }
    pub fn prepare_reconnect(&self, id: Uuid) {
        let mut slots = self.slots.lock().unwrap();
        if slots
            .get(&id)
            .is_some_and(|s| s.ready.upgrade().is_some_and(|t| t.closed.is_cancelled()))
        {
            slots.remove(&id);
        }
    }
    pub fn forget_host(&self, endpoint: &str) -> Result<()> {
        self.trust.store.remove_ssh_host(endpoint)?;
        self.trust.session.lock().unwrap().remove(endpoint);
        Ok(())
    }
    pub async fn connect(
        self: &Arc<Self>,
        profile: ConnectionProfile,
        password: String,
    ) -> Result<Arc<Session>> {
        profile.validate()?;
        let transport = if profile.ssh.is_some() {
            Some(self.transport(&profile).await?)
        } else {
            None
        };
        let generation = serde_json::to_string(&profile.for_database(""))?;
        let id = profile.id;
        self.stage("Connecting to PostgreSQL / negotiating database TLS…");
        let session = Session::connect_transport(profile, password, transport.clone()).await?;
        if let Some(transport) = transport {
            ensure!(
                !transport.closed.is_cancelled()
                    && self
                        .slots
                        .lock()
                        .unwrap()
                        .get(&id)
                        .is_some_and(|slot| slot.generation == generation),
                "Connection settings changed or SSH transport closed during establishment"
            );
        }
        Ok(session)
    }
    /// A test uses a separate coordinator and transport, while sharing app-owned trust.
    pub async fn test(
        self: &Arc<Self>,
        profile: ConnectionProfile,
        password: String,
        passphrase: String,
    ) -> Result<()> {
        self.test_progress(profile, password, passphrase, None)
            .await
    }
    pub async fn test_progress(
        self: &Arc<Self>,
        profile: ConnectionProfile,
        password: String,
        passphrase: String,
        progress: Option<watch::Sender<&'static str>>,
    ) -> Result<()> {
        let isolated = Arc::new(Self {
            trust: self.trust.clone(),
            slots: Mutex::new(HashMap::new()),
            supplied: Mutex::new(HashMap::from([(profile.id, passphrase)])),
            progress,
            receiver: Mutex::new(None),
        });
        let session = isolated.connect(profile, password).await?;
        isolated.stage("Running test query…");
        session.query("SELECT 1".into()).await?;
        Ok(())
    }
    fn stage(&self, message: &'static str) {
        if let Some(progress) = &self.progress {
            progress.send_replace(message);
        }
    }
    async fn transport(self: &Arc<Self>, profile: &ConnectionProfile) -> Result<Arc<Transport>> {
        let generation = serde_json::to_string(&profile.for_database(""))?;
        let pending = {
            let mut slots = self.slots.lock().unwrap();
            let slot = slots.entry(profile.id).or_insert_with(|| Slot {
                generation: generation.clone(),
                ready: Weak::new(),
                pending: Weak::new(),
            });
            ensure!(
                slot.generation == generation,
                "Connection settings changed; reconnect using the current profile"
            );
            if let Some(ready) = slot.ready.upgrade() {
                ensure!(
                    !ready.closed.is_cancelled(),
                    "SSH transport lost. Use Reconnect; SQL is never replayed."
                );
                return Ok(ready);
            }
            if let Some(pending) = slot
                .pending
                .upgrade()
                .filter(|p| !p.cancel.is_cancelled() && p.result.borrow().is_none())
            {
                pending.waiters.fetch_add(1, Ordering::Relaxed);
                pending
            } else {
                let (result, _) = watch::channel(None);
                let pending = Arc::new(Pending {
                    result,
                    waiters: AtomicUsize::new(1),
                    cancel: CancellationToken::new(),
                });
                slot.pending = Arc::downgrade(&pending);
                let connector = self.clone();
                let profile = profile.clone();
                let job = pending.clone();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        biased;
                        _ = job.cancel.cancelled() => Err(anyhow::anyhow!("SSH connection cancelled")),
                        result = connector.establish(&profile, job.cancel.clone()) => result,
                    };
                    if let Ok(transport) = &result {
                        let mut slots = connector.slots.lock().unwrap();
                        if let Some(slot) = slots.get_mut(&profile.id).filter(|s| {
                            s.generation == generation
                                && s.pending.upgrade().is_some_and(|p| Arc::ptr_eq(&p, &job))
                        }) {
                            slot.ready = Arc::downgrade(transport);
                        } else {
                            transport.shutdown();
                        }
                    }
                    job.result
                        .send_replace(Some(result.map_err(|e| Arc::from(format!("{e:#}")))));
                });
                pending
            }
        };
        let _waiter = Waiter(pending.clone());
        let mut result = pending.result.subscribe();
        loop {
            if let Some(result) = result.borrow_and_update().clone() {
                return result.map_err(|e| anyhow::anyhow!(e.to_string()));
            }
            result
                .changed()
                .await
                .context("SSH connection worker stopped")?;
        }
    }
    async fn establish(
        &self,
        profile: &ConnectionProfile,
        attempt_cancel: CancellationToken,
    ) -> Result<Arc<Transport>> {
        let cancel_on_failure = attempt_cancel.clone().drop_guard();
        let ssh = profile.ssh.as_ref().context("Missing SSH settings")?;
        // Key decryption is deliberately outside Tokio's I/O workers and the GPUI thread.
        self.stage("Reading SSH authentication credentials…");
        let mut unlocked_passphrase = None;
        let private_key = if let SshAuth::PrivateKey { path } = &ssh.auth {
            let path = path.clone();
            let supplied = self.supplied.lock().unwrap().remove(&profile.id);
            let store = self.trust.store.clone();
            let id = profile.id;
            let remember = ssh.remember_passphrase;
            let saved = tokio::task::spawn_blocking(move || {
                if remember {
                    store.load_ssh_passphrase(id)
                } else {
                    Ok(None)
                }
            })
            .await??;
            let passphrase = supplied.or(saved).filter(|value| !value.is_empty());
            let key = tokio::task::spawn_blocking({
                let path = path.clone();
                let passphrase = passphrase.clone();
                move || load_private_key(path, passphrase.as_deref())
            })
            .await?;
            Some(match key {
                Ok(key) => key,
                Err(keys::Error::KeyIsEncrypted) if passphrase.is_none() => {
                    let (reply, response) = oneshot::channel();
                    let cancelled = CancellationToken::new();
                    let _cancel_prompt = cancelled.clone().drop_guard();
                    self.stage("Waiting for SSH key passphrase…");
                    self.trust
                        .prompts
                        .send(Prompt::Passphrase {
                            name: profile.display_name(),
                            reply,
                            cancelled,
                        })
                        .map_err(|_| anyhow::anyhow!("SSH passphrase UI unavailable"))?;
                    let passphrase = response
                        .await
                        .context("SSH passphrase cancelled")?
                        .context("SSH passphrase cancelled")?;
                    unlocked_passphrase = Some(passphrase.clone());
                    tokio::task::spawn_blocking(move || load_private_key(path, Some(&passphrase)))
                        .await?
                        .context("SSH private key could not be decrypted or parsed")?
                }
                Err(_) => bail!(
                    "SSH private key could not be decrypted or parsed; check its path and passphrase"
                ),
            })
        } else {
            None
        };
        self.stage("Connecting to SSH bastion…");
        let socket = tokio::time::timeout(
            NETWORK_TIMEOUT,
            tokio::net::TcpStream::connect((ssh.host.trim().trim_matches(['[', ']']), ssh.port)),
        )
        .await
        .context("SSH TCP connection timed out")?
        .context("SSH TCP connection failed")?;
        let (prompting, mut prompt_state) = watch::channel(false);
        let closed = CancellationToken::new();
        let handler = HostHandler {
            attempt_cancel,
            endpoint: ssh.endpoint(),
            trust: self.trust.clone(),
            prompting,
            closed: closed.clone(),
        };
        let config = client::Config {
            keepalive_interval: Some(Duration::from_secs(10)),
            keepalive_max: 3,
            nodelay: true,
            ..Default::default()
        };
        self.stage("SSH handshake / verifying host key…");
        let connect = client::connect_stream(Arc::new(config), socket, handler);
        tokio::pin!(connect);
        // Human verification has no network deadline. Resume a fresh bounded handshake afterwards.
        let mut handle = loop {
            let human = *prompt_state.borrow_and_update();
            tokio::select! {
                result = &mut connect => break result.context("SSH handshake / host verification failed")?,
                _ = prompt_state.changed() => {},
                _ = tokio::time::sleep(NETWORK_TIMEOUT), if !human => bail!("SSH handshake timed out"),
            }
        };
        self.stage("Authenticating SSH public key…");
        let authenticated = tokio::time::timeout(NETWORK_TIMEOUT, async {
            let hash = handle.best_supported_rsa_hash().await?.flatten();
            if let Some(key) = private_key {
                if key.algorithm().is_rsa() {
                    ensure!(
                        hash.is_some(),
                        "SSH server does not support RSA SHA-2 signatures"
                    );
                }
                Ok::<_, anyhow::Error>(
                    handle
                        .authenticate_publickey(
                            &ssh.user,
                            PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                        )
                        .await?
                        .success(),
                )
            } else {
                let SshAuth::Agent { fingerprint } = &ssh.auth else {
                    unreachable!()
                };
                let mut agent = AgentClient::connect_env()
                    .await
                    .context("Could not connect to SSH_AUTH_SOCK")?;
                let identity = agent
                    .request_identities()
                    .await?
                    .into_iter()
                    .find_map(|identity| match identity {
                        AgentIdentity::PublicKey { key, .. }
                            if key.fingerprint(HashAlg::Sha256).to_string() == *fingerprint =>
                        {
                            Some(key)
                        }
                        _ => None,
                    })
                    .context("Chosen SSH agent identity is unavailable")?;
                if identity.algorithm().is_rsa() {
                    ensure!(
                        hash.is_some(),
                        "SSH server does not support RSA SHA-2 signatures"
                    );
                }
                Ok(handle
                    .authenticate_publickey_with(&ssh.user, identity, hash, &mut agent)
                    .await?
                    .success())
            }
        })
        .await
        .context("SSH authentication timed out")??;
        ensure!(authenticated, "SSH public key authentication rejected");
        if ssh.remember_passphrase
            && let Some(passphrase) = unlocked_passphrase
        {
            let store = self.trust.store.clone();
            let profile = profile.clone();
            tokio::task::spawn_blocking(move || {
                store.remember_ssh_passphrase(&profile, &passphrase)
            })
            .await??;
        }
        cancel_on_failure.disarm();
        Ok(Arc::new(Transport {
            handle: Arc::new(handle),
            closed,
        }))
    }
}

fn load_private_key(
    path: std::path::PathBuf,
    passphrase: Option<&str>,
) -> std::result::Result<keys::PrivateKey, keys::Error> {
    let text = std::fs::read_to_string(path)?;
    // russh's PKCS#8 decoder otherwise reports an ASN.1 error when a password is absent.
    if passphrase.is_none()
        && text
            .lines()
            .any(|line| line == "-----BEGIN ENCRYPTED PRIVATE KEY-----")
    {
        return Err(keys::Error::KeyIsEncrypted);
    }
    keys::decode_secret_key(&text, passphrase)
}

pub async fn agent_identities() -> Result<Vec<String>> {
    tokio::time::timeout(NETWORK_TIMEOUT, async {
        let mut agent = AgentClient::connect_env()
            .await
            .context("Could not connect to SSH_AUTH_SOCK")?;
        Ok(agent
            .request_identities()
            .await?
            .into_iter()
            .filter_map(|identity| match identity {
                AgentIdentity::PublicKey { key, .. } => {
                    Some(key.fingerprint(HashAlg::Sha256).to_string())
                }
                _ => None,
            })
            .collect())
    })
    .await
    .context("SSH agent identity lookup timed out")?
}
