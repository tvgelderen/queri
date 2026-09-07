use super::{AsyncMutex, push_row, validate_original};
use crate::{
    model::*,
    sql::{qualified, quote_identifier},
};
use anyhow::{Context, Result, ensure};
use futures::TryStreamExt;
use postgres_native_tls::MakeTlsConnector;
use std::time::Duration;
use tokio_postgres::{Client, GenericClient, SimpleQueryMessage, types::ToSql};

pub struct Postgres {
    pub client: AsyncMutex<Client>,
    pub cancel: tokio_postgres::CancelToken,
    pub tls: MakeTlsConnector,
    driver: tokio::task::JoinHandle<()>,
}

impl Drop for Postgres {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

impl Postgres {
    pub async fn connect(profile: &ConnectionProfile, password: &str) -> Result<Self> {
        let ConnectionConfig::Postgres {
            host,
            port,
            database,
            user,
            tls,
            timeout_seconds,
        } = &profile.config
        else {
            anyhow::bail!("Expected PostgreSQL configuration");
        };
        let mut config = tokio_postgres::Config::new();
        config
            .host(host)
            .port(*port)
            .user(user)
            .password(password)
            .application_name("Queri")
            .connect_timeout(Duration::from_secs(*timeout_seconds));
        if !database.is_empty() {
            config.dbname(database);
        }
        config.ssl_mode(if *tls == TlsMode::Disable {
            tokio_postgres::config::SslMode::Disable
        } else {
            tokio_postgres::config::SslMode::Require
        });
        let mut builder = native_tls::TlsConnector::builder();
        if *tls == TlsMode::Require {
            builder
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
        }
        let connector = MakeTlsConnector::new(builder.build()?);
        let (client, connection) = tokio::time::timeout(
            Duration::from_secs(*timeout_seconds),
            config.connect(connector.clone()),
        )
        .await
        .context("Connection timed out")??;
        let driver = tokio::spawn(async move {
            if let Err(error) = connection.await {
                eprintln!("PostgreSQL connection closed: {error}");
            }
        });
        let cancel = client.cancel_token();
        Ok(Self {
            client: AsyncMutex::new(client),
            cancel,
            tls: connector,
            driver,
        })
    }
}

/// The simple protocol preserves exact server text for arbitrary extension types, numerics,
/// arrays and timestamps. Table metadata supplies types to the row inspector.
pub async fn query(client: &Client, sql: &str) -> Result<Vec<ResultSet>> {
    let stream = client.simple_query_raw(sql).await?;
    futures::pin_mut!(stream);
    let mut sets = vec![];
    let mut current = ResultSet::default();
    let (mut bytes, mut count) = (0, 0);
    while let Some(message) = stream.try_next().await? {
        match message {
            SimpleQueryMessage::RowDescription(columns) => {
                current.columns = columns
                    .iter()
                    .map(|column| column.name().to_owned())
                    .collect();
            }
            SimpleQueryMessage::Row(row) => {
                let values = (0..row.len())
                    .map(|index| {
                        row.try_get(index).map(|value| {
                            value
                                .map(|value| Cell::Text(value.to_owned()))
                                .unwrap_or(Cell::Null)
                        })
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                push_row(&mut current, values, &mut bytes, &mut count);
            }
            SimpleQueryMessage::CommandComplete(affected) => {
                current.affected = affected;
                sets.push(std::mem::take(&mut current));
            }
            _ => {}
        }
    }
    Ok(sets)
}

pub async fn names(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<String>> {
    Ok(client
        .query(sql, params)
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect())
}

pub async fn tables(client: &Client, schema: &str) -> Result<Vec<TableRef>> {
    Ok(client.query("SELECT c.relname, CASE c.relkind WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized view' ELSE 'table' END FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind IN ('r','p','v','m','f') ORDER BY c.relname", &[&schema]).await?.iter().map(|row| TableRef { schema: schema.into(), name: row.get(0), kind: row.get(1) }).collect())
}

async fn metadata(client: &impl GenericClient, table: &TableRef) -> Result<TableMetadata> {
    let rows = client.query("SELECT a.attname, format_type(a.atttypid,a.atttypmod), NOT a.attnotnull, pg_get_expr(d.adbin,d.adrelid), EXISTS(SELECT 1 FROM pg_index i WHERE i.indrelid=c.oid AND i.indisprimary AND a.attnum=ANY(i.indkey)), a.attgenerated <> '' OR a.attidentity='a' FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_attribute a ON a.attrelid=c.oid LEFT JOIN pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum WHERE n.nspname=$1 AND c.relname=$2 AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum", &[&table.schema, &table.name]).await?;
    let columns = rows
        .iter()
        .map(|row| Column {
            name: row.get(0),
            data_type: row.get(1),
            nullable: row.get(2),
            default: row.get(3),
            primary_key: row.get(4),
            generated: row.get(5),
        })
        .collect::<Vec<_>>();
    ensure!(
        !columns.is_empty(),
        "The table no longer exists or is inaccessible."
    );
    let rows = client.query("SELECT indexname,indexdef FROM pg_indexes WHERE schemaname=$1 AND tablename=$2 ORDER BY indexname", &[&table.schema, &table.name]).await?;
    let indexes = ResultSet {
        columns: vec!["Name".into(), "Definition".into()],
        rows: rows
            .iter()
            .map(|row| vec![Cell::Text(row.get(0)), Cell::Text(row.get(1))])
            .collect(),
        ..Default::default()
    };
    let rows = client.query("SELECT con.conname,pg_get_constraintdef(con.oid) FROM pg_constraint con JOIN pg_class c ON c.oid=con.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2 ORDER BY con.conname", &[&table.schema, &table.name]).await?;
    let constraints = ResultSet {
        columns: vec!["Name".into(), "Definition".into()],
        rows: rows
            .iter()
            .map(|row| vec![Cell::Text(row.get(0)), Cell::Text(row.get(1))])
            .collect(),
        ..Default::default()
    };
    Ok(TableMetadata {
        columns,
        indexes,
        constraints,
    })
}

pub async fn browse(
    client: &Client,
    table: &TableRef,
    options: &BrowseOptions,
) -> Result<TablePage> {
    let metadata = metadata(client, table).await?;
    let table_name = qualified(&table.schema, &table.name)?;
    let mut params: Vec<&(dyn ToSql + Sync)> = vec![];
    let text = options
        .filter
        .as_ref()
        .and_then(|filter| filter.predicate.value());
    let filter = match &options.filter {
        Some(filter) => {
            let column = filter.validate(&metadata, false)?;
            let name = quote_identifier(&column.name)?;
            if filter.predicate.value().is_some() {
                params.push(&text);
            }
            let (left, right) = match column.filter_kind(false) {
                FilterKind::Number => {
                    let base = column.data_type.split('(').next().unwrap_or("numeric");
                    let native_type = match base {
                        "smallint" => "smallint",
                        "integer" => "integer",
                        "bigint" => "bigint",
                        "real" => "real",
                        "double precision" => "double precision",
                        _ => "numeric",
                    };
                    (name.clone(), format!("$1::text::{native_type}"))
                }
                FilterKind::Date => {
                    let native_type = if column.data_type == "date" {
                        "date"
                    } else if column.data_type.contains("with time zone") {
                        "timestamp with time zone"
                    } else {
                        "timestamp without time zone"
                    };
                    (name.clone(), format!("$1::text::{native_type}"))
                }
                FilterKind::Boolean => (name.clone(), "$1::text::boolean".into()),
                FilterKind::Text => (format!("lower({name}::text)"), "lower($1::text)".into()),
            };
            let predicate = match &filter.predicate {
                FilterPredicate::Contains(_) => format!("strpos({left},{right}) > 0"),
                FilterPredicate::NotContains(_) => format!("strpos({left},{right}) = 0"),
                FilterPredicate::Equals(_) => format!("{left} = {right}"),
                FilterPredicate::NotEquals(_) => format!("{left} <> {right}"),
                FilterPredicate::StartsWith(_) => format!("starts_with({left},{right})"),
                FilterPredicate::EndsWith(_) => format!("right({left},length({right})) = {right}"),
                FilterPredicate::GreaterThan(_) => format!("{left} > {right}"),
                FilterPredicate::GreaterOrEqual(_) => format!("{left} >= {right}"),
                FilterPredicate::LessThan(_) => format!("{left} < {right}"),
                FilterPredicate::LessOrEqual(_) => format!("{left} <= {right}"),
                FilterPredicate::IsNull => format!("{name} IS NULL"),
                FilterPredicate::IsNotNull => format!("{name} IS NOT NULL"),
            };
            format!(" WHERE {predicate}")
        }
        None => String::new(),
    };
    let sort = match &options.sort {
        Some((column, descending)) => {
            ensure!(
                metadata.columns.iter().any(|col| &col.name == column),
                "Unknown sort column."
            );
            format!(
                " ORDER BY {} {}",
                quote_identifier(column)?,
                if *descending { "DESC" } else { "ASC" }
            )
        }
        None => {
            let keys = metadata
                .columns
                .iter()
                .filter(|col| col.primary_key)
                .map(|col| quote_identifier(&col.name))
                .collect::<Result<Vec<_>>>()?;
            if keys.is_empty() {
                String::new()
            } else {
                format!(" ORDER BY {}", keys.join(","))
            }
        }
    };
    let count: i64 = client
        .query_one(
            &format!("SELECT count(*) FROM {table_name}{filter}"),
            &params,
        )
        .await
        .context("Could not load rows. Check the filter operator and value.")?
        .get(0);
    let columns = metadata
        .columns
        .iter()
        .map(|col| Ok(format!("{}::text", quote_identifier(&col.name)?)))
        .collect::<Result<Vec<_>>>()?
        .join(",");
    let rows = client
        .query(
            &format!(
                "SELECT {columns} FROM {table_name}{filter}{sort} LIMIT {} OFFSET {}",
                options.limit, options.offset
            ),
            &params,
        )
        .await?;
    let mut data = ResultSet {
        columns: metadata
            .columns
            .iter()
            .map(|col| col.name.clone())
            .collect(),
        affected: u64::try_from(rows.len())?,
        ..Default::default()
    };
    let (mut bytes, mut row_count) = (0, 0);
    for row in rows {
        let values = (0..data.columns.len())
            .map(|index| {
                row.get::<_, Option<String>>(index)
                    .map(Cell::Text)
                    .unwrap_or(Cell::Null)
            })
            .collect();
        push_row(&mut data, values, &mut bytes, &mut row_count);
    }
    Ok(TablePage {
        data,
        total: u64::try_from(count)?,
        metadata,
    })
}

fn field_json(field: &FieldValue) -> serde_json::Value {
    match field {
        FieldValue::Value(cell) => cell
            .optional_text()
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
        _ => serde_json::Value::Null,
    }
}

fn predicate(metadata: &TableMetadata, start: usize) -> Result<String> {
    metadata
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            Ok(format!(
                "{}::text IS NOT DISTINCT FROM ${}::text",
                quote_identifier(&column.name)?,
                start + index
            ))
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join(" AND "))
}

pub async fn change_rows(client: &mut Client, table: &TableRef, change: RowChange) -> Result<u64> {
    let tx = client.transaction().await?;
    let metadata = metadata(&tx, table).await?;
    ensure!(
        table.kind == "table",
        "Views cannot be edited in the row inspector."
    );
    let table_name = qualified(&table.schema, &table.name)?;
    let affected = match change {
        RowChange::Insert(fields) => {
            ensure!(
                fields.len() == metadata.columns.len(),
                "The table structure changed. Refresh before inserting."
            );
            let mut names = vec![];
            let mut object = serde_json::Map::new();
            for (column, field) in metadata.columns.iter().zip(fields) {
                if column.generated || matches!(field, FieldValue::Default) {
                    continue;
                }
                names.push(quote_identifier(&column.name)?);
                object.insert(column.name.clone(), field_json(&field));
            }
            if names.is_empty() {
                tx.execute(&format!("INSERT INTO {table_name} DEFAULT VALUES"), &[])
                    .await?
            } else {
                let json = serde_json::Value::Object(object);
                tx.execute(&format!("INSERT INTO {table_name} ({}) SELECT {} FROM json_populate_record(NULL::{table_name},$1::json)", names.join(","), names.join(",")), &[&json]).await?
            }
        }
        RowChange::Update { original, values } => {
            validate_original(&metadata, table, &original)?;
            ensure!(
                values.len() == metadata.columns.len(),
                "The table structure changed. Refresh before editing."
            );
            let mut object = serde_json::Map::new();
            let mut assignments = vec![];
            for (column, field) in metadata.columns.iter().zip(values) {
                if column.generated {
                    continue;
                }
                let name = quote_identifier(&column.name)?;
                if matches!(field, FieldValue::Default) {
                    assignments.push(format!("{name}=DEFAULT"));
                } else {
                    object.insert(column.name.clone(), field_json(&field));
                    assignments.push(format!(
                        "{name}=(json_populate_record(NULL::{table_name},$1::json)).{name}"
                    ));
                }
            }
            let json = serde_json::Value::Object(object);
            let old = original.iter().map(Cell::optional_text).collect::<Vec<_>>();
            let mut params: Vec<&(dyn ToSql + Sync)> = vec![&json];
            for cell in &old {
                params.push(cell);
            }
            // The harmless JSON predicate keeps parameter 1 typed even when every field uses DEFAULT.
            let affected = tx
                .execute(
                    &format!(
                        "UPDATE {table_name} SET {} WHERE $1::json IS NOT NULL AND {}",
                        assignments.join(","),
                        predicate(&metadata, 2)?
                    ),
                    &params,
                )
                .await?;
            ensure!(
                affected == 1,
                "The row changed or was deleted. Refresh and retry; no changes were saved."
            );
            affected
        }
        RowChange::Delete { originals } => {
            ensure!(!originals.is_empty(), "Select at least one row.");
            let mut total = 0;
            for original in originals {
                validate_original(&metadata, table, &original)?;
                let old = original.iter().map(Cell::optional_text).collect::<Vec<_>>();
                let mut params: Vec<&(dyn ToSql + Sync)> = vec![];
                for cell in &old {
                    params.push(cell);
                }
                let affected = tx
                    .execute(
                        &format!(
                            "DELETE FROM {table_name} WHERE {}",
                            predicate(&metadata, 1)?
                        ),
                        &params,
                    )
                    .await?;
                ensure!(
                    affected == 1,
                    "A selected row changed or was deleted. Refresh and retry; no rows were deleted."
                );
                total += affected;
            }
            total
        }
    };
    tx.commit().await?;
    Ok(affected)
}

pub async fn database_connections(client: &Client, database: &str) -> Result<ResultSet> {
    let rows = client.query("SELECT pid::text,usename,coalesce(application_name,''),coalesce(state,'unknown') FROM pg_stat_activity WHERE datname=$1 ORDER BY pid", &[&database]).await?;
    Ok(ResultSet {
        columns: vec![
            "PID".into(),
            "User".into(),
            "Application".into(),
            "State".into(),
        ],
        rows: rows
            .iter()
            .map(|row| {
                (0..4)
                    .map(|i| {
                        row.get::<_, Option<String>>(i)
                            .map(Cell::Text)
                            .unwrap_or(Cell::Null)
                    })
                    .collect()
            })
            .collect(),
        ..Default::default()
    })
}

pub async fn database_action(client: &Client, action: DatabaseAction) -> Result<()> {
    let (source, force) = match &action {
        DatabaseAction::Create { .. } => (None, false),
        DatabaseAction::Rename { source, force, .. }
        | DatabaseAction::Clone { source, force, .. } => (Some(source), *force),
        DatabaseAction::Delete { name, force } => (Some(name), *force),
    };
    if let Some(source) = source {
        let current: String = client
            .query_one("SELECT current_database()", &[])
            .await?
            .get(0);
        ensure!(
            source != &current,
            "Choose a different maintenance database (for example postgres) before administering this database."
        );
        if force {
            client.query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=$1 AND pid<>pg_backend_pid()", &[source]).await?;
        }
    }
    let sql = match action {
        DatabaseAction::Create { name } => format!("CREATE DATABASE {}", quote_identifier(&name)?),
        DatabaseAction::Rename { source, target, .. } => format!(
            "ALTER DATABASE {} RENAME TO {}",
            quote_identifier(&source)?,
            quote_identifier(&target)?
        ),
        DatabaseAction::Clone { source, target, .. } => format!(
            "CREATE DATABASE {} WITH TEMPLATE {}",
            quote_identifier(&target)?,
            quote_identifier(&source)?
        ),
        DatabaseAction::Delete { name, .. } => {
            format!("DROP DATABASE {}", quote_identifier(&name)?)
        }
    };
    client.batch_execute(&sql).await?;
    Ok(())
}
