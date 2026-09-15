use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, Size,
    button::{Button, ButtonVariants},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle},
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
    scroll::ScrollableElement,
    switch::Switch,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;

#[cfg(all(test, feature = "test-support"))]
#[path = "controls_tests.rs"]
mod tests;

pub fn shortcut(keys: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .px_1p5()
        .py_0p5()
        .rounded(cx.theme().radius_tokens().sm)
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .font_family(cx.theme().mono_font_family.clone())
        .child(keys.into())
}

fn brand_color(cx: &App) -> Hsla {
    rgb(if cx.theme().is_dark() {
        0xfcfcfb
    } else {
        0x242422
    })
    .into()
}

/// The mark in ink with its tail cell in brand blue, as on queri.dev.
fn branded(body: &'static str, width: Pixels, height: Pixels, cx: &App) -> Div {
    div()
        .relative()
        .flex_shrink_0()
        .w(width)
        .h(height)
        .child(
            svg()
                .path(body)
                .text_color(brand_color(cx))
                .absolute()
                .inset_0()
                .size_full(),
        )
        .child(
            svg()
                .path("brand/queri-tail.svg")
                .text_color(rgb(super::theme::BRAND_BLUE))
                .absolute()
                .top_0()
                .left_0()
                .size(height),
        )
}

pub fn brand_mark(size: Pixels, cx: &App) -> Div {
    branded("brand/queri-mark-body.svg", size, size, cx)
}

pub fn brand_logo(height: Pixels, cx: &App) -> Stateful<Div> {
    branded(
        "brand/queri-logo-body.svg",
        height * (222. / 64.),
        height,
        cx,
    )
    .id("brand-logo")
    .role(Role::Image)
    .aria_label("Queri")
}

pub fn empty_state(
    icon: impl Into<Option<IconName>>,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> Empty {
    Empty::new().size_full().gap_5().header(
        EmptyHeader::new()
            .when_some(icon.into(), |header, icon| {
                header.media(
                    EmptyMedia::new()
                        .with_variant(EmptyMediaVariant::Icon)
                        .size_12()
                        .child(Icon::new(icon).large()),
                )
            })
            .title(EmptyTitle::new().text_lg().child(title.into()))
            .description(EmptyDescription::new().child(description.into())),
    )
}

pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: impl Into<SharedString>,
) -> Button {
    let label = label.into();
    Button::new(id)
        .ghost()
        .icon(icon)
        .accessibility_label(label.clone())
        .tooltip(label)
}

/// A quiet, full-width navigation row for the sidebar; the shortcut appears in its tooltip.
pub fn nav_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: impl Into<SharedString>,
    hint: Option<&'static str>,
    cx: &App,
) -> Button {
    let label = label.into();
    Button::new(id)
        .ghost()
        .w_full()
        .accessibility_label(label.clone())
        .when_some(hint, |button, hint| {
            button.tooltip(format!("{label} · {hint}"))
        })
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .gap_2p5()
                .child(
                    Icon::new(icon)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(div().flex_1().min_w_0().text_ellipsis().child(label)),
        )
}

/// Small uppercase section label, echoing the site's eyebrow style.
pub fn section_label(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().muted_foreground)
        .child(label.into())
}

/// A compact pill for applied state such as filters, sorts and scopes.
pub fn chip(cx: &App) -> Div {
    h_flex()
        .h_7()
        .min_w_0()
        .pl_2p5()
        .pr_0p5()
        .gap_1p5()
        .rounded_full()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .text_xs()
}

/// A small numeric badge tinted with the brand accent.
pub fn count_badge(count: usize, cx: &App) -> Div {
    div()
        .min_w(px(18.))
        .h(px(18.))
        .px_1()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(super::theme::accent_soft(cx))
        .text_color(super::theme::accent_text(cx))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .child(count.to_string())
}

pub fn toolbar_divider(cx: &App) -> Div {
    div()
        .flex_shrink_0()
        .w_px()
        .h_4()
        .mx_1()
        .bg(cx.theme().border)
}

pub fn menu_item(label: impl Into<SharedString>, icon: impl Into<Icon>) -> PopupMenuItem {
    // GPUI Kit adds a 4px gap; another spacing step keeps menu labels at 8px.
    PopupMenuItem::new(label).icon(icon.into().mr_1())
}

pub fn destructive_menu_item(label: impl Into<SharedString>, disabled: bool) -> PopupMenuItem {
    menu_item(label, IconName::Trash)
        .destructive(true)
        .disabled(disabled)
}

pub fn disclosure_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    expanded: bool,
) -> Button {
    let label = label.into();
    Button::new(id)
        .ghost()
        .w_full()
        .accessibility_label(label.clone())
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .gap_2()
                .child(div().min_w_0().text_ellipsis().child(label))
                .child(
                    Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .with_size(Size::Medium),
                ),
        )
}

pub fn setting_switch(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    let label = label.into();
    let on_change = Rc::new(on_change);
    let on_toggle = on_change.clone();
    h_flex()
        .id((id.clone(), "setting-row"))
        .w_full()
        .justify_between()
        .gap_4()
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_change(&!checked, window, cx);
        })
        .child(div().min_w_0().text_sm().child(label.clone()))
        .child(
            div().flex_shrink_0().child(
                Switch::new(id)
                    .accessibility_label(label)
                    .checked(checked)
                    .on_change(move |on, window, cx| {
                        cx.stop_propagation();
                        on_toggle(on, window, cx);
                    }),
            ),
        )
}

/// Small, explicit option menus for settings whose choices fit without search.
pub fn choice<T: Clone + PartialEq + 'static>(
    button: Button,
    selected: T,
    options: impl IntoIterator<Item = (T, SharedString)>,
    on_select: impl Fn(&T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let options = options.into_iter().collect::<Vec<_>>();
    let on_select = Rc::new(on_select);
    button
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for (value, label) in &options {
                let handler = on_select.clone();
                let value = value.clone();
                menu = menu.item(
                    PopupMenuItem::new(label.clone())
                        .checked(value == selected)
                        .on_click(move |_, window, cx| handler(&value, window, cx)),
                );
            }
            menu
        })
}

/// Title row shared by full-page tools: a title, quiet context, and trailing actions.
pub fn page_header(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    actions: impl IntoIterator<Item = AnyElement>,
    cx: &App,
) -> Div {
    h_flex()
        .flex_shrink_0()
        .min_h_12()
        .px_4()
        .py_2()
        .gap_4()
        .justify_between()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            gpui_kit::component::v_flex()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_ellipsis()
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .text_ellipsis()
                        .child(subtitle.into()),
                ),
        )
        .child(h_flex().flex_shrink_0().gap_1p5().children(actions))
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    Neutral,
    Success,
    Warning,
    Danger,
}

impl Tone {
    pub fn color(self, cx: &App) -> Hsla {
        match self {
            Self::Neutral => cx.theme().muted_foreground,
            Self::Success => cx.theme().success,
            Self::Warning => cx.theme().warning,
            Self::Danger => cx.theme().danger,
        }
    }
    fn icon(self) -> IconName {
        match self {
            Self::Neutral => IconName::Info,
            Self::Success => IconName::CircleCheck,
            Self::Warning => IconName::TriangleAlert,
            Self::Danger => IconName::CircleAlert,
        }
    }
}

/// An inline status message with a tone-tinted icon; errors are announced as alerts.
pub fn notice(
    id: impl Into<ElementId>,
    tone: Tone,
    message: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    let color = tone.color(cx);
    h_flex()
        .id(id)
        .role(if tone == Tone::Danger {
            Role::Alert
        } else {
            Role::Status
        })
        .w_full()
        .items_start()
        .gap_2()
        .px_3()
        .py_2()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(if tone == Tone::Neutral {
            cx.theme().border
        } else {
            color.opacity(0.28)
        })
        .bg(if tone == Tone::Neutral {
            cx.theme().muted
        } else {
            color.opacity(0.08)
        })
        .text_sm()
        .line_height(relative(1.5))
        .child(
            div()
                .flex_shrink_0()
                .pt(px(2.))
                .text_color(color)
                .child(Icon::new(tone.icon()).small()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .child(message.into()),
        )
}

/// A bordered surface for grouped content.
pub fn card(cx: &App) -> Div {
    gpui_kit::component::v_flex()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
}

/// A small tinted label for kinds and states.
pub fn badge(label: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .flex_shrink_0()
        .px_1p5()
        .py(px(1.))
        .rounded(px(4.))
        .bg(color.opacity(0.12))
        .text_color(color)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .child(label.into())
}

/// A tinted square that carries the dialog's tone before any text is read.
pub fn dialog_icon(icon: IconName, tone: Tone, cx: &App) -> Div {
    let (fg, bg) = match tone {
        Tone::Neutral => (super::theme::accent_text(cx), super::theme::accent_soft(cx)),
        tone => (tone.color(cx), tone.color(cx).opacity(0.12)),
    };
    div()
        .flex_shrink_0()
        .size_9()
        .flex()
        .items_center()
        .justify_center()
        .rounded(cx.theme().radius)
        .bg(bg)
        .text_color(fg)
        .child(Icon::new(icon))
}

/// Read-only SQL with editor syntax colors and a line-number gutter.
pub fn sql_code(sql: &str, cx: &App) -> Div {
    use gpui_kit::component::{highlighter::SyntaxHighlighter, input::Rope};
    let mut highlighter = SyntaxHighlighter::new("sql");
    highlighter.update(None, &Rope::from_str(sql), None);
    let styles = highlighter.styles(&(0..sql.len()), cx.theme().highlight_theme.as_ref());
    let muted = cx.theme().muted_foreground;
    let mut start = 0;
    let lines = sql.split('\n').enumerate().map(|(index, line)| {
        let end = start + line.len();
        let highlights = styles
            .iter()
            .filter_map(|(range, style)| {
                let from = range.start.max(start);
                let to = range.end.min(end);
                (from < to).then(|| (from - start..to - start, *style))
            })
            .collect::<Vec<_>>();
        start = end + 1;
        let text = if line.trim().is_empty() {
            StyledText::new(" ")
        } else {
            StyledText::new(line.to_owned()).with_highlights(highlights)
        };
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
            .child(div().flex_1().min_w_0().pr_4().child(text))
    });
    gpui_kit::component::v_flex()
        .py_2p5()
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(cx.theme().mono_font_size)
        .line_height(relative(1.65))
        .text_color(cx.theme().foreground)
        .children(lines)
}

/// A framed SQL preview: a quiet header naming where it runs, then scrollable code.
pub fn sql_preview(
    id: impl Into<ElementId>,
    target: impl Into<SharedString>,
    meta: impl Into<SharedString>,
    sql: &str,
    cx: &App,
) -> Div {
    let muted = cx.theme().muted_foreground;
    card(cx)
        .w_full()
        .overflow_hidden()
        .child(
            h_flex()
                .h_9()
                .px_3()
                .gap_3()
                .justify_between()
                .bg(cx.theme().muted)
                .border_b_1()
                .border_color(cx.theme().border)
                .text_xs()
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_2()
                        .child(Icon::new(IconName::Database).xsmall().text_color(muted))
                        .child(
                            div()
                                .min_w_0()
                                .text_ellipsis()
                                .font_weight(FontWeight::MEDIUM)
                                .child(target.into()),
                        ),
                )
                .child(div().flex_shrink_0().text_color(muted).child(meta.into())),
        )
        .child(
            div()
                .id(id)
                .max_h(px(280.))
                .overflow_y_scrollbar()
                .child(sql_code(sql, cx)),
        )
}
