use super::*;
use crate::sql::quote_identifier as quote;
use anyhow::{Result, ensure};
use sqlparser::{parser::Parser, tokenizer::Token};

pub(super) fn fragment(value: &str, engine: &Engine, data_type: bool) -> Result<String> {
    let dialect: &dyn sqlparser::dialect::Dialect = match engine {
        Engine::Postgres => &sqlparser::dialect::PostgreSqlDialect {},
        Engine::Sqlite => &sqlparser::dialect::SQLiteDialect {},
    };
    let mut parser = Parser::new(dialect).try_with_sql(value)?;
    let rendered = if data_type {
        parser.parse_data_type()?.to_string()
    } else {
        parser.parse_expr()?.to_string()
    };
    ensure!(
        parser.next_token().token == Token::EOF,
        "Unexpected trailing SQL fragment"
    );
    Ok(rendered)
}
fn columns(names: &[String], table: &Table) -> Result<String> {
    ensure!(!names.is_empty(), "Empty key");
    let mut seen = std::collections::HashSet::new();
    names
        .iter()
        .map(|n| {
            ensure!(
                seen.insert(n) && table.columns.iter().any(|c| &c.name == n),
                "Invalid or duplicate key column {n}"
            );
            quote(n)
        })
        .collect::<Result<Vec<_>>>()
        .map(|v| v.join(", "))
}
fn action(value: &str) -> Result<&str> {
    ensure!(
        matches!(
            value,
            "NO ACTION" | "RESTRICT" | "CASCADE" | "SET NULL" | "SET DEFAULT"
        ),
        "Unsupported foreign key action"
    );
    Ok(value)
}
pub(super) fn table_sql(
    snapshot: &Snapshot,
    table: &Table,
    available: &[&Table],
) -> Result<(Vec<String>, Vec<String>)> {
    ensure!(
        table.unsupported.is_empty()
            && table.options.is_empty()
            && table.dependencies.is_empty()
            && !table.columns.is_empty(),
        "Unsupported creation metadata"
    );
    let namespace = quote(&snapshot.scope.namespace)?;
    let name = format!("{namespace}.{}", quote(&table.name)?);
    let mut definitions = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for c in &table.columns {
        ensure!(seen.insert(&c.name), "Duplicate column");
        ensure!(
            !c.generated && c.identity.is_none() && c.collation.is_none(),
            "Unsupported column metadata"
        );
        let ty = fragment(&c.data_type, &snapshot.engine, true)?;
        ensure!(
            snapshot.engine != Engine::Postgres || ty != "BLOB",
            "Unsupported PostgreSQL type"
        );
        let default = c
            .default
            .as_ref()
            .map(|d| fragment(d, &snapshot.engine, false).map(|d| format!(" DEFAULT ({d})")))
            .transpose()?
            .unwrap_or_default();
        definitions.push(format!(
            "{} {ty}{}{default}",
            quote(&c.name)?,
            if c.nullable { "" } else { " NOT NULL" }
        ));
    }
    let mut constraints = table.constraints.clone();
    let mut primary: Vec<_> = table
        .columns
        .iter()
        .filter(|c| c.primary_key_position > 0)
        .collect();
    primary.sort_by_key(|c| c.primary_key_position);
    if !primary.is_empty()
        && !constraints
            .iter()
            .any(|c| matches!(c, Constraint::Primary { .. }))
    {
        constraints.push(Constraint::Primary {
            columns: primary.iter().map(|c| c.name.clone()).collect(),
        });
    }
    ensure!(
        constraints
            .iter()
            .filter(|c| matches!(c, Constraint::Primary { .. }))
            .count()
            <= 1,
        "Duplicate primary key"
    );
    if !primary.is_empty() {
        let derived: Vec<_> = primary.iter().map(|c| c.name.clone()).collect();
        ensure!(
            primary
                .iter()
                .enumerate()
                .all(|(i, c)| c.primary_key_position == i as i64 + 1),
            "Invalid primary key positions"
        );
        ensure!(
            constraints
                .iter()
                .all(|c| !matches!(c, Constraint::Primary {columns} if columns != &derived)),
            "Conflicting primary key metadata"
        );
    }
    let mut deferred = Vec::new();
    let mut unique_definitions = std::collections::HashSet::new();
    for constraint in constraints {
        let sql = match constraint {
            Constraint::Primary { columns: cols } => {
                format!("PRIMARY KEY ({})", columns(&cols, table)?)
            }
            Constraint::Unique { columns: cols } => format!("UNIQUE ({})", columns(&cols, table)?),
            Constraint::Check { expression } => format!(
                "CHECK ({})",
                fragment(&expression, &snapshot.engine, false)?
            ),
            Constraint::ForeignKey {
                columns: cols,
                target,
                referenced_columns,
                on_update,
                on_delete,
                deferral,
            } => {
                let target = target
                    .strip_prefix(&format!("{}.", snapshot.scope.namespace))
                    .unwrap_or(&target);
                let parent = available.iter().find(|t| t.name == target).ok_or_else(|| {
                    anyhow::anyhow!("Missing selected or retained dependency {target}")
                })?;
                ensure!(
                    cols.len() == referenced_columns.len(),
                    "Foreign key arity mismatch"
                );
                let referenced = columns(&referenced_columns, parent)?;
                let mut pk: Vec<_> = parent
                    .columns
                    .iter()
                    .filter(|c| c.primary_key_position > 0)
                    .collect();
                pk.sort_by_key(|c| c.primary_key_position);
                ensure!(parent.constraints.iter().any(|c| matches!(c, Constraint::Primary {columns} | Constraint::Unique {columns} if columns == &referenced_columns)) || pk.iter().map(|c| c.name.clone()).collect::<Vec<_>>() == referenced_columns || parent.indexes.iter().any(|i| i.unique && i.columns == referenced_columns), "Referenced columns are not a unique key");
                let deferral =
                    deferral
                        .as_deref()
                        .unwrap_or(if snapshot.engine == Engine::Sqlite {
                            ""
                        } else {
                            "NOT DEFERRABLE"
                        });
                let deferral = match deferral {
                    "INITIALLY DEFERRED" => "DEFERRABLE INITIALLY DEFERRED",
                    "INITIALLY IMMEDIATE" => "DEFERRABLE INITIALLY IMMEDIATE",
                    other => other,
                };
                ensure!(
                    matches!(
                        deferral,
                        "" | "NOT DEFERRABLE"
                            | "DEFERRABLE"
                            | "DEFERRABLE INITIALLY DEFERRED"
                            | "DEFERRABLE INITIALLY IMMEDIATE"
                    ),
                    "Invalid deferral"
                );
                let target = if snapshot.engine == Engine::Postgres {
                    format!("{namespace}.{}", quote(target)?)
                } else {
                    quote(target)?
                };
                format!(
                    "FOREIGN KEY ({}) REFERENCES {target} ({referenced}) ON UPDATE {} ON DELETE {} {deferral}",
                    columns(&cols, table)?,
                    action(&on_update)?,
                    action(&on_delete)?
                )
            }
        };
        ensure!(
            unique_definitions.insert(sql.clone()),
            "Duplicate constraint"
        );
        if snapshot.engine == Engine::Postgres && sql.starts_with("FOREIGN KEY") {
            deferred.push(format!("ALTER TABLE {name} ADD {sql}"));
        } else {
            definitions.push(sql);
        }
    }
    let mut sql = vec![format!("CREATE TABLE {name} ({})", definitions.join(", "))];
    let mut indexes = std::collections::HashSet::new();
    for index in &table.indexes {
        if index.name.starts_with("automatic:") {
            continue;
        }
        ensure!(
            indexes.insert(&index.name) && index.definition.is_none(),
            "Duplicate or unsupported index definition"
        );
        let index_name = if snapshot.engine == Engine::Sqlite {
            format!("{namespace}.{}", quote(&index.name)?)
        } else {
            quote(&index.name)?
        };
        let target = if snapshot.engine == Engine::Sqlite {
            quote(&table.name)?
        } else {
            name.clone()
        };
        sql.push(format!(
            "CREATE {}INDEX {index_name} ON {target} ({})",
            if index.unique { "UNIQUE " } else { "" },
            columns(&index.columns, table)?
        ));
    }
    Ok((sql, deferred))
}
