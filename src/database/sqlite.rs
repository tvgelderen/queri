use super::{browse_ordering, push_row, validate_original};
use crate::{
    model::*,
    sql::{qualified, quote_identifier},
};
use anyhow::{Context, Result, ensure};
use fallible_iterator::FallibleIterator;
use rusqlite::{
    Connection, Statement, params, params_from_iter,
    types::{Value, ValueRef},
};

fn cell(value: ValueRef<'_>) -> Result<Cell> {
    Ok(match value {
        ValueRef::Null => Cell::Null,
        ValueRef::Integer(value) => Cell::Integer(value),
        ValueRef::Real(value) => Cell::Real(value),
        ValueRef::Text(value) => Cell::Text(std::str::from_utf8(value)?.to_owned()),
        ValueRef::Blob(value) => Cell::Blob(value.to_vec()),
    })
}

fn value(cell: &Cell) -> Value {
    match cell {
        Cell::Null => Value::Null,
        Cell::Integer(value) => Value::Integer(*value),
        Cell::Real(value) => Value::Real(*value),
        Cell::Text(value) => Value::Text(value.clone()),
        Cell::Blob(value) => Value::Blob(value.clone()),
    }
}

fn result(
    stmt: &mut Statement<'_>,
    params: &[Value],
    bytes: &mut usize,
    count: &mut usize,
) -> Result<ResultSet> {
    let columns: Vec<String> = stmt.column_names().map(str::to_owned).collect();
    let mut set = ResultSet {
        columns,
        ..Default::default()
    };
    if stmt.column_count() == 0 {
        set.affected = u64::try_from(stmt.execute(params_from_iter(params))?)?;
        return Ok(set);
    }
    let mut rows = stmt.query(params_from_iter(params))?;
    while let Some(row) = rows.next()? {
        let values = (0..set.columns.len())
            .map(|index| cell(row.get_ref(index)?))
            .collect::<Result<Vec<_>>>()?;
        push_row(&mut set, values, bytes, count);
        set.affected += 1;
    }
    Ok(set)
}

thread_local! {
    // rusqlite's busy handler accepts a function pointer, so its deadline is worker-local.
    static BUSY_DEADLINE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

fn wait_for_lock(_: i32) -> bool {
    BUSY_DEADLINE.with(|deadline| {
        let Some(remaining) = deadline
            .get()
            .and_then(|end| end.checked_duration_since(std::time::Instant::now()))
        else {
            return false;
        };
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
        true
    })
}

struct Deadline<'a> {
    conn: &'a mut Connection,
    busy_ms: Option<u32>,
}
impl Drop for Deadline<'_> {
    fn drop(&mut self) {
        if let Some(busy_ms) = self.busy_ms {
            let _ = self.conn.progress_handler(0, None::<fn() -> bool>);
            let _ = self
                .conn
                .busy_timeout(std::time::Duration::from_millis(u64::from(busy_ms)));
        }
        BUSY_DEADLINE.with(|deadline| deadline.set(None));
    }
}

pub fn query(
    conn: &mut Connection,
    sql: &str,
    timeout: StatementTimeout,
) -> Result<Vec<ResultSet>> {
    query_bound(conn, sql, timeout, &[])
}

pub fn query_bound(
    conn: &mut Connection,
    sql: &str,
    timeout: StatementTimeout,
    values: &[Cell],
) -> Result<Vec<ResultSet>> {
    let values: Vec<_> = values.iter().map(value).collect();
    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let busy_ms = if timeout.duration().is_some() {
        Some(conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?)
    } else {
        None
    };
    let scope = Deadline { conn, busy_ms };
    if busy_ms.is_some() {
        scope.conn.busy_handler(Some(wait_for_lock))?;
        let expired = timed_out.clone();
        scope.conn.progress_handler(
            1000,
            Some(move || {
                let expired_now = BUSY_DEADLINE.with(|deadline| {
                    deadline
                        .get()
                        .is_some_and(|end| std::time::Instant::now() >= end)
                });
                if expired_now {
                    expired.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                expired_now
            }),
        )?;
    }
    let conn = &*scope.conn;
    let mut sets = vec![];
    let mut batch = rusqlite::Batch::new(conn, sql);
    let (mut bytes, mut count) = (0, 0);
    loop {
        let deadline = timeout
            .duration()
            .map(|duration| std::time::Instant::now() + duration);
        BUSY_DEADLINE.with(|value| value.set(deadline));
        let next = (|| -> Result<Option<ResultSet>> {
            let Some(mut stmt) = batch.next()? else {
                return Ok(None);
            };
            Ok(Some(result(&mut stmt, &values, &mut bytes, &mut count)?))
        })();
        match next {
            Ok(Some(set)) => sets.push(set),
            Ok(None) => break,
            Err(error) => {
                let lock_expired = error.downcast_ref::<rusqlite::Error>().is_some_and(|e| {
                    matches!(
                        e.sqlite_error_code(),
                        Some(
                            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                        )
                    )
                }) && deadline
                    .is_some_and(|end| std::time::Instant::now() >= end);
                return Err(
                    if timed_out.load(std::sync::atomic::Ordering::Relaxed) || lock_expired {
                        error.context(super::StatementTimedOut)
                    } else {
                        error
                    },
                );
            }
        }
    }
    Ok(sets)
}

fn select(conn: &Connection, sql: &str, values: &[Value]) -> Result<ResultSet> {
    result(&mut conn.prepare(sql)?, values, &mut 0, &mut 0)
}

pub fn tables(conn: &Connection, schema: &str) -> Result<Vec<TableRef>> {
    let mut stmt = conn.prepare(&format!("SELECT name,type FROM {}.sqlite_schema WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name", quote_identifier(schema)?))?;
    let mut rows = stmt.query([])?;
    let mut tables = Vec::new();
    while let Some(row) = rows.next()? {
        tables.push(TableRef {
            schema: schema.into(),
            name: row.get(0)?,
            kind: row.get::<_, String>(1)?.parse()?,
        });
    }
    Ok(tables)
}

pub(super) fn metadata(conn: &Connection, table: &TableRef) -> Result<TableMetadata> {
    let mut stmt = conn.prepare(
        "SELECT name,type,\"notnull\",dflt_value,pk,hidden FROM pragma_table_xinfo(?1,?2)",
    )?;
    let columns = stmt
        .query_map(params![table.name, table.schema], |row| {
            let pk: i64 = row.get(4)?;
            let not_null: i64 = row.get(2)?;
            Ok(Column {
                name: row.get(0)?,
                data_type: row.get(1)?,
                nullable: not_null == 0 && pk == 0,
                default: row.get(3)?,
                primary_key: pk > 0,
                generated: row.get::<_, i64>(5)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(
        !columns.is_empty(),
        "The table no longer exists or has no visible columns."
    );
    let args = [
        Value::Text(table.name.clone()),
        Value::Text(table.schema.clone()),
    ];
    let indexes = select(
        conn,
        "SELECT name, \"unique\" AS is_unique, origin, partial, (SELECT group_concat(COALESCE(name,'(expression)'),', ') FROM pragma_index_info(l.name,?2)) AS columns FROM pragma_index_list(?1,?2) l",
        &args,
    )?;
    let mut constraints = select(
        conn,
        "SELECT id,seq,\"table\" AS referenced_table,\"from\" AS column,\"to\" AS referenced_column,on_update,on_delete FROM pragma_foreign_key_list(?1,?2)",
        &args,
    )?;
    // SQLite exposes CHECK clauses only in the original CREATE statement.
    let definition: String = conn.query_row(
        &format!(
            "SELECT coalesce(sql,'') FROM {}.sqlite_schema WHERE name=?1",
            quote_identifier(&table.schema)?
        ),
        [&table.name],
        |row| row.get(0),
    )?;
    constraints.columns.push("table_definition".into());
    if constraints.rows.is_empty() {
        constraints
            .rows
            .push(vec![Cell::Null; constraints.columns.len() - 1]);
    }
    for row in &mut constraints.rows {
        row.push(Cell::Text(definition.clone()));
    }
    Ok(TableMetadata {
        columns,
        indexes,
        constraints,
    })
}

pub fn browse(conn: &Connection, table: &TableRef, options: &BrowseOptions) -> Result<TablePage> {
    let metadata = metadata(conn, table)?;
    let qualified = qualified(&table.schema, &table.name)?;
    let mut values = vec![];
    let mut filter = match &options.filter {
        Some(expression) => format!(" WHERE ({})", expression.compile(&mut |filter| {
            let column = filter.validate(&metadata, true)?;
            let name = quote_identifier(&column.name)?;
            let kind = column.filter_kind(true);
            if let Some(text) = filter.predicate.value() {
                values.push(
                    filter_value(text, kind)
                        .with_context(|| format!("Invalid filter value for {}", column.name))?,
                );
            }
            let parameter = format!("?{}", values.len());
            let (left, right) = match kind {
                FilterKind::Number | FilterKind::Boolean => (name.clone(), parameter.clone()),
                // SQLite has no date storage class. Only interpret ISO text, never guess
                // whether numeric dates are Unix timestamps or Julian day numbers.
                FilterKind::Date => (
                    format!(
                        "julianday(CASE WHEN typeof({name})='text' AND {name} GLOB '????-??-??*' THEN {name} END)"
                    ),
                    format!("julianday({parameter})"),
                ),
                FilterKind::Text => (format!("lower(CAST({name} AS TEXT))"), format!("lower({parameter})")),
            };
            let predicate = match &filter.predicate {
                FilterPredicate::Contains(_) => format!("instr({left},{right}) > 0"),
                FilterPredicate::NotContains(_) => format!("instr({left},{right}) = 0"),
                FilterPredicate::Equals(_) => format!("{left} = {right}"),
                FilterPredicate::NotEquals(_) => format!("{left} <> {right}"),
                FilterPredicate::StartsWith(_) => {
                    format!("substr({left},1,length({right})) = {right}")
                }
                FilterPredicate::EndsWith(_) => {
                    format!("substr({left},length({left})-length({right})+1) = {right}")
                }
                FilterPredicate::GreaterThan(_) => format!("{left} > {right}"),
                FilterPredicate::GreaterOrEqual(_) => format!("{left} >= {right}"),
                FilterPredicate::LessThan(_) => format!("{left} < {right}"),
                FilterPredicate::LessOrEqual(_) => format!("{left} <= {right}"),
                FilterPredicate::IsNull => format!("{name} IS NULL"),
                FilterPredicate::IsNotNull => format!("{name} IS NOT NULL"),
            };
            if kind == FilterKind::Number && filter.predicate.value().is_some() {
                Ok(format!("(typeof({name}) IN ('integer','real') AND ({predicate}))"))
            } else {
                Ok(predicate)
            }
        })?),
        None => String::new(),
    };
    if let Some(scope) = &options.relationship {
        let (predicate, params) = relationship_predicate(conn, table, scope, values.len() + 1)?;
        values.extend(params);
        filter.push_str(if filter.is_empty() {
            " WHERE "
        } else {
            " AND "
        });
        filter.push_str(&predicate);
    }
    let sort = browse_ordering(&metadata, &options.sort)?;
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) FROM {qualified}{filter}"),
        params_from_iter(&values),
        |row| row.get(0),
    )?;
    let names = metadata
        .columns
        .iter()
        .map(|col| quote_identifier(&col.name))
        .collect::<Result<Vec<_>>>()?
        .join(",");
    let data = select(
        conn,
        &format!(
            "SELECT {names} FROM {qualified}{filter}{sort} LIMIT {} OFFSET {}",
            options.limit, options.offset
        ),
        &values,
    )?;
    Ok(TablePage {
        data,
        total: u64::try_from(total)?,
        metadata,
    })
}

fn filter_value(text: &str, kind: FilterKind) -> Result<Value> {
    match kind {
        FilterKind::Number => {
            if let Ok(integer) = text.trim().parse::<i64>() {
                return Ok(Value::Integer(integer));
            }
            let number: f64 = text.trim().parse().context("Enter a valid number.")?;
            ensure!(number.is_finite(), "Enter a finite number.");
            Ok(Value::Real(number))
        }
        FilterKind::Boolean => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(Value::Integer(1)),
            "false" | "0" => Ok(Value::Integer(0)),
            _ => anyhow::bail!("Enter true or false."),
        },
        FilterKind::Date => {
            let valid = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
                || chrono::DateTime::parse_from_rfc3339(text).is_ok()
                || ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"]
                    .iter()
                    .any(|format| chrono::NaiveDateTime::parse_from_str(text, format).is_ok());
            ensure!(valid, "Enter a date (YYYY-MM-DD) or an ISO timestamp.");
            Ok(Value::Text(text.to_owned()))
        }
        FilterKind::Text => Ok(Value::Text(text.to_owned())),
    }
}

fn predicate(
    metadata: &TableMetadata,
    original: &[Cell],
    params: &mut Vec<Value>,
) -> Result<String> {
    metadata
        .columns
        .iter()
        .zip(original)
        .map(|(col, old)| {
            params.push(value(old));
            Ok(format!(
                "{} IS ?{}",
                quote_identifier(&col.name)?,
                params.len()
            ))
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join(" AND "))
}

pub fn change_rows(conn: &mut Connection, table: &TableRef, change: RowChange) -> Result<u64> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let metadata = metadata(&tx, table)?;
    ensure!(
        table.is_table(),
        "Views cannot be edited in the row inspector."
    );
    let qualified = qualified(&table.schema, &table.name)?;
    let affected = match change {
        RowChange::Insert(fields) => {
            ensure!(
                fields.len() == metadata.columns.len(),
                "The table structure changed. Refresh before inserting."
            );
            let mut names = vec![];
            let mut params = vec![];
            for (column, field) in metadata.columns.iter().zip(fields) {
                if column.generated || matches!(field, FieldValue::Default) {
                    continue;
                }
                names.push(quote_identifier(&column.name)?);
                params.push(match field {
                    FieldValue::Value(cell) => value(&cell),
                    _ => Value::Null,
                });
            }
            let sql = if names.is_empty() {
                format!("INSERT INTO {qualified} DEFAULT VALUES")
            } else {
                format!(
                    "INSERT INTO {qualified} ({}) VALUES ({})",
                    names.join(","),
                    (1..=params.len())
                        .map(|i| format!("?{i}"))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            tx.execute(&sql, params_from_iter(params))?
        }
        RowChange::Update { original, values } => {
            validate_original(&metadata, table, &original)?;
            ensure!(
                values.len() == metadata.columns.len(),
                "The table structure changed. Refresh before editing."
            );
            let mut params = vec![];
            let mut assignments = vec![];
            for (column, field) in metadata.columns.iter().zip(values) {
                if column.generated {
                    continue;
                }
                let expression = match field {
                    FieldValue::Default => column.default.clone().unwrap_or_else(|| "NULL".into()),
                    FieldValue::Null => "NULL".into(),
                    FieldValue::Value(cell) => {
                        params.push(value(&cell));
                        format!("?{}", params.len())
                    }
                };
                assignments.push(format!("{}={expression}", quote_identifier(&column.name)?));
            }
            let predicate = predicate(&metadata, &original, &mut params)?;
            let affected = tx.execute(
                &format!(
                    "UPDATE {qualified} SET {} WHERE {predicate}",
                    assignments.join(",")
                ),
                params_from_iter(params),
            )?;
            ensure!(
                affected == 1,
                "The row changed or was deleted. Refresh and try again; no changes were saved."
            );
            affected
        }
        RowChange::Delete { originals } => {
            ensure!(!originals.is_empty(), "Select at least one row.");
            let mut total = 0;
            for original in originals {
                validate_original(&metadata, table, &original)?;
                let mut params = vec![];
                let predicate = predicate(&metadata, &original, &mut params)?;
                let affected = tx.execute(
                    &format!("DELETE FROM {qualified} WHERE {predicate}"),
                    params_from_iter(params),
                )?;
                ensure!(
                    affected == 1,
                    "A selected row changed or was deleted. Refresh and retry; no rows were deleted."
                );
                total += affected;
            }
            total
        }
    };
    tx.commit()?;
    Ok(u64::try_from(affected)?)
}

pub fn foreign_keys(
    conn: &Connection,
    table: &TableRef,
    incoming: bool,
) -> Result<Vec<ForeignKey>> {
    let sources = if incoming {
        tables(conn, &table.schema)?
    } else {
        vec![table.clone()]
    };
    let mut keys = vec![];
    for source in sources.into_iter().filter(TableRef::is_table) {
        let mut stmt=conn.prepare("SELECT id,seq,\"table\",\"from\",\"to\" FROM pragma_foreign_key_list(?1,?2) ORDER BY id,seq")?;
        let rows = stmt.query_map(params![source.name, source.schema], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut local: Vec<ForeignKey> = vec![];
        for row in rows {
            let (id, target, from, to) = row?;
            let id = id.to_string();
            if local.last().is_none_or(|k| k.id != id) {
                local.push(ForeignKey {
                    id: id.clone(),
                    name: format!("{} → {}", source.name, target),
                    source: source.clone(),
                    target: TableRef {
                        schema: source.schema.clone(),
                        name: target.clone(),
                        kind: RelationKind::Table,
                    },
                    columns: vec![],
                });
            }
            if let Some(key) = local.last_mut() {
                let target_column = if let Some(to) = to {
                    to
                } else {
                    conn.query_row(
                        "SELECT name FROM pragma_table_info(?1,?2) WHERE pk=?3",
                        params![target, source.schema, i64::try_from(key.columns.len())? + 1],
                        |row| row.get(0),
                    )
                    .context("Could not resolve the referenced primary key")?
                };
                key.columns.push(ForeignKeyColumn {
                    source: from,
                    target: target_column,
                });
            }
        }
        keys.extend(
            local
                .into_iter()
                .filter(|k| !incoming || k.target.name.eq_ignore_ascii_case(&table.name)),
        );
    }
    Ok(keys)
}

fn relationship_predicate(
    conn: &Connection,
    table: &TableRef,
    scope: &RelationshipScope,
    start: usize,
) -> Result<(String, Vec<Value>)> {
    let origin = if scope.incoming {
        &scope.key.target
    } else {
        &scope.key.source
    };
    ensure!(
        scope.destination() == table
            && foreign_keys(conn, origin, scope.incoming)?.contains(&scope.key),
        "This relationship changed or was removed. Open it again from the source table."
    );
    ensure!(
        scope.values.len() == scope.key.columns.len()
            && !scope.values.iter().any(|v| matches!(v, Cell::Null)),
        "The relationship key is incomplete."
    );
    let mut predicates = vec![];
    let mut values = vec![];
    for (i, (column, cell)) in scope.key.columns.iter().zip(&scope.values).enumerate() {
        values.push(match cell {
            Cell::Null => Value::Null,
            Cell::Text(s) => Value::Text(s.clone()),
            Cell::Integer(n) => Value::Integer(*n),
            Cell::Real(n) => Value::Real(*n),
            Cell::Blob(b) => Value::Blob(b.clone()),
        });
        let parameter = start + i;
        let parent = quote_identifier(&column.target)?;
        if scope.incoming {
            // Parent on the left supplies its declared affinity and collation. Unary +
            // strips the child's affinity, matching SQLite foreign-key comparison rules.
            predicates.push(format!(
                "queri_parent.{parent} = ?{parameter} AND queri_parent.{parent} = +{}.{}",
                qualified(&table.schema, &table.name)?,
                quote_identifier(&column.source)?
            ));
        } else {
            predicates.push(format!(
                "{}.{parent} = ?{parameter}",
                qualified(&table.schema, &table.name)?
            ));
        }
    }
    let predicate = predicates.join(" AND ");
    Ok((
        if scope.incoming {
            format!(
                "EXISTS (SELECT 1 FROM {} AS queri_parent WHERE {predicate})",
                qualified(&origin.schema, &origin.name)?
            )
        } else {
            predicate
        },
        values,
    ))
}
