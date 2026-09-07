use crate::model::*;
use anyhow::{Context, Result, ensure};
use directories::ProjectDirs;
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
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
        let version: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        ensure!(
            version <= 1,
            "This application database was created by a newer Queri version."
        );
        if version == 0 {
            let tx = conn.transaction()?;
            tx.execute_batch("CREATE TABLE connections (id TEXT PRIMARY KEY, profile TEXT NOT NULL);
                CREATE TABLE preferences (id INTEGER PRIMARY KEY CHECK (id=1), value TEXT NOT NULL);
                CREATE TABLE history (id INTEGER PRIMARY KEY, connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
                    database_name TEXT NOT NULL, sql TEXT NOT NULL, executed_at TEXT NOT NULL, elapsed_ms INTEGER NOT NULL, affected INTEGER NOT NULL, error TEXT);
                CREATE INDEX history_connection_time ON history(connection_id, id DESC);
                PRAGMA user_version=1;")?;
            tx.commit()?;
        }
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
        self.connection()?.execute("INSERT INTO connections(id, profile) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET profile=excluded.profile", params![profile.id.to_string(), serde_json::to_string(profile)?])?;
        Ok(())
    }

    pub fn delete_profile(&self, id: Uuid) -> Result<()> {
        self.connection()?
            .execute("DELETE FROM connections WHERE id=?1", [id.to_string()])?;
        Ok(())
    }

    pub fn preferences(&self) -> Result<Preferences> {
        use rusqlite::OptionalExtension;
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
        let mut stmt = conn.prepare("SELECT id,database_name,sql,executed_at,elapsed_ms,affected,error FROM history WHERE connection_id=?1 AND instr(lower(sql),lower(?2))>0 ORDER BY id DESC LIMIT 100 OFFSET ?3")?;
        let rows = stmt.query_map(
            params![connection.to_string(), search, i64::try_from(offset)?],
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

fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(index)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

pub fn save_password(id: Uuid, password: &str) -> Result<()> {
    keyring::Entry::new("dev.queri.Queri", &id.to_string())?.set_password(password).context("Could not save the password in Secret Service. Unlock your keyring, or turn off Remember password.")
}

pub fn load_password(id: Uuid) -> Result<Option<String>> {
    match keyring::Entry::new("dev.queri.Queri", &id.to_string())?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => {
            Err(error).context("Could not read the saved password. Unlock your keyring and retry.")
        }
    }
}

pub fn delete_password(id: Uuid) -> Result<()> {
    match keyring::Entry::new("dev.queri.Queri", &id.to_string())?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => {
            Err(error).context("Could not remove the saved password from Secret Service.")
        }
    }
}
