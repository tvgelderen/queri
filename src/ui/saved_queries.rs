use super::{
    Services,
    controls::{choice, destructive_menu_item, icon_button, menu_item},
    field, task,
};
use crate::model::{ConnectionProfile, SavedQuery};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::DropdownMenu,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use uuid::Uuid;

pub struct QuerySaved(pub SavedQuery);
pub struct OpenQuery(pub SavedQuery);
pub struct QueryForm {
    services: Services,
    query: SavedQuery,
    name: Entity<InputState>,
    busy: bool,
    error: String,
}
impl EventEmitter<QuerySaved> for QueryForm {}
pub fn edit(
    services: Services,
    query: SavedQuery,
    window: &mut Window,
    cx: &mut App,
) -> Entity<QueryForm> {
    let form = cx.new(|cx| QueryForm {
        services,
        name: cx.new(|cx| InputState::new(window, cx).default_value(&query.name)),
        query,
        busy: false,
        error: String::new(),
    });
    let view = form.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("Save query").child(view.clone())
    });
    form
}
impl Render for QueryForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_4()
            .child(field("Name", &self.name))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Saved locally. Opening a saved query never runs it."),
            )
            .child(
                div()
                    .id("save-query-error")
                    .role(Role::Alert)
                    .text_sm()
                    .children((!self.error.is_empty()).then(|| self.error.clone())),
            )
            .child(
                Button::new("confirm-save-query")
                    .primary()
                    .label(if self.busy { "Saving…" } else { "Save query" })
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.query.name = this.name.read(cx).value().trim().to_owned();
                        if this.query.name.is_empty() {
                            this.error = "Enter a query name.".into();
                            this.name.read(cx).focus_handle(cx).focus(window, cx);
                            cx.notify();
                            return;
                        }
                        this.busy = true;
                        this.error.clear();
                        let store = this.services.store.clone();
                        let query = this.query.clone();
                        task(
                            &this.services,
                            window,
                            cx,
                            async move {
                                tokio::task::spawn_blocking(move || store.save_query(&query))
                                    .await?
                            },
                            |this, result, window, cx| {
                                this.busy = false;
                                match result {
                                    Ok(query) => {
                                        cx.emit(QuerySaved(query));
                                        window.close_dialog(cx);
                                    }
                                    Err(error) => this.error = format!("{error:#}"),
                                }
                                cx.notify();
                            },
                        );
                        cx.notify();
                    })),
            )
    }
}
pub struct SavedQueries {
    services: Services,
    profiles: Vec<ConnectionProfile>,
    search: Entity<InputState>,
    connection: Option<Uuid>,
    entries: Vec<SavedQuery>,
    busy: bool,
    error: String,
    subscriptions: Vec<Subscription>,
}
impl EventEmitter<OpenQuery> for SavedQueries {}
impl SavedQueries {
    pub fn new(
        services: Services,
        profiles: Vec<ConnectionProfile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search names or SQL…"));
        let subscription = cx.subscribe_in(
            &search,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.load(window, cx);
                }
            },
        );
        Self {
            services,
            profiles,
            search,
            connection: None,
            entries: vec![],
            busy: false,
            error: String::new(),
            subscriptions: vec![subscription],
        }
    }
    pub fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error.clear();
        let store = self.services.store.clone();
        let search = self.search.read(cx).value().to_string();
        let connection = self.connection;
        task(
            &self.services,
            window,
            cx,
            async move {
                tokio::task::spawn_blocking(move || store.saved_queries(&search, connection))
                    .await?
            },
            |this, result, _, cx| {
                this.busy = false;
                match result {
                    Ok(entries) => this.entries = entries,
                    Err(e) => this.error = format!("Could not load queries: {e:#}"),
                };
                cx.notify();
            },
        );
        cx.notify();
    }
    fn edit(&mut self, query: SavedQuery, window: &mut Window, cx: &mut Context<Self>) {
        let form = edit(self.services.clone(), query, window, cx);
        self.subscriptions.push(cx.subscribe_in(
            &form,
            window,
            |this, _, _: &QuerySaved, window, cx| this.load(window, cx),
        ));
    }
    fn favorite(&mut self, mut query: SavedQuery, window: &mut Window, cx: &mut Context<Self>) {
        query.favorite = !query.favorite;
        self.busy = true;
        let store = self.services.store.clone();
        task(
            &self.services,
            window,
            cx,
            async move { tokio::task::spawn_blocking(move || store.save_query(&query)).await? },
            |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok(_) => this.load(window, cx),
                    Err(e) => this.error = format!("{e:#}"),
                };
                cx.notify();
            },
        );
        cx.notify();
    }
    fn delete(&mut self, query: SavedQuery, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .title("Delete saved query?")
                .child(format!(
                    "“{}” will be removed. Open SQL drafts are kept.",
                    query.name
                ))
                .button_props(
                    gpui_kit::component::dialog::DialogButtonProps::default()
                        .ok_text("Delete query")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        let store = this.services.store.clone();
                        let id = query.id;
                        this.busy = true;
                        task(
                            &this.services,
                            window,
                            cx,
                            async move {
                                tokio::task::spawn_blocking(move || store.delete_query(id)).await?
                            },
                            |this, result, window, cx| {
                                this.busy = false;
                                match result {
                                    Ok(()) => this.load(window, cx),
                                    Err(e) => this.error = format!("{e:#}"),
                                };
                                cx.notify();
                            },
                        );
                    });
                    true
                })
        });
    }
}
impl Render for SavedQueries {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut connections = vec![(None, SharedString::from("All connections"))];
        connections.extend(
            self.profiles
                .iter()
                .map(|p| (Some(p.id), p.name.clone().into())),
        );
        let label = self
            .profiles
            .iter()
            .find(|p| Some(p.id) == self.connection)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "All connections".into());
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&self.search).aria_label("Search saved queries; press Enter"))
                    .child(
                        icon_button("search-saved", IconName::Search, "Search saved queries")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
                    ),
            )
            .child(choice(
                Button::new("saved-connection")
                    .label(label)
                    .disabled(self.busy),
                self.connection,
                connections,
                cx.listener(|this, value, window, cx| {
                    this.connection = *value;
                    this.load(window, cx);
                }),
            ))
            .child(
                div()
                    .id("saved-query-status")
                    .role(Role::Status)
                    .text_sm()
                    .child(if self.busy {
                        "Loading…".into()
                    } else {
                        self.error.clone()
                    }),
            )
            .child(
                v_flex()
                    .id("saved-query-list")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .gap_2()
                    .when(self.entries.is_empty() && !self.busy, |view| {
                        view.child(
                            v_flex()
                                .items_center()
                                .gap_2()
                                .py_8()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size_10()
                                        .rounded(cx.theme().radius_lg)
                                        .bg(super::theme::accent_soft(cx))
                                        .child(
                                            Icon::new(IconName::Bookmark)
                                                .text_color(super::theme::accent_text(cx)),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("No saved queries yet"),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Save SQL from an editor or from query history."),
                                ),
                        )
                    })
                    .children(self.entries.iter().map(|query| {
                        let open = query.clone();
                        let favorite = query.clone();
                        let rename = query.clone();
                        let mut duplicate = query.clone();
                        let delete = query.clone();
                        duplicate.id = Uuid::new_v4();
                        duplicate.revision = 0;
                        duplicate.name = format!("{} copy", duplicate.name);
                        let context = query
                            .connection_id
                            .and_then(|id| self.profiles.iter().find(|p| p.id == id))
                            .map(|p| format!("{} · {}", p.name, query.database))
                            .unwrap_or_else(|| "Choose a connection when opening".into());
                        v_flex()
                            .gap_1()
                            .py_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "open-{}",
                                            query.id
                                        )))
                                        .ghost()
                                        .label(query.name.clone())
                                        .on_click(
                                            cx.listener(move |_, _, _, cx| {
                                                cx.emit(OpenQuery(open.clone()))
                                            }),
                                        ),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "favorite-{}",
                                            query.id
                                        )))
                                        .ghost()
                                        .small()
                                        .label(if query.favorite {
                                            "Unfavorite"
                                        } else {
                                            "Favorite"
                                        })
                                        .disabled(self.busy)
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.favorite(favorite.clone(), window, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        icon_button(
                                            SharedString::from(format!(
                                                "saved-actions-{}",
                                                query.id
                                            )),
                                            IconName::Ellipsis,
                                            "Saved query actions",
                                        )
                                        .disabled(self.busy)
                                        .dropdown_menu({
                                            let view = cx.entity();
                                            move |menu, _, cx| {
                                                let rename = rename.clone();
                                                let duplicate = duplicate.clone();
                                                let delete = delete.clone();
                                                view.update(cx, |this, cx| {
                                                    menu.item(
                                                        menu_item("Rename…", IconName::Pencil)
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.edit(
                                                                        rename.clone(),
                                                                        window,
                                                                        cx,
                                                                    )
                                                                },
                                                            )),
                                                    )
                                                    .item(
                                                        menu_item("Duplicate…", IconName::Copy)
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.edit(
                                                                        duplicate.clone(),
                                                                        window,
                                                                        cx,
                                                                    )
                                                                },
                                                            )),
                                                    )
                                                    .separator()
                                                    .item(
                                                        destructive_menu_item(
                                                            "Delete query…",
                                                            this.busy,
                                                        )
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.delete(
                                                                    delete.clone(),
                                                                    window,
                                                                    cx,
                                                                )
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
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(context),
                            )
                    })),
            )
    }
}
