use super::*;

pub(super) async fn inspect(
    tx: &tokio_postgres::Transaction<'_>,
    scope: Scope,
) -> Result<Snapshot> {
    let exists = tx
        .query_one(
            "SELECT EXISTS (SELECT FROM pg_catalog.pg_namespace WHERE nspname=$1)",
            &[&scope.namespace],
        )
        .await?
        .get::<_, bool>(0);
    anyhow::ensure!(exists, "Schema does not exist");
    let rows = tx.query("SELECT c.oid,c.relname,c.relkind::text,has_schema_privilege(n.oid,'USAGE') AND has_table_privilege(c.oid,'SELECT'),c.reloptions FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind IN ('r','p','v','m','f') ORDER BY c.relname", &[&scope.namespace]).await?;
    let mut tables = Vec::new();
    for row in rows {
        let oid: u32 = row.get(0);
        let name: String = row.get(1);
        let kind: String = row.get(2);
        let mut unsupported = Vec::new();
        if kind != "r" {
            unsupported.push(format!("Unsupported object kind: {kind}"));
        }
        if !row.get::<_, bool>(3) {
            unsupported.push("Insufficient schema or table privileges".into());
        }
        let columns = tx.query("SELECT a.attname,pg_catalog.format_type(a.atttypid,a.atttypmod),NOT a.attnotnull,pg_catalog.pg_get_expr(d.adbin,d.adrelid),a.attgenerated::text,a.attidentity::text,(SELECT n.nspname || '.' || c.collname FROM pg_catalog.pg_collation c JOIN pg_catalog.pg_namespace n ON n.oid=c.collnamespace WHERE c.oid=a.attcollation AND a.attcollation<>(SELECT typcollation FROM pg_catalog.pg_type WHERE oid=a.atttypid)) FROM pg_catalog.pg_attribute a LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE a.attrelid=$1 AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum", &[&oid]).await?.iter().map(|r| {
            if !r.get::<_, String>(5).is_empty() { unsupported.push("Identity sequence options are not yet inspected".into()); }
            Column {name:r.get(0),data_type:r.get(1),nullable:r.get(2),default:r.get(3),primary_key_position:0,generated:!r.get::<_, String>(4).is_empty(),identity:Some(r.get(5)).filter(|s: &String| !s.is_empty()),collation:r.get(6)}
        }).collect();
        let indexes = tx.query("SELECT CASE WHEN EXISTS(SELECT FROM pg_catalog.pg_constraint c WHERE c.conindid=i.indexrelid AND c.conrelid=i.indrelid) THEN 'automatic:' || ci.relname ELSE ci.relname END,i.indisunique,pg_catalog.pg_get_indexdef(i.indexrelid), ARRAY(SELECT a.attname::text FROM unnest(i.indkey) WITH ORDINALITY k(num,ord) JOIN pg_catalog.pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.num ORDER BY k.ord), NOT COALESCE((to_jsonb(i)->>'indnullsnotdistinct')::boolean,false) AND i.indexprs IS NULL AND i.indpred IS NULL AND i.indnatts=i.indnkeyatts AND am.amname='btree' AND NOT EXISTS(SELECT FROM unnest(i.indoption) o WHERE o<>0) AND NOT EXISTS(SELECT FROM unnest(i.indclass) cl JOIN pg_catalog.pg_opclass op ON op.oid=cl WHERE NOT op.opcdefault) AND NOT EXISTS(SELECT FROM unnest(i.indkey,i.indcollation) k(num,coll) JOIN pg_catalog.pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.num WHERE k.coll<>a.attcollation) FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ci ON ci.oid=i.indexrelid JOIN pg_catalog.pg_am am ON am.oid=ci.relam WHERE i.indrelid=$1 ORDER BY ci.relname", &[&oid]).await?.iter().map(|r| Index { name:r.get(0), unique:r.get(1),columns:r.get(3),definition:if r.get::<_,bool>(4) {None} else {Some(r.get(2))} }).collect();
        if tx.query_one("SELECT EXISTS(SELECT FROM pg_catalog.pg_index i WHERE i.indrelid=$1 AND COALESCE((to_jsonb(i)->>'indnullsnotdistinct')::boolean,false))", &[&oid]).await?.get::<_,bool>(0) {
            unsupported.push("NULLS NOT DISTINCT indexes require further coverage".into());
        }
        let mut constraints = Vec::new();
        let mut constraint_names = Vec::new();
        for r in tx.query("SELECT c.contype::text, ARRAY(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY k(num,ord) JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.num ORDER BY k.ord), COALESCE(n.nspname || '.' || t.relname,''), ARRAY(SELECT a.attname::text FROM unnest(c.confkey) WITH ORDINALITY k(num,ord) JOIN pg_catalog.pg_attribute a ON a.attrelid=c.confrelid AND a.attnum=k.num ORDER BY k.ord),c.confupdtype::text,c.confdeltype::text,c.condeferrable,c.condeferred,pg_catalog.pg_get_expr(c.conbin,c.conrelid),c.conname,c.confmatchtype::text,c.convalidated,c.connoinherit,COALESCE((to_jsonb(c)->>'confdelsetcols')::text,'null') FROM pg_catalog.pg_constraint c LEFT JOIN pg_catalog.pg_class t ON t.oid=c.confrelid LEFT JOIN pg_catalog.pg_namespace n ON n.oid=t.relnamespace WHERE c.conrelid=$1 AND c.contype<>'n' ORDER BY c.conname", &[&oid]).await? {
            let kind: String = r.get(0);
            if ((kind == "p" || kind == "u") && r.get::<_, bool>(6))
                || (kind == "f" && r.get::<_, String>(10) != "s")
                || !r.get::<_, bool>(11) || (kind == "c" && r.get::<_, bool>(12))
                || r.get::<_, String>(13) != "null"
            {
                unsupported.push(format!("Constraint {} has unsupported deferral, match, validation, inheritance or column-specific action semantics", r.get::<_, String>(9)));
            }
            constraint_names.push(r.get::<_,String>(9));
            let columns: Vec<String> = r.get(1);
            match r.get::<_, String>(0).as_str() {
                "p" => constraints.push(Constraint::Primary {columns}),
                "u" => constraints.push(Constraint::Unique {columns}),
                "c" => constraints.push(Constraint::Check {expression:r.get(8)}),
                "f" => constraints.push(Constraint::ForeignKey {columns,target:r.get(2),referenced_columns:r.get(3),on_update:action(r.get(4)),on_delete:action(r.get(5)),deferral:Some(if r.get::<_, bool>(6) {if r.get::<_, bool>(7) {"INITIALLY DEFERRED"} else {"INITIALLY IMMEDIATE"}} else {"NOT DEFERRABLE"}.into())}),
                other => unsupported.push(format!("Unsupported constraint kind: {other}")),
            }
        }
        let options: Vec<String> = row.get::<_, Option<Vec<String>>>(4).unwrap_or_default();
        let mut dependencies: Vec<String> = tx.query("SELECT pg_catalog.pg_get_triggerdef(oid,true) FROM pg_catalog.pg_trigger WHERE tgrelid=$1 AND NOT tgisinternal ORDER BY tgname", &[&oid]).await?.iter().map(|r| r.get(0)).collect();
        for dependency in tx.query("SELECT 'foreign key: ' || conname FROM pg_catalog.pg_constraint WHERE confrelid=$1 AND conrelid<>$1 UNION SELECT 'view: ' || n.nspname || '.' || v.relname FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_rewrite r ON r.oid=d.objid JOIN pg_catalog.pg_class v ON v.oid=r.ev_class JOIN pg_catalog.pg_namespace n ON n.oid=v.relnamespace WHERE d.refobjid=$1 AND v.oid<>$1 ORDER BY 1", &[&oid]).await? {
            dependencies.push(dependency.get(0));
        }
        if tx.query_one("SELECT EXISTS(SELECT FROM pg_catalog.pg_attribute WHERE attrelid=$1 AND attcollation<>(SELECT typcollation FROM pg_catalog.pg_type WHERE oid=atttypid)) OR EXISTS(SELECT FROM pg_catalog.pg_class WHERE oid=$1 AND (relrowsecurity OR relispartition))", &[&oid]).await?.get::<_, bool>(0) {
            unsupported.push("Collation, row security or partition metadata requires further coverage".into());
        }
        tables.push(Table {
            constraint_names,
            name,
            columns,
            indexes,
            constraints,
            dependencies,
            options,
            definition: None,
            unsupported,
        });
    }
    Ok(Snapshot {
        engine: Engine::Postgres,
        engine_version: tx
            .query_one("SELECT current_setting('server_version')", &[])
            .await?
            .get(0),
        scope,
        tables,
    })
}

fn action(code: &str) -> String {
    match code {
        "a" => "NO ACTION",
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        other => other,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn catalog_inspection() {
        let Ok(url) = std::env::var("QUERI_SCHEMA_TEST_POSTGRES") else {
            return;
        };
        let (mut client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .unwrap();
        tokio::spawn(async move { connection.await.unwrap() });
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("CREATE SCHEMA schema_inspection_test; CREATE TABLE schema_inspection_test.parent(a int,b int,PRIMARY KEY(a,b)); CREATE TABLE schema_inspection_test.child(x int DEFAULT 4,y int CHECK(y>0),UNIQUE(y,x),FOREIGN KEY(x,y) REFERENCES schema_inspection_test.parent(a,b) DEFERRABLE INITIALLY DEFERRED);").await.unwrap();
        let snapshot = inspect(
            &tx,
            Scope {
                namespace: "schema_inspection_test".into(),
            },
        )
        .await
        .unwrap();
        let child = &snapshot.tables[0];
        assert!(child.unsupported.is_empty(), "{:?}", child.unsupported);
        assert!(child.constraints.iter().any(|c| matches!(c,Constraint::ForeignKey {columns,referenced_columns,..} if columns==&["x","y"] && referenced_columns==&["a","b"])));
        tx.batch_execute("CREATE TABLE schema_inspection_test.special(id int UNIQUE NULLS NOT DISTINCT, value int UNIQUE DEFERRABLE INITIALLY DEFERRED, parent_a int, parent_b int, FOREIGN KEY(parent_a,parent_b) REFERENCES schema_inspection_test.parent(a,b) MATCH FULL NOT VALID)").await.unwrap();
        let special = inspect(
            &tx,
            Scope {
                namespace: "schema_inspection_test".into(),
            },
        )
        .await
        .unwrap();
        let table = special.tables.iter().find(|t| t.name == "special").unwrap();
        assert!(
            table
                .unsupported
                .iter()
                .any(|s| s.contains("NULLS NOT DISTINCT"))
        );
        assert!(
            table
                .unsupported
                .iter()
                .any(|s| s.contains("unsupported deferral"))
        );
        let mut empty = special.clone();
        empty.tables.clear();
        let plan = crate::schema::plan(&empty, &special).unwrap();
        assert!(!plan.blocked.is_empty());
        assert!(plan.sql().is_empty());
        tx.rollback().await.unwrap();
    }
}
