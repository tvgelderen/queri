use anyhow::Result;
use queri::{
    database::{Session, runtime},
    model::*,
    storage::Store,
};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

fn sqlite_profile(path: std::path::PathBuf) -> ConnectionProfile {
    ConnectionProfile {
        ssh: None,
        environment: None,
        policy: ExecutionPolicy::Normal,
        remember_password: false,
        id: Uuid::new_v4(),
        name: "Fixture".into(),
        notes: String::new(),
        color: String::new(),
        config: ConnectionConfig::Sqlite {
            path,
            read_only: false,
            create: true,
            wal: true,
            foreign_keys: true,
        },
    }
}

fn table() -> TableRef {
    TableRef {
        schema: "main".into(),
        name: "items".into(),
        kind: RelationKind::Table,
    }
}
fn page() -> BrowseOptions {
    BrowseOptions {
        limit: 25,
        ..Default::default()
    }
}

async fn fixture() -> Result<Arc<Session>> {
    let session = Session::connect(sqlite_profile(":memory:".into()), String::new()).await?;
    session.query("CREATE TABLE items (tenant INTEGER, id INTEGER, name TEXT NOT NULL DEFAULT 'New', quantity INTEGER, payload BLOB, PRIMARY KEY(tenant,id)); CREATE INDEX items_name ON items(name); INSERT INTO items VALUES(1,1,'Alpha',10,X'00ff'),(1,2,'Beta',20,NULL),(2,1,'Alpine',30,NULL)".into()).await?;
    Ok(session)
}

#[test]
fn sqlite_creation_never_overwrites_an_existing_file() -> Result<()> {
    runtime()?.block_on(async {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("new.sqlite");
        queri::database::create_sqlite_file(path.clone()).await?;
        let session = Session::connect(sqlite_profile(path.clone()), String::new()).await?;
        session
            .query("CREATE TABLE retained(value TEXT); INSERT INTO retained VALUES('kept');".into())
            .await?;
        assert!(queri::database::create_sqlite_file(path).await.is_err());
        assert_eq!(
            session
                .query("SELECT value FROM retained".into())
                .await?
                .sets[0]
                .rows,
            vec![vec![Cell::Text("kept".into())]]
        );
        Ok(())
    })
}

#[test]
fn sqlite_metadata_filter_sort_pagination_and_exact_values() -> Result<()> {
    runtime()?.block_on(async {
        let session = fixture().await?;
        let data = session
            .browse(
                table(),
                BrowseOptions {
                    limit: 1,
                    offset: 1,
                    filter: Some(
                        TableFilter {
                            column: "name".into(),
                            predicate: FilterPredicate::Contains("al".into()),
                        }
                        .into(),
                    ),
                    sort: vec![("quantity".into(), true)],
                    relationship: None,
                },
            )
            .await?;
        assert_eq!(data.total, 2);
        assert_eq!(data.data.rows[0][2], Cell::Text("Alpha".into()));
        assert_eq!(data.data.rows[0][4], Cell::Blob(vec![0, 255]));
        assert_eq!(
            data.metadata
                .columns
                .iter()
                .filter(|col| col.primary_key)
                .count(),
            2
        );
        assert!(
            data.metadata
                .indexes
                .rows
                .iter()
                .any(|row| row[0].text() == "items_name")
        );
        session
            .query("CREATE VIEW item_view AS SELECT * FROM items".into())
            .await?;
        let relations = session.tables("main".into()).await?;
        assert_eq!(relations.len(), 2);
        assert_eq!(relations[0].kind, RelationKind::View);
        assert!(!relations[0].is_table());
        assert_eq!(relations[1], table());
        assert!(
            !session
                .browse(relations[0].clone(), page())
                .await?
                .metadata
                .editable(&relations[0])
        );
        Ok(())
    })
}

async fn filter_workflows(session: &Session, schema: &str) -> Result<()> {
    use FilterPredicate::*;
    let table = TableRef {
        schema: schema.into(),
        name: "filter\"items".into(),
        kind: RelationKind::Table,
    };
    let qualified = queri::sql::qualified(schema, &table.name)?;
    session.query(format!(
        "CREATE TABLE {qualified}(id INTEGER PRIMARY KEY, \"label\"\"quoted\" TEXT, amount NUMERIC(30,2), happened TIMESTAMP WITH TIME ZONE, enabled BOOLEAN);
         INSERT INTO {qualified} VALUES
         (1,'Alpha',2,'2026-01-02T03:00:00Z',true),
         (2,'Alpine',10,'2026-01-10T01:00:00+02:00',false),
         (3,'Beta',100,'2026-02-01T00:00:00Z',true),
         (4,'',0,NULL,false),
         (5,NULL,NULL,NULL,NULL),
         (6,'A%_''z',-3,'2025-12-31T00:00:00Z',NULL);"
    )).await?;
    let label = "label\"quoted";
    let cases = vec![
        (label, Contains("AL".into()), vec![1, 2]),
        (label, NotContains("AL".into()), vec![3, 4, 6]),
        (label, Equals("alpha".into()), vec![1]),
        (label, NotEquals("ALPHA".into()), vec![2, 3, 4, 6]),
        (label, StartsWith("al".into()), vec![1, 2]),
        (label, EndsWith("HA".into()), vec![1]),
        (label, Contains("%_".into()), vec![6]),
        (label, StartsWith("a%".into()), vec![6]),
        (label, EndsWith("_'z".into()), vec![6]),
        (label, Equals("".into()), vec![4]),
        (label, NotEquals("".into()), vec![1, 2, 3, 6]),
        (label, StartsWith("".into()), vec![1, 2, 3, 4, 6]),
        (label, EndsWith("".into()), vec![1, 2, 3, 4, 6]),
        (label, Contains("' OR 1=1 --".into()), vec![]),
        (label, IsNull, vec![5]),
        (label, IsNotNull, vec![1, 2, 3, 4, 6]),
        ("amount", Equals("10".into()), vec![2]),
        ("amount", NotEquals("10".into()), vec![1, 3, 4, 6]),
        ("amount", GreaterThan("2".into()), vec![2, 3]),
        ("amount", GreaterOrEqual("2".into()), vec![1, 2, 3]),
        ("amount", LessThan("2".into()), vec![4, 6]),
        ("amount", LessOrEqual("2".into()), vec![1, 4, 6]),
        ("amount", IsNull, vec![5]),
        ("amount", IsNotNull, vec![1, 2, 3, 4, 6]),
        ("happened", Equals("2026-01-09T23:00:00Z".into()), vec![2]),
        (
            "happened",
            NotEquals("2026-01-09T23:00:00Z".into()),
            vec![1, 3, 6],
        ),
        (
            "happened",
            GreaterThan("2026-01-10T00:00:00Z".into()),
            vec![3],
        ),
        (
            "happened",
            GreaterOrEqual("2026-01-09T23:00:00Z".into()),
            vec![2, 3],
        ),
        (
            "happened",
            LessThan("2026-01-10T00:00:00Z".into()),
            vec![1, 2, 6],
        ),
        (
            "happened",
            LessOrEqual("2026-01-09T23:00:00Z".into()),
            vec![1, 2, 6],
        ),
        ("happened", IsNull, vec![4, 5]),
        ("happened", IsNotNull, vec![1, 2, 3, 6]),
        ("enabled", Equals("true".into()), vec![1, 3]),
        ("enabled", NotEquals("false".into()), vec![1, 3]),
        ("enabled", IsNull, vec![5, 6]),
    ];
    for (column, predicate, expected) in cases {
        let description = format!("{column} {predicate:?}");
        let data = session
            .browse(
                table.clone(),
                BrowseOptions {
                    filter: Some(
                        TableFilter {
                            column: column.into(),
                            predicate,
                        }
                        .into(),
                    ),
                    ..page()
                },
            )
            .await?;
        assert_eq!(data.total, u64::try_from(expected.len())?, "{description}");
        let ids = data
            .data
            .rows
            .iter()
            .map(|row| row[0].text())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            expected.iter().map(i32::to_string).collect::<Vec<_>>(),
            "{description}"
        );
    }
    let paged = session
        .browse(
            table.clone(),
            BrowseOptions {
                limit: 1,
                offset: 1,
                filter: Some(
                    TableFilter {
                        column: label.into(),
                        predicate: StartsWith("al".into()),
                    }
                    .into(),
                ),
                sort: vec![("id".into(), true)],
                relationship: None,
            },
        )
        .await?;
    assert_eq!(paged.total, 2);
    assert_eq!(paged.data.rows[0][0].text(), "1");

    for (column, predicate) in [
        ("missing", IsNull),
        ("amount", Contains("2".into())),
        ("amount", Equals("not a number".into())),
        ("amount", GreaterThan("".into())),
        ("happened", LessThan("invalid date".into())),
        ("happened", Equals("2026-02-30".into())),
        ("enabled", Equals("maybe".into())),
    ] {
        assert!(
            session
                .browse(
                    table.clone(),
                    BrowseOptions {
                        filter: Some(
                            TableFilter {
                                column: column.into(),
                                predicate
                            }
                            .into()
                        ),
                        ..page()
                    }
                )
                .await
                .is_err(),
            "reject invalid filter for {column}"
        );
    }
    assert_eq!(session.browse(table, page()).await?.total, 6);
    Ok(())
}

#[test]
fn sqlite_type_aware_filters() -> Result<()> {
    runtime()?.block_on(async {
        let session = fixture().await?;
        filter_workflows(&session, "main").await?;
        let data = session
            .browse(
                table(),
                BrowseOptions {
                    filter: Some(
                        TableFilter {
                            column: "quantity".into(),
                            predicate: FilterPredicate::GreaterThan("20".into()),
                        }
                        .into(),
                    ),
                    ..page()
                },
            )
            .await?;
        assert_eq!(data.total, 1);
        assert_eq!(data.data.rows[0][2].text(), "Alpine");
        Ok(())
    })
}

#[test]
fn sqlite_filters_preserve_integers_and_do_not_guess_date_encodings() -> Result<()> {
    runtime()?.block_on(async {
        let session = fixture().await?;
        session
            .query(
                "CREATE TABLE scalars(id INTEGER PRIMARY KEY,n INTEGER,d DATE,raw TEXT);
            INSERT INTO scalars VALUES(1,9223372036854775807,'2026-01-01','2026-01-01'),
            (2,9223372036854775806,1767225600,'2026-01-02'),(3,'bad','2026-99-99','2026-01-03');"
                    .into(),
            )
            .await?;
        let table = TableRef {
            name: "scalars".into(),
            ..table()
        };
        for (column, predicate) in [
            (
                "n",
                FilterPredicate::GreaterThan("9223372036854775806".into()),
            ),
            ("d", FilterPredicate::GreaterThan("2025-12-31".into())),
        ] {
            let result = session
                .browse(
                    table.clone(),
                    BrowseOptions {
                        filter: Some(
                            TableFilter {
                                column: column.into(),
                                predicate,
                            }
                            .into(),
                        ),
                        ..page()
                    },
                )
                .await?;
            assert_eq!(result.total, 1);
            assert_eq!(result.data.rows[0][0].text(), "1");
            assert_eq!(
                result.metadata.columns[3].filter_kind(true),
                FilterKind::Text
            );
        }
        Ok(())
    })
}

#[test]
#[ignore = "requires QUERI_TEST_POSTGRES pointing at a disposable PostgreSQL database"]
fn postgres_type_aware_filters() -> Result<()> {
    runtime()?.block_on(async {
        let (profile, password) = postgres_profile()?;
        let session = Session::connect(profile, password).await?;
        let schema = format!("queri_filters_{}", Uuid::new_v4().simple());
        session.query(format!("CREATE SCHEMA {schema}")).await?;
        let result = filter_workflows(&session, &schema).await;
        session
            .query(format!("DROP SCHEMA {schema} CASCADE"))
            .await?;
        result
    })
}

#[test]
fn sqlite_insert_defaults_update_composite_key_conflict_and_atomic_delete() -> Result<()> {
    runtime()?.block_on(async {
        let session = fixture().await?;
        session
            .change_rows(
                table(),
                RowChange::Insert(vec![
                    FieldValue::Value(Cell::Integer(3)),
                    FieldValue::Value(Cell::Integer(1)),
                    FieldValue::Default,
                    FieldValue::Value(Cell::Integer(i64::MAX)),
                    FieldValue::Null,
                ]),
            )
            .await?;
        let data = session.browse(table(), page()).await?;
        assert_eq!(data.data.rows[3][2].text(), "New");
        assert_eq!(data.data.rows[3][3], Cell::Integer(i64::MAX));
        let original = data.data.rows[0].clone();
        let mut values: Vec<_> = original.iter().cloned().map(FieldValue::Value).collect();
        values[2] = FieldValue::Value(Cell::Text("Changed".into()));
        assert_eq!(
            session
                .change_rows(
                    table(),
                    RowChange::Update {
                        original: original.clone(),
                        values: values.clone()
                    }
                )
                .await?,
            1
        );
        assert!(
            session
                .change_rows(
                    table(),
                    RowChange::Update {
                        original: original.clone(),
                        values
                    }
                )
                .await
                .is_err()
        );
        let result = session
            .change_rows(
                table(),
                RowChange::Delete {
                    originals: vec![data.data.rows[1].clone(), original],
                },
            )
            .await;
        assert!(result.is_err());
        assert_eq!(session.browse(table(), page()).await?.total, 4);
        let rows = session.browse(table(), page()).await?.data.rows;
        session
            .change_rows(
                table(),
                RowChange::Delete {
                    originals: vec![rows[0].clone()],
                },
            )
            .await?;
        let rows = session.browse(table(), page()).await?.data.rows;
        assert!(
            rows.iter()
                .any(|row| row[0] == Cell::Integer(2) && row[1] == Cell::Integer(1))
        );
        Ok(())
    })
}

#[test]
fn sqlite_batches_transactions_quoted_identifiers_and_bounds() -> Result<()> {
    runtime()?.block_on(async {
        let session = fixture().await?;
        session.query("BEGIN; INSERT INTO items(tenant,id,name) VALUES(4,1,'a; b'); ROLLBACK;".into()).await?;
        assert_eq!(session.browse(table(), page()).await?.total, 3);
        session.query("CREATE TABLE \"odd\"\"table\"(id INTEGER PRIMARY KEY, \"x'\" TEXT);".into()).await?;
        let odd = TableRef { name: "odd\"table".into(), ..table() };
        session.change_rows(odd.clone(), RowChange::Insert(vec![FieldValue::Value(Cell::Integer(1)), FieldValue::Value(Cell::Text("'); DROP TABLE items;--".into()))])).await?;
        assert_eq!(session.browse(odd, page()).await?.total, 1);
        assert_eq!(session.browse(table(), page()).await?.total, 3);
        let output = session.query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<11000) SELECT x FROM n; SELECT 'second';".into()).await?;
        assert_eq!(output.sets.len(), 2);
        assert_eq!(output.sets[0].rows.len(), MAX_RESULT_ROWS);
        assert!(output.sets[0].truncated);
        assert!(output.sets[1].truncated);
        Ok(())
    })
}

#[test]
fn sqlite_cancellation_does_not_poison_session() -> Result<()> {
    runtime()?.block_on(async {
        let session = fixture().await?;
        let running = session.clone();
        let task = tokio::spawn(async move { running.query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n".into()).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        session.cancel().await?;
        assert!(tokio::time::timeout(Duration::from_secs(3), task).await??.is_err());
        assert_eq!(session.query("SELECT 42".into()).await?.sets[0].rows[0][0], Cell::Integer(42));
        Ok(())
    })
}

#[test]
fn sqlite_read_only_and_missing_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    runtime()?.block_on(async {
        let mut profile = sqlite_profile(dir.path().join("fixture.db"));
        let session = Session::connect(profile.clone(), String::new()).await?;
        session
            .query("CREATE TABLE t(id INTEGER PRIMARY KEY)".into())
            .await?;
        drop(session);
        if let ConnectionConfig::Sqlite {
            read_only, create, ..
        } = &mut profile.config
        {
            *read_only = true;
            *create = false;
        }
        let session = Session::connect(profile.clone(), String::new()).await?;
        assert!(
            session
                .query("INSERT INTO t VALUES(1)".into())
                .await
                .is_err()
        );
        assert_eq!(session.tables("main".into()).await?.len(), 1);
        if let ConnectionConfig::Sqlite { path, .. } = &mut profile.config {
            *path = dir.path().join("missing.db");
        }
        assert!(Session::connect(profile, String::new()).await.is_err());
        Ok(())
    })
}

#[test]
fn saved_relation_kinds_keep_the_existing_json_format() -> Result<()> {
    for (name, kind) in [
        ("table", RelationKind::Table),
        ("view", RelationKind::View),
        ("materialized view", RelationKind::MaterializedView),
    ] {
        let saved = serde_json::json!({"schema": "public", "name": "items", "kind": name});
        let relation: TableRef = serde_json::from_value(saved.clone())?;
        assert_eq!(relation.kind, kind);
        assert_eq!(relation.is_table(), kind == RelationKind::Table);
        assert_eq!(serde_json::to_value(relation)?, saved);
    }
    Ok(())
}

#[test]
fn persistence_restart_search_and_connection_deletion() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("app.db");
    let store = Store::open(&path)?;
    let profile = sqlite_profile(dir.path().join("data.db"));
    store.save_profile(&profile)?;
    store.record(&HistoryEntry {
        id: 0,
        connection_id: profile.id,
        database: "data.db".into(),
        sql: "SELECT 100%".into(),
        executed_at: chrono::Utc::now().to_rfc3339(),
        elapsed_ms: 7,
        affected: 1,
        error: None,
    })?;
    store.save_preferences(&Preferences {
        warn_destructive: false,
        ..Default::default()
    })?;
    drop(store);
    let store = Store::open(path)?;
    assert_eq!(store.profiles()?.len(), 1);
    assert!(!store.preferences()?.warn_destructive);
    assert_eq!(store.history(profile.id, "%", 0)?.len(), 1);
    store.delete_profile(profile.id)?;
    assert!(store.history(profile.id, "", 0)?.is_empty());
    Ok(())
}

#[test]
fn sql_warnings_ignore_literals_and_check_each_statement() {
    assert!(!queri::sql::destructive(
        "SELECT 'DROP TABLE t;'; -- DELETE FROM t\nSELECT 1"
    ));
    assert!(queri::sql::destructive(
        "DELETE FROM a WHERE id=1; DELETE FROM b;"
    ));
    assert!(queri::sql::destructive("ALTER TABLE a DROP COLUMN x"));
    assert!(queri::sql::destructive("TRUNCATE a"));
}

fn postgres_profile() -> Result<(ConnectionProfile, String)> {
    let url = std::env::var("QUERI_TEST_POSTGRES").map_err(|_| {
        anyhow::anyhow!("Set QUERI_TEST_POSTGRES to a disposable PostgreSQL database.")
    })?;
    let config: tokio_postgres::Config = url.parse()?;
    let host = match config.get_hosts().first() {
        Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
        _ => "localhost".into(),
    };
    Ok((
        ConnectionProfile {
            ssh: None,
            environment: None,
            policy: ExecutionPolicy::Normal,
            remember_password: false,
            id: Uuid::new_v4(),
            name: "Integration".into(),
            notes: String::new(),
            color: String::new(),
            config: ConnectionConfig::Postgres {
                host,
                port: config.get_ports().first().copied().unwrap_or(5432),
                database: config.get_dbname().unwrap_or("postgres").into(),
                user: config.get_user().unwrap_or("postgres").into(),
                tls: TlsMode::Disable,
                timeout_seconds: 5,
            },
        },
        String::from_utf8(config.get_password().unwrap_or_default().to_vec())?,
    ))
}

#[test]
#[ignore = "requires QUERI_TEST_POSTGRES pointing at a disposable PostgreSQL database"]
fn postgres_real_database_workflows() -> Result<()> {
    runtime()?.block_on(async {
        let (profile,password) = postgres_profile()?;
        let session = Session::connect(profile,password).await?;
        let schema = format!("queri_{}", Uuid::new_v4().simple());
        session.query(format!("CREATE SCHEMA {schema}; CREATE TABLE {schema}.items(tenant integer,id bigint,name text DEFAULT 'New',amount numeric(38,10),data jsonb,payload bytea,PRIMARY KEY(tenant,id)); INSERT INTO {schema}.items VALUES(1,1,'Alpha',12345678901234567890.1234567890,'{{\"a\":1}}','\\x00ff'),(2,1,'Beta',NULL,NULL,NULL);")).await?;
        let table = TableRef { schema: schema.clone(), name: "items".into(), kind: RelationKind::Table };
        session.query(format!("CREATE VIEW {schema}.item_view AS SELECT * FROM {schema}.items; CREATE MATERIALIZED VIEW {schema}.materialized_items AS SELECT * FROM {schema}.items")).await?;
        let relations = session.tables(schema.clone()).await?;
        assert_eq!(relations.iter().map(|table| table.kind).collect::<Vec<_>>(), vec![RelationKind::View, RelationKind::Table, RelationKind::MaterializedView]);
        for relation in relations.iter().filter(|relation| !relation.is_table()) {
            let page = session.browse(relation.clone(), page()).await?;
            assert!(!page.metadata.editable(relation));
        }
        let data = session.browse(table.clone(), page()).await?;
        assert_eq!(data.total, 2);
        assert_eq!(data.data.rows[0][3].text(), "12345678901234567890.1234567890");
        assert_eq!(data.data.rows[0][5].text(), "\\x00ff");
        let exact = session.browse(table.clone(), BrowseOptions {
            filter: Some(TableFilter { column: "amount".into(), predicate: FilterPredicate::Equals("12345678901234567890.12345678901".into()) }.into()),
            ..page()
        }).await?;
        assert_eq!(exact.total, 0, "filter input must not be rounded to the column's scale");
        let original = data.data.rows[0].clone();
        let mut values: Vec<_> = original.iter().cloned().map(FieldValue::Value).collect();
        values[2] = FieldValue::Value(Cell::Text("Changed".into()));
        session.change_rows(table.clone(), RowChange::Update { original: original.clone(), values: values.clone() }).await?;
        assert!(session.change_rows(table.clone(), RowChange::Update { original, values }).await.is_err());
        let filtered = session.browse(table.clone(), BrowseOptions { filter: Some(TableFilter { column: "name".into(), predicate: FilterPredicate::Contains("hang".into()) }.into()), ..page() }).await?;
        assert_eq!(filtered.total, 1);
        session.change_rows(table.clone(), RowChange::Insert(vec![FieldValue::Value(Cell::Text("3".into())),FieldValue::Value(Cell::Text("9223372036854775807".into())),FieldValue::Default,FieldValue::Null,FieldValue::Value(Cell::Text("{\"nested\":[1,2]}".into())),FieldValue::Null])).await?;
        let data = session.browse(table.clone(), page()).await?;
        assert_eq!(data.data.rows[2][1].text(), "9223372036854775807");
        assert_eq!(data.data.rows[2][2].text(), "New");
        let query_session = session.clone();
        let running = tokio::spawn(async move { query_session.query("SELECT pg_sleep(20)".into()).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        session.cancel().await?;
        assert!(tokio::time::timeout(Duration::from_secs(3), running).await??.is_err());
        assert_eq!(session.query("SELECT 42".into()).await?.sets[0].rows[0][0].text(), "42");
        let results = session.query("SELECT 1; SELECT 'a;b'".into()).await?;
        assert_eq!(results.sets.len(), 2);
        session.change_rows(table.clone(), RowChange::Delete { originals: vec![data.data.rows[0].clone()] }).await?;
        assert_eq!(session.browse(table, page()).await?.total, 2);
        session.query(format!("DROP SCHEMA {schema} CASCADE")).await?;
        Ok(())
    })
}

#[test]
#[ignore = "requires PostgreSQL CREATEDB permission"]
fn postgres_database_lifecycle_and_auth_failure() -> Result<()> {
    runtime()?.block_on(async {
        let (profile, password) = postgres_profile()?;
        assert!(
            Session::connect(profile.clone(), "wrong-password".into())
                .await
                .is_err()
        );
        let session = Session::connect(profile, password).await?;
        let source = format!("queri_{}", Uuid::new_v4().simple());
        let clone = format!("{source}_clone");
        let renamed = format!("{source}_renamed");
        session
            .database_action(DatabaseAction::Create {
                name: source.clone(),
            })
            .await?;
        assert!(session.databases().await?.contains(&source));
        session
            .database_action(DatabaseAction::Clone {
                source: source.clone(),
                target: clone.clone(),
                force: false,
            })
            .await?;
        session
            .database_action(DatabaseAction::Rename {
                source: clone,
                target: renamed.clone(),
                force: false,
            })
            .await?;
        session
            .database_action(DatabaseAction::Delete {
                name: source,
                force: false,
            })
            .await?;
        session
            .database_action(DatabaseAction::Delete {
                name: renamed,
                force: false,
            })
            .await?;
        Ok(())
    })
}

#[test]
fn sqlite_relationship_navigation_preserves_composite_keys_and_affinity() -> Result<()> {
    runtime()?.block_on(async {
        let session=Session::connect(sqlite_profile(":memory:".into()),String::new()).await?;
        session.query("CREATE TABLE parent(code TEXT COLLATE NOCASE, tenant INTEGER, PRIMARY KEY(tenant,code));
            CREATE TABLE child(id INTEGER PRIMARY KEY, t TEXT, c TEXT, FOREIGN KEY(t,c) REFERENCES parent);
            INSERT INTO parent VALUES('Alpha',1),('Beta',2);
            INSERT INTO child VALUES(1,'01','alpha'),(2,'2','Beta'),(3,NULL,'Alpha');".into()).await?;
        let child=TableRef{schema:"main".into(),name:"child".into(),kind:RelationKind::Table};
        let keys=session.foreign_keys(child.clone(),false).await?;
        assert_eq!(keys.len(),1);
        assert_eq!(keys[0].columns[0].target,"tenant");
        let outgoing=RelationshipScope { key:keys[0].clone(),incoming:false,values:vec![Cell::Text("01".into()),Cell::Text("alpha".into())] };
        let result=compare_generated_page(&session,outgoing.destination().clone(),BrowseOptions{relationship:Some(outgoing),..page()}).await?;
        assert_eq!(result.total,1);assert_eq!(result.data.rows[0][0],Cell::Text("Alpha".into()));
        let incoming=RelationshipScope {key:keys[0].clone(),incoming:true,values:vec![Cell::Integer(1),Cell::Text("Alpha".into())]};
        let result=compare_generated_page(&session,child.clone(),BrowseOptions{relationship:Some(incoming.clone()),..page()}).await?;
        assert_eq!(result.total,1);assert_eq!(result.data.rows[0][0],Cell::Integer(1));
        let filtered=compare_generated_page(&session,child.clone(),BrowseOptions{relationship:Some(incoming),filter:Some(TableFilter{column:"id".into(),predicate:FilterPredicate::Equals("2".into())}.into()),..page()}).await?;
        assert_eq!(filtered.total,0);
        Ok(())
    })
}

#[test]
#[ignore = "requires disposable PostgreSQL"]
fn postgres_relationship_navigation_uses_native_key_types() -> Result<()> {
    runtime()?.block_on(async {
        let (profile,password)=postgres_profile()?;
        let session=Session::connect(profile,password).await?;
        let schema=format!("relations_{}",Uuid::new_v4().simple());
        session.query(format!("CREATE SCHEMA {schema};
            CREATE TABLE {schema}.parent(id uuid, tenant integer, PRIMARY KEY(tenant,id));
            CREATE TABLE {schema}.child(id integer PRIMARY KEY,t bigint,p uuid,FOREIGN KEY(t,p) REFERENCES {schema}.parent(tenant,id));
            INSERT INTO {schema}.parent VALUES('12345678-1234-1234-1234-123456789abc',1);
            INSERT INTO {schema}.child VALUES(1,1,'12345678-1234-1234-1234-123456789abc'),(2,NULL,NULL);")).await?;
        let child=TableRef{schema:schema.clone(),name:"child".into(),kind:RelationKind::Table};
        let keys=session.foreign_keys(child.clone(),false).await?;
        assert_eq!(keys.len(),1);
        let scope=RelationshipScope{key:keys[0].clone(),incoming:false,values:vec![Cell::Text("1".into()),Cell::Text("12345678-1234-1234-1234-123456789abc".into())]};
        let result=compare_generated_page(&session,scope.destination().clone(),BrowseOptions{relationship:Some(scope.clone()),..page()}).await?;
        assert_eq!(result.total,1);
        let incoming=RelationshipScope{incoming:true,..scope};
        assert_eq!(compare_generated_page(&session,child,BrowseOptions{relationship:Some(incoming),..page()}).await?.total,1);
        session.query(format!("DROP SCHEMA {schema} CASCADE")).await?;
        Ok(())
    })
}

#[test]
fn sqlite_formatted_sql_executes_equivalently() -> Result<()> {
    runtime()?.block_on(async {
        let session = fixture().await?;
        for sql in [
            "SELECT name, row_number() OVER (ORDER BY id) FROM items WHERE quantity > 10;",
            "WITH x(n) AS (VALUES(1),(2)) SELECT sum(n), 'é', x'00ff' FROM x;",
        ] {
            let formatted = queri::sql::format_sql(sql, queri::sql::SqlDialect::Sqlite)?;
            assert_eq!(
                session.query(sql.into()).await?.sets[0].rows,
                session.query(formatted).await?.sets[0].rows
            );
        }
        Ok(())
    })
}
#[test]
#[ignore = "requires disposable PostgreSQL"]
fn postgres_formatted_sql_executes_equivalently() -> Result<()> {
    runtime()?.block_on(async {
        let (profile,password)=postgres_profile()?;
        let session=Session::connect(profile,password).await?;
        let sql="WITH x AS (SELECT 1 AS n) SELECT n::numeric, $body$ SELECT 'hello'; $body$, '{\"name\":\"Alice\"}'::jsonb->>'name' FROM x;";
        let formatted=queri::sql::format_sql(sql,queri::sql::SqlDialect::Postgres)?;
        assert_eq!(session.query(sql.into()).await?.sets[0].rows,session.query(formatted).await?.sets[0].rows);
        Ok(())
    })
}

#[test]
fn sqlite_transaction_status_uses_autocommit() -> Result<()> {
    runtime()?.block_on(async {
        let session = fixture().await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        session.query("BEGIN".into()).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
        assert!(session.query("SELECT * FROM missing".into()).await.is_err());
        // SQLite statement errors do not imply PostgreSQL's failed transaction state.
        assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
        session.query("ROLLBACK".into()).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        session.query("SAVEPOINT s; RELEASE s".into()).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        assert!(
            session
                .query_postgres_params("SELECT $1".into(), vec![None])
                .await
                .is_err()
        );
        Ok(())
    })
}

#[test]
#[ignore = "requires disposable PostgreSQL"]
fn postgres_authoritative_transaction_status() -> Result<()> {
    runtime()?.block_on(async {
        let (profile, password) = postgres_profile()?;
        let session = Session::connect(profile.clone(), password.clone()).await?;
        let other = Session::connect(profile.clone(), password.clone()).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        session
            .query("SELECT 'BEGIN;'; /* BEGIN */ SELECT 1".into())
            .await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        session
            .query("START TRANSACTION; SAVEPOINT s".into())
            .await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
        assert_eq!(other.transaction_status().await?, TransactionStatus::Idle);
        assert!(session.query("SELECT 1/0".into()).await.is_err());
        assert_eq!(
            session.transaction_status().await?,
            TransactionStatus::Failed
        );
        session.query("ROLLBACK TO s".into()).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
        session.query("COMMIT".into()).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        session.query("BEGIN; ROLLBACK".into()).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        assert!(
            session
                .query("BEGIN; SELECT 1/0; COMMIT".into())
                .await
                .is_err()
        );
        assert_eq!(
            session.transaction_status().await?,
            TransactionStatus::Failed
        );
        // COMMIT in a failed transaction is a server-side rollback.
        session.query("COMMIT".into()).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        assert!(session.query("SELECT 1/0".into()).await.is_err());
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);

        session.query("BEGIN".into()).await?;
        let pid = session.query("SELECT pg_backend_pid()".into()).await?.sets[0].rows[0][0].text();
        other
            .query_postgres_params(
                "SELECT pg_terminate_backend($1::integer)".into(),
                vec![Some(pid)],
            )
            .await?;
        assert_eq!(
            session.transaction_status().await?,
            TransactionStatus::Unknown
        );
        assert!(session.query("SELECT 1".into()).await.is_err());
        let reconnected = Session::connect(profile, password.clone()).await?;
        assert_eq!(
            reconnected.transaction_status().await?,
            TransactionStatus::Idle
        );
        Ok(())
    })
}

async fn wait_for_postgres_sleep(observer: &Session, pid: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let output = observer
                .query_postgres_params(
                    "SELECT wait_event FROM pg_stat_activity WHERE pid=$1::integer".into(),
                    vec![Some(pid.into())],
                )
                .await?;
            if output.sets[0]
                .rows
                .first()
                .is_some_and(|row| row[0].text() == "PgSleep")
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

#[test]
#[ignore = "requires disposable PostgreSQL"]
fn postgres_cancellation_and_abandoned_work_state() -> Result<()> {
    runtime()?.block_on(async {
        let (profile, password) = postgres_profile()?;
        let session = Session::connect(profile.clone(), password.clone()).await?;
        let observer = Session::connect(profile, password).await?;
        let pid = session.query("SELECT pg_backend_pid()".into()).await?.sets[0].rows[0][0].text();
        for in_transaction in [false, true] {
            if in_transaction {
                session.query("BEGIN".into()).await?;
            }
            let running = session.clone();
            let task = tokio::spawn(async move {
                if in_transaction {
                    running
                        .query_postgres_params(
                            "SELECT pg_sleep($1::float8)".into(),
                            vec![Some("20".into())],
                        )
                        .await
                } else {
                    running.query("SELECT pg_sleep(20)".into()).await
                }
            });
            wait_for_postgres_sleep(&observer, &pid).await?;
            session.cancel().await?;
            assert!(
                tokio::time::timeout(Duration::from_secs(3), task)
                    .await??
                    .is_err()
            );
            assert_eq!(
                session.transaction_status().await?,
                if in_transaction {
                    TransactionStatus::Failed
                } else {
                    TransactionStatus::Idle
                }
            );
            session.query("ROLLBACK".into()).await?;
            assert_eq!(
                session.query("SELECT 42".into()).await?.sets[0].rows[0][0].text(),
                "42"
            );
        }
        let running = session.clone();
        let serialized =
            tokio::spawn(async move { running.query("SELECT pg_sleep(0.1); BEGIN".into()).await });
        wait_for_postgres_sleep(&observer, &pid).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
        serialized.await??;
        session.query("ROLLBACK".into()).await?;
        let running = session.clone();
        let task = tokio::spawn(async move {
            running
                .query("BEGIN; SELECT pg_sleep(0.3); SELECT 1/0".into())
                .await
        });
        wait_for_postgres_sleep(&observer, &pid).await?;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        // The response consumer disappeared, but the driver must still drain
        // the script before the status Sync can complete.
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), session.transaction_status()).await??,
            TransactionStatus::Failed
        );
        session.query("ROLLBACK".into()).await?;
        Ok(())
    })
}

#[test]
#[ignore = "requires disposable PostgreSQL"]
fn postgres_text_bind_fidelity_errors_and_limits() -> Result<()> {
    runtime()?.block_on(async {
        let (profile, password) = postgres_profile()?;
        let session = Session::connect(profile, password).await?;
        session.query("CREATE TEMP TABLE bind_effects(n integer); CREATE TYPE pg_temp.mood AS ENUM ('calm', 'é'); SET TIME ZONE 'UTC'".into()).await?;
        let sql = "SELECT $1::text, $2::text, $3::numeric, $4::int[], $5::timestamp, $6::timestamptz, $7::bytea, $8::pg_temp.mood, $9::text, $3::numeric";
        let values = vec![
            None, Some(""), Some("12345678901234567890.12345678901234567890"),
            Some("{1,NULL,-3}"), Some("2024-02-29 12:34:56.123456"),
            Some("2024-02-29 12:34:56.123456+05:30"), Some("\\x00ff80"), Some("é"),
            Some("'); DROP TABLE bind_effects; --\n雪"),
        ].into_iter().map(|v| v.map(str::to_owned)).collect();
        let empty = session.query_postgres_params("SELECT $1::text AS value WHERE false".into(), vec![None]).await?;
        assert_eq!(empty.sets[0].columns, vec!["value"]);
        assert!(empty.sets[0].rows.is_empty());
        assert_eq!(session.query_postgres_params("SELECT $1".into(), vec![Some("inferred text".into())]).await?.sets[0].rows[0][0].text(), "inferred text");
        session.query_postgres_params("BEGIN".into(), vec![]).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
        // A local encoding error sent no SQL and must not mark the transaction failed.
        assert!(session.query_postgres_params("SELECT \0".into(), vec![]).await.is_err());
        assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
        session.query_postgres_params("COMMIT".into(), vec![]).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        let output = session.query_postgres_params(sql.into(), values).await?;
        let reference = session.query("SELECT NULL::text, ''::text, 12345678901234567890.12345678901234567890::numeric, '{1,NULL,-3}'::int[], '2024-02-29 12:34:56.123456'::timestamp, '2024-02-29 12:34:56.123456+05:30'::timestamptz, '\\x00ff80'::bytea, 'é'::pg_temp.mood, $v$'); DROP TABLE bind_effects; --
雪$v$::text, 12345678901234567890.12345678901234567890::numeric".into()).await?;
        assert_eq!(output.sets[0].rows, reference.sets[0].rows);
        assert_eq!(output.sets[0].rows[0][0], Cell::Null);
        assert_eq!(output.sets[0].rows[0][1], Cell::Text(String::new()));
        assert_eq!(output.sets[0].rows[0][2].text(), "12345678901234567890.12345678901234567890");
        let inserted = session.query_postgres_params("INSERT INTO bind_effects VALUES ($1::int)".into(), vec![Some("7".into())]).await?;
        assert_eq!(inserted.sets[0].affected, 1);
        assert!(inserted.sets[0].columns.is_empty());
        for (sql, params) in [
            ("SELECT $1::int", vec![Some("bad".into())]),
            ("SELECT $1::int", vec![]),
            ("SELECT 1", vec![None]),
            ("SELECT $1::text", vec![Some("a\0b".into())]),
            ("INSERT INTO bind_effects VALUES (99); SELECT 1", vec![]),
            ("SELEC $1", vec![None]),
        ] {
            session.query("BEGIN".into()).await?;
            assert!(session.query_postgres_params(sql.into(), params).await.is_err());
            assert_eq!(session.transaction_status().await?, TransactionStatus::Failed);
            session.query("ROLLBACK".into()).await?;
        }
        assert_eq!(session.query("SELECT count(*) FROM bind_effects".into()).await?.sets[0].rows[0][0].text(), "1");
        let limited = session.query_postgres_params(
            "INSERT INTO bind_effects SELECT generate_series(1,$1::int) RETURNING n".into(), vec![Some("11000".into())]
        ).await?;
        assert_eq!(limited.sets[0].rows.len(), MAX_RESULT_ROWS);
        assert_eq!(limited.sets[0].affected, 11000);
        assert!(limited.sets[0].truncated);
        assert_eq!(session.query("SELECT count(*) FROM bind_effects".into()).await?.sets[0].rows[0][0].text(), "11001");
        let bytes = session.query_postgres_params(
            "SELECT repeat('x', $1::int) FROM generate_series(1,34)".into(), vec![Some("1048576".into())]
        ).await?;
        assert!(bytes.sets[0].truncated);
        assert_eq!(bytes.sets[0].rows.len(), MAX_RESULT_BYTES / 1048576);
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        Ok(())
    })
}

#[test]
fn sqlite_transaction_commit_rollback_isolation_and_automatic_rollback() -> Result<()> {
    let dir = tempfile::tempdir()?;
    runtime()?.block_on(async {
        let profile = sqlite_profile(dir.path().join("transactions.db"));
        let first = Session::connect(profile.clone(), String::new()).await?;
        let second = Session::connect(profile, String::new()).await?;
        first
            .query(
                "CREATE TABLE tx(id INTEGER PRIMARY KEY); BEGIN; INSERT INTO tx VALUES(1)".into(),
            )
            .await?;
        assert_eq!(first.transaction_status().await?, TransactionStatus::Open);
        assert_eq!(second.transaction_status().await?, TransactionStatus::Idle);
        assert_eq!(
            second.query("SELECT count(*) FROM tx".into()).await?.sets[0].rows[0][0],
            Cell::Integer(0)
        );
        first.query("COMMIT".into()).await?;
        assert_eq!(
            second.query("SELECT count(*) FROM tx".into()).await?.sets[0].rows[0][0],
            Cell::Integer(1)
        );
        first
            .query("BEGIN; INSERT INTO tx VALUES(2); ROLLBACK".into())
            .await?;
        assert_eq!(
            second.query("SELECT count(*) FROM tx".into()).await?.sets[0].rows[0][0],
            Cell::Integer(1)
        );
        first
            .query("BEGIN; INSERT INTO tx VALUES(3)".into())
            .await?;
        assert!(
            first
                .query("INSERT OR ROLLBACK INTO tx VALUES(1)".into())
                .await
                .is_err()
        );
        assert_eq!(first.transaction_status().await?, TransactionStatus::Idle);
        assert_eq!(
            second.query("SELECT count(*) FROM tx".into()).await?.sets[0].rows[0][0],
            Cell::Integer(1)
        );
        Ok(())
    })
}

#[test]
#[ignore = "requires disposable PostgreSQL"]
fn postgres_transaction_commit_rollback_isolation() -> Result<()> {
    runtime()?.block_on(async {
        let (profile, password) = postgres_profile()?;
        let first = Session::connect(profile.clone(), password.clone()).await?;
        let second = Session::connect(profile, password).await?;
        let name = format!("transaction_{}", Uuid::new_v4().simple());
        first
            .query(format!("CREATE TABLE {name}(id INTEGER PRIMARY KEY)"))
            .await?;
        first
            .query(format!("BEGIN; INSERT INTO {name} VALUES(1)"))
            .await?;
        assert_eq!(
            second
                .query(format!("SELECT count(*) FROM {name}"))
                .await?
                .sets[0]
                .rows[0][0]
                .text(),
            "0"
        );
        first.query("COMMIT".into()).await?;
        assert_eq!(
            second
                .query(format!("SELECT count(*) FROM {name}"))
                .await?
                .sets[0]
                .rows[0][0]
                .text(),
            "1"
        );
        first
            .query(format!(
                "BEGIN; INSERT INTO {name} VALUES(2); SAVEPOINT recovery"
            ))
            .await?;
        assert!(
            first
                .query(format!("INSERT INTO {name} VALUES(1)"))
                .await
                .is_err()
        );
        assert_eq!(first.transaction_status().await?, TransactionStatus::Failed);
        first.query("ROLLBACK TO recovery; ROLLBACK".into()).await?;
        assert_eq!(
            second
                .query(format!("SELECT count(*) FROM {name}"))
                .await?
                .sets[0]
                .rows[0][0]
                .text(),
            "1"
        );
        first.query(format!("DROP TABLE {name}")).await?;
        Ok(())
    })
}

#[test]
#[ignore = "requires disposable PostgreSQL"]
fn postgres_transaction_disconnect_during_commit_is_unknown() -> Result<()> {
    runtime()?.block_on(async {
        let (profile, password) = postgres_profile()?;
        let writer = Session::connect(profile.clone(), password.clone()).await?;
        let observer = Session::connect(profile, password).await?;
        let name = format!("commit_loss_{}", Uuid::new_v4().simple());
        observer.query(format!("CREATE TABLE {name}(id integer); CREATE FUNCTION {name}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(30); RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER delay_commit AFTER INSERT ON {name} DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION {name}()")).await?;
        let pid = writer.query("SELECT pg_backend_pid()".into()).await?.sets[0].rows[0][0].text();
        writer.query(format!("BEGIN; INSERT INTO {name} VALUES(1)")).await?;
        let committing = writer.clone();
        let commit = tokio::spawn(async move { committing.query("COMMIT".into()).await });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let state = observer.query(format!("SELECT count(*) FROM pg_stat_activity WHERE pid={pid} AND wait_event='PgSleep' AND query='COMMIT'")).await?;
                if state.sets[0].rows[0][0].text() == "1" { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, anyhow::Error>(())
        }).await??;
        observer.query(format!("SELECT pg_terminate_backend({pid})")).await?;
        assert!(tokio::time::timeout(Duration::from_secs(5), commit).await??.is_err());
        assert_eq!(writer.transaction_status().await?, TransactionStatus::Unknown);
        assert_eq!(observer.query(format!("SELECT count(*) FROM {name}")).await?.sets[0].rows[0][0].text(), "0");
        observer.query(format!("DROP TABLE {name}; DROP FUNCTION {name}()")).await?;
        Ok(())
    })
}

#[test]
fn sqlite_statement_timeouts_recover_and_bound_lock_waits() -> Result<()> {
    runtime()?.block_on(async {
        let dir = tempfile::tempdir()?;
        let profile = sqlite_profile(dir.path().join("timeouts.db"));
        let session = Session::connect(profile.clone(), String::new()).await?;
        let other = Session::connect(profile.clone(), String::new()).await?;
        session.query("CREATE TABLE locked(id INTEGER); INSERT INTO locked VALUES(1)".into()).await?;
        session.set_statement_timeout(StatementTimeout::custom(200)?)?;
        let error = session.query("BEGIN; WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n".into()).await.unwrap_err();
        assert!(queri::database::is_statement_timeout(&error), "{error:#}");
        // A read interruption retains SQLite's real open transaction, not PostgreSQL's Failed state.
        assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
        session.query("ROLLBACK; SELECT 42".into()).await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
        let error = session.query("BEGIN; WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) INSERT INTO locked SELECT x FROM n".into()).await.unwrap_err();
        assert!(queri::database::is_statement_timeout(&error));
        assert_eq!(session.transaction_status().await?, TransactionStatus::Idle, "SQLite rolls back an interrupted write transaction");
        other.query("BEGIN IMMEDIATE; UPDATE locked SET id=2".into()).await?;
        let started = std::time::Instant::now();
        let error = session.query("UPDATE locked SET id=3".into()).await.unwrap_err();
        assert!(queri::database::is_statement_timeout(&error), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(3), "default five-second busy wait must not defeat deadline");
        other.query("ROLLBACK".into()).await?;
        session.query("UPDATE locked SET id=4; SELECT id FROM locked".into()).await?;
        // No detached timer can interrupt the following work, and another tab keeps its policy.
        other.query("SELECT 1".into()).await?;
        session.set_statement_timeout(StatementTimeout::NoTimeout)?;
        session.query("SELECT 1".into()).await?;
        let reopened = Session::connect(profile, String::new()).await?;
        reopened.set_statement_timeout(StatementTimeout::custom(200)?)?;
        let error = reopened.query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n".into()).await.unwrap_err();
        assert!(queri::database::is_statement_timeout(&error));
        Ok(())
    })
}

#[test]
fn sqlite_statement_timeouts_reset_for_each_batch_statement() -> Result<()> {
    runtime()?.block_on(async {
        let session = Session::connect(sqlite_profile(":memory:".into()), String::new()).await?;
        session.set_statement_timeout(StatementTimeout::custom(200)?)?;
        let started = std::time::Instant::now();
        let output = session.query("SELECT 1;".repeat(200_000)).await?;
        assert!(started.elapsed() > Duration::from_millis(200));
        assert_eq!(output.sets.len(), 200_000);
        assert!(output.sets.last().unwrap().truncated);
        session.query("SELECT 42".into()).await?;
        Ok(())
    })
}

#[test]
#[ignore = "requires QUERI_TEST_POSTGRES pointing at a disposable PostgreSQL database"]
fn postgres_statement_timeouts_bind_batch_recovery_and_independent_sessions() -> Result<()> {
    runtime()?.block_on(async {
        let (profile, password) = postgres_profile()?;
        let session = Session::connect(profile.clone(), password.clone()).await?;
        let other = Session::connect(profile.clone(), password.clone()).await?;
        let initial =
            session.query("SHOW statement_timeout".into()).await?.sets[0].rows[0][0].clone();
        session.set_statement_timeout(StatementTimeout::custom(200)?)?;
        let error = session
            .query_postgres_params(
                "SELECT pg_sleep($1::double precision)".into(),
                vec![Some("10".into())],
            )
            .await
            .unwrap_err();
        assert!(queri::database::is_statement_timeout(&error), "{error:#}");
        let error = session
            .query("SELECT sum(x::numeric) FROM generate_series(1, 1000000000) AS x".into())
            .await
            .unwrap_err();
        assert!(queri::database::is_statement_timeout(&error));
        other.query("SELECT pg_sleep(0.4)".into()).await?;
        session.query("SELECT 42".into()).await?;
        session.query("BEGIN; SAVEPOINT recover".into()).await?;
        let error = session
            .query("SELECT pg_sleep(10)".into())
            .await
            .unwrap_err();
        assert!(queri::database::is_statement_timeout(&error));
        assert_eq!(
            session.transaction_status().await?,
            TransactionStatus::Failed
        );
        session
            .query("ROLLBACK TO SAVEPOINT recover; SELECT 1".into())
            .await?;
        assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
        session.set_statement_timeout(StatementTimeout::custom(300)?)?;
        session.query("SELECT 1".into()).await?;
        session.query("ROLLBACK".into()).await?;
        assert_eq!(
            session.query("SHOW statement_timeout".into()).await?.sets[0].rows[0][0].text(),
            "300ms"
        );
        session.query("SET statement_timeout=0".into()).await?;
        assert_eq!(
            session.query("SHOW statement_timeout".into()).await?.sets[0].rows[0][0].text(),
            "300ms"
        );
        session.set_statement_timeout(StatementTimeout::custom(1000)?)?;
        session
            .query("SELECT pg_sleep(0.6); SELECT pg_sleep(0.6); SELECT 42".into())
            .await?;
        assert_eq!(
            other.query("SHOW statement_timeout".into()).await?.sets[0].rows[0][0],
            initial
        );
        session.set_statement_timeout(StatementTimeout::NoTimeout)?;
        assert_eq!(
            session.query("SHOW statement_timeout".into()).await?.sets[0].rows[0][0].text(),
            "0"
        );
        session.query("SELECT pg_sleep(0.3)".into()).await?;
        session.set_statement_timeout(StatementTimeout::Inherit)?;
        assert_eq!(
            session.query("SHOW statement_timeout".into()).await?.sets[0].rows[0][0],
            initial
        );
        let reopened = Session::connect(profile, password).await?;
        reopened.set_statement_timeout(StatementTimeout::custom(200)?)?;
        let error = reopened
            .query("SELECT pg_sleep(10)".into())
            .await
            .unwrap_err();
        assert!(queri::database::is_statement_timeout(&error));
        // Session advisory locks are deterministic and avoid fixture table collisions.
        other
            .query("SELECT pg_advisory_lock(782394223)".into())
            .await?;
        let error = reopened
            .query("BEGIN; SELECT pg_advisory_xact_lock(782394223)".into())
            .await
            .unwrap_err();
        assert!(queri::database::is_statement_timeout(&error));
        assert_eq!(
            reopened.transaction_status().await?,
            TransactionStatus::Failed
        );
        reopened.query("ROLLBACK".into()).await?;
        other
            .query("SELECT pg_advisory_unlock(782394223)".into())
            .await?;
        reopened
            .query("SELECT pg_advisory_xact_lock(782394223); SELECT 1".into())
            .await?;
        assert_eq!(
            reopened.transaction_status().await?,
            TransactionStatus::Idle
        );
        Ok(())
    })
}

#[test]
#[ignore = "requires QUERI_TEST_POSTGRES and role creation on the disposable cluster"]
fn postgres_statement_timeout_inherit_restores_initial_role_policy() -> Result<()> {
    runtime()?.block_on(async {
        let (mut profile, password) = postgres_profile()?;
        let admin = Session::connect(profile.clone(), password).await?;
        let role = format!("queri_timeout_{}", Uuid::new_v4().simple());
        admin.query(format!("CREATE ROLE {role} LOGIN PASSWORD 'timeout-fixture'; ALTER ROLE {role} SET statement_timeout='800ms'" )).await?;
        if let ConnectionConfig::Postgres { user, .. } = &mut profile.config { *user = role.clone(); }
        let result: Result<()> = async {
            let session = Session::connect(profile, "timeout-fixture".into()).await?;
            anyhow::ensure!(session.query("SHOW statement_timeout".into()).await?.sets[0].rows[0][0].text() == "800ms");
            session.set_statement_timeout(StatementTimeout::NoTimeout)?;
            session.query("SELECT pg_sleep(1.2)".into()).await?;
            session.set_statement_timeout(StatementTimeout::custom(200)?)?;
            anyhow::ensure!(session.query("SHOW statement_timeout".into()).await?.sets[0].rows[0][0].text() == "200ms");
            session.set_statement_timeout(StatementTimeout::Inherit)?;
            anyhow::ensure!(session.query("SHOW statement_timeout".into()).await?.sets[0].rows[0][0].text() == "800ms");
            let error = session.query("SELECT pg_sleep(10)".into()).await.unwrap_err();
            anyhow::ensure!(queri::database::is_statement_timeout(&error));
            session.query("SELECT 1".into()).await?;
            Ok(())
        }.await;
        admin.query(format!("DROP ROLE {role}")).await?;
        result
    })
}

#[test]
#[ignore = "requires QUERI_TEST_POSTGRES pointing at a disposable PostgreSQL database"]
fn postgres_safeguards_defaults_overrides_boundaries_and_reconnect() -> Result<()> {
    runtime()?.block_on(async {
        let (mut profile, password) = postgres_profile()?;
        let ordinary = Session::connect(profile.clone(), password.clone()).await?;
        let name = format!("safeguards_{}", Uuid::new_v4().simple());
        ordinary.query(format!("CREATE TABLE {name}(id integer PRIMARY KEY)")).await?;
        profile.environment = Some(Environment::Production);
        // Environment alone never changes execution.
        let labelled = Session::connect(profile.clone(), password.clone()).await?;
        labelled.query(format!("INSERT INTO {name} VALUES(1)")).await?;
        profile.policy = ExecutionPolicy::ReadOnlyByDefault;
        let session = Session::connect(profile.clone(), password.clone()).await?;
        assert!(session.query(format!("INSERT INTO {name} VALUES(2)")).await.is_err());
        let table = TableRef { schema: "public".into(), name: name.clone(), kind: RelationKind::Table };
        assert!(session.change_rows(table.clone(), RowChange::Insert(vec![FieldValue::Value(Cell::Integer(2))])).await.is_err());
        assert!(session.database_action(DatabaseAction::Create { name: format!("{name}_db") }).await.is_err());
        // Raw SQL remains deliberately overridable, including explicit read-write transactions.
        session.query("BEGIN READ WRITE".into()).await?;
        assert!(session.read_write_observed());
        session.query(format!("INSERT INTO {name} VALUES(2); ROLLBACK")).await?;
        assert!(!session.read_write_observed());
        session.query("SET default_transaction_read_only=off".into()).await?;
        assert!(session.read_write_observed());
        session.query_postgres_params(format!("INSERT INTO {name} VALUES($1::int)"), vec![Some("3".into())]).await?;
        assert!(session.change_rows(table, RowChange::Insert(vec![FieldValue::Value(Cell::Integer(4))])).await.is_err());
        // PostgreSQL allows writes to existing temporary tables in read-only transactions.
        session.query("CREATE TEMP TABLE temporary_safeguard(id int); SET default_transaction_read_only=on".into()).await?;
        session.query("INSERT INTO temporary_safeguard VALUES(1)".into()).await?;
        drop(session);
        let reconnected = Session::connect(profile, password.clone()).await?;
        assert!(!reconnected.read_write_observed());
        assert!(reconnected.query_postgres_params(format!("INSERT INTO {name} VALUES($1::int)"), vec![Some("4".into())]).await.is_err());
        assert!(reconnected.query("CREATE TEMP TABLE another_temporary(id int)".into()).await.is_err());
        ordinary.query(format!("DROP TABLE {name}")).await?;
        // Normal policy must not clear a database/server read-only default.
        let database = format!("{name}_default");
        ordinary.database_action(DatabaseAction::Create { name: database.clone() }).await?;
        ordinary.query(format!("ALTER DATABASE {database} SET default_transaction_read_only=on")).await?;
        let inherited = Session::connect(ordinary.profile.for_database(&database), password).await?;
        assert!(inherited.query("CREATE TABLE persistent(id int)".into()).await.is_err());
        drop(inherited);
        ordinary.database_action(DatabaseAction::Delete { name: database, force: true }).await?;
        Ok(())
    })
}

async fn compare_generated_page(
    session: &Session,
    table: TableRef,
    options: BrowseOptions,
) -> Result<TablePage> {
    let query = session.browse_sql(table.clone(), options.clone()).await?;
    let page = session.browse(table, options).await?;
    let generated = session.query_params(query).await?;
    assert_eq!(generated.sets.len(), 1);
    assert_eq!(generated.sets[0].columns, page.data.columns);
    assert_eq!(generated.sets[0].rows, page.data.rows);
    Ok(page)
}

async fn grouped_views(session: &Session, schema: &str) -> Result<()> {
    let table = TableRef {
        schema: schema.into(),
        name: "view_items".into(),
        kind: RelationKind::Table,
    };
    session.query(format!("CREATE TABLE {schema}.view_items(tenant INTEGER, id INTEGER, label TEXT, amount INTEGER, PRIMARY KEY(tenant,id)); INSERT INTO {schema}.view_items VALUES(2,1,'Alpha',10),(1,2,'ALPHA',10),(1,1,NULL,10),(1,3,'Beta',20),(3,1,'x'' OR 1=1 --',30);")).await?;
    let leaf = |column: &str, predicate| {
        FilterExpression::from(TableFilter {
            column: column.into(),
            predicate,
        })
    };
    let filter = FilterExpression::All(vec![
        FilterExpression::Any(vec![
            leaf("label", FilterPredicate::Equals("alpha".into())),
            leaf("label", FilterPredicate::IsNull),
        ]),
        leaf("amount", FilterPredicate::GreaterOrEqual("10".into())),
    ]);
    let options = BrowseOptions {
        filter: Some(filter),
        sort: vec![("amount".into(), true), ("tenant".into(), false)],
        limit: 1,
        ..Default::default()
    };
    for (offset, expected) in [(0, ("1", "1")), (1, ("1", "2")), (2, ("2", "1"))] {
        let result = compare_generated_page(
            session,
            table.clone(),
            BrowseOptions {
                offset,
                ..options.clone()
            },
        )
        .await?;
        assert_eq!(result.total, 3);
        assert_eq!(
            (result.data.rows[0][0].text(), result.data.rows[0][1].text()),
            (expected.0.into(), expected.1.into())
        );
    }
    let result = compare_generated_page(
        session,
        table.clone(),
        BrowseOptions {
            filter: Some(leaf(
                "label",
                FilterPredicate::Equals("x' OR 1=1 --".into()),
            )),
            ..page()
        },
    )
    .await?;
    assert_eq!(result.total, 1);
    let result = compare_generated_page(
        session,
        table.clone(),
        BrowseOptions {
            filter: Some(FilterExpression::All(vec![
                leaf("label", FilterPredicate::NotEquals("beta".into())),
                leaf("amount", FilterPredicate::LessThan("20".into())),
            ])),
            ..page()
        },
    )
    .await?;
    assert_eq!(result.total, 2, "negative comparisons exclude NULL");
    for filter in [
        FilterExpression::Any(vec![]),
        leaf("missing", FilterPredicate::IsNull),
    ] {
        assert!(
            compare_generated_page(
                session,
                table.clone(),
                BrowseOptions {
                    filter: Some(filter),
                    ..page()
                }
            )
            .await
            .is_err()
        );
    }
    assert!(
        compare_generated_page(
            session,
            table,
            BrowseOptions {
                sort: vec![("missing".into(), false)],
                ..page()
            }
        )
        .await
        .is_err()
    );
    session.query(format!("CREATE TABLE {schema}.view_parent(tenant INTEGER, id INTEGER, PRIMARY KEY(tenant,id)); CREATE TABLE {schema}.view_child(id INTEGER PRIMARY KEY, tenant INTEGER, parent INTEGER, FOREIGN KEY(tenant,parent) REFERENCES {schema}.view_parent(tenant,id)); INSERT INTO {schema}.view_parent VALUES(1,1),(2,1); INSERT INTO {schema}.view_child VALUES(1,1,1),(2,2,1),(3,NULL,NULL);").replace("REFERENCES main.", "REFERENCES ")).await?;
    let child = TableRef {
        schema: schema.into(),
        name: "view_child".into(),
        kind: RelationKind::Table,
    };
    let key = session.foreign_keys(child.clone(), false).await?.remove(0);
    let scoped = compare_generated_page(
        session,
        child,
        BrowseOptions {
            relationship: Some(RelationshipScope {
                key,
                incoming: true,
                values: vec![Cell::Integer(1), Cell::Integer(1)],
            }),
            filter: Some(FilterExpression::Any(vec![
                leaf("id", FilterPredicate::Equals("1".into())),
                leaf("id", FilterPredicate::Equals("2".into())),
            ])),
            ..page()
        },
    )
    .await?;
    assert_eq!(
        scoped.total, 1,
        "relationship scope must AND the entire OR group"
    );
    assert_eq!(scoped.data.rows[0][0].text(), "1");
    Ok(())
}

#[test]
fn sqlite_grouped_views() -> Result<()> {
    runtime()?.block_on(async {
        let session = Session::connect(sqlite_profile(":memory:".into()), String::new()).await?;
        grouped_views(&session, "main").await
    })
}

#[test]
#[ignore = "requires disposable PostgreSQL"]
fn postgres_grouped_views() -> Result<()> {
    runtime()?.block_on(async {
        let (profile, password) = postgres_profile()?;
        let session = Session::connect(profile, password).await?;
        let schema = format!("views_{}", Uuid::new_v4().simple());
        session.query(format!("CREATE SCHEMA {schema}")).await?;
        let result = grouped_views(&session, &schema).await;
        session
            .query(format!("DROP SCHEMA {schema} CASCADE"))
            .await?;
        result
    })
}

#[test]
fn sqlite_completion_catalog_session_scopes_and_transactions() -> Result<()> {
    runtime()?.block_on(async {
        let session=fixture().await?;
        session.query("CREATE TABLE \"Mixed table\"(\"select\" TEXT, \"日本語\" INTEGER); CREATE TEMP TABLE items(temp_value TEXT); ATTACH ':memory:' AS other; CREATE TABLE other.attached(value BLOB);".into()).await?;
        let names=session.completion_catalog(vec![]).await;
        assert!(names.relations.iter().all(|r|r.columns.is_none()));
        assert!(names.relations.iter().any(|r|r.schema=="temp" && r.name=="items" && r.visible));
        assert!(names.relations.iter().any(|r|r.schema=="main" && r.name=="items" && !r.visible));
        assert!(names.relations.iter().any(|r|r.schema=="other" && r.name=="attached"));
        let catalog=session.completion_catalog(vec![("main".into(),"Mixed table".into())]).await;
        let columns=catalog.relations.iter().find(|r|r.name=="Mixed table").unwrap().columns.as_ref().unwrap();
        assert_eq!(columns[0].name,"select"); assert_eq!(columns[1].name,"日本語"); assert_eq!(columns[1].detail,"INTEGER");
        session.query("BEGIN; CREATE TABLE uncommitted(value TEXT)".into()).await?;
        session.invalidate_completions();
        let during=session.completion_catalog(vec![("temp".into(),"items".into())]).await;
        assert!(during.status.contains("transaction"));
        assert!(!during.relations.iter().any(|r|r.name=="uncommitted"));
        assert_eq!(session.transaction_status().await?,TransactionStatus::Open);
        assert!(during.relations.iter().find(|r|r.name=="Mixed table").unwrap().columns.is_some());
        session.query("ROLLBACK".into()).await?;
        let other=fixture().await?;
        assert!(!other.completion_catalog(vec![]).await.relations.iter().any(|r|r.schema=="temp"));
        session.query("CREATE TEMP TABLE fresh(value TEXT)".into()).await?;
        assert!(session.completion_catalog(vec![]).await.relations.iter().any(|r|r.name=="fresh"));
        Ok(())
    })
}

#[test]
#[ignore = "requires QUERI_TEST_POSTGRES pointing at a disposable PostgreSQL database"]
fn postgres_completion_catalog_session_scopes_and_transactions() -> Result<()> {
    runtime()?.block_on(async {
        let (profile,password)=postgres_profile()?;
        let session=Session::connect(profile.clone(),password.clone()).await?;
        session.query("CREATE TEMP TABLE \"Mixed table\"(\"select\" text, \"日本語\" integer); COMMENT ON COLUMN \"Mixed table\".\"日本語\" IS 'column help'; SET search_path=pg_temp;".into()).await?;
        let names=session.completion_catalog(vec![]).await;
        let table=names.relations.iter().find(|r|r.name=="Mixed table").expect("session temp table");
        assert!(table.visible); assert!(table.columns.is_none());
        let key=(table.schema.clone(),table.name.clone());
        let catalog=session.completion_catalog(vec![key.clone()]).await;
        let columns=catalog.relations.iter().find(|r|r.name=="Mixed table").unwrap().columns.as_ref().unwrap();
        assert_eq!(columns[0].name,"select");assert_eq!(columns[1].detail,"integer column help");
        session.query("BEGIN; CREATE TEMP TABLE uncommitted(value text)".into()).await?;
        session.invalidate_completions();
        assert!(session.completion_catalog(vec![key.clone()]).await.status.contains("transaction"));
        assert_eq!(session.transaction_status().await?,TransactionStatus::Open);
        assert!(session.query("SELECT 1/0".into()).await.is_err());
        session.invalidate_completions();
        assert!(session.completion_catalog(vec![key]).await.status.contains("transaction"));
        assert_eq!(session.transaction_status().await?,TransactionStatus::Failed);
        session.query("ROLLBACK".into()).await?;
        assert!(!session.completion_catalog(vec![]).await.relations.iter().any(|r|r.name=="uncommitted"));
        let other=Session::connect(profile,password).await?;
        assert!(!other.completion_catalog(vec![]).await.relations.iter().any(|r|r.name=="Mixed table"));
        session.query("CREATE TEMP TABLE fresh(value text)".into()).await?;
        assert!(session.completion_catalog(vec![]).await.relations.iter().any(|r|r.name=="fresh"));
        Ok(())
    })
}

#[test]
#[ignore = "requires QUERI_TEST_POSTGRES and role creation permission"]
fn postgres_completion_permissions_search_path_and_disconnection() -> Result<()> {
    runtime()?.block_on(async {
        let (profile,password)=postgres_profile()?;
        let session=Session::connect(profile.clone(),password.clone()).await?;
        let admin=Session::connect(profile,password).await?;
        let suffix=Uuid::new_v4().simple().to_string();
        let schema=format!("completion_{suffix}");let role=format!("completion_role_{suffix}");
        session.query(format!("CREATE SCHEMA {schema}; CREATE TABLE {schema}.items(secret text, allowed integer); CREATE VIEW {schema}.item_view AS SELECT allowed FROM {schema}.items; CREATE ROLE {role}; GRANT USAGE ON SCHEMA {schema} TO {role}; GRANT SELECT(allowed) ON {schema}.items TO {role}; CREATE TEMP TABLE items(temp_value text); SET search_path={schema},pg_temp")).await?;
        let outcome: Result<()> = async {
            let catalog=session.completion_catalog(vec![]).await;
            anyhow::ensure!(catalog.relations.iter().any(|r|r.name=="items" && r.schema==schema && r.visible));
            anyhow::ensure!(catalog.relations.iter().any(|r|r.name=="item_view"));
            session.query("SET search_path=pg_temp".into()).await?;
            let catalog=session.completion_catalog(vec![]).await;
            anyhow::ensure!(!catalog.relations.iter().any(|r|r.name=="items" && r.schema==schema && r.visible));
            session.query(format!("SET ROLE {role}")).await?;
            let catalog=session.completion_catalog(vec![(schema.clone(),"items".into())]).await;
            let columns=catalog.relations.iter().find(|r|r.schema==schema && r.name=="items").and_then(|r|r.columns.as_ref()).unwrap();
            anyhow::ensure!(columns.len()==1 && columns[0].name=="allowed");
            anyhow::ensure!(!catalog.relations.iter().any(|r|r.schema==schema && r.name=="item_view"));
            session.query("RESET ROLE".into()).await?;
            let pid=session.query("SELECT pg_backend_pid()".into()).await?.sets[0].rows[0][0].text();
            admin.query(format!("SELECT pg_terminate_backend({pid})")).await?;
            session.invalidate_completions();
            let catalog=session.completion_catalog(vec![]).await;
            anyhow::ensure!(catalog.status.contains("unavailable"));
            anyhow::ensure!(session.query("SELECT 1".into()).await.is_err());
            Ok(())
        }.await;
        let _=session.query("RESET ROLE".into()).await;
        admin.query(format!("DROP SCHEMA {schema} CASCADE; DROP ROLE {role}")).await?;
        outcome
    })
}

#[test]
#[ignore = "requires QUERI_TEST_POSTGRES pointing at a disposable PostgreSQL database"]
fn postgres_completion_busy_execution_never_queues_catalog_sql() -> Result<()> {
    runtime()?.block_on(async {
        let (profile,password)=postgres_profile()?;
        let session=Session::connect(profile.clone(),password.clone()).await?;
        let observer=Session::connect(profile,password).await?;
        session.query("CREATE TEMP TABLE cached(value text)".into()).await?;
        let pid=session.query("SELECT pg_backend_pid()".into()).await?.sets[0].rows[0][0].text();
        session.completion_catalog(vec![]).await;
        let running=session.clone();
        let job=tokio::spawn(async move {running.query("BEGIN; SELECT pg_sleep(30)".into()).await});
        tokio::time::timeout(Duration::from_secs(5),async {
            loop {
                let state=observer.query(format!("SELECT count(*) FROM pg_stat_activity WHERE pid={pid} AND wait_event='PgSleep'")).await?;
                if state.sets[0].rows[0][0].text()=="1" {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_,anyhow::Error>(())
        }).await??;
        let snapshot=tokio::time::timeout(Duration::from_secs(1),session.completion_catalog(vec![])).await?;
        anyhow::ensure!(snapshot.status.contains("busy"));
        anyhow::ensure!(snapshot.relations.iter().any(|r|r.name=="cached"));
        session.cancel().await?;
        anyhow::ensure!(job.await?.is_err());
        anyhow::ensure!(session.transaction_status().await?==TransactionStatus::Failed);
        anyhow::ensure!(session.completion_catalog(vec![]).await.status.contains("transaction"));
        session.query("ROLLBACK".into()).await?;
        anyhow::ensure!(session.completion_catalog(vec![]).await.status.contains("ready"));
        Ok(())
    })
}

async fn parameter_execution(session: Arc<Session>, pg: bool) -> Result<()> {
    use queri::{parameters::BoundQuery, sql::SqlDialect};
    let dialect = if pg {
        SqlDialect::Postgres
    } else {
        SqlDialect::Sqlite
    };
    let text = "雪'; DROP TABLE parameter_fixture; --";
    session
        .query("CREATE TEMP TABLE parameter_fixture(value text)".into())
        .await?;
    let sql = if pg {
        "SELECT :value::text, :value::text, :empty::text, :null::text"
    } else {
        "SELECT :value, :value, :empty, :null"
    };
    let p = queri::parameters::discover(sql, dialect)?;
    let out = session
        .query_params(p.bind(vec![
            Cell::Text(text.into()),
            Cell::Text("".into()),
            Cell::Null,
        ])?)
        .await?;
    assert_eq!(
        out.sets[0].rows[0],
        vec![
            Cell::Text(text.into()),
            Cell::Text(text.into()),
            Cell::Text("".into()),
            Cell::Null
        ]
    );
    assert!(
        session
            .query_params(BoundQuery {
                sql: "INSERT INTO parameter_fixture VALUES(:x); DELETE FROM parameter_fixture"
                    .into(),
                values: vec![Cell::Text(text.into())]
            })
            .await
            .is_err()
    );
    assert!(
        session
            .query_params(BoundQuery {
                sql: "INSERT INTO parameter_fixture VALUES(:x)".into(),
                values: vec![]
            })
            .await
            .is_err()
    );
    assert!(
        session
            .query("SELECT * FROM parameter_fixture".into())
            .await?
            .sets[0]
            .rows
            .is_empty()
    );
    session.query("BEGIN".into()).await?;
    session
        .query_params(BoundQuery {
            sql: "INSERT INTO parameter_fixture VALUES(:x)".into(),
            values: vec![Cell::Text(text.into())],
        })
        .await?;
    assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
    session.query("ROLLBACK".into()).await?;
    if !pg {
        let out = session
            .query_params(BoundQuery {
                sql: "SELECT ?1, ?2, ?3, ?1".into(),
                values: vec![
                    Cell::Integer(i64::MAX),
                    Cell::Real(1.25),
                    Cell::Blob(vec![0, 255]),
                ],
            })
            .await?;
        assert_eq!(
            out.sets[0].rows[0],
            vec![
                Cell::Integer(i64::MAX),
                Cell::Real(1.25),
                Cell::Blob(vec![0, 255]),
                Cell::Integer(i64::MAX)
            ]
        );
        session.set_statement_timeout(StatementTimeout::custom(20)?)?;
        let error = session.query_params(BoundQuery { sql: "WITH RECURSIVE n(x) AS (SELECT ?1 UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n".into(), values: vec![Cell::Integer(1)] }).await.unwrap_err();
        assert!(queri::database::is_statement_timeout(&error));
    }
    Ok(())
}
#[test]
fn sqlite_parameter_execution() -> Result<()> {
    runtime()?.block_on(async { parameter_execution(fixture().await?, false).await })
}
#[test]
#[ignore = "requires QUERI_TEST_POSTGRES"]
fn postgres_parameter_execution() -> Result<()> {
    runtime()?.block_on(async {
        let (p, password) = postgres_profile()?;
        parameter_execution(Session::connect(p, password).await?, true).await
    })
}

async fn quoted_generated_sql(session: &Session, schema: &str) -> Result<()> {
    let name = "odd\" 雪 table";
    let qualified = queri::sql::qualified(schema, name)?;
    session.query(format!("CREATE TABLE {qualified}(\"key\" INTEGER PRIMARY KEY, \"a'b 雪\" TEXT, \"order\" INTEGER); INSERT INTO {qualified} VALUES(1,'雪''; --',2),(2,'雪''; --',2),(3,NULL,1),(4,'',3)")).await?;
    let table = TableRef {
        schema: schema.into(),
        name: name.into(),
        kind: RelationKind::Table,
    };
    let options = BrowseOptions {
        filter: Some(FilterExpression::Any(vec![
            TableFilter {
                column: "a'b 雪".into(),
                predicate: FilterPredicate::Equals("雪'; --".into()),
            }
            .into(),
            TableFilter {
                column: "a'b 雪".into(),
                predicate: FilterPredicate::IsNull,
            }
            .into(),
        ])),
        sort: vec![("order".into(), false)],
        limit: 1,
        offset: 1,
        relationship: None,
    };
    let query = session.browse_sql(table.clone(), options.clone()).await?;
    assert!(!query.sql.contains("雪'; --"));
    let result = compare_generated_page(session, table.clone(), options).await?;
    assert_eq!(result.data.rows.len(), 1);
    compare_generated_page(session, table, page()).await?;
    session.query(format!("DROP TABLE {qualified}")).await?;
    Ok(())
}
#[test]
fn sqlite_generated_quoted_sql() -> Result<()> {
    runtime()?.block_on(async {
        let session = fixture().await?;
        quoted_generated_sql(&session,"main").await?;
        session.query("CREATE TABLE blob_parent(k BLOB PRIMARY KEY); CREATE TABLE blob_child(id INTEGER PRIMARY KEY, k BLOB REFERENCES blob_parent(k)); INSERT INTO blob_parent VALUES(X'00ff'); INSERT INTO blob_child VALUES(1,X'00ff')".into()).await?;
        let table = TableRef {schema:"main".into(),name:"blob_child".into(),kind:RelationKind::Table};
        let key = session.foreign_keys(table.clone(),false).await?.remove(0);
        compare_generated_page(&session,table,BrowseOptions {relationship:Some(RelationshipScope {key,incoming:true,values:vec![Cell::Blob(vec![0,255])]}),..page()}).await?;
        Ok(())
    })
}
#[test]
#[ignore = "requires QUERI_TEST_POSTGRES"]
fn postgres_generated_quoted_sql() -> Result<()> {
    runtime()?.block_on(async {
        let (profile, password) = postgres_profile()?;
        let session = Session::connect(profile, password).await?;
        session
            .query("CREATE TEMP TABLE initialize_temp(n integer)".into())
            .await?;
        let schema = session
            .query("SELECT nspname FROM pg_namespace WHERE oid=pg_my_temp_schema()".into())
            .await?
            .sets[0]
            .rows[0][0]
            .text();
        quoted_generated_sql(&session, &schema).await
    })
}

#[test]
fn sqlite_parameter_limits_and_cancel() -> Result<()> {
    use queri::parameters::BoundQuery;
    runtime()?.block_on(async {
        let session = fixture().await?;
        let query = BoundQuery {sql:"WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x < :limit) SELECT x FROM n".into(),values:vec![Cell::Integer(11000)]};
        let result = session.query_params(query).await?;
        assert!(result.sets[0].truncated);
        assert_eq!(result.sets[0].rows.len(),MAX_RESULT_ROWS);
        assert_eq!(result.sets[0].affected,11000);
        let running = session.clone();
        let query = tokio::spawn(async move { running.query_params(BoundQuery {sql:"WITH RECURSIVE n(x) AS (SELECT :start UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n".into(),values:vec![Cell::Integer(1)]}).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        session.cancel().await?;
        assert!(tokio::time::timeout(Duration::from_secs(5),query).await??.is_err());
        assert_eq!(session.transaction_status().await?,TransactionStatus::Idle);
        assert_eq!(session.query_params(BoundQuery {sql:"SELECT :x".into(),values:vec![Cell::Integer(42)]}).await?.sets[0].rows[0][0],Cell::Integer(42));
        Ok(())
    })
}
