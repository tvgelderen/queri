use crate::query_plan::{PlanNode, QueryPlan};
use gpui_kit::{
    component::{
        ActiveTheme, Sizable,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    *,
};
use std::collections::BTreeSet;

pub struct PlanView {
    plan: QueryPlan,
    source: String,
    show_raw: bool,
    expanded: BTreeSet<String>,
}
impl PlanView {
    pub fn new(plan: QueryPlan, source: String, _: &mut Window, _: &mut Context<Self>) -> Self {
        Self {
            plan,
            source,
            show_raw: false,
            expanded: BTreeSet::from(["plan-0".into()]),
        }
    }
    fn node(&self, node: &PlanNode, key: String, cx: &mut Context<Self>) -> AnyElement {
        let expanded = self.expanded.contains(&key);
        let button_key = key.clone();
        let mut view = v_flex().gap_1().child(
            Button::new(SharedString::from(key.clone()))
                .ghost()
                .small()
                .label(format!(
                    "{} {}",
                    if expanded { "▾" } else { "▸" },
                    node.label
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.expanded.remove(&button_key) {
                        this.expanded.insert(button_key.clone());
                    }
                    cx.notify();
                })),
        );
        if !node.summary.is_empty() {
            let color = if node.estimate_mismatch {
                cx.theme().warning
            } else {
                cx.theme().muted_foreground
            };
            view = view.child(
                div()
                    .text_sm()
                    .text_color(color)
                    .child(node.summary.clone()),
            );
        }
        if expanded {
            view = view.child(
                div()
                    .pl_4()
                    .text_sm()
                    .whitespace_normal()
                    .child(node.details.clone()),
            );
            for (i, child) in node.children.iter().enumerate() {
                view = view.child(
                    div()
                        .pl_4()
                        .child(self.node(child, format!("{key}-{i}"), cx)),
                );
            }
        }
        view.into_any_element()
    }
}
impl Render for PlanView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let actions = h_flex()
            .gap_2()
            .child(
                Button::new("plan-raw")
                    .label(if self.show_raw { "Tree" } else { "Raw output" })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_raw = !this.show_raw;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("copy-plan")
                    .label("Copy plan")
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(this.plan.raw.clone()));
                    })),
            );
        let mut view = v_flex().id("query-plan").size_full().overflow_y_scroll().gap_2().p_3()
            .child(actions)
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child("Costs are estimates, not milliseconds. Actual timings are per loop and include child work."))
            .child(div().text_sm().whitespace_normal().child(self.source.clone()));
        if self.show_raw {
            view = view.child(
                div()
                    .text_sm()
                    .whitespace_normal()
                    .child(self.plan.raw.clone()),
            );
        } else {
            for (i, node) in self.plan.nodes.iter().enumerate() {
                view = view.child(self.node(node, format!("plan-{i}"), cx));
            }
        }
        view
    }
}
