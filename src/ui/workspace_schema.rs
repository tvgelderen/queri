use super::*;
use crate::schema::{PlanOptions, Scope, plan_with_options};
use crate::ui::schema_designer::{SchemaDesigner, SchemaDesignerEvent, SchemaEditIntent};
use crate::ui::schema_review::{ReviewState, SchemaReview, SchemaReviewEvent};

/// The open table-designer draft. Switching tabs keeps it; only save, discard, or apply end it.
pub(super) struct SchemaDraft {
    /// Storage key identifying the connection, database, namespace, and table.
    key: String,
    pub(super) designer: Entity<SchemaDesigner>,
    pub(super) review: Option<Entity<SchemaReview>>,
}

impl Workspace {
    pub(super) fn save_close_schema_draft(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.schema_draft else {
            return;
        };
        match save_draft(&self.services, &draft.key, &draft.designer, cx) {
            Ok(()) => {
                self.schema_draft = None;
                self.active_content = ActiveContent::Document;
                self.status =
                    "Schema draft saved. Reopen the same table designer to continue.".into();
            }
            Err(error) => self.error = format!("Could not save schema draft: {error}"),
        }
        cx.notify();
    }

    pub(super) fn open_schema_comparison(
        &mut self,
        database: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.schema_draft.is_some() || self.schema_comparison.is_some() {
            self.error = "Close the current schema workspace first.".into();
            cx.notify();
            return;
        }
        use crate::ui::schema_compare_setup::{SchemaCompareSetup, SchemaCompareSetupEvent};
        let initial = self.selected_profile().map(|p| p.for_database(database));
        let setup = cx.new(|cx| {
            SchemaCompareSetup::new(
                self.services.clone(),
                self.profiles.clone(),
                self.passwords.clone(),
                initial,
                window,
                cx,
            )
        });
        self.subscriptions.push(cx.subscribe_in(
            &setup,
            window,
            |this, setup, event, window, cx| {
                match event {
                    SchemaCompareSetupEvent::Back => {
                        if setup.read(cx).can_close(cx) {
                            this.schema_comparison = None;
                        }
                    }
                    SchemaCompareSetupEvent::OpenSQL(sql) => {
                        if let Some(profile) = setup.read(cx).target_profile(cx) {
                            this.add_document(profile, None, sql.clone(), true, window, cx);
                            this.schema_comparison = None;
                        }
                    }
                    SchemaCompareSetupEvent::Applied => {
                        this.status = "Target schema changes applied; comparison refreshed.".into();
                        if let Some(profile) = setup.read(cx).target_profile(cx) {
                            let namespace = setup.read(cx).target_namespace(cx);
                            if this.selected == Some(profile.id) {
                                this.reload_schema_node(
                                    &profile.database(),
                                    &namespace,
                                    window,
                                    cx,
                                );
                            }
                            this.refresh_documents_after_schema_change(&profile, window, cx);
                        }
                    }
                }
                cx.notify();
            },
        ));
        self.schema_comparison = Some(setup);
        self.active_content = ActiveContent::Comparison;
        cx.notify();
    }

    pub(super) fn close_schema_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let applying = self
            .schema_draft
            .as_ref()
            .and_then(|draft| draft.review.as_ref())
            .is_some_and(|review| review.read(cx).state == ReviewState::Applying);
        if applying {
            return;
        }
        let view = cx.entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .title("Discard schema draft?")
                .child("Unsaved schema edits will be lost.")
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Discard draft")
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        if let Some(draft) = &this.schema_draft
                            && let Err(error) =
                                this.services.store.save_schema_draft(&draft.key, None)
                        {
                            this.error = format!("Could not discard saved schema draft: {error}");
                            return;
                        }
                        this.schema_draft = None;
                        this.schema_request = Uuid::new_v4();
                        cx.notify();
                    });
                    true
                })
        });
    }

    pub(super) fn open_schema_designer(
        &mut self,
        database: &str,
        namespace: &str,
        table: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Keep the current draft alive until the user explicitly leaves it.
        if self.schema_draft.is_some() {
            self.error = "A schema draft is already open.".into();
            cx.notify();
            return;
        }
        let Some(profile) = self.selected_profile().map(|p| p.for_database(database)) else {
            return;
        };
        let draft_key =
            serde_json::json!([profile.id, profile.database(), namespace, table]).to_string();
        let password = self.passwords.get(&profile.id).cloned().unwrap_or_default();
        let connector = self.services.connector.clone();
        let generation = self.generation;
        let request = Uuid::new_v4();
        self.schema_request = request;
        let namespace = namespace.to_owned();
        task(
            &self.services,
            window,
            cx,
            async move {
                let session = connector.connect(profile, password).await?;
                let snapshot = session.schema_snapshot(Scope { namespace }).await?;
                anyhow::Ok((session, snapshot))
            },
            move |this, result, window, cx| {
                if this.generation != generation || this.schema_request != request {
                    return;
                }
                match result {
                    Ok((session, snapshot)) => {
                        this.show_schema_designer(draft_key, session, snapshot, table, window, cx)
                    }
                    Err(error) => this.error = format!("Could not load schema: {error:#}"),
                }
                cx.notify();
            },
        );
    }

    /// Opens the designer for `table`, restoring a saved draft for the same key when present.
    fn show_schema_designer(
        &mut self,
        key: String,
        session: Arc<Session>,
        snapshot: crate::schema::Snapshot,
        table: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = table
            .as_ref()
            .and_then(|name| snapshot.tables.iter().find(|t| &t.name == name))
            .cloned();
        if table.is_some() && existing.is_none() {
            self.error = "The selected table no longer exists. Refresh the explorer.".into();
            return;
        }
        let saved = self.services.store.schema_draft(&key).and_then(|value| {
            value
                .map(|value| serde_json::from_str::<SchemaEditIntent>(&value).map_err(Into::into))
                .transpose()
        });
        let designer = match saved {
            Ok(Some(intent)) => {
                let mut restore_error = None;
                let restored = cx.new(|cx| match SchemaDesigner::restore(intent, window, cx) {
                    Ok(view) => view,
                    Err(error) => {
                        restore_error = Some(error);
                        SchemaDesigner::new(snapshot.clone(), existing.clone(), window, cx)
                    }
                });
                if let Some(error) = restore_error {
                    self.error = format!("Could not restore schema draft: {error}");
                    return;
                }
                self.status = "Saved schema draft restored. Review checks the original baseline before applying.".into();
                restored
            }
            Ok(None) => cx.new(|cx| SchemaDesigner::new(snapshot, existing, window, cx)),
            Err(error) => {
                self.error = format!("Could not read schema draft: {error}");
                return;
            }
        };
        let draft_key = key.clone();
        self.subscriptions.push(cx.subscribe_in(
            &designer,
            window,
            move |this, designer, event, window, cx| {
                match event {
                    SchemaDesignerEvent::Changed => {
                        if let Err(error) = save_draft(&this.services, &draft_key, designer, cx) {
                            this.error = format!("Could not save schema draft: {error}");
                        }
                    }
                    SchemaDesignerEvent::Cancel => this.close_schema_draft(window, cx),
                    SchemaDesignerEvent::Review(intent) => {
                        this.review_schema_draft(intent, session.clone(), window, cx)
                    }
                }
                cx.notify();
            },
        ));
        self.schema_draft = Some(SchemaDraft {
            key,
            designer,
            review: None,
        });
        self.active_content = ActiveContent::Designer;
    }

    fn review_schema_draft(
        &mut self,
        intent: &SchemaEditIntent,
        session: Arc<Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plan = match plan_with_options(&intent.baseline, &intent.desired, &plan_options(intent))
        {
            Ok(plan) => plan,
            Err(error) => {
                self.error = error.to_string();
                return;
            }
        };
        let profile = session.profile.clone();
        let namespace = intent.baseline.scope.namespace.clone();
        let review = cx.new(|_| SchemaReview::new(plan, self.services.clone(), session));
        self.subscriptions.push(cx.subscribe_in(
            &review,
            window,
            move |this, _, event, window, cx| {
                match event {
                    SchemaReviewEvent::Back => {
                        if let Some(draft) = &mut this.schema_draft {
                            draft.review = None;
                        }
                    }
                    SchemaReviewEvent::Applied => {
                        if let Some(draft) = this.schema_draft.take()
                            && let Err(error) =
                                this.services.store.save_schema_draft(&draft.key, None)
                        {
                            this.error =
                                format!("Schema applied, but saved draft cleanup failed: {error}");
                        }
                        this.status = "Schema changes applied.".into();
                        this.reload_schema_node(&profile.database(), &namespace, window, cx);
                        this.refresh_documents_after_schema_change(&profile, window, cx);
                    }
                    SchemaReviewEvent::OpenSql(sql) => {
                        this.add_document(profile.clone(), None, sql.clone(), true, window, cx);
                        if let Some(draft) = &mut this.schema_draft {
                            draft.review = None;
                        }
                        this.status =
                            "Schema SQL opened as an editable query; nothing executed.".into();
                    }
                }
                cx.notify();
            },
        ));
        if let Some(draft) = &mut self.schema_draft {
            draft.review = Some(review);
        }
    }

    /// Marks a namespace's tables as unloaded and reloads its explorer node.
    fn reload_schema_node(
        &mut self,
        database: &str,
        namespace: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(branch) = self.branches.get_mut(database) {
            branch.schemas.insert(namespace.to_owned(), None);
        }
        self.load_node(
            &format!("schema:{}", serde_json::json!([database, namespace])),
            window,
            cx,
        );
    }

    /// Drops cached completions and reloads idle table documents of the changed database.
    fn refresh_documents_after_schema_change(
        &self,
        profile: &ConnectionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let database = profile.database();
        for document in &self.documents {
            let doc = document.read(cx);
            if doc.profile.id != profile.id || doc.profile.database() != database {
                continue;
            }
            if let Some(session) = &doc.session {
                session.invalidate_completions();
            }
            if !doc.has_edits(cx) && !doc.busy() && doc.table_ref().is_some() {
                document.update(cx, |doc, cx| doc.refresh(window, cx));
            }
        }
    }
}

fn save_draft(
    services: &Services,
    key: &str,
    designer: &Entity<SchemaDesigner>,
    cx: &App,
) -> anyhow::Result<()> {
    let value = serde_json::to_string(&designer.read(cx).intent(cx))?;
    services.store.save_schema_draft(key, Some(&value))
}

fn plan_options(intent: &SchemaEditIntent) -> PlanOptions {
    let mut options = PlanOptions {
        allow_destructive: intent.allow_destructive,
        ..Default::default()
    };
    if let Some(original) = &intent.original_table {
        if let Some(index) = intent
            .baseline
            .tables
            .iter()
            .position(|table| &table.name == original)
            && let Some(desired) = intent.desired.tables.get(index)
            && desired.name != *original
        {
            options
                .table_renames
                .insert(original.clone(), desired.name.clone());
        }
        options.column_renames.insert(
            original.clone(),
            intent.column_renames.iter().cloned().collect(),
        );
    }
    options
}
