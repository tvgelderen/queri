use super::controls::{badge, card, count_badge, icon_button, page_header, setting_switch};
use super::theme::accent_text;
use super::{field, input};
use crate::schema::{Column, Constraint, Index, Snapshot, Table};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Sizable,
        button::{Button, ButtonVariants},
        checkbox::Checkbox,
        h_flex,
        input::{Input, InputEvent, InputState},
        menu::{DropdownMenu, PopupMenuItem},
        scroll::ScrollableElement,
        v_flex,
    },
    prelude::FluentBuilder,
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
    SaveAndClose,
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
#[derive(Clone, Copy)]
enum ExtraKind {
    PrimaryKey,
    Unique,
    Check,
    ForeignKey,
    Index,
    UniqueIndex,
}
impl ExtraKind {
    const ALL: [Self; 6] = [
        Self::PrimaryKey,
        Self::Unique,
        Self::Check,
        Self::ForeignKey,
        Self::Index,
        Self::UniqueIndex,
    ];
    fn title(self) -> &'static str {
        match self {
            Self::PrimaryKey => "primary key",
            Self::Unique => "unique constraint",
            Self::Check => "check constraint",
            Self::ForeignKey => "foreign key",
            Self::Index => "index",
            Self::UniqueIndex => "unique index",
        }
    }
}
const COLUMNS_LABEL: &str = "Columns (comma separated)";
/// An editable constraint or index; each variant owns exactly the inputs its kind needs.
enum ExtraFields {
    PrimaryKey {
        columns: Entity<InputState>,
    },
    Unique {
        columns: Entity<InputState>,
    },
    Check {
        expression: Entity<InputState>,
    },
    ForeignKey {
        columns: Entity<InputState>,
        target: Entity<InputState>,
        referenced_columns: Entity<InputState>,
        on_update: Entity<InputState>,
        on_delete: Entity<InputState>,
        deferral: Entity<InputState>,
    },
    Index {
        /// Preserves properties the designer does not edit, such as the definition.
        original: Option<Index>,
        unique: bool,
        name: Entity<InputState>,
        columns: Entity<InputState>,
    },
}
impl ExtraFields {
    fn blank(kind: ExtraKind, window: &mut Window, cx: &mut Context<SchemaDesigner>) -> Self {
        let mut empty = || draft_input("", window, cx);
        match kind {
            ExtraKind::PrimaryKey => Self::PrimaryKey { columns: empty() },
            ExtraKind::Unique => Self::Unique { columns: empty() },
            ExtraKind::Check => Self::Check {
                expression: empty(),
            },
            ExtraKind::ForeignKey => Self::ForeignKey {
                columns: empty(),
                target: empty(),
                referenced_columns: empty(),
                on_update: empty(),
                on_delete: empty(),
                deferral: empty(),
            },
            ExtraKind::Index | ExtraKind::UniqueIndex => Self::Index {
                original: None,
                unique: matches!(kind, ExtraKind::UniqueIndex),
                name: empty(),
                columns: empty(),
            },
        }
    }
    /// Primary keys are edited through column toggles, so they have no extra fields.
    fn from_constraint(
        constraint: &Constraint,
        window: &mut Window,
        cx: &mut Context<SchemaDesigner>,
    ) -> Option<Self> {
        let mut input = |value: String| draft_input(value, window, cx);
        Some(match constraint {
            Constraint::Primary { .. } => return None,
            Constraint::Unique { columns } => Self::Unique {
                columns: input(columns.join(",")),
            },
            Constraint::Check { expression } => Self::Check {
                expression: input(expression.clone()),
            },
            Constraint::ForeignKey {
                columns,
                target,
                referenced_columns,
                on_update,
                on_delete,
                deferral,
            } => Self::ForeignKey {
                columns: input(columns.join(",")),
                target: input(target.clone()),
                referenced_columns: input(referenced_columns.join(",")),
                on_update: input(on_update.clone()),
                on_delete: input(on_delete.clone()),
                deferral: input(deferral.clone().unwrap_or_default()),
            },
        })
    }
    fn from_index(index: &Index, window: &mut Window, cx: &mut Context<SchemaDesigner>) -> Self {
        Self::Index {
            original: Some(index.clone()),
            unique: index.unique,
            name: draft_input(index.name.clone(), window, cx),
            columns: draft_input(index.columns.join(","), window, cx),
        }
    }
    fn kind(&self) -> ExtraKind {
        match self {
            Self::PrimaryKey { .. } => ExtraKind::PrimaryKey,
            Self::Unique { .. } => ExtraKind::Unique,
            Self::Check { .. } => ExtraKind::Check,
            Self::ForeignKey { .. } => ExtraKind::ForeignKey,
            Self::Index { unique: false, .. } => ExtraKind::Index,
            Self::Index { unique: true, .. } => ExtraKind::UniqueIndex,
        }
    }
    fn inputs(&self) -> Vec<(&'static str, &Entity<InputState>)> {
        match self {
            Self::PrimaryKey { columns } | Self::Unique { columns } => {
                vec![(COLUMNS_LABEL, columns)]
            }
            Self::Check { expression } => vec![("Check expression", expression)],
            Self::ForeignKey {
                columns,
                target,
                referenced_columns,
                on_update,
                on_delete,
                deferral,
            } => vec![
                (COLUMNS_LABEL, columns),
                ("Target table", target),
                ("Referenced columns (comma separated)", referenced_columns),
                ("On update", on_update),
                ("On delete", on_delete),
                ("Deferral (optional)", deferral),
            ],
            Self::Index { name, columns, .. } => {
                vec![("Index name", name), (COLUMNS_LABEL, columns)]
            }
        }
    }
    fn apply(&self, table: &mut Table, cx: &App) {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let list = |input: &Entity<InputState>| {
            text(input)
                .split(',')
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
                .collect()
        };
        match self {
            Self::PrimaryKey { columns } => table.constraints.push(Constraint::Primary {
                columns: list(columns),
            }),
            Self::Unique { columns } => table.constraints.push(Constraint::Unique {
                columns: list(columns),
            }),
            Self::Check { expression } => table.constraints.push(Constraint::Check {
                expression: text(expression),
            }),
            Self::ForeignKey {
                columns,
                target,
                referenced_columns,
                on_update,
                on_delete,
                deferral,
            } => table.constraints.push(Constraint::ForeignKey {
                columns: list(columns),
                target: text(target),
                referenced_columns: list(referenced_columns),
                on_update: text(on_update),
                on_delete: text(on_delete),
                deferral: Some(text(deferral)).filter(|value| !value.is_empty()),
            }),
            Self::Index {
                original,
                unique,
                name,
                columns,
            } => {
                let mut index = original.clone().unwrap_or(Index {
                    name: String::new(),
                    columns: vec![],
                    unique: false,
                    definition: None,
                });
                index.name = text(name);
                index.columns = list(columns);
                index.unique = *unique;
                table.indexes.push(index);
            }
        }
    }
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
        let mut extras: Vec<_> = table
            .constraints
            .iter()
            .filter_map(|c| ExtraFields::from_constraint(c, window, cx))
            .collect();
        for index in &table.indexes {
            extras.push(ExtraFields::from_index(index, window, cx));
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
        for extra in &self.extras {
            extra.apply(&mut table, cx);
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
impl SchemaDesigner {
    fn blank_column() -> Column {
        Column {
            name: String::new(),
            data_type: "TEXT".into(),
            nullable: true,
            default: None,
            primary_key_position: 0,
            generated: false,
            identity: None,
            collation: None,
        }
    }
    fn columns_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = cx.theme().muted_foreground;
        let head = |label: &'static str| {
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(muted)
                .child(label)
        };
        let mut table = card(cx).overflow_hidden().child(
            h_flex()
                .h_8()
                .px_3()
                .gap_3()
                .bg(cx.theme().muted)
                .border_b_1()
                .border_color(cx.theme().border)
                .child(div().flex_1().child(head("Name")))
                .child(div().w(px(180.)).flex_shrink_0().child(head("Type")))
                .child(div().flex_1().child(head("Default")))
                .child(div().w(px(64.)).flex().justify_center().child(head("Null")))
                .child(
                    div()
                        .w(px(64.))
                        .flex()
                        .justify_center()
                        .child(head("Primary")),
                )
                .child(div().w_7()),
        );
        if self.columns.is_empty() {
            table = table.child(
                div()
                    .px_3()
                    .py_6()
                    .text_sm()
                    .text_center()
                    .text_color(muted)
                    .child("No columns yet. Add one to get started."),
            );
        }
        for (i, f) in self.columns.iter().enumerate() {
            let number = i + 1;
            table = table.child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .when(i > 0, |row| {
                        row.border_t_1().border_color(cx.theme().border)
                    })
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&f.name)
                                .small()
                                .aria_label(format!("Column {number} name")),
                        ),
                    )
                    .child(
                        div().w(px(180.)).flex_shrink_0().min_w_0().child(
                            Input::new(&f.kind)
                                .small()
                                .font_family(cx.theme().mono_font_family.clone())
                                .aria_label(format!("Column {number} data type")),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&f.default)
                                .small()
                                .font_family(cx.theme().mono_font_family.clone())
                                .aria_label(format!(
                                    "Column {number} default expression; empty means none"
                                )),
                        ),
                    )
                    .child(
                        div().w(px(64.)).flex().justify_center().child(
                            Checkbox::new(("nullable", i))
                                .accessibility_label(format!("Column {number} allows NULL"))
                                .checked(f.nullable)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.columns[i].nullable ^= true;
                                    cx.emit(SchemaDesignerEvent::Changed);
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(
                        div().w(px(64.)).flex().justify_center().child(
                            Checkbox::new(("primary", i))
                                .accessibility_label(format!(
                                    "Column {number} is in the primary key"
                                ))
                                .checked(f.primary)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.columns[i].primary ^= true;
                                    cx.emit(SchemaDesignerEvent::Changed);
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(
                        icon_button(
                            ("remove", i),
                            IconName::Trash,
                            format!("Remove column {number}"),
                        )
                        .small()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.columns.remove(i);
                            cx.emit(SchemaDesignerEvent::Changed);
                            cx.notify();
                        })),
                    ),
            );
        }
        v_flex()
            .gap_3()
            .child(section_header(
                "Columns",
                self.columns.len(),
                Button::new("add-column")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .label("Add column")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.columns
                            .push(Self::column(Self::blank_column(), window, cx));
                        cx.emit(SchemaDesignerEvent::Changed);
                        cx.notify();
                    }))
                    .into_any_element(),
                cx,
            ))
            .child(table)
    }
    fn extras_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let add = Button::new("add-extra")
            .ghost()
            .small()
            .icon(IconName::Plus)
            .label("Add")
            .dropdown_caret(true)
            .dropdown_menu({
                let view = cx.entity().downgrade();
                move |mut menu, _, _| {
                    for kind in ExtraKind::ALL {
                        let view = view.clone();
                        menu = menu.item(PopupMenuItem::new(capitalize(kind.title())).on_click(
                            move |_, window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.extras.push(ExtraFields::blank(kind, window, cx));
                                    cx.emit(SchemaDesignerEvent::Changed);
                                    cx.notify();
                                });
                            },
                        ));
                    }
                    menu
                }
            })
            .into_any_element();
        let mut list = v_flex().gap_3();
        if self.extras.is_empty() {
            list = list.child(
                card(cx)
                    .px_3()
                    .py_6()
                    .items_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No constraints or indexes. Primary keys are set per column."),
            );
        }
        for (i, extra) in self.extras.iter().enumerate() {
            let inputs = extra.inputs();
            let columns = if inputs.len() > 2 { 3 } else { 2 };
            list =
                list.child(
                    card(cx)
                        .child(
                            h_flex()
                                .h_10()
                                .px_3()
                                .gap_2()
                                .justify_between()
                                .border_b_1()
                                .border_color(cx.theme().border)
                                .child(h_flex().gap_2().child(badge(
                                    capitalize(extra.kind().title()),
                                    accent_text(cx),
                                )))
                                .child(
                                    icon_button(
                                        ("remove-extra", i),
                                        IconName::Trash,
                                        format!("Remove {}", extra.kind().title()),
                                    )
                                    .small()
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.extras.remove(i);
                                            cx.emit(SchemaDesignerEvent::Changed);
                                            cx.notify();
                                        },
                                    )),
                                ),
                        )
                        .child(
                            div()
                                .grid()
                                .grid_cols(columns)
                                .gap_x_3()
                                .gap_y_1()
                                .p_3()
                                .children(inputs.into_iter().map(|(label, f)| field(label, f))),
                        ),
                );
        }
        v_flex()
            .gap_3()
            .child(section_header(
                "Constraints and indexes",
                self.extras.len(),
                add,
                cx,
            ))
            .child(list)
    }
}
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
fn section_header(
    title: &'static str,
    count: usize,
    action: AnyElement,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .justify_between()
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(count_badge(count, cx)),
        )
        .child(action)
}
impl Render for SchemaDesigner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let subtitle = match &self.original {
            Some(name) => format!("Editing {name} · changes stay local until you review them"),
            None => "New table · changes stay local until you review them".into(),
        };
        let actions = [
            Button::new("reset-schema")
                .ghost()
                .small()
                .icon(IconName::RotateCcw)
                .label("Reset")
                .tooltip("Discard edits and start from the current schema")
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
                }))
                .into_any_element(),
            Button::new("close-schema")
                .ghost()
                .small()
                .label("Discard")
                .tooltip("Close and discard this draft")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SchemaDesignerEvent::Cancel)))
                .into_any_element(),
            Button::new("save-close-schema")
                .outline()
                .small()
                .label("Save and close")
                .tooltip("Keep this draft for later, including after restart")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SchemaDesignerEvent::SaveAndClose)))
                .into_any_element(),
            Button::new("review-schema")
                .primary()
                .small()
                .label("Review changes")
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.emit(SchemaDesignerEvent::Review(Box::new(this.intent(cx))));
                }))
                .into_any_element(),
        ];
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(page_header("Design table", subtitle, actions, cx))
            .child(
                div()
                    .id("schema-designer")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(px(1080.))
                            .mx_auto()
                            .px_6()
                            .py_6()
                            .gap_8()
                            .child(
                                div()
                                    .max_w(px(420.))
                                    .child(field("Table name", &self.name)),
                            )
                            .child(self.columns_section(cx))
                            .child(self.extras_section(cx))
                            .child(
                                v_flex()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Safety"),
                                    )
                                    .child(
                                        card(cx).p_4().gap_1().child(setting_switch(
                                            "allow-destructive-designer",
                                            "Allow destructive changes",
                                            self.allow_destructive,
                                            cx.listener(|this, checked: &bool, _, cx| {
                                                this.allow_destructive = *checked;
                                                cx.emit(SchemaDesignerEvent::Changed);
                                                cx.notify();
                                            }),
                                        ))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("Dropping columns, constraints or indexes needs this consent. It resets when a saved draft is reopened."),
                                        ),
                                    ),
                            ),
                    ),
            )
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
                    view.extras.push(super::ExtraFields::Index {
                        original: None, unique: false,
                        name: super::draft_input("lookup", window, cx), columns: super::draft_input("description", window, cx),
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
