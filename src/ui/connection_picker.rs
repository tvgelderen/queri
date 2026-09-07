use super::{
    ConnectionState,
    controls::{destructive_menu_item, icon_button, menu_item},
};
use crate::model::{ConnectionConfig, ConnectionProfile};
use gpui_kit::assets::IconName;
use gpui_kit::base::Button as Trigger;
use gpui_kit::component::{
    ActiveTheme, Icon, IndexPath, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    list::{List, ListDelegate, ListItem, ListState},
    menu::DropdownMenu,
    popover::Popover,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub enum ConnectionPickerEvent {
    Selected(Uuid),
    NewConnection,
    EditConnection(Uuid),
    RefreshConnection(Uuid),
    RemoveConnection(Uuid),
}

struct DismissPicker;
struct ConnectionMenuOpen(bool);

#[derive(Clone)]
struct ConnectionItem {
    id: Uuid,
    name: String,
    provider: &'static str,
    endpoint: String,
    subtitle: String,
    logo: &'static str,
}

impl From<&ConnectionProfile> for ConnectionItem {
    fn from(profile: &ConnectionProfile) -> Self {
        let (endpoint, logo) = match &profile.config {
            ConnectionConfig::Postgres { host, port, .. } => {
                (format!("{host}:{port}"), "brand/postgresql.svg")
            }
            ConnectionConfig::Sqlite { path, .. } => {
                (path.to_string_lossy().into_owned(), "brand/sqlite.svg")
            }
        };
        let subtitle = match &profile.config {
            ConnectionConfig::Postgres { .. } => format!("{} · {endpoint}", profile.provider()),
            ConnectionConfig::Sqlite { .. } => format!("SQLite · {}", profile.database()),
        };
        Self {
            id: profile.id,
            name: profile.name.clone(),
            provider: profile.provider(),
            endpoint,
            subtitle,
            logo,
        }
    }
}

impl ConnectionItem {
    fn description(&self) -> String {
        format!("{} · {}", self.provider, self.endpoint)
    }

    fn identity(&self, state: Option<ConnectionState>, cx: &App) -> Div {
        h_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .line_height(relative(1.2))
            .child(
                svg()
                    .path(self.logo)
                    .size_8()
                    .flex_shrink_0()
                    .text_color(cx.theme().primary),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_1p5()
                            .child(
                                div()
                                    .min_w_0()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_ellipsis()
                                    .child(self.name.clone()),
                            )
                            .when_some(state, |row, state| {
                                row.child(
                                    div()
                                        .id("connection-status")
                                        .role(Role::Status)
                                        .aria_label(state.label())
                                        .size_4()
                                        .flex_shrink_0()
                                        .child(
                                            Icon::new(if state == ConnectionState::Ready {
                                                IconName::Plug
                                            } else {
                                                IconName::Unplug
                                            })
                                            .small()
                                            .text_color(match state {
                                                ConnectionState::Ready => cx.theme().foreground,
                                                ConnectionState::Failed => cx.theme().danger,
                                                _ => cx.theme().muted_foreground,
                                            }),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .text_ellipsis()
                            .child(self.subtitle.clone()),
                    ),
            )
    }
}

pub struct ConnectionPicker {
    list: Entity<ListState<Connections>>,
    focus: FocusHandle,
    open: bool,
    menu_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConnectionPickerEvent> for ConnectionPicker {}

impl ConnectionPicker {
    pub fn new(
        profiles: &[ConnectionProfile],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list =
            cx.new(|cx| ListState::new(Connections::new(profiles), window, cx).searchable(true));
        let subscriptions = vec![
            cx.subscribe_in(
                &list,
                window,
                |this, _, event: &ConnectionPickerEvent, window, cx| {
                    this.close(window, cx);
                    cx.emit(*event);
                },
            ),
            cx.subscribe_in(&list, window, |this, _, _: &DismissPicker, window, cx| {
                this.close(window, cx);
            }),
            cx.subscribe(&list, |this, _, event: &ConnectionMenuOpen, cx| {
                this.menu_open = event.0;
                cx.notify();
            }),
        ];
        Self {
            list,
            focus: cx.focus_handle(),
            open: false,
            menu_open: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_profiles(
        &mut self,
        profiles: &[ConnectionProfile],
        selected: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.list.update(cx, |list, _| {
            *list.delegate_mut() = Connections::new(profiles);
        });
        self.set_selected(selected, window, cx);
    }

    pub fn set_selected(
        &mut self,
        selected: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.list.update(cx, |list, cx| {
            list.delegate_mut().active = selected;
            let index = list.delegate().active_index();
            list.set_selected_index(index, window, cx);
            cx.notify();
        });
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    pub(super) fn set_state(&mut self, state: ConnectionState, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| {
            list.delegate_mut().state = state;
            cx.notify();
        });
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.menu_open = false;
        self.focus(window, cx);
        cx.notify();
    }
}

impl Render for ConnectionPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connections = self.list.read(cx).delegate();
        let active = connections
            .items
            .iter()
            .find(|item| Some(item.id) == connections.active);
        let label = active.map_or_else(
            || "Select connection".to_owned(),
            |item| {
                format!(
                    "Switch connection: {} · {} · {}",
                    item.name,
                    item.description(),
                    connections.state.label()
                )
            },
        );
        let identity = active.map_or_else(
            || {
                h_flex()
                    .min_w_0()
                    .gap_3()
                    .child(Icon::new(IconName::Database).small())
                    .child(div().text_sm().child("Select connection"))
            },
            |item| item.identity(Some(connections.state), cx),
        );
        let list = self.list.clone();
        let view = cx.entity();
        let ring = cx.theme().ring;
        let hover = cx.theme().accent;

        Popover::new("connection-popover")
            .open(self.open)
            // The row menu is rendered outside this popover's bounds.
            .overlay_closable(!self.menu_open)
            .on_open_change(cx.listener(|this, open, window, cx| {
                this.open = *open;
                if *open {
                    this.list.update(cx, |list, cx| {
                        list.set_query("", window, cx);
                        list.delegate_mut().filter("");
                        let index = list.delegate().active_index();
                        list.set_selected_index(index, window, cx);
                        cx.notify();
                    });
                } else {
                    this.menu_open = false;
                }
                cx.notify();
            }))
            .track_focus(&self.list.focus_handle(cx))
            .trigger_style(StyleRefinement::default().w_full())
            .trigger(
                Trigger::new("connection-picker")
                    .track_focus(&self.focus)
                    .accessibility_label(label.clone())
                    .w_full()
                    .min_w_0()
                    .min_h(px(64.))
                    .px_3()
                    .py_3()
                    .gap_2()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .focus_visible(move |style| style.border_color(ring))
                    .styles(move |styles| styles.selected(|style| style.border_color(ring)))
                    .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                    .child(div().flex_1().min_w_0().child(identity))
                    .child(
                        Icon::new(IconName::ChevronsUpDown)
                            .small()
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .content(move |_, _, _| {
                let view = view.clone();
                v_flex()
                    .w(px(320.))
                    .gap_3()
                    .child(
                        List::new(&list)
                            .search_placeholder("Search name, type, or host…")
                            .max_h(px(300.)),
                    )
                    .child(
                        Button::new("picker-new-connection")
                            .ghost()
                            .icon(IconName::Plus)
                            .label("New connection")
                            .tooltip("New connection · Ctrl+Shift+N")
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.close(window, cx);
                                    cx.emit(ConnectionPickerEvent::NewConnection);
                                });
                            }),
                    )
            })
    }
}

struct Connections {
    items: Vec<ConnectionItem>,
    matches: Vec<ConnectionItem>,
    cursor: Option<IndexPath>,
    active: Option<Uuid>,
    state: ConnectionState,
}

impl Connections {
    fn new(profiles: &[ConnectionProfile]) -> Self {
        let items = profiles
            .iter()
            .map(ConnectionItem::from)
            .collect::<Vec<_>>();
        Self {
            matches: items.clone(),
            items,
            cursor: None,
            active: None,
            state: ConnectionState::Disconnected,
        }
    }

    fn filter(&mut self, query: &str) {
        let query = query.trim().to_lowercase();
        self.matches = self
            .items
            .iter()
            .filter(|item| {
                format!("{} {} {}", item.name, item.provider, item.endpoint)
                    .to_lowercase()
                    .contains(&query)
            })
            .cloned()
            .collect();
    }

    fn active_index(&self) -> Option<IndexPath> {
        self.matches
            .iter()
            .position(|item| Some(item.id) == self.active)
            .map(IndexPath::new)
    }
}

impl EventEmitter<ConnectionPickerEvent> for ListState<Connections> {}
impl EventEmitter<DismissPicker> for ListState<Connections> {}
impl EventEmitter<ConnectionMenuOpen> for ListState<Connections> {}

impl ListDelegate for Connections {
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
        // The list resets its cursor before completing the search task.
        cx.spawn_in(window, async move |list, cx| {
            let _ = list.update_in(cx, |list, window, cx| {
                list.delegate_mut().filter(&query);
                let selected = if query.is_empty() {
                    list.delegate().active_index()
                } else {
                    (!list.delegate().matches.is_empty()).then_some(IndexPath::default())
                };
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
        self.cursor = index;
    }

    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        if let Some(item) = self.cursor.and_then(|index| self.matches.get(index.row)) {
            cx.emit(ConnectionPickerEvent::Selected(item.id));
        }
    }

    fn cancel(&mut self, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        cx.emit(DismissPicker);
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let item = self.matches.get(index.row)?;
        let label = format!("{} · {}", item.name, item.description());
        let id = item.id;
        let list = cx.entity();
        Some(
            ListItem::new(index)
                .selected(self.cursor == Some(index))
                .accessibility_label(label.clone())
                .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .py_1()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(item.identity(None, cx)))
                        .child(
                            div()
                                .size_4()
                                .flex_shrink_0()
                                .when(Some(item.id) == self.active, |check| {
                                    check.child(Icon::new(IconName::Check).small())
                                }),
                        )
                        .when(Some(id) == self.active, |row| {
                            let open_list = list.clone();
                            row.child(
                                icon_button(
                                    "connection-menu",
                                    IconName::Ellipsis,
                                    format!("Actions for {}", item.name),
                                )
                                .small()
                                .dropdown_menu(move |menu, _, cx| {
                                    let edit = list.clone();
                                    let refresh = list.clone();
                                    let remove = list.clone();
                                    menu.item(
                                        menu_item("Edit connection…", IconName::Pencil).on_click(
                                            move |_, _, cx| {
                                                edit.update(cx, |_, cx| {
                                                    cx.emit(ConnectionPickerEvent::EditConnection(
                                                        id,
                                                    ));
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        menu_item("Refresh connection", IconName::RefreshCw)
                                            .disabled(
                                                list.read(cx).delegate().state
                                                    == ConnectionState::Connecting,
                                            )
                                            .on_click(move |_, _, cx| {
                                                refresh.update(cx, |_, cx| {
                                                    cx.emit(
                                                        ConnectionPickerEvent::RefreshConnection(
                                                            id,
                                                        ),
                                                    );
                                                });
                                            }),
                                    )
                                    .separator()
                                    .item(
                                        destructive_menu_item("Remove connection…", false)
                                            .on_click(move |_, _, cx| {
                                                remove.update(cx, |_, cx| {
                                                    cx.emit(
                                                        ConnectionPickerEvent::RemoveConnection(id),
                                                    );
                                                });
                                            }),
                                    )
                                })
                                .on_open_change(
                                    move |open, _, cx| {
                                        open_list
                                            .update(cx, |_, cx| cx.emit(ConnectionMenuOpen(*open)));
                                    },
                                ),
                            )
                        }),
                ),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        v_flex()
            .px_3()
            .py_6()
            .gap_2()
            .text_sm()
            .child("No matching connections")
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Try another name, database type, or host."),
            )
    }
}
