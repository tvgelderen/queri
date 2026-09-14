use super::*;
use crate::sql::quote_identifier as quote;
use anyhow::{Result, ensure};

pub(super) fn table(
    snapshot: &Snapshot,
    before: &Table,
    after: &Table,
    options: &PlanOptions,
) -> Result<Vec<String>> {
    ensure!(
        snapshot.scope.namespace == "main",
        "Rebuild currently requires the main database"
    );
    ensure!(
        before.unsupported.is_empty() && after.unsupported.is_empty(),
        "Rebuild contains unsupported metadata"
    );
    ensure!(
        before.options.is_empty() && after.options.is_empty(),
        "Rebuild does not yet preserve table options (STRICT/WITHOUT ROWID)"
    );
    ensure!(
        before.dependencies == after.dependencies,
        "Changing triggers during rebuild is unsupported"
    );
    ensure!(
        !snapshot
            .tables
            .iter()
            .any(|t| t.unsupported.iter().any(|s| s.contains("view"))),
        "Rebuild requires view dependency coverage"
    );
    ensure!(
        options
            .column_renames
            .get(&before.name)
            .is_none_or(|r| r.is_empty()),
        "Rebuild with renames requires dependency rewriting"
    );
    for column in &before.columns {
        match after.columns.iter().find(|c| c.name == column.name) {
            Some(c) => ensure!(
                c.data_type == column.data_type || options.allow_destructive,
                "Type changes require destructive opt-in"
            ),
            None => ensure!(
                options.allow_destructive
                    && before.dependencies.is_empty()
                    && before.indexes.is_empty(),
                "Dropping columns requires destructive opt-in and complete dependency coverage"
            ),
        }
    }
    if let Some(ddl) = &before.definition {
        let upper = ddl.to_uppercase();
        ensure!(
            !upper.contains("ON CONFLICT")
                && !upper.contains(" DESC")
                && !upper.contains("COLLATE"),
            "Rebuild cannot preserve conflict policies, descending keys or collations"
        );
    }
    let temp = format!("__queri_rebuild_{}", before.name);
    ensure!(
        !snapshot
            .tables
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(&temp)),
        "Rebuild temporary table name already exists"
    );
    let mut target = after.clone();
    target.name = temp.clone();
    target.dependencies.clear();
    target.indexes.clear();
    let available = snapshot.tables.iter().collect::<Vec<_>>();
    let (mut sql, _) = create::table_sql(snapshot, &target, &available)?;
    // Copy the hidden rowid as well: applications can retain references to it.
    let rowid = ["rowid", "_rowid_", "oid"]
        .into_iter()
        .find(|alias| {
            !before
                .columns
                .iter()
                .chain(&after.columns)
                .any(|c| c.name.eq_ignore_ascii_case(alias))
        })
        .ok_or_else(|| anyhow::anyhow!("All hidden rowid aliases are shadowed"))?;
    let mut mapping = vec![quote(rowid)?];
    mapping.extend(
        after
            .columns
            .iter()
            .filter(|c| before.columns.iter().any(|b| b.name == c.name))
            .map(|c| quote(&c.name))
            .collect::<Result<Vec<_>>>()?,
    );
    let mapping = mapping.join(", ");
    sql.push(format!(
        "INSERT INTO {} ({mapping}) SELECT {mapping} FROM {}",
        quote(&temp)?,
        quote(&before.name)?
    ));
    sql.push(format!("DROP TABLE {}", quote(&before.name)?));
    sql.push(format!(
        "ALTER TABLE {} RENAME TO {}",
        quote(&temp)?,
        quote(&before.name)?
    ));
    for index in &after.indexes {
        if index.name.starts_with("automatic:") {
            continue;
        }
        if let Some(ddl) = &index.definition {
            ensure!(
                before.indexes.contains(index),
                "Modified raw index definitions are unsupported"
            );
            sql.push(ddl.clone());
        } else {
            let mut indexed = after.clone();
            indexed.dependencies.clear();
            indexed.indexes = vec![index.clone()];
            let (ddl, _) = create::table_sql(snapshot, &indexed, &available)?;
            sql.extend(ddl.into_iter().skip(1));
        }
    }
    sql.extend(before.dependencies.clone());
    Ok(sql)
}
