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

pub fn query(conn: &Connection, sql: &str) -> Result<Vec<ResultSet>> {
    let mut sets = vec![];
    let mut batch = rusqlite::Batch::new(conn, sql);
    let (mut bytes, mut count) = (0, 0);
    while let Some(mut stmt) = batch.next()? {
        sets.push(result(&mut stmt, &[], &mut bytes, &mut count)?);
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

fn metadata(conn: &Connection, table: &TableRef) -> Result<TableMetadata> {
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
    let filter = match &options.filter {
        Some(filter) => {
            let column = filter.validate(&metadata, true)?;
            let name = quote_identifier(&column.name)?;
            let kind = column.filter_kind(true);
            if let Some(text) = filter.predicate.value() {
                values.push(
                    filter_value(text, kind)
                        .with_context(|| format!("Invalid filter value for {}", column.name))?,
                );
            }
            let (left, right) = match kind {
                FilterKind::Number | FilterKind::Boolean => (name.clone(), "?1"),
                // SQLite has no date storage class. Only interpret ISO text, never guess
                // whether numeric dates are Unix timestamps or Julian day numbers.
                FilterKind::Date => (
                    format!(
                        "julianday(CASE WHEN typeof({name})='text' AND {name} GLOB '????-??-??*' THEN {name} END)"
                    ),
                    "julianday(?1)",
                ),
                FilterKind::Text => (format!("lower(CAST({name} AS TEXT))"), "lower(?1)"),
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
                format!(" WHERE typeof({name}) IN ('integer','real') AND ({predicate})")
            } else {
                format!(" WHERE {predicate}")
            }
        }
        None => String::new(),
    };
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
