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
        kind: "table".into(),
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
                    filter: Some(TableFilter {
                        column: "name".into(),
                        predicate: FilterPredicate::Contains("al".into()),
                    }),
                    sort: Some(("quantity".into(), true)),
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
        assert_eq!(session.tables("main".into()).await?.len(), 1);
        Ok(())
    })
}

async fn filter_workflows(session: &Session, schema: &str) -> Result<()> {
    use FilterPredicate::*;
    let table = TableRef {
        schema: schema.into(),
        name: "filter\"items".into(),
        kind: "table".into(),
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
                    filter: Some(TableFilter {
                        column: column.into(),
                        predicate,
                    }),
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
                filter: Some(TableFilter {
                    column: label.into(),
                    predicate: StartsWith("al".into()),
                }),
                sort: Some(("id".into(), true)),
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
                        filter: Some(TableFilter {
                            column: column.into(),
                            predicate
                        }),
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
                    filter: Some(TableFilter {
                        column: "quantity".into(),
                        predicate: FilterPredicate::GreaterThan("20".into()),
                    }),
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
                        filter: Some(TableFilter {
                            column: column.into(),
                            predicate,
                        }),
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

#[test]
#[ignore = "requires an unlocked desktop Secret Service"]
fn secret_service_credential_round_trip() -> Result<()> {
    let id = Uuid::new_v4();
    assert_eq!(queri::storage::load_password(id)?, None);
    queri::storage::save_password(id, "queri-disposable-test-secret")?;
    let loaded = queri::storage::load_password(id)?;
    queri::storage::delete_password(id)?;
    assert_eq!(loaded.as_deref(), Some("queri-disposable-test-secret"));
    assert_eq!(queri::storage::load_password(id)?, None);
    Ok(())
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
        let table = TableRef { schema: schema.clone(), name: "items".into(), kind: "table".into() };
        let data = session.browse(table.clone(), page()).await?;
        assert_eq!(data.total, 2);
        assert_eq!(data.data.rows[0][3].text(), "12345678901234567890.1234567890");
        assert_eq!(data.data.rows[0][5].text(), "\\x00ff");
        let exact = session.browse(table.clone(), BrowseOptions {
            filter: Some(TableFilter { column: "amount".into(), predicate: FilterPredicate::Equals("12345678901234567890.12345678901".into()) }),
            ..page()
        }).await?;
        assert_eq!(exact.total, 0, "filter input must not be rounded to the column's scale");
        let original = data.data.rows[0].clone();
        let mut values: Vec<_> = original.iter().cloned().map(FieldValue::Value).collect();
        values[2] = FieldValue::Value(Cell::Text("Changed".into()));
        session.change_rows(table.clone(), RowChange::Update { original: original.clone(), values: values.clone() }).await?;
        assert!(session.change_rows(table.clone(), RowChange::Update { original, values }).await.is_err());
        let filtered = session.browse(table.clone(), BrowseOptions { filter: Some(TableFilter { column: "name".into(), predicate: FilterPredicate::Contains("hang".into()) }), ..page() }).await?;
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
