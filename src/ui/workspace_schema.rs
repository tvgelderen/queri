use super::*;
use crate::schema::{PlanOptions, Scope, plan_with_options};
use crate::ui::schema_designer::{SchemaDesigner, SchemaDesignerEvent};
use crate::ui::schema_review::{SchemaReview, SchemaReviewEvent};

impl Workspace {
    pub(super) fn save_close_schema_draft(&mut self, cx: &mut Context<Self>) {
        let (Some(designer), Some(key)) = (&self.schema_designer, &self.schema_draft_key) else {
            return;
        };
        let result = serde_json::to_string(&designer.read(cx).intent(cx))
            .map_err(anyhow::Error::from)
            .and_then(|value| self.services.store.save_schema_draft(key, Some(&value)));
        match result {
            Ok(()) => {
                self.schema_designer = None;
                self.schema_review = None;
                self.schema_draft_key = None;
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
        if self.schema_designer.is_some() || self.schema_comparison.is_some() {
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
                            let database = profile.database();
                            if this.selected == Some(profile.id) {
                                if let Some(branch) = this.branches.get_mut(&database) {
                                    branch.schemas.insert(namespace.clone(), None);
                                }
                                this.load_node(
                                    &format!("schema:{}", serde_json::json!([database, namespace])),
                                    window,
                                    cx,
                                );
                            }
                            for document in &this.documents {
                                let doc = document.read(cx);
                                if doc.profile.id == profile.id
                                    && doc.profile.database() == database
                                {
                                    if let Some(session) = &doc.session {
                                        session.invalidate_completions();
                                    }
                                    if !doc.has_edits(cx)
                                        && !doc.busy()
                                        && doc.table_ref().is_some()
                                    {
                                        document.update(cx, |doc, cx| doc.refresh(window, cx));
                                    }
                                }
                            }
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
        if self.schema_review.as_ref().is_some_and(|review| {
            matches!(
                review.read(cx).state,
                super::super::schema_review::ReviewState::Applying
            )
        }) {
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
                        if let Some(key) = this.schema_draft_key.take()
                            && let Err(error) = this.services.store.save_schema_draft(&key, None)
                        {
                            this.error = format!("Could not discard saved schema draft: {error}");
                            this.schema_draft_key = Some(key);
                            return;
                        }
                        this.schema_designer = None;
                        this.schema_review = None;
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
        if self.schema_designer.is_some() {
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
                        let existing = table
                            .as_ref()
                            .and_then(|name| snapshot.tables.iter().find(|t| &t.name == name))
                            .cloned();
                        if table.is_some() && existing.is_none() {
                            this.error =
                                "The selected table no longer exists. Refresh the explorer.".into();
                            cx.notify();
                            return;
                        }
                        let saved =
                            this.services
                                .store
                                .schema_draft(&draft_key)
                                .and_then(|value| {
                                    value
                                        .map(|value| {
                                            serde_json::from_str(&value).map_err(Into::into)
                                        })
                                        .transpose()
                                });
                        let designer = match saved {
                            Ok(Some(intent)) => {
                                let mut restore_error = None;
                                let restored = cx.new(|cx| {
                                    match SchemaDesigner::restore(intent, window, cx) {
                                        Ok(view) => view,
                                        Err(error) => {
                                            restore_error = Some(error);
                                            SchemaDesigner::new(
                                                snapshot.clone(),
                                                existing.clone(),
                                                window,
                                                cx,
                                            )
                                        }
                                    }
                                });
                                if let Some(error) = restore_error {
                                    this.error = format!("Could not restore schema draft: {error}");
                                    cx.notify();
                                    return;
                                }
                                this.status = "Saved schema draft restored. Review checks the original baseline before applying.".into();
                                restored
                            }
                            Ok(None) => {
                                cx.new(|cx| SchemaDesigner::new(snapshot, existing, window, cx))
                            }
                            Err(error) => {
                                this.error = format!("Could not read schema draft: {error}");
                                return;
                            }
                        };
                        this.schema_draft_key = Some(draft_key.clone());
                        this.subscriptions.push(cx.subscribe_in(&designer, window, move |this, designer, event, window, cx| {
                        if matches!(event, SchemaDesignerEvent::Changed) {
                            let result = serde_json::to_string(&designer.read(cx).intent(cx)).map_err(anyhow::Error::from)
                                .and_then(|value| this.services.store.save_schema_draft(&draft_key, Some(&value)));
                            if let Err(error) = result { this.error = format!("Could not save schema draft: {error}"); }
                        }
                        if matches!(event, SchemaDesignerEvent::Cancel) {
                            this.close_schema_draft(window, cx);
                        }
                        if let SchemaDesignerEvent::Review(intent) = event {
                            let mut options = PlanOptions { allow_destructive: intent.allow_destructive, ..Default::default() };
                            if let Some(original) = &intent.original_table {
                                if let Some(index) = intent.baseline.tables.iter().position(|table| &table.name == original)
                                    && let Some(desired) = intent.desired.tables.get(index)
                                    && desired.name != *original
                                {
                                    options.table_renames.insert(original.clone(), desired.name.clone());
                                }
                                options.column_renames.insert(original.clone(), intent.column_renames.iter().cloned().collect());
                            }
                            match plan_with_options(&intent.baseline, &intent.desired, &options) {
                                Ok(plan) => {
                                    let review = cx.new(|_| SchemaReview::new(plan, this.services.clone(), session.clone()));
                                    let profile = session.profile.clone();
                                    let namespace = intent.baseline.scope.namespace.clone();
                                    this.subscriptions.push(cx.subscribe_in(&review, window, move |this, _, event, window, cx| {
                                        match event {
                                            SchemaReviewEvent::Back => this.schema_review = None,
                                            SchemaReviewEvent::Applied => {
                                                if let Some(key) = this.schema_draft_key.take()
                                                    && let Err(error) = this.services.store.save_schema_draft(&key, None) { this.error = format!("Schema applied, but saved draft cleanup failed: {error}"); }
                                                this.status = "Schema changes applied.".into();
                                                this.schema_review = None;
                                                this.schema_designer = None;
                                                let database = profile.database();
                                                if let Some(branch) = this.branches.get_mut(&database) {
                                                    branch.schemas.insert(namespace.clone(), None);
                                                }
                                                this.load_node(&format!("schema:{}", serde_json::json!([database, namespace])), window, cx);
                                                for document in &this.documents {
                                                    let doc = document.read(cx);
                                                    if doc.profile.id == profile.id && doc.profile.database() == profile.database() {
                                                        if let Some(session) = &doc.session { session.invalidate_completions(); }
                                                        if !doc.has_edits(cx) && !doc.busy() && doc.table_ref().is_some() {
                                                            document.update(cx, |doc, cx| doc.refresh(window, cx));
                                                        }
                                                    }
                                                }
                                            }
                                            SchemaReviewEvent::OpenSql(sql) => {
                                                this.add_document(profile.clone(), None, sql.clone(), true, window, cx);
                                                this.schema_review = None;
                                                this.status = "Schema SQL opened as an editable query; nothing executed.".into();
                                            }
                                        }
                                        cx.notify();
                                    }));
                                    this.schema_review = Some(review);
                                }
                                Err(error) => this.error = error.to_string(),
                            }
                        }
                        cx.notify();
                    }));
                        this.schema_designer = Some(designer);
                        this.active_content = ActiveContent::Designer;
                    }
                    Err(error) => this.error = format!("Could not load schema: {error:#}"),
                }
                cx.notify();
            },
        );
    }
}
