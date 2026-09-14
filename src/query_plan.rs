//! Backend plan output remains available even when a node has unfamiliar fields.
use crate::{
    model::{Cell, ResultSet},
    sql::SqlDialect,
};
use anyhow::{Context, Result, ensure};
use sqlparser::{
    ast::Statement,
    dialect::{Dialect, PostgreSqlDialect, SQLiteDialect},
    parser::Parser,
};

const SQLITE_PLAN: &str = "EXPLAIN QUERY PLAN";
const POSTGRES_PLAN: &str = "EXPLAIN (FORMAT JSON)";
const POSTGRES_ANALYZE: &str = "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)";

/// ANALYZE executes the statement, so hand-written ANALYZE variants also count.
pub fn is_analyze(sql: &str) -> bool {
    sql.starts_with("EXPLAIN (ANALYZE,")
}

/// Whether `sql` produces output that `QueryPlan::parse` understands.
pub fn is_plan_sql(sql: &str) -> bool {
    sql.starts_with(SQLITE_PLAN) || sql.starts_with(POSTGRES_PLAN) || is_analyze(sql)
}

pub fn explain_sql(sql: &str, dialect: SqlDialect, analyze: bool) -> Result<String> {
    let pg = matches!(dialect, SqlDialect::Postgres);
    ensure!(pg || !analyze, "SQLite provides estimated plans only.");
    let dialect: &dyn Dialect = if pg {
        &PostgreSqlDialect {}
    } else {
        &SQLiteDialect {}
    };
    let statements = Parser::parse_sql(dialect, sql)?;
    ensure!(
        statements.len() == 1,
        "Select exactly one statement to explain."
    );
    ensure!(
        matches!(
            statements[0],
            Statement::Query(_)
                | Statement::Insert(_)
                | Statement::Update { .. }
                | Statement::Delete(_)
        ),
        "Select a SELECT, INSERT, UPDATE, or DELETE statement."
    );
    let prefix = if !pg {
        SQLITE_PLAN
    } else if analyze {
        POSTGRES_ANALYZE
    } else {
        POSTGRES_PLAN
    };
    Ok(format!("{prefix}\n{sql}"))
}

#[derive(Clone, Debug)]
pub struct PlanNode {
    pub label: String,
    pub details: String,
    pub summary: String,
    pub estimate_mismatch: bool,
    pub children: Vec<PlanNode>,
}
#[derive(Clone, Debug)]
pub struct QueryPlan {
    pub nodes: Vec<PlanNode>,
    pub raw: String,
}
impl QueryPlan {
    pub fn parse(set: &ResultSet, postgres: bool) -> Result<Self> {
        ensure!(
            !set.truncated,
            "Plan exceeds the display limit; it cannot be rendered completely."
        );
        if postgres {
            let value = set
                .rows
                .first()
                .and_then(|r| r.first())
                .context("Database returned no plan")?
                .text();
            let raw: serde_json::Value = serde_json::from_str(&value)?;
            let root = raw
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.get("Plan"))
                .context("Database returned an unfamiliar plan; inspect raw results")?;
            Ok(Self {
                nodes: vec![pg_node(root, 0)?],
                raw: serde_json::to_string_pretty(&raw)?,
            })
        } else {
            let rows = set
                .rows
                .iter()
                .map(|row| {
                    ensure!(row.len() == 4, "Unexpected SQLite plan shape");
                    Ok((integer(&row[0])?, integer(&row[1])?, row[3].text()))
                })
                .collect::<Result<Vec<_>>>()?;
            let roots = rows.iter().filter(|(id, parent, _)| {
                id == parent || !rows.iter().any(|(other, _, _)| other == parent)
            });
            let nodes = roots
                .map(|(id, _, _)| sqlite_node(*id, &rows, &mut Vec::new()))
                .collect::<Result<Vec<_>>>()?;
            ensure!(!nodes.is_empty(), "SQLite returned no plan roots");
            Ok(Self {
                nodes,
                raw: rows
                    .iter()
                    .map(|(id, p, text)| format!("{id}\t{p}\t{text}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
        }
    }
}
fn integer(cell: &Cell) -> Result<i64> {
    cell.text().parse().context("Invalid plan node identifier")
}
fn pg_node(value: &serde_json::Value, depth: usize) -> Result<PlanNode> {
    ensure!(depth < 128, "Plan nesting exceeds the viewer limit");
    let label = value
        .get("Node Type")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown node")
        .to_owned();
    let mut details = value.clone();
    if let Some(object) = details.as_object_mut() {
        object.remove("Plans");
    }
    let children = value
        .get("Plans")
        .and_then(|v| v.as_array())
        .map(|nodes| nodes.iter().map(|v| pg_node(v, depth + 1)).collect())
        .unwrap_or(Ok(vec![]))?;
    let metric = |name: &str| {
        value
            .get(name)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".into())
    };
    let estimate_mismatch = match (
        value.get("Plan Rows").and_then(|v| v.as_f64()),
        value.get("Actual Rows").and_then(|v| v.as_f64()),
    ) {
        (Some(estimated), Some(actual)) => {
            estimated.max(1.0) / actual.max(1.0) >= 10.0
                || actual.max(1.0) / estimated.max(1.0) >= 10.0
        }
        _ => false,
    };
    let mut summary = format!(
        "Estimated rows: {} · total cost: {}",
        metric("Plan Rows"),
        metric("Total Cost")
    );
    if value.get("Actual Rows").is_some() {
        summary.push_str(&format!(
            " · actual rows/loop: {} · time/loop: {} ms · loops: {}",
            metric("Actual Rows"),
            metric("Actual Total Time"),
            metric("Actual Loops")
        ));
    }
    if estimate_mismatch {
        summary.push_str(" · estimate differs by at least 10× (counts below 1 treated as 1)");
    }
    Ok(PlanNode {
        label,
        summary,
        estimate_mismatch,
        details: serde_json::to_string_pretty(&details)?,
        children,
    })
}
fn sqlite_node(id: i64, rows: &[(i64, i64, String)], ancestors: &mut Vec<i64>) -> Result<PlanNode> {
    ensure!(
        ancestors.len() < 128 && !ancestors.contains(&id),
        "Invalid or deeply nested SQLite plan"
    );
    ancestors.push(id);
    let label = rows
        .iter()
        .find(|r| r.0 == id)
        .context("Missing plan node")?
        .2
        .clone();
    let children = rows
        .iter()
        .filter(|(child, parent, _)| *parent == id && *child != id)
        .map(|(child, _, _)| sqlite_node(*child, rows, ancestors))
        .collect::<Result<Vec<_>>>()?;
    ancestors.pop();
    Ok(PlanNode {
        label,
        details: format!("Node {id}"),
        summary: String::new(),
        estimate_mismatch: false,
        children,
    })
}
