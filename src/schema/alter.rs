use super::*;
use crate::sql::quote_identifier as quote;
use anyhow::{Result, ensure};

pub(super) fn columns(
    snapshot: &Snapshot,
    before: &Table,
    after: &Table,
    options: &PlanOptions,
) -> Result<Vec<String>> {
    ensure!(
        before.unsupported.is_empty()
            && after.unsupported.is_empty()
            && before.dependencies.is_empty()
            && after.dependencies.is_empty()
            && before.options == after.options,
        "Unsupported table metadata or dependencies"
    );
    ensure!(
        snapshot.engine == Engine::Postgres
            || (before.constraints == after.constraints && before.indexes == after.indexes),
        "Constraint and index alterations are unsupported"
    );
    let prefix = format!(
        "ALTER TABLE {}.{}",
        quote(&snapshot.scope.namespace)?,
        quote(&before.name)?
    );
    let mut old = before.columns.clone();
    let mut sql = Vec::new();
    if let Some(renames) = options.column_renames.get(&before.name) {
        ensure!(
            before.constraints.is_empty() && before.indexes.is_empty(),
            "Rename requires dependency coverage"
        );
        for (from, to) in renames {
            ensure!(!old.iter().any(|c| &c.name == to), "Rename target exists");
            let column = old
                .iter_mut()
                .find(|c| &c.name == from)
                .ok_or_else(|| anyhow::anyhow!("Rename source missing"))?;
            ensure!(
                after.columns.iter().any(|c| &c.name == to),
                "Rename target missing from desired schema"
            );
            sql.push(format!(
                "{prefix} RENAME COLUMN {} TO {}",
                quote(from)?,
                quote(to)?
            ));
            column.name = to.clone();
        }
    }
    for column in &old {
        if !after.columns.iter().any(|c| c.name == column.name) {
            ensure!(
                options.allow_destructive,
                "Dropping columns requires destructive opt-in"
            );
            ensure!(
                before.constraints.is_empty() && before.indexes.is_empty(),
                "Drop requires dependency coverage"
            );
            sql.push(format!("{prefix} DROP COLUMN {}", quote(&column.name)?));
        }
    }
    for column in &after.columns {
        ensure!(
            !column.generated && column.identity.is_none() && column.collation.is_none(),
            "Unsupported column metadata"
        );
        let name = quote(&column.name)?;
        let ty = create::fragment(&column.data_type, &snapshot.engine, true)?;
        let default = column
            .default
            .as_ref()
            .map(|d| create::fragment(d, &snapshot.engine, false))
            .transpose()?;
        if let Some(previous) = old.iter().find(|c| c.name == column.name) {
            ensure!(
                previous.primary_key_position == column.primary_key_position
                    && previous.generated == column.generated
                    && previous.identity == column.identity
                    && previous.collation == column.collation,
                "Unsupported column metadata change"
            );
            if previous != column {
                ensure!(
                    snapshot.engine == Engine::Postgres,
                    "SQLite column changes require a rebuild"
                );
                if previous.data_type != column.data_type {
                    ensure!(
                        options.allow_destructive,
                        "Type changes require destructive opt-in"
                    );
                    sql.push(format!("{prefix} ALTER COLUMN {name} TYPE {ty}"));
                }
                if previous.default != column.default {
                    sql.push(format!(
                        "{prefix} ALTER COLUMN {name} {}",
                        default
                            .map(|d| format!("SET DEFAULT {d}"))
                            .unwrap_or_else(|| "DROP DEFAULT".into())
                    ));
                }
                if previous.nullable != column.nullable {
                    sql.push(format!(
                        "{prefix} ALTER COLUMN {name} {} NOT NULL",
                        if column.nullable { "DROP" } else { "SET" }
                    ));
                }
            }
        } else {
            ensure!(
                column.primary_key_position == 0,
                "Adding primary keys is unsupported"
            );
            sql.push(format!(
                "{prefix} ADD COLUMN {name} {ty}{}{}",
                if column.nullable { "" } else { " NOT NULL" },
                default.map(|d| format!(" DEFAULT {d}")).unwrap_or_default()
            ));
        }
    }
    if snapshot.engine == Engine::Postgres {
        let changed_constraints = before.constraints != after.constraints;
        let independent = |table: &Table| {
            table
                .indexes
                .iter()
                .filter(|i| !i.name.starts_with("automatic:"))
                .cloned()
                .collect::<Vec<_>>()
        };
        let old_indexes = independent(before);
        let new_indexes = independent(after);
        if changed_constraints || old_indexes != new_indexes {
            ensure!(
                options.allow_destructive,
                "Constraint and index replacement requires destructive opt-in"
            );
            let mut drops = Vec::new();
            if changed_constraints {
                let names = &before.constraint_names;
                ensure!(
                    names.len() == before.constraints.len(),
                    "Missing catalog constraint names"
                );
                for (name, constraint) in names.iter().zip(&before.constraints) {
                    if after.constraints.contains(constraint) {
                        continue;
                    }
                    drops.push(format!("{prefix} DROP CONSTRAINT {}", quote(name)?));
                }
            }
            for index in &old_indexes {
                if !new_indexes.contains(index) {
                    drops.push(format!(
                        "DROP INDEX {}.{}",
                        quote(&snapshot.scope.namespace)?,
                        quote(&index.name)?
                    ));
                }
            }
            drops.extend(sql);
            sql = drops;
            let mut creation = after.clone();
            creation.constraints = if changed_constraints {
                after
                    .constraints
                    .iter()
                    .filter(|c| !before.constraints.contains(c))
                    .cloned()
                    .collect()
            } else {
                vec![]
            };
            creation.indexes = new_indexes
                .into_iter()
                .filter(|i| !old_indexes.contains(i))
                .collect();
            // Reuse the validated CREATE renderer, then turn its table definitions into ADD clauses.
            for column in &mut creation.columns {
                column.primary_key_position = 0;
            }
            let available = snapshot
                .tables
                .iter()
                .filter(|t| t.name != after.name)
                .chain(std::iter::once(after))
                .collect::<Vec<_>>();
            let (statements, deferred) = create::table_sql(snapshot, &creation, &available)?;
            let parsed = sqlparser::parser::Parser::parse_sql(
                &sqlparser::dialect::PostgreSqlDialect {},
                &statements[0],
            )?;
            if let sqlparser::ast::Statement::CreateTable(table) = &parsed[0] {
                for constraint in &table.constraints {
                    sql.push(format!("{prefix} ADD {constraint}"));
                }
            }
            sql.extend(statements.into_iter().skip(1));
            sql.extend(deferred);
        }
    }
    Ok(sql)
}
