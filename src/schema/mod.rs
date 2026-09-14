mod alter;
mod create;
mod rebuild;
// Persistent schema inspection and conservative change planning.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Engine {
    Postgres,
    Sqlite,
}
/// One named namespace: a PostgreSQL schema or SQLite attached database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub namespace: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key_position: i64,
    pub generated: bool,
    pub identity: Option<String>,
    pub collation: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    pub unique: bool,
    pub columns: Vec<String>,
    pub definition: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Constraint {
    Primary {
        columns: Vec<String>,
    },
    Unique {
        columns: Vec<String>,
    },
    Check {
        expression: String,
    },
    ForeignKey {
        columns: Vec<String>,
        target: String,
        referenced_columns: Vec<String>,
        on_update: String,
        on_delete: String,
        deferral: Option<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    #[serde(default)]
    pub constraint_names: Vec<String>,
    pub name: String,
    pub columns: Vec<Column>,
    /// Original SQLite table DDL; PostgreSQL metadata uses structured fields.
    pub definition: Option<String>,
    pub indexes: Vec<Index>,
    pub constraints: Vec<Constraint>,
    pub dependencies: Vec<String>,
    pub options: Vec<String>,
    pub unsupported: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub engine: Engine,
    pub engine_version: String,
    pub scope: Scope,
    pub tables: Vec<Table>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Difference {
    Added(String),
    Removed(String),
    Changed(String),
    Unknown(String),
}
pub fn compare(baseline: &Snapshot, desired: &Snapshot) -> Vec<Difference> {
    if baseline.engine != desired.engine || baseline.scope != desired.scope {
        return vec![Difference::Unknown("Engine or scope mismatch".into())];
    }
    let mut result = Vec::new();
    for table in &baseline.tables {
        match desired.tables.iter().find(|t| t.name == table.name) {
            None => result.push(Difference::Removed(table.name.clone())),
            Some(other) if !table.unsupported.is_empty() || !other.unsupported.is_empty() => {
                result.push(Difference::Unknown(table.name.clone()))
            }
            Some(other) if !equivalent(other, table, &baseline.engine) => {
                result.push(Difference::Changed(table.name.clone()))
            }
            _ => {}
        }
    }
    for table in &desired.tables {
        if !baseline.tables.iter().any(|t| t.name == table.name) {
            result.push(Difference::Added(table.name.clone()));
        }
    }
    result
}
fn canonical_sql(sql: &str) -> String {
    sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::GenericDialect {}, sql)
        .map(|statements| {
            statements
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(";")
        })
        .unwrap_or_else(|_| sql.to_owned())
}
fn equivalent(left: &Table, right: &Table, engine: &Engine) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    // Raw source formatting is not schema identity. Parser rendering preserves literals.
    for table in [&mut left, &mut right] {
        table.constraint_names.clear();
        if *engine == Engine::Postgres {
            table.definition = None;
        }
        for column in &mut table.columns {
            if *engine == Engine::Postgres {
                column.data_type = match column.data_type.to_ascii_lowercase().as_str() {
                    "int" | "int4" | "integer" => "integer".into(),
                    "int2" | "smallint" => "smallint".into(),
                    "int8" | "bigint" => "bigint".into(),
                    "bool" | "boolean" => "boolean".into(),
                    _ => column.data_type.clone(),
                };
            }
            if let Some(Constraint::Primary { columns }) = table
                .constraints
                .iter()
                .find(|c| matches!(c, Constraint::Primary { .. }))
                && let Some(position) = columns.iter().position(|name| name == &column.name)
            {
                column.primary_key_position = position as i64 + 1;
                if *engine == Engine::Postgres {
                    column.nullable = false;
                }
            }
        }
        table
            .indexes
            .retain(|index| !index.name.starts_with("automatic:"));
        table.indexes.sort_by(|a, b| a.name.cmp(&b.name));
        table.constraints.sort_by_key(|c| format!("{c:?}"));
        for index in &mut table.indexes {
            index.definition = index.definition.as_ref().map(|sql| canonical_sql(sql));
        }
        table.dependencies = table
            .dependencies
            .iter()
            .map(|sql| canonical_sql(sql))
            .collect();
        table.definition = table.definition.as_ref().map(|sql| {
            sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::GenericDialect {}, sql)
                .map(|statements| {
                    statements
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(";")
                })
                .unwrap_or_else(|_| sql.clone())
        });
    }
    left == right
}
/// Immutable plan: SQL can only originate from the planner.
#[derive(Clone, Debug)]
pub struct Plan {
    pub(crate) baseline: Snapshot,
    pub(crate) sql: Vec<String>,
    pub blocked: Vec<String>,
    pub risks: Vec<String>,
    pub expected: Snapshot,
}
impl Plan {
    pub fn sql(&self) -> &[String] {
        if self.blocked.is_empty() {
            &self.sql
        } else {
            &[]
        }
    }
}
/// Creates ordinary tables whose complete definition is modeled.
/// Existing-table changes remain blocked until their dependents are modeled.
#[derive(Clone, Debug, Default)]
pub struct PlanOptions {
    /// Only these table differences are included; None selects all.
    pub selected_tables: Option<Vec<String>>,
    pub allow_destructive: bool,
    pub table_renames: std::collections::BTreeMap<String, String>,
    pub column_renames:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
}
pub fn plan(baseline: &Snapshot, desired: &Snapshot) -> anyhow::Result<Plan> {
    plan_with_options(baseline, desired, &PlanOptions::default())
}
pub fn plan_with_options(
    baseline: &Snapshot,
    desired: &Snapshot,
    options: &PlanOptions,
) -> anyhow::Result<Plan> {
    anyhow::ensure!(
        baseline.engine == desired.engine && baseline.scope == desired.scope,
        "Engine or scope mismatch"
    );
    let mut plan = Plan {
        baseline: baseline.clone(),
        sql: vec![],
        blocked: vec![],
        risks: vec![],
        expected: baseline.clone(),
    };
    for snapshot in [baseline, desired] {
        let mut names = std::collections::HashSet::new();
        anyhow::ensure!(
            snapshot.tables.iter().all(|t| names.insert(&t.name)),
            "Duplicate table definitions"
        );
    }
    let mut renamed = baseline.clone();
    for (from, to) in &options.table_renames {
        anyhow::ensure!(
            baseline.engine == Engine::Postgres,
            "Table rename is only supported for PostgreSQL"
        );
        if options
            .selected_tables
            .as_ref()
            .is_some_and(|names| !names.contains(from) && !names.contains(to))
        {
            continue;
        }
        anyhow::ensure!(
            !baseline.tables.iter().any(|t| &t.name == to),
            "Rename target exists"
        );
        anyhow::ensure!(
            desired.tables.iter().any(|t| &t.name == to)
                && !desired.tables.iter().any(|t| &t.name == from),
            "Invalid desired rename"
        );
        let table = renamed
            .tables
            .iter_mut()
            .find(|t| &t.name == from)
            .ok_or_else(|| anyhow::anyhow!("Rename source missing"))?;
        if !table.dependencies.is_empty()
            || !table.unsupported.is_empty()
            || !table.indexes.is_empty()
            || !table.constraints.is_empty()
        {
            plan.blocked.push(format!(
                "{from}: rename requires complete dependency coverage"
            ));
            continue;
        }
        plan.sql.push(format!(
            "ALTER TABLE {}.{} RENAME TO {}",
            crate::sql::quote_identifier(&baseline.scope.namespace)?,
            crate::sql::quote_identifier(from)?,
            crate::sql::quote_identifier(to)?
        ));
        table.name = to.clone();
        plan.expected
            .tables
            .iter_mut()
            .find(|t| &t.name == from)
            .unwrap()
            .name = to.clone();
    }
    let baseline = &renamed;
    let mut foreign_keys = Vec::new();
    for difference in compare(baseline, desired) {
        let name = match &difference {
            Difference::Added(n)
            | Difference::Removed(n)
            | Difference::Changed(n)
            | Difference::Unknown(n) => n,
        };
        if options
            .selected_tables
            .as_ref()
            .is_some_and(|selected| !selected.contains(name))
        {
            continue;
        }
        match difference {
            Difference::Added(name) => {
                let table = desired.tables.iter().find(|t| t.name == name).unwrap();
                let available: Vec<&Table> = baseline
                    .tables
                    .iter()
                    .chain(desired.tables.iter().filter(|t| {
                        !baseline.tables.iter().any(|b| b.name == t.name)
                            && options
                                .selected_tables
                                .as_ref()
                                .is_none_or(|names| names.contains(&t.name))
                    }))
                    .collect();
                match create::table_sql(baseline, table, &available) {
                    Ok((sql, deferred)) => {
                        plan.sql.extend(sql);
                        foreign_keys.extend(deferred);
                    }
                    Err(error) => {
                        plan.blocked.push(format!("{name}: {error}"));
                        continue;
                    }
                }
                plan.expected.tables.push(table.clone());
            }
            Difference::Changed(name) => {
                let before = baseline.tables.iter().find(|t| t.name == name).unwrap();
                let after = desired.tables.iter().find(|t| t.name == name).unwrap();
                match if baseline.engine == Engine::Sqlite {
                    alter::columns(baseline, before, after, options)
                        .or_else(|_| rebuild::table(baseline, before, after, options))
                } else {
                    alter::columns(baseline, before, after, options)
                } {
                    Ok(sql) => {
                        plan.sql.extend(sql);
                        *plan
                            .expected
                            .tables
                            .iter_mut()
                            .find(|t| t.name == name)
                            .unwrap() = after.clone();
                    }
                    Err(error) => plan.blocked.push(format!("{name}: {error}")),
                }
            }
            Difference::Removed(name)
                if baseline.engine == Engine::Postgres && options.allow_destructive =>
            {
                let table = baseline.tables.iter().find(|t| t.name == name).unwrap();
                if !table.dependencies.is_empty() || !table.unsupported.is_empty() {
                    plan.blocked.push(format!(
                        "{name}: drop requires complete dependency coverage"
                    ));
                } else {
                    plan.sql.push(format!(
                        "DROP TABLE {}.{}",
                        crate::sql::quote_identifier(&baseline.scope.namespace)?,
                        crate::sql::quote_identifier(&name)?
                    ));
                    plan.expected.tables.retain(|t| t.name != name);
                }
            }
            difference => plan.blocked.push(format!(
                "{difference:?}: alteration requires complete dependent coverage"
            )),
        }
    }
    let mut index_names = std::collections::HashSet::new();
    for table in &plan.expected.tables {
        for index in &table.indexes {
            if !index_names.insert(&index.name) {
                plan.blocked
                    .push(format!("Duplicate index name: {}", index.name));
            }
        }
    }
    plan.sql.extend(foreign_keys);
    Ok(plan)
}

/// Verify modeled definitions before committing DDL. Unchanged objects retain exact metadata checks.
pub fn verify_expected(expected: &Snapshot, actual: &Snapshot) -> anyhow::Result<()> {
    anyhow::ensure!(
        expected.engine == actual.engine
            && expected.scope == actual.scope
            && expected.tables.len() == actual.tables.len(),
        "Schema verification mismatch"
    );
    for table in &expected.tables {
        let other = actual
            .tables
            .iter()
            .find(|t| t.name == table.name)
            .ok_or_else(|| anyhow::anyhow!("Missing expected table {}", table.name))?;
        let mut left = table.clone();
        let mut right = other.clone();
        left.definition = None;
        right.definition = None;
        anyhow::ensure!(
            equivalent(&left, &right, &expected.engine),
            "Schema verification mismatch for {}",
            table.name
        );
    }
    Ok(())
}

/// Map a PostgreSQL snapshot to a destination namespace without rewriting raw SQL.
/// Definitions that need expression-level name resolution remain explicitly blocked.
pub fn map_scope(source: &Snapshot, target: Scope) -> anyhow::Result<Snapshot> {
    anyhow::ensure!(
        source.engine == Engine::Postgres,
        "Namespace mapping requires PostgreSQL"
    );
    if source.scope == target {
        return Ok(source.clone());
    }
    let mut mapped = source.clone();
    for table in &mut mapped.tables {
        anyhow::ensure!(
            table.dependencies.is_empty() && table.unsupported.is_empty(),
            "{}: unresolved raw definitions or dependencies",
            table.name
        );
        for column in &table.columns {
            // Defaults may contain regclass literals, functions or qualified casts.
            // Do not infer their resolution from their spelling.
            anyhow::ensure!(
                column.default.is_none()
                    && column.collation.is_none()
                    && column.identity.is_none()
                    && !column.generated,
                "{}: namespace-dependent column metadata",
                column.name
            );
            let ty = column.data_type.to_ascii_lowercase();
            anyhow::ensure!(
                matches!(
                    ty.as_str(),
                    "smallint"
                        | "integer"
                        | "bigint"
                        | "int2"
                        | "int4"
                        | "int8"
                        | "int"
                        | "text"
                        | "boolean"
                        | "bool"
                        | "real"
                        | "double precision"
                        | "date"
                        | "uuid"
                        | "bytea"
                        | "json"
                        | "jsonb"
                        | "numeric"
                        | "timestamp without time zone"
                        | "timestamp with time zone"
                ),
                "{}: unresolved type namespace",
                column.name
            );
        }
        anyhow::ensure!(
            table
                .indexes
                .iter()
                .all(|i| i.name.starts_with("automatic:") || i.definition.is_none()),
            "{}: unresolved index definition",
            table.name
        );
        for constraint in &mut table.constraints {
            match constraint {
                Constraint::ForeignKey {
                    target: reference, ..
                } => {
                    let prefix = format!("{}.", source.scope.namespace);
                    let name = reference.strip_prefix(&prefix).unwrap_or(reference);
                    anyhow::ensure!(
                        source.tables.iter().any(|t| t.name == name),
                        "Unresolved foreign key namespace"
                    );
                    *reference = format!("{}.{}", target.namespace, name);
                }
                Constraint::Check { .. } => anyhow::bail!("Unresolved check expression namespace"),
                _ => {}
            }
        }
    }
    mapped.scope = target;
    Ok(mapped)
}
