use queri::schema::*;
#[test]
fn conservative_planner() {
    let baseline = Snapshot {
        engine: Engine::Sqlite,
        engine_version: String::new(),
        scope: Scope {
            namespace: "main".into(),
        },
        tables: vec![],
    };
    let mut desired = baseline.clone();
    desired.tables.push(Table {
        constraint_names: Vec::new(),
        name: "a\"b".into(),
        columns: vec![Column {
            name: "x".into(),
            data_type: "TEXT".into(),
            nullable: true,
            default: None,
            primary_key_position: 0,
            generated: false,
            identity: None,
            collation: None,
        }],
        definition: None,
        unsupported: vec![],
        indexes: vec![],
        constraints: vec![],
        dependencies: vec![],
        options: vec![],
    });
    let plan = plan(&baseline, &desired).unwrap();
    assert!(plan.blocked.is_empty());
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(&plan.sql()[0]).unwrap();
    desired.tables[0].columns[0].data_type = "TEXT); DROP TABLE x; --".into();
    assert!(
        !queri::schema::plan(&baseline, &desired)
            .unwrap()
            .blocked
            .is_empty()
    );
}

#[test]
fn selection_and_blocked_sql() {
    let baseline = Snapshot {
        engine: Engine::Sqlite,
        engine_version: String::new(),
        scope: Scope {
            namespace: "main".into(),
        },
        tables: vec![Table {
            constraint_names: Vec::new(),
            name: "keep".into(),
            columns: vec![],
            definition: None,
            indexes: vec![],
            constraints: vec![],
            dependencies: vec![],
            options: vec![],
            unsupported: vec![],
        }],
    };
    let mut desired = baseline.clone();
    desired.tables.clear();
    let blocked = plan(&baseline, &desired).unwrap();
    assert!(!blocked.blocked.is_empty());
    assert!(blocked.sql().is_empty());
    let selected = plan_with_options(
        &baseline,
        &desired,
        &PlanOptions {
            selected_tables: Some(vec![]),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(selected.blocked.is_empty());
    assert!(selected.sql().is_empty());
    assert_eq!(selected.expected, baseline);
}

fn rich_snapshot(engine: Engine) -> Snapshot {
    let mut snapshot = Snapshot {
        engine,
        engine_version: String::new(),
        scope: Scope {
            namespace: "main".into(),
        },
        tables: vec![],
    };
    for (name, target) in [("a", "b"), ("b", "a")] {
        snapshot.tables.push(Table {
            constraint_names: Vec::new(),
            name: name.into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "INTEGER".into(),
                nullable: false,
                default: Some("1 + 2".into()),
                primary_key_position: 1,
                generated: false,
                identity: None,
                collation: None,
            }],
            definition: None,
            indexes: vec![Index {
                name: format!("{name}_id"),
                unique: false,
                columns: vec!["id".into()],
                definition: None,
            }],
            constraints: vec![
                Constraint::Check {
                    expression: "id > 0".into(),
                },
                Constraint::ForeignKey {
                    columns: vec!["id".into()],
                    target: target.into(),
                    referenced_columns: vec!["id".into()],
                    on_update: "CASCADE".into(),
                    on_delete: "NO ACTION".into(),
                    deferral: Some("DEFERRABLE INITIALLY DEFERRED".into()),
                },
            ],
            dependencies: vec![],
            options: vec![],
            unsupported: vec![],
        });
    }
    snapshot
}
#[test]
fn rich_sqlite_creation_and_validation() {
    let desired = rich_snapshot(Engine::Sqlite);
    let mut baseline = desired.clone();
    baseline.tables.clear();
    let planned = plan(&baseline, &desired).unwrap();
    assert!(planned.blocked.is_empty(), "{:?}", planned.blocked);
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    for sql in planned.sql() {
        conn.execute_batch(sql).unwrap();
    }
    conn.execute_batch(
        "BEGIN; INSERT INTO a DEFAULT VALUES; INSERT INTO b DEFAULT VALUES; COMMIT;",
    )
    .unwrap();
    assert!(conn.execute("INSERT INTO a VALUES(-1)", []).is_err());
    assert!(
        !plan_with_options(
            &baseline,
            &desired,
            &PlanOptions {
                selected_tables: Some(vec!["a".into()]),
                ..Default::default()
            }
        )
        .unwrap()
        .blocked
        .is_empty()
    );
    let mut invalid = desired.clone();
    invalid.tables[0].columns[0].default = Some("1) ; DROP TABLE a; --".into());
    assert!(!plan(&baseline, &invalid).unwrap().blocked.is_empty());
    invalid = desired.clone();
    let duplicate = invalid.tables[0].constraints[0].clone();
    invalid.tables[0].constraints.push(duplicate);
    assert!(!plan(&baseline, &invalid).unwrap().blocked.is_empty());
}
#[tokio::test]
async fn rich_postgres_cycles() {
    let Ok(url) = std::env::var("QUERI_SCHEMA_TEST_POSTGRES") else {
        return;
    };
    let (mut client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move { connection.await.unwrap() });
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("CREATE SCHEMA main").await.unwrap();
    let desired = rich_snapshot(Engine::Postgres);
    let mut baseline = desired.clone();
    baseline.tables.clear();
    let planned = plan(&baseline, &desired).unwrap();
    assert!(planned.blocked.is_empty(), "{:?}", planned.blocked);
    assert!(
        planned
            .sql()
            .iter()
            .rev()
            .take(2)
            .all(|s| s.starts_with("ALTER TABLE"))
    );
    for sql in planned.sql() {
        tx.batch_execute(sql).await.unwrap();
    }
    tx.batch_execute("INSERT INTO main.a DEFAULT VALUES; INSERT INTO main.b DEFAULT VALUES; SET CONSTRAINTS ALL IMMEDIATE").await.unwrap();
    let keys: i64 = tx.query_one("SELECT count(*) FROM pg_constraint c JOIN pg_namespace n ON n.oid=c.connamespace WHERE n.nspname='main' AND c.contype='f' AND c.condeferrable AND c.condeferred", &[]).await.unwrap().get(0);
    assert_eq!(keys, 2);
    tx.rollback().await.unwrap();
}

#[test]
fn session_alteration_stale_and_rollback() {
    use queri::{
        database::{Session, runtime},
        model::*,
    };
    runtime().unwrap().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::connect(
            ConnectionProfile {
                ssh: None,
                id: uuid::Uuid::new_v4(),
                name: "Schema".into(),
                notes: String::new(),
                color: String::new(),
                environment: None,
                policy: ExecutionPolicy::Normal,
                remember_password: false,
                config: ConnectionConfig::Sqlite {
                    path: dir.path().join("schema.db"),
                    read_only: false,
                    create: true,
                    wal: false,
                    foreign_keys: true,
                },
            },
            String::new(),
        )
        .await
        .unwrap();
        session
            .query("CREATE TABLE items(value TEXT); INSERT INTO items VALUES('kept')".into())
            .await
            .unwrap();
        let scope = Scope {
            namespace: "main".into(),
        };
        let initial = session.schema_snapshot(scope.clone()).await.unwrap();
        let mut changed = initial.clone();
        changed.tables[0].columns[0].nullable = false;
        changed.tables[0].columns[0].default = Some("'fallback'".into());
        let rebuild = plan_with_options(&initial, &changed, &PlanOptions::default()).unwrap();
        assert!(rebuild.blocked.is_empty(), "{:?}", rebuild.blocked);
        session.apply_schema_plan(rebuild).await.unwrap();
        let baseline = session.schema_snapshot(scope.clone()).await.unwrap();
        let mut desired = baseline.clone();
        desired.tables[0].columns[0].name = "renamed".into();
        let mut options = PlanOptions::default();
        options
            .column_renames
            .entry("items".into())
            .or_default()
            .insert("value".into(), "renamed".into());
        let rename = plan_with_options(&baseline, &desired, &options).unwrap();
        assert!(rename.blocked.is_empty(), "{:?}", rename.blocked);
        session.apply_schema_plan(rename.clone()).await.unwrap();
        assert!(
            session
                .apply_schema_plan(rename)
                .await
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
        session
            .query("SELECT renamed FROM items WHERE renamed = 'kept'".into())
            .await
            .unwrap();
        let baseline = session.schema_snapshot(scope.clone()).await.unwrap();
        let mut desired = baseline.clone();
        let mut added = desired.tables[0].columns[0].clone();
        added.name = "added".into();
        desired.tables[0].columns.push(added);
        let mut mismatch = plan(&baseline, &desired).unwrap();
        mismatch.expected.tables[0].columns[0].nullable = !baseline.tables[0].columns[0].nullable;
        assert!(
            session
                .apply_schema_plan(mismatch)
                .await
                .unwrap_err()
                .to_string()
                .contains("verification")
        );
        assert_eq!(
            session.schema_snapshot(scope.clone()).await.unwrap(),
            baseline
        );
        let valid = plan(&baseline, &desired).unwrap();
        session.apply_schema_plan(valid).await.unwrap();
        let current = session.schema_snapshot(scope).await.unwrap();
        assert!(!plan(&current, &baseline).unwrap().blocked.is_empty());
        let drop = plan_with_options(
            &current,
            &baseline,
            &PlanOptions {
                allow_destructive: true,
                ..Default::default()
            },
        )
        .unwrap();
        session.apply_schema_plan(drop).await.unwrap();
    });
}

#[test]
fn postgres_columns_preserve_data() {
    let Ok(url) = std::env::var("QUERI_SCHEMA_TEST_POSTGRES") else {
        return;
    };
    queri::database::runtime().unwrap().block_on(async {
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .unwrap();
        tokio::spawn(async move { connection.await.unwrap() });
        client
            .batch_execute(
                "CREATE TEMP TABLE alteration(value integer); INSERT INTO alteration VALUES(42)",
            )
            .await
            .unwrap();
        let mut baseline = rich_snapshot(Engine::Postgres);
        baseline.scope.namespace = "pg_temp".into();
        baseline.tables.truncate(1);
        let table = &mut baseline.tables[0];
        table.name = "alteration".into();
        table.constraints.clear();
        table.indexes.clear();
        table.columns[0].name = "value".into();
        table.columns[0].primary_key_position = 0;
        table.columns[0].nullable = true;
        table.columns[0].default = None;
        let mut desired = baseline.clone();
        desired.tables[0].columns[0].name = "renamed".into();
        desired.tables[0].columns[0].data_type = "bigint".into();
        desired.tables[0].columns[0].default = Some("7".into());
        desired.tables[0].columns[0].nullable = false;
        let mut options = PlanOptions {
            allow_destructive: true,
            ..Default::default()
        };
        options
            .column_renames
            .entry("alteration".into())
            .or_default()
            .insert("value".into(), "renamed".into());
        let plan = plan_with_options(&baseline, &desired, &options).unwrap();
        assert!(plan.blocked.is_empty(), "{:?}", plan.blocked);
        for sql in plan.sql() {
            client.batch_execute(sql).await.unwrap();
        }
        assert_eq!(
            client
                .query_one("SELECT renamed FROM alteration", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            42
        );
        client
            .batch_execute("INSERT INTO alteration DEFAULT VALUES")
            .await
            .unwrap();
        assert_eq!(
            client
                .query_one("SELECT renamed FROM alteration WHERE renamed=7", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            7
        );
        assert!(
            client
                .batch_execute("INSERT INTO alteration VALUES(NULL)")
                .await
                .is_err()
        );
    });
}

#[test]
fn postgres_explicit_rename_drop_and_mapping() {
    let baseline = Snapshot {
        engine: Engine::Postgres,
        engine_version: String::new(),
        scope: Scope {
            namespace: "source".into(),
        },
        tables: vec![Table {
            constraint_names: Vec::new(),
            name: "before".into(),
            columns: vec![],
            definition: None,
            indexes: vec![],
            constraints: vec![],
            dependencies: vec![],
            options: vec![],
            unsupported: vec![],
        }],
    };
    let mut desired = baseline.clone();
    desired.tables[0].name = "after".into();
    let mut options = PlanOptions::default();
    options
        .table_renames
        .insert("before".into(), "after".into());
    let rename = plan_with_options(&baseline, &desired, &options).unwrap();
    assert!(rename.blocked.is_empty(), "{:?}", rename.blocked);
    assert_eq!(
        rename.sql(),
        &["ALTER TABLE \"source\".\"before\" RENAME TO \"after\""]
    );
    assert_eq!(rename.expected, desired);
    options.table_renames.clear();
    desired.tables.clear();
    assert!(
        !plan_with_options(&baseline, &desired, &options)
            .unwrap()
            .blocked
            .is_empty()
    );
    options.allow_destructive = true;
    assert_eq!(
        plan_with_options(&baseline, &desired, &options)
            .unwrap()
            .sql(),
        &["DROP TABLE \"source\".\"before\""]
    );
    let mapped = map_scope(
        &baseline,
        Scope {
            namespace: "target".into(),
        },
    )
    .unwrap();
    assert_eq!(mapped.scope.namespace, "target");
    let mut dependent = baseline.clone();
    dependent.tables[0]
        .dependencies
        .push("view: source.report".into());
    assert!(
        !plan_with_options(&dependent, &desired, &options)
            .unwrap()
            .blocked
            .is_empty()
    );
    assert!(map_scope(&dependent, mapped.scope).is_err());
}

#[test]
fn postgres_session_constraint_indexes() {
    use queri::{
        database::{Session, runtime},
        model::*,
    };
    let Ok(url) = std::env::var("QUERI_SCHEMA_TEST_POSTGRES") else {
        return;
    };
    runtime().unwrap().block_on(async {
        let config: tokio_postgres::Config = url.parse().unwrap();
        let host = match &config.get_hosts()[0] { tokio_postgres::config::Host::Tcp(h) => h.clone(), _ => panic!("TCP required") };
        let session = Session::connect(ConnectionProfile {
            ssh: None, id: uuid::Uuid::new_v4(), name: "schema test".into(), notes: String::new(), color: String::new(), environment: None,
            policy: ExecutionPolicy::Normal, remember_password: false,
            config: ConnectionConfig::Postgres {host, port: config.get_ports()[0], database: config.get_dbname().unwrap().into(), user: config.get_user().unwrap().into(), tls: TlsMode::Disable, timeout_seconds: 5},
        }, String::from_utf8(config.get_password().unwrap_or_default().to_vec()).unwrap()).await.unwrap();
        let namespace = format!("schema_alter_{}", uuid::Uuid::new_v4().simple());
        let target = format!("{namespace}_copy");
        session.query(format!("CREATE SCHEMA {namespace}; CREATE SCHEMA {target}; CREATE TABLE {namespace}.items(id integer PRIMARY KEY, value integer UNIQUE); INSERT INTO {namespace}.items VALUES(1,10),(2,20); CREATE INDEX value_idx ON {namespace}.items(value)")).await.unwrap();
        let baseline = session.schema_snapshot(Scope { namespace: namespace.clone() }).await.unwrap();
        assert!(baseline.tables[0].unsupported.is_empty(), "{:?}", baseline.tables[0].unsupported);
        let mut desired = baseline.clone();
        desired.tables[0].constraints.retain(|c| !matches!(c, Constraint::Unique { .. }));
        desired.tables[0].indexes.retain(|i| i.name.starts_with("automatic:"));
        desired.tables[0].indexes.push(Index {name: "id_value_idx".into(),unique:false,columns:vec!["id".into(),"value".into()],definition:None});
        let options = PlanOptions {allow_destructive:true,..Default::default()};
        assert!(!plan(&baseline,&desired).unwrap().blocked.is_empty());
        let changes = plan_with_options(&baseline,&desired,&options).unwrap();
        assert!(changes.blocked.is_empty(), "{:?}", changes.blocked);
        session.apply_schema_plan(changes).await.unwrap();
        let mapped = map_scope(&baseline, Scope {namespace:target.clone()}).unwrap();
        let empty = session.schema_snapshot(mapped.scope.clone()).await.unwrap();
        session.apply_schema_plan(plan(&empty,&mapped).unwrap()).await.unwrap();
        session.query(format!("INSERT INTO {namespace}.items VALUES(3,10)")).await.unwrap();
        let before_failure = session.schema_snapshot(baseline.scope.clone()).await.unwrap();
        let mut invalid = before_failure.clone();
        invalid.tables[0].constraints.push(Constraint::Unique {columns:vec!["value".into()]});
        let changes = plan_with_options(&before_failure,&invalid,&options).unwrap();
        assert!(changes.blocked.is_empty(), "{:?}",changes.blocked);
        assert!(session.apply_schema_plan(changes).await.is_err());
        assert_eq!(session.schema_snapshot(baseline.scope).await.unwrap(),before_failure);
        session.query(format!("DO $$ BEGIN IF (SELECT array_agg(value ORDER BY id) FROM {namespace}.items) <> ARRAY[10,20,10] THEN RAISE EXCEPTION 'data changed'; END IF; END $$")).await.unwrap();
        session.query(format!("DROP SCHEMA {namespace} CASCADE; DROP SCHEMA {target} CASCADE")).await.unwrap();
    });
}

#[test]
fn native_type_aliases_are_engine_specific() {
    for engine in [Engine::Postgres, Engine::Sqlite] {
        let mut baseline = rich_snapshot(engine.clone());
        baseline.tables[0].columns[0].data_type = "INTEGER".into();
        let mut desired = baseline.clone();
        desired.tables[0].columns[0].data_type = "INT".into();
        assert_eq!(
            compare(&baseline, &desired).is_empty(),
            engine == Engine::Postgres
        );
    }
}
