use super::*;
use crate::schema::{Column, Constraint, Engine, Index, Plan, Scope, Snapshot, Table};

mod postgres;
mod sqlite;
use sqlite::inspect;
impl Session {
    /// Call on a dedicated session. SQLite attached namespaces belong to that session.
    pub async fn schema_snapshot(&self, scope: Scope) -> Result<Snapshot> {
        match &self.backend {
            Backend::Sqlite { .. } => {
                self.with_sqlite(move |conn| {
                    anyhow::ensure!(
                        conn.is_autocommit(),
                        "Schema inspection requires an idle session"
                    );
                    let tx = conn.transaction()?;
                    let snapshot = inspect(&tx, scope)?;
                    tx.commit()?;
                    Ok(snapshot)
                })
                .await
            }
            Backend::Postgres(pg) => {
                let mut client = pg.client.lock().await;
                anyhow::ensure!(
                    client.transaction_status().await? == b'I',
                    "Schema inspection requires an idle session"
                );
                let tx = client
                    .build_transaction()
                    .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
                    .read_only(true)
                    .start()
                    .await?;
                let snapshot = postgres::inspect(&tx, scope).await?;
                tx.commit().await?;
                Ok(snapshot)
            }
        }
    }
    /// Applies an immutable plan atomically with an exclusive SQLite writer reservation.
    pub async fn apply_schema_plan(&self, plan: Plan) -> Result<()> {
        anyhow::ensure!(!self.profile.blocks_mutations(), "Connection is read-only");
        anyhow::ensure!(
            plan.blocked.is_empty(),
            "Schema plan contains blocked operations"
        );
        match &self.backend {
            Backend::Sqlite { .. } => {
                self.with_sqlite(move |conn| {
                    anyhow::ensure!(
                        conn.is_autocommit(),
                        "Schema application requires an idle dedicated session"
                    );
                    let foreign_keys: bool =
                        conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
                    conn.execute_batch("PRAGMA foreign_keys=OFF")?;
                    let result = (|| -> Result<()> {
                        let tx = conn
                            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                        anyhow::ensure!(
                            inspect(&tx, plan.baseline.scope.clone())? == plan.baseline,
                            "Schema baseline is stale"
                        );
                        for sql in &plan.sql {
                            tx.execute_batch(sql)?;
                        }
                        crate::schema::verify_expected(
                            &plan.expected,
                            &inspect(&tx, plan.baseline.scope.clone())?,
                        )?;
                        let violations: bool = tx
                            .prepare("PRAGMA main.foreign_key_check")?
                            .query([])?
                            .next()?
                            .is_some();
                        anyhow::ensure!(!violations, "Foreign key validation failed");
                        tx.commit()?;
                        Ok(())
                    })();
                    let restored = conn.execute_batch(if foreign_keys {
                        "PRAGMA foreign_keys=ON"
                    } else {
                        "PRAGMA foreign_keys=OFF"
                    });
                    restored?;
                    result?;
                    Ok(())
                })
                .await?
            }
            Backend::Postgres(pg) => {
                let mut client = pg.client.lock().await;
                anyhow::ensure!(
                    client.transaction_status().await? == b'I',
                    "Schema application requires an idle dedicated session"
                );
                let tx = client.transaction().await?;
                tx.batch_execute(
                    "SET LOCAL lock_timeout = '5s'; SET LOCAL statement_timeout = '30s'",
                )
                .await?;
                // Stable ordering prevents concurrent planners taking locks in opposite order.
                let mut names: Vec<_> = plan.baseline.tables.iter().filter(|table| {
                    plan.expected.tables.iter().find(|t| t.name == table.name) != Some(*table)
                    || plan.expected.tables.iter().any(|t| t.constraints.iter().any(|c| matches!(c, Constraint::ForeignKey { target, .. } if target == &table.name)))
                }).map(|t| &t.name).collect();
                names.sort();
                for name in names {
                    tx.batch_execute(&format!(
                        "LOCK TABLE {}.{} IN ACCESS EXCLUSIVE MODE",
                        quote_identifier(&plan.baseline.scope.namespace)?,
                        quote_identifier(name)?
                    ))
                    .await?;
                }
                anyhow::ensure!(
                    postgres::inspect(&tx, plan.baseline.scope.clone()).await? == plan.baseline,
                    "Schema baseline is stale"
                );
                for sql in &plan.sql {
                    tx.batch_execute(sql).await?;
                }
                crate::schema::verify_expected(
                    &plan.expected,
                    &postgres::inspect(&tx, plan.baseline.scope.clone()).await?,
                )?;
                tx.commit().await?;
            }
        }
        self.invalidate_completions();
        Ok(())
    }
}
