use super::{
    Services,
    controls::{choice, empty_state, icon_button, menu_item, shortcut},
    grid::{Grid, GridEvent},
    row_editor::{RowEditor, RowSaved},
    task,
};
use crate::{database::Session, model::*, sql};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    menu::DropdownMenu,
    resizable::{ResizableState, resizable_panel, v_resizable},
    tab::{Tab, TabBar},
    table::{ColumnSort, DataTable, TableEvent, TableState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{sync::Arc, time::Instant};

#[cfg(all(test, feature = "test-support"))]
#[path = "document_tests.rs"]
mod tests;

actions!(
    queri_document,
    [RunQuery, CancelQuery, Refresh, InspectCell, EditRow]
);

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Data,
    Structure,
    Indexes,
    Constraints,
}
enum DocumentKind {
    Sql {
        editor: Entity<EditorState>,
    },
    Table {
        table: TableRef,
        page: Option<Box<TablePage>>,
        options: Box<BrowseOptions>,
        section: Section,
        filter: Entity<InputState>,
        filter_column: usize,
        filter_operator: FilterOperator,
        filter_apply_focus: FocusHandle,
        filter_open: bool,
    },
}
#[derive(Clone, Copy, PartialEq)]
enum Operation {
    Idle,
    Connecting,
    Running,
    Cancelling,
}

#[derive(Clone, Copy, PartialEq)]
enum QueryOutcome {
    NotRun,
    Succeeded,
    Failed,
    Cancelled,
}

pub struct DocumentChanged;
impl EventEmitter<DocumentChanged> for Document {}
pub struct NewDocumentQuery;
impl EventEmitter<NewDocumentQuery> for Document {}

pub struct Document {
    pub profile: ConnectionProfile,
    pub session: Option<Arc<Session>>,
    services: Services,
    password: String,
    kind: DocumentKind,
    grid: Entity<TableState<Grid>>,
    results: Vec<Arc<ResultSet>>,
    result_index: usize,
    operation: Operation,
    query_outcome: QueryOutcome,
    status: String,
    error: String,
    database_available: bool,
    pub warn_destructive: bool,
    pub editor_height: f32,
    pub query_number: u32,
    subscriptions: Vec<Subscription>,
}

impl Document {
    pub fn init(cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new("ctrl-enter", RunQuery, Some("QueriDocument")),
            KeyBinding::new("ctrl-enter", RunQuery, Some("QueriDocument > Input")),
            KeyBinding::new("ctrl-shift-c", CancelQuery, Some("QueriDocument")),
            KeyBinding::new("f5", Refresh, Some("QueriDocument")),
            KeyBinding::new("alt-enter", InspectCell, Some("QueriDocument")),
            KeyBinding::new("ctrl-e", EditRow, Some("QueriDocument")),
        ]);
    }

    pub fn new(
        services: Services,
        profile: ConnectionProfile,
        password: String,
        table: Option<TableRef>,
        sql: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let kind = match table {
            Some(table) => DocumentKind::Table {
                table,
                page: None,
                options: Box::new(BrowseOptions {
                    limit: 100,
                    ..Default::default()
                }),
                section: Section::Data,
                filter: cx.new(|cx| InputState::new(window, cx).placeholder("Value to match…")),
                filter_column: 0,
                filter_operator: FilterOperator::Contains,
                filter_apply_focus: cx.focus_handle(),
                filter_open: false,
            },
            None => DocumentKind::Sql {
                editor: cx.new(|cx| {
                    EditorState::new(window, cx)
                        .language("sql")
                        .default_value(sql)
                        .placeholder("Write SQL, then press Ctrl+Enter")
                }),
            },
        };
        let grid = cx.new(|cx| {
            TableState::new(Grid::new(Arc::new(ResultSet::default())), window, cx)
                .cell_selectable(true)
        });
        let mut subscriptions = vec![
            cx.subscribe_in(&grid, window, |this, _, event: &GridEvent, window, cx| {
                if this.busy() {
                    return;
                }
                let GridEvent::Sort(column, sort) = event;
                if let DocumentKind::Table { options, .. } = &mut this.kind {
                    options.sort = match sort {
                        ColumnSort::Default => None,
                        ColumnSort::Ascending => Some((column.clone(), false)),
                        ColumnSort::Descending => Some((column.clone(), true)),
                    };
                    options.offset = 0;
                    this.refresh(window, cx);
                }
            }),
            cx.subscribe_in(&grid, window, |this, _, event: &TableEvent, window, cx| {
                if matches!(
                    event,
                    TableEvent::DoubleClickedCell(..) | TableEvent::DoubleClickedRow(..)
                ) {
                    this.inspect(window, cx);
                }
                cx.notify();
            }),
            cx.observe(&grid, |_, _, cx| cx.notify()),
        ];
        if let DocumentKind::Sql { editor } = &kind {
            subscriptions.push(cx.observe(editor, |_, _, cx| cx.notify()));
            subscriptions.push(cx.subscribe(editor, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.emit(DocumentChanged);
                }
            }));
        }
        if let DocumentKind::Table { filter, .. } = &kind {
            subscriptions.push(cx.subscribe_in(
                filter,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.apply_filter(window, cx);
                    }
                },
            ));
        }
        Self {
            profile,
            session: None,
            services,
            password,
            kind,
            grid,
            results: vec![],
            result_index: 0,
            operation: Operation::Idle,
            query_outcome: QueryOutcome::NotRun,
            status: "Connecting…".into(),
            error: String::new(),
            database_available: true,
            warn_destructive: true,
            editor_height: default_editor_height(),
            query_number: 1,
            subscriptions,
        }
    }

    pub fn title(&self) -> String {
        match &self.kind {
            DocumentKind::Sql { .. } => format!("Query {}", self.query_number),
            DocumentKind::Table { table, .. } => format!("{}.{}", table.schema, table.name),
        }
    }
    pub fn table_ref(&self) -> Option<&TableRef> {
        match &self.kind {
            DocumentKind::Table { table, .. } => Some(table),
            _ => None,
        }
    }
    pub fn draft(&self, cx: &App) -> Draft {
        Draft {
            connection_id: self.profile.id,
            database: self.profile.database(),
            query_number: self.table_ref().is_none().then_some(self.query_number),
            sql: match &self.kind {
                DocumentKind::Sql { editor } => editor.read(cx).value().to_string(),
                _ => String::new(),
            },
            table: self.table_ref().cloned(),
            editor_height: self.editor_height,
        }
    }
    pub fn busy(&self) -> bool {
        self.operation != Operation::Idle
    }
    pub fn set_password(&mut self, password: String) {
        self.password = password;
    }
    pub fn reconfigure(&mut self, profile: &ConnectionProfile, password: String) {
        self.profile = profile.for_database(&self.profile.database());
        self.password = password;
        self.session = None;
        self.database_available = true;
    }
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.session = None;
        self.status = "Disconnected".into();
        cx.notify();
    }
    pub fn rename_database(&mut self, name: &str, cx: &mut Context<Self>) {
        self.profile = self.profile.for_database(name);
        self.database_available = true;
        self.disconnect(cx);
    }
    pub fn database_removed(&mut self, cx: &mut Context<Self>) {
        self.database_available = false;
        self.disconnect(cx);
        self.error = "This database is no longer available. Your SQL is kept here; copy it into a new query on another database.".into();
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match &self.kind {
            DocumentKind::Sql { editor } => editor.read(cx).focus_handle(cx).focus(window, cx),
            DocumentKind::Table { .. } => self.grid.read(cx).focus_handle(cx).focus(window, cx),
        }
    }

    pub fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || !self.database_available {
            return;
        }
        self.operation = Operation::Connecting;
        self.error.clear();
        self.status = "Connecting…".into();
        let profile = self.profile.clone();
        let password = self.password.clone();
        task(
            &self.services,
            window,
            cx,
            async move { Session::connect(profile, password).await },
            |this, result, window, cx| {
                this.operation = Operation::Idle;
                match result {
                    Ok(session) => {
                        this.session = Some(session);
                        this.status = "Connected · ready".into();
                        if matches!(this.kind, DocumentKind::Table { .. }) {
                            this.refresh(window, cx);
                        }
                    }
                    Err(error) => {
                        this.error = format!(
                            "{error:#}\nRetry the connection, or edit its settings in the sidebar."
                        );
                        this.status = "Connection failed".into();
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(session) = self.session.clone() else {
            self.connect(window, cx);
            return;
        };
        let DocumentKind::Table { table, options, .. } = &self.kind else {
            return;
        };
        let table = table.clone();
        let options = options.as_ref().clone();
        let filtered = options.filter.is_some();
        self.operation = Operation::Running;
        self.error.clear();
        self.status = "Loading table…".into();
        task(
            &self.services,
            window,
            cx,
            async move { session.browse(table, options).await },
            move |this, result, _, cx| {
                this.operation = Operation::Idle;
                match result {
                    Ok(page) => {
                        this.status = format!(
                            "{} matching rows{}",
                            page.total,
                            if page.data.truncated {
                                " · display size limit reached"
                            } else {
                                ""
                            }
                        );
                        if let DocumentKind::Table {
                            page: current,
                            filter_column,
                            filter_operator,
                            ..
                        } = &mut this.kind
                        {
                            let kind = page
                                .metadata
                                .columns
                                .get(*filter_column)
                                .map(|column| {
                                    column.filter_kind(matches!(
                                        this.profile.config,
                                        ConnectionConfig::Sqlite { .. }
                                    ))
                                })
                                .unwrap_or(FilterKind::Text);
                            if !kind.operators().contains(filter_operator) {
                                *filter_operator = kind.default_operator();
                            }
                            *current = Some(Box::new(page));
                        }
                        this.update_grid(cx);
                    }
                    Err(error) => {
                        let recovery = if filtered {
                            "Check the filter value, change the operator, or clear the filter."
                        } else {
                            "Refresh the table to retry, or check the connection settings."
                        };
                        this.error = format!("{error:#}\n{recovery}");
                        this.status = "Could not load table".into();
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn update_grid(&mut self, cx: &mut Context<Self>) {
        let (data, checkable, sortable) = match &self.kind {
            DocumentKind::Sql { .. } => (
                self.results
                    .get(self.result_index)
                    .cloned()
                    .unwrap_or_default(),
                false,
                false,
            ),
            DocumentKind::Table {
                table,
                page: Some(page),
                section,
                ..
            } => {
                let data = match section {
                    Section::Data => page.data.clone(),
                    Section::Indexes => page.metadata.indexes.clone(),
                    Section::Constraints => page.metadata.constraints.clone(),
                    Section::Structure => ResultSet {
                        columns: vec![
                            "Column".into(),
                            "Type".into(),
                            "Nullable".into(),
                            "Default".into(),
                            "Primary key".into(),
                            "Generated".into(),
                        ],
                        rows: page
                            .metadata
                            .columns
                            .iter()
                            .map(|col| {
                                vec![
                                    Cell::Text(col.name.clone()),
                                    Cell::Text(col.data_type.clone()),
                                    Cell::Text(col.nullable.to_string()),
                                    col.default.clone().map(Cell::Text).unwrap_or(Cell::Null),
                                    Cell::Text(col.primary_key.to_string()),
                                    Cell::Text(col.generated.to_string()),
                                ]
                            })
                            .collect(),
                        ..Default::default()
                    },
                };
                (
                    Arc::new(data),
                    *section == Section::Data
                        && page.metadata.editable(table)
                        && !self
                            .session
                            .as_ref()
                            .is_some_and(|session| session.read_only()),
                    *section == Section::Data,
                )
            }
            _ => return,
        };
        self.grid.update(cx, |state, cx| {
            let grid = state.delegate_mut();
            grid.replace(data);
            if let DocumentKind::Table {
                page: Some(page),
                section: Section::Data,
                ..
            } = &self.kind
            {
                grid.set_column_types(&page.metadata.columns);
            }
            grid.checkable = checkable;
            grid.sortable = sortable;
            grid.sort = match &self.kind {
                DocumentKind::Table { options, .. } => {
                    options.sort.as_ref().map(|(column, descending)| {
                        (
                            column.clone(),
                            if *descending {
                                ColumnSort::Descending
                            } else {
                                ColumnSort::Ascending
                            },
                        )
                    })
                }
                _ => None,
            };
            state.refresh(cx);
            state.clear_selection(cx);
            cx.notify();
        });
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let DocumentKind::Sql { editor } = &self.kind else {
            return;
        };
        let selected = editor.read(cx).selected_value().to_string();
        let sql = if selected.trim().is_empty() {
            editor.read(cx).value().to_string()
        } else {
            selected
        };
        if sql.trim().is_empty() {
            self.error = "Enter SQL to execute.".into();
            cx.notify();
            return;
        }
        if self.warn_destructive && sql::destructive(&sql) {
            let view = cx.entity();
            let database = self.profile.database();
            window.open_alert_dialog(cx, move |dialog, _, _| {
                let sql = sql.clone();
                let view = view.clone();
                dialog
                    .title("Run destructive SQL?")
                    .child(format!(
                        "This statement can remove data or database objects in {database}."
                    ))
                    .button_props(
                        DialogButtonProps::default()
                            .ok_text("Run SQL")
                            .show_cancel(true),
                    )
                    .on_ok(move |_, window, cx| {
                        view.update(cx, |this, cx| this.execute(sql.clone(), window, cx));
                        true
                    })
            });
        } else {
            self.execute(sql, window, cx);
        }
    }

    fn execute(&mut self, sql: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(session) = self.session.clone() else {
            self.error = "Connect before running SQL.".into();
            cx.notify();
            return;
        };
        let store = self.services.store.clone();
        let profile = self.profile.clone();
        self.operation = Operation::Running;
        self.status = "Executing…".into();
        self.error.clear();
        task(
            &self.services,
            window,
            cx,
            async move {
                let start = Instant::now();
                let result = session.query(sql.clone()).await;
                let entry = HistoryEntry {
                    id: 0,
                    connection_id: profile.id,
                    database: profile.database(),
                    sql,
                    executed_at: chrono::Utc::now().to_rfc3339(),
                    elapsed_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                    affected: result
                        .as_ref()
                        .map(|output| output.sets.iter().map(|set| set.affected).sum())
                        .unwrap_or(0),
                    error: result.as_ref().err().map(|error| format!("{error:#}")),
                };
                let history_error = tokio::task::spawn_blocking(move || store.record(&entry))
                    .await?
                    .err()
                    .map(|error| format!("{error:#}"));
                Ok((result, history_error))
            },
            |this, result, _, cx| {
                let cancelled = this.operation == Operation::Cancelling;
                this.operation = Operation::Idle;
                match result {
                    Ok((Ok(output), history_error)) => {
                        this.query_outcome = QueryOutcome::Succeeded;
                        this.status = format!(
                            "Completed in {} ms · {} result sets",
                            output.elapsed_ms,
                            output.sets.len()
                        );
                        if let Some(error) = history_error {
                            this.status
                                .push_str(&format!(" · History could not be saved: {error}"));
                        }
                        this.results = output.sets.into_iter().map(Arc::new).collect();
                        this.result_index = 0;
                        this.update_grid(cx);
                    }
                    Ok((Err(error), _)) | Err(error) => {
                        this.query_outcome = if cancelled {
                            QueryOutcome::Cancelled
                        } else {
                            QueryOutcome::Failed
                        };
                        this.error = format!(
                            "{error:#}\nReview your SQL and run it again when you are ready."
                        );
                        this.status = if cancelled {
                            "Query cancelled"
                        } else {
                            "Query failed"
                        }
                        .into();
                    }
                }
                cx.emit(DocumentChanged);
                cx.notify();
            },
        );
        cx.notify();
    }

    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.operation != Operation::Running {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.operation = Operation::Cancelling;
        self.status = "Cancelling…".into();
        task(
            &self.services,
            window,
            cx,
            async move { session.cancel().await },
            |this, result, _, cx| {
                if let Err(error) = result {
                    this.error = format!("{error:#}");
                    if this.operation == Operation::Cancelling {
                        this.operation = Operation::Running;
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn selected_row(&self, cx: &App) -> Option<usize> {
        let grid = self.grid.read(cx);
        if grid.delegate().checked.len() == 1 {
            return grid.delegate().checked.first().copied();
        }
        grid.selected_row()
            .or_else(|| grid.selected_cell().map(|(row, _)| row))
    }

    fn inspect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let grid = self.grid.read(cx);
        let Some(row_index) = self.selected_row(cx) else {
            return;
        };
        let Some(row) = grid.delegate().data.rows.get(row_index) else {
            return;
        };
        let columns = &grid.delegate().data.columns;
        let text = if let Some((_, col)) = grid.selected_cell() {
            row.get(col.saturating_sub(usize::from(grid.delegate().checkable)))
                .map(Cell::text)
                .unwrap_or_default()
        } else {
            columns
                .iter()
                .zip(row)
                .map(|(name, cell)| format!("{name}: {}", cell.text()))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .default_value(text.clone())
                .soft_wrap(true)
        });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Cell / row contents")
                .child(div().h(px(360.)).child(Editor::new(&editor).readonly(true)))
                .child(
                    Button::new("copy-value")
                        .label("Copy full value")
                        .on_click({
                            let text = text.clone();
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                            }
                        }),
                )
        });
    }

    fn edit_row(&mut self, insert: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let DocumentKind::Table {
            table,
            page: Some(page),
            section: Section::Data,
            ..
        } = &self.kind
        else {
            return;
        };
        let Some(session) = self.session.clone() else {
            return;
        };
        if session.read_only() || table.kind != "table" {
            return;
        }
        let original = if insert {
            None
        } else {
            if !page.metadata.editable(table) {
                return;
            }
            let Some(index) = self.selected_row(cx) else {
                return;
            };
            let Some(row) = page.data.rows.get(index) else {
                return;
            };
            Some(row.clone())
        };
        let editor = cx.new(|cx| {
            RowEditor::new(
                self.services.clone(),
                session,
                table.clone(),
                page.metadata.clone(),
                original,
                window,
                cx,
            )
        });
        self.subscriptions.push(cx.subscribe_in(
            &editor,
            window,
            |this, _, _: &RowSaved, window, cx| this.refresh(window, cx),
        ));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(if insert { "Insert row" } else { "Edit row" })
                .child(editor.clone())
        });
    }

    fn delete_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let DocumentKind::Table {
            table,
            page: Some(page),
            section: Section::Data,
            ..
        } = &self.kind
        else {
            return;
        };
        if !page.metadata.editable(table) {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        if session.read_only() {
            return;
        }
        let mut indices = self
            .grid
            .read(cx)
            .delegate()
            .checked
            .iter()
            .copied()
            .collect::<Vec<_>>();
        if indices.is_empty()
            && let Some(row) = self.selected_row(cx)
        {
            indices.push(row);
        }
        let originals = indices
            .into_iter()
            .filter_map(|i| page.data.rows.get(i).cloned())
            .collect::<Vec<_>>();
        if originals.is_empty() {
            return;
        }
        let table = table.clone();
        let view = cx.entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let (view, session, table, originals) = (
                view.clone(),
                session.clone(),
                table.clone(),
                originals.clone(),
            );
            dialog
                .title(format!("Delete {} rows?", originals.len()))
                .child(format!(
                    "Delete the selected rows from {}.{}?",
                    table.schema, table.name
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete rows")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let (session, table, originals) =
                        (session.clone(), table.clone(), originals.clone());
                    view.update(cx, |this, cx| {
                        this.operation = Operation::Running;
                        task(
                            &this.services,
                            window,
                            cx,
                            async move {
                                session
                                    .change_rows(table, RowChange::Delete { originals })
                                    .await
                            },
                            |this, result, window, cx| {
                                this.operation = Operation::Idle;
                                match result {
                                    Ok(_) => this.refresh(window, cx),
                                    Err(error) => this.error = format!("{error:#}"),
                                };
                                cx.notify();
                            },
                        );
                        cx.notify();
                    });
                    true
                })
        });
    }

    fn apply_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        if let DocumentKind::Table {
            page: Some(page),
            options,
            filter,
            filter_column,
            filter_operator,
            ..
        } = &mut self.kind
        {
            let value = filter.read(cx).value().to_string();
            options.filter = if value.is_empty() && *filter_operator == FilterOperator::Contains {
                None
            } else {
                page.metadata
                    .columns
                    .get(*filter_column)
                    .map(|col| TableFilter {
                        column: col.name.clone(),
                        predicate: filter_operator.predicate(value),
                    })
            };
            options.offset = 0;
        }
        self.refresh(window, cx);
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let DocumentKind::Table {
            options, filter, ..
        } = &mut self.kind
        {
            filter.update(cx, |filter, cx| filter.set_value("", window, cx));
            options.filter = None;
            options.offset = 0;
        }
        self.refresh(window, cx);
    }

    fn can_insert(&self) -> bool {
        !self.busy()
            && self
                .session
                .as_ref()
                .is_some_and(|session| !session.read_only())
            && matches!(&self.kind, DocumentKind::Table { table, page: Some(_), .. } if table.kind == "table")
    }

    fn table_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let DocumentKind::Table {
            table,
            page,
            section,
            options,
            ..
        } = &self.kind
        else {
            return div().into_any_element();
        };
        let Some(page) = page else {
            let (title, description) = if self.busy() {
                ("Loading table…", "Fetching rows and the table definition.")
            } else {
                (
                    "Table unavailable",
                    "Refresh to try again, or check the connection settings.",
                )
            };
            return empty_state(IconName::Table2, title, description)
                .when(!self.busy() && self.database_available, |view| {
                    view.child(
                        Button::new("empty-refresh-table")
                            .label("Refresh table")
                            .icon(IconName::RefreshCw)
                            .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                    )
                })
                .into_any_element();
        };
        let is_empty = match section {
            Section::Data => page.data.rows.is_empty(),
            Section::Structure => page.metadata.columns.is_empty(),
            Section::Indexes => page.metadata.indexes.rows.is_empty(),
            Section::Constraints => page.metadata.constraints.rows.is_empty(),
        };
        if !is_empty {
            return DataTable::new(&self.grid)
                .bordered(false)
                .into_any_element();
        }
        match section {
            Section::Data if options.filter.is_some() => empty_state(
                IconName::Search,
                "No matching rows",
                "Try another filter value, or clear the filter to see all rows.",
            )
            .child(
                Button::new("empty-clear-filter")
                    .label("Clear filter")
                    .disabled(self.busy())
                    .on_click(cx.listener(|this, _, window, cx| this.clear_filter(window, cx))),
            )
            .into_any_element(),
            Section::Data => {
                let read_only = self
                    .session
                    .as_ref()
                    .is_some_and(|session| session.read_only());
                let (title, description) = if table.kind != "table" {
                    (
                        "This view has no rows",
                        "Views show data from their underlying tables. Refresh to check for changes.",
                    )
                } else if read_only {
                    (
                        "This table is empty",
                        "This connection is read-only. Refresh to check for new rows.",
                    )
                } else {
                    (
                        "This table is empty",
                        "Add your first row to start working with this table.",
                    )
                };
                empty_state(IconName::Table2, title, description)
                    .when(self.can_insert(), |view| {
                        view.child(
                            Button::new("empty-insert-row")
                                .primary()
                                .icon(IconName::Plus)
                                .label("Insert row")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.edit_row(true, window, cx)
                                })),
                        )
                    })
                    .when(read_only || table.kind != "table", |view| {
                        view.child(
                            Button::new("empty-refresh-table")
                                .icon(IconName::RefreshCw)
                                .label("Refresh table")
                                .disabled(self.busy())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.refresh(window, cx)),
                                ),
                        )
                    })
                    .into_any_element()
            }
            Section::Structure => empty_state(
                IconName::Table2,
                "No columns available",
                "Refresh the table to reload its definition.",
            )
            .child(
                Button::new("empty-refresh-table")
                    .label("Refresh table")
                    .disabled(self.busy())
                    .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
            )
            .into_any_element(),
            Section::Indexes | Section::Constraints => {
                let read_only = self
                    .session
                    .as_ref()
                    .is_some_and(|session| session.read_only());
                let (title, description) = if *section == Section::Indexes && table.kind != "table"
                {
                    (
                        "No indexes",
                        "No indexes are reported for this view. Inspect its underlying tables for index definitions.",
                    )
                } else if *section == Section::Constraints && table.kind != "table" {
                    (
                        "No constraints",
                        "No constraints are reported for this view. Inspect its underlying tables for data rules.",
                    )
                } else if *section == Section::Indexes && read_only {
                    (
                        "No indexes",
                        "This table has no indexes. This connection is read-only.",
                    )
                } else if read_only {
                    (
                        "No constraints",
                        "This table has no constraints. This connection is read-only.",
                    )
                } else if *section == Section::Indexes {
                    (
                        "No indexes",
                        "Indexes help your database find rows efficiently. You can define them with SQL.",
                    )
                } else {
                    (
                        "No constraints",
                        "Constraints define rules for your data. You can add them with SQL.",
                    )
                };
                empty_state(
                    if *section == Section::Indexes {
                        IconName::ListTree
                    } else {
                        IconName::ShieldCheck
                    },
                    title,
                    description,
                )
                .child(
                    Button::new("empty-metadata-query")
                        .icon(IconName::Code)
                        .label("Open SQL editor")
                        .disabled(!self.database_available)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(NewDocumentQuery))),
                )
                .into_any_element()
            }
        }
    }

    fn query_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let selection = matches!(&self.kind, DocumentKind::Sql { editor } if !editor.read(cx).selected_value().trim().is_empty());
        h_flex()
            .gap_2()
            .child(
                Button::new("run-query")
                    .primary()
                    .small()
                    .icon(IconName::Play)
                    .label(if selection {
                        "Run selection"
                    } else {
                        "Run query"
                    })
                    .tooltip(if selection {
                        "Run selected SQL · Ctrl+Enter"
                    } else {
                        "Run entire script · Ctrl+Enter"
                    })
                    .disabled(self.busy() || self.session.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.run(window, cx))),
            )
            .child(shortcut("Ctrl ↵", cx))
            .when(
                matches!(self.operation, Operation::Running | Operation::Cancelling),
                |bar| {
                    bar.child(
                        Button::new("cancel-query")
                            .small()
                            .label(if self.operation == Operation::Cancelling {
                                "Cancelling…"
                            } else {
                                "Cancel"
                            })
                            .disabled(self.operation != Operation::Running)
                            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                    )
                },
            )
            .into_any_element()
    }

    fn results_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = |result: &ResultSet| {
            format!(
                "{} {}{}",
                result.affected,
                if result.columns.is_empty() {
                    "affected"
                } else if result.affected == 1 {
                    "row"
                } else {
                    "rows"
                },
                if result.truncated {
                    " · display limit reached"
                } else {
                    ""
                }
            )
        };
        h_flex()
            .id("result-tabs")
            .flex_shrink_0()
            .flex_wrap()
            .min_h_10()
            .px_4()
            .gap_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .when(self.results.len() <= 1, |bar| {
                bar.child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Results"),
                )
                .when_some(self.results.first(), |bar, result| {
                    bar.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(count(result)),
                    )
                })
            })
            .when(self.results.len() > 1, |bar| {
                bar.child(
                    div().flex_1().min_w_0().child(
                        TabBar::new("sql-result-tabs")
                            .underline()
                            .small()
                            .menu(true)
                            .selected_index(self.result_index)
                            .on_click(cx.listener(|this, index, _, cx| {
                                this.result_index = *index;
                                this.update_grid(cx);
                                cx.notify();
                            }))
                            .children(self.results.iter().enumerate().map(|(index, result)| {
                                Tab::new().label(format!(
                                    "{} {} · {}",
                                    if result.columns.is_empty() {
                                        "Statement"
                                    } else {
                                        "Result"
                                    },
                                    index + 1,
                                    count(result)
                                ))
                            })),
                    ),
                )
            })
            .child(div().flex_1())
            .child(
                h_flex()
                    .id("query-status")
                    .role(Role::Status)
                    .gap_1p5()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        Icon::new(if self.busy() {
                            IconName::RefreshCw
                        } else {
                            match self.query_outcome {
                                QueryOutcome::Succeeded => IconName::Check,
                                QueryOutcome::Failed => IconName::CircleAlert,
                                QueryOutcome::Cancelled => IconName::Square,
                                QueryOutcome::NotRun => IconName::Code,
                            }
                        })
                        .small(),
                    )
                    .child(self.status.clone()),
            )
            .when(
                !self.busy()
                    && !self.results.is_empty()
                    && matches!(
                        self.query_outcome,
                        QueryOutcome::Failed | QueryOutcome::Cancelled
                    ),
                |bar| {
                    bar.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Previous results"),
                    )
                },
            )
            .into_any_element()
    }

    fn query_results(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(result) = self.results.get(self.result_index) {
            if result.columns.is_empty() {
                return empty_state(
                    IconName::Check,
                    "Statement complete",
                    format!(
                        "{} {} affected. This statement did not return a result table.",
                        result.affected,
                        if result.affected == 1 { "row" } else { "rows" }
                    ),
                )
                .into_any_element();
            }
            if result.rows.is_empty() {
                return empty_state(
                    IconName::Search,
                    "No rows returned",
                    "Query completed. Adjust your conditions to find more data.",
                )
                .into_any_element();
            }
            return DataTable::new(&self.grid)
                .bordered(false)
                .into_any_element();
        }
        let (title, description) = match self.operation {
            Operation::Connecting => ("Connecting…", "Opening a database session for this query."),
            Operation::Running => (
                "Running query…",
                "Results will appear here when your query completes.",
            ),
            Operation::Cancelling => (
                "Cancelling query…",
                "Waiting for the database to stop the active statement.",
            ),
            Operation::Idle if self.session.is_none() => (
                "Connect to run SQL",
                "Reconnect this session to execute queries against the database.",
            ),
            Operation::Idle => match self.query_outcome {
                QueryOutcome::NotRun => (
                    "Your results will appear here",
                    "Write SQL above, then run it with Ctrl+Enter.",
                ),
                QueryOutcome::Succeeded => (
                    "Query complete",
                    "The script completed without returning a result table.",
                ),
                QueryOutcome::Failed => (
                    "Query could not complete",
                    "Review the error below, adjust your SQL, and try again.",
                ),
                QueryOutcome::Cancelled => (
                    "Query cancelled",
                    "You can edit or run your query again when you are ready.",
                ),
            },
        };
        let icon = match self.operation {
            Operation::Connecting | Operation::Running | Operation::Cancelling => {
                IconName::RefreshCw
            }
            Operation::Idle if self.session.is_none() => IconName::Unplug,
            Operation::Idle => match self.query_outcome {
                QueryOutcome::NotRun => IconName::Code,
                QueryOutcome::Succeeded => IconName::Check,
                QueryOutcome::Failed => IconName::CircleAlert,
                QueryOutcome::Cancelled => IconName::Square,
            },
        };
        empty_state(icon, title, description)
            .when(
                !self.busy() && self.session.is_none() && self.database_available,
                |view| {
                    view.child(
                        Button::new("empty-reconnect")
                            .icon(IconName::RefreshCw)
                            .label("Reconnect")
                            .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))),
                    )
                },
            )
            .when(
                self.query_outcome == QueryOutcome::NotRun
                    && self.session.is_some()
                    && !self.busy(),
                |view| {
                    view.child(
                        Button::new("focus-sql-editor")
                            .ghost()
                            .small()
                            .icon(IconName::Pencil)
                            .label("Write a query")
                            .on_click(cx.listener(|this, _, window, cx| this.focus(window, cx))),
                    )
                },
            )
            .into_any_element()
    }

    fn table_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let DocumentKind::Table {
            table,
            page,
            options,
            section,
            filter,
            filter_column,
            filter_operator,
            filter_apply_focus,
            filter_open,
            ..
        } = &self.kind
        else {
            return div().into_any_element();
        };
        let column = page
            .as_ref()
            .and_then(|page| page.metadata.columns.get(*filter_column))
            .map(|col| col.name.clone())
            .unwrap_or_else(|| "Column".into());
        let sqlite = matches!(self.profile.config, ConnectionConfig::Sqlite { .. });
        let filter_kind = page
            .as_ref()
            .and_then(|page| page.metadata.columns.get(*filter_column))
            .map(|column| column.filter_kind(sqlite))
            .unwrap_or(FilterKind::Text);
        let writable = self.can_insert() && *section == Section::Data;
        let editable = writable
            && page
                .as_ref()
                .is_some_and(|page| page.metadata.editable(table));
        let selected = self.grid.read(cx).delegate().checked.len();
        let selected = if selected == 0 {
            usize::from(self.selected_row(cx).is_some())
        } else {
            selected
        };
        let sections = TabBar::new("table-sections")
            .segmented()
            .selected_index(match section {
                Section::Data => 0,
                Section::Structure => 1,
                Section::Indexes => 2,
                Section::Constraints => 3,
            })
            .children(
                ["Data", "Structure", "Indexes", "Constraints"]
                    .map(|label| Tab::new().label(label)),
            )
            .on_click(cx.listener(|this, index, _, cx| {
                if let DocumentKind::Table { section, .. } = &mut this.kind {
                    *section = match index {
                        1 => Section::Structure,
                        2 => Section::Indexes,
                        3 => Section::Constraints,
                        _ => Section::Data,
                    };
                }
                this.update_grid(cx);
                cx.notify();
            }));
        v_flex()
            .flex_shrink_0()
            .px_4()
            .pb_2()
            .child(
                h_flex()
                    .gap_3()
                    .flex_wrap()
                    .justify_between()
                    .child(div().min_w(px(270.)).child(sections))
                    .when(*section == Section::Data, |view| {
                        view.child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    Button::new("toggle-filter")
                                        .ghost()
                                        .icon(IconName::ListFilter)
                                        .label(if options.filter.is_some() {
                                            "Filter applied"
                                        } else {
                                            "Filter"
                                        })
                                        .selected(*filter_open || options.filter.is_some())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            if let DocumentKind::Table {
                                                filter_open,
                                                filter,
                                                filter_operator,
                                                filter_apply_focus,
                                                ..
                                            } = &mut this.kind
                                            {
                                                *filter_open = !*filter_open;
                                                if *filter_open {
                                                    if filter_operator.needs_value() {
                                                        filter
                                                            .read(cx)
                                                            .focus_handle(cx)
                                                            .focus(window, cx);
                                                    } else {
                                                        filter_apply_focus.focus(window, cx);
                                                    }
                                                }
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    icon_button(
                                        "refresh-table",
                                        IconName::RefreshCw,
                                        "Refresh table · F5",
                                    )
                                    .disabled(self.busy())
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.refresh(window, cx)),
                                    ),
                                )
                                .when(selected > 0, |row| {
                                    row.child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!("{selected} selected")),
                                    )
                                    .child(
                                        Button::new("edit")
                                            .ghost()
                                            .icon(IconName::Pencil)
                                            .label("Edit")
                                            .tooltip("Edit row · Ctrl+E")
                                            .disabled(
                                                !editable
                                                    || selected != 1
                                                    || self.selected_row(cx).is_none(),
                                            )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.edit_row(false, window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("delete")
                                            .danger()
                                            .icon(IconName::Trash)
                                            .label("Delete")
                                            .tooltip("Delete selected rows")
                                            .disabled(
                                                !editable
                                                    || (self.selected_row(cx).is_none()
                                                        && self
                                                            .grid
                                                            .read(cx)
                                                            .delegate()
                                                            .checked
                                                            .is_empty()),
                                            )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.delete_rows(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("inspect-cell")
                                            .ghost()
                                            .icon(IconName::ScanText)
                                            .label("Inspect")
                                            .tooltip("Inspect cell · Alt+Enter")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.inspect(window, cx)
                                            })),
                                    )
                                })
                                .child(
                                    Button::new("insert")
                                        .icon(IconName::Plus)
                                        .label("Insert row")
                                        .disabled(!writable)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.edit_row(true, window, cx)
                                        })),
                                ),
                        )
                    }),
            )
            .when(*section == Section::Data && *filter_open, |view| {
                let column_picker = choice(
                    Button::new("filter-column")
                        .small()
                        .w_full()
                        .disabled(self.busy() || page.is_none())
                        .accessibility_label(format!("Filter column: {column}"))
                        .label(column.clone())
                        .tooltip(column),
                    *filter_column,
                    page.as_ref()
                        .map(|page| {
                            page.metadata
                                .columns
                                .iter()
                                .enumerate()
                                .map(|(i, col)| (i, SharedString::from(col.name.clone())))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                    cx.listener(|this, index, _, cx| {
                        if let DocumentKind::Table {
                            filter_column,
                            filter_operator,
                            page,
                            ..
                        } = &mut this.kind
                        {
                            *filter_column = *index;
                            let kind = page
                                .as_ref()
                                .and_then(|page| page.metadata.columns.get(*index))
                                .map(|column| {
                                    column.filter_kind(matches!(
                                        this.profile.config,
                                        ConnectionConfig::Sqlite { .. }
                                    ))
                                })
                                .unwrap_or(FilterKind::Text);
                            if !kind.operators().contains(filter_operator) {
                                *filter_operator = kind.default_operator();
                            }
                        }
                        cx.notify();
                    }),
                );
                let operator_picker = choice(
                    Button::new("filter-operator")
                        .small()
                        .disabled(self.busy() || page.is_none())
                        .accessibility_label(format!(
                            "Filter operator: {}",
                            filter_operator.label(filter_kind)
                        ))
                        .label(filter_operator.label(filter_kind)),
                    *filter_operator,
                    filter_kind.operators().iter().map(|operator| {
                        (*operator, SharedString::from(operator.label(filter_kind)))
                    }),
                    cx.listener(|this, operator, window, cx| {
                        if let DocumentKind::Table {
                            filter_operator,
                            filter,
                            filter_apply_focus,
                            ..
                        } = &mut this.kind
                        {
                            *filter_operator = *operator;
                            if operator.needs_value() {
                                filter.read(cx).focus_handle(cx).focus(window, cx);
                            } else {
                                filter_apply_focus.focus(window, cx);
                            }
                        }
                        cx.notify();
                    }),
                );
                view.child(
                    h_flex()
                        .mt_2()
                        .p_2()
                        .gap_2()
                        .flex_wrap()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().muted)
                        .child(div().w(px(140.)).max_w_full().child(column_picker))
                        .child(operator_picker)
                        .when(filter_operator.needs_value(), |row| {
                            row.child(
                                div().flex_1().min_w(px(120.)).child(
                                    Input::new(filter)
                                        .id("filter-value")
                                        .small()
                                        .aria_label("Filter value"),
                                ),
                            )
                        })
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_shrink_0()
                                .child(
                                    Button::new("apply-filter")
                                        .track_focus(filter_apply_focus)
                                        .small()
                                        .primary()
                                        .label("Apply")
                                        .tooltip("Apply filter · Enter")
                                        .disabled(self.busy())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.apply_filter(window, cx)
                                        })),
                                )
                                .when(options.filter.is_some(), |row| {
                                    row.child(
                                        icon_button("clear-filter", IconName::X, "Clear filter")
                                            .small()
                                            .disabled(self.busy())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.clear_filter(window, cx)
                                            })),
                                    )
                                }),
                        ),
                )
            })
            .when(*section == Section::Data && !*filter_open, |view| {
                view.when_some(options.filter.as_ref(), |view, filter| {
                    let kind = page
                        .as_ref()
                        .and_then(|page| {
                            page.metadata
                                .columns
                                .iter()
                                .find(|column| column.name == filter.column)
                        })
                        .map(|column| column.filter_kind(sqlite))
                        .unwrap_or(FilterKind::Text);
                    let summary = filter.summary(kind);
                    view.child(
                        h_flex()
                            .pt_2()
                            .gap_2()
                            .child(Icon::new(IconName::ListFilter).small())
                            .child(
                                div()
                                    .id("applied-filter")
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .text_ellipsis()
                                    .child(summary.clone())
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(summary.clone()).build(window, cx)
                                    }),
                            )
                            .child(
                                Button::new("clear-applied-filter")
                                    .ghost()
                                    .small()
                                    .label("Clear filter")
                                    .disabled(self.busy())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.clear_filter(window, cx)
                                    })),
                            ),
                    )
                })
            })
            .when(
                *section == Section::Data && !editable && page.is_some(),
                |view| {
                    view.child(
                        div()
                            .pt_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                if self
                                    .session
                                    .as_ref()
                                    .is_some_and(|session| session.read_only())
                                {
                                    "Read-only connection"
                                } else if table.kind != "table" {
                                    "Views are read-only"
                                } else {
                                    "Editing and deleting rows requires a primary key"
                                },
                            ),
                    )
                },
            )
            .into_any_element()
    }

    fn table_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let DocumentKind::Table {
            options,
            page,
            section,
            ..
        } = &self.kind
        else {
            return div().into_any_element();
        };
        if *section != Section::Data {
            return div().into_any_element();
        }
        let summary = if self.busy() || !self.error.is_empty() {
            self.status.clone()
        } else if let Some(page) = page {
            if page.total == 0 {
                "0 rows".into()
            } else {
                let shown = u64::try_from(page.data.rows.len()).unwrap_or(0);
                format!(
                    "{}–{} of {} rows{}",
                    (options.offset + 1).min(page.total),
                    (options.offset + shown).min(page.total),
                    page.total,
                    if page.data.truncated {
                        " · display limit reached"
                    } else {
                        ""
                    }
                )
            }
        } else {
            self.status.clone()
        };
        h_flex()
            .gap_2()
            .px_4()
            .py_2()
            .flex_wrap()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(cx.theme().muted_foreground)
                    .child(summary),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        icon_button("previous", IconName::ChevronLeft, "Previous page")
                            .disabled(self.busy() || options.offset == 0)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let DocumentKind::Table { options, .. } = &mut this.kind {
                                    options.offset =
                                        options.offset.saturating_sub(u64::from(options.limit));
                                }
                                this.refresh(window, cx);
                            })),
                    )
                    .child(div().text_sm().child(format!(
                        "Page {}",
                        options.offset / u64::from(options.limit) + 1,
                    )))
                    .child(
                        icon_button("next", IconName::ChevronRight, "Next page")
                            .disabled(
                                self.busy()
                                    || page.as_ref().is_none_or(|page| {
                                        options.offset + u64::from(options.limit) >= page.total
                                    }),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let DocumentKind::Table { options, .. } = &mut this.kind {
                                    options.offset += u64::from(options.limit);
                                }
                                this.refresh(window, cx);
                            })),
                    )
                    .child(choice(
                        Button::new("page-size")
                            .small()
                            .ghost()
                            .label(format!("{} rows", options.limit))
                            .accessibility_label("Rows per page")
                            .disabled(self.busy()),
                        options.limit,
                        [
                            (100, "100 rows".into()),
                            (250, "250 rows".into()),
                            (1000, "1,000 rows".into()),
                        ],
                        cx.listener(|this, limit, window, cx| {
                            if let DocumentKind::Table { options, .. } = &mut this.kind {
                                options.limit = *limit;
                                options.offset = 0;
                            }
                            this.refresh(window, cx);
                        }),
                    )),
            )
            .into_any_element()
    }
}

impl Render for Document {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let context = format!(
            "{} / {}{}",
            self.profile.name,
            self.profile.database(),
            self.table_ref()
                .map(|table| format!(" / {}", table.schema))
                .unwrap_or_default()
        );
        let content = match &self.kind {
            DocumentKind::Sql { editor } => v_flex()
                .size_full()
                .min_h_0()
                .child(
                    div().flex_1().min_h_0().overflow_hidden().child(
                        v_resizable(("sql-split", cx.entity_id()))
                            .on_resize(cx.listener(
                                |this, state: &Entity<ResizableState>, _, cx| {
                                    if let Some(height) = state.read(cx).sizes().first() {
                                        this.editor_height = f32::from(*height);
                                        cx.emit(DocumentChanged);
                                    }
                                },
                            ))
                            .child(
                                resizable_panel()
                                    .size(px(self.editor_height))
                                    .size_range(px(100.)..px(1200.))
                                    .child(
                                        Editor::new(editor)
                                            .bordered(false)
                                            .aria_label("SQL editor"),
                                    ),
                            )
                            .child(
                                resizable_panel().size_range(px(260.)..px(2000.)).child(
                                    v_flex().size_full().child(self.results_header(cx)).child(
                                        v_flex()
                                            .flex_1()
                                            .min_h_0()
                                            .overflow_hidden()
                                            .child(self.query_results(cx)),
                                    ),
                                ),
                            ),
                    ),
                )
                .into_any_element(),
            DocumentKind::Table { .. } => v_flex()
                .size_full()
                .min_h_0()
                .child(self.table_toolbar(cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .child(self.table_body(cx)),
                )
                .child(self.table_footer(cx))
                .into_any_element(),
        };
        v_flex()
            .id("document")
            .key_context("QueriDocument")
            .size_full()
            .min_w_0()
            // Keep the table's keyboard target mounted when its grid becomes an empty state.
            .when(
                matches!(self.kind, DocumentKind::Table { .. })
                    && self.grid.read(cx).delegate().data.rows.is_empty(),
                |view| view.track_focus(&self.grid.read(cx).focus_handle(cx)),
            )
            .on_action(cx.listener(|this, _: &RunQuery, window, cx| this.run(window, cx)))
            .on_action(cx.listener(|this, _: &CancelQuery, window, cx| this.cancel(window, cx)))
            .on_action(cx.listener(|this, _: &Refresh, window, cx| this.refresh(window, cx)))
            .on_action(cx.listener(|this, _: &InspectCell, window, cx| this.inspect(window, cx)))
            .on_action(
                cx.listener(|this, _: &EditRow, window, cx| this.edit_row(false, window, cx)),
            )
            .child(
                h_flex()
                    .gap_3()
                    .flex_shrink_0()
                    .flex_wrap()
                    .min_h(px(56.))
                    .px_4()
                    .py_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(140.))
                            .gap_0p5()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_ellipsis()
                                    .child(self.table_ref().map(|table| table.name.clone()).unwrap_or_else(|| self.title())),
                            )
                            .child(
                                div()
                                    .id("document-context")
                                    .text_ellipsis()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(context.clone())
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(context.clone()).build(window, cx)
                                    }),
                            ),
                    )
                    .when(matches!(self.kind, DocumentKind::Sql { .. }), |bar| {
                        bar.child(self.query_actions(cx))
                    })
                    .when(matches!(self.kind, DocumentKind::Table { section, .. } if section != Section::Data), |bar| {
                        bar.child(
                            icon_button("refresh-metadata", IconName::RefreshCw, "Refresh table · F5")
                                .disabled(self.busy())
                                .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                        )
                    })
                    .when(
                        self.session.is_none() && self.database_available && !self.busy(),
                        |bar| {
                            bar.child(
                                Button::new("reconnect")
                                    .small()
                                    .label("Reconnect")
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.connect(window, cx)),
                                    ),
                            )
                        },
                    )
                    .when(
                        matches!(self.kind, DocumentKind::Sql { .. })
                            && self.selected_row(cx).is_some(),
                        |bar| {
                            bar.child(
                                icon_button(
                                    "inspect-cell",
                                    IconName::ScanText,
                                    "Inspect cell · Alt+Enter",
                                )
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.inspect(window, cx)),
                                ),
                            )
                        },
                    )
                    .child(
                        icon_button("document-menu", IconName::Ellipsis, "Document actions")
                            .dropdown_menu({
                                let view = cx.entity();
                                move |menu, _, cx| {
                                    view.update(cx, |this, cx| {
                                        let context = format!(
                                            "{} / {}",
                                            this.profile.name,
                                            this.profile.database()
                                        );
                                        let menu = menu
                                            .item(
                                                menu_item("New query on this database", IconName::Code)
                                                    .disabled(!this.database_available)
                                                    .on_click(cx.listener(|_, _, _, cx| {
                                                        cx.emit(NewDocumentQuery)
                                                    })),
                                            )
                                            .item(
                                                menu_item("Copy connection context", IconName::Copy)
                                                    .on_click(move |_, _, cx| {
                                                        cx.write_to_clipboard(
                                                            ClipboardItem::new_string(
                                                                context.clone(),
                                                            ),
                                                        )
                                                    }),
                                            )
                                            .separator()
                                            .item(
                                                menu_item("Reconnect session", Icon::empty())
                                                    .disabled(
                                                        this.busy() || !this.database_available,
                                                    )
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.session = None;
                                                            this.connect(window, cx);
                                                        },
                                                    )),
                                            );
                                        if let DocumentKind::Sql { editor } = &this.kind {
                                            let sql = editor.read(cx).value().to_string();
                                            menu.separator().item(
                                                menu_item("Copy SQL", IconName::Copy)
                                                    .on_click(move |_, _, cx| {
                                                        cx.write_to_clipboard(
                                                            ClipboardItem::new_string(sql.clone()),
                                                        )
                                                    }),
                                            )
                                        } else {
                                            menu
                                        }
                                    })
                                }
                            }),
                    ),
            )
            .child(div().flex_1().min_h_0().overflow_hidden().child(content))
            .when(!self.error.is_empty(), |view| {
                view.child(
                    div()
                        .id("document-error")
                        .role(Role::Alert)
                        .max_h(px(140.))
                        .overflow_y_scroll()
                        .p_3()
                        .text_sm()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().muted)
                        .text_color(cx.theme().danger)
                        .child(self.error.clone()),
                )
            })
    }
}
