use super::controls::{Tone, card, icon_button, notice, page_header};
use super::{Services, task};
use crate::{database::Session, schema::Plan};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        scroll::ScrollableElement,
        v_flex,
    },
    prelude::FluentBuilder,
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
/// Light syntax colouring for formatted SQL: keywords in the accent, quoted text quieter.
fn highlight_sql(line: &str, cx: &App) -> StyledText {
    const KEYWORDS: &[&str] = &[
        "ADD",
        "ALTER",
        "AS",
        "BY",
        "CASCADE",
        "CHECK",
        "COLUMN",
        "CONSTRAINT",
        "CREATE",
        "DEFAULT",
        "DELETE",
        "DROP",
        "EXISTS",
        "FOREIGN",
        "FROM",
        "IF",
        "INDEX",
        "INSERT",
        "INTO",
        "KEY",
        "NO",
        "NOT",
        "NULL",
        "ON",
        "PRIMARY",
        "REFERENCES",
        "RENAME",
        "SELECT",
        "SET",
        "TABLE",
        "TO",
        "TYPE",
        "UNIQUE",
        "UPDATE",
        "USING",
        "VALUES",
        "ACTION",
        "RESTRICT",
        "BEGIN",
        "COMMIT",
        "PRAGMA",
        "WHERE",
        "AND",
        "OR",
    ];
    let keyword = HighlightStyle {
        color: Some(super::theme::accent_text(cx)),
        ..Default::default()
    };
    let literal = HighlightStyle {
        color: Some(cx.theme().success),
        ..Default::default()
    };
    let mut highlights = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\'' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                i += 1;
            }
            i = (i + 1).min(bytes.len());
            highlights.push((start..i, literal));
        } else if c == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            i = (i + 1).min(bytes.len());
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if KEYWORDS.contains(&line[start..i].to_ascii_uppercase().as_str()) {
                highlights.push((start..i, keyword));
            }
        } else {
            i += 1;
        }
    }
    StyledText::new(line.to_owned()).with_highlights(highlights)
}
impl SchemaReview {
    fn can_apply(&self) -> bool {
        self.state == ReviewState::Review
            && self.plan.blocked.is_empty()
            && !self.session.read_only()
            && !self.plan.sql().is_empty()
    }
    fn request_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state != ReviewState::Applying || self.cancel_requested {
            return;
        }
        self.cancel_requested = true;
        let session = self.session.clone();
        task(
            &self.services,
            window,
            cx,
            async move { session.cancel().await },
            |this, result, _, cx| {
                if let Err(error) = result {
                    this.cancel_error = Some(format!(
                        "Cancellation request failed: {error}. Waiting for execution to settle."
                    ));
                }
                cx.notify();
            },
        );
        cx.notify();
    }
}
impl Render for SchemaReview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let statements = self.plan.sql();
        let sql = statements.join(";\n");
        let copy = sql.clone();
        let open = sql.clone();
        let applying = self.state == ReviewState::Applying;
        let destructive = !self.plan.risks.is_empty();
        let subtitle = format!(
            "{} / {} · {} {}",
            self.session.profile.context(),
            self.plan.expected.scope.namespace,
            statements.len(),
            if statements.len() == 1 {
                "statement"
            } else {
                "statements"
            }
        );
        let mut actions = vec![
            Button::new("back-schema")
                .ghost()
                .small()
                .icon(IconName::ArrowLeft)
                .label("Back")
                .disabled(matches!(
                    self.state,
                    ReviewState::Applying | ReviewState::Succeeded
                ))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SchemaReviewEvent::Back)))
                .into_any_element(),
            icon_button("copy-sql", IconName::Copy, "Copy SQL")
                .small()
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                })
                .into_any_element(),
            Button::new("open-sql")
                .ghost()
                .small()
                .icon(IconName::SquareCode)
                .label("Open in editor")
                .disabled(applying)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.state == ReviewState::Applying {
                        return;
                    }
                    cx.emit(SchemaReviewEvent::OpenSql(open.clone()))
                }))
                .into_any_element(),
        ];
        if applying {
            actions.push(
                Button::new("cancel-schema")
                    .outline()
                    .small()
                    .label(if self.cancel_requested {
                        "Cancelling…"
                    } else {
                        "Cancel"
                    })
                    .disabled(self.cancel_requested)
                    .on_click(cx.listener(|this, _, window, cx| this.request_cancel(window, cx)))
                    .into_any_element(),
            );
        }
        let apply = Button::new("apply-schema")
            .small()
            .label(match self.state {
                ReviewState::Applying => "Applying…",
                ReviewState::Succeeded => "Applied",
                _ => "Apply changes",
            })
            .disabled(!self.can_apply())
            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx)));
        actions.push(
            if destructive {
                apply.danger()
            } else {
                apply.primary()
            }
            .into_any_element(),
        );

        let has_notices = self.state != ReviewState::Review
            || self.cancel_error.is_some()
            || self.session.read_only()
            || !self.plan.blocked.is_empty()
            || !self.plan.risks.is_empty();
        let mut notices = v_flex().gap_2();
        match &self.state {
            ReviewState::Review => {}
            ReviewState::Applying => {
                notices = notices.child(notice(
                    "schema-state",
                    Tone::Neutral,
                    "Applying changes in a single transaction. The result is verified before it is committed.",
                    cx,
                ))
            }
            ReviewState::Succeeded => {
                notices = notices.child(notice(
                    "schema-state",
                    Tone::Success,
                    "Changes applied and verified.",
                    cx,
                ))
            }
            ReviewState::Failed(message) => {
                notices = notices.child(notice(
                    "schema-state",
                    Tone::Danger,
                    format!("Could not apply changes: {message}"),
                    cx,
                ))
            }
        }
        if let Some(error) = &self.cancel_error {
            notices = notices.child(notice(
                "schema-cancel-error",
                Tone::Danger,
                error.clone(),
                cx,
            ));
        }
        if self.session.read_only() {
            notices = notices.child(notice(
                "schema-read-only",
                Tone::Warning,
                "This connection is read-only. Copy the SQL or open it in the editor instead.",
                cx,
            ));
        }
        for (index, message) in self.plan.blocked.iter().enumerate() {
            notices = notices.child(notice(
                ("schema-blocked", index),
                Tone::Danger,
                format!("Blocked: {message}"),
                cx,
            ));
        }
        for (index, message) in self.plan.risks.iter().enumerate() {
            notices = notices.child(notice(
                ("schema-risk", index),
                Tone::Warning,
                message.clone(),
                cx,
            ));
        }

        let formatted = sqlformat::format(
            &sql,
            &sqlformat::QueryParams::None,
            &sqlformat::FormatOptions::default(),
        );
        let muted = cx.theme().muted_foreground;
        let code = if formatted.trim().is_empty() {
            div()
                .p_6()
                .text_sm()
                .text_center()
                .text_color(muted)
                .child("No changes to apply. The draft matches the current schema.")
                .into_any_element()
        } else {
            v_flex()
                .py_3()
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(cx.theme().mono_font_size)
                .line_height(relative(1.65))
                .children(formatted.lines().enumerate().map(|(index, line)| {
                    h_flex()
                        .items_start()
                        .child(
                            div()
                                .w_10()
                                .flex_shrink_0()
                                .pr_3()
                                .text_right()
                                .text_color(muted.opacity(0.6))
                                .child((index + 1).to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .pr_4()
                                .child(highlight_sql(line, cx)),
                        )
                }))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(page_header("Review changes", subtitle, actions, cx))
            .child(
                div()
                    .id("schema-review")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(px(960.))
                            .mx_auto()
                            .p_6()
                            .gap_4()
                            .when(has_notices, |view| view.child(notices))
                            .child(
                                card(cx)
                                    .overflow_hidden()
                                    .child(
                                        h_flex()
                                            .h_10()
                                            .px_4()
                                            .gap_2()
                                            .justify_between()
                                            .bg(cx.theme().muted)
                                            .border_b_1()
                                            .border_color(cx.theme().border)
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child("Generated SQL"),
                                            )
                                            .child(div().text_xs().text_color(muted).child(
                                                "Applied atomically · rolled back on failure",
                                            )),
                                    )
                                    .child(code),
                            ),
                    ),
            )
    }
}
