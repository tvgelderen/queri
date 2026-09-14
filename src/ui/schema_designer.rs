use super::{field, input};
use crate::schema::{Column, Constraint, Index, Snapshot, Table};
use gpui_kit::{
    component::{
        button::Button,
        h_flex,
        input::{InputEvent, InputState},
        scroll::ScrollableElement,
        v_flex,
    },
    *,
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SchemaEditIntent {
    #[serde(default)]
    pub allow_destructive: bool,
    pub baseline: Snapshot,
    pub desired: Snapshot,
    pub original_table: Option<String>,
    pub column_renames: Vec<(String, String)>,
}
pub enum SchemaDesignerEvent {
    Review(Box<SchemaEditIntent>),
    Cancel,
    Changed,
}
impl EventEmitter<SchemaDesignerEvent> for SchemaDesigner {}
struct ColumnFields {
    original: Column,
    name: Entity<InputState>,
    kind: Entity<InputState>,
    default: Entity<InputState>,
    nullable: bool,
    primary: bool,
}
struct ExtraFields {
    original_index: Option<Index>,
    kind: usize,
    fields: Vec<Entity<InputState>>,
}
pub struct SchemaDesigner {
    allow_destructive: bool,
    extras: Vec<ExtraFields>,
    baseline: Snapshot,
    original: Option<String>,
    table: Table,
    name: Entity<InputState>,
    columns: Vec<ColumnFields>,
}
impl SchemaDesigner {
    pub fn new(
        baseline: Snapshot,
        table: Option<Table>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let original = table.as_ref().map(|t| t.name.clone());
        let mut table = table.unwrap_or(Table {
            constraint_names: Vec::new(),
            name: String::new(),
            columns: vec![],
            definition: None,
            indexes: vec![],
            constraints: vec![],
            dependencies: vec![],
            options: vec![],
            unsupported: vec![],
        });
        if let Some(Constraint::Primary { columns }) = table
            .constraints
            .iter()
            .find(|c| matches!(c, Constraint::Primary { .. }))
        {
            for column in &mut table.columns {
                column.primary_key_position = columns
                    .iter()
                    .position(|name| name == &column.name)
                    .map_or(0, |i| i as i64 + 1);
            }
        }
        let name = draft_input(table.name.clone(), window, cx);
        let columns = table
            .columns
            .iter()
            .cloned()
            .map(|c| Self::column(c, window, cx))
            .collect();
        let mut extras = vec![];
        for c in &table.constraints {
            let (kind, values) = match c {
                Constraint::Primary { .. } => continue,
                Constraint::Unique { columns } => (1, vec![columns.join(",")]),
                Constraint::Check { expression } => (2, vec![expression.clone()]),
                Constraint::ForeignKey {
                    columns,
                    target,
                    referenced_columns,
                    on_update,
                    on_delete,
                    deferral,
                } => (
                    3,
                    vec![
                        columns.join(","),
                        target.clone(),
                        referenced_columns.join(","),
                        on_update.clone(),
                        on_delete.clone(),
                        deferral.clone().unwrap_or_default(),
                    ],
                ),
            };
            extras.push(ExtraFields {
                original_index: None,
                kind,
                fields: values
                    .into_iter()
                    .map(|v| draft_input(v, window, cx))
                    .collect(),
            });
        }
        for index in &table.indexes {
            extras.push(ExtraFields {
                original_index: Some(index.clone()),
                kind: if index.unique { 5 } else { 4 },
                fields: vec![
                    draft_input(index.name.clone(), window, cx),
                    draft_input(index.columns.join(","), window, cx),
                ],
            });
        }
        Self {
            allow_destructive: false,
            extras,
            baseline,
            original,
            table,
            name,
            columns,
        }
    }
    pub fn restore(
        intent: SchemaEditIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            intent.baseline.engine == intent.desired.engine
                && intent.baseline.scope == intent.desired.scope,
            "Draft engine or scope is invalid"
        );
        let index = match &intent.original_table {
            Some(name) => intent
                .baseline
                .tables
                .iter()
                .position(|t| &t.name == name)
                .ok_or_else(|| anyhow::anyhow!("Draft table is missing"))?,
            None => intent.baseline.tables.len(),
        };
        let table = intent
            .desired
            .tables
            .get(index)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Draft table is missing"))?;
        let mut remainder = intent.desired.clone();
        if intent.original_table.is_some() {
            remainder.tables[index] = intent.baseline.tables[index].clone();
        } else {
            remainder.tables.remove(index);
        }
        anyhow::ensure!(
            remainder == intent.baseline,
            "Draft contains unrelated changes"
        );
        let mut view = Self::new(intent.baseline, Some(table), window, cx);
        view.original = intent.original_table;
        // Destructive consent must be renewed after a restart.
        for column in &mut view.columns {
            if let Some((old, _)) = intent
                .column_renames
                .iter()
                .find(|(_, new)| new == &column.original.name)
            {
                column.original.name = old.clone();
            } else if !view
                .baseline
                .tables
                .iter()
                .find(|t| Some(&t.name) == view.original.as_ref())
                .is_some_and(|t| t.columns.iter().any(|c| c.name == column.original.name))
            {
                column.original.name.clear();
            }
        }
        Ok(view)
    }
    fn column(c: Column, window: &mut Window, cx: &mut Context<Self>) -> ColumnFields {
        ColumnFields {
            name: draft_input(c.name.clone(), window, cx),
            kind: draft_input(c.data_type.clone(), window, cx),
            default: draft_input(c.default.clone().unwrap_or_default(), window, cx),
            nullable: c.nullable,
            primary: c.primary_key_position > 0,
            original: c,
        }
    }
    pub fn intent(&self, cx: &App) -> SchemaEditIntent {
        let mut table = self.table.clone();
        table.name = self.name.read(cx).value().to_string();
        let mut renames = vec![];
        table.columns = self
            .columns
            .iter()
            .map(|f| {
                let mut c = f.original.clone();
                c.name = f.name.read(cx).value().to_string();
                if !f.original.name.is_empty() && c.name != f.original.name {
                    renames.push((f.original.name.clone(), c.name.clone()));
                }
                c.data_type = f.kind.read(cx).value().to_string();
                let default = f.default.read(cx).value().to_string();
                c.default = if default.is_empty() {
                    None
                } else {
                    Some(default)
                };
                c.nullable = f.nullable;
                c.primary_key_position = if f.primary {
                    if f.original.primary_key_position > 0 {
                        f.original.primary_key_position
                    } else {
                        i64::MAX
                    }
                } else {
                    0
                };
                c
            })
            .collect();
        table.constraints.clear();
        table.indexes.clear();
        for e in &self.extras {
            let values: Vec<String> = e
                .fields
                .iter()
                .map(|f| f.read(cx).value().to_string())
                .collect();
            let list = |i: usize| {
                values[i]
                    .split(',')
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
                    .collect()
            };
            match e.kind {
                0 => table
                    .constraints
                    .push(Constraint::Primary { columns: list(0) }),
                1 => table
                    .constraints
                    .push(Constraint::Unique { columns: list(0) }),
                2 => table.constraints.push(Constraint::Check {
                    expression: values[0].clone(),
                }),
                3 => table.constraints.push(Constraint::ForeignKey {
                    columns: list(0),
                    target: values[1].clone(),
                    referenced_columns: list(2),
                    on_update: values[3].clone(),
                    on_delete: values[4].clone(),
                    deferral: if values[5].is_empty() {
                        None
                    } else {
                        Some(values[5].clone())
                    },
                }),
                _ => {
                    let mut index = e.original_index.clone().unwrap_or(Index {
                        name: values[0].clone(),
                        columns: vec![],
                        unique: false,
                        definition: None,
                    });
                    index.name = values[0].clone();
                    index.columns = list(1);
                    index.unique = e.kind == 5;
                    table.indexes.push(index);
                }
            }
        }
        let primary = table
            .constraints
            .iter()
            .find_map(|c| match c {
                Constraint::Primary { columns } => Some(columns.clone()),
                _ => None,
            })
            .unwrap_or_else(|| {
                let mut columns: Vec<_> = table
                    .columns
                    .iter()
                    .filter(|c| c.primary_key_position > 0)
                    .collect();
                columns.sort_by_key(|c| c.primary_key_position);
                columns.into_iter().map(|c| c.name.clone()).collect()
            });
        table
            .constraints
            .retain(|c| !matches!(c, Constraint::Primary { .. }));
        if !primary.is_empty() {
            table.constraints.push(Constraint::Primary {
                columns: primary.clone(),
            });
        }
        for column in &mut table.columns {
            column.primary_key_position = primary
                .iter()
                .position(|name| name == &column.name)
                .map_or(0, |i| i as i64 + 1);
        }
        let mut desired = self.baseline.clone();
        if let Some(original) = &self.original {
            if let Some(t) = desired.tables.iter_mut().find(|t| &t.name == original) {
                *t = table;
            }
        } else {
            desired.tables.push(table);
        }
        SchemaEditIntent {
            allow_destructive: self.allow_destructive,
            baseline: self.baseline.clone(),
            desired,
            original_table: self.original.clone(),
            column_renames: renames,
        }
    }
}
impl Render for SchemaDesigner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("schema-designer")
            .size_full()
            .overflow_y_scrollbar()
            .gap_3()
            .p_4()
            .child("Table designer — changes remain local until reviewed")
            .child(div().flex_none().child(field("Table name", &self.name)))
            .children(self.columns.iter().enumerate().map(|(i, f)| {
                v_flex()
                    .gap_2()
                    .child(format!("Column {}", i + 1))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(div().flex_1().child(field("Column name", &f.name)))
                            .child(div().flex_1().child(field("Data type", &f.kind)))
                            .child(
                                div().flex_1().child(field(
                                    "Default expression (empty means none)",
                                    &f.default,
                                )),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new(("nullable", i))
                                    .label(if f.nullable {
                                        "NULL allowed"
                                    } else {
                                        "Value required"
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.columns[i].nullable ^= true;
                                        cx.emit(SchemaDesignerEvent::Changed);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(("primary", i))
                                    .label(if f.primary {
                                        "Primary key"
                                    } else {
                                        "Not a primary key"
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.columns[i].primary ^= true;
                                        cx.emit(SchemaDesignerEvent::Changed);
                                        cx.notify();
                                    })),
                            )
                            .child(Button::new(("remove", i)).label("Remove column").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.columns.remove(i);
                                    cx.emit(SchemaDesignerEvent::Changed);
                                    cx.notify();
                                }),
                            )),
                    )
            }))
            .child(
                Button::new("add-column")
                    .label("Add column")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.columns.push(Self::column(
                            Column {
                                name: String::new(),
                                data_type: "TEXT".into(),
                                nullable: true,
                                default: None,
                                primary_key_position: 0,
                                generated: false,
                                identity: None,
                                collation: None,
                            },
                            window,
                            cx,
                        ));
                        cx.emit(SchemaDesignerEvent::Changed);
                        cx.notify();
                    })),
            )
            .children(self.extras.iter().enumerate().map(|(i, e)| {
                let labels = extra_labels(e.kind);
                v_flex()
                    .gap_2()
                    .child(extra_title(e.kind))
                    .children(
                        e.fields
                            .iter()
                            .zip(labels)
                            .map(|(f, label)| field(*label, f)),
                    )
                    .child(
                        Button::new(("remove-extra", i))
                            .label("Remove constraint or index")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.extras.remove(i);
                                cx.emit(SchemaDesignerEvent::Changed);
                                cx.notify();
                            })),
                    )
            }))
            .child(h_flex().flex_wrap().gap_2().children((0..6).map(|kind| {
                Button::new(("add-extra", kind))
                    .label(format!("Add {}", extra_title(kind)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.extras.push(ExtraFields {
                            original_index: None,
                            kind,
                            fields: extra_labels(kind)
                                .iter()
                                .map(|_| draft_input("", window, cx))
                                .collect(),
                        });
                        cx.emit(SchemaDesignerEvent::Changed);
                        cx.notify();
                    }))
            })))
            .child(
                Button::new("reset-schema")
                    .label("Reset draft")
                    .on_click(cx.listener(|this, _, window, cx| {
                        *this = Self::new(
                            this.baseline.clone(),
                            this.original
                                .as_ref()
                                .and_then(|n| this.baseline.tables.iter().find(|t| &t.name == n))
                                .cloned(),
                            window,
                            cx,
                        );
                        cx.emit(SchemaDesignerEvent::Changed);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("allow-destructive-designer")
                    .label(if self.allow_destructive {
                        "Destructive changes enabled"
                    } else {
                        "Allow destructive changes"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.allow_destructive ^= true;
                        cx.emit(SchemaDesignerEvent::Changed);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("review-schema")
                    .label("Review changes")
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.emit(SchemaDesignerEvent::Review(Box::new(this.intent(cx))));
                    })),
            )
    }
}

fn extra_title(kind: usize) -> &'static str {
    [
        "primary key",
        "unique constraint",
        "check constraint",
        "foreign key",
        "index",
        "unique index",
    ][kind]
}
fn extra_labels(kind: usize) -> &'static [&'static str] {
    match kind {
        0 | 1 => &["Columns (comma separated)"],
        2 => &["Check expression"],
        3 => &[
            "Columns (comma separated)",
            "Target table",
            "Referenced columns (comma separated)",
            "On update",
            "On delete",
            "Deferral (optional)",
        ],
        _ => &["Index name", "Columns (comma separated)"],
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::SchemaDesigner;
    use crate::schema::Snapshot;
    use gpui_kit::{AppContext, TestAppContext, WindowOptions};
    #[gpui_kit::test]
    fn draft_preserves_snapshot_and_exposes_rename(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let baseline = Snapshot {
                    engine: crate::schema::Engine::Sqlite,
                    engine_version: "3".into(),
                    scope: crate::schema::Scope {
                        namespace: "main".into(),
                    },
                    tables: vec![],
                };
                let view = cx.new(|cx| SchemaDesigner::new(baseline.clone(), None, window, cx));
                view.update(cx, |view, cx| {
                    view.name
                        .update(cx, |name, cx| name.set_value("new_table", window, cx));
                    let intent = view.intent(cx);
                    assert_eq!(intent.baseline, baseline);
                    assert!(baseline.tables.is_empty());
                    assert_eq!(intent.desired.tables[0].name, "new_table");
                    assert!(intent.column_renames.is_empty());
                    let directory = tempfile::tempdir().unwrap();
                    let path = directory.path().join("drafts.db");
                    {
                        let store = crate::storage::Store::open(&path).unwrap();
                        store
                            .save_schema_draft(
                                "context",
                                Some(&serde_json::to_string(&intent).unwrap()),
                            )
                            .unwrap();
                    }
                    let store = crate::storage::Store::open(&path).unwrap();
                    let saved =
                        serde_json::from_str(&store.schema_draft("context").unwrap().unwrap())
                            .unwrap();
                    let restored = SchemaDesigner::restore(saved, window, cx).unwrap();
                    assert_eq!(restored.intent(cx).desired, intent.desired);
                    assert_eq!(restored.intent(cx).baseline, baseline);
                    assert!(!restored.intent(cx).allow_destructive);
                    store.save_schema_draft("context", None).unwrap();
                    assert!(store.schema_draft("context").unwrap().is_none());
                    let mut invalid = intent;
                    invalid
                        .desired
                        .tables
                        .push(invalid.desired.tables[0].clone());
                    assert!(SchemaDesigner::restore(invalid, window, cx).is_err());
                    use crate::{database::Session, model::*};
                    let runtime = crate::database::runtime().unwrap();
                    let database = directory.path().join("designer.db");
                    let session = runtime.block_on(Session::connect(ConnectionProfile {
                        id: uuid::Uuid::new_v4(), name: "Designer test".into(), notes: String::new(), color: String::new(), ssh: None, environment: None,
                        policy: ExecutionPolicy::Normal, remember_password: false,
                        config: ConnectionConfig::Sqlite { path: database.clone(), read_only: false, create: true, wal: false, foreign_keys: true },
                    }, String::new())).unwrap();
                    view.baseline = runtime.block_on(session.schema_snapshot(baseline.scope)).unwrap();
                    for name in ["description", "id"] {
                        view.columns.push(SchemaDesigner::column(crate::schema::Column {
                            name: name.into(), data_type: "INTEGER".into(), nullable: true, default: None, primary_key_position: 0, generated: false, identity: None, collation: None,
                        }, window, cx));
                    }
                    view.columns[1].primary = true;
                    view.extras.push(super::ExtraFields {
                        original_index: None, kind: 4,
                        fields: vec![super::draft_input("lookup", window, cx), super::draft_input("description", window, cx)],
                    });
                    view.columns[0].primary = true;
                    view.columns[0].original.primary_key_position = 2;
                    view.columns[1].original.primary_key_position = 1;
                    let composite = view.intent(cx);
                    assert_eq!(composite.desired.tables[0].columns[0].primary_key_position, 2);
                    assert_eq!(composite.desired.tables[0].columns[1].primary_key_position, 1);
                    view.columns[0].primary = false;
                    let intent = view.intent(cx);
                    assert_eq!(intent.desired.tables[0].columns[1].primary_key_position, 1);
                    let plan = crate::schema::plan(&intent.baseline, &intent.desired).unwrap();
                    assert!(plan.blocked.is_empty(), "{:?}", plan.blocked);
                    runtime.block_on(session.apply_schema_plan(plan)).unwrap();
                    let db = rusqlite::Connection::open(database).unwrap();
                    db.execute("INSERT INTO new_table(id) VALUES(1)", []).unwrap();
                    assert!(db.execute("INSERT INTO new_table(id) VALUES(1)", []).is_err());
                    assert_eq!(db.query_row("SELECT count(*) FROM sqlite_schema WHERE type='index' AND name='lookup'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);

                });
                view
            })
            .unwrap();
        });
    }
}

fn draft_input(
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<SchemaDesigner>,
) -> Entity<InputState> {
    let entity = input(value, window, cx);
    cx.subscribe(&entity, |_, _, event, cx| {
        if matches!(event, InputEvent::Change) {
            cx.emit(SchemaDesignerEvent::Changed);
        }
    })
    .detach();
    entity
}
