use super::{
    Services,
    admin::{AdminAction, AdminEvent, DatabaseAdmin},
    connection_form::{ConnectionForm, SavedConnection},
    connection_picker::{ConnectionPicker, ConnectionPickerEvent},
    controls::{
        brand_logo, brand_mark, destructive_menu_item, icon_button, menu_item, setting_switch,
        shortcut,
    },
    document::{Document, DocumentChanged, NewDocumentQuery},
    document_tabs::{document_tab, document_tab_bar},
    history::{History, NewHistoryQuery, ReplayQuery},
    table_finder::{OpenTable, TableFinder},
    task,
};
use crate::{database::Session, model::*, storage};
use gpui_kit::assets::IconName;
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, Theme, ThemeMode, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    list::ListItem,
    menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem},
    resizable::{h_resizable, resizable_panel},
    tab::{Tab, TabBar},
    tooltip::Tooltip,
    tree::{TreeEvent, TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

#[cfg(all(test, feature = "test-support"))]
#[path = "workspace_tests.rs"]
mod tests;

actions!(
    queri_workspace,
    [
        NewQuery,
        NewConnection,
        CloseDocument,
        ToggleHistory,
        OpenTreeItem,
        NextDocument,
        FindTable
    ]
);

#[derive(Clone)]
enum Node {
    Database(String),
    Schema { database: String, schema: String },
    Table { database: String, table: TableRef },
}

#[derive(Clone, Copy, PartialEq)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Ready,
    Failed,
}

struct DatabaseBranch {
    session: Arc<Session>,
    schemas: BTreeMap<String, Option<Vec<TableRef>>>,
}

pub struct Workspace {
    services: Services,
    preferences: Preferences,
    profiles: Vec<ConnectionProfile>,
    connection_picker: Entity<ConnectionPicker>,
    selected: Option<Uuid>,
    passwords: HashMap<Uuid, String>,
    generation: Uuid,
    connection_state: ConnectionState,
    databases: Vec<String>,
    branches: BTreeMap<String, DatabaseBranch>,
    loading: BTreeSet<String>,
    expanded: BTreeSet<String>,
    nodes: HashMap<String, Node>,
    tree: Entity<TreeState>,
    documents: Vec<Entity<Document>>,
    active: usize,
    document_scroll: ScrollHandle,
    history: Option<Entity<History>>,
    status: String,
    error: String,
    focus: FocusHandle,
    save_task: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(
        services: Services,
        preferences: Preferences,
        profiles: Vec<ConnectionProfile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Document::init(cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-n", NewQuery, Some("QueriWorkspace")),
            KeyBinding::new("ctrl-p", FindTable, Some("QueriWorkspace")),
            KeyBinding::new("ctrl-p", FindTable, Some("QueriWorkspace > Input")),
            KeyBinding::new("ctrl-shift-n", NewConnection, Some("QueriWorkspace")),
            KeyBinding::new("ctrl-w", CloseDocument, Some("QueriWorkspace")),
            KeyBinding::new("ctrl-h", ToggleHistory, Some("QueriWorkspace")),
            KeyBinding::new("ctrl-tab", NextDocument, Some("QueriWorkspace")),
            KeyBinding::new("ctrl-h", ToggleHistory, Some("QueriWorkspace > Input")),
            KeyBinding::new("enter", OpenTreeItem, Some("QueriWorkspace > Tree")),
        ]);
        let tree = cx.new(|cx| TreeState::new(cx));
        let connection_picker = cx.new(|cx| ConnectionPicker::new(&profiles, window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &connection_picker,
                window,
                |this, _, event: &ConnectionPickerEvent, window, cx| match event {
                    ConnectionPickerEvent::Selected(id) => {
                        if this.selected != Some(*id)
                            || this.connection_state == ConnectionState::Failed
                        {
                            this.activate(*id, window, cx);
                        }
                    }
                    ConnectionPickerEvent::NewConnection => this.connection_form(None, window, cx),
                    ConnectionPickerEvent::EditConnection(id) => {
                        if let Some(profile) = this
                            .profiles
                            .iter()
                            .find(|profile| profile.id == *id)
                            .cloned()
                        {
                            this.connection_form(Some(profile), window, cx);
                        }
                    }
                    ConnectionPickerEvent::RefreshConnection(id) => this.activate(*id, window, cx),
                    ConnectionPickerEvent::RemoveConnection(id) => {
                        this.delete_connection(*id, window, cx)
                    }
                },
            ),
            cx.subscribe_in(
                &tree,
                window,
                |this, _, event: &TreeEvent, window, cx| match event {
                    TreeEvent::Expanded(id) => {
                        this.expanded.insert(id.to_string());
                        this.load_node(id, window, cx);
                    }
                    TreeEvent::Collapsed(id) => {
                        this.expanded.remove(id.as_ref());
                    }
                },
            ),
            cx.observe_window_bounds(window, |this, window, cx| {
                let size = window.viewport_size();
                this.preferences.window_width = f32::from(size.width);
                this.preferences.window_height = f32::from(size.height);
                this.schedule_save(window, cx);
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                if matches!(this.preferences.appearance, Appearance::System) {
                    this.apply_appearance(window, cx);
                }
            }),
            cx.on_app_quit(|this, cx| {
                this.save_task = None;
                let preferences = this.snapshot(cx);
                let services = this.services.clone();
                let sessions = this
                    .documents
                    .iter()
                    .filter_map(|doc| doc.read(cx).session.clone())
                    .collect::<Vec<_>>();
                async move {
                    let result = services.save_preferences(preferences).await;
                    if let Err(error) = result {
                        eprintln!("Could not save workspace: {error:#}");
                    }
                    let job = services.runtime.spawn(async move {
                        for session in sessions {
                            let _ = session.cancel().await;
                        }
                    });
                    let _ = job.await;
                }
            }),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        window.on_window_should_close(cx, |_, cx| {
            cx.quit();
            false
        });
        let mut workspace = Self {
            services,
            preferences,
            profiles,
            connection_picker,
            selected: None,
            passwords: HashMap::new(),
            generation: Uuid::new_v4(),
            connection_state: ConnectionState::Disconnected,
            databases: vec![],
            branches: BTreeMap::new(),
            loading: BTreeSet::new(),
            expanded: BTreeSet::new(),
            nodes: HashMap::new(),
            tree,
            documents: vec![],
            active: 0,
            document_scroll: ScrollHandle::new(),
            history: None,
            status: "Choose a connection to begin".into(),
            error: String::new(),
            focus,
            save_task: None,
            subscriptions,
        };
        workspace.apply_appearance(window, cx);
        let drafts = workspace.preferences.drafts.clone();
        for draft in drafts {
            if let Some(profile) = workspace
                .profiles
                .iter()
                .find(|profile| profile.id == draft.connection_id)
                .cloned()
            {
                workspace.add_document(
                    profile.for_database(&draft.database),
                    draft.table,
                    draft.sql,
                    false,
                    window,
                    cx,
                );
                if let Some(doc) = workspace.documents.last() {
                    doc.update(cx, |doc, _| {
                        doc.editor_height = draft.editor_height.clamp(100., 1200.);
                        if let Some(number) = draft.query_number {
                            doc.query_number = number;
                        }
                    });
                }
            }
        }
        workspace.active = workspace
            .preferences
            .active_document
            .min(workspace.documents.len().saturating_sub(1));
        workspace.reveal_active_tab(window, cx);
        if let Some(id) = workspace
            .preferences
            .active_connection
            .filter(|id| workspace.profiles.iter().any(|profile| profile.id == *id))
        {
            workspace.activate(id, window, cx);
        }
        workspace
    }

    fn snapshot(&self, cx: &App) -> Preferences {
        let mut preferences = self.preferences.clone();
        preferences.active_connection = self.selected;
        preferences.active_document = self.active;
        preferences.drafts = self
            .documents
            .iter()
            .map(|doc| doc.read(cx).draft(cx))
            .collect();
        preferences
    }

    fn schedule_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let Ok((services, preferences)) =
                view.update_in(cx, |this, _, cx| (this.services.clone(), this.snapshot(cx)))
            else {
                return;
            };
            if let Err(error) = services.save_preferences(preferences).await {
                let _ = view.update_in(cx, |this, _, cx| {
                    this.error = format!("Could not save workspace: {error:#}");
                    cx.notify();
                });
            }
        }));
    }

    fn selected_profile(&self) -> Option<ConnectionProfile> {
        self.profiles
            .iter()
            .find(|profile| Some(profile.id) == self.selected)
            .cloned()
    }

    fn sync_connections(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.connection_picker.update(cx, |picker, cx| {
            picker.set_profiles(&self.profiles, self.selected, window, cx);
        });
    }

    fn activate(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
        else {
            return;
        };
        self.selected = Some(id);
        self.connection_picker
            .update(cx, |picker, cx| picker.set_selected(Some(id), window, cx));
        self.generation = Uuid::new_v4();
        let generation = self.generation;
        self.connection_state = ConnectionState::Connecting;
        self.connection_picker
            .update(cx, |picker, cx| picker.set_connecting(true, cx));
        self.databases.clear();
        self.branches.clear();
        self.nodes.clear();
        self.loading.clear();
        self.expanded.clear();
        self.tree.update(cx, |tree, cx| tree.set_items(vec![], cx));
        self.history = None;
        self.status = format!("Connecting to {}…", profile.name);
        self.error.clear();
        let cached = self.passwords.get(&id).cloned();
        task(
            &self.services,
            window,
            cx,
            async move {
                let password = if let Some(password) = cached {
                    password
                } else if profile.remember_password {
                    tokio::task::spawn_blocking(move || storage::load_password(id))
                        .await??
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let session = Session::connect(profile, password.clone()).await?;
                let databases = session.databases().await?;
                Ok((password, databases))
            },
            move |this, result, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.connection_picker
                    .update(cx, |picker, cx| picker.set_connecting(false, cx));
                match result {
                    Ok((password, databases)) => {
                        this.connection_state = ConnectionState::Ready;
                        this.passwords.insert(id, password.clone());
                        this.databases = databases;
                        this.status = "Connected".into();
                        this.rebuild_tree(cx);
                        for doc in &this.documents {
                            if doc.read(cx).profile.id == id && doc.read(cx).session.is_none() {
                                doc.update(cx, |doc, cx| {
                                    doc.set_password(password.clone());
                                    if this.databases.contains(&doc.profile.database()) {
                                        doc.connect(window, cx);
                                    } else {
                                        doc.database_removed(cx);
                                    }
                                });
                            }
                        }
                    }
                    Err(error) => {
                        this.connection_state = ConnectionState::Failed;
                        this.error = format!(
                            "{error:#}\nEdit the connection to update its credentials, or retry."
                        );
                        this.status = "Connection failed".into();
                    }
                }
                this.schedule_save(window, cx);
                cx.notify();
            },
        );
        cx.notify();
    }

    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        self.nodes.clear();
        let mut items = vec![];
        for database in &self.databases {
            let id = format!("db:{database}");
            self.nodes
                .insert(id.clone(), Node::Database(database.clone()));
            let mut children = vec![];
            if let Some(branch) = self.branches.get(database) {
                for (schema, tables) in &branch.schemas {
                    let schema_id = format!("schema:{}", serde_json::json!([database, schema]));
                    self.nodes.insert(
                        schema_id.clone(),
                        Node::Schema {
                            database: database.clone(),
                            schema: schema.clone(),
                        },
                    );
                    let table_items =
                        if let Some(tables) = tables.as_ref().filter(|tables| !tables.is_empty()) {
                            tables
                                .iter()
                                .map(|table| {
                                    let table_id = format!(
                                        "table:{}",
                                        serde_json::json!([database, schema, table.name])
                                    );
                                    self.nodes.insert(
                                        table_id.clone(),
                                        Node::Table {
                                            database: database.clone(),
                                            table: table.clone(),
                                        },
                                    );
                                    TreeItem::new(
                                        table_id,
                                        format!(
                                            "{}{}",
                                            table.name,
                                            if table.is_table() { "" } else { " (view)" }
                                        ),
                                    )
                                })
                                .collect()
                        } else {
                            vec![
                                TreeItem::new(
                                    format!("{schema_id}:placeholder"),
                                    if tables.is_some() {
                                        "No tables in this schema"
                                    } else if self.loading.contains(&schema_id) {
                                        "Loading tables…"
                                    } else {
                                        "Expand to load tables"
                                    },
                                )
                                .disabled(true),
                            ]
                        };
                    children.push(
                        TreeItem::new(schema_id.clone(), schema.clone())
                            .children(table_items)
                            .expanded(self.expanded.contains(&schema_id)),
                    );
                }
            } else {
                children.push(
                    TreeItem::new(
                        format!("{id}:placeholder"),
                        if self.loading.contains(&id) {
                            "Loading schemas…"
                        } else {
                            "Expand to load schemas"
                        },
                    )
                    .disabled(true),
                );
            }
            if children.is_empty() {
                children.push(
                    TreeItem::new(format!("{id}:empty"), "No schemas available").disabled(true),
                );
            }
            items.push(
                TreeItem::new(id.clone(), database.clone())
                    .children(children)
                    .expanded(self.expanded.contains(&id)),
            );
        }
        let selected = self
            .tree
            .read(cx)
            .selected_item()
            .map(|item| item.id.clone());
        self.tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            if let Some(id) = selected {
                tree.set_selected_index(tree.index_of(&id), cx);
            }
        });
        cx.notify();
    }

    fn load_node(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading.contains(id) {
            return;
        }
        let Some(node) = self.nodes.get(id).cloned() else {
            return;
        };
        let generation = self.generation;
        let id = id.to_owned();
        match node {
            Node::Database(database) => {
                if self.branches.contains_key(&database) {
                    return;
                }
                let Some(profile) = self.selected_profile() else {
                    return;
                };
                let password = self.passwords.get(&profile.id).cloned().unwrap_or_default();
                self.loading.insert(id.clone());
                let profile = profile.for_database(&database);
                task(
                    &self.services,
                    window,
                    cx,
                    async move {
                        let session = Session::connect(profile, password).await?;
                        let schemas = session
                            .schemas()
                            .await?
                            .into_iter()
                            .map(|name| (name, None))
                            .collect();
                        Ok(DatabaseBranch { session, schemas })
                    },
                    move |this, result, _, cx| {
                        if this.generation != generation {
                            return;
                        }
                        this.loading.remove(&id);
                        match result {
                            Ok(branch) => {
                                this.branches.insert(database, branch);
                                this.error.clear();
                            }
                            Err(error) => {
                                this.error = format!(
                                    "Could not load schemas: {error:#}\nCollapse and expand the database to retry, or refresh the connection."
                                )
                            }
                        }
                        this.rebuild_tree(cx);
                    },
                );
            }
            Node::Schema { database, schema } => {
                let Some(branch) = self.branches.get(&database) else {
                    return;
                };
                if branch.schemas.get(&schema).is_some_and(Option::is_some) {
                    return;
                }
                let session = branch.session.clone();
                let query_schema = schema.clone();
                self.loading.insert(id.clone());
                task(
                    &self.services,
                    window,
                    cx,
                    async move { session.tables(query_schema).await },
                    move |this, result, _, cx| {
                        if this.generation != generation {
                            return;
                        }
                        this.loading.remove(&id);
                        match result {
                            Ok(tables) => {
                                if let Some(branch) = this.branches.get_mut(&database) {
                                    branch.schemas.insert(schema, Some(tables));
                                }
                                this.error.clear();
                            }
                            Err(error) => {
                                this.error = format!(
                                    "Could not load tables: {error:#}\nCollapse and expand the schema to retry, or refresh the connection."
                                )
                            }
                        }
                        this.rebuild_tree(cx);
                    },
                );
            }
            Node::Table { .. } => {}
        }
        self.rebuild_tree(cx);
    }

    fn open_tree_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self
            .tree
            .read(cx)
            .selected_item()
            .map(|item| item.id.to_string())
        else {
            return;
        };
        self.open_node(&id, window, cx);
    }

    fn open_node(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(id).cloned() else {
            return;
        };
        match node {
            Node::Table { database, table } => {
                let Some(profile) = self.selected_profile() else {
                    return;
                };
                self.open_table(profile.for_database(&database), table, window, cx);
            }
            _ => {
                if !self.expanded.remove(id) {
                    self.expanded.insert(id.to_owned());
                }
                self.rebuild_tree(cx);
                self.load_node(id, window, cx);
            }
        }
    }

    fn open_table(
        &mut self,
        profile: ConnectionProfile,
        table: TableRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.documents.iter().position(|doc| {
            let doc = doc.read(cx);
            doc.profile.id == profile.id
                && doc.profile.database() == profile.database()
                && doc.table_ref() == Some(&table)
        }) {
            self.select_document(index, window, cx);
        } else {
            self.add_document(profile, Some(table), String::new(), true, window, cx);
        }
    }

    fn add_document(
        &mut self,
        profile: ConnectionProfile,
        table: Option<TableRef>,
        sql: String,
        connect: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let password = self.passwords.get(&profile.id).cloned().unwrap_or_default();
        let query_number = self
            .documents
            .iter()
            .filter(|doc| doc.read(cx).table_ref().is_none())
            .map(|doc| doc.read(cx).query_number)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let doc = cx.new(|cx| {
            Document::new(
                self.services.clone(),
                profile,
                password,
                table,
                sql,
                window,
                cx,
            )
        });
        doc.update(cx, |doc, cx| {
            doc.query_number = query_number;
            doc.warn_destructive = self.preferences.warn_destructive;
            if connect {
                doc.connect(window, cx);
                doc.focus(window, cx);
            }
        });
        self.subscriptions.push(cx.subscribe_in(
            &doc,
            window,
            |this, _, _: &DocumentChanged, window, cx| this.schedule_save(window, cx),
        ));
        self.subscriptions.push(cx.subscribe_in(
            &doc,
            window,
            |this, doc, _: &NewDocumentQuery, window, cx| {
                let profile = doc.read(cx).profile.clone();
                this.add_document(profile, None, String::new(), true, window, cx);
            },
        ));
        self.documents.push(doc);
        self.active = self.documents.len() - 1;
        if !self.history_docked(window) {
            self.history = None;
        }
        self.reveal_active_tab(window, cx);
        if connect {
            self.schedule_save(window, cx);
        }
        cx.notify();
    }

    fn new_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = self.selected_profile() else {
            if self.profiles.is_empty() {
                self.connection_form(None, window, cx);
            } else {
                self.connection_picker
                    .update(cx, |picker, cx| picker.focus(window, cx));
            }
            return;
        };
        let database = self
            .current_database(cx)
            .unwrap_or_else(|| profile.database());
        self.add_document(
            profile.for_database(&database),
            None,
            String::new(),
            true,
            window,
            cx,
        );
    }

    fn current_database(&self, cx: &App) -> Option<String> {
        let profile = self.selected_profile()?;
        self.tree
            .read(cx)
            .selected_item()
            .and_then(|item| self.nodes.get(item.id.as_ref()))
            .map(|node| match node {
                Node::Database(database)
                | Node::Schema { database, .. }
                | Node::Table { database, .. } => database.clone(),
            })
            .or_else(|| {
                self.documents
                    .get(self.active)
                    .map(|doc| doc.read(cx))
                    .filter(|doc| doc.profile.id == profile.id)
                    .map(|doc| doc.profile.database())
            })
            .or_else(|| Some(profile.database()))
    }

    fn find_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_state != ConnectionState::Ready {
            return;
        }
        let Some(profile) = self.selected_profile() else {
            return;
        };
        let database = self
            .current_database(cx)
            .unwrap_or_else(|| profile.database());
        let profile = profile.for_database(&database);
        let password = self.passwords.get(&profile.id).cloned().unwrap_or_default();
        let finder = cx.new(|cx| {
            TableFinder::new(self.services.clone(), profile.clone(), password, window, cx)
        });
        self.subscriptions.push(cx.subscribe_in(
            &finder,
            window,
            move |this, _, event: &OpenTable, window, cx| {
                this.open_table(profile.clone(), event.0.clone(), window, cx);
            },
        ));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Find table")
                .width(px(560.))
                .child(finder.clone())
        });
    }

    fn select_document(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.documents.get(index).cloned() else {
            return;
        };
        self.active = index;
        if !self.history_docked(window) {
            self.history = None;
        }
        self.reveal_active_tab(window, cx);
        let id = doc.read(cx).profile.id;
        doc.update(cx, |doc, cx| doc.focus(window, cx));
        if self.selected != Some(id) || !self.passwords.contains_key(&id) {
            self.activate(id, window, cx);
        } else if doc.read(cx).session.is_none() && !doc.read(cx).busy() {
            doc.update(cx, |doc, cx| {
                doc.set_password(self.passwords.get(&id).cloned().unwrap_or_default());
                doc.connect(window, cx);
            });
        }
        self.schedule_save(window, cx);
        cx.notify();
    }

    fn close_document(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(doc) = self.documents.get(index) {
            if doc.read(cx).busy() {
                doc.update(cx, |doc, cx| doc.cancel(window, cx));
                self.error =
                    "Cancelling the active operation. Close the tab again after it finishes."
                        .into();
                cx.notify();
                return;
            }
            self.documents.remove(index);
            self.active = self
                .active
                .saturating_sub(usize::from(index <= self.active))
                .min(self.documents.len().saturating_sub(1));
            if let Some(doc) = self.documents.get(self.active) {
                doc.update(cx, |doc, cx| doc.focus(window, cx));
            } else {
                self.focus.focus(window, cx);
            }
            self.reveal_active_tab(window, cx);
            self.schedule_save(window, cx);
            cx.notify();
        }
    }

    fn reveal_active_tab(&self, window: &mut Window, cx: &Context<Self>) {
        let scroll = self.document_scroll.clone();
        let view = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = view.update(cx, |this, cx| {
                if !this.documents.is_empty() {
                    scroll.scroll_to_item(this.active);
                    cx.notify();
                }
            });
        });
    }

    fn connection_form(
        &mut self,
        profile: Option<ConnectionProfile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if profile.as_ref().is_some_and(|profile| {
            self.documents
                .iter()
                .any(|doc| doc.read(cx).profile.id == profile.id && doc.read(cx).busy())
        }) {
            self.error =
                "Wait for this connection's operations to finish before editing it.".into();
            cx.notify();
            return;
        }
        let password = profile
            .as_ref()
            .and_then(|profile| self.passwords.get(&profile.id))
            .cloned();
        let form =
            cx.new(|cx| ConnectionForm::new(self.services.clone(), profile, password, window, cx));
        self.subscriptions.push(cx.subscribe_in(
            &form,
            window,
            |this, _, saved: &SavedConnection, window, cx| {
                this.profiles
                    .retain(|profile| profile.id != saved.profile.id);
                this.profiles.push(saved.profile.clone());
                this.profiles
                    .sort_by_key(|profile| profile.name.to_lowercase());
                this.sync_connections(window, cx);
                this.passwords
                    .insert(saved.profile.id, saved.password.clone());
                for doc in &this.documents {
                    if doc.read(cx).profile.id == saved.profile.id {
                        doc.update(cx, |doc, _| {
                            doc.reconfigure(&saved.profile, saved.password.clone())
                        });
                    }
                }
                this.activate(saved.profile.id, window, cx);
            },
        ));
        let title = if form.read(cx).is_existing() {
            "Edit connection"
        } else {
            "New connection"
        };
        // A menu's focus handle disappears on selection; restore to a persistent control.
        if self.profiles.is_empty() {
            self.focus.focus(window, cx);
        } else {
            self.connection_picker
                .update(cx, |picker, cx| picker.focus(window, cx));
        }
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .title(title)
                .centered()
                .width(px(620.))
                .overlay_closable(false)
                .close_button(!form.read(cx).busy())
                .keyboard(!form.read(cx).busy())
                .footer(form.update(cx, |form, cx| form.render_footer(cx)))
                .child(form.clone())
        });
    }

    fn delete_connection(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
        else {
            return;
        };
        self.connection_picker
            .update(cx, |picker, cx| picker.focus(window, cx));
        if self
            .documents
            .iter()
            .any(|doc| doc.read(cx).profile.id == profile.id && doc.read(cx).busy())
        {
            self.error =
                "Wait for this connection's operations to finish before removing it.".into();
            cx.notify();
            return;
        }
        let view = cx.entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let profile = profile.clone();
            dialog
                .title(format!("Remove {}?", profile.name))
                .child("Its saved connection, SQL drafts, and query history will be removed. The database itself is retained.")
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Remove connection")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    view.update(cx, |this, cx| {
                        let store = this.services.store.clone();
                        let id = profile.id;
                        let remember_password = profile.remember_password;
                        task(
                            &this.services,
                            window,
                            cx,
                            async move {
                                tokio::task::spawn_blocking(move || {
                                    if remember_password {
                                        storage::delete_password(id)?;
                                    }
                                    store.delete_profile(id)
                                })
                                .await?
                            },
                            move |this, result, window, cx| {
                                match result {
                                    Ok(()) => {
                                        this.profiles.retain(|profile| profile.id != id);
                                        this.passwords.remove(&id);
                                        this.documents.retain(|doc| doc.read(cx).profile.id != id);
                                        this.active = 0;
                                        if this.selected == Some(id) {
                                            this.selected = None;
                                            this.generation = Uuid::new_v4();
                                            this.connection_state = ConnectionState::Disconnected;
                                            this.databases.clear();
                                            this.branches.clear();
                                            this.history = None;
                                            this.rebuild_tree(cx);
                                        }
                                        this.status = "Choose a connection to begin".into();
                                        this.sync_connections(window, cx);
                                        this.schedule_save(window, cx);
                                    }
                                    Err(error) => this.error = format!("{error:#}"),
                                }
                                cx.notify();
                            },
                        );
                    });
                    true
                })
        });
    }

    fn toggle_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.is_some() {
            self.history = None;
            if let Some(doc) = self.documents.get(self.active) {
                doc.update(cx, |doc, cx| doc.focus(window, cx));
            } else {
                self.focus.focus(window, cx);
            }
            cx.notify();
            return;
        }
        let Some(id) = self.selected else {
            return;
        };
        let history = cx.new(|cx| History::new(self.services.clone(), id, window, cx));
        self.subscriptions.push(cx.subscribe_in(
            &history,
            window,
            |this, _, _: &NewHistoryQuery, window, cx| this.new_query(window, cx),
        ));
        self.subscriptions.push(cx.subscribe_in(
            &history,
            window,
            |this, _, event: &ReplayQuery, window, cx| {
                if let Some(profile) = this
                    .profiles
                    .iter()
                    .find(|profile| profile.id == event.0.connection_id)
                    .cloned()
                {
                    this.add_document(
                        profile.for_database(&event.0.database),
                        None,
                        event.0.sql.clone(),
                        true,
                        window,
                        cx,
                    );
                }
            },
        ));
        history.update(cx, |history, cx| {
            history.load(window, cx);
            history.focus(window, cx);
        });
        self.history = Some(history);
        cx.notify();
    }

    fn history_docked(&self, window: &Window) -> bool {
        window.viewport_size().width >= px(self.preferences.sidebar_width + 920.)
    }

    fn history_panel(&self, history: &Entity<History>, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .justify_between()
                    .h_12()
                    .flex_shrink_0()
                    .px_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(IconName::Clock).small())
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Query history"),
                            ),
                    )
                    .child(
                        icon_button("close-history", IconName::X, "Close query history").on_click(
                            cx.listener(|this, _, window, cx| this.toggle_history(window, cx)),
                        ),
                    ),
            )
            .child(div().flex_1().min_h_0().child(history.clone()))
            .into_any_element()
    }

    fn admin(&mut self, action: AdminAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = self.selected_profile() else {
            return;
        };
        let title = action.label();
        let sessions = matches!(action, AdminAction::Sessions(_));
        let password = self.passwords.get(&profile.id).cloned().unwrap_or_default();
        let admin = cx.new(|cx| {
            DatabaseAdmin::new(self.services.clone(), profile, password, action, window, cx)
        });
        self.subscriptions.push(cx.subscribe_in(
            &admin,
            window,
            |this, admin, event: &AdminEvent, window, cx| {
                let id = admin.read(cx).connection_id();
                match event {
                    AdminEvent::Execute(action) => {
                        let source = match action {
                            DatabaseAction::Create { .. } => None,
                            DatabaseAction::Rename { source, .. } | DatabaseAction::Clone { source, .. } => Some(source.as_str()),
                            DatabaseAction::Delete { name, .. } => Some(name.as_str()),
                        };
                        let affected = this.documents.iter().filter(|doc| {
                            let doc = doc.read(cx);
                            doc.profile.id == id && source == Some(doc.profile.database().as_str())
                        }).cloned().collect::<Vec<_>>();
                        if affected.iter().any(|doc| doc.read(cx).busy()) || !this.loading.is_empty() {
                            admin.update(cx, |admin, cx| admin.show_error("Wait for queries and explorer loading to finish, or cancel the query, then retry.".into(), cx));
                            return;
                        }
                        for doc in affected { doc.update(cx, |doc, cx| doc.disconnect(cx)); }
                        if this.selected == Some(id) && let Some(source) = source {
                            this.branches.remove(source);
                            this.rebuild_tree(cx);
                        }
                        admin.update(cx, |admin, cx| admin.execute(action.clone(), window, cx));
                    }
                    AdminEvent::Completed { action, maintenance } => this.database_changed(id, action, maintenance, window, cx),
                }
            },
        ));
        if sessions {
            admin.update(cx, |admin, cx| admin.load_sessions(window, cx));
        }
        self.tree.update(cx, |tree, cx| tree.focus(window, cx));
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .title(title)
                .width(px(520.))
                .centered()
                .overlay_closable(false)
                .close_button(!admin.read(cx).busy())
                .keyboard(!admin.read(cx).busy())
                .footer(admin.update(cx, |admin, cx| admin.render_footer(cx)))
                .child(admin.clone())
        });
    }

    fn database_changed(
        &mut self,
        id: Uuid,
        action: &DatabaseAction,
        maintenance: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = match action {
            DatabaseAction::Create { name } | DatabaseAction::Clone { target: name, .. } => name,
            DatabaseAction::Rename { source, target, .. } => {
                for doc in &self.documents {
                    if doc.read(cx).profile.id == id && doc.read(cx).profile.database() == *source {
                        doc.update(cx, |doc, cx| doc.rename_database(target, cx));
                    }
                }
                target
            }
            DatabaseAction::Delete { name, .. } => {
                for doc in &self.documents {
                    if doc.read(cx).profile.id == id && doc.read(cx).profile.database() == *name {
                        doc.update(cx, |doc, cx| doc.database_removed(cx));
                    }
                }
                maintenance
            }
        };
        let old = match action {
            DatabaseAction::Rename { source, .. } => Some(source),
            DatabaseAction::Delete { name, .. } => Some(name),
            _ => None,
        };
        if let Some(profile) = self.profiles.iter_mut().find(|profile| profile.id == id)
            && old.is_some_and(|old| profile.database() == *old)
        {
            *profile = profile.for_database(target);
            let profile = profile.clone();
            let store = self.services.store.clone();
            task(
                &self.services,
                window,
                cx,
                async move { tokio::task::spawn_blocking(move || store.save_profile(&profile)).await? },
                |this, result, _, cx| {
                    if let Err(error) = result {
                        this.error = format!(
                            "The database changed, but its connection settings could not be saved: {error:#}. Edit the connection to save its new default database."
                        );
                    }
                    cx.notify();
                },
            );
        }
        if self.selected == Some(id) {
            match action {
                DatabaseAction::Create { name } | DatabaseAction::Clone { target: name, .. } => {
                    self.databases.push(name.clone())
                }
                DatabaseAction::Rename { source, target, .. } => {
                    for database in &mut self.databases {
                        if database == source {
                            *database = target.clone();
                        }
                    }
                    self.expanded.remove(&format!("db:{source}"));
                }
                DatabaseAction::Delete { name, .. } => {
                    self.databases.retain(|database| database != name);
                    self.expanded.remove(&format!("db:{name}"));
                }
            }
            self.databases.sort();
            self.databases.dedup();
            self.rebuild_tree(cx);
            let node = format!("db:{target}");
            self.tree.update(cx, |tree, cx| {
                tree.set_selected_index(tree.index_of(&SharedString::from(node.clone())), cx)
            });
            if !matches!(action, DatabaseAction::Delete { .. }) {
                self.expanded.insert(node.clone());
                self.load_node(&node, window, cx);
            }
        }
        self.status = match action {
            DatabaseAction::Create { name } => format!("Created {name}"),
            DatabaseAction::Clone { target, .. } => format!("Created {target}"),
            DatabaseAction::Rename { target, .. } => format!("Renamed database to {target}"),
            DatabaseAction::Delete { name, .. } => format!("Deleted {name} · SQL drafts retained"),
        };
        self.schedule_save(window, cx);
        cx.notify();
    }

    fn database_menu(
        &self,
        mut menu: PopupMenu,
        database: &str,
        cx: &mut Context<Self>,
    ) -> PopupMenu {
        let database_query = database.to_owned();
        menu = menu.min_w(px(224.)).label(database.to_owned()).item(
            menu_item("New query", IconName::Code).on_click(cx.listener(
                move |this, _, window, cx| {
                    if let Some(profile) = this.selected_profile() {
                        this.add_document(
                            profile.for_database(&database_query),
                            None,
                            String::new(),
                            true,
                            window,
                            cx,
                        );
                    }
                },
            )),
        );
        if self
            .selected_profile()
            .is_some_and(|profile| matches!(profile.config, ConnectionConfig::Postgres { .. }))
        {
            menu = menu.separator();
            for (action, icon) in [
                (AdminAction::Rename(database.into()), IconName::Pencil),
                (AdminAction::Clone(database.into()), IconName::Copy),
                (AdminAction::Sessions(database.into()), IconName::Activity),
            ] {
                menu = menu.item(menu_item(action.label(), icon).on_click(
                    cx.listener(move |this, _, window, cx| this.admin(action.clone(), window, cx)),
                ));
            }
            let database = database.to_owned();
            menu = menu
                .separator()
                .item(
                    destructive_menu_item("Delete database", false).on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.admin(AdminAction::Delete(database.clone()), window, cx)
                        },
                    )),
                );
        }
        menu
    }

    fn apply_appearance(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.preferences.appearance {
            Appearance::System => Theme::sync_system_appearance(Some(window), cx),
            Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
        super::theme::apply(cx);
    }

    fn welcome(&self, cx: &mut Context<Self>) -> AnyElement {
        let (icon, title, description, action) = match self.connection_state {
            ConnectionState::Connecting => (
                IconName::RefreshCw,
                "Opening your connection",
                "Your databases will appear in the sidebar when the connection is ready.",
                "Connecting…",
            ),
            ConnectionState::Failed => (
                IconName::Unplug,
                "Connection unavailable",
                "Check your connection settings, then try again.",
                "Retry connection",
            ),
            ConnectionState::Ready if self.databases.is_empty() => (
                IconName::Database,
                "No databases available",
                "Refresh the connection, or check that your account has access to a database.",
                "Refresh connection",
            ),
            ConnectionState::Ready => (
                IconName::Table2,
                "Explore a little. Discover a lot.",
                "Find a table, explore your data, or start a fresh query. Each opens in its own tab.",
                "Find table",
            ),
            ConnectionState::Disconnected if !self.profiles.is_empty() => (
                IconName::Database,
                "Welcome back",
                "Choose a saved connection to pick up where you left off.",
                "Choose a connection",
            ),
            ConnectionState::Disconnected => (
                IconName::Database,
                "Your data, clearly.",
                "A focused workspace for PostgreSQL and SQLite. Connect a database to explore, query, and edit with clarity.",
                "Add connection",
            ),
        };
        v_flex()
            .id("welcome")
            .size_full()
            .overflow_y_scroll()
            .items_center()
            .justify_center()
            .p_8()
            .child(
                v_flex()
                    .max_w(rems(32.))
                    .w_full()
                    .items_center()
                    .gap_5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(56.))
                            .when(
                                self.connection_state == ConnectionState::Disconnected,
                                |view| view.child(brand_mark(px(56.), cx)),
                            )
                            .when(
                                self.connection_state != ConnectionState::Disconnected,
                                |view| {
                                    view.rounded(cx.theme().radius_tokens().xl)
                                        .bg(cx.theme().muted)
                                        .child(Icon::new(icon).large())
                                },
                            ),
                    )
                    .child(
                        v_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_3xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_center()
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .line_height(relative(1.6))
                                    .text_center()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(description),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .justify_center()
                            .child(
                                Button::new("start")
                                    .primary()
                                    .icon(match self.connection_state {
                                        ConnectionState::Connecting | ConnectionState::Failed => {
                                            IconName::RefreshCw
                                        }
                                        ConnectionState::Ready if self.databases.is_empty() => {
                                            IconName::RefreshCw
                                        }
                                        ConnectionState::Ready => IconName::Search,
                                        _ => IconName::Plus,
                                    })
                                    .label(action)
                                    .loading(self.connection_state == ConnectionState::Connecting)
                                    .disabled(self.connection_state == ConnectionState::Connecting)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.connection_state == ConnectionState::Failed
                                            || (this.connection_state == ConnectionState::Ready
                                                && this.databases.is_empty())
                                        {
                                            if let Some(id) = this.selected {
                                                this.activate(id, window, cx);
                                            }
                                        } else if this.connection_state == ConnectionState::Ready {
                                            this.find_table(window, cx);
                                        } else {
                                            this.new_query(window, cx);
                                        }
                                    })),
                            )
                            .when(self.connection_state == ConnectionState::Failed, |row| {
                                row.child(
                                    Button::new("welcome-edit-connection")
                                        .label("Edit connection")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.connection_form(
                                                this.selected_profile(),
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                            })
                            .when(
                                self.connection_state == ConnectionState::Ready
                                    && !self.databases.is_empty(),
                                |row| {
                                    row.child(
                                        Button::new("welcome-new-query")
                                            .icon(IconName::Code)
                                            .label("New query")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.new_query(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("browse-databases")
                                            .ghost()
                                            .icon(IconName::Table2)
                                            .label("Browse tables")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.browse_tables(window, cx)
                                            })),
                                    )
                                },
                            ),
                    )
                    .when(
                        self.connection_state == ConnectionState::Disconnected
                            && !self.profiles.is_empty(),
                        |view| {
                            view.child(
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .pt_3()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Saved connections"),
                                    )
                                    .children(self.profiles.iter().take(3).map(|profile| {
                                        let id = profile.id;
                                        Button::new(SharedString::from(format!(
                                            "welcome-connect-{id}"
                                        )))
                                        .icon(IconName::Database)
                                        .label(profile.name.clone())
                                        .tooltip(format!(
                                            "Connect to {} · {}",
                                            profile.name,
                                            profile.provider()
                                        ))
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.activate(id, window, cx)
                                            }),
                                        )
                                    })),
                            )
                        },
                    )
                    .when(
                        self.connection_state == ConnectionState::Ready
                            && !self.databases.is_empty(),
                        |view| {
                            view.child(
                                h_flex()
                                    .gap_6()
                                    .pt_5()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(shortcut("Ctrl P", cx))
                                    .child("Find table")
                                    .child(shortcut("Ctrl N", cx))
                                    .child("New query"),
                            )
                        },
                    ),
            )
            .into_any_element()
    }

    fn browse_tables(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(database) = self
            .selected_profile()
            .map(|profile| profile.database())
            .filter(|database| self.databases.contains(database))
            .or_else(|| self.databases.first().cloned())
        else {
            return;
        };
        let id = SharedString::from(format!("db:{database}"));
        self.expanded.insert(id.to_string());
        self.rebuild_tree(cx);
        self.load_node(&id, window, cx);
        self.tree.update(cx, |tree, cx| {
            tree.set_selected_index(tree.index_of(&id), cx);
            tree.focus(window, cx);
        });
    }

    fn settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        self.focus.focus(window, cx);
        window.open_dialog(cx, move |dialog, _, cx| {
            let appearance = match view.read(cx).preferences.appearance {
                Appearance::System => 0,
                Appearance::Light => 1,
                Appearance::Dark => 2,
            };
            let checked = view.read(cx).preferences.warn_destructive;
            let theme_view = view.clone();
            let warning_view = view.clone();
            dialog.title("Preferences").child(
                v_flex()
                    .gap_5()
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(div().text_sm().child("Appearance"))
                            .child(
                                TabBar::new("appearance")
                                    .segmented()
                                    .selected_index(appearance)
                                    .children(
                                        ["System", "Light", "Dark"]
                                            .map(|label| Tab::new().label(label)),
                                    )
                                    .on_click(move |index, window, cx| {
                                        theme_view.update(cx, |this, cx| {
                                            this.preferences.appearance = match index {
                                                1 => Appearance::Light,
                                                2 => Appearance::Dark,
                                                _ => Appearance::System,
                                            };
                                            this.apply_appearance(window, cx);
                                            this.schedule_save(window, cx);
                                            cx.notify();
                                        })
                                    }),
                            ),
                    )
                    .child(setting_switch(
                        "warn-destructive",
                        "Confirm destructive SQL before execution",
                        checked,
                        move |on, window, cx| {
                            warning_view.update(cx, |this, cx| {
                                this.preferences.warn_destructive = *on;
                                for doc in &this.documents {
                                    doc.update(cx, |doc, cx| {
                                        doc.warn_destructive = *on;
                                        cx.notify();
                                    });
                                }
                                this.schedule_save(window, cx);
                                cx.notify();
                            })
                        },
                    ))
                    .child(
                        v_flex()
                            .gap_3()
                            .pt_4()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Keyboard shortcuts"),
                            )
                            .children(
                                [
                                    ("New query", "Ctrl + N"),
                                    ("Find table", "Ctrl + P"),
                                    ("Run SQL", "Ctrl + Enter"),
                                    ("Cancel query", "Ctrl + Shift + C"),
                                    ("Refresh table", "F5"),
                                    ("Edit row", "Ctrl + E"),
                                    ("Inspect cell", "Alt + Enter"),
                                    ("Query history", "Ctrl + H"),
                                    ("Close tab", "Ctrl + W"),
                                    ("Next tab", "Ctrl + Tab"),
                                ]
                                .map(|(label, keys)| {
                                    h_flex()
                                        .justify_between()
                                        .gap_4()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(label)
                                        .child(shortcut(keys, cx))
                                }),
                            ),
                    ),
            )
        });
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_profile = self.selected_profile();
        let view = cx.entity();
        let explorer = tree(&self.tree, move |index, entry, selected, _, cx| {
            let view = view.clone();
            let label = entry.item().label.clone();
            let id = entry.item().id.clone();
            let folder = entry.is_folder();
            let object_icon = match view.read(cx).nodes.get(id.as_ref()) {
                Some(Node::Database(_)) => Some(IconName::Database),
                Some(Node::Schema { .. }) => Some(IconName::Layers),
                Some(Node::Table { table, .. }) => Some(if table.is_table() {
                    IconName::Table2
                } else {
                    IconName::Eye
                }),
                None => None,
            };
            let database = match view.read(cx).nodes.get(id.as_ref()) {
                Some(Node::Database(name)) => Some(name.clone()),
                _ => None,
            };
            let menu_view = view.clone();
            let context_view = view.clone();
            let context_database = database.clone();
            let tooltip = label.clone();
            ListItem::new(index)
                .rounded(cx.theme().radius)
                .accessibility_label(label.clone())
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .min_h_7()
                        .gap_1p5()
                        .pl(px(u16::try_from(entry.depth())
                            .map(f32::from)
                            .unwrap_or(0.)
                            * 14.))
                        .child(div().size_4().flex_shrink_0().when(folder, |icon| {
                            icon.child(
                                Icon::new(if entry.is_expanded() {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .small()
                                .text_color(cx.theme().muted_foreground),
                            )
                        }))
                        .when_some(object_icon, |row, icon| {
                            row.child(Icon::new(icon).small().text_color(if selected {
                                cx.theme().foreground
                            } else {
                                cx.theme().muted_foreground
                            }))
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .text_ellipsis()
                                .when(selected, |label| label.font_weight(FontWeight::MEDIUM))
                                .child(label),
                        )
                        .when_some(database, |row, database| {
                            row.child(
                                icon_button(
                                    SharedString::from(format!("database-menu:{database}")),
                                    IconName::Ellipsis,
                                    format!("Actions for {database}"),
                                )
                                .dropdown_menu(
                                    move |menu, _, cx| {
                                        menu_view.update(cx, |this, cx| {
                                            this.database_menu(menu, &database, cx)
                                        })
                                    },
                                ),
                            )
                        })
                        .context_menu(move |menu, _, cx| {
                            if let Some(database) = &context_database {
                                context_view
                                    .update(cx, |this, cx| this.database_menu(menu, database, cx))
                            } else {
                                menu
                            }
                        }),
                )
                .on_click(move |_, window, cx| {
                    if !folder {
                        view.update(cx, |this, cx| this.open_node(&id, window, cx));
                    }
                })
        });
        let sidebar = v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .gap_2p5()
                    .h_12()
                    .flex_shrink_0()
                    .px_4()
                    .child(div().flex_1().child(brand_logo(px(32.), cx)))
                    .child(
                        icon_button("app-menu", IconName::Ellipsis, "Application menu")
                            .dropdown_menu({
                                let view = cx.entity();
                                move |menu, _, cx| {
                                    view.update(cx, |_, cx| {
                                        menu.item(
                                            menu_item("New query", Icon::empty())
                                                .action(Box::new(NewQuery)),
                                        )
                                            .item(
                                                menu_item("New connection…", Icon::empty())
                                                    .action(Box::new(NewConnection)),
                                            )
                                            .separator()
                                            .item(
                                                menu_item("Preferences…", IconName::Settings)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.settings(window, cx)
                                                        },
                                                    )),
                                            )
                                    })
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .when(!self.profiles.is_empty(), |view| {
                        view.child(self.connection_picker.clone())
                    })
                    .when(self.profiles.is_empty(), |view| {
                        view.child(
                            v_flex()
                                .gap_3()
                                .py_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .line_height(relative(1.5))
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Your databases, one workspace. Add a connection to get started."),
                                )
                                .child(
                                    Button::new("sidebar-add-connection")
                                        .primary()
                                        .icon(IconName::Plus)
                                        .label("Add connection")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.connection_form(None, window, cx)
                                        })),
                                ),
                        )
                    }),
            )
            .when_some(selected_profile.as_ref(), |sidebar, _| {
                let status = match self.connection_state {
                    ConnectionState::Ready => "Connected",
                    ConnectionState::Connecting => "Connecting…",
                    ConnectionState::Failed => "Connection failed",
                    ConnectionState::Disconnected => "Disconnected",
                };
                sidebar.child(
                    h_flex()
                        .px_4()
                        .pt_1()
                        .min_h_8()
                        .gap_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            h_flex()
                                .id("sidebar-connection-status")
                                .role(Role::Status)
                                .aria_label(status)
                                .flex_1()
                                .gap_1p5()
                                .when(self.connection_state == ConnectionState::Failed, |status| {
                                    status.text_color(cx.theme().danger)
                                })
                                .child(
                                    Icon::new(match self.connection_state {
                                        ConnectionState::Ready => IconName::Check,
                                        ConnectionState::Connecting => IconName::RefreshCw,
                                        _ => IconName::Unplug,
                                    })
                                    .xsmall(),
                                )
                                .child(status),
                        )
                        .when(self.connection_state == ConnectionState::Ready, |row| {
                            row.child(format!(
                                "{} {}",
                                self.databases.len(),
                                if self.databases.len() == 1 { "database" } else { "databases" }
                            ))
                        })
                        .when(self.connection_state == ConnectionState::Failed, |row| {
                            row.child(
                                Button::new("sidebar-retry-connection")
                                    .ghost()
                                    .small()
                                    .label("Retry")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(id) = this.selected {
                                            this.activate(id, window, cx);
                                        }
                                    })),
                            )
                        }),
                )
            })
            .child(
                h_flex()
                    .px_3()
                    .pt_4()
                    .pb_2()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().muted_foreground)
                            .child("Databases"),
                    )
                    .when(
                        selected_profile.as_ref().is_some_and(|profile| {
                            matches!(profile.config, ConnectionConfig::Postgres { .. })
                        }),
                        |row| {
                            row.child(
                                Button::new("create-database")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Plus)
                                    .label("Create")
                                    .disabled(self.connection_state != ConnectionState::Ready)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.admin(AdminAction::Create, window, cx)
                                    })),
                            )
                        },
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px_2()
                    .when(!self.databases.is_empty(), |view| view.child(explorer))
                    .when(self.databases.is_empty(), |view| {
                        view.child(
                            v_flex()
                                .px_2()
                                .py_3()
                                .gap_2()
                                .text_sm()
                                .line_height(relative(1.6))
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Icon::new(
                                                if self.connection_state
                                                    == ConnectionState::Connecting
                                                {
                                                    IconName::RefreshCw
                                                } else {
                                                    IconName::Database
                                                },
                                            )
                                            .small(),
                                        )
                                        .child(match self.connection_state {
                                            ConnectionState::Connecting => "Loading databases…",
                                            ConnectionState::Failed => "Connection unavailable",
                                            ConnectionState::Ready => "No databases found",
                                            ConnectionState::Disconnected => {
                                                "No connection selected"
                                            }
                                        }),
                                )
                                .child(div().text_xs().child(match self.connection_state {
                                    ConnectionState::Connecting => "This may take a moment.",
                                    ConnectionState::Failed => {
                                        "Check the connection settings, or try connecting again."
                                    }
                                    ConnectionState::Ready => {
                                        "Refresh the connection to check again."
                                    }
                                    ConnectionState::Disconnected => {
                                        "Connect to browse schemas and tables here."
                                    }
                                })),
                        )
                    }),
            )
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(
                        div().flex_1().child(
                            Button::new("history")
                                .outline()
                                .icon(IconName::Clock)
                                .label("Query history")
                                .selected(self.history.is_some())
                                .disabled(self.selected.is_none())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_history(window, cx)
                                })),
                        ),
                    )
                    .child(
                        icon_button("preferences", IconName::Settings, "Preferences")
                            .on_click(cx.listener(|this, _, window, cx| this.settings(window, cx))),
                    ),
            );
        let mut body = v_flex().size_full().min_w_0().child(
            h_flex()
                .min_w_0()
                .h_10()
                .items_end()
                .flex_shrink_0()
                .bg(cx.theme().tab_bar)
                .border_b_1()
                .border_color(cx.theme().border)
                .when(!self.documents.is_empty(), |bar| {
                    bar.child(
                        div().flex_1().min_w_0().child(
                            document_tab_bar(
                                &self.document_scroll,
                                self.documents.iter().enumerate().map(|(index, doc)| {
                                    let title = doc.read(cx).title();
                                    let context = format!(
                                        "{} · {} / {}",
                                        title,
                                        doc.read(cx).profile.name,
                                        doc.read(cx).profile.database()
                                    );
                                    document_tab(
                                        index,
                                        title.clone(),
                                        context,
                                        if doc.read(cx).table_ref().is_some() {
                                            IconName::Table2
                                        } else {
                                            IconName::Code
                                        },
                                        index == self.active,
                                        cx,
                                    )
                                    .set_position(index + 1, self.documents.len())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select_document(index, window, cx)
                                    }))
                                    .on_aux_click(cx.listener(
                                        move |this, event: &ClickEvent, window, cx| {
                                            if event.is_middle_click() {
                                                cx.stop_propagation();
                                                this.close_document(index, window, cx);
                                            }
                                        },
                                    ))
                                    .child(
                                        Button::new(("close-tab", index))
                                            .text()
                                            .small()
                                            .icon(IconName::X)
                                            .accessibility_label(format!("Close {title}"))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.close_document(index, window, cx)
                                            })),
                                    )
                                }),
                                cx,
                            )
                            .when(self.documents.len() > 1, |bar| {
                                bar.child(
                                    Button::new("document-tabs-menu")
                                        .ghost()
                                        .small()
                                        .dropdown_caret(true)
                                        .accessibility_label("Open tabs")
                                        .tooltip("Open tabs")
                                        .dropdown_menu({
                                            let view = cx.entity();
                                            move |menu, _, cx| {
                                                view.update(cx, |this, cx| {
                                                    let mut menu = menu.scrollable(true);
                                                    for (index, doc) in
                                                        this.documents.iter().enumerate()
                                                    {
                                                        menu = menu.item(
                                                            PopupMenuItem::new(
                                                                doc.read(cx).title(),
                                                            )
                                                            .checked(index == this.active)
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.select_document(
                                                                        index, window, cx,
                                                                    )
                                                                },
                                                            )),
                                                        );
                                                    }
                                                    menu
                                                })
                                            }
                                        })
                                        .anchor(Anchor::TopRight),
                                )
                            }),
                        ),
                    )
                })
                .when(self.documents.is_empty(), |bar| {
                    bar.child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .px_5()
                            .gap_2()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(Icon::new(IconName::PanelsTopLeft).small())
                            .child("Workspace"),
                    )
                })
                .child(
                    h_flex().h_9().px_3().flex_shrink_0().child(
                        Button::new("new-query")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .label(if self.profiles.is_empty() {
                                "Add connection"
                            } else {
                                "New query"
                            })
                            .tooltip(if self.profiles.is_empty() {
                                "Add connection · Ctrl+Shift+N"
                            } else {
                                "New query · Ctrl+N"
                            })
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_query(window, cx)),
                            ),
                    ),
                ),
        );
        body = if let Some(history) = self
            .history
            .as_ref()
            .filter(|_| !self.history_docked(window))
        {
            body.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.history_panel(history, cx)),
            )
        } else if let Some(doc) = self.documents.get(self.active) {
            body.child(div().flex_1().min_h_0().child(doc.clone()))
        } else {
            body.child(div().flex_1().min_h_0().child(self.welcome(cx)))
        };
        let mut panels = h_resizable("workspace-panels")
            .on_resize(
                cx.listener(|this, state: &Entity<ResizableState>, window, cx| {
                    if let Some(width) = state.read(cx).sizes().first() {
                        this.preferences.sidebar_width = f32::from(*width);
                        this.schedule_save(window, cx);
                    }
                }),
            )
            .child(
                resizable_panel()
                    .size(px(self.preferences.sidebar_width))
                    .size_range(px(220.)..px(460.))
                    .child(sidebar),
            )
            .child(resizable_panel().child(body));
        if let Some(history) = self
            .history
            .as_ref()
            .filter(|_| self.history_docked(window))
        {
            panels = panels.child(
                resizable_panel()
                    .size(px(320.))
                    .size_range(px(240.)..px(500.))
                    .child(self.history_panel(history, cx)),
            );
        }
        v_flex()
            .id("workspace")
            .key_context("QueriWorkspace")
            .track_focus(&self.focus)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &NewQuery, window, cx| this.new_query(window, cx)))
            .on_action(cx.listener(|this, _: &FindTable, window, cx| this.find_table(window, cx)))
            .on_action(cx.listener(|this, _: &NewConnection, window, cx| {
                this.connection_form(None, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseDocument, window, cx| {
                this.close_document(this.active, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ToggleHistory, window, cx| this.toggle_history(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenTreeItem, window, cx| this.open_tree_item(window, cx)),
            )
            .on_action(cx.listener(|this, _: &NextDocument, window, cx| {
                if !this.documents.is_empty() {
                    this.select_document((this.active + 1) % this.documents.len(), window, cx);
                }
            }))
            .child(div().flex_1().min_h_0().child(panels))
            .when(!self.error.is_empty(), |view| {
                view.child(
                    h_flex()
                        .gap_3()
                        .flex_shrink_0()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .id("workspace-error")
                                .flex_1()
                                .min_w_0()
                                .max_h(px(100.))
                                .overflow_y_scroll()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(self.error.clone()),
                        )
                        .when(
                            self.selected.is_some() && self.databases.is_empty(),
                            |row| {
                                row.child(
                                    Button::new("edit-failed-connection")
                                        .small()
                                        .label("Edit connection")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.connection_form(
                                                this.selected_profile(),
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new("retry-connection")
                                        .small()
                                        .label("Retry")
                                        .disabled(
                                            self.connection_state == ConnectionState::Connecting,
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            if let Some(id) = this.selected {
                                                this.activate(id, window, cx);
                                            }
                                        })),
                                )
                            },
                        )
                        .child(
                            Button::new("dismiss-error")
                                .small()
                                .label("Dismiss")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.error.clear();
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                h_flex()
                    .px_3()
                    .py_1p5()
                    .gap_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .bg(cx.theme().status_bar)
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .child(self.status.clone()),
                    )
                    .child(if self.documents.is_empty() {
                        "No open tabs".into()
                    } else {
                        format!(
                            "{} open {}",
                            self.documents.len(),
                            if self.documents.len() == 1 {
                                "tab"
                            } else {
                                "tabs"
                            }
                        )
                    }),
            )
    }
}
