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
            version <= 6,
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
        if version < 3 {
            tx.execute_batch(
                "CREATE TABLE saved_queries (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                sql TEXT NOT NULL,
                favorite INTEGER NOT NULL DEFAULT 0,
                connection_id TEXT REFERENCES connections(id) ON DELETE SET NULL,
                database_name TEXT NOT NULL,
                revision INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            PRAGMA user_version=3;",
            )?;
        }
        if version < 4 {
            tx.execute_batch(
                "CREATE TABLE saved_table_views (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                connection_id TEXT REFERENCES connections(id) ON DELETE SET NULL,
                database_name TEXT NOT NULL,
                table_ref TEXT NOT NULL,
                settings TEXT NOT NULL,
                relationship TEXT,
                revision INTEGER NOT NULL DEFAULT 1
            ); PRAGMA user_version=4;",
            )?;
        }
        if version < 5 {
            tx.execute_batch("ALTER TABLE saved_queries ADD COLUMN parameters TEXT NOT NULL DEFAULT '[]'; PRAGMA user_version=5;")?;
        }
        if version < 6 {
            tx.execute_batch("ALTER TABLE credentials RENAME TO legacy_credentials;
                CREATE TABLE credentials (
                    connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
                    purpose TEXT NOT NULL, aad_version INTEGER NOT NULL,
                    nonce BLOB NOT NULL, ciphertext BLOB NOT NULL,
                    PRIMARY KEY(connection_id,purpose));
                INSERT INTO credentials SELECT connection_id,'database',0,nonce,ciphertext FROM legacy_credentials;
                DROP TABLE legacy_credentials;
                CREATE TABLE ssh_hosts (endpoint TEXT PRIMARY KEY, algorithm TEXT NOT NULL, fingerprint TEXT NOT NULL);
                PRAGMA user_version=6;")?;
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

    pub fn saved_queries(&self, search: &str, connection: Option<Uuid>) -> Result<Vec<SavedQuery>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT id,name,sql,favorite,connection_id,database_name,revision,parameters
            FROM saved_queries WHERE (?2 IS NULL OR connection_id=?2)
            AND (instr(lower(name),lower(?1))>0 OR instr(lower(sql),lower(?1))>0)
            ORDER BY favorite DESC, name COLLATE NOCASE, id",
        )?;
        let rows = stmt.query_map(
            params![search, connection.map(|id| id.to_string())],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )?;
        rows.map(|row| {
            let (id, name, sql, favorite, connection, database, revision, parameters) = row?;
            Ok(SavedQuery {
                parameters: serde_json::from_str(&parameters)?,
                id: id.parse()?,
                name,
                sql,
                favorite,
                connection_id: connection.map(|id| id.parse()).transpose()?,
                database,
                revision,
            })
        })
        .collect()
    }

    /// Revision zero inserts. Updates require the revision the editor originally loaded.
    pub fn save_query(&self, query: &SavedQuery) -> Result<SavedQuery> {
        ensure!(!query.name.trim().is_empty(), "Enter a query name.");
        let conn = self.connection()?;
        if query.revision == 0 {
            conn.execute(
                "INSERT INTO saved_queries(id,name,sql,favorite,connection_id,database_name,parameters)
                VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    query.id.to_string(),
                    query.name.trim(),
                    query.sql,
                    query.favorite,
                    query.connection_id.map(|id| id.to_string()),
                    query.database,
                    serde_json::to_string(&query.parameters)?
                ],
            )?;
        } else {
            let changed = conn.execute(
                "UPDATE saved_queries SET name=?2,sql=?3,favorite=?4,
                connection_id=?5,database_name=?6,parameters=?8,revision=revision+1,updated_at=CURRENT_TIMESTAMP
                WHERE id=?1 AND revision=?7",
                params![
                    query.id.to_string(),
                    query.name.trim(),
                    query.sql,
                    query.favorite,
                    query.connection_id.map(|id| id.to_string()),
                    query.database,
                    query.revision,
                    serde_json::to_string(&query.parameters)?
                ],
            )?;
            ensure!(
                changed == 1,
                "This saved query was changed or deleted elsewhere. Reopen it from the library or save as a new query."
            );
        }
        let mut saved = query.clone();
        saved.name = saved.name.trim().to_owned();
        saved.revision += 1;
        Ok(saved)
    }

    pub fn delete_query(&self, id: Uuid) -> Result<()> {
        self.connection()?
            .execute("DELETE FROM saved_queries WHERE id=?1", [id.to_string()])?;
        Ok(())
    }

    pub fn rename_query_database(
        &self,
        connection: Uuid,
        source: &str,
        target: &str,
    ) -> Result<()> {
        self.connection()?.execute(
            "UPDATE saved_queries SET database_name=?3,revision=revision+1,
            updated_at=CURRENT_TIMESTAMP WHERE connection_id=?1 AND database_name=?2",
            params![connection.to_string(), source, target],
        )?;
        self.connection()?.execute("UPDATE saved_table_views SET database_name=?3,revision=revision+1 WHERE connection_id=?1 AND database_name=?2", params![connection.to_string(), source, target])?;
        Ok(())
    }

    pub fn saved_table_views(&self) -> Result<Vec<SavedTableView>> {
        let conn = self.connection()?;
        let mut statement = conn.prepare("SELECT id,name,connection_id,database_name,table_ref,settings,relationship,revision FROM saved_table_views ORDER BY name COLLATE NOCASE,id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?;
        rows.map(|row| {
            let (id, name, connection, database, table, settings, relationship, revision) = row?;
            Ok(SavedTableView {
                id: id.parse()?,
                name,
                connection_id: connection.map(|id| id.parse()).transpose()?,
                database,
                table: serde_json::from_str(&table)?,
                settings: serde_json::from_str(&settings)?,
                relationship: relationship
                    .map(|scope| serde_json::from_str(&scope))
                    .transpose()?,
                revision,
            })
        })
        .collect()
    }

    pub fn save_table_view(&self, view: &SavedTableView) -> Result<SavedTableView> {
        ensure!(!view.name.trim().is_empty(), "Enter a view name.");
        ensure!(
            (1..=1000).contains(&view.settings.page_size),
            "Page size must be 1–1000 rows."
        );
        ensure!(
            view.relationship
                .as_ref()
                .is_none_or(|scope| scope.destination() == &view.table),
            "Relationship scope belongs to another table."
        );
        let conn = self.connection()?;
        let table = serde_json::to_string(&view.table)?;
        let settings = serde_json::to_string(&view.settings)?;
        let relationship = view
            .relationship
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let connection = view.connection_id.map(|id| id.to_string());
        if view.revision == 0 {
            conn.execute("INSERT INTO saved_table_views(id,name,connection_id,database_name,table_ref,settings,relationship) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![view.id.to_string(), view.name.trim(), connection, view.database, table, settings, relationship])?;
        } else {
            let changed = conn.execute("UPDATE saved_table_views SET name=?2,settings=?6,relationship=?7,revision=revision+1 WHERE id=?1 AND revision=?8 AND connection_id IS ?3 AND database_name=?4 AND table_ref=?5", params![view.id.to_string(), view.name.trim(), connection, view.database, table, settings, relationship, view.revision])?;
            ensure!(
                changed == 1,
                "This saved view changed, was deleted, or its association changed elsewhere. Reopen it from Saved views, or use Save as. Your current settings are retained."
            );
        }
        let mut saved = view.clone();
        saved.name = saved.name.trim().into();
        saved.revision += 1;
        Ok(saved)
    }

    pub fn delete_table_view(&self, id: Uuid, revision: i64) -> Result<()> {
        let changed = self.connection()?.execute(
            "DELETE FROM saved_table_views WHERE id=?1 AND revision=?2",
            params![id.to_string(), revision],
        )?;
        ensure!(
            changed == 1,
            "This saved view changed or was deleted elsewhere. Reopen Saved views."
        );
        Ok(())
    }

    pub fn profiles(&self) -> Result<Vec<ConnectionProfile>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare("SELECT profile FROM connections ORDER BY json_extract(profile, '$.name') COLLATE NOCASE")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn save_profile(&self, profile: &ConnectionProfile) -> Result<()> {
        profile.validate()?;
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT profile FROM connections WHERE id=?1",
                [profile.id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let previous = previous
            .map(|p| serde_json::from_str::<ConnectionProfile>(&p))
            .transpose()?;
        if !profile.ssh.as_ref().is_some_and(|s| s.remember_passphrase)
            || previous
                .as_ref()
                .and_then(|p| p.ssh.as_ref())
                .map(|s| &s.auth)
                != profile.ssh.as_ref().map(|s| &s.auth)
        {
            tx.execute(
                "DELETE FROM credentials WHERE connection_id=?1 AND purpose='ssh-passphrase'",
                [profile.id.to_string()],
            )?;
        }
        write_profile(&tx, profile)?;
        tx.commit()?;
        Ok(())
    }

    pub fn save_connection(&self, profile: &ConnectionProfile, password: &str) -> Result<()> {
        self.save_connection_with_ssh(profile, password, None)
    }

    pub fn save_connection_with_ssh(
        &self,
        profile: &ConnectionProfile,
        password: &str,
        passphrase: Option<&str>,
    ) -> Result<()> {
        profile.validate()?;
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT profile FROM connections WHERE id=?1",
                [profile.id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let previous = previous
            .map(|p| serde_json::from_str::<ConnectionProfile>(&p))
            .transpose()?;
        write_profile(&tx, profile)?;
        self.write_credential(
            &tx,
            profile.id,
            "database",
            if profile.remember_password
                && matches!(profile.config, ConnectionConfig::Postgres { .. })
            {
                Some(password)
            } else {
                None
            },
        )?;
        let remember = profile.ssh.as_ref().is_some_and(|ssh| {
            ssh.remember_passphrase && matches!(ssh.auth, SshAuth::PrivateKey { .. })
        });
        if !remember
            || previous
                .as_ref()
                .and_then(|p| p.ssh.as_ref())
                .map(|s| &s.auth)
                != profile.ssh.as_ref().map(|s| &s.auth)
        {
            tx.execute(
                "DELETE FROM credentials WHERE connection_id=?1 AND purpose='ssh-passphrase'",
                [profile.id.to_string()],
            )?;
        }
        if remember && let Some(passphrase) = passphrase {
            self.write_credential(&tx, profile.id, "ssh-passphrase", Some(passphrase))?;
        }
        tx.commit()?;
        Ok(())
    }

    fn write_credential(
        &self,
        conn: &Connection,
        id: Uuid,
        purpose: &str,
        value: Option<&str>,
    ) -> Result<()> {
        if let Some(value) = value {
            let cipher = self.encryption_cipher(conn)?;
            let mut nonce = XNonce::default();
            getrandom::fill(&mut nonce).context("Could not generate a credential nonce")?;
            let aad = credential_aad(id, purpose);
            let ciphertext = cipher
                .encrypt(
                    &nonce,
                    Payload {
                        msg: value.as_bytes(),
                        aad: &aad,
                    },
                )
                .map_err(|_| anyhow::anyhow!("Could not encrypt the credential"))?;
            conn.execute("INSERT INTO credentials(connection_id,purpose,aad_version,nonce,ciphertext) VALUES(?1,?2,1,?3,?4)
                ON CONFLICT(connection_id,purpose) DO UPDATE SET aad_version=1,nonce=excluded.nonce,ciphertext=excluded.ciphertext",
                params![id.to_string(),purpose,nonce.as_slice(),ciphertext])?;
        } else {
            conn.execute(
                "DELETE FROM credentials WHERE connection_id=?1 AND purpose=?2",
                params![id.to_string(), purpose],
            )?;
        }
        Ok(())
    }

    pub fn load_password(&self, id: Uuid) -> Result<Option<String>> {
        self.load_credential(id, "database")
    }

    pub fn remember_ssh_passphrase(
        &self,
        profile: &ConnectionProfile,
        passphrase: &str,
    ) -> Result<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual: Option<String> = tx
            .query_row(
                "SELECT profile FROM connections WHERE id=?1",
                [profile.id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(actual) = actual {
            let actual: ConnectionProfile = serde_json::from_str(&actual)?;
            ensure!(
                actual.ssh == profile.ssh
                    && actual.ssh.as_ref().is_some_and(|s| s.remember_passphrase),
                "SSH credentials changed while unlocking the key; retry using current settings"
            );
            self.write_credential(&tx, profile.id, "ssh-passphrase", Some(passphrase))?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn clear_ssh_passphrase(&self, id: Uuid) -> Result<()> {
        self.connection()?.execute(
            "DELETE FROM credentials WHERE connection_id=?1 AND purpose='ssh-passphrase'",
            [id.to_string()],
        )?;
        Ok(())
    }

    pub fn load_ssh_passphrase(&self, id: Uuid) -> Result<Option<String>> {
        self.load_credential(id, "ssh-passphrase")
    }

    fn load_credential(&self, id: Uuid, purpose: &str) -> Result<Option<String>> {
        let credential = self.connection()?.query_row(
            "SELECT nonce,ciphertext,aad_version FROM credentials WHERE connection_id=?1 AND purpose=?2",
            params![id.to_string(),purpose], |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, u32>(2)?))).optional()?;
        let Some((nonce, ciphertext, version)) = credential else {
            return Ok(None);
        };
        let key =
            std::fs::read(self.key_path()).context("Could not read the password encryption key")?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(&key).context("Invalid password encryption key")?;
        let plaintext = decrypt_credential(&cipher, id, purpose, version, &nonce, &ciphertext)?;
        if version == 0 {
            let mut upgraded_nonce = XNonce::default();
            getrandom::fill(&mut upgraded_nonce)
                .context("Could not generate a credential nonce")?;
            let aad = credential_aad(id, purpose);
            let upgraded = cipher
                .encrypt(
                    &upgraded_nonce,
                    Payload {
                        msg: plaintext.as_bytes(),
                        aad: &aad,
                    },
                )
                .map_err(|_| anyhow::anyhow!("Could not migrate the saved credential"))?;
            // A concurrent save wins over migration of the older snapshot.
            self.connection()?.execute("UPDATE credentials SET aad_version=1,nonce=?3,ciphertext=?4 WHERE connection_id=?1 AND purpose=?2 AND aad_version=0 AND nonce=?5 AND ciphertext=?6",
                params![id.to_string(), purpose, upgraded_nonce.as_slice(), upgraded, nonce, ciphertext])?;
        }
        Ok(Some(plaintext))
    }

    pub fn ssh_host(&self, endpoint: &str) -> Result<Option<(String, String)>> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT algorithm,fingerprint FROM ssh_hosts WHERE endpoint=?1",
                [endpoint],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn save_ssh_host(
        &self,
        endpoint: &str,
        algorithm: &str,
        fingerprint: &str,
        previous: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual: Option<String> = tx
            .query_row(
                "SELECT fingerprint FROM ssh_hosts WHERE endpoint=?1",
                [endpoint],
                |r| r.get(0),
            )
            .optional()?;
        ensure!(
            actual.as_deref() == previous,
            "SSH host trust changed during this attempt. Retry verification."
        );
        tx.execute("INSERT INTO ssh_hosts(endpoint,algorithm,fingerprint) VALUES(?1,?2,?3) ON CONFLICT(endpoint) DO UPDATE SET algorithm=excluded.algorithm,fingerprint=excluded.fingerprint", params![endpoint,algorithm,fingerprint])?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_ssh_host(&self, endpoint: &str) -> Result<()> {
        self.connection()?
            .execute("DELETE FROM ssh_hosts WHERE endpoint=?1", [endpoint])?;
        Ok(())
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
                "SELECT connection_id, nonce, ciphertext, purpose, aad_version FROM credentials LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, u32>(4)?,
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
        if let Some((id, nonce, ciphertext, purpose, version)) = credential {
            // Check an existing record before writing with a possibly damaged key file.
            decrypt_credential(
                &cipher,
                Uuid::parse_str(&id)?,
                &purpose,
                version,
                &nonce,
                &ciphertext,
            )?;
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

fn credential_aad(id: Uuid, purpose: &str) -> Vec<u8> {
    [id.as_bytes().as_slice(), b"\0queri:", purpose.as_bytes()].concat()
}

fn decrypt_credential(
    cipher: &XChaCha20Poly1305,
    id: Uuid,
    purpose: &str,
    version: u32,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<String> {
    let nonce = XNonce::try_from(nonce).context("Invalid saved password nonce")?;
    ensure!(
        version <= 1 && (version != 0 || purpose == "database"),
        "Invalid credential format"
    );
    let aad = if version == 0 {
        id.as_bytes().to_vec()
    } else {
        credential_aad(id, purpose)
    };
    let plaintext = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad: &aad,
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

#[cfg(test)]
mod saved_query_tests {
    use super::*;
    #[test]
    fn saved_queries_detect_stale_updates_and_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app");
        let store = Store::open(&path).unwrap();
        let query = SavedQuery {
            parameters: vec![],
            id: Uuid::new_v4(),
            name: " Test ".into(),
            sql: "SELECT 1".into(),
            favorite: false,
            connection_id: None,
            database: String::new(),
            revision: 0,
        };
        let first = store.save_query(&query).unwrap();
        assert_eq!(first.name, "Test");
        let mut second = first.clone();
        second.sql = "SELECT 2".into();
        store.save_query(&second).unwrap();
        assert!(store.save_query(&first).is_err());
        let entries = Store::open(path)
            .unwrap()
            .saved_queries("select 2", None)
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].revision, 2);
        store.delete_query(first.id).unwrap();
        assert!(store.saved_queries("", None).unwrap().is_empty());
    }
}
