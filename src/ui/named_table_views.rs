use super::*;
use uuid::Uuid;

struct ViewSaved(SavedTableView);
struct ViewOpened(SavedTableView);
struct SaveView {
    services: Services,
    view: SavedTableView,
    name: Entity<InputState>,
    include_scope: Option<bool>,
    busy: bool,
    error: String,
}
impl EventEmitter<ViewSaved> for SaveView {}

impl Render for SaveView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().gap_3()
            .child(Input::new(&self.name).id("saved-view-name").aria_label("View name"))
            .child(div().text_sm().child("Saves applied filters, ordered sorts, column layout and page size. Unapplied filter edits are not saved."))
            .when(self.view.relationship.is_some(), |view| view
                .child(div().text_sm().child("This tab has record-specific relationship scope. Choose what to save:"))
                .child(choice(Button::new("saved-view-scope").label(match self.include_scope { None => "Choose relationship scope…", Some(false) => "Reusable settings only", Some(true) => "Include this record’s relationship scope" }), self.include_scope, [(Some(false), SharedString::from("Reusable settings only")), (Some(true), SharedString::from("Include this record’s relationship scope"))], cx.listener(|this, scope, _, cx| { this.include_scope = *scope; cx.notify(); }))))
            .child(div().id("saved-view-error").test_support().role(Role::Alert).aria_label(self.error.clone()).text_sm().text_color(cx.theme().danger).child(self.error.clone()))
            .child(Button::new("confirm-save-view").primary().label("Save view").disabled(self.busy).on_click(cx.listener(|this, _, window, cx| {
                this.view.name = this.name.read(cx).value().trim().to_owned();
                if this.view.name.is_empty() { this.error = "Enter a view name.".into(); this.name.read(cx).focus_handle(cx).focus(window, cx); cx.notify(); return; }
                if this.view.relationship.is_some() && this.include_scope.is_none() { this.error = "Choose whether to include the record-specific relationship scope.".into(); cx.notify(); return; }
                let mut view = this.view.clone();
                if this.include_scope != Some(true) { view.relationship = None; }
                let store = this.services.store.clone(); this.busy = true; this.error.clear();
                task(&this.services, window, cx, async move { tokio::task::spawn_blocking(move || store.save_table_view(&view)).await? }, |this, result, window, cx| {
                    this.busy = false;
                    match result { Ok(view) => { cx.emit(ViewSaved(view)); window.close_dialog(cx); }, Err(error) => this.error = format!("{error:#}") }
                    cx.notify();
                }); cx.notify();
            })))
    }
}

struct ViewLibrary {
    services: Services,
    connection: Uuid,
    database: String,
    table: TableRef,
    entries: Vec<SavedTableView>,
    busy: bool,
    error: String,
}
impl EventEmitter<ViewOpened> for ViewLibrary {}
impl ViewLibrary {
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.services.store.clone();
        self.busy = true;
        task(
            &self.services,
            window,
            cx,
            async move { tokio::task::spawn_blocking(move || store.saved_table_views()).await? },
            |this, result, _, cx| {
                this.busy = false;
                match result {
                    Ok(entries) => this.entries = entries,
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            },
        );
    }
}
impl Render for ViewLibrary {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().gap_2()
            .child(div().text_sm().child("Load replaces this tab’s settings and relationship scope. Views remain associated with their original connection and table. Open that table to load its views."))
            .child(div().id("saved-view-library-error").role(Role::Alert).child(self.error.clone()))
            .child(v_flex().id("saved-view-list").max_h(px(360.)).overflow_y_scroll().gap_2().children(self.entries.iter().map(|entry| {
                let available = entry.connection_id == Some(self.connection) && entry.database == self.database && entry.table == self.table;
                let selected = entry.clone(); let delete = entry.clone();
                v_flex().gap_1()
                    .child(div().text_sm().child(format!("{} · {}.{}.{}{}{}", entry.name, entry.database, entry.table.schema, entry.table.name, if entry.connection_id.is_none() { " · connection removed (retained)" } else if !available { " · another connection/table" } else { "" }, if entry.relationship.is_some() { " · includes record scope" } else { " · reusable settings" })))
                    .child(h_flex().gap_2()
                        .child(Button::new(SharedString::from(format!("load-view-{}", entry.id))).small().label("Load view").disabled(self.busy || !available).on_click(cx.listener(move |_, _, window, cx| { cx.emit(ViewOpened(selected.clone())); window.close_dialog(cx); })))
                        .child(Button::new(SharedString::from(format!("delete-view-{}", entry.id))).small().label("Delete view").disabled(self.busy).on_click(cx.listener(move |this, _, window, cx| {
                            let store = this.services.store.clone(); let id = delete.id; let revision = delete.revision; this.busy = true;
                            task(&this.services, window, cx, async move { tokio::task::spawn_blocking(move || store.delete_table_view(id, revision)).await? }, |this, result, window, cx| {
                                this.busy = false; match result { Ok(()) => this.load(window, cx), Err(error) => this.error = error.to_string() } cx.notify();
                            }); cx.notify();
                        }))))
            })))
            .when(self.entries.is_empty() && !self.busy, |view| view.child("No saved table views."))
    }
}

impl Document {
    pub fn restore_saved_table_view(&mut self, saved: Option<SavedTableView>) {
        if let DocumentKind::Table { saved_view, .. } = &mut self.kind {
            *saved_view = saved.map(Box::new);
        }
    }

    fn save_named_view(&mut self, new: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(settings) = self.table_view_settings() else {
            return;
        };
        let DocumentKind::Table {
            saved_view,
            table,
            options,
            ..
        } = &self.kind
        else {
            return;
        };
        let mut view = saved_view
            .as_deref()
            .cloned()
            .filter(|_| !new)
            .unwrap_or_else(|| SavedTableView {
                id: Uuid::new_v4(),
                name: String::new(),
                connection_id: Some(self.profile.id),
                database: self.profile.database(),
                table: table.clone(),
                settings: settings.clone(),
                relationship: None,
                revision: 0,
            });
        // Preserve the loaded association and revision; the store refuses stale or reassigned updates.
        view.settings = settings;
        view.relationship = options.relationship.clone();
        let services = self.services.clone();
        let form = cx.new(|cx| SaveView {
            services,
            name: cx.new(|cx| InputState::new(window, cx).default_value(&view.name)),
            include_scope: None,
            view,
            busy: false,
            error: String::new(),
        });
        self.subscriptions
            .push(cx.subscribe(&form, |this, _, event: &ViewSaved, cx| {
                this.restore_saved_table_view(Some(event.0.clone()));
                cx.emit(DocumentChanged);
                cx.notify();
            }));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title("Save table view").child(form.clone())
        });
    }

    fn open_named_views(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_edits(cx) {
            self.resolve_edits(window, cx, |this, w, cx| this.open_named_views(w, cx));
            return;
        }
        let Some(table) = self.table_ref().cloned() else {
            return;
        };
        let services = self.services.clone();
        let connection = self.profile.id;
        let database = self.profile.database();
        let library = cx.new(|cx| {
            let mut library = ViewLibrary {
                services,
                connection,
                database,
                table,
                entries: vec![],
                busy: false,
                error: String::new(),
            };
            library.load(window, cx);
            library
        });
        self.subscriptions.push(cx.subscribe_in(&library, window, |this, _, event: &ViewOpened, window, cx| {
            if this.busy() { return; }
            let view = &event.0;
            if view.connection_id != Some(this.profile.id) || view.database != this.profile.database() || this.table_ref() != Some(&view.table) {
                this.error = "This view belongs to another connection or table. Open its original table.".into(); cx.notify(); return;
            }
            this.restore_table_view(Some(view.settings.clone()));
            this.set_relationship(view.relationship.clone());
            this.restore_saved_table_view(Some(view.clone()));
            cx.emit(DocumentChanged);
            this.refresh(window, cx);
        }));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title("Saved table views").child(library.clone())
        });
    }

    pub(super) fn named_view_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let DocumentKind::Table { saved_view, .. } = &self.kind else {
            return Empty.into_any_element();
        };
        let loaded = saved_view.is_some();
        Button::new("table-views-menu")
            .ghost()
            .small()
            .icon(IconName::Bookmark)
            .label(
                saved_view
                    .as_ref()
                    .map(|view| view.name.clone())
                    .unwrap_or_else(|| "Views".into()),
            )
            .accessibility_label("Saved views")
            .when_some(saved_view.as_ref(), |button, view| {
                button.tooltip(format!("{} · revision {}", view.name, view.revision))
            })
            .dropdown_caret(true)
            .disabled(self.busy())
            .dropdown_menu({
                let view = cx.entity();
                move |menu, _, cx| {
                    view.update(cx, |_, cx| {
                        menu.min_w(px(200.))
                            .item(menu_item("Load view…", IconName::FolderOpen).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.open_named_views(window, cx)
                                }),
                            ))
                            .separator()
                            .item(
                                menu_item("Save view", IconName::Save)
                                    .disabled(!loaded)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_named_view(false, window, cx)
                                    })),
                            )
                            .item(
                                menu_item("Save view as…", IconName::Plus).on_click(cx.listener(
                                    |this, _, window, cx| this.save_named_view(true, window, cx),
                                )),
                            )
                    })
                }
            })
            .into_any_element()
    }
}
