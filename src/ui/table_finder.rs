use super::{Services, controls::empty_state, task};
use crate::{
    database::Session,
    model::{ConnectionProfile, TableRef},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, IndexPath, Sizable, WindowExt,
    button::Button,
    h_flex,
    list::{List, ListDelegate, ListItem, ListState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::*;

pub struct OpenTable(pub TableRef);
impl EventEmitter<OpenTable> for TableFinder {}

enum LoadState {
    Loading,
    Ready,
    Failed(String),
}

pub struct TableFinder {
    services: Services,
    profile: ConnectionProfile,
    password: String,
    state: LoadState,
    list: Entity<ListState<Tables>>,
    _subscription: Subscription,
}

impl TableFinder {
    pub fn new(
        services: Services,
        profile: ConnectionProfile,
        password: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = cx.new(|cx| ListState::new(Tables::default(), window, cx).searchable(true));
        let subscription = cx.subscribe_in(&list, window, |_, _, event: &OpenTable, window, cx| {
            window.close_dialog(cx);
            cx.emit(OpenTable(event.0.clone()));
        });
        let mut finder = Self {
            services,
            profile,
            password,
            state: LoadState::Loading,
            list,
            _subscription: subscription,
        };
        finder.load(window, cx);
        finder
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state = LoadState::Loading;
        let profile = self.profile.clone();
        let password = self.password.clone();
        task(
            &self.services,
            window,
            cx,
            async move {
                let session = Session::connect(profile, password).await?;
                let mut tables = vec![];
                // Load every schema in this database, even if its tree branch was never opened.
                for schema in session.schemas().await? {
                    tables.extend(session.tables(schema).await?);
                }
                tables.sort_by_cached_key(|table| {
                    (table.name.to_lowercase(), table.schema.to_lowercase())
                });
                Ok(tables)
            },
            |this, result, window, cx| {
                match result {
                    Ok(tables) => {
                        this.state = LoadState::Ready;
                        this.list.update(cx, |list, cx| {
                            list.delegate_mut().items = tables;
                            list.delegate_mut().filter("");
                            if !list.delegate().matches.is_empty() {
                                list.set_selected_index(Some(IndexPath::default()), window, cx);
                            }
                            list.focus(window, cx);
                            cx.notify();
                        });
                    }
                    Err(error) => {
                        this.state = LoadState::Failed(format!(
                            "Could not load tables. Check the connection and database permissions, then retry.\n\n{error:#}"
                        ))
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }
}

impl Render for TableFinder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.state {
            LoadState::Loading => empty_state(
                IconName::Search,
                "Finding your tables",
                "Loading tables and views across all schemas in this database.",
            )
            .into_any_element(),
            LoadState::Failed(error) => {
                empty_state(IconName::Unplug, "Tables unavailable", error.clone())
                    .child(
                        Button::new("retry-find-tables")
                            .label("Retry")
                            .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
                    )
                    .into_any_element()
            }
            LoadState::Ready => List::new(&self.list)
                .search_placeholder("Search tables and schemas…")
                .into_any_element(),
        };
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} / {}",
                        self.profile.name,
                        self.profile.database()
                    )),
            )
            .child(div().h(px(320.)).child(body))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("↑ ↓ to navigate · Enter to open · Esc to close"),
            )
    }
}

#[derive(Default)]
struct Tables {
    items: Vec<TableRef>,
    matches: Vec<TableRef>,
    selected: Option<IndexPath>,
}

impl Tables {
    fn filter(&mut self, query: &str) {
        let query = query.trim().to_lowercase();
        self.matches = self
            .items
            .iter()
            .filter(|table| {
                format!("{}.{}", table.schema, table.name)
                    .to_lowercase()
                    .contains(&query)
            })
            .cloned()
            .collect();
    }
}

impl EventEmitter<OpenTable> for ListState<Tables> {}

impl ListDelegate for Tables {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.matches.len()
    }

    fn perform_search(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        let query = query.to_owned();
        // Publish results and selection together, after the list resets its old
        // selection, so Enter works immediately after an empty search too.
        cx.spawn_in(window, async move |list, cx| {
            let _ = list.update_in(cx, |list, window, cx| {
                list.delegate_mut().filter(&query);
                let selected =
                    (!list.delegate().matches.is_empty()).then_some(IndexPath::default());
                list.set_selected_index(selected, window, cx);
                cx.notify();
            });
        })
    }

    fn set_selected_index(
        &mut self,
        index: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected = index;
    }

    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        if let Some(table) = self.selected.and_then(|index| self.matches.get(index.row)) {
            cx.emit(OpenTable(table.clone()));
        }
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        window.close_dialog(cx);
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let table = self.matches.get(index.row)?;
        let label = format!("{}.{}", table.schema, table.name);
        Some(
            ListItem::new(index)
                .selected(self.selected == Some(index))
                .accessibility_label(format!("Open {}.{}", table.schema, table.name))
                .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                .child(
                    h_flex()
                        .min_w_0()
                        .w_full()
                        .gap_3()
                        .py_1()
                        .child(
                            Icon::new(if table.kind == "table" {
                                IconName::Table2
                            } else {
                                IconName::Eye
                            })
                            .small(),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_ellipsis()
                                .text_sm()
                                .child(table.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(table.schema.clone()),
                        ),
                ),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        empty_state(
            IconName::Search,
            "No matching tables",
            "Try another table or schema name, or clear the search.",
        )
    }
}
