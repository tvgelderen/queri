use crate::model::*;
use anyhow::{Context, Result, ensure};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{
    fs::{File, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn default_path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("QUERI_DATA_DIR") {
            return Ok(PathBuf::from(path).join("queri.sqlite3"));
        }
        let dirs = ProjectDirs::from("dev", "queri", "Queri")
            .context("Could not locate the application data directory")?;
        Ok(dirs.data_local_dir().join("queri.sqlite3"))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let store = Self { path };
        let mut conn = store.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        ensure!(
            version <= 2,
            "This application database was created by a newer Queri version."
        );
        if version == 0 {
            tx.execute_batch("CREATE TABLE connections (id TEXT PRIMARY KEY, profile TEXT NOT NULL);
                CREATE TABLE preferences (id INTEGER PRIMARY KEY CHECK (id=1), value TEXT NOT NULL);
                CREATE TABLE history (id INTEGER PRIMARY KEY, connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
                    database_name TEXT NOT NULL, sql TEXT NOT NULL, executed_at TEXT NOT NULL, elapsed_ms INTEGER NOT NULL, affected INTEGER NOT NULL, error TEXT);
                CREATE INDEX history_connection_time ON history(connection_id, id DESC);")?;
        }
        if version < 2 {
            tx.execute_batch(
                "CREATE TABLE credentials (
                connection_id TEXT PRIMARY KEY REFERENCES connections(id) ON DELETE CASCADE,
                nonce BLOB NOT NULL, ciphertext BLOB NOT NULL);
                PRAGMA user_version=2;",
            )?;
        }
        tx.commit()?;
        Ok(store)
    }

    fn connection(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Ok(conn)
    }

    pub fn profiles(&self) -> Result<Vec<ConnectionProfile>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare("SELECT profile FROM connections ORDER BY json_extract(profile, '$.name') COLLATE NOCASE")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn save_profile(&self, profile: &ConnectionProfile) -> Result<()> {
        profile.validate()?;
        write_profile(&self.connection()?, profile)
    }

    pub fn save_connection(&self, profile: &ConnectionProfile, password: &str) -> Result<()> {
        profile.validate()?;
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_profile(&tx, profile)?;
        if profile.remember_password && matches!(profile.config, ConnectionConfig::Postgres { .. })
        {
            let cipher = self.encryption_cipher(&tx)?;
            let mut nonce = XNonce::default();
            getrandom::fill(&mut nonce).context("Could not generate a password nonce")?;
            let ciphertext = cipher
                .encrypt(
                    &nonce,
                    Payload {
                        msg: password.as_bytes(),
                        aad: profile.id.as_bytes(),
                    },
                )
                .map_err(|_| anyhow::anyhow!("Could not encrypt the password"))?;
            tx.execute("INSERT INTO credentials(connection_id, nonce, ciphertext) VALUES (?1, ?2, ?3)
                ON CONFLICT(connection_id) DO UPDATE SET nonce=excluded.nonce, ciphertext=excluded.ciphertext",
                params![profile.id.to_string(), nonce.as_slice(), ciphertext])?;
        } else {
            tx.execute(
                "DELETE FROM credentials WHERE connection_id=?1",
                [profile.id.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_password(&self, id: Uuid) -> Result<Option<String>> {
        let credential = self
            .connection()?
            .query_row(
                "SELECT nonce, ciphertext FROM credentials WHERE connection_id=?1",
                [id.to_string()],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?;
        let Some((nonce, ciphertext)) = credential else {
            return Ok(None);
        };
        let key =
            std::fs::read(self.key_path()).context("Could not read the password encryption key")?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(&key).context("Invalid password encryption key")?;
        decrypt_password(&cipher, id, &nonce, &ciphertext).map(Some)
    }

    fn key_path(&self) -> PathBuf {
        let mut path = self.path.as_os_str().to_os_string();
        path.push(".key");
        PathBuf::from(path)
    }

    // The caller holds SQLite's write lock until the key and credentials are durable.
    fn encryption_cipher(&self, conn: &Connection) -> Result<XChaCha20Poly1305> {
        let credential = conn
            .query_row(
                "SELECT connection_id, nonce, ciphertext FROM credentials LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;
        let path = self.key_path();
        let key = match std::fs::read(&path) {
            Ok(key) => key,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                ensure!(
                    credential.is_none(),
                    "The password encryption key is missing; restore the key file or remove the saved passwords"
                );
                let mut key = vec![0; 32];
                getrandom::fill(&mut key)
                    .context("Could not generate the password encryption key")?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .context("Could not create the password encryption key")?;
                file.write_all(&key)
                    .context("Could not write the password encryption key")?;
                file.sync_all()
                    .context("Could not persist the password encryption key")?;
                let parent = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                File::open(parent)?.sync_all()?;
                key
            }
            Err(error) => return Err(error).context("Could not read the password encryption key"),
        };
        let cipher =
            XChaCha20Poly1305::new_from_slice(&key).context("Invalid password encryption key")?;
        if let Some((id, nonce, ciphertext)) = credential {
            // Check an existing record before writing with a possibly damaged key file.
            decrypt_password(&cipher, Uuid::parse_str(&id)?, &nonce, &ciphertext)?;
        }
        Ok(cipher)
    }

    pub fn delete_profile(&self, id: Uuid) -> Result<()> {
        self.connection()?
            .execute("DELETE FROM connections WHERE id=?1", [id.to_string()])?;
        Ok(())
    }

    pub fn preferences(&self) -> Result<Preferences> {
        let value: Option<String> = self
            .connection()?
            .query_row("SELECT value FROM preferences WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).context("Invalid saved preferences"))
            .unwrap_or_else(|| Ok(Preferences::default()))
    }

    pub fn save_preferences(&self, preferences: &Preferences) -> Result<()> {
        self.connection()?.execute("INSERT INTO preferences(id,value) VALUES (1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value", [serde_json::to_string(preferences)?])?;
        Ok(())
    }

    pub fn record(&self, entry: &HistoryEntry) -> Result<()> {
        self.connection()?.execute("INSERT INTO history(connection_id,database_name,sql,executed_at,elapsed_ms,affected,error) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![entry.connection_id.to_string(), entry.database, entry.sql, entry.executed_at, i64::try_from(entry.elapsed_ms)?, i64::try_from(entry.affected)?, entry.error])?;
        Ok(())
    }

    pub fn history(
        &self,
        connection: Uuid,
        search: &str,
        offset: u64,
    ) -> Result<Vec<HistoryEntry>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare("SELECT id,database_name,sql,executed_at,elapsed_ms,affected,error FROM history WHERE connection_id=?1 AND instr(lower(sql),lower(?2))>0 ORDER BY id DESC LIMIT ?4 OFFSET ?3")?;
        let rows = stmt.query_map(
            params![
                connection.to_string(),
                search,
                i64::try_from(offset)?,
                HISTORY_PAGE_SIZE
            ],
            |row| {
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    connection_id: connection,
                    database: row.get(1)?,
                    sql: row.get(2)?,
                    executed_at: row.get(3)?,
                    elapsed_ms: unsigned(row, 4)?,
                    affected: unsigned(row, 5)?,
                    error: row.get(6)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete_history(&self, id: i64) -> Result<()> {
        self.connection()?
            .execute("DELETE FROM history WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn clear_history(&self, id: Uuid) -> Result<()> {
        self.connection()?.execute(
            "DELETE FROM history WHERE connection_id=?1",
            [id.to_string()],
        )?;
        Ok(())
    }
}

fn write_profile(conn: &Connection, profile: &ConnectionProfile) -> Result<()> {
    conn.execute("INSERT INTO connections(id, profile) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET profile=excluded.profile", params![profile.id.to_string(), serde_json::to_string(profile)?])?;
    Ok(())
}

fn decrypt_password(
    cipher: &XChaCha20Poly1305,
    id: Uuid,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<String> {
    let nonce = XNonce::try_from(nonce).context("Invalid saved password nonce")?;
    let plaintext = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad: id.as_bytes(),
            },
        )
        .map_err(|_| {
            anyhow::anyhow!(
                "Could not decrypt the saved password; the key or encrypted data is damaged"
            )
        })?;
    String::from_utf8(plaintext).context("Invalid saved password text")
}

fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(index)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}
