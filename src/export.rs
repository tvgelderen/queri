//! Export loaded snapshots only; this module never executes SQL.
use crate::model::{Cell, ResultSet};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{io::Write, path::Path};

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Csv,
    Tsv,
    Json,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Json => "json",
        }
    }
}
#[derive(Clone)]
pub struct Selection {
    pub rows: Vec<usize>,
    pub columns: Vec<usize>,
}
impl Selection {
    pub fn all(data: &ResultSet) -> Self {
        Self {
            rows: (0..data.rows.len()).collect(),
            columns: (0..data.columns.len()).collect(),
        }
    }
}
#[derive(Serialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
enum Value<'a> {
    Null,
    Text(&'a str),
    Integer(String),
    Real(String),
    Blob(String),
}

fn field(writer: &mut impl Write, text: &str, delimiter: char) -> Result<()> {
    if text.contains([delimiter, '"', '\n', '\r']) || text.is_empty() {
        write!(writer, "\"{}\"", text.replace('"', "\"\""))?;
    } else {
        write!(writer, "{text}")?;
    }
    Ok(())
}
pub fn serialize(
    data: &ResultSet,
    selection: &Selection,
    format: Format,
    headers: bool,
    null: &str,
    writer: &mut impl Write,
) -> Result<()> {
    ensure!(
        selection.columns.iter().all(|i| *i < data.columns.len()),
        "Selected columns are no longer available."
    );
    ensure!(
        selection.rows.iter().all(|i| data
            .rows
            .get(*i)
            .is_some_and(|r| selection.columns.iter().all(|c| *c < r.len()))),
        "Selected rows are no longer available."
    );
    if format == Format::Json {
        #[derive(Serialize)]
        struct Export<'a> {
            version: u8,
            truncated: bool,
            columns: Vec<&'a str>,
            rows: Vec<Vec<Value<'a>>>,
        }
        let rows = selection
            .rows
            .iter()
            .map(|r| {
                selection
                    .columns
                    .iter()
                    .map(|c| match &data.rows[*r][*c] {
                        Cell::Null => Value::Null,
                        Cell::Text(s) => Value::Text(s),
                        Cell::Integer(n) => Value::Integer(n.to_string()),
                        Cell::Real(n) => Value::Real(n.to_string()),
                        cell @ Cell::Blob(_) => Value::Blob(cell.text()),
                    })
                    .collect()
            })
            .collect();
        serde_json::to_writer_pretty(
            writer,
            &Export {
                version: 1,
                truncated: data.truncated,
                columns: selection
                    .columns
                    .iter()
                    .map(|c| data.columns[*c].as_str())
                    .collect(),
                rows,
            },
        )?;
        return Ok(());
    }
    let delimiter = if format == Format::Csv { ',' } else { '\t' };
    if headers {
        for (i, c) in selection.columns.iter().enumerate() {
            if i > 0 {
                write!(writer, "{delimiter}")?;
            }
            field(writer, &data.columns[*c], delimiter)?;
        }
        writeln!(writer)?;
    }
    for r in &selection.rows {
        for (i, c) in selection.columns.iter().enumerate() {
            if i > 0 {
                write!(writer, "{delimiter}")?;
            }
            let cell = &data.rows[*r][*c];
            field(
                writer,
                &if matches!(cell, Cell::Null) {
                    null.to_owned()
                } else {
                    cell.text()
                },
                delimiter,
            )?;
        }
        writeln!(writer)?;
    }
    Ok(())
}

/// Finalize only after a complete, flushed write on the destination filesystem.
pub fn atomic_write(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file =
        tempfile::NamedTempFile::new_in(parent).context("Could not create export file")?;
    write(file.as_file_mut())?;
    file.as_file_mut().sync_all()?;
    file.persist(path)
        .map_err(|e| e.error)
        .context("Could not save file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_exact_values_and_duplicate_columns() {
        let data = ResultSet {
            columns: vec!["x".into(), "x".into()],
            rows: vec![
                vec![Cell::Integer(i64::MAX), Cell::Text("a,\"b\"\n".into())],
                vec![Cell::Null, Cell::Blob(vec![0, 255])],
            ],
            truncated: true,
            ..Default::default()
        };
        let mut bytes = Vec::new();
        serialize(
            &data,
            &Selection::all(&data),
            Format::Csv,
            true,
            "\\N",
            &mut bytes,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "x,x\n9223372036854775807,\"a,\"\"b\"\"\n\"\n\\N,00ff\n"
        );
        let mut bytes = Vec::new();
        serialize(
            &data,
            &Selection::all(&data),
            Format::Json,
            true,
            "",
            &mut bytes,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["columns"], serde_json::json!(["x", "x"]));
        assert_eq!(value["rows"][0][0]["value"], i64::MAX.to_string());
        assert_eq!(value["rows"][1][0]["type"], "null");
        assert_eq!(value["rows"][1][1]["value"], "00ff");
    }
    #[test]
    fn failed_write_preserves_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data");
        std::fs::write(&path, "original").unwrap();
        assert!(
            atomic_write(&path, |f| {
                f.write_all(b"partial")?;
                anyhow::bail!("failed")
            })
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
