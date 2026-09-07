use super::{
    Services,
    controls::{destructive_menu_item, empty_state, icon_button, menu_item},
    task,
};
use crate::model::HistoryEntry;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    menu::DropdownMenu,
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use uuid::Uuid;

#[cfg(all(test, feature = "test-support"))]
#[path = "history_tests.rs"]
mod tests;

pub struct ReplayQuery(pub HistoryEntry);
impl EventEmitter<ReplayQuery> for History {}
pub struct NewHistoryQuery;
impl EventEmitter<NewHistoryQuery> for History {}

pub struct History {
    services: Services,
    connection: Uuid,
    search: Entity<InputState>,
    applied_search: String,
    entries: Vec<HistoryEntry>,
    offset: u64,
    busy: bool,
    error: String,
    _subscription: Subscription,
}

impl History {
    pub fn new(
        services: Services,
        connection: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search SQL…"));
        let subscription = cx.subscribe_in(
            &search,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.offset = 0;
                    this.load(window, cx);
                }
            },
        );
        Self {
            services,
            connection,
            search,
            applied_search: String::new(),
            entries: vec![],
            offset: 0,
            busy: false,
            error: String::new(),
            _subscription: subscription,
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
    }

    pub fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let store = self.services.store.clone();
        let connection = self.connection;
        let search = self.search.read(cx).value().to_string();
        self.applied_search = search.clone();
        let offset = self.offset;
        task(
            &self.services,
            window,
            cx,
            async move {
                tokio::task::spawn_blocking(move || store.history(connection, &search, offset))
                    .await?
            },
            |this, result, _, cx| {
                this.busy = false;
                match result {
                    Ok(entries) => {
                        this.entries = entries;
                        this.error.clear();
                    }
                    Err(error) => {
                        this.error = format!(
                            "Could not load history: {error:#}. Retry using Search history."
                        )
                    }
                };
                cx.notify();
            },
        );
        cx.notify();
    }
    fn delete(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.services.store.clone();
        self.busy = true;
        task(
            &self.services,
            window,
            cx,
            async move { tokio::task::spawn_blocking(move || store.delete_history(id)).await? },
            |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => this.load(window, cx),
                    Err(error) => {
                        this.error = format!(
                            "Could not delete the query: {error:#}. Retry from the query's actions menu."
                        )
                    }
                };
                cx.notify();
            },
        );
        cx.notify();
    }
    fn confirm_delete(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        self.search.read(cx).focus_handle(cx).focus(window, cx);
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .title("Delete this query from history?")
                .child(
                    "The recorded query will be removed. Your database and open SQL tabs are kept.",
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete query")
                        .ok_variant(ButtonVariant::Danger)
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    view.update(cx, |this, cx| this.delete(id, window, cx));
                    true
                })
        });
    }
    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
        let view = cx.entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .title("Clear connection history?")
                .child("All recorded queries for this connection will be removed.")
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Clear history")
                        .ok_variant(ButtonVariant::Danger)
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    view.update(cx, |this, cx| {
                        let store = this.services.store.clone();
                        let connection = this.connection;
                        this.busy = true;
                        task(
                            &this.services,
                            window,
                            cx,
                            async move {
                                tokio::task::spawn_blocking(move || store.clear_history(connection))
                                    .await?
                            },
                            |this, result, window, cx| {
                                this.busy = false;
                                match result {
                                    Ok(()) => {
                                        this.offset = 0;
                                        this.load(window, cx);
                                    }
                                    Err(error) => {
                                        this.error = format!("Could not clear history: {error:#}. Retry from History actions.");
                                    }
                                };
                                cx.notify();
                            },
                        );
                    });
                    true
                })
        });
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        self.offset = 0;
        self.load(window, cx);
    }

    fn empty(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.busy {
            return empty_state(
                IconName::Clock,
                "Loading history…",
                "Fetching queries for this connection.",
            )
            .into_any_element();
        }
        if !self.error.is_empty() {
            return empty_state(
                IconName::Clock,
                "History unavailable",
                "Refresh to try loading your query history again.",
            )
            .child(
                Button::new("retry-history")
                    .label("Refresh history")
                    .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
            )
            .into_any_element();
        }
        if !self.applied_search.is_empty() {
            return empty_state(
                IconName::Search,
                "No matching queries",
                format!("No saved queries contain “{}”.", self.applied_search),
            )
            .child(
                Button::new("empty-clear-history-search")
                    .label("Clear search")
                    .on_click(cx.listener(|this, _, window, cx| this.clear_search(window, cx))),
            )
            .into_any_element();
        }
        empty_state(
            IconName::Clock,
            "No queries yet",
            "Queries you run on this connection are saved here for later.",
        )
        .child(
            Button::new("history-new-query")
                .ghost()
                .small()
                .icon(IconName::Plus)
                .label("New query")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(NewHistoryQuery))),
        )
        .into_any_element()
    }
}

impl Render for History {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .gap_3()
            .p_4()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .id("history-search")
                                .prefix(Icon::new(IconName::Search).small())
                                .aria_label("Search query history; press Enter to search"),
                        ),
                    )
                    .when(!self.search.read(cx).value().is_empty(), |row| {
                        row.child(
                            icon_button("clear-history-search", IconName::X, "Clear search")
                                .disabled(self.busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.clear_search(window, cx)
                                    }),
                                ),
                        )
                    })
                    .child(
                        icon_button("search-history", IconName::Search, "Search history · Enter")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.offset = 0;
                                this.load(window, cx);
                            })),
                    )
                    .child(
                        icon_button("history-menu", IconName::Ellipsis, "History actions")
                            .dropdown_menu({
                                let view = cx.entity();
                                move |menu, _, cx| {
                                    view.update(cx, |this, cx| {
                                        menu.item(
                                            destructive_menu_item(
                                                "Clear connection history…",
                                                this.busy,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, window, cx| {
                                                    this.clear(window, cx)
                                                }),
                                            ),
                                        )
                                    })
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Press Enter to search this connection."),
            )
            .when(!self.entries.is_empty(), |view| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Select a query to open it without running it."),
                )
            })
            .child(
                v_flex()
                    .id("history-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(v_flex().gap_2().children(self.entries.iter().map(|entry| {
                        let replay = entry.clone();
                        let copy = entry.sql.clone();
                        let id = entry.id;
                        let view = cx.entity();
                        let time = chrono::DateTime::parse_from_rfc3339(&entry.executed_at)
                            .map(|time| {
                                time.with_timezone(&chrono::Local)
                                    .format("%b %-d · %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_else(|_| entry.executed_at.clone());
                        ListItem::new(SharedString::from(format!("history-{id}")))
                            .accessibility_label(format!(
                                "Open query from {}: {}",
                                entry.executed_at, entry.sql
                            ))
                            .on_click(
                                cx.listener(move |_, _, _, cx| {
                                    cx.emit(ReplayQuery(replay.clone()))
                                }),
                            )
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .gap_1p5()
                                    .py_2()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .flex_1()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_ellipsis()
                                                    .child(entry.database.clone()),
                                            )
                                            .child(
                                                icon_button(
                                                    SharedString::from(format!(
                                                        "history-actions-{id}"
                                                    )),
                                                    IconName::Ellipsis,
                                                    "Query actions",
                                                )
                                                .dropdown_menu(move |menu, _, cx| {
                                                    view.update(cx, |this, cx| {
                                                        let copy = copy.clone();
                                                        menu.item(
                                                            menu_item("Copy SQL", IconName::Copy)
                                                                .on_click(move |_, _, cx| {
                                                                    cx.write_to_clipboard(
                                                                        ClipboardItem::new_string(
                                                                            copy.clone(),
                                                                        ),
                                                                    )
                                                                }),
                                                        )
                                                        .separator()
                                                        .item(
                                                            destructive_menu_item(
                                                                "Delete from history…",
                                                                this.busy,
                                                            )
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.confirm_delete(
                                                                        id, window, cx,
                                                                    )
                                                                },
                                                            )),
                                                        )
                                                    })
                                                }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(time),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_family(cx.theme().mono_font_family.clone())
                                            .child(format!(
                                                "{}{}",
                                                entry.sql.chars().take(140).collect::<String>(),
                                                if entry.sql.chars().count() > 140 {
                                                    "…"
                                                } else {
                                                    ""
                                                }
                                            )),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .text_xs()
                                            .font_family(cx.theme().mono_font_family.clone())
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                Icon::new(if entry.error.is_some() {
                                                    IconName::CircleAlert
                                                } else {
                                                    IconName::Check
                                                })
                                                .small(),
                                            )
                                            .child(format!(
                                                "{} · {} ms",
                                                if entry.error.is_some() {
                                                    "Failed"
                                                } else {
                                                    "Completed"
                                                },
                                                entry.elapsed_ms
                                            )),
                                    ),
                            )
                            .into_any_element()
                    })))
                    .when(self.entries.is_empty(), |list| list.child(self.empty(cx))),
            )
            .when(!self.error.is_empty(), |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(self.error.clone()),
                )
            })
            .when(self.offset > 0 || self.entries.len() >= 100, |view| {
                view.child(
                    h_flex()
                        .gap_2()
                        .child(
                            icon_button(
                                "history-previous",
                                IconName::ChevronLeft,
                                "Previous history page",
                            )
                            .disabled(self.busy || self.offset == 0)
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.offset = this.offset.saturating_sub(100);
                                    this.load(window, cx);
                                },
                            )),
                        )
                        .child(
                            icon_button(
                                "history-next",
                                IconName::ChevronRight,
                                "Next history page",
                            )
                            .disabled(self.busy || self.entries.len() < 100)
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.offset += 100;
                                    this.load(window, cx);
                                },
                            )),
                        ),
                )
            })
    }
}
