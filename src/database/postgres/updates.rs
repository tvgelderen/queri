use super::*;
use crate::row_updates::UpdateBatch;
use std::sync::atomic::{AtomicBool, Ordering};

pub(in crate::database) async fn apply(
    client: &Client,
    table: &TableRef,
    batch: UpdateBatch,
    cancelled: &AtomicBool,
) -> Result<u64> {
    let state = client.transaction_status().await?;
    ensure!(
        matches!(state, b'I' | b'T'),
        "Resolve the failed or unknown transaction before saving."
    );
    let nested = state == b'T';
    client
        .batch_execute(if nested {
            "SAVEPOINT queri_inline_edits"
        } else {
            "BEGIN"
        })
        .await?;
    let result = async {
        // Hold a relation lock through validation and writing so DDL cannot invalidate the shape.
        client
            .batch_execute(&format!(
                "LOCK TABLE {} IN ROW EXCLUSIVE MODE",
                qualified(&table.schema, &table.name)?
            ))
            .await?;
        let metadata = metadata(client, table).await?;
        batch.validate(&metadata, table)?;
        let table_name = qualified(&table.schema, &table.name)?;
        for (index, row) in batch.rows.iter().enumerate() {
            ensure!(!cancelled.load(Ordering::Relaxed), "Save cancelled");
            update(client, &metadata, &table_name, row)
                .await
                .with_context(|| format!("Could not save pending row {}", index + 1))?;
        }
        ensure!(!cancelled.load(Ordering::Relaxed), "Save cancelled");
        Ok::<_, anyhow::Error>(u64::try_from(batch.rows.len())?)
    }
    .await;
    match result {
        Ok(count) => {
            client
                .batch_execute(if nested {
                    "RELEASE SAVEPOINT queri_inline_edits"
                } else {
                    "COMMIT"
                })
                .await
                .context("Could not confirm save; verify transaction outcome before retrying")?;
            Ok(count)
        }
        Err(error) => {
            client
                .batch_execute(if nested {
                    "ROLLBACK TO SAVEPOINT queri_inline_edits; RELEASE SAVEPOINT queri_inline_edits"
                } else {
                    "ROLLBACK"
                })
                .await
                .context("Could not confirm rollback; transaction outcome is unknown")?;
            Err(error)
        }
    }
}

pub(super) async fn update(
    client: &impl GenericClient,
    metadata: &TableMetadata,
    table_name: &str,
    row: &crate::row_updates::RowUpdate,
) -> Result<u64> {
    if row.fields.is_empty() {
        return Ok(0);
    }
    let mut object = serde_json::Map::new();
    let mut assignments = vec![];
    for (column, field) in &row.fields {
        let column = &metadata.columns[*column];
        let name = quote_identifier(&column.name)?;
        if matches!(field, FieldValue::Default) {
            assignments.push(format!("{name}=DEFAULT"));
        } else {
            object.insert(column.name.clone(), field_json(field));
            assignments.push(format!(
                "{name}=(json_populate_record(NULL::{table_name},$1::json)).{name}"
            ));
        }
    }
    let json = serde_json::Value::Object(object);
    let old = row
        .original
        .iter()
        .map(Cell::optional_text)
        .collect::<Vec<_>>();
    let mut params: Vec<&(dyn ToSql + Sync)> = vec![&json];
    for cell in &old {
        params.push(cell);
    }
    let sql = format!(
        "UPDATE {table_name} SET {} WHERE $1::json IS NOT NULL AND {}",
        assignments.join(","),
        predicate(metadata, 2)?
    );
    let count = client.execute(&sql, &params).await?;
    ensure!(
        count == 1,
        "The row changed or was deleted. No pending edits were saved."
    );
    Ok(count)
}
