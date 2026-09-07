use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

mod filter;
pub use filter::{FilterKind, FilterOperator, FilterPredicate, TableFilter};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TlsMode {
    Disable,
    Verify,
    Require,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "provider")]
pub enum ConnectionConfig {
    Postgres {
        host: String,
        port: u16,
        database: String,
        user: String,
        tls: TlsMode,
        timeout_seconds: u64,
    },
    Sqlite {
        path: PathBuf,
        read_only: bool,
        create: bool,
        wal: bool,
        foreign_keys: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    pub notes: String,
    pub color: String,
    #[serde(default)]
    pub remember_password: bool,
    pub config: ConnectionConfig,
}

#[derive(Clone, Copy, Debug)]
pub enum ConnectionField {
    Name,
    Color,
    Host,
    Port,
    User,
    Timeout,
    Path,
}

#[derive(Debug)]
pub struct ConnectionValidationError {
    pub field: ConnectionField,
    pub message: &'static str,
}

impl std::fmt::Display for ConnectionValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for ConnectionValidationError {}

impl ConnectionProfile {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty(),
            ConnectionValidationError {
                field: ConnectionField::Name,
                message: "Enter a connection name."
            }
        );
        ensure!(
            self.name.len() <= 255,
            ConnectionValidationError {
                field: ConnectionField::Name,
                message: "Connection name is too long."
            }
        );
        ensure!(
            self.color.is_empty()
                || self
                    .color
                    .strip_prefix('#')
                    .is_some_and(|value| value.len() == 6
                        && value.bytes().all(|byte| byte.is_ascii_hexdigit())),
            ConnectionValidationError {
                field: ConnectionField::Color,
                message: "Enter a color as #RRGGBB, or leave it empty."
            }
        );
        match &self.config {
            ConnectionConfig::Postgres {
                host,
                port,
                user,
                timeout_seconds,
                ..
            } => {
                ensure!(
                    !host.trim().is_empty(),
                    ConnectionValidationError {
                        field: ConnectionField::Host,
                        message: "Enter a PostgreSQL host."
                    }
                );
                ensure!(
                    *port > 0,
                    ConnectionValidationError {
                        field: ConnectionField::Port,
                        message: "Port must be between 1 and 65535."
                    }
                );
                ensure!(
                    !user.trim().is_empty(),
                    ConnectionValidationError {
                        field: ConnectionField::User,
                        message: "Enter a PostgreSQL user."
                    }
                );
                ensure!(
                    (1..=60).contains(timeout_seconds),
                    ConnectionValidationError {
                        field: ConnectionField::Timeout,
                        message: "Timeout must be 1–60 seconds."
                    }
                );
            }
            ConnectionConfig::Sqlite {
                path,
                read_only,
                create,
                ..
            } => {
                ensure!(
                    !path.as_os_str().is_empty(),
                    ConnectionValidationError {
                        field: ConnectionField::Path,
                        message: "Choose a SQLite file."
                    }
                );
                ensure!(
                    !(*read_only && *create),
                    ConnectionValidationError {
                        field: ConnectionField::Path,
                        message: "A read-only connection cannot create a database."
                    }
                );
            }
        }
        Ok(())
    }

    pub fn provider(&self) -> &'static str {
        match self.config {
            ConnectionConfig::Postgres { .. } => "PostgreSQL",
            ConnectionConfig::Sqlite { .. } => "SQLite",
        }
    }

    pub fn database(&self) -> String {
        match &self.config {
            ConnectionConfig::Postgres { database, user, .. } => {
                if database.is_empty() {
                    user.clone()
                } else {
                    database.clone()
                }
            }
            ConnectionConfig::Sqlite { path, .. } => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    pub fn for_database(&self, name: &str) -> Self {
        let mut profile = self.clone();
        if let ConnectionConfig::Postgres { database, .. } = &mut profile.config {
            *database = name.to_owned();
        }
        profile
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Cell {
    Null,
    Text(String),
    Integer(i64),
    Real(f64),
    Blob(Vec<u8>),
}

impl Cell {
    pub fn text(&self) -> String {
        match self {
            Self::Null => "NULL".into(),
            Self::Text(value) => value.clone(),
            Self::Integer(value) => value.to_string(),
            Self::Real(value) => value.to_string(),
            Self::Blob(value) => value.iter().map(|byte| format!("{byte:02x}")).collect(),
        }
    }

    pub fn optional_text(&self) -> Option<String> {
        match self {
            Self::Null => None,
            _ => Some(self.text()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    pub affected: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default)]
pub struct QueryOutput {
    pub sets: Vec<ResultSet>,
    pub elapsed_ms: u64,
}

pub const MAX_RESULT_ROWS: usize = 10_000;
pub const MAX_RESULT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RelationKind {
    #[serde(rename = "table")]
    Table,
    #[serde(rename = "view")]
    View,
    #[serde(rename = "materialized view")]
    MaterializedView,
}

impl std::str::FromStr for RelationKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "table" => Ok(Self::Table),
            "view" => Ok(Self::View),
            "materialized view" => Ok(Self::MaterializedView),
            _ => bail!("Unknown relation kind: {value}"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TableRef {
    pub schema: String,
    pub name: String,
    pub kind: RelationKind,
}

impl TableRef {
    pub fn is_table(&self) -> bool {
        self.kind == RelationKind::Table
    }
}

#[derive(Clone, Debug)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
    pub generated: bool,
}

#[derive(Clone, Debug, Default)]
pub struct TableMetadata {
    pub columns: Vec<Column>,
    pub indexes: ResultSet,
    pub constraints: ResultSet,
}

impl TableMetadata {
    pub fn editable(&self, table: &TableRef) -> bool {
        table.is_table() && self.columns.iter().any(|column| column.primary_key)
    }
}

#[derive(Clone, Debug, Default)]
pub struct BrowseOptions {
    pub offset: u64,
    pub limit: u32,
    pub filter: Option<TableFilter>,
    pub sort: Option<(String, bool)>,
}

#[derive(Clone, Debug)]
pub struct TablePage {
    pub data: ResultSet,
    pub total: u64,
    pub metadata: TableMetadata,
}

#[derive(Clone, Debug)]
pub enum FieldValue {
    Default,
    Null,
    Value(Cell),
}

#[derive(Clone, Debug)]
pub enum RowChange {
    Insert(Vec<FieldValue>),
    Update {
        original: Vec<Cell>,
        values: Vec<FieldValue>,
    },
    Delete {
        originals: Vec<Vec<Cell>>,
    },
}

#[derive(Clone, Debug)]
pub enum DatabaseAction {
    Create {
        name: String,
    },
    Rename {
        source: String,
        target: String,
        force: bool,
    },
    Clone {
        source: String,
        target: String,
        force: bool,
    },
    Delete {
        name: String,
        force: bool,
    },
}

pub const HISTORY_PAGE_SIZE: u16 = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub connection_id: Uuid,
    pub database: String,
    pub sql: String,
    pub executed_at: String,
    pub elapsed_ms: u64,
    pub affected: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub warn_destructive: bool,
    pub appearance: Appearance,
    pub sidebar_width: f32,
    pub window_width: f32,
    pub window_height: f32,
    pub active_connection: Option<Uuid>,
    pub active_document: usize,
    pub drafts: Vec<Draft>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            warn_destructive: true,
            appearance: Appearance::System,
            sidebar_width: 260.,
            window_width: 1280.,
            window_height: 820.,
            active_connection: None,
            active_document: 0,
            drafts: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Draft {
    pub connection_id: Uuid,
    pub database: String,
    pub sql: String,
    #[serde(default)]
    pub query_number: Option<u32>,
    #[serde(default)]
    pub table: Option<TableRef>,
    #[serde(default = "default_editor_height")]
    pub editor_height: f32,
}

pub fn default_editor_height() -> f32 {
    260.
}

pub fn parse_field(
    text: &str,
    column: &Column,
    sqlite: bool,
    original: Option<&Cell>,
) -> Result<Cell> {
    if !sqlite {
        return Ok(Cell::Text(text.to_owned()));
    }
    let kind = column.data_type.to_uppercase();
    if matches!(original, Some(Cell::Blob(_))) || kind.contains("BLOB") {
        ensure!(
            text.len().is_multiple_of(2) && text.is_ascii(),
            "{}: enter an even number of hexadecimal digits.",
            column.name
        );
        let bytes = text
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair)?;
                Ok(u8::from_str_radix(pair, 16)?)
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(Cell::Blob(bytes));
    }
    if kind.contains("INT") || matches!(original, Some(Cell::Integer(_))) {
        return Ok(Cell::Integer(text.parse().map_err(|_| {
            anyhow::anyhow!("{}: enter a signed 64-bit integer.", column.name)
        })?));
    }
    if kind.contains("REAL")
        || kind.contains("FLOA")
        || kind.contains("DOUB")
        || matches!(original, Some(Cell::Real(_)))
    {
        let number: f64 = text.parse()?;
        if !number.is_finite() {
            bail!("{}: enter a finite number.", column.name);
        }
        return Ok(Cell::Real(number));
    }
    Ok(Cell::Text(text.to_owned()))
}
