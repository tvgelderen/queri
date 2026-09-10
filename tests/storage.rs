use anyhow::Result;
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use queri::{model::*, storage::Store};
use rusqlite::{Connection, params};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{Arc, Barrier},
    thread,
};
use uuid::Uuid;

fn profile() -> ConnectionProfile {
    ConnectionProfile {
        environment: None,
        policy: ExecutionPolicy::Normal,
        id: Uuid::new_v4(),
        name: "Remembered PostgreSQL".into(),
        notes: String::new(),
        color: String::new(),
        remember_password: true,
        config: ConnectionConfig::Postgres {
            host: "localhost".into(),
            port: 5432,
            database: "postgres".into(),
            user: "postgres".into(),
            tls: TlsMode::Disable,
            timeout_seconds: 5,
        },
    }
}

fn credential(conn: &Connection, id: Uuid) -> Result<(Vec<u8>, Vec<u8>)> {
    Ok(conn.query_row(
        "SELECT nonce, ciphertext FROM credentials WHERE connection_id=?1",
        [id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

#[test]
fn encrypted_password_round_trip_replacement_empty_and_restart() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("app.db");
    let store = Store::open(&path)?;
    let mut profile = profile();
    let password = "a long unique password: café 🔑";
    assert_eq!(store.load_password(profile.id)?, None);
    assert!(!directory.path().join("app.db.key").exists());
    store.save_connection(&profile, password)?;
    let conn = Connection::open(&path)?;
    let first = credential(&conn, profile.id)?;
    assert_eq!(store.load_password(profile.id)?.as_deref(), Some(password));
    assert!(
        !first
            .1
            .windows(password.len())
            .any(|bytes| bytes == password.as_bytes())
    );
    for entry in fs::read_dir(directory.path())? {
        let bytes = fs::read(entry?.path())?;
        assert!(
            !bytes
                .windows(password.len())
                .any(|bytes| bytes == password.as_bytes())
        );
    }
    let key_path = directory.path().join("app.db.key");
    let key = fs::read(&key_path)?;
    assert_eq!(key.len(), 32);
    assert_eq!(fs::metadata(&key_path)?.permissions().mode() & 0o777, 0o600);
    store.save_connection(&profile, password)?;
    let second = credential(&conn, profile.id)?;
    assert_ne!(first.0, second.0);
    assert_ne!(first.1, second.1);
    assert_eq!(fs::read(&key_path)?, key);
    profile.name = "Renamed metadata".into();
    store.save_profile(&profile)?;
    assert_eq!(credential(&conn, profile.id)?, second);
    store.save_connection(&profile, "replacement")?;
    assert_eq!(
        store.load_password(profile.id)?.as_deref(),
        Some("replacement")
    );
    store.save_connection(&profile, "")?;
    assert_eq!(store.load_password(profile.id)?.as_deref(), Some(""));
    drop(store);
    let store = Store::open(&path)?;
    assert_eq!(store.profiles()?[0].name, "Renamed metadata");
    assert_eq!(store.load_password(profile.id)?.as_deref(), Some(""));
    Ok(())
}

#[test]
fn remember_off_and_cascading_delete_work_without_a_key() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("app.db");
    let store = Store::open(&path)?;
    let mut first = profile();
    let second = profile();
    store.save_connection(&first, "first password")?;
    store.save_connection(&second, "second password")?;
    fs::remove_file(directory.path().join("app.db.key"))?;
    first.remember_password = false;
    store.save_connection(&first, "still entered for this session")?;
    assert_eq!(store.load_password(first.id)?, None);
    assert_eq!(store.profiles()?.len(), 2);
    assert!(
        !store
            .profiles()?
            .iter()
            .find(|profile| profile.id == first.id)
            .expect("first profile")
            .remember_password
    );
    store.delete_profile(second.id)?;
    assert_eq!(store.load_password(second.id)?, None);
    let conn = Connection::open(&path)?;
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM credentials", [], |row| row
            .get::<_, i64>(0))?,
        0
    );
    assert_eq!(store.profiles()?.len(), 1);
    conn.pragma_update(None, "foreign_keys", true)?;
    assert!(
        conn.execute(
            "INSERT INTO credentials VALUES (?1, X'00', X'00')",
            [Uuid::new_v4().to_string()]
        )
        .is_err()
    );
    store.save_connection(&profile(), "new key after all old passwords are removed")?;
    Ok(())
}

#[test]
fn session_only_and_sqlite_connections_do_not_create_keys() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("app.db"))?;
    let mut profile = profile();
    profile.remember_password = false;
    store.save_connection(&profile, "session only")?;
    assert_eq!(store.load_password(profile.id)?, None);
    profile.remember_password = true;
    profile.config = ConnectionConfig::Sqlite {
        path: directory.path().join("data.db"),
        read_only: false,
        create: true,
        wal: true,
        foreign_keys: true,
    };
    store.save_connection(&profile, "ignored for SQLite")?;
    assert_eq!(store.load_password(profile.id)?, None);
    assert!(!directory.path().join("app.db.key").exists());
    Ok(())
}

#[test]
fn version_one_migration_preserves_settings_drafts_and_history() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("app.db");
    let profile = profile();
    let preferences = Preferences {
        active_connection: Some(profile.id),
        drafts: vec![Draft {
            statement_timeout: StatementTimeout::Inherit,
            saved_query: None,
            relationship: None,
            connection_id: profile.id,
            database: "postgres".into(),
            sql: "SELECT 'draft'".into(),
            query_number: Some(3),
            table: None,
            editor_height: 300.,
        }],
        ..Default::default()
    };
    let conn = Connection::open(&path)?;
    conn.execute_batch("CREATE TABLE connections (id TEXT PRIMARY KEY, profile TEXT NOT NULL);
        CREATE TABLE preferences (id INTEGER PRIMARY KEY CHECK (id=1), value TEXT NOT NULL);
        CREATE TABLE history (id INTEGER PRIMARY KEY, connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
            database_name TEXT NOT NULL, sql TEXT NOT NULL, executed_at TEXT NOT NULL, elapsed_ms INTEGER NOT NULL, affected INTEGER NOT NULL, error TEXT);
        CREATE INDEX history_connection_time ON history(connection_id, id DESC);
        PRAGMA user_version=1;")?;
    conn.execute(
        "INSERT INTO connections VALUES (?1, ?2)",
        params![profile.id.to_string(), serde_json::to_string(&profile)?],
    )?;
    conn.execute(
        "INSERT INTO preferences VALUES (1, ?1)",
        [serde_json::to_string(&preferences)?],
    )?;
    conn.execute(
        "INSERT INTO history VALUES (1, ?1, 'postgres', 'SELECT 42', '2026-11-05', 7, 1, NULL)",
        [profile.id.to_string()],
    )?;
    let store = Store::open(&path)?;
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))?,
        3
    );
    assert_eq!(
        serde_json::to_value(&store.profiles()?[0])?,
        serde_json::to_value(&profile)?
    );
    assert_eq!(
        serde_json::to_value(store.preferences()?)?,
        serde_json::to_value(preferences)?
    );
    assert_eq!(store.history(profile.id, "", 0)?[0].sql, "SELECT 42");
    assert_eq!(store.load_password(profile.id)?, None);
    assert!(!directory.path().join("app.db.key").exists());
    store.save_connection(&profile, "entered once after updating")?;
    assert_eq!(
        store.load_password(profile.id)?.as_deref(),
        Some("entered once after updating")
    );
    assert_eq!(store.history(profile.id, "", 0)?.len(), 1);
    Ok(())
}

#[test]
fn damaged_and_missing_keys_never_get_replaced_while_credentials_exist() -> Result<()> {
    for damage in [
        None,
        Some(vec![]),
        Some(vec![1; 31]),
        Some(vec![1; 33]),
        Some(vec![1; 32]),
    ] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("app.db");
        let key_path = directory.path().join("app.db.key");
        let store = Store::open(&path)?;
        let mut saved = profile();
        store.save_connection(&saved, "original password")?;
        match &damage {
            Some(bytes) => fs::write(&key_path, bytes)?,
            None => fs::remove_file(&key_path)?,
        }
        drop(store);
        let store = Store::open(&path)?;
        assert!(store.load_password(saved.id).is_err());
        saved.name = "Must roll back".into();
        assert!(store.save_connection(&saved, "replacement").is_err());
        assert!(
            store
                .save_connection(&profile(), "another password")
                .is_err()
        );
        assert_eq!(store.profiles()?.len(), 1);
        assert_ne!(store.profiles()?[0].name, "Must roll back");
        assert_eq!(fs::read(&key_path).ok(), damage);
        store.save_profile(&saved)?;
        saved.remember_password = false;
        store.save_connection(&saved, "session password")?;
        assert_eq!(store.load_password(saved.id)?, None);
        assert_eq!(fs::read(&key_path).ok(), damage);
    }
    Ok(())
}

#[test]
fn tampered_swapped_and_malformed_credentials_fail_safely() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("app.db");
    let store = Store::open(&path)?;
    let first = profile();
    let second = profile();
    store.save_connection(&first, "first secret")?;
    store.save_connection(&second, "second secret")?;
    let conn = Connection::open(&path)?;
    let (nonce, ciphertext) = credential(&conn, first.id)?;
    let mut tampered = ciphertext.clone();
    tampered[0] ^= 1;
    for malformed in [vec![], vec![0; 15], tampered] {
        conn.execute(
            "UPDATE credentials SET ciphertext=?1 WHERE connection_id=?2",
            params![malformed, first.id.to_string()],
        )?;
        assert!(store.load_password(first.id).is_err());
    }
    conn.execute(
        "UPDATE credentials SET ciphertext=?1 WHERE connection_id=?2",
        params![ciphertext, first.id.to_string()],
    )?;
    for malformed in [vec![], vec![0_u8; 23], vec![0; 25], vec![0; 24]] {
        conn.execute(
            "UPDATE credentials SET nonce=?1 WHERE connection_id=?2",
            params![malformed, first.id.to_string()],
        )?;
        assert!(store.load_password(first.id).is_err());
    }
    conn.execute(
        "UPDATE credentials SET nonce=?1 WHERE connection_id=?2",
        params![nonce, first.id.to_string()],
    )?;
    let other = credential(&conn, second.id)?;
    conn.execute(
        "UPDATE credentials SET nonce=?1, ciphertext=?2 WHERE connection_id=?3",
        params![other.0, other.1, first.id.to_string()],
    )?;
    conn.execute(
        "UPDATE credentials SET nonce=?1, ciphertext=?2 WHERE connection_id=?3",
        params![nonce, ciphertext, second.id.to_string()],
    )?;
    assert!(store.load_password(first.id).is_err());
    assert!(store.load_password(second.id).is_err());
    conn.execute("UPDATE credentials SET nonce='not a blob'", [])?;
    assert!(store.load_password(first.id).is_err());
    let key = fs::read(directory.path().join("app.db.key"))?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key)?;
    let nonce = XNonce::default();
    let invalid_text = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &[255],
                aad: first.id.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("test encryption failed"))?;
    conn.execute(
        "UPDATE credentials SET nonce=?1, ciphertext=?2 WHERE connection_id=?3",
        params![nonce.as_slice(), invalid_text, first.id.to_string()],
    )?;
    assert!(store.load_password(first.id).is_err());
    Ok(())
}

#[test]
fn concurrent_first_saves_share_a_single_key() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("app.db");
    let store = Store::open(&path)?;
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let barrier = barrier.clone();
            let path = path.clone();
            thread::spawn(move || -> Result<Uuid> {
                let profile = profile();
                barrier.wait();
                let store = Store::open(path)?;
                store.save_connection(&profile, &profile.id.to_string())?;
                Ok(profile.id)
            })
        })
        .collect();
    for handle in handles {
        let id = handle.join().expect("save thread")?;
        assert_eq!(store.load_password(id)?, Some(id.to_string()));
    }
    assert_eq!(store.profiles()?.len(), 8);
    Ok(())
}

#[test]
fn failed_key_creation_and_credential_writes_roll_back_metadata() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("app.db");
    let key_path = directory.path().join("app.db.key");
    let store = Store::open(&path)?;
    let mut profile = profile();
    fs::create_dir(&key_path)?;
    assert!(store.save_connection(&profile, "secret").is_err());
    assert!(store.profiles()?.is_empty());
    fs::remove_dir(&key_path)?;
    let conn = Connection::open(&path)?;
    conn.execute_batch("CREATE TRIGGER fail_credential_write BEFORE INSERT ON credentials BEGIN SELECT RAISE(ABORT, 'injected persistence failure'); END;")?;
    assert!(store.save_connection(&profile, "secret").is_err());
    assert!(store.profiles()?.is_empty());
    assert_eq!(store.load_password(profile.id)?, None);
    conn.execute_batch("DROP TRIGGER fail_credential_write;")?;
    store.save_connection(&profile, "original")?;
    conn.execute_batch("CREATE TRIGGER fail_credential_write BEFORE INSERT ON credentials BEGIN SELECT RAISE(ABORT, 'injected persistence failure'); END;")?;
    profile.name = "Must roll back".into();
    assert!(store.save_connection(&profile, "replacement").is_err());
    assert_ne!(store.profiles()?[0].name, "Must roll back");
    assert_eq!(
        store.load_password(profile.id)?.as_deref(),
        Some("original")
    );
    conn.execute_batch("CREATE TRIGGER fail_credential_delete BEFORE DELETE ON credentials BEGIN SELECT RAISE(ABORT, 'injected deletion failure'); END;")?;
    profile.remember_password = false;
    assert!(store.save_connection(&profile, "session password").is_err());
    assert!(store.profiles()?[0].remember_password);
    assert_eq!(
        store.load_password(profile.id)?.as_deref(),
        Some("original")
    );
    Ok(())
}

#[test]
fn saved_queries_survive_connection_removal_and_database_rename() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("app.db"))?;
    let profile = profile();
    store.save_profile(&profile)?;
    let saved = store.save_query(&SavedQuery {
        id: Uuid::new_v4(),
        name: "Find user".into(),
        sql: "SELECT 1".into(),
        favorite: true,
        connection_id: Some(profile.id),
        database: "old".into(),
        revision: 0,
    })?;
    store.rename_query_database(profile.id, "old", "new")?;
    assert_eq!(
        store.saved_queries("", Some(profile.id))?[0].database,
        "new"
    );
    assert!(store.save_query(&saved).is_err());
    store.delete_profile(profile.id)?;
    let queries = store.saved_queries("", None)?;
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].connection_id, None);
    assert_eq!(queries[0].sql, "SELECT 1");
    Ok(())
}

#[test]
fn statement_timeout_drafts_are_backward_compatible_and_persistent() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("app.db"))?;
    let legacy = serde_json::json!({
        "connection_id": uuid::Uuid::new_v4(), "database": "postgres", "sql": "SELECT 1"
    });
    let mut draft: Draft = serde_json::from_value(legacy)?;
    assert_eq!(draft.statement_timeout, StatementTimeout::Inherit);
    for timeout in [
        StatementTimeout::Inherit,
        StatementTimeout::NoTimeout,
        StatementTimeout::custom(12345)?,
    ] {
        draft.statement_timeout = timeout;
        store.save_preferences(&Preferences {
            drafts: vec![draft.clone()],
            ..Default::default()
        })?;
        assert_eq!(store.preferences()?.drafts[0].statement_timeout, timeout);
    }
    assert!(StatementTimeout::custom(0).is_err());
    assert!(StatementTimeout::custom(u32::MAX).is_err());
    assert!(serde_json::from_str::<StatementTimeout>(r#"{"Custom":0}"#).is_err());
    Ok(())
}

#[test]
fn safeguards_old_profile_defaults_and_roundtrip() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("app.db");
    let store = Store::open(path.clone())?;
    let mut profile = profile();
    let mut legacy = serde_json::to_value(&profile)?;
    legacy.as_object_mut().unwrap().remove("environment");
    legacy.as_object_mut().unwrap().remove("policy");
    Connection::open(path)?.execute(
        "INSERT INTO connections(id, profile) VALUES(?1, ?2)",
        params![profile.id.to_string(), legacy.to_string()],
    )?;
    let loaded = store.profiles()?.remove(0);
    assert_eq!(loaded.environment, None);
    assert_eq!(loaded.policy, ExecutionPolicy::Normal);
    for environment in [
        Environment::Development,
        Environment::Staging,
        Environment::Production,
    ] {
        for policy in [
            ExecutionPolicy::Normal,
            ExecutionPolicy::ConfirmExecutions,
            ExecutionPolicy::ReadOnlyByDefault,
        ] {
            profile.environment = Some(environment);
            profile.policy = policy;
            store.save_profile(&profile)?;
            let loaded = store.profiles()?.remove(0);
            assert_eq!(loaded.environment, Some(environment));
            assert_eq!(loaded.policy, policy);
        }
    }
    Ok(())
}
