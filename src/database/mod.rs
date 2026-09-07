mod postgres;
mod sqlite;

use crate::model::*;
use anyhow::{Context, Result, bail};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

pub struct Session {
    backend: Backend,
    pub profile: ConnectionProfile,
}

enum Backend {
    Postgres(Box<postgres::Postgres>),
    Sqlite {
        connection: Arc<Mutex<rusqlite::Connection>>,
        interrupt: rusqlite::InterruptHandle,
    },
}

impl Session {
    pub async fn connect(profile: ConnectionProfile, password: String) -> Result<Arc<Self>> {
        profile.validate()?;
        let backend = match &profile.config {
            ConnectionConfig::Postgres { .. } => Backend::Postgres(Box::new(
                postgres::Postgres::connect(&profile, &password).await?,
            )),
            ConnectionConfig::Sqlite {
                path,
                read_only,
                create,
                wal,
                foreign_keys,
            } => {
                let path = path.clone();
                let (read_only, create, wal, foreign_keys) =
                    (*read_only, *create, *wal, *foreign_keys);
                let connection = tokio::task::spawn_blocking(move || {
                    let flags = if read_only {
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    } else {
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                            | if create {
                                rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                            } else {
                                rusqlite::OpenFlags::empty()
                            }
                    };
                    let conn = rusqlite::Connection::open_with_flags(path, flags)?;
                    conn.busy_timeout(Duration::from_secs(5))?;
                    conn.pragma_update(None, "foreign_keys", foreign_keys)?;
                    if !read_only {
                        conn.pragma_update(
                            None,
                            "journal_mode",
                            if wal { "WAL" } else { "DELETE" },
                        )?;
                    }
                    Ok::<_, anyhow::Error>(conn)
                })
                .await??;
                let interrupt = connection.get_interrupt_handle();
                Backend::Sqlite {
                    connection: Arc::new(Mutex::new(connection)),
                    interrupt,
                }
            }
        };
        Ok(Arc::new(Self { backend, profile }))
    }

    async fn with_sqlite<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut rusqlite::Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let Backend::Sqlite { connection, .. } = &self.backend else {
            bail!("Expected a SQLite session");
        };
        let connection = connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = connection
                .lock()
                .map_err(|_| anyhow::anyhow!("SQLite worker stopped unexpectedly"))?;
            f(&mut connection)
        })
        .await
        .context("SQLite worker failed")?
    }

    pub async fn query(&self, sql: String) -> Result<QueryOutput> {
        let start = Instant::now();
        let sets = match &self.backend {
            Backend::Postgres(pg) => postgres::query(&*pg.client.lock().await, &sql).await?,
            Backend::Sqlite { .. } => {
                self.with_sqlite(move |conn| sqlite::query(conn, &sql))
                    .await?
            }
        };
        Ok(QueryOutput {
            sets,
            elapsed_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    pub async fn cancel(&self) -> Result<()> {
        match &self.backend {
            Backend::Postgres(pg) => pg
                .cancel
                .cancel_query(pg.tls.clone())
                .await
                .context("Could not send the cancellation request"),
            Backend::Sqlite { interrupt, .. } => {
                interrupt.interrupt();
                Ok(())
            }
        }
    }

    pub async fn databases(&self) -> Result<Vec<String>> {
        match &self.backend {
            Backend::Postgres(pg) => postgres::names(&*pg.client.lock().await, "SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname", &[]).await,
            Backend::Sqlite { .. } => Ok(vec![self.profile.database()]),
        }
    }

    pub async fn schemas(&self) -> Result<Vec<String>> {
        match &self.backend {
            Backend::Postgres(pg) => postgres::names(&*pg.client.lock().await, "SELECT nspname FROM pg_namespace WHERE nspname NOT LIKE 'pg_%' AND nspname <> 'information_schema' ORDER BY nspname", &[]).await,
            Backend::Sqlite { .. } => self.with_sqlite(|conn| {
                let mut stmt = conn.prepare("PRAGMA database_list")?;
                Ok(stmt.query_map([], |row| row.get(1))?.collect::<rusqlite::Result<Vec<String>>>()?)
            }).await,
        }
    }

    pub async fn tables(&self, schema: String) -> Result<Vec<TableRef>> {
        match &self.backend {
            Backend::Postgres(pg) => postgres::tables(&*pg.client.lock().await, &schema).await,
            Backend::Sqlite { .. } => {
                self.with_sqlite(move |conn| sqlite::tables(conn, &schema))
                    .await
            }
        }
    }

    pub async fn browse(&self, table: TableRef, options: BrowseOptions) -> Result<TablePage> {
        anyhow::ensure!(
            (1..=1000).contains(&options.limit),
            "Page size must be 1–1000 rows."
        );
        match &self.backend {
            Backend::Postgres(pg) => {
                postgres::browse(&*pg.client.lock().await, &table, &options).await
            }
            Backend::Sqlite { .. } => {
                self.with_sqlite(move |conn| sqlite::browse(conn, &table, &options))
                    .await
            }
        }
    }

    pub async fn change_rows(&self, table: TableRef, change: RowChange) -> Result<u64> {
        anyhow::ensure!(!self.read_only(), "This connection is read-only.");
        match &self.backend {
            Backend::Postgres(pg) => {
                postgres::change_rows(&mut *pg.client.lock().await, &table, change).await
            }
            Backend::Sqlite { .. } => {
                self.with_sqlite(move |conn| sqlite::change_rows(conn, &table, change))
                    .await
            }
        }
    }

    pub async fn database_connections(&self, database: String) -> Result<ResultSet> {
        match &self.backend {
            Backend::Postgres(pg) => {
                postgres::database_connections(&*pg.client.lock().await, &database).await
            }
            _ => bail!("Database administration is only available for PostgreSQL."),
        }
    }

    pub async fn database_action(&self, action: DatabaseAction) -> Result<()> {
        match &self.backend {
            Backend::Postgres(pg) => {
                postgres::database_action(&*pg.client.lock().await, action).await
            }
            _ => bail!("Database administration is only available for PostgreSQL."),
        }
    }

    pub fn read_only(&self) -> bool {
        matches!(
            self.profile.config,
            ConnectionConfig::Sqlite {
                read_only: true,
                ..
            }
        )
    }
}

pub fn runtime() -> Result<Arc<tokio::runtime::Runtime>> {
    Ok(Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()?,
    ))
}

/// Reserve a new SQLite file without opening or overwriting an existing database.
pub async fn create_sqlite_file(path: std::path::PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
            .context("Could not create the SQLite file. Choose an unused path in a writable folder, or use Open existing file")?;
        Ok(())
    }).await?
}

pub(super) fn push_row(set: &mut ResultSet, row: Vec<Cell>, bytes: &mut usize, count: &mut usize) {
    let size: usize = row
        .iter()
        .map(|cell| match cell {
            Cell::Text(value) => value.len(),
            Cell::Blob(value) => value.len(),
            _ => 16,
        })
        .sum();
    if *count < MAX_RESULT_ROWS && size <= MAX_RESULT_BYTES.saturating_sub(*bytes) {
        *bytes += size;
        *count += 1;
        set.rows.push(row);
    } else {
        set.truncated = true;
    }
}

pub(super) fn validate_original(
    metadata: &TableMetadata,
    table: &TableRef,
    original: &[Cell],
) -> Result<()> {
    anyhow::ensure!(
        metadata.editable(table),
        "Editing requires a table with a primary key."
    );
    anyhow::ensure!(
        original.len() == metadata.columns.len(),
        "The table structure changed. Refresh before editing."
    );
    for (column, value) in metadata.columns.iter().zip(original) {
        anyhow::ensure!(
            !column.primary_key || !matches!(value, Cell::Null),
            "Rows with a NULL primary-key component cannot be edited reliably."
        );
    }
    Ok(())
}
