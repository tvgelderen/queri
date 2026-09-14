use super::{Backend, Session};
use crate::{
    completion::{Catalog, Column, Relation},
    sql::quote_identifier,
};
use anyhow::{Result, bail};

#[derive(Default)]
pub(super) struct Cache {
    epoch: u64,
    loaded: bool,
    blocked: bool,
    catalog: Catalog,
}

impl Session {
    /// Invalidation retains the last snapshot for completion during user transactions.
    pub fn invalidate_completions(&self) {
        self.completion_epoch
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn completion_epoch(&self) -> u64 {
        self.completion_epoch
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Never waits behind user execution or another completion request.
    pub async fn completion_catalog(&self, wanted: Vec<(String, String)>) -> Catalog {
        let Ok(mut cache) = self.completion_cache.try_lock() else {
            return Catalog {
                status: "Completion metadata busy; retry Ctrl+Space".into(),
                ..Default::default()
            };
        };
        let epoch = self.completion_epoch();
        let refresh = !cache.loaded || cache.epoch != epoch;
        let missing: Vec<_> = wanted
            .into_iter()
            .filter(|(s, n)| {
                refresh
                    || cache
                        .catalog
                        .relations
                        .iter()
                        .any(|r| &r.schema == s && &r.name == n && r.columns.is_none())
            })
            .take(16)
            .collect();
        if !refresh && (cache.blocked || missing.is_empty()) {
            return cache.catalog.clone();
        }
        let result = match &self.backend {
            Backend::Postgres(pg) => match pg.client.try_lock() {
                Ok(client) => match client.transaction_status().await {
                    Ok(b'I') => postgres_catalog(&client, refresh, &missing).await,
                    Ok(_) => Err(anyhow::anyhow!(
                        "Using cached completion metadata; finish the transaction, then refresh completions"
                    )),
                    Err(error) => Err(anyhow::anyhow!(
                        "Completion connection unavailable: {error}"
                    )),
                },
                Err(_) => Err(anyhow::anyhow!(
                    "Using cached completion metadata; session busy"
                )),
            },
            Backend::Sqlite { connection, .. } => {
                let connection = connection.clone();
                tokio::task::spawn_blocking(move || {
                    let Ok(conn) = connection.try_lock() else { bail!("Using cached completion metadata; session busy") };
                    if !conn.is_autocommit() { bail!("Using cached completion metadata; finish the transaction, then refresh completions") }
                    sqlite_catalog(&conn, refresh, &missing)
                }).await.unwrap_or_else(|e| Err(e.into()))
            }
        };
        match result {
            Ok(mut catalog) => {
                if refresh {
                    cache.catalog = catalog;
                } else {
                    for relation in &mut cache.catalog.relations {
                        if let Some(found) = catalog
                            .relations
                            .iter_mut()
                            .find(|r| r.schema == relation.schema && r.name == relation.name)
                        {
                            relation.columns = found.columns.take();
                        }
                    }
                    cache.catalog.status = "Completion metadata ready".into();
                }
                cache.blocked = false;
                cache.loaded = true;
                cache.epoch = epoch;
            }
            Err(error) => {
                cache.catalog.status = format!("{error:#}. Refresh completions to retry.");
                // Retry only after explicit refresh or execution, not on every keystroke.
                cache.loaded = true;
                cache.epoch = epoch;
                cache.blocked = true;
            }
        }
        cache.catalog.clone()
    }
}

async fn postgres_catalog(
    client: &tokio_postgres::Client,
    names: bool,
    wanted: &[(String, String)],
) -> Result<Catalog> {
    let mut catalog = Catalog {
        status: "Completion metadata ready (up to 20,000 relations)".into(),
        ..Default::default()
    };
    if names {
        catalog.schemas = client.query("SELECT CASE WHEN oid=pg_my_temp_schema() THEN 'pg_temp' ELSE nspname END FROM pg_catalog.pg_namespace WHERE has_schema_privilege(oid, 'USAGE') AND (nspname !~ '^pg_' OR oid = pg_my_temp_schema()) ORDER BY nspname LIMIT 20000", &[]).await?.iter().map(|r| r.get(0)).collect();
        catalog.relations = client.query("SELECT CASE WHEN n.oid=pg_my_temp_schema() THEN 'pg_temp' ELSE n.nspname END, c.relname, pg_catalog.pg_table_is_visible(c.oid) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p','v','m','f') AND has_schema_privilege(n.oid, 'USAGE') AND has_any_column_privilege(c.oid, 'SELECT') AND (n.nspname !~ '^pg_' OR n.oid=pg_my_temp_schema()) AND n.nspname <> 'information_schema' ORDER BY n.nspname,c.relname LIMIT 20000", &[]).await?.iter().map(|r| Relation { schema:r.get(0), name:r.get(1), visible:r.get(2), columns:None }).collect();
    }
    for (schema, name) in wanted {
        let columns = client.query("SELECT a.attname, pg_catalog.format_type(a.atttypid,a.atttypmod), left(COALESCE(pg_catalog.col_description(c.oid,a.attnum),''),1024) FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE (n.nspname=$1 OR ($1='pg_temp' AND n.oid=pg_my_temp_schema())) AND c.relname=$2 AND a.attnum>0 AND NOT a.attisdropped AND has_schema_privilege(n.oid,'USAGE') AND has_column_privilege(c.oid,a.attnum,'SELECT') ORDER BY a.attnum LIMIT 2000", &[schema,name]).await?.iter().map(|r| Column {name:r.get(0), detail:format!("{} {}",r.get::<_,String>(1),r.get::<_,String>(2)).trim().into()}).collect();
        if let Some(r) = catalog
            .relations
            .iter_mut()
            .find(|r| &r.schema == schema && &r.name == name)
        {
            r.columns = Some(columns);
        } else {
            catalog.relations.push(Relation {
                schema: schema.clone(),
                name: name.clone(),
                visible: false,
                columns: Some(columns),
            });
        }
    }
    Ok(catalog)
}

fn sqlite_catalog(
    conn: &rusqlite::Connection,
    names: bool,
    wanted: &[(String, String)],
) -> Result<Catalog> {
    let mut catalog = Catalog {
        status: "Completion metadata ready (up to 20,000 relations)".into(),
        ..Default::default()
    };
    if names {
        catalog.schemas = conn
            .prepare("PRAGMA database_list")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        if !catalog.schemas.iter().any(|s| s == "temp") {
            catalog.schemas.push("temp".into());
        }
        // SQLite resolves temp, main, then attached databases in attachment order.
        let mut order = catalog.schemas.clone();
        order.sort_by_key(|s| match s.as_str() {
            "temp" => 0,
            "main" => 1,
            _ => 2,
        });
        let mut seen = std::collections::HashSet::new();
        for schema in order {
            let sql = format!(
                "SELECT name FROM {}.sqlite_schema WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name LIMIT {}",
                quote_identifier(&schema)?,
                20000usize.saturating_sub(catalog.relations.len())
            );
            for name in conn
                .prepare(&sql)?
                .query_map([], |r| r.get::<_, String>(0))?
            {
                let name = name?;
                let visible = seen.insert(name.to_lowercase());
                catalog.relations.push(Relation {
                    schema: schema.clone(),
                    name,
                    visible,
                    columns: None,
                });
            }
        }
    }
    for (schema, name) in wanted {
        let sql = format!(
            "PRAGMA {}.table_xinfo({})",
            quote_identifier(schema)?,
            quote_identifier(name)?
        );
        let columns = conn
            .prepare(&sql)?
            .query_map([], |r| {
                Ok((
                    r.get::<_, i32>(6)?,
                    Column {
                        name: r.get(1)?,
                        detail: r.get(2)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|(hidden, _)| *hidden != 1)
            .take(2000)
            .map(|(_, c)| c)
            .collect();
        if let Some(r) = catalog
            .relations
            .iter_mut()
            .find(|r| &r.schema == schema && &r.name == name)
        {
            r.columns = Some(columns);
        } else {
            catalog.relations.push(Relation {
                schema: schema.clone(),
                name: name.clone(),
                visible: false,
                columns: Some(columns),
            });
        }
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    #[test]
    fn sqlite_completion_does_not_wait_for_execution_lock() {
        super::super::runtime().unwrap().block_on(async {
            let profile = ConnectionProfile {
                ssh: None,
                id: uuid::Uuid::new_v4(),
                name: "completion".into(),
                notes: String::new(),
                color: String::new(),
                environment: None,
                policy: ExecutionPolicy::Normal,
                remember_password: false,
                config: ConnectionConfig::Sqlite {
                    path: ":memory:".into(),
                    read_only: false,
                    create: true,
                    wal: false,
                    foreign_keys: true,
                },
            };
            let session = Session::connect(profile, String::new()).await.unwrap();
            session
                .query("CREATE TABLE cached(id integer)".into())
                .await
                .unwrap();
            session.completion_catalog(vec![]).await;
            session.invalidate_completions();
            let Backend::Sqlite { connection, .. } = &session.backend else {
                unreachable!()
            };
            let connection = connection.clone();
            let (locked, ready) = tokio::sync::oneshot::channel();
            let (release, released) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                let _guard = connection.lock().unwrap();
                locked.send(()).unwrap();
                let _ = released.recv();
            });
            ready.await.unwrap();
            let snapshot = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                session.completion_catalog(vec![]),
            )
            .await
            .unwrap();
            assert!(snapshot.status.contains("busy"));
            assert!(snapshot.relations.iter().any(|r| r.name == "cached"));
            release.send(()).unwrap();
            worker.join().unwrap();
            session.invalidate_completions();
            assert!(
                session
                    .completion_catalog(vec![])
                    .await
                    .status
                    .contains("ready")
            );
        });
    }
}
