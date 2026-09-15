//! Comparison of captured schemas. Endpoint connection and credential resolution belong to
//! the parent; only the writable target session is retained here.
use super::controls::{Tone, badge, card, icon_button, notice, page_header, section_label};
use super::{
    Services,
    schema_review::{SchemaReview, SchemaReviewEvent},
};
use crate::{
    database::Session,
    schema::{self, Difference, PlanOptions, Snapshot},
};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable,
        button::{Button, ButtonVariants},
        checkbox::Checkbox,
        h_flex,
        scroll::ScrollableElement,
        switch::Switch,
        v_flex,
    },
    prelude::FluentBuilder,
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
    source_label: Option<String>,
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
            source_label: None,
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
fn engine_name(engine: &schema::Engine) -> &'static str {
    match engine {
        schema::Engine::Postgres => "PostgreSQL",
        schema::Engine::Sqlite => "SQLite",
    }
}
fn constraint_label(constraint: &schema::Constraint) -> String {
    match constraint {
        schema::Constraint::Primary { columns } => format!("Primary key ({})", columns.join(", ")),
        schema::Constraint::Unique { columns } => format!("Unique ({})", columns.join(", ")),
        schema::Constraint::Check { expression } => format!("Check {expression}"),
        schema::Constraint::ForeignKey {
            columns,
            target,
            referenced_columns,
            on_delete,
            on_update,
            deferral,
        } => format!(
            "Foreign key ({}) → {} ({}) · on delete {} · on update {}{}",
            columns.join(", "),
            target,
            referenced_columns.join(", "),
            on_delete.to_lowercase(),
            on_update.to_lowercase(),
            deferral
                .as_ref()
                .map(|s| format!(" · {s}"))
                .unwrap_or_default()
        ),
    }
}
/// One side of a table difference: its columns, constraints, indexes and coverage gaps.
fn table_side(
    label: &'static str,
    table: Option<&schema::Table>,
    cx: &App,
) -> impl IntoElement + use<> {
    let muted = cx.theme().muted_foreground;
    let mut side = v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .p_3()
        .child(div().pb_1().child(section_label(label, cx)));
    let Some(table) = table else {
        return side.child(div().text_sm().text_color(muted).child("Table absent"));
    };
    for column in &table.columns {
        side = side.child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(div().min_w_0().text_ellipsis().child(column.name.clone()))
                .child(
                    div()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_xs()
                        .text_color(muted)
                        .child(column.data_type.to_lowercase()),
                )
                .when(!column.nullable, |row| {
                    row.child(div().text_xs().text_color(muted).child("not null"))
                }),
        );
    }
    let details = table
        .constraints
        .iter()
        .map(constraint_label)
        .chain(table.indexes.iter().map(|index| {
            format!(
                "{} {} ({})",
                if index.unique {
                    "Unique index"
                } else {
                    "Index"
                },
                index.name,
                index.columns.join(", ")
            )
        }))
        .collect::<Vec<_>>();
    if !details.is_empty() {
        side = side.child(
            v_flex()
                .mt_2()
                .gap_0p5()
                .text_xs()
                .text_color(muted)
                .children(details),
        );
    }
    side.children(table.unsupported.iter().map(|warning| {
        h_flex()
            .mt_1()
            .gap_1p5()
            .text_xs()
            .text_color(cx.theme().warning)
            .child(Icon::new(IconName::TriangleAlert).xsmall())
            .child(div().min_w_0().child(warning.clone()))
    }))
}
fn endpoint(
    id: &'static str,
    role: &'static str,
    title: String,
    detail: String,
    on_change: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    card(cx)
        .flex_1()
        .min_w_0()
        .px_4()
        .py_3()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(section_label(role, cx))
                .child(
                    Button::new(id)
                        .ghost()
                        .xsmall()
                        .label("Change")
                        .on_click(on_change),
                ),
        )
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_ellipsis()
                .child(title),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .text_ellipsis()
                .child(detail),
        )
}
impl SchemaComparison {
    /// Names the source endpoint for display; the snapshot alone only knows its engine.
    pub fn set_source_label(&mut self, label: String) {
        self.source_label = Some(label);
    }
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
        let differences = schema::compare(&self.target, &self.source);
        let selected = differences
            .iter()
            .filter(|d| match d {
                Difference::Added(n) | Difference::Removed(n) | Difference::Changed(n) => {
                    self.selected.contains(n)
                }
                Difference::Unknown(_) => false,
            })
            .count();
        let subtitle = if self.capturing {
            "Capturing fresh source and target snapshots…".to_owned()
        } else {
            format!(
                "{} {} · {} selected",
                differences.len(),
                if differences.len() == 1 {
                    "difference"
                } else {
                    "differences"
                },
                selected
            )
        };
        let actions = [
            Button::new("back")
                .ghost()
                .small()
                .icon(IconName::ArrowLeft)
                .label("Endpoints")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SchemaComparisonEvent::Back)))
                .into_any_element(),
            icon_button(
                "recapture",
                IconName::RefreshCw,
                "Capture both schemas again",
            )
            .small()
            .loading(self.capturing)
            .on_click(cx.listener(|this, _, _, cx| this.request_capture(cx)))
            .into_any_element(),
            Button::new("review-selection")
                .primary()
                .small()
                .label(if selected == 0 {
                    "Review SQL".to_owned()
                } else {
                    format!(
                        "Review {selected} {}",
                        if selected == 1 { "change" } else { "changes" }
                    )
                })
                .disabled(unavailable || self.selected.is_empty())
                .on_click(cx.listener(|this, _, _, cx| this.open_review(cx)))
                .into_any_element(),
        ];
        let source_title = self
            .source_label
            .clone()
            .unwrap_or_else(|| format!("{} schema", engine_name(&self.source.engine)));
        let endpoints = h_flex()
            .items_stretch()
            .gap_3()
            .child(endpoint(
                "change-source",
                "Source · read only",
                source_title,
                format!("Schema {}", self.source.scope.namespace),
                cx.listener(|_, _, _, cx| cx.emit(SchemaComparisonEvent::ChangeSource)),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(IconName::ArrowRight).small()),
            )
            .child(endpoint(
                "change-target",
                "Target · changes applied here",
                self.target_session.profile.context(),
                format!("Schema {}", self.target.scope.namespace),
                cx.listener(|_, _, _, cx| cx.emit(SchemaComparisonEvent::ChangeTarget)),
                cx,
            ));
        let mut notices = v_flex().gap_2();
        let has_notices = blocked.is_some()
            || self.error.is_some()
            || self.target_session.read_only()
            || (self.source.engine == self.target.engine && self.source.scope != self.target.scope);
        if let Some(message) = blocked {
            notices = notices.child(notice("comparison-blocked", Tone::Danger, message, cx));
        }
        if self.source.engine == self.target.engine && self.source.scope != self.target.scope {
            notices = notices.child(
                h_flex().child(
                    Button::new("map-source-namespace")
                        .outline()
                        .small()
                        .icon(IconName::ArrowRightLeft)
                        .label(format!(
                            "Map source {} to target {}",
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
                ),
            );
        }
        if let Some(message) = &self.error {
            notices = notices.child(notice(
                "comparison-error",
                Tone::Danger,
                message.clone(),
                cx,
            ));
        }
        if self.target_session.read_only() {
            notices = notices.child(notice(
                "comparison-read-only",
                Tone::Warning,
                "The target connection is read-only, so changes cannot be applied.",
                cx,
            ));
        }
        let options = card(cx)
            .px_4()
            .py_3()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(div().text_sm().child("Include destructive changes"))
                    .child(
                        Switch::new("allow-destructive")
                            .accessibility_label("Include destructive changes")
                            .checked(self.allow_destructive)
                            .disabled(unavailable)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.allow_destructive = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Removals start unselected. Unsupported objects show as Unknown, and the planner may block changes that depend on them."),
            );
        let mut list = v_flex().gap_3();
        if differences.is_empty() && !self.capturing {
            list = list.child(
                card(cx)
                    .py_10()
                    .items_center()
                    .gap_1()
                    .child(Icon::new(IconName::CircleCheck).text_color(cx.theme().success))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Schemas match"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("There is nothing to apply to the target."),
                    ),
            );
        }
        for (index, difference) in differences.into_iter().enumerate() {
            let (kind, tone, name, unknown) = match difference {
                Difference::Added(n) => ("Added", Tone::Success, n, false),
                Difference::Removed(n) => ("Removed", Tone::Danger, n, false),
                Difference::Changed(n) => ("Changed", Tone::Warning, n, false),
                Difference::Unknown(n) => ("Unknown", Tone::Neutral, n, true),
            };
            let checked = self.selected.contains(&name);
            let toggle = name.clone();
            let before = self.target.tables.iter().find(|table| table.name == name);
            let after = self.source.tables.iter().find(|table| table.name == name);
            list = list.child(
                card(cx)
                    .overflow_hidden()
                    .when(checked, |card| {
                        card.border_color(cx.theme().ring.opacity(0.6))
                    })
                    .child(
                        h_flex()
                            .h_11()
                            .px_3()
                            .gap_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                Checkbox::new(("select-table", index))
                                    .accessibility_label(format!("Include {kind} table {name}"))
                                    .checked(checked)
                                    .disabled(unavailable || unknown)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !this.selected.remove(&toggle) {
                                            this.selected.insert(toggle.clone());
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_ellipsis()
                                    .child(name.clone()),
                            )
                            .when(kind == "Removed", |row| {
                                row.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Destructive"),
                                )
                            })
                            .child(badge(kind, tone.color(cx))),
                    )
                    .child(
                        h_flex()
                            .items_start()
                            .child(table_side("Target · before", before, cx))
                            .child(div().w_px().self_stretch().bg(cx.theme().border))
                            .child(table_side("Source · after", after, cx)),
                    ),
            );
        }
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(page_header("Compare schemas", subtitle, actions, cx))
            .child(
                div()
                    .id("schema-comparison")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(px(1080.))
                            .mx_auto()
                            .p_6()
                            .gap_5()
                            .child(endpoints)
                            .when(has_notices, |view| view.child(notices))
                            .child(options)
                            .child(
                                v_flex()
                                    .gap_3()
                                    .child(
                                        h_flex().gap_2().child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child("Differences"),
                                        ),
                                    )
                                    .child(list),
                            ),
                    ),
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
