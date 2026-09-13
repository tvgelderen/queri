//! Statistics describe their input scope; loaded rows are not a random sample.
use crate::model::*;
use anyhow::{Context, Result, ensure};

#[derive(Clone, Debug)]
pub struct ColumnStatistics {
    pub rows: u64,
    pub nulls: u64,
    pub distinct: Option<u64>,
    pub min: Option<Cell>,
    pub max: Option<Cell>,
    pub common: Vec<(Cell, u64)>,
    pub note: String,
}
impl ColumnStatistics {
    pub fn loaded(data: &ResultSet, column: usize, postgres: bool) -> Result<Self> {
        ensure!(column < data.columns.len(), "Unknown column");
        let values = data
            .rows
            .iter()
            .map(|r| r.get(column).context("Incomplete row"))
            .collect::<Result<Vec<_>>>()?;
        let rows = u64::try_from(values.len())?;
        let nulls = u64::try_from(values.iter().filter(|v| matches!(v, Cell::Null)).count())?;
        if postgres {
            return Ok(Self {
                rows,
                nulls,
                distinct: None,
                min: None,
                max: None,
                common: vec![],
                note:
                    "Loaded rows only. Database comparisons and ordering require All matching rows."
                        .into(),
            });
        }
        let mut common: Vec<(Cell, u64)> = vec![];
        for value in values.iter().filter(|v| !matches!(v, Cell::Null)) {
            if let Some((_, count)) = common.iter_mut().find(|(v, _)| v == *value) {
                *count += 1;
            } else {
                common.push(((*value).clone(), 1));
            }
        }
        let distinct = Some(u64::try_from(common.len())?);
        let integers = values
            .iter()
            .filter_map(|v| {
                if let Cell::Integer(n) = v {
                    Some(*n)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let (min, max) = if u64::try_from(integers.len())? == rows - nulls {
            (
                integers.iter().min().copied().map(Cell::Integer),
                integers.iter().max().copied().map(Cell::Integer),
            )
        } else {
            (None, None)
        };
        common.sort_by_key(|item| std::cmp::Reverse(item.1));
        common.truncate(10);
        Ok(Self {rows,nulls,distinct,min,max,common,note:"Loaded rows only. Distinct/common values compare exact stored representations, not database collations. Range is available for integer-only values.".into()})
    }
    pub(crate) fn from_result(set: &ResultSet, group: bool, range: bool) -> Result<Self> {
        ensure!(
            !set.truncated,
            "Statistics exceeded the display limit; no complete result is available"
        );
        let row = set.rows.first().context("Missing statistics summary")?;
        ensure!(row.len() == 8, "Unexpected statistics result");
        let number =
            |i: usize| -> Result<u64> { row[i].text().parse().context("Invalid statistics count") };
        let value = |i: usize| {
            if matches!(row[i], Cell::Null) {
                None
            } else {
                Some(row[i].clone())
            }
        };
        let common = set
            .rows
            .iter()
            .skip(1)
            .map(|r| {
                ensure!(r.len() == 8, "Invalid common-value result");
                Ok((r[6].clone(), r[7].text().parse()?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {rows:number(1)?,nulls:number(2)?,distinct:if group{Some(number(3)?)}else{None},min:if range{value(4)}else{None},max:if range{value(5)}else{None},common,note:"All matching rows, using database comparison rules. Metrics unavailable for this type are shown as unavailable.".into()})
    }
    pub fn text(&self) -> String {
        let metric = |v: &Option<Cell>| {
            v.as_ref()
                .map(Cell::text)
                .unwrap_or_else(|| "Unavailable or no non-NULL values".into())
        };
        let percent = if self.rows == 0 {
            "—".into()
        } else {
            format!("{:.2}%", self.nulls as f64 * 100.0 / self.rows as f64)
        };
        format!(
            "{}\nRows: {}\nNULL: {} ({})\nDistinct non-NULL: {}\nMinimum: {}\nMaximum: {}\nMost common non-NULL values:\n{}",
            self.note,
            self.rows,
            self.nulls,
            percent,
            self.distinct
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Unavailable".into()),
            metric(&self.min),
            metric(&self.max),
            self.common
                .iter()
                .map(|(v, n)| format!("{} × {}", n, v.text()))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

pub(crate) fn capabilities(column: &Column, postgres: bool) -> (bool, bool) {
    if !postgres {
        return (true, true);
    }
    let name = column
        .data_type
        .split('(')
        .next()
        .unwrap_or_default()
        .trim();
    let numeric = matches!(
        name,
        "smallint" | "integer" | "bigint" | "real" | "double precision" | "numeric" | "decimal"
    );
    let range = numeric
        || matches!(
            name,
            "text"
                | "character varying"
                | "character"
                | "date"
                | "timestamp without time zone"
                | "timestamp with time zone"
                | "time without time zone"
                | "time with time zone"
                | "interval"
        );
    (
        range || matches!(name, "boolean" | "uuid" | "bytea" | "jsonb"),
        range,
    )
}
pub(crate) fn query(from: &str, column: &Column, postgres: bool) -> Result<(String, bool, bool)> {
    let (group, range) = capabilities(column, postgres);
    let cast = if postgres { "::text" } else { "" };
    let distinct = if group {
        format!("count(DISTINCT value){cast}")
    } else {
        "NULL".into()
    };
    let min = if range {
        format!("min(value){cast}")
    } else {
        "NULL".into()
    };
    let max = if range {
        format!("max(value){cast}")
    } else {
        "NULL".into()
    };
    let common = if group {
        format!(
            " UNION ALL SELECT 1,NULL,NULL,NULL,NULL,NULL,value{cast},frequency{cast} FROM (SELECT value,count(*) AS frequency FROM source WHERE value IS NOT NULL GROUP BY value ORDER BY count(*) DESC LIMIT 10) AS common"
        )
    } else {
        String::new()
    };
    let sql = format!(
        "WITH source AS MATERIALIZED (SELECT {} AS value {from}) SELECT 0 AS kind,count(*){cast} AS rows,(count(*)-count(value)){cast} AS nulls,{distinct} AS distinct_values,{min} AS minimum,{max} AS maximum,NULL{cast} AS value,NULL{cast} AS frequency FROM source{common}",
        crate::sql::quote_identifier(&column.name)?
    );
    Ok((sql, group, range))
}
