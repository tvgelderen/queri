use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, Size,
    button::{Button, ButtonVariants},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle},
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
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
