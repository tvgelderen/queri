//! Comparison of captured schemas. Endpoint connection and credential resolution belong to
//! the parent; only the writable target session is retained here.
use super::{
    Services,
    schema_review::{SchemaReview, SchemaReviewEvent},
};
use crate::{
    database::Session,
    schema::{self, Difference, PlanOptions, Snapshot},
};
use gpui_kit::{
    component::{Disableable, button::Button, scroll::ScrollableElement, v_flex},
    *,
};
use std::{collections::BTreeSet, sync::Arc};

pub enum SchemaComparisonEvent {
    Back,
    OpenSQL(String),
    Applied,
    ChangeSource,
    ChangeTarget,
    /// Capture both endpoints again and call `complete_capture` with this generation.
    Capture {
        generation: u64,
    },
}
impl EventEmitter<SchemaComparisonEvent> for SchemaComparison {}

pub struct SchemaComparison {
    services: Services,
    source: Snapshot,
    target: Snapshot,
    target_session: Arc<Session>,
    selected: BTreeSet<String>,
    review: Option<Entity<SchemaReview>>,
    generation: u64,
    capturing: bool,
    allow_destructive: bool,
    error: Option<String>,
}
fn default_selection(differences: &[Difference]) -> BTreeSet<String> {
    differences
        .iter()
        .filter_map(|d| match d {
            Difference::Added(n) | Difference::Changed(n) => Some(n.clone()),
            Difference::Removed(_) | Difference::Unknown(_) => None,
        })
        .collect()
}
fn compatibility(source: &Snapshot, target: &Snapshot) -> Option<String> {
    if source.engine != target.engine {
        Some("Blocked: Source and Target must use the same database engine.".into())
    } else if source.scope != target.scope {
        Some("Blocked: different namespaces require explicit mapping. Use Map Source namespace to Target to review a mapped capture.".into())
    } else {
        None
    }
}
impl SchemaComparison {
    pub fn busy(&self, cx: &App) -> bool {
        self.review.as_ref().is_some_and(|review| {
            review.read(cx).state == super::schema_review::ReviewState::Applying
        })
    }
    /// Snapshots must come from dedicated endpoint captures. The target snapshot must
    /// describe `target_session`; the review revalidates its baseline before applying.
    pub fn new(
        services: Services,
        source: Snapshot,
        target: Snapshot,
        target_session: Arc<Session>,
    ) -> Self {
        let selected = default_selection(&schema::compare(&target, &source));
        Self {
            services,
            source,
            target,
            target_session,
            selected,
            review: None,
            generation: 0,
            capturing: false,
            allow_destructive: false,
            error: None,
        }
    }
    pub fn request_capture(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.capturing = true;
        self.allow_destructive = false;
        self.review = None;
        self.error = None;
        cx.emit(SchemaComparisonEvent::Capture {
            generation: self.generation,
        });
        cx.notify();
    }
    /// Returns false for a stale completion. Errors keep the previous capture visible,
    /// but prevent review until both endpoints have been successfully recaptured.
    pub fn complete_capture(
        &mut self,
        generation: u64,
        result: anyhow::Result<(Snapshot, Snapshot, Arc<Session>)>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !accept_capture(self.generation, generation, self.capturing) {
            return false;
        }
        self.capturing = false;
        match result {
            Ok((source, target, session)) => {
                self.source = source;
                self.target = target;
                self.target_session = session;
                self.selected = default_selection(&schema::compare(&self.target, &self.source));
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Capture failed: {error:#}")),
        }
        cx.notify();
        true
    }
    fn open_review(&mut self, cx: &mut Context<Self>) {
        if self.capturing
            || self.error.is_some()
            || self.target_session.read_only()
            || compatibility(&self.source, &self.target).is_some()
        {
            return;
        }
        let options = PlanOptions {
            selected_tables: Some(self.selected.iter().cloned().collect()),
            allow_destructive: self.allow_destructive,
            ..Default::default()
        };
        match schema::plan_with_options(&self.target, &self.source, &options) {
            Ok(plan) => {
                let review = cx.new(|_| {
                    SchemaReview::new(plan, self.services.clone(), self.target_session.clone())
                });
                cx.subscribe(&review, |this, _, event, cx| match event {
                    SchemaReviewEvent::Back => {
                        this.review = None;
                        cx.notify();
                    }
                    SchemaReviewEvent::OpenSql(sql) => {
                        cx.emit(SchemaComparisonEvent::OpenSQL(sql.clone()))
                    }
                    SchemaReviewEvent::Applied => {
                        cx.emit(SchemaComparisonEvent::Applied);
                        this.request_capture(cx);
                    }
                })
                .detach();
                self.review = Some(review);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }
}
fn accept_capture(current: u64, received: u64, pending: bool) -> bool {
    pending && current == received
}
impl Render for SchemaComparison {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(review) = &self.review {
            return div().size_full().child(review.clone()).into_any_element();
        }
        let blocked = compatibility(&self.source, &self.target);
        let unavailable = self.capturing
            || self.error.is_some()
            || blocked.is_some()
            || self.target_session.read_only();
        let mut view = v_flex().id("schema-comparison").size_full().overflow_y_scrollbar().p_4().gap_3()
            .child("Compare schemas")
            .child(format!("Desired Source: {:?} / {} (capture only)", self.source.engine, self.source.scope.namespace))
            .child(Button::new("change-source").label("Change Source connection / database / schema").on_click(cx.listener(|_, _, _, cx| { cx.emit(SchemaComparisonEvent::ChangeSource); })))
            .child(format!("Writable Target: {} / {}", self.target_session.profile.context(), self.target.scope.namespace))
            .child(Button::new("change-target").label("Change Target connection / database / schema").on_click(cx.listener(|_, _, _, cx| { cx.emit(SchemaComparisonEvent::ChangeTarget); })))
            .child(if self.capturing { "Capture status: waiting for fresh Source and Target snapshots" } else { "Capture status: captured" })
            .child("Coverage: unsupported objects are Unknown. Alterations and removals may be blocked by the planner until dependent coverage is complete.")
            .children(blocked.map(|message| div().child(message)))
            .children(self.error.clone().map(|message| div().child(message)));
        if self.source.engine == self.target.engine && self.source.scope != self.target.scope {
            view = view.child(
                Button::new("map-source-namespace")
                    .label(format!(
                        "Map Source namespace {} to Target {}",
                        self.source.scope.namespace, self.target.scope.namespace
                    ))
                    .disabled(self.capturing)
                    .on_click(cx.listener(|this, _, _, cx| {
                        match schema::map_scope(&this.source, this.target.scope.clone()) {
                            Ok(mapped) => {
                                this.source = mapped;
                                this.error = None;
                            }
                            Err(error) => this.error = Some(error.to_string()),
                        }
                        cx.notify();
                    })),
            );
        }
        view = view.child(
            Button::new("allow-destructive")
                .label(if self.allow_destructive {
                    "Destructive changes: allowed"
                } else {
                    "Destructive changes: blocked (click to allow)"
                })
                .disabled(unavailable)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.allow_destructive = !this.allow_destructive;
                    cx.notify();
                })),
        );
        if self.target_session.read_only() {
            view = view.child("Blocked: Target connection is read-only.");
        }
        for (index, difference) in schema::compare(&self.target, &self.source)
            .into_iter()
            .enumerate()
        {
            let (kind, name, unknown) = match difference {
                Difference::Added(n) => ("Added", n, false),
                Difference::Removed(n) => ("Removed (destructive)", n, false),
                Difference::Changed(n) => ("Changed", n, false),
                Difference::Unknown(n) => ("Unknown", n, true),
            };
            let selected = self.selected.contains(&name);
            let toggle = name.clone();
            view = view.child(
                Button::new(("select-table", index))
                    .label(format!(
                        "{} {kind}: {name}",
                        if selected { "Selected" } else { "Unselected" }
                    ))
                    .disabled(unavailable || unknown)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.selected.remove(&toggle) {
                            this.selected.insert(toggle.clone());
                        }
                        cx.notify();
                    })),
            );
            for (label, snapshot) in [
                ("Before (Target)", &self.target),
                ("After (Source)", &self.source),
            ] {
                if let Some(table) = snapshot.tables.iter().find(|table| table.name == name) {
                    view = view.child(format!(
                        "{label}: {} columns, {} constraints, {} indexes",
                        table.columns.len(),
                        table.constraints.len(),
                        table.indexes.len()
                    ));
                    for column in &table.columns {
                        view = view.child(format!(
                            "{} · {}{}",
                            column.name,
                            column.data_type,
                            if column.nullable { "" } else { " · NOT NULL" }
                        ));
                    }
                    for constraint in &table.constraints {
                        let label = match constraint {
                            schema::Constraint::Primary { columns } => {
                                format!("Primary key: {}", columns.join(", "))
                            }
                            schema::Constraint::Unique { columns } => {
                                format!("Unique: {}", columns.join(", "))
                            }
                            schema::Constraint::Check { expression } => {
                                format!("Check: {expression}")
                            }
                            schema::Constraint::ForeignKey {
                                columns,
                                target,
                                referenced_columns,
                                on_delete,
                                on_update,
                                deferral,
                            } => format!(
                                "Foreign key: ({}) → {} ({}) · on delete {} · on update {}{}",
                                columns.join(", "),
                                target,
                                referenced_columns.join(", "),
                                on_delete,
                                on_update,
                                deferral
                                    .as_ref()
                                    .map(|s| format!(" · {s}"))
                                    .unwrap_or_default()
                            ),
                        };
                        view = view.child(label);
                    }
                    for index in &table.indexes {
                        view = view.child(format!("Index: {index:?}"));
                    }
                    for warning in &table.unsupported {
                        view = view.child(format!("Coverage warning: {warning}"));
                    }
                } else {
                    view = view.child(format!("{label}: table absent"));
                }
            }
        }
        view.child(
            Button::new("review-selection")
                .label("Select changes — review SQL")
                .disabled(unavailable || self.selected.is_empty())
                .on_click(cx.listener(|this, _, _, cx| this.open_review(cx))),
        )
        .child(
            Button::new("recapture")
                .label("Recapture and compare")
                .on_click(cx.listener(|this, _, _, cx| this.request_capture(cx))),
        )
        .child(
            Button::new("back")
                .label("Back")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SchemaComparisonEvent::Back))),
        )
        .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::{accept_capture, compatibility, default_selection};
    use crate::schema::{self, Difference, Snapshot};
    use std::collections::BTreeSet;
    #[test]
    fn destructive_and_unknown_are_not_selected() {
        assert_eq!(
            default_selection(&[
                Difference::Added("new".into()),
                Difference::Removed("old".into()),
                Difference::Unknown("opaque".into())
            ]),
            BTreeSet::from(["new".into()])
        );
    }
    #[test]
    fn stale_and_duplicate_captures_are_ignored() {
        assert!(!accept_capture(2, 1, true));
        assert!(!accept_capture(2, 2, false));
        assert!(accept_capture(2, 2, true));
    }
    #[test]
    fn mismatched_engines_and_namespaces_block() {
        let target = Snapshot {
            engine: schema::Engine::Sqlite,
            engine_version: String::new(),
            scope: schema::Scope {
                namespace: "main".into(),
            },
            tables: vec![],
        };
        let mut source = target.clone();
        source.engine = schema::Engine::Postgres;
        assert!(compatibility(&source, &target).is_some());
        source.engine = target.engine.clone();
        source.scope.namespace = "other".into();
        assert!(compatibility(&source, &target).is_some());
    }
}
