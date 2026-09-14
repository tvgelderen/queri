use super::*;
use crate::row_updates::UpdateBatch;
use std::sync::atomic::{AtomicBool, Ordering};

pub(in crate::database) fn apply(
    conn: &mut Connection,
    table: &TableRef,
    batch: UpdateBatch,
    timeout: StatementTimeout,
    cancelled: &AtomicBool,
) -> Result<u64> {
    let nested = !conn.is_autocommit();
    query(
        conn,
        if nested {
            "SAVEPOINT queri_inline_edits"
        } else {
            "BEGIN IMMEDIATE"
        },
        timeout,
    )?;
    let result = (|| {
        let metadata = metadata(conn, table)?;
        batch.validate(&metadata, table)?;
        let table_name = qualified(&table.schema, &table.name)?;
        for (index, row) in batch.rows.iter().enumerate() {
            ensure!(!cancelled.load(Ordering::Relaxed), "Save cancelled");
            let statement = statement(&metadata, &table_name, row)?;
            let sets = query_bound(conn, &statement.sql, timeout, &statement.values)
                .with_context(|| format!("Could not save pending row {}", index + 1))?;
            ensure!(
                sets.first().is_some_and(|set| set.affected == 1),
                "Pending row {} changed or was deleted. No pending edits were saved.",
                index + 1
            );
        }
        ensure!(!cancelled.load(Ordering::Relaxed), "Save cancelled");
        Ok::<_, anyhow::Error>(u64::try_from(batch.rows.len())?)
    })();
    match result {
        Ok(count) => {
            let commit = query(
                conn,
                if nested {
                    "RELEASE SAVEPOINT queri_inline_edits"
                } else {
                    "COMMIT"
                },
                timeout,
            );
            if let Err(error) = commit {
                rollback(conn, nested)?;
                return Err(error);
            }
            Ok(count)
        }
        Err(error) => {
            rollback(conn, nested)?;
            Err(error)
        }
    }
}
fn rollback(conn: &Connection, nested: bool) -> Result<()> {
    if conn.is_autocommit() {
        ensure!(
            !nested,
            "SQLite rolled back the entire transaction, including changes made before this save."
        );
        return Ok(());
    }
    conn.execute_batch(if nested {
        "ROLLBACK TO SAVEPOINT queri_inline_edits; RELEASE SAVEPOINT queri_inline_edits"
    } else {
        "ROLLBACK"
    })
    .context("Could not confirm rollback; transaction outcome is unknown")?;
    Ok(())
}

pub(super) fn statement(
    metadata: &TableMetadata,
    table_name: &str,
    row: &crate::row_updates::RowUpdate,
) -> Result<crate::parameters::BoundQuery> {
    let mut params = vec![];
    let mut assignments = vec![];
    for (column, field) in &row.fields {
        let column = &metadata.columns[*column];
        let expression = match field {
            FieldValue::Default => column.default.clone().unwrap_or_else(|| "NULL".into()),
            FieldValue::Null => "NULL".into(),
            FieldValue::Value(value) => {
                params.push(value.clone());
                format!("?{}", params.len())
            }
        };
        assignments.push(format!("{}={expression}", quote_identifier(&column.name)?));
    }
    let predicate = original_row_predicate(metadata, &row.original, &mut params)?;
    let sql = format!(
        "UPDATE {table_name} SET {} WHERE {predicate}",
        assignments.join(",")
    );
    Ok(crate::parameters::BoundQuery {
        sql,
        values: params,
    })
}
