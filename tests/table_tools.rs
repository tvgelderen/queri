use anyhow::Result;
use queri::{
    database::{Session, runtime},
    model::*,
    parameters::BoundQuery,
    query_plan::{QueryPlan, explain_sql},
    row_updates::{PendingEdits, UpdateBatch},
    sql::SqlDialect,
};
use std::sync::Arc;

fn table(pg: bool) -> TableRef {
    TableRef {
        schema: if pg { "public" } else { "main" }.into(),
        name: "tool_items".into(),
        kind: RelationKind::Table,
    }
}
fn options() -> BrowseOptions {
    BrowseOptions {
        limit: 100,
        ..Default::default()
    }
}
async fn session(pg: bool, path: std::path::PathBuf) -> Result<Arc<Session>> {
    let (config, password) = if pg {
        let config: tokio_postgres::Config = std::env::var("QUERI_TEST_POSTGRES")?.parse()?;
        let host = match config.get_hosts().first() {
            Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
            _ => "localhost".into(),
        };
        (
            ConnectionConfig::Postgres {
                host,
                port: config.get_ports().first().copied().unwrap_or(5432),
                database: config.get_dbname().unwrap_or("postgres").into(),
                user: config.get_user().unwrap_or("postgres").into(),
                tls: TlsMode::Disable,
                timeout_seconds: 5,
            },
            String::from_utf8(config.get_password().unwrap_or_default().to_vec())?,
        )
    } else {
        (
            ConnectionConfig::Sqlite {
                path,
                read_only: false,
                create: true,
                wal: false,
                foreign_keys: true,
            },
            String::new(),
        )
    };
    let session = Session::connect(
        ConnectionProfile {
            ssh: None,
            id: uuid::Uuid::new_v4(),
            name: "Tools fixture".into(),
            notes: String::new(),
            color: String::new(),
            environment: None,
            policy: ExecutionPolicy::Normal,
            remember_password: false,
            config,
        },
        password,
    )
    .await?;
    session.query("DROP TABLE IF EXISTS tool_items; CREATE TABLE tool_items(id INTEGER PRIMARY KEY, name TEXT DEFAULT 'default', amount BIGINT); INSERT INTO tool_items VALUES(1,'Alpha',9007199254740993),(2,'Beta',20),(3,NULL,20);".into()).await?;
    Ok(session)
}
fn batch(page: &TablePage, changes: &[(usize, usize, FieldValue)]) -> UpdateBatch {
    let mut edits = PendingEdits::default();
    for (row, column, value) in changes {
        edits.set(&page.data.rows[*row], *column, value.clone());
    }
    UpdateBatch {
        columns: page.metadata.columns.clone(),
        rows: edits.rows,
    }
}
async fn workflow(pg: bool) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let session = session(pg, directory.path().join("database.sqlite")).await?;
    let peer_password = if pg {
        let config: tokio_postgres::Config = std::env::var("QUERI_TEST_POSTGRES")?.parse()?;
        String::from_utf8(config.get_password().unwrap_or_default().to_vec())?
    } else {
        String::new()
    };
    let peer = Session::connect(session.profile.clone(), peer_password).await?;
    let dialect = if pg {
        SqlDialect::Postgres
    } else {
        SqlDialect::Sqlite
    };
    let sql = explain_sql("SELECT * FROM tool_items WHERE id=1", dialect, false)?;
    let output = session.query(sql).await?;
    let plan = QueryPlan::parse(&output.sets[0], pg)?;
    assert!(!plan.nodes.is_empty());
    assert!(!plan.raw.is_empty());
    let bound = explain_sql(
        if pg {
            "SELECT * FROM tool_items WHERE id=$1"
        } else {
            "SELECT * FROM tool_items WHERE id=?1"
        },
        dialect,
        false,
    )?;
    let output = session
        .query_params(BoundQuery {
            sql: bound,
            values: vec![if pg {
                Cell::Text("1".into())
            } else {
                Cell::Integer(1)
            }],
        })
        .await?;
    assert!(!QueryPlan::parse(&output.sets[0], pg)?.nodes.is_empty());
    let output = session
        .query(explain_sql(
            "UPDATE tool_items SET name='not executed' WHERE id=1",
            dialect,
            false,
        )?)
        .await?;
    QueryPlan::parse(&output.sets[0], pg)?;
    let page = session.browse(table(pg), options()).await?;
    assert_eq!(page.data.rows[0][1], Cell::Text("Alpha".into()));
    let stats = session
        .column_statistics(table(pg), options(), "amount".into())
        .await?;
    assert_eq!(stats.rows, 3);
    assert_eq!(stats.distinct, Some(2));
    assert_eq!(stats.max.unwrap().text(), "9007199254740993");
    assert_eq!(stats.common[0].1, 2);
    let filtered = BrowseOptions {
        filter: Some(
            TableFilter {
                column: "id".into(),
                predicate: FilterPredicate::GreaterThan("1".into()),
            }
            .into(),
        ),
        offset: 1,
        limit: 1,
        ..Default::default()
    };
    let stats = session
        .column_statistics(table(pg), filtered, "name".into())
        .await?;
    assert_eq!(stats.rows, 2);
    assert_eq!(stats.nulls, 1);
    assert_eq!(stats.distinct, Some(1));
    let update = batch(
        &page,
        &[
            (0, 1, FieldValue::Value(Cell::Text("Changed".into()))),
            (1, 1, FieldValue::Null),
        ],
    );
    assert_eq!(session.apply_updates(table(pg), update).await?, 2);
    let page = session.browse(table(pg), options()).await?;
    assert_eq!(page.data.rows[0][1].text(), "Changed");
    assert_eq!(page.data.rows[1][1], Cell::Null);
    assert_eq!(page.data.rows[0][2].text(), "9007199254740993");
    // A conflict in the second row rolls back the first update.
    let update = batch(
        &page,
        &[
            (0, 1, FieldValue::Value(Cell::Text("rollback me".into()))),
            (1, 1, FieldValue::Value(Cell::Text("conflict".into()))),
        ],
    );
    peer.query("UPDATE tool_items SET name='external' WHERE id=2".into())
        .await?;
    assert!(session.apply_updates(table(pg), update).await.is_err());
    assert_eq!(
        session.browse(table(pg), options()).await?.data.rows[0][1].text(),
        "Changed"
    );
    // Saving inside an existing transaction must not commit earlier work.
    session
        .query("BEGIN; INSERT INTO tool_items VALUES(4,'prior',40)".into())
        .await?;
    let page = session.browse(table(pg), options()).await?;
    session
        .apply_updates(table(pg), batch(&page, &[(0, 1, FieldValue::Default)]))
        .await?;
    assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
    assert_eq!(
        session.browse(table(pg), options()).await?.data.rows[0][1].text(),
        "default"
    );
    let page = session.browse(table(pg), options()).await?;
    let conflict = batch(
        &page,
        &[
            (0, 1, FieldValue::Null),
            (
                1,
                0,
                FieldValue::Value(if pg {
                    Cell::Text("1".into())
                } else {
                    Cell::Integer(1)
                }),
            ),
        ],
    );
    assert!(session.apply_updates(table(pg), conflict).await.is_err());
    assert_eq!(session.transaction_status().await?, TransactionStatus::Open);
    assert_eq!(session.browse(table(pg), options()).await?.total, 4);
    session.query("ROLLBACK".into()).await?;
    let page = session.browse(table(pg), options()).await?;
    assert_eq!(page.total, 3);
    assert_eq!(page.data.rows[0][1].text(), "Changed");
    // A trigger on an unchanged column must not fire for sparse edits.
    if !pg {
        session.query("CREATE TRIGGER untouched_amount BEFORE UPDATE OF amount ON tool_items BEGIN SELECT RAISE(ABORT,'unchanged column touched'); END".into()).await?;
        session
            .apply_updates(
                table(pg),
                batch(
                    &page,
                    &[
                        (0, 1, FieldValue::Value(Cell::Text("Changed".into()))),
                        (1, 1, FieldValue::Value(Cell::Text("sparse".into()))),
                    ],
                ),
            )
            .await?;
    }
    let page = session.browse(table(pg), options()).await?;
    // Same column count is not enough to validate an edited row's schema.
    let update = batch(&page, &[(0, 1, FieldValue::Null)]);
    session
        .query("ALTER TABLE tool_items RENAME COLUMN name TO title".into())
        .await?;
    assert!(session.apply_updates(table(pg), update).await.is_err());
    // A timed-out batch rolls back its own writes and leaves its session usable.
    if pg {
        session.query("CREATE OR REPLACE FUNCTION tool_delay() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.title='slow' THEN PERFORM pg_sleep(10); END IF; RETURN NEW; END $$; CREATE TRIGGER tool_delay BEFORE UPDATE ON tool_items FOR EACH ROW EXECUTE FUNCTION tool_delay();".into()).await?;
    } else {
        session.query("CREATE TRIGGER tool_delay BEFORE UPDATE ON tool_items WHEN NEW.title='slow' BEGIN SELECT sum(n) FROM (WITH RECURSIVE x(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM x WHERE n<100000000) SELECT n FROM x); END;".into()).await?;
    }
    session.set_statement_timeout(StatementTimeout::custom(20)?)?;
    let page = session.browse(table(pg), options()).await?;
    assert!(
        session
            .apply_updates(
                table(pg),
                batch(
                    &page,
                    &[(0, 1, FieldValue::Value(Cell::Text("slow".into())))]
                )
            )
            .await
            .is_err()
    );
    assert_eq!(session.transaction_status().await?, TransactionStatus::Idle);
    assert_eq!(
        session.browse(table(pg), options()).await?.data.rows[0][1].text(),
        "Changed"
    );
    session.set_statement_timeout(StatementTimeout::NoTimeout)?;
    session
        .query("BEGIN; INSERT INTO tool_items VALUES(4,'prior',40)".into())
        .await?;
    let page = session.browse(table(pg), options()).await?;
    let update = batch(
        &page,
        &[(0, 1, FieldValue::Value(Cell::Text("slow".into())))],
    );
    let writer = session.clone();
    let saving = tokio::spawn(async move { writer.apply_updates(table(pg), update).await });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    session.cancel().await?;
    assert!(saving.await?.is_err());
    // SQLite may automatically roll back a transaction interrupted during a write.
    let state = session.transaction_status().await?;
    if state == TransactionStatus::Open {
        assert_eq!(session.browse(table(pg), options()).await?.total, 4);
        session.query("ROLLBACK".into()).await?;
    } else {
        assert_eq!(state, TransactionStatus::Idle);
    }
    assert_eq!(
        session.browse(table(pg), options()).await?.data.rows[0][1].text(),
        "Changed"
    );
    if pg {
        session.query("BEGIN".into()).await?;
        let output = session
            .query(explain_sql(
                "UPDATE tool_items SET title='analyzed' WHERE id=1",
                dialect,
                true,
            )?)
            .await?;
        QueryPlan::parse(&output.sets[0], true)?;
        session.query("ROLLBACK".into()).await?;
        assert_eq!(
            session.browse(table(pg), options()).await?.data.rows[0][1].text(),
            "Changed"
        );
    }
    Ok(())
}
#[test]
fn sqlite_table_tools() -> Result<()> {
    runtime()?.block_on(workflow(false))
}
#[test]
#[ignore = "requires QUERI_TEST_POSTGRES pointing to a disposable database"]
fn postgres_table_tools() -> Result<()> {
    runtime()?.block_on(workflow(true))
}
#[test]
fn plan_validation_and_pending_edits() -> Result<()> {
    explain_sql(
        "SELECT * FROM items WHERE id=:id",
        SqlDialect::Postgres,
        false,
    )?;
    assert!(explain_sql("SELECT 1; DELETE FROM x", SqlDialect::Sqlite, false).is_err());
    assert!(explain_sql("BEGIN", SqlDialect::Postgres, false).is_err());
    assert!(explain_sql("SELECT 1", SqlDialect::Sqlite, true).is_err());
    let data = ResultSet {
        columns: vec!["value".into()],
        rows: vec![
            vec![Cell::Null],
            vec![Cell::Integer(9007199254740993)],
            vec![Cell::Integer(1)],
            vec![Cell::Integer(1)],
        ],
        ..Default::default()
    };
    let stats = queri::statistics::ColumnStatistics::loaded(&data, 0, false)?;
    assert_eq!(stats.nulls, 1);
    assert_eq!(stats.distinct, Some(2));
    assert_eq!(stats.max, Some(Cell::Integer(9007199254740993)));
    assert!(
        queri::statistics::ColumnStatistics::loaded(&data, 0, true)?
            .distinct
            .is_none()
    );
    let mut edits = PendingEdits::default();
    let row = vec![Cell::Integer(1), Cell::Text("old".into())];
    edits.set(&row, 1, FieldValue::Null);
    assert_eq!(edits.rows.len(), 1);
    edits.set(&row, 1, FieldValue::Value(row[1].clone()));
    assert!(edits.rows.is_empty());
    edits.set(&row, 0, FieldValue::Value(Cell::Integer(5)));
    assert_eq!(edits.rows[0].original, row);
    edits.revert(&row);
    assert!(edits.rows.is_empty());
    let malformed = ResultSet {
        rows: vec![vec![Cell::Text("[]".into())]],
        ..Default::default()
    };
    assert!(QueryPlan::parse(&malformed, true).is_err());
    Ok(())
}
