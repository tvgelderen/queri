//! Page-local edits retain their original identity until the batch is saved.
use crate::model::*;
use anyhow::{Result, ensure};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct RowUpdate {
    pub original: Vec<Cell>,
    pub fields: BTreeMap<usize, FieldValue>,
}
impl RowUpdate {
    pub(crate) fn from_values(
        original: Vec<Cell>,
        values: Vec<FieldValue>,
        metadata: &TableMetadata,
    ) -> Result<Self> {
        ensure!(
            values.len() == metadata.columns.len(),
            "The table structure changed. Refresh before editing."
        );
        let mut edits = PendingEdits::default();
        for (index, value) in values.into_iter().enumerate() {
            if !metadata.columns[index].generated {
                edits.set(&original, index, value);
            }
        }
        Ok(edits.rows.pop().unwrap_or(Self {
            original,
            fields: BTreeMap::new(),
        }))
    }
}

#[derive(Clone, Debug)]
pub struct UpdateBatch {
    pub columns: Vec<Column>,
    pub rows: Vec<RowUpdate>,
}
impl UpdateBatch {
    pub(crate) fn validate(&self, metadata: &TableMetadata, table: &TableRef) -> Result<()> {
        ensure!(
            self.columns == metadata.columns,
            "The table structure changed. Discard the edits and refresh before editing."
        );
        ensure!(!self.rows.is_empty(), "No pending changes.");
        for row in &self.rows {
            validate_original(metadata, table, &row.original)?;
            ensure!(!row.fields.is_empty(), "No changes in row");
            for index in row.fields.keys() {
                ensure!(
                    metadata.columns.get(*index).is_some_and(|c| !c.generated),
                    "Column cannot be edited"
                );
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct PendingEdits {
    pub rows: Vec<RowUpdate>,
}
impl PendingEdits {
    pub fn set(&mut self, original: &[Cell], column: usize, value: FieldValue) {
        let index = match self.rows.iter().position(|r| r.original == original) {
            Some(index) => index,
            None => {
                self.rows.push(RowUpdate {
                    original: original.to_vec(),
                    fields: BTreeMap::new(),
                });
                self.rows.len() - 1
            }
        };
        let unchanged = match (&value, original.get(column)) {
            (FieldValue::Null, Some(Cell::Null)) => true,
            (FieldValue::Value(value), Some(old)) => value == old,
            _ => false,
        };
        if unchanged {
            self.rows[index].fields.remove(&column);
        } else {
            self.rows[index].fields.insert(column, value);
        }
        self.rows.retain(|r| !r.fields.is_empty());
    }
    pub fn field(&self, original: &[Cell], column: usize) -> Option<&FieldValue> {
        self.rows
            .iter()
            .find(|r| r.original == original)?
            .fields
            .get(&column)
    }
    pub fn revert(&mut self, original: &[Cell]) {
        self.rows.retain(|r| r.original != original);
    }
}

pub(crate) fn validate_original(
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
