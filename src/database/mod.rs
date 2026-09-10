mod postgres;
mod sqlite;

use crate::model::*;
use crate::sql::quote_identifier;
use anyhow::{Context, Result, bail};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

pub struct Session {
    backend: Backend,
    pub profile: ConnectionProfile,
    timeout: Mutex<StatementTimeout>,
    read_write_observed: AtomicBool,
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
        Ok(Arc::new(Self {
            backend,
            profile,
            timeout: Mutex::new(StatementTimeout::Inherit),
            read_write_observed: AtomicBool::new(false),
        }))
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

    /// Changes the policy used by the next execution; never sends SQL into a failed transaction.
    pub fn set_statement_timeout(&self, timeout: StatementTimeout) -> Result<()> {
        if let StatementTimeout::Custom(ms) = timeout {
            StatementTimeout::custom(ms.get())?;
        }
        *self.timeout.lock().unwrap() = timeout;
        Ok(())
    }

    pub async fn query(&self, sql: String) -> Result<QueryOutput> {
        let start = Instant::now();
        let timeout = *self.timeout.lock().unwrap();
        let sets = match &self.backend {
            Backend::Postgres(pg) => {
                let client = pg.client.lock().await;
                pg.apply_timeout(&client, timeout, &sql)
                    .await
                    .map_err(classify_postgres_error)?;
                let result = postgres::query(&client, &sql).await;
                self.observe_read_only(&client).await;
                result.map_err(classify_postgres_error)?
            }
            Backend::Sqlite { .. } => {
                self.with_sqlite(move |conn| sqlite::query(conn, &sql, timeout))
                    .await?
            }
        };
        Ok(QueryOutput {
            sets,
            elapsed_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    /// Waits for prior session work and observes the backend's transaction state.
    /// PostgreSQL uses protocol Sync/ReadyForQuery, even after SQL errors.
    /// A failed connection returns Unknown; this never reconnects implicitly.
    pub async fn transaction_status(&self) -> Result<TransactionStatus> {
        match &self.backend {
            Backend::Postgres(pg) => Ok(match pg.client.lock().await.transaction_status().await {
                Ok(b'I') => TransactionStatus::Idle,
                Ok(b'T') => TransactionStatus::Open,
                Ok(b'E') => TransactionStatus::Failed,
                _ => TransactionStatus::Unknown,
            }),
            Backend::Sqlite { .. } => {
                self.with_sqlite(|conn| {
                    Ok(if conn.is_autocommit() {
                        TransactionStatus::Idle
                    } else {
                        TransactionStatus::Open
                    })
                })
                .await
            }
        }
    }

    /// Executes one PostgreSQL statement with positional text bind values.
    /// None is SQL NULL, Some("") is empty text. Types are inferred by PostgreSQL;
    /// use SQL casts for ambiguous parameters. Results retain exact server text
    /// and the same display limits as query(). SQLite is deliberately unsupported.
    pub async fn query_postgres_params(
        &self,
        sql: String,
        params: Vec<Option<String>>,
    ) -> Result<QueryOutput> {
        let Backend::Postgres(pg) = &self.backend else {
            bail!("Text bind execution requires a PostgreSQL session");
        };
        let start = Instant::now();
        let client = pg.client.lock().await;
        let timeout = *self.timeout.lock().unwrap();
        pg.apply_timeout(&client, timeout, &sql)
            .await
            .map_err(classify_postgres_error)?;
        let params: Vec<_> = params.iter().map(|value| value.as_deref()).collect();
        let stream = client
            .query_text_raw(&sql, &params)
            .await
            .map_err(|e| classify_postgres_error(e.into()))?;
        let result = postgres::collect_results(stream).await;
        self.observe_read_only(&client).await;
        let sets = result.map_err(classify_postgres_error)?;
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

    pub async fn foreign_keys(&self, table: TableRef, incoming: bool) -> Result<Vec<ForeignKey>> {
        match &self.backend {
            Backend::Postgres(pg) => {
                postgres::foreign_keys(&*pg.client.lock().await, &table, incoming).await
            }
            Backend::Sqlite { .. } => {
                self.with_sqlite(move |conn| sqlite::foreign_keys(conn, &table, incoming))
                    .await
            }
        }
    }

    pub async fn table_metadata(&self, table: TableRef) -> Result<TableMetadata> {
        match &self.backend {
            Backend::Postgres(pg) => postgres::metadata(&*pg.client.lock().await, &table).await,
            Backend::Sqlite { .. } => {
                self.with_sqlite(move |conn| sqlite::metadata(conn, &table))
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
        anyhow::ensure!(
            !self.read_only(),
            "Connection policy blocks generated row changes."
        );
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
        anyhow::ensure!(
            !self.read_only(),
            "Connection policy blocks database administration."
        );
        match &self.backend {
            Backend::Postgres(pg) => {
                postgres::database_action(&*pg.client.lock().await, action).await
            }
            _ => bail!("Database administration is only available for PostgreSQL."),
        }
    }

    /// Whether generated changes are blocked, independently of raw SQL overrides.
    pub fn read_only(&self) -> bool {
        self.profile.blocks_mutations()
    }

    pub fn read_write_observed(&self) -> bool {
        self.read_write_observed.load(Ordering::Relaxed)
    }

    async fn observe_read_only(&self, client: &tokio_postgres::Client) {
        if self.profile.policy != ExecutionPolicy::ReadOnlyByDefault {
            return;
        }
        // SHOW does not acquire a data snapshot or interfere with transaction setup.
        // Never inject inspection SQL into an aborted transaction.
        if matches!(client.transaction_status().await, Ok(b'I' | b'T'))
            && let Ok(messages) = client
                .simple_query("SHOW default_transaction_read_only; SHOW transaction_read_only")
                .await
        {
            let read_write = messages.iter().any(|message| {
                matches!(message, tokio_postgres::SimpleQueryMessage::Row(row) if row.get(0) == Some("off"))
            });
            self.read_write_observed
                .store(read_write, Ordering::Relaxed);
        }
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

fn browse_ordering(metadata: &TableMetadata, sort: &[(String, bool)]) -> Result<String> {
    let mut columns = Vec::new();
    for (column, descending) in sort {
        anyhow::ensure!(
            metadata.columns.iter().any(|col| &col.name == column),
            "Sort column ‘{column}’ no longer exists. Remove or replace it."
        );
        anyhow::ensure!(
            !columns.iter().any(|(name, _)| name == column),
            "Duplicate sort column: {column}"
        );
        columns.push((column.clone(), *descending));
    }
    for column in metadata.columns.iter().filter(|col| col.primary_key) {
        if !columns.iter().any(|(name, _)| name == &column.name) {
            columns.push((column.name.clone(), false));
        }
    }
    let terms = columns
        .iter()
        .map(|(name, descending)| {
            Ok(format!(
                "{} {}",
                quote_identifier(name)?,
                if *descending { "DESC" } else { "ASC" }
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(if terms.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", terms.join(","))
    })
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

#[derive(Debug)]
pub struct StatementTimedOut;
impl std::fmt::Display for StatementTimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Statement timed out")
    }
}
impl std::error::Error for StatementTimedOut {}

pub fn is_statement_timeout(error: &anyhow::Error) -> bool {
    error.downcast_ref::<StatementTimedOut>().is_some()
}

fn classify_postgres_error(error: anyhow::Error) -> anyhow::Error {
    if error
        .downcast_ref::<tokio_postgres::Error>()
        .and_then(|e| e.as_db_error())
        .is_some_and(|e| {
            e.code() == &tokio_postgres::error::SqlState::QUERY_CANCELED
                && e.message().contains("statement timeout")
        })
    {
        error.context(StatementTimedOut)
    } else {
        error
    }
}
