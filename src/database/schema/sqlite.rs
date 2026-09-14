use super::*;
pub(super) fn inspect(conn: &rusqlite::Connection, scope: Scope) -> Result<Snapshot> {
    let mut statement = conn.prepare(&format!("SELECT name, sql, type FROM {}.sqlite_schema WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name", quote_identifier(&scope.namespace)?))?;
    let definitions = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut tables = Vec::new();
    for (name, definition, kind) in definitions {
        let mut stmt = conn.prepare(&format!(
            "PRAGMA {}.table_xinfo({})",
            quote_identifier(&scope.namespace)?,
            quote_identifier(&name)?
        ))?;
        let mut columns = stmt
            .query_map([], |r| {
                Ok(Column {
                    name: r.get(1)?,
                    data_type: r.get(2)?,
                    nullable: r.get::<_, i64>(3)? == 0,
                    default: r.get(4)?,
                    primary_key_position: r.get(5)?,
                    generated: r.get::<_, i64>(6)? != 0,
                    identity: None,
                    collation: None,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut unsupported = Vec::new();
        if kind != "table" {
            unsupported.push(format!("Unsupported object kind: {kind}"));
        }
        let mut constraints = Vec::new();
        let mut primary: Vec<_> = columns
            .iter()
            .filter(|c| c.primary_key_position > 0)
            .collect();
        primary.sort_by_key(|c| c.primary_key_position);
        if !primary.is_empty() {
            constraints.push(Constraint::Primary {
                columns: primary.iter().map(|c| c.name.clone()).collect(),
            });
        }
        let schema = quote_identifier(&scope.namespace)?;
        let quoted = quote_identifier(&name)?;
        let mut indexes = Vec::new();
        let mut stmt = conn.prepare(&format!("PRAGMA {schema}.index_list({quoted})"))?;
        let entries = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, bool>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (index_name, unique, origin, partial) in entries {
            let mut stmt = conn.prepare(&format!(
                "PRAGMA {schema}.index_xinfo({})",
                quote_identifier(&index_name)?
            ))?;
            let parts = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, bool>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, bool>(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let ordinary = !partial
                && parts
                    .iter()
                    .filter(|p| p.3)
                    .all(|p| p.0.is_some() && !p.1 && p.2.eq_ignore_ascii_case("BINARY"));
            let columns: Vec<String> = parts
                .iter()
                .filter(|p| p.3)
                .map(|p| {
                    if ordinary {
                        return p.0.clone().unwrap();
                    }
                    format!(
                        "{} {} COLLATE {}",
                        p.0.as_deref().unwrap_or("<expression>"),
                        if p.1 { "DESC" } else { "ASC" },
                        p.2
                    )
                })
                .collect();
            if unique && origin == "u" {
                constraints.push(Constraint::Unique {
                    columns: parts
                        .iter()
                        .filter(|p| p.3)
                        .filter_map(|p| p.0.clone())
                        .collect(),
                });
            }
            let ddl = conn.query_row(
                &format!("SELECT sql FROM {schema}.sqlite_schema WHERE name=?1"),
                [&index_name],
                |r| r.get(0),
            )?;
            indexes.push(Index {
                name: if origin == "c" {
                    index_name
                } else {
                    format!("automatic:{origin}:{}", columns.join(","))
                },
                unique,
                columns,
                definition: if ordinary { None } else { ddl },
            });
        }
        indexes.sort_by(|a, b| a.name.cmp(&b.name));
        let mut stmt = conn.prepare(&format!("PRAGMA {schema}.foreign_key_list({quoted})"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut groups = std::collections::BTreeMap::new();
        for row in rows {
            groups.entry(row.0).or_insert_with(Vec::new).push(row);
        }
        for mut rows in groups.into_values() {
            rows.sort_by_key(|r| r.1);
            constraints.push(Constraint::ForeignKey {
                columns: rows.iter().map(|r| r.3.clone()).collect(),
                target: rows[0].2.clone(),
                referenced_columns: rows
                    .iter()
                    .map(|r| r.4.clone().unwrap_or_default())
                    .collect(),
                on_update: rows[0].5.clone(),
                on_delete: rows[0].6.clone(),
                deferral: None,
            });
        }
        let mut options = Vec::new();
        if let Some(ddl) = &definition {
            match sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::SQLiteDialect {}, ddl) {
                Ok(ast) => {
                    // Preserve all parsed clauses, including column checks, collations and generated expressions.
                    for statement in &ast {
                        if let sqlparser::ast::Statement::CreateTable(table) = statement {
                            for constraint in &table.constraints {
                                if let sqlparser::ast::TableConstraint::Check(check) = constraint {
                                    constraints.push(Constraint::Check {
                                        expression: check.expr.to_string(),
                                    });
                                }
                            }
                            for column in &table.columns {
                                if let Some(inspected) =
                                    columns.iter_mut().find(|c| c.name == column.name.value)
                                {
                                    inspected.collation =
                                        column.options.iter().find_map(|o| match &o.option {
                                            sqlparser::ast::ColumnOption::Collation(name) => {
                                                Some(name.to_string())
                                            }
                                            _ => None,
                                        });
                                }
                                for option in &column.options {
                                    let clause = option.to_string().to_uppercase();
                                    if clause.contains("AUTOINCREMENT")
                                        || clause.contains("ON CONFLICT")
                                        || !(clause == "NULL"
                                            || clause == "NOT NULL"
                                            || clause.starts_with("DEFAULT ")
                                            || clause.starts_with("PRIMARY KEY")
                                            || clause.starts_with("UNIQUE")
                                            || clause.starts_with("REFERENCES ")
                                            || clause.starts_with("CHECK ")
                                            || clause.starts_with("COLLATE "))
                                    {
                                        unsupported
                                            .push(format!("Unsupported column clause: {option}"));
                                    }
                                    if let sqlparser::ast::ColumnOption::Check(check) =
                                        &option.option
                                    {
                                        constraints.push(Constraint::Check {
                                            expression: check.expr.to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    options = ast
                        .into_iter()
                        .map(|mut statement| {
                            if let sqlparser::ast::Statement::CreateTable(table) = &mut statement {
                                table.columns.clear();
                                table.constraints.clear();
                            }
                            statement.to_string()
                        })
                        .collect();
                }
                Err(error) => unsupported.push(format!("Unparsed SQLite definition: {error}")),
            }
            if ddl.to_uppercase().contains("DEFERRABLE") {
                unsupported.push("SQLite FK deferral mapping is not yet resolved".into());
            }
        } else {
            unsupported.push("Missing object definition".into());
        }
        let ordinary = format!("CREATE TABLE {} ()", quote_identifier(&name)?);
        if let Ok(ast) =
            sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::SQLiteDialect {}, &ordinary)
        {
            let ordinary = ast[0].to_string().replace(&quote_identifier(&name)?, &name);
            options.retain(|option| {
                option.replace(&quote_identifier(&name).unwrap_or_default(), &name) != ordinary
            });
        }
        let mut stmt = conn.prepare(&format!("SELECT sql FROM {schema}.sqlite_schema WHERE type='trigger' AND tbl_name=?1 ORDER BY name"))?;
        let dependencies = stmt
            .query_map([&name], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        tables.push(Table {
            constraint_names: Vec::new(),
            name,
            columns,
            definition,
            indexes,
            constraints,
            dependencies,
            options,
            unsupported,
        });
    }
    Ok(Snapshot {
        engine: Engine::Sqlite,
        engine_version: conn.query_row("SELECT sqlite_version()", [], |r| r.get(0))?,
        scope,
        tables,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspect_constraints_and_dependents() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE parent(a INTEGER,b INTEGER,PRIMARY KEY(a,b)); CREATE TABLE child(x INTEGER DEFAULT 4,y INTEGER CHECK(y > 0), FOREIGN KEY(x,y) REFERENCES parent(a,b) ON DELETE CASCADE); CREATE UNIQUE INDEX child_unique ON child(y,x); CREATE TRIGGER child_insert AFTER INSERT ON child BEGIN SELECT 1; END; CREATE VIEW child_view AS SELECT * FROM child;").unwrap();
        let snapshot = inspect(
            &conn,
            Scope {
                namespace: "main".into(),
            },
        )
        .unwrap();
        let child = snapshot.tables.iter().find(|t| t.name == "child").unwrap();
        assert!(child.unsupported.is_empty(), "{:?}", child.unsupported);
        assert_eq!(child.columns[0].default.as_deref(), Some("4"));
        assert!(child.indexes[0].unique);
        assert_eq!(child.dependencies.len(), 1);
        assert!(
            child
                .constraints
                .iter()
                .any(|c| matches!(c, Constraint::Check { .. }))
        );
        assert!(child.constraints.iter().any(|c| matches!(c, Constraint::ForeignKey {columns,referenced_columns,on_delete,..} if columns == &["x","y"] && referenced_columns == &["a","b"] && on_delete == "CASCADE")));
        assert!(
            !snapshot
                .tables
                .iter()
                .find(|t| t.name == "child_view")
                .unwrap()
                .unsupported
                .is_empty()
        );
    }
}
