use super::{Services, task};
use crate::{database::Session, schema::Plan};
use gpui_kit::{
    component::{Disableable, button::Button, scroll::ScrollableElement, v_flex},
    *,
};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum ReviewState {
    Review,
    Applying,
    Succeeded,
    Failed(String),
}
pub enum SchemaReviewEvent {
    OpenSql(String),
    Applied,
    Back,
}
impl EventEmitter<SchemaReviewEvent> for SchemaReview {}
pub struct SchemaReview {
    pub plan: Plan,
    pub state: ReviewState,
    cancel_requested: bool,
    cancel_error: Option<String>,
    services: Services,
    session: Arc<Session>,
}
impl SchemaReview {
    pub fn new(plan: Plan, services: Services, session: Arc<Session>) -> Self {
        Self {
            plan,
            cancel_requested: false,
            cancel_error: None,
            services,
            session,
            state: ReviewState::Review,
        }
    }
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state != ReviewState::Review
            || !self.plan.blocked.is_empty()
            || self.session.read_only()
            || self.plan.sql().is_empty()
        {
            return;
        }
        self.state = ReviewState::Applying;
        let session = self.session.clone();
        let plan = self.plan.clone();
        task(
            &self.services,
            window,
            cx,
            async move { session.apply_schema_plan(plan).await },
            |this, result, _, cx| {
                this.state = match result {
                    Ok(()) => {
                        cx.emit(SchemaReviewEvent::Applied);
                        ReviewState::Succeeded
                    }
                    Err(e) => ReviewState::Failed(format!(
                        "{e:#}. Refresh schema before another attempt; the outcome may be unknown."
                    )),
                };
                cx.notify();
            },
        );
        cx.notify();
    }
}
impl Render for SchemaReview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sql = self.plan.sql().join(";\n");
        let copy = sql.clone();
        let open = sql.clone();
        v_flex()
            .id("schema-review")
            .overflow_y_scrollbar()
            .gap_3()
            .p_4()
            .child(format!(
                "Review schema changes — {} / {}",
                self.session.profile.context(),
                self.plan.expected.scope.namespace
            ))
            .children(
                self.plan
                    .blocked
                    .iter()
                    .map(|s| div().child(format!("Blocked: {s}"))),
            )
            .children(
                self.plan
                    .risks
                    .iter()
                    .map(|s| div().child(format!("Risk: {s}"))),
            )
            .child(div().font_family("monospace").child(sqlformat::format(
                &sql,
                &sqlformat::QueryParams::None,
                &sqlformat::FormatOptions::default(),
            )))
            .child(match &self.state {
                ReviewState::Review => "Ready for review".to_owned(),
                ReviewState::Applying => "Applying changes…".to_owned(),
                ReviewState::Succeeded => "Changes applied".to_owned(),
                ReviewState::Failed(message) => format!("Could not apply changes: {message}"),
            })
            .child(
                Button::new("copy-sql")
                    .label("Copy SQL")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                    }),
            )
            .child(
                Button::new("open-sql")
                    .label("Open SQL in editor")
                    .disabled(self.state == ReviewState::Applying)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.state == ReviewState::Applying { return; }
                        cx.emit(SchemaReviewEvent::OpenSql(open.clone()))
                    })),
            )
            .child(
                Button::new("apply-schema")
                    .label("Confirm and apply changes")
                    .disabled(
                        self.state != ReviewState::Review
                            || !self.plan.blocked.is_empty()
                            || self.session.read_only()
                            || self.plan.sql().is_empty(),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
            )
            .children(self.cancel_error.clone().map(|error| div().child(error)))
            .child(
                Button::new("cancel-schema")
                    .label(if self.cancel_requested { "Cancellation requested…" } else { "Cancel execution" })
                    .disabled(self.state != ReviewState::Applying || self.cancel_requested)
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.state != ReviewState::Applying || this.cancel_requested { return; }
                        this.cancel_requested = true;
                        let session = this.session.clone();
                        task(&this.services, window, cx, async move { session.cancel().await }, |this, result, _, cx| {
                            if let Err(error) = result {
                                this.cancel_error = Some(format!("Cancellation request failed: {error}. Waiting for execution to settle."));
                            }
                            cx.notify();
                        });
                        cx.notify();
                    })),
            )
            .child(
                Button::new("back-schema")
                    .label("Back to draft")
                    .disabled(matches!(
                        self.state,
                        ReviewState::Applying | ReviewState::Succeeded
                    ))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SchemaReviewEvent::Back))),
            )
    }
}
