use super::{
    Services,
    controls::{chip, choice, count_badge, empty_state, icon_button, menu_item, toolbar_divider},
    grid::{Grid, GridEvent},
    grid_search::{FindGrid, GridSearch},
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
    menu::{DropdownMenu, PopupMenu},
    resizable::{ResizableState, resizable_panel, v_resizable},
    tab::{Tab, TabBar},
    table::{ColumnSort, DataTable, TableEvent, TableState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{sync::Arc, time::Instant};

#[path = "generated_sql.rs"]
mod generated_sql;
#[path = "named_table_views.rs"]
mod named_table_views;
pub use generated_sql::OpenGeneratedSql;
#[path = "parameters.rs"]
mod parameters;
#[path = "table_views.rs"]
mod table_views;
#[cfg(all(test, feature = "test-support"))]
#[path = "document_tests.rs"]
mod tests;

actions!(
    queri_document,
    [
        RunQuery,
        CancelQuery,
        Refresh,
        InspectCell,
        EditRow,
        CopyCells,
        FormatSql,
        BeginTransaction,
        CommitTransaction,
        RollbackTransaction
    ]
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
        columns: Vec<ViewColumn>,
        columns_open: bool,
        saved_view: Option<Box<SavedTableView>>,
        filter_draft: Option<Box<FilterExpression>>,
        filter_target: Vec<usize>,
        pending_filter: Option<Box<Option<FilterExpression>>>,
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
    TimedOut,
}

pub struct DocumentChanged;
struct TimeoutValidation(String);

impl Render for TimeoutValidation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("timeout-validation")
            .test_support()
            .role(Role::Alert)
            .aria_label(self.0.clone())
            .text_color(cx.theme().danger)
            .child(self.0.clone())
    }
}

impl EventEmitter<DocumentChanged> for Document {}
pub struct OpenRelated(pub RelationshipScope);
impl EventEmitter<OpenRelated> for Document {}
pub struct OpenSqlFile(pub String);
impl EventEmitter<OpenSqlFile> for Document {}
pub struct ReconnectRequested;
impl EventEmitter<ReconnectRequested> for Document {}
pub struct NewDocumentQuery;
impl EventEmitter<NewDocumentQuery> for Document {}
pub struct ConnectionPasswordRequired;
impl EventEmitter<ConnectionPasswordRequired> for Document {}

pub struct Document {
    pub profile: ConnectionProfile,
    pub session: Option<Arc<Session>>,
    services: Services,
    password: Option<String>,
    kind: DocumentKind,
    grid: Entity<TableState<Grid>>,
    grid_search: Entity<GridSearch>,
    results: Vec<Arc<ResultSet>>,
    result_index: usize,
    operation: Operation,
    query_outcome: QueryOutcome,
    pub(super) transaction: TransactionStatus,
    transaction_started: Option<Instant>,
    session_generation: u64,
    pub(super) releasing: bool,
    cancel_pending: bool,
    status: String,
    error: String,
    database_available: bool,
    pub warn_destructive: bool,
    pub editor_height: f32,
    pub query_number: u32,
    pub saved_query: Option<SavedQuery>,
    pub statement_timeout: StatementTimeout,
    timeout_input: Entity<InputState>,
    applied_browse: Option<BrowseOptions>,
    parameter_definitions: Vec<crate::parameters::Definition>,
    pub(super) parameter_seed: Option<crate::parameters::BoundQuery>,
    formatting: bool,
    completion: std::rc::Rc<super::completion::SqlCompletion>,
    subscriptions: Vec<Subscription>,
}

impl Document {
    pub fn init(cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new(
                "ctrl-space",
                gpui_kit::component::input::ShowCompletions,
                Some("QueriDocument > Input && mode == editor"),
            ),
            KeyBinding::new("ctrl-f", FindGrid, Some("QueriDocument")),
            KeyBinding::new("ctrl-f", FindGrid, Some("QueriDocument > DataTable")),
            KeyBinding::new("ctrl-shift-f", FormatSql, Some("QueriDocument")),
            KeyBinding::new("ctrl-shift-f", FormatSql, Some("QueriDocument > Input")),
            KeyBinding::new("ctrl-c", CopyCells, Some("QueriDocument > DataTable")),
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
        password: Option<String>,
        table: Option<TableRef>,
        sql: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let completion = std::rc::Rc::new(super::completion::SqlCompletion::new(
            services.runtime.clone(),
        ));
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
                columns: vec![],
                columns_open: false,
                saved_view: None,
                filter_draft: None,
                filter_target: vec![],
                pending_filter: None,
            },
            None => DocumentKind::Sql {
                editor: cx.new(|cx| {
                    let mut editor = EditorState::new(window, cx)
                        .language("sql")
                        .searchable(true)
                        .default_value(sql)
                        .placeholder("Write SQL, then press Ctrl+Enter");
                    editor.lsp_mut().completion_provider = Some(completion.clone());
                    editor
                }),
            },
        };
        let grid = cx.new(|cx| {
            TableState::new(Grid::new(Arc::new(ResultSet::default())), window, cx)
                .cell_selectable(true)
                .range_selectable(true)
                .loop_selection(false)
        });
        let grid_search = cx.new(|cx| GridSearch::new(grid.clone(), window, cx));
        let mut subscriptions = vec![
            cx.subscribe_in(&grid, window, |this, _, event: &GridEvent, window, cx| {
                if this.busy() {
                    return;
                }
                let GridEvent::Sort(column, sort) = event;
                if let DocumentKind::Table { options, .. } = &mut this.kind {
                    if let Some(index) = options.sort.iter().position(|(name, _)| name == column) {
                        if *sort == ColumnSort::Default {
                            options.sort.remove(index);
                        } else {
                            options.sort[index].1 = *sort == ColumnSort::Descending;
                        }
                    } else if *sort != ColumnSort::Default {
                        options
                            .sort
                            .push((column.clone(), *sort == ColumnSort::Descending));
                    }
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
            subscriptions.push(cx.subscribe(editor, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.parameter_seed = None;
                    this.parameter_definitions = this.parameter_definitions(cx);
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
            grid_search,
            results: vec![],
            result_index: 0,
            operation: Operation::Idle,
            query_outcome: QueryOutcome::NotRun,
            transaction: TransactionStatus::Idle,
            transaction_started: None,
            session_generation: 0,
            releasing: false,
            cancel_pending: false,
            status: "Connecting…".into(),
            error: String::new(),
            database_available: true,
            warn_destructive: true,
            editor_height: default_editor_height(),
            query_number: 1,
            saved_query: None,
            statement_timeout: StatementTimeout::Inherit,
            timeout_input: cx.new(|cx| InputState::new(window, cx).placeholder("Milliseconds")),
            applied_browse: None,
            parameter_definitions: vec![],
            parameter_seed: None,
            formatting: false,
            completion,
            subscriptions,
        }
    }

    pub(super) fn sql_editor(&self) -> Option<Entity<EditorState>> {
        match &self.kind {
            DocumentKind::Sql { editor } => Some(editor.clone()),
            _ => None,
        }
    }

    pub fn title(&self) -> String {
        match &self.kind {
            DocumentKind::Sql { .. } => self
                .saved_query
                .as_ref()
                .map(|q| q.name.clone())
                .unwrap_or_else(|| format!("Query {}", self.query_number)),
            DocumentKind::Table { table, .. } => format!("{}.{}", table.schema, table.name),
        }
    }
    pub fn relationship(&self) -> Option<&RelationshipScope> {
        match &self.kind {
            DocumentKind::Table { options, .. } => options.relationship.as_ref(),
            _ => None,
        }
    }
    pub fn set_relationship(&mut self, scope: Option<RelationshipScope>) {
        if let DocumentKind::Table { options, .. } = &mut self.kind {
            options.relationship = scope;
            options.offset = 0;
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
            parameters: self.parameter_definitions(cx),
            saved_table_view: match &self.kind {
                DocumentKind::Table { saved_view, .. } => saved_view.as_deref().cloned(),
                _ => None,
            },
            table_view: self.table_view_settings(),
            statement_timeout: self.statement_timeout,
            relationship: self.relationship().cloned(),
            saved_query: self.saved_query.clone(),
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
        self.operation != Operation::Idle || self.releasing || self.cancel_pending
    }
    pub fn set_password(&mut self, password: String) {
        self.password = Some(password);
    }
    pub fn reconfigure(
        &mut self,
        profile: &ConnectionProfile,
        password: String,
        cx: &mut Context<Self>,
    ) {
        self.profile = profile.for_database(&self.profile.database());
        self.password = Some(password);
        self.applied_browse = None;
        self.parameter_seed = None;
        self.session_generation += 1;
        self.operation = Operation::Idle;
        self.cancel_pending = false;
        self.session = None;
        self.completion.set_session(None);
        if let Some(editor) = self.sql_editor() {
            editor.update(cx, |editor, cx| editor.dismiss_lsp_overlays(cx));
        }
        self.transaction = TransactionStatus::Unknown;
        self.transaction_started = None;
        self.database_available = true;
    }
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.applied_browse = None;
        self.parameter_seed = None;
        self.session_generation += 1;
        self.operation = Operation::Idle;
        self.cancel_pending = false;
        self.session = None;
        self.completion.set_session(None);
        if let Some(editor) = self.sql_editor() {
            editor.update(cx, |editor, cx| editor.dismiss_lsp_overlays(cx));
        }
        self.transaction = TransactionStatus::Unknown;
        self.transaction_started = None;
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
        let password = match self.password.clone() {
            Some(password) => password,
            None if matches!(self.profile.config, ConnectionConfig::Postgres { .. }) => {
                cx.emit(ConnectionPasswordRequired);
                return;
            }
            None => String::new(),
        };
        self.operation = Operation::Connecting;
        self.error.clear();
        self.status = "Connecting…".into();
        let profile = self.profile.clone();
        self.applied_browse = None;
        self.parameter_seed = None;
        self.session_generation += 1;
        let generation = self.session_generation;
        task(
            &self.services,
            window,
            cx,
            async move {
                let session = Session::connect(profile, password).await?;
                let state = session
                    .transaction_status()
                    .await
                    .unwrap_or(TransactionStatus::Unknown);
                Ok((session, state))
            },
            move |this, result, window, cx| {
                if this.session_generation != generation {
                    return;
                }
                this.operation = Operation::Idle;
                match result {
                    Ok((session, state)) => {
                        if let Err(error) = session.set_statement_timeout(this.statement_timeout) {
                            this.error = error.to_string();
                        }
                        this.completion.set_session(Some(session.clone()));
                        this.session = Some(session);
                        this.observe_transaction(state, cx);
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
        if matches!(self.kind, DocumentKind::Sql { .. }) {
            if let Some(editor) = self.sql_editor() {
                editor.update(cx, |editor, cx| editor.dismiss_lsp_overlays(cx));
            }
            session.invalidate_completions();
            let generation = self.session_generation;
            task(
                &self.services,
                window,
                cx,
                async move { Ok(session.completion_catalog(vec![]).await) },
                move |this, result, _, cx| {
                    if this.session_generation == generation {
                        if let Ok(catalog) = result {
                            *this.completion.status.borrow_mut() = catalog.status;
                        }
                        cx.notify();
                    }
                },
            );
            return;
        }
        let DocumentKind::Table {
            table,
            options,
            pending_filter,
            ..
        } = &self.kind
        else {
            return;
        };
        let table = table.clone();
        let mut options = options.as_ref().clone();
        if let Some(filter) = pending_filter {
            options.filter = (**filter).clone();
            options.offset = 0;
        }
        let filtered = options.filter.is_some();
        let applied_options = options.clone();
        self.operation = Operation::Running;
        self.error.clear();
        self.status = "Loading table…".into();
        let generation = self.session_generation;
        task(
            &self.services,
            window,
            cx,
            async move {
                let result = session.browse(table.clone(), options).await;
                let repair_metadata = if result.is_err() {
                    session.table_metadata(table).await.ok()
                } else {
                    None
                };
                let state = session
                    .transaction_status()
                    .await
                    .unwrap_or(TransactionStatus::Unknown);
                Ok((result, state, repair_metadata))
            },
            move |this, observed, _, cx| {
                if this.session_generation != generation {
                    return;
                }
                let (result, state, repair_metadata) = match observed {
                    Ok(pair) => pair,
                    Err(error) => (Err(error), TransactionStatus::Unknown, None),
                };
                this.observe_transaction(state, cx);
                this.operation = Operation::Idle;
                match result {
                    Ok(page) => {
                        this.applied_browse = Some(applied_options);
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
                            options,
                            pending_filter,
                            filter_column,
                            filter_operator,
                            ..
                        } = &mut this.kind
                        {
                            if let Some(filter) = pending_filter.take() {
                                options.filter = *filter;
                                options.offset = 0;
                            }
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
                        cx.emit(DocumentChanged);
                    }
                    Err(error) => {
                        if let (Some(metadata), DocumentKind::Table { page, .. }) =
                            (repair_metadata, &mut this.kind)
                        {
                            let same_schema = page.as_ref().is_some_and(|page| {
                                page.metadata
                                    .columns
                                    .iter()
                                    .map(|col| (&col.name, &col.data_type))
                                    .eq(metadata
                                        .columns
                                        .iter()
                                        .map(|col| (&col.name, &col.data_type)))
                            });
                            if !same_schema {
                                *page = Some(Box::new(TablePage {
                                    data: ResultSet {
                                        columns: metadata
                                            .columns
                                            .iter()
                                            .map(|column| column.name.clone())
                                            .collect(),
                                        ..Default::default()
                                    },
                                    total: 0,
                                    metadata,
                                }));
                                this.update_grid(cx);
                            }
                        }
                        if let DocumentKind::Table { pending_filter, .. } = &mut this.kind {
                            pending_filter.take();
                        }
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
        self.grid.clone().update(cx, |state, cx| {
            let grid = state.delegate_mut();
            grid.replace(data);
            if let DocumentKind::Table {
                section: Section::Data,
                ..
            } = &self.kind
                && let Some(settings) = self.table_view_settings()
            {
                match settings.display_columns(&grid.data.columns) {
                    Ok(columns) => grid.display_columns = columns,
                    Err(error) => {
                        grid.display_columns.clear();
                        self.error = error.to_string();
                    }
                }
            }
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
                DocumentKind::Table { options, .. } => options
                    .sort
                    .iter()
                    .map(|(column, descending)| {
                        (
                            column.clone(),
                            if *descending {
                                ColumnSort::Descending
                            } else {
                                ColumnSort::Ascending
                            },
                        )
                    })
                    .collect(),
                _ => vec![],
            };
            state.clear_selection(cx);
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
        self.execute(sql, window, cx);
    }

    fn execute(&mut self, sql: String, window: &mut Window, cx: &mut Context<Self>) {
        match crate::parameters::discover(&sql, self.parameter_dialect()) {
            Ok(parameters) if !parameters.names.is_empty() => {
                self.enter_parameters(sql, parameters, window, cx)
            }
            Ok(_) => self.submit_sql(sql, None, window, cx),
            Err(error) => {
                self.error = format!("{error:#}");
                cx.notify();
            }
        }
    }

    // The parameter-entry UI must use this same submission boundary.
    fn submit_sql(
        &mut self,
        sql: String,
        params: Option<Vec<Cell>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() || self.session.is_none() {
            return;
        }
        let destructive = self.warn_destructive && sql::destructive(&sql);
        if self.profile.policy == ExecutionPolicy::ConfirmExecutions || destructive {
            let view = cx.entity();
            let generation = self.session_generation;
            let editor_sql = self.submission_snapshot(cx);
            let context = self.profile.context();
            let preview: String = sql.chars().take(2000).collect();
            let preview = if preview.len() < sql.len() {
                format!("{preview}… (preview truncated)")
            } else {
                preview
            };
            let message = format!(
                "Submit SQL to {context}. Cancel sends nothing to the database.\n\n{preview}"
            );
            window.open_alert_dialog(cx, move |dialog, _, _| {
                let sql = sql.clone();
                let params = params.clone();
                let view = view.clone();
                let editor_sql = editor_sql.clone();
                dialog
                    .title(if destructive {
                        "Run destructive SQL?"
                    } else {
                        "Run SQL?"
                    })
                    .child(
                        div()
                            .id("sql-confirmation-context")
                            .test_support()
                            .aria_label(message.clone())
                            .max_h(px(240.))
                            .overflow_y_scroll()
                            .child(message.clone()),
                    )
                    .button_props(
                        DialogButtonProps::default()
                            .ok_text("Run SQL")
                            .show_cancel(true),
                    )
                    .on_ok(move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            if this.session_generation == generation
                                && this.submission_snapshot(cx) == editor_sql
                            {
                                this.execute_confirmed(sql.clone(), params.clone(), window, cx);
                            }
                        });
                        true
                    })
            });
        } else {
            self.execute_confirmed(sql, params, window, cx);
        }
    }

    fn execute_confirmed(
        &mut self,
        sql: String,
        params: Option<Vec<Cell>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let generation = self.session_generation;
        let timeout = self.statement_timeout;
        if let Some(editor) = self.sql_editor() {
            editor.update(cx, |editor, cx| editor.dismiss_lsp_overlays(cx));
        }
        self.operation = Operation::Running;
        self.status = "Executing…".into();
        self.error.clear();
        task(
            &self.services,
            window,
            cx,
            async move {
                let start = Instant::now();
                let parameterized = params.is_some();
                let result = match session.set_statement_timeout(timeout) {
                    Ok(()) => match params {
                        Some(values) => {
                            session
                                .query_params(crate::parameters::BoundQuery {
                                    sql: sql.clone(),
                                    values,
                                })
                                .await
                        }
                        None => session.query(sql.clone()).await,
                    },
                    Err(error) => Err(error),
                };
                let transaction = session
                    .transaction_status()
                    .await
                    .unwrap_or(TransactionStatus::Unknown);
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
                    error: result.as_ref().err().map(|error| if parameterized { if crate::database::is_statement_timeout(error) { "Statement timed out".into() } else { "Parameterized query failed. Details were displayed only in the live tab.".into() } } else { format!("{error:#}") }),
                };
                let history_error = tokio::task::spawn_blocking(move || store.record(&entry))
                    .await?
                    .err()
                    .map(|error| format!("{error:#}"));
                Ok((result, history_error, transaction))
            },
            move |this, result, _, cx| {
                if this.session_generation != generation {
                    return;
                }
                let transaction = result
                    .as_ref()
                    .map(|(_, _, state)| *state)
                    .unwrap_or(TransactionStatus::Unknown);
                this.observe_transaction(transaction, cx);
                let cancelled = this.operation == Operation::Cancelling;
                this.operation = Operation::Idle;
                match result {
                    Ok((Ok(output), history_error, _)) => {
                        this.query_outcome = QueryOutcome::Succeeded;
                        this.status = format!(
                            "Completed in {} ms{}",
                            output.elapsed_ms,
                            if output.sets.len() > 1 {
                                format!(" · {} results", output.sets.len())
                            } else {
                                String::new()
                            }
                        );
                        if let Some(error) = history_error {
                            this.status
                                .push_str(&format!(" · History could not be saved: {error}"));
                        }
                        this.results = output.sets.into_iter().map(Arc::new).collect();
                        this.result_index = 0;
                        this.update_grid(cx);
                    }
                    Ok((Err(error), _, _)) | Err(error) => {
                        let timed_out = crate::database::is_statement_timeout(&error);
                        this.query_outcome = if timed_out {
                            QueryOutcome::TimedOut
                        } else if cancelled {
                            QueryOutcome::Cancelled
                        } else {
                            QueryOutcome::Failed
                        };
                        this.error = format!(
                            "{error:#}\nReview your SQL and run it again when you are ready."
                        );
                        this.status = if timed_out {
                            "Statement timed out"
                        } else if cancelled {
                            "Query cancelled"
                        } else {
                            "Query failed"
                        }
                        .into();
                    }
                }
                if transaction == TransactionStatus::Unknown {
                    this.query_outcome = QueryOutcome::Failed;
                    this.status = "Transaction outcome unknown".into();
                    this.error = "Connection lost or transaction status unavailable. The last operation may have taken effect, including COMMIT. Queri has not retried it. Verify the outcome before running it again.".into();
                }
                cx.emit(DocumentChanged);
                cx.notify();
            },
        );
        cx.notify();
    }

    pub(super) fn observe_transaction(&mut self, state: TransactionStatus, cx: &mut Context<Self>) {
        self.transaction = state;
        match state {
            TransactionStatus::Open | TransactionStatus::Failed
                if self.transaction_started.is_none() =>
            {
                self.transaction_started = Some(Instant::now());
                let generation = self.session_generation;
                let started = self.transaction_started;
                let runtime = self.services.runtime.clone();
                cx.spawn(async move |view, cx| {
                    loop {
                        let _ = runtime
                            .spawn(async {
                                tokio::time::sleep(std::time::Duration::from_secs(1)).await
                            })
                            .await;
                        let keep = view
                            .update(cx, |this, cx| {
                                cx.notify();
                                this.session_generation == generation
                                    && this.transaction_started == started
                            })
                            .unwrap_or(false);
                        if !keep {
                            break;
                        }
                    }
                })
                .detach();
            }
            TransactionStatus::Idle => self.transaction_started = None,
            _ => {}
        }
        cx.notify();
    }

    fn transaction_command(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        let allowed = match command {
            "BEGIN" => self.transaction == TransactionStatus::Idle,
            "COMMIT" => self.transaction == TransactionStatus::Open,
            "ROLLBACK" => matches!(
                self.transaction,
                TransactionStatus::Open | TransactionStatus::Failed
            ),
            _ => false,
        };
        if allowed && !self.busy() && self.session.is_some() {
            if command == "ROLLBACK" {
                // Recovery must remain available without another confirmation.
                self.execute_confirmed(command.into(), None, window, cx);
            } else {
                self.execute(command.into(), window, cx);
            }
        }
    }

    fn reconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        cx.emit(ReconnectRequested);
    }

    fn set_timeout(&mut self, timeout: StatementTimeout, cx: &mut Context<Self>) {
        if self.busy() || self.transaction == TransactionStatus::Failed {
            return;
        }
        self.statement_timeout = timeout;
        cx.emit(DocumentChanged);
        cx.notify();
    }

    fn timeout_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.busy() || self.transaction == TransactionStatus::Failed;
        let mode = match self.statement_timeout {
            StatementTimeout::Inherit => 0,
            StatementTimeout::NoTimeout => 1,
            StatementTimeout::Custom(_) => 2,
        };
        h_flex().gap_2()
            .child(choice(
                Button::new("statement-timeout").ghost().small().icon(IconName::Timer).label(match self.statement_timeout {
                    StatementTimeout::Inherit => "Timeout: inherit".to_string(),
                    StatementTimeout::NoTimeout => "Timeout: none".to_string(),
                    StatementTimeout::Custom(ms) => format!("Timeout: {ms} ms"),
                }).disabled(disabled)
                .tooltip("Per statement. Inherit uses the initial connection policy; None removes the application limit. Server policies may still apply. Recover a failed transaction before changing this setting."),
                mode,
                [(0, "Inherit connection/server".into()), (1, "No application timeout".into()), (2, "Custom duration…".into())],
                cx.listener(|this, mode, window, cx| {
                    if this.busy() || this.transaction == TransactionStatus::Failed { return; }
                    match mode {
                        0 => this.set_timeout(StatementTimeout::Inherit, cx),
                        1 => this.set_timeout(StatementTimeout::NoTimeout, cx),
                        _ => {
                            let value = match this.statement_timeout { StatementTimeout::Custom(ms) => ms.to_string(), _ => "30000".into() };
                            this.timeout_input.update(cx, |input, cx| input.set_value(value, window, cx));
                            let input = this.timeout_input.clone();
                            let view = cx.entity().downgrade();
                            let validation = cx.new(|_| TimeoutValidation(String::new()));
                            window.open_alert_dialog(cx, move |dialog, _, _| {
                                let input = input.clone();
                                let view = view.clone();
                                let validation = validation.clone();
                                dialog.title("Statement timeout")
                                    .child("Positive duration in milliseconds (maximum 2147483647). Applies separately to each statement.")
                                    .child(Input::new(&input))
                                    .child(validation.clone())
                                    .button_props(DialogButtonProps::default().ok_text("Apply").show_cancel(true)).on_ok(move |_, window, cx| {
                                        let timeout = input.read(cx).value().parse::<u32>().ok().and_then(|ms| StatementTimeout::custom(ms).ok());
                                        let Some(timeout) = timeout else {
                                            validation.update(cx, |message, cx| {
                                                message.0 = "Enter a whole number from 1 to 2147483647 milliseconds.".into();
                                                cx.notify();
                                            });
                                            input.read(cx).focus_handle(cx).focus(window, cx);
                                            return false;
                                        };
                                        let _ = view.update(cx, |this, cx| this.set_timeout(timeout, cx));
                                        true
                                    })
                            });
                        }
                    }
                }),
            )).into_any_element()
    }

    fn transaction_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let (label, tone) = match self.transaction {
            TransactionStatus::Idle if self.session.is_some() => ("Autocommit", cx.theme().success),
            TransactionStatus::Idle => ("Disconnected", cx.theme().muted_foreground),
            TransactionStatus::Open => ("Transaction open", cx.theme().warning),
            TransactionStatus::Failed => ("Transaction failed", cx.theme().danger),
            TransactionStatus::Unknown => ("Transaction outcome unknown", cx.theme().danger),
        };
        let elapsed = self
            .transaction_started
            .map(|start| format!(" · {}s", start.elapsed().as_secs()))
            .unwrap_or_default();
        let detail = match self.transaction {
            TransactionStatus::Idle => {
                "Each statement commits on its own. Begin opens a transaction on this tab's session."
            }
            TransactionStatus::Open => {
                "Changes stay on this tab's session until you commit or roll back."
            }
            TransactionStatus::Failed => "Roll back or recover a savepoint before continuing.",
            TransactionStatus::Unknown => {
                "The connection was lost. Verify the result before repeating the operation."
            }
        };
        let unavailable = self.busy() || self.session.is_none();
        h_flex()
            .gap_1()
            .flex_wrap()
            .child(
                h_flex()
                    .id("transaction-status")
                    .h_7()
                    .px_2p5()
                    .gap_1p5()
                    .rounded_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .aria_label(format!("{label}{elapsed}"))
                    .child(div().size_1p5().rounded_full().bg(tone))
                    .child(format!("{label}{elapsed}"))
                    .tooltip(move |window, cx| Tooltip::new(detail).build(window, cx)),
            )
            .when(
                self.profile.policy == ExecutionPolicy::ReadOnlyByDefault,
                |view| {
                    let observed = self.session.as_ref().is_some_and(|session| session.read_write_observed());
                    view.child(
                        h_flex()
                            .id("read-only-policy")
                            .h_7()
                            .px_2p5()
                            .gap_1p5()
                            .rounded_full()
                            .bg(cx.theme().muted)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .aria_label(if observed {
                                "Read-write override observed · generated changes blocked"
                            } else {
                                "Read-only by default · SQL can override · generated changes blocked"
                            })
                            .child(Icon::new(IconName::Lock).xsmall())
                            .child(if observed { "Read-write override" } else { "Read-only" })
                            .tooltip(move |window, cx| Tooltip::new(if observed {
                                "Read-write override observed · generated changes blocked"
                            } else {
                                "Read-only by default · SQL can override · generated changes blocked"
                            }).build(window, cx)),
                    )
                },
            )
            .when(self.transaction == TransactionStatus::Idle, |row| {
                row.child(
                    Button::new("begin-transaction")
                        .ghost()
                        .small()
                        .label("Begin")
                        .tooltip("Begin a transaction")
                        .disabled(unavailable)
                        .on_click(cx.listener(|this, _, window, cx| this.transaction_command("BEGIN", window, cx))),
                )
            })
            .when(self.transaction == TransactionStatus::Open, |row| {
                row.child(
                    Button::new("commit-transaction")
                        .outline()
                        .small()
                        .icon(IconName::Check)
                        .label("Commit")
                        .disabled(unavailable)
                        .on_click(cx.listener(|this, _, window, cx| this.transaction_command("COMMIT", window, cx))),
                )
            })
            .when(
                matches!(self.transaction, TransactionStatus::Open | TransactionStatus::Failed),
                |row| {
                    row.child(
                        Button::new("rollback-transaction")
                            .ghost()
                            .small()
                            .icon(IconName::Undo2)
                            .label("Roll back")
                            .accessibility_label("Rollback")
                            .disabled(unavailable)
                            .on_click(cx.listener(|this, _, window, cx| this.transaction_command("ROLLBACK", window, cx))),
                    )
                },
            )
            .child(self.timeout_controls(cx))
            .into_any_element()
    }

    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.operation != Operation::Running {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.operation = Operation::Cancelling;
        self.cancel_pending = true;
        let generation = self.session_generation;
        self.status = "Cancelling…".into();
        task(
            &self.services,
            window,
            cx,
            async move { session.cancel().await },
            move |this, result, _, cx| {
                if this.session_generation != generation {
                    return;
                }
                this.cancel_pending = false;
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

    fn related_rows(&mut self, incoming: bool, window: &mut Window, cx: &mut Context<Self>) {
        let DocumentKind::Table {
            table,
            page: Some(page),
            section: Section::Data,
            ..
        } = &self.kind
        else {
            return;
        };
        let Some(index) = self.selected_row(cx) else {
            return;
        };
        let Some(row) = page.data.rows.get(index).cloned() else {
            return;
        };
        let columns = page.data.columns.clone();
        let table = table.clone();
        let Some(session) = self.session.clone() else {
            return;
        };
        self.operation = Operation::Running;
        self.status = "Loading relationships…".into();
        task(
            &self.services,
            window,
            cx,
            async move { session.foreign_keys(table, incoming).await },
            move |this, result, window, cx| {
                this.operation = Operation::Idle;
                match result {
                    Err(e) => this.error = format!("Could not load relationships: {e:#}"),
                    Ok(keys) => {
                        let scopes: Vec<_> = keys
                            .into_iter()
                            .filter_map(|key| {
                                let values: Option<Vec<_>> = key
                                    .columns
                                    .iter()
                                    .map(|pair| {
                                        let name =
                                            if incoming { &pair.target } else { &pair.source };
                                        columns
                                            .iter()
                                            .position(|c| c == name)
                                            .and_then(|i| row.get(i).cloned())
                                            .filter(|v| !matches!(v, Cell::Null))
                                    })
                                    .collect();
                                values.map(|values| RelationshipScope {
                                    key,
                                    incoming,
                                    values,
                                })
                            })
                            .collect();
                        if scopes.is_empty() {
                            this.status =
                                "No relationships with a complete key for this row.".into();
                        } else {
                            this.status.clear();
                            let view = cx.entity().downgrade();
                            window.open_dialog(cx,move |dialog,_,cx| {
                                dialog.title(if incoming {"Find referencing rows"} else {"Open referenced row"})
                                    .child(v_flex().gap_3()
                                        .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Opens a separate table tab. Uncommitted changes in this tab are not visible there."))
                                        .child(v_flex().id("relationships").max_h(px(360.)).overflow_y_scroll().gap_2()
                                            .children(scopes.iter().enumerate().map(|(index,scope)| {
                                                let view=view.clone();let scope=scope.clone();
                                                let names=scope.key.columns.iter().map(|p|format!("{} → {}",p.source,p.target)).collect::<Vec<_>>().join(", ");
                                                Button::new(("relationship",index)).label(format!("{}.{} · {} · {names}",scope.destination().schema,scope.destination().name,scope.key.name))
                                                    .on_click(move |_,window,cx| {
                                                        window.close_dialog(cx);
                                                        let _=view.update(cx,|_,cx|cx.emit(OpenRelated(scope.clone())));
                                                    })
                                            }))))
                            });
                        }
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn cell_selection(&self, cx: &App) -> Option<crate::export::Selection> {
        let state = self.grid.read(cx);
        let ((r1, c1), (r2, c2)) = state.selected_range()?;
        let skip = usize::from(state.delegate().checkable);
        if c2 < skip {
            return None;
        }
        Some(crate::export::Selection {
            rows: (r1..=r2).collect(),
            columns: (c1.max(skip) - skip..=c2 - skip)
                .filter_map(|index| state.delegate().display_columns.get(index).copied())
                .collect(),
        })
    }

    fn copy_cells(&mut self, format: crate::export::Format, headers: bool, cx: &mut Context<Self>) {
        let Some(selection) = self.cell_selection(cx) else {
            return;
        };
        let data = &self.grid.read(cx).delegate().data;
        if format == crate::export::Format::Tsv
            && !headers
            && selection.rows.len() == 1
            && selection.columns.len() == 1
        {
            if let Some(cell) = data
                .rows
                .get(selection.rows[0])
                .and_then(|r| r.get(selection.columns[0]))
            {
                cx.write_to_clipboard(ClipboardItem::new_string(cell.text()));
            }
            return;
        }
        let mut bytes = Vec::new();
        match crate::export::serialize(data, &selection, format, headers, "\\N", &mut bytes)
            .and_then(|()| Ok(String::from_utf8(bytes)?))
        {
            Ok(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            Err(error) => self.error = format!("Could not copy selection: {error:#}"),
        }
        cx.notify();
    }

    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.grid.read(cx);
        let grid = state.delegate();
        let data = Arc::new(grid.displayed_data());
        let rows: Vec<_> = if grid.checked.is_empty() {
            state.selected_row().into_iter().collect()
        } else {
            grid.checked.iter().copied().collect()
        };
        let selected_rows = (!rows.is_empty()).then(|| crate::export::Selection {
            rows,
            columns: (0..data.columns.len()).collect(),
        });
        let cells = self.cell_selection(cx).map(|mut selection| {
            selection.columns = selection
                .columns
                .iter()
                .filter_map(|source| {
                    grid.display_columns
                        .iter()
                        .position(|index| index == source)
                })
                .collect();
            selection
        });
        super::export_dialog::open(
            self.services.clone(),
            data,
            selected_rows,
            cells,
            window,
            cx,
        );
    }

    fn data_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_1()
            .child(
                Button::new("export-data")
                    .ghost()
                    .small()
                    .icon(IconName::Download)
                    .label("Export")
                    .accessibility_label("Export…")
                    .disabled(self.grid.read(cx).delegate().data.columns.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.export(window, cx))),
            )
            .child(
                icon_button("copy-selection", IconName::Copy, "Copy selected cells")
                    .small()
                    .disabled(self.cell_selection(cx).is_none())
                    .dropdown_menu({
                        let view = cx.entity();
                        move |menu, _, cx| {
                            view.update(cx, |_, cx| {
                                menu.item(menu_item("Copy cells", IconName::Copy).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.copy_cells(crate::export::Format::Tsv, false, cx)
                                    }),
                                ))
                                .item(menu_item("Copy with headers", IconName::Copy).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.copy_cells(crate::export::Format::Tsv, true, cx)
                                    }),
                                ))
                                .item(
                                    menu_item("Copy as JSON", IconName::Copy).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.copy_cells(crate::export::Format::Json, true, cx)
                                        }),
                                    ),
                                )
                            })
                        }
                    }),
            )
            .into_any_element()
    }

    fn selected_row(&self, cx: &App) -> Option<usize> {
        let grid = self.grid.read(cx);
        if grid.delegate().checked.len() == 1 {
            return grid.delegate().checked.first().copied();
        }
        if grid.delegate().checked.is_empty()
            && grid
                .selected_range()
                .is_some_and(|(start, end)| start.0 != end.0)
        {
            return None;
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
        if let Some((_, col)) = grid.selected_cell() {
            let Some(index) = col
                .checked_sub(usize::from(grid.delegate().checkable))
                .and_then(|index| grid.delegate().display_columns.get(index).copied())
            else {
                return;
            };
            if let Some(cell) = row.get(index) {
                super::cell_inspector::open(
                    self.services.clone(),
                    cell.clone(),
                    columns
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| "Cell contents".into()),
                    match &self.kind {
                        DocumentKind::Table {
                            page: Some(page),
                            section: Section::Data,
                            ..
                        } => page
                            .metadata
                            .columns
                            .get(index)
                            .map(|c| c.data_type.clone()),
                        _ => None,
                    },
                    window,
                    cx,
                );
            }
            return;
        }
        let text = {
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
        if session.read_only() || !table.is_table() {
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
                    "Delete the selected rows from {}.{} on {}? Cancel makes no changes.",
                    table.schema,
                    table.name,
                    session.profile.context()
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
        let candidate = match &self.kind {
            DocumentKind::Table {
                filter_draft: Some(draft),
                ..
            } => Some((**draft).clone()),
            _ => self.filter_input(cx).map(Into::into),
        };
        if let DocumentKind::Table { pending_filter, .. } = &mut self.kind {
            *pending_filter = Some(Box::new(candidate));
        }
        self.refresh(window, cx);
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let DocumentKind::Table {
            options,
            filter,
            filter_draft,
            filter_target,
            ..
        } = &mut self.kind
        {
            filter.update(cx, |filter, cx| filter.set_value("", window, cx));
            options.filter = None;
            *filter_draft = None;
            filter_target.clear();
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
            && matches!(&self.kind, DocumentKind::Table { table, page: Some(_), .. } if table.is_table())
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
                let (title, description) = if !table.is_table() {
                    (
                        "This view has no rows",
                        "Views show data from their underlying tables. Refresh to check for changes.",
                    )
                } else if read_only {
                    (
                        "This table is empty",
                        "Connection policy blocks generated changes. Refresh to check for new rows.",
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
                    .when(read_only || !table.is_table(), |view| {
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
                let (title, description) = if *section == Section::Indexes && !table.is_table() {
                    (
                        "No indexes",
                        "No indexes are reported for this view. Inspect its underlying tables for index definitions.",
                    )
                } else if *section == Section::Constraints && !table.is_table() {
                    (
                        "No constraints",
                        "No constraints are reported for this view. Inspect its underlying tables for data rules.",
                    )
                } else if *section == Section::Indexes && read_only {
                    (
                        "No indexes",
                        "This table has no indexes. Connection policy blocks generated changes.",
                    )
                } else if read_only {
                    (
                        "No constraints",
                        "This table has no constraints. Connection policy blocks generated changes.",
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

    fn format_sql(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.formatting {
            return;
        }
        let DocumentKind::Sql { editor } = &self.kind else {
            return;
        };
        let editor = editor.clone();
        let original = editor.read(cx).value().to_string();
        let selection = editor.read(cx).selected_range();
        let range = if selection.is_empty() {
            0..original.len()
        } else {
            selection.clone()
        };
        let input = original[range.clone()].to_owned();
        let dialect = if matches!(self.profile.config, ConnectionConfig::Sqlite { .. }) {
            sql::SqlDialect::Sqlite
        } else {
            sql::SqlDialect::Postgres
        };
        self.formatting = true;
        self.error.clear();
        task(
            &self.services,
            window,
            cx,
            async move { tokio::task::spawn_blocking(move || sql::format_sql(&input, dialect)).await? },
            move |this, result, window, cx| {
                this.formatting = false;
                match result {
                    Ok(formatted) => {
                        if editor.read(cx).value().as_ref() != original
                            || editor.read(cx).selected_range() != selection
                        {
                            this.status =
                                "SQL changed while formatting. Format again to apply it.".into();
                        } else if original[range.clone()] != formatted {
                            editor.update(cx, |editor, cx| {
                                editor.set_selected_range(range.clone(), cx);
                                editor.replace(formatted.clone(), window, cx);
                                if !selection.is_empty() {
                                    editor.set_selected_range(
                                        range.start..range.start + formatted.len(),
                                        cx,
                                    );
                                }
                            });
                            cx.emit(DocumentChanged);
                            this.status = "SQL formatted · Undo to restore".into();
                        } else {
                            this.status = "SQL is already formatted".into();
                        }
                    }
                    Err(e) => this.error = format!("{e:#}"),
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn save_query(&mut self, new: bool, window: &mut Window, cx: &mut Context<Self>) {
        let sql = self.draft(cx).sql;
        let mut query = self
            .saved_query
            .clone()
            .filter(|_| !new)
            .unwrap_or_else(|| SavedQuery {
                parameters: vec![],
                id: uuid::Uuid::new_v4(),
                name: String::new(),
                sql: String::new(),
                favorite: false,
                connection_id: Some(self.profile.id),
                database: self.profile.database(),
                revision: 0,
            });
        query.parameters = self.parameter_definitions(cx);
        query.sql = sql;
        query.connection_id = Some(self.profile.id);
        query.database = self.profile.database();
        let form = super::saved_queries::edit(self.services.clone(), query, window, cx);
        self.subscriptions.push(cx.subscribe_in(
            &form,
            window,
            |this, _, event: &super::saved_queries::QuerySaved, _, cx| {
                this.saved_query = Some(event.0.clone());
                cx.emit(DocumentChanged);
                cx.notify();
            },
        ));
    }

    fn sql_file(&mut self, import: bool, window: &mut Window, cx: &mut Context<Self>) {
        let sql = self.draft(cx).sql;
        if import {
            let receiver = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Open SQL file".into()),
            });
            cx.spawn_in(window, async move |view, cx| {
                let result = receiver.await;
                let _ = view.update_in(cx, |this, window, cx| {
                    match result {
                        Ok(Ok(Some(paths))) => {
                            if let Some(path) = paths.into_iter().next() {
                                task(
                                    &this.services,
                                    window,
                                    cx,
                                    async move {
                                        Ok(tokio::task::spawn_blocking(move || {
                                            std::fs::read_to_string(path)
                                        })
                                        .await??)
                                    },
                                    |this, result, _, cx| {
                                        match result {
                                            Ok(sql) => cx.emit(OpenSqlFile(sql)),
                                            Err(e) => {
                                                this.error =
                                                    format!("Could not open UTF-8 SQL file: {e:#}")
                                            }
                                        }
                                        cx.notify();
                                    },
                                );
                            }
                        }
                        Ok(Err(e)) => this.error = format!("Could not open file picker: {e}"),
                        _ => {}
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            let receiver = cx.prompt_for_new_path(std::path::Path::new("."), Some("query.sql"));
            cx.spawn_in(window, async move |view, cx| {
                let result = receiver.await;
                let _ = view.update_in(cx, |this, window, cx| {
                    match result {
                        Ok(Ok(Some(path))) => task(
                            &this.services,
                            window,
                            cx,
                            async move {
                                tokio::task::spawn_blocking(move || {
                                    crate::export::atomic_write(&path, |file| {
                                        use std::io::Write;
                                        file.write_all(sql.as_bytes())?;
                                        Ok(())
                                    })
                                })
                                .await?
                            },
                            |this, result, _, cx| {
                                match result {
                                    Ok(()) => this.status = "SQL file saved".into(),
                                    Err(e) => this.error = format!("Could not save SQL: {e:#}"),
                                };
                                cx.notify();
                            },
                        ),
                        Ok(Err(e)) => this.error = format!("Could not open file picker: {e}"),
                        _ => {}
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn query_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let selection = matches!(&self.kind, DocumentKind::Sql { editor } if !editor.read(cx).selected_value().trim().is_empty());
        let running = matches!(self.operation, Operation::Running | Operation::Cancelling);
        h_flex()
            .flex_wrap()
            .gap_1()
            .child(
                Button::new("format-sql")
                    .ghost()
                    .small()
                    .icon(IconName::WandSparkles)
                    .label(if self.formatting {
                        "Formatting…"
                    } else if selection {
                        "Format selection"
                    } else {
                        "Format"
                    })
                    .disabled(self.formatting)
                    .tooltip("Format SQL · Ctrl+Shift+F")
                    .on_click(cx.listener(|this, _, window, cx| this.format_sql(window, cx))),
            )
            .child(
                Button::new("save-query")
                    .ghost()
                    .small()
                    .icon(IconName::Bookmark)
                    .label(if self.saved_query.is_some() {
                        "Save changes"
                    } else {
                        "Save"
                    })
                    .tooltip(if self.saved_query.is_some() {
                        "Update the saved query"
                    } else {
                        "Save to your query library"
                    })
                    .on_click(
                        cx.listener(|this, _, window, cx| this.save_query(false, window, cx)),
                    ),
            )
            .child(
                icon_button("query-files", IconName::Ellipsis, "Query actions")
                    .small()
                    .dropdown_menu({
                        let view = cx.entity();
                        move |menu, _, cx| {
                            view.update(cx, |this, cx| {
                                let status = this.completion.status.borrow().clone();
                                let menu = menu
                                    .min_w(px(224.))
                                    .label(status)
                                    .item(
                                        menu_item("Refresh completions", IconName::RefreshCw)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.refresh(window, cx)
                                            })),
                                    )
                                    .item(menu_item("Save as new query…", IconName::Plus).on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.save_query(true, window, cx)
                                        }),
                                    ))
                                    .separator()
                                    .item(menu_item("Open SQL file…", IconName::Folder).on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.sql_file(true, window, cx)
                                        }),
                                    ))
                                    .item(menu_item("Save SQL file…", IconName::File).on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.sql_file(false, window, cx)
                                        }),
                                    ))
                                    .separator();
                                this.session_menu(menu, cx)
                            })
                        }
                    }),
            )
            .child(toolbar_divider(cx))
            .when(running, |bar| {
                bar.child(
                    Button::new("cancel-query")
                        .outline()
                        .small()
                        .icon(IconName::Square)
                        .label(if self.operation == Operation::Cancelling {
                            "Cancelling…"
                        } else {
                            "Cancel"
                        })
                        .tooltip("Cancel query · Ctrl+Shift+C")
                        .disabled(self.operation != Operation::Running)
                        .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                )
            })
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
                    .loading(self.operation == Operation::Running)
                    .disabled(self.busy() || self.session.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.run(window, cx))),
            )
            .into_any_element()
    }

    /// Session and context actions shared by table and SQL documents.
    fn session_menu(&self, menu: PopupMenu, cx: &mut Context<Self>) -> PopupMenu {
        let context = format!(
            "{} / {}",
            self.profile.display_name(),
            self.profile.database()
        );
        let menu = menu
            .item(
                menu_item("New query on this database", IconName::Code)
                    .disabled(!self.database_available)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(NewDocumentQuery))),
            )
            .item(
                menu_item("Copy connection context", IconName::Copy).on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(context.clone()))
                }),
            )
            .item(
                menu_item("Reconnect session", IconName::RefreshCw)
                    .disabled(self.busy() || !self.database_available)
                    .on_click(cx.listener(|this, _, window, cx| this.reconnect(window, cx))),
            );
        if let DocumentKind::Sql { editor } = &self.kind {
            let sql = editor.read(cx).value().to_string();
            menu.separator()
                .item(
                    menu_item("Copy SQL", IconName::Copy).on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(sql.clone()))
                    }),
                )
        } else {
            menu
        }
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
        let outcome_color = if self.busy() {
            cx.theme().muted_foreground
        } else {
            match self.query_outcome {
                QueryOutcome::Succeeded => cx.theme().success,
                QueryOutcome::Failed | QueryOutcome::TimedOut => cx.theme().danger,
                QueryOutcome::Cancelled | QueryOutcome::NotRun => cx.theme().muted_foreground,
            }
        };
        h_flex()
            .id("result-tabs")
            .flex_shrink_0()
            .flex_wrap()
            .min_h_10()
            .px_4()
            .gap_2()
            .border_y_1()
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
                            .px_1p5()
                            .rounded(cx.theme().radius_tokens().sm)
                            .bg(cx.theme().secondary_hover)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
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
            .child(self.data_actions(cx))
            .child(
                h_flex()
                    .id("query-status")
                    .role(Role::Status)
                    .gap_1p5()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        Icon::new(if self.busy() {
                            IconName::LoaderCircle
                        } else {
                            match self.query_outcome {
                                QueryOutcome::Succeeded => IconName::CircleCheck,
                                QueryOutcome::Failed | QueryOutcome::TimedOut => {
                                    IconName::CircleAlert
                                }
                                QueryOutcome::Cancelled => IconName::CircleStop,
                                QueryOutcome::NotRun => IconName::CircleDashed,
                            }
                        })
                        .small()
                        .text_color(outcome_color),
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
                QueryOutcome::TimedOut => (
                    "Statement timed out",
                    "Adjust the timeout or SQL. Recover any failed transaction before continuing.",
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
                QueryOutcome::Failed | QueryOutcome::TimedOut => IconName::CircleAlert,
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
        let filter_count = options.filter.as_ref().map(filter_conditions).unwrap_or(0);
        let busy = self.busy();
        let data = *section == Section::Data;
        if !data {
            return div().into_any_element();
        }
        let toolbar = h_flex()
            .gap_1()
            .flex_wrap()
            .justify_between()
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        Button::new("toggle-filter")
                            .ghost()
                            .small()
                            .icon(IconName::ListFilter)
                            .accessibility_label(if options.filter.is_some() {
                                "Filter applied"
                            } else {
                                "Filter"
                            })
                            .selected(*filter_open)
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .child("Filter")
                                    .when(filter_count > 0, |row| {
                                        row.child(count_badge(filter_count, cx))
                                    }),
                            )
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
                                            filter.read(cx).focus_handle(cx).focus(window, cx);
                                        } else {
                                            filter_apply_focus.focus(window, cx);
                                        }
                                    }
                                }
                                cx.notify();
                            })),
                    )
                    .child(self.column_toggle(cx))
                    .child(self.named_view_controls(cx)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        icon_button("refresh-table", IconName::RefreshCw, "Refresh table · F5")
                            .small()
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                    )
                    .child(
                        icon_button(
                            "open-table-sql",
                            IconName::SquareCode,
                            "Open as SQL · the loaded page, including its limit and offset",
                        )
                        .small()
                        .disabled(busy || self.applied_browse.is_none())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_table_sql(window, cx)),
                        ),
                    )
                    .child(self.data_actions(cx))
                    .child(toolbar_divider(cx))
                    .child(
                        Button::new("row-actions")
                            .ghost()
                            .small()
                            .label(if selected > 1 {
                                format!("{selected} rows")
                            } else {
                                "Row actions".into()
                            })
                            .accessibility_label("Row actions")
                            .dropdown_caret(true)
                            .tooltip(format!(
                                "{selected} selected · Edit: Ctrl+E · Inspect: Alt+Enter"
                            ))
                            .disabled(busy || selected == 0)
                            .dropdown_menu({
                                let view = cx.entity();
                                move |menu, _, cx| {
                                    view.update(cx, |_, cx| {
                                        menu.item(
                                            menu_item("Inspect", IconName::ScanText)
                                                .disabled(selected != 1)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.inspect(window, cx)
                                                })),
                                        )
                                        .item(
                                            menu_item("Edit row…", IconName::Pencil)
                                                .disabled(!editable || selected != 1)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.edit_row(false, window, cx)
                                                })),
                                        )
                                        .item(
                                            menu_item("Filter by this value", IconName::ListFilter)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.filter_from_cell(window, cx)
                                                })),
                                        )
                                        .separator()
                                        .item(
                                            menu_item("Open referenced row…", IconName::ArrowRight)
                                                .disabled(selected != 1)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.related_rows(false, window, cx)
                                                })),
                                        )
                                        .item(
                                            menu_item("Find referencing rows…", IconName::Search)
                                                .disabled(selected != 1)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.related_rows(true, window, cx)
                                                })),
                                        )
                                        .separator()
                                        .item(
                                            super::controls::destructive_menu_item(
                                                if selected == 1 {
                                                    "Delete row…".to_owned()
                                                } else {
                                                    format!("Delete {selected} rows…")
                                                },
                                                !editable,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, window, cx| {
                                                    this.delete_rows(window, cx)
                                                }),
                                            ),
                                        )
                                    })
                                }
                            }),
                    )
                    .child(
                        Button::new("insert")
                            .small()
                            .outline()
                            .icon(IconName::Plus)
                            .label("Insert row")
                            .disabled(!writable)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.edit_row(true, window, cx)),
                            ),
                    ),
            );
        let applied = options
            .filter
            .as_ref()
            .filter(|_| !*filter_open)
            .map(|filter| {
                filter.summary(
                    &page
                        .as_ref()
                        .map(|page| page.metadata.clone())
                        .unwrap_or_default(),
                    sqlite,
                )
            });
        let has_chips =
            options.relationship.is_some() || applied.is_some() || !options.sort.is_empty();
        let chips =
            h_flex()
                .gap_1p5()
                .flex_wrap()
                .when_some(options.relationship.as_ref(), |row, scope| {
                    row.child(
                        chip(cx)
                            .child(
                                Icon::new(IconName::Link)
                                    .xsmall()
                                    .text_color(super::theme::accent_text(cx)),
                            )
                            .child(div().text_ellipsis().child(scope.label()))
                            .child(
                                icon_button(
                                    "clear-relationship",
                                    IconName::X,
                                    "Clear relationship filter",
                                )
                                .xsmall()
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.set_relationship(None);
                                        cx.emit(DocumentChanged);
                                        this.refresh(window, cx);
                                    },
                                )),
                            ),
                    )
                })
                .when_some(applied, |row, summary| {
                    let tooltip = summary.clone();
                    row.child(
                        chip(cx)
                            .max_w(px(560.))
                            .child(
                                Icon::new(IconName::ListFilter)
                                    .xsmall()
                                    .text_color(super::theme::accent_text(cx)),
                            )
                            .child(
                                div()
                                    .id("applied-filter")
                                    .min_w_0()
                                    .text_ellipsis()
                                    .child(summary)
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(tooltip.clone()).build(window, cx)
                                    }),
                            )
                            .child(
                                icon_button("clear-applied-filter", IconName::X, "Clear filter")
                                    .xsmall()
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.clear_filter(window, cx)
                                    })),
                            ),
                    )
                })
                .child(self.sort_controls(cx));
        let mut panel = v_flex()
            .flex_shrink_0()
            .px_4()
            .pb_2()
            .gap_2()
            .child(toolbar)
            .when(has_chips, |view| view.child(chips))
            .child(self.column_controls(cx));
        if *filter_open {
            let column_picker = choice(
                Button::new("filter-column")
                    .small()
                    .outline()
                    .w_full()
                    .disabled(busy || page.is_none())
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
                    .outline()
                    .disabled(busy || page.is_none())
                    .accessibility_label(format!(
                        "Filter operator: {}",
                        filter_operator.label(filter_kind)
                    ))
                    .label(filter_operator.label(filter_kind)),
                *filter_operator,
                filter_kind
                    .operators()
                    .iter()
                    .map(|operator| (*operator, SharedString::from(operator.label(filter_kind)))),
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
            panel = panel.child(
                v_flex()
                    .p_3()
                    .gap_3()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .child(self.filter_tree(cx))
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(div().w(px(160.)).max_w_full().child(column_picker))
                            .child(operator_picker)
                            .when(filter_operator.needs_value(), |row| {
                                row.child(
                                    div().flex_1().min_w(px(140.)).child(
                                        Input::new(filter)
                                            .id("filter-value")
                                            .small()
                                            .aria_label("Filter value"),
                                    ),
                                )
                            })
                            .child(
                                Button::new("add-filter-condition")
                                    .small()
                                    .outline()
                                    .icon(IconName::Plus)
                                    .label("Add condition")
                                    .accessibility_label("Add / replace condition")
                                    .tooltip("Adds to the selected group, or replaces the selected condition")
                                    .disabled(busy || page.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| this.add_filter_condition(cx))),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .when(options.filter.is_some(), |row| {
                                row.child(
                                    Button::new("clear-filter")
                                        .ghost()
                                        .small()
                                        .label("Clear filter")
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, _, window, cx| this.clear_filter(window, cx))),
                                )
                            })
                            .child(
                                Button::new("apply-filter")
                                    .track_focus(filter_apply_focus)
                                    .small()
                                    .primary()
                                    .label("Apply")
                                    .tooltip("Apply filter · Enter")
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| this.apply_filter(window, cx))),
                            ),
                    ),
            );
        }
        panel.into_any_element()
    }

    fn table_sections(&self, cx: &mut Context<Self>) -> AnyElement {
        let DocumentKind::Table { section, .. } = &self.kind else {
            return div().into_any_element();
        };
        TabBar::new("table-sections")
            .segmented()
            .small()
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
            }))
            .into_any_element()
    }

    fn read_only_reason(&self) -> Option<&'static str> {
        let DocumentKind::Table {
            table,
            page: Some(page),
            section: Section::Data,
            ..
        } = &self.kind
        else {
            return None;
        };
        if self.can_insert() && page.metadata.editable(table) {
            None
        } else if self
            .session
            .as_ref()
            .is_some_and(|session| session.read_only())
        {
            Some("Generated changes blocked by connection policy")
        } else if !table.is_table() {
            Some("Views are read-only")
        } else {
            Some("Editing and deleting rows requires a primary key")
        }
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
        let read_only = self.read_only_reason();
        h_flex()
            .gap_2()
            .px_4()
            .py_1p5()
            .flex_wrap()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
                            .child(summary),
                    )
                    .when_some(read_only, |row, reason| {
                        row.child(
                            h_flex()
                                .id("read-only-reason")
                                .min_w_0()
                                .gap_1()
                                .child(Icon::new(IconName::Lock).xsmall())
                                .child(div().text_ellipsis().child(reason)),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        icon_button("previous", IconName::ChevronLeft, "Previous page")
                            .small()
                            .disabled(self.busy() || options.offset == 0)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let DocumentKind::Table { options, .. } = &mut this.kind {
                                    options.offset =
                                        options.offset.saturating_sub(u64::from(options.limit));
                                }
                                this.refresh(window, cx);
                            })),
                    )
                    .child(div().text_xs().px_1().child(format!(
                        "Page {}",
                        options.offset / u64::from(options.limit) + 1,
                    )))
                    .child(
                        icon_button("next", IconName::ChevronRight, "Next page")
                            .small()
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

fn filter_conditions(filter: &FilterExpression) -> usize {
    match filter {
        FilterExpression::Predicate(_) => 1,
        FilterExpression::All(children) | FilterExpression::Any(children) => {
            children.iter().map(filter_conditions).sum()
        }
    }
}

impl Render for Document {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let context = format!(
            "{} / {}{}",
            self.profile.display_name(),
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
            .on_action(cx.listener(|this, _: &FindGrid, window, cx| if this.grid.read(cx).focus_handle(cx).is_focused(window) { this.grid_search.update(cx, |search, cx| search.open(window, cx)) } else { cx.propagate(); }))
            .on_action(cx.listener(|this, _: &FormatSql, window, cx| this.format_sql(window,cx)))
            .on_action(cx.listener(|this, _: &CopyCells, _, cx| this.copy_cells(crate::export::Format::Tsv, false,cx)))
            .on_action(cx.listener(|this, _: &BeginTransaction, window, cx| this.transaction_command("BEGIN", window, cx)))
            .on_action(cx.listener(|this, _: &CommitTransaction, window, cx| this.transaction_command("COMMIT", window, cx)))
            .on_action(cx.listener(|this, _: &RollbackTransaction, window, cx| this.transaction_command("ROLLBACK", window, cx)))
            .on_action(cx.listener(|this, _: &RunQuery, window, cx| this.run(window, cx)))
            .on_action(cx.listener(|this, _: &CancelQuery, window, cx| this.cancel(window, cx)))
            .on_action(cx.listener(|this, _: &Refresh, window, cx| this.refresh(window, cx)))
            .on_action(cx.listener(|this, _: &InspectCell, window, cx| this.inspect(window, cx)))
            .on_action(
                cx.listener(|this, _: &EditRow, window, cx| this.edit_row(false, window, cx)),
            )
            .child(
                h_flex()
                    .gap_x_4()
                    .gap_y_2()
                    .flex_shrink_0()
                    .flex_wrap()
                    .min_h(px(56.))
                    .px_4()
                    .pt_3()
                    .pb_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(160.))
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .min_w_0()
                                    .gap_2()
                                    .child(
                                        Icon::new(match self.table_ref() {
                                            Some(table) if table.is_table() => IconName::Table2,
                                            Some(_) => IconName::Eye,
                                            None => IconName::SquareCode,
                                        })
                                        .small()
                                        .text_color(super::theme::accent_text(cx)),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_base()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_ellipsis()
                                            .child(self.table_ref().map(|table| table.name.clone()).unwrap_or_else(|| self.title())),
                                    ),
                            )
                            .child(
                                div()
                                    .id("document-context").test_support()
                                    .aria_label(context.clone())
                                    .text_ellipsis()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(context.clone())
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(context.clone()).build(window, cx)
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .when(
                                self.session.is_none() && self.database_available && !self.busy(),
                                |bar| {
                                    bar.child(
                                        Button::new("reconnect")
                                            .outline()
                                            .small()
                                            .icon(IconName::RefreshCw)
                                            .label("Reconnect")
                                            .on_click(
                                                cx.listener(|this, _, window, cx| this.connect(window, cx)),
                                            ),
                                    )
                                },
                            )
                            .when(matches!(self.kind, DocumentKind::Sql { .. }), |bar| {
                                bar.child(self.transaction_controls(cx))
                                    .child(toolbar_divider(cx))
                                    .when(self.selected_row(cx).is_some(), |bar| {
                                        bar.child(
                                            icon_button(
                                                "inspect-cell",
                                                IconName::ScanText,
                                                "Inspect cell · Alt+Enter",
                                            )
                                            .small()
                                            .on_click(
                                                cx.listener(|this, _, window, cx| this.inspect(window, cx)),
                                            ),
                                        )
                                    })
                                    .child(self.query_actions(cx))
                            })
                            .when(matches!(self.kind, DocumentKind::Table { .. }), |bar| {
                                bar.child(self.table_sections(cx))
                                    .when(matches!(self.kind, DocumentKind::Table { section, .. } if section != Section::Data), |bar| {
                                        bar.child(
                                            icon_button("refresh-metadata", IconName::RefreshCw, "Refresh table · F5")
                                                .small()
                                                .disabled(self.busy())
                                                .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                                        )
                                    })
                                    .child(
                                        icon_button("document-menu", IconName::Ellipsis, "Document actions")
                                            .small()
                                            .dropdown_menu({
                                                let view = cx.entity();
                                                move |menu, _, cx| {
                                                    view.update(cx, |this, cx| this.session_menu(menu.min_w(px(224.)), cx))
                                                }
                                            }),
                                    )
                            }),
                    ),
            )
            .child(self.grid_search.clone())
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
