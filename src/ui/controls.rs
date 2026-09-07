use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
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

pub fn brand_mark(size: Pixels, cx: &App) -> Svg {
    svg()
        .path("brand/queri-mark.svg")
        .text_color(brand_color(cx))
        .flex_shrink_0()
        .size(size)
}

pub fn brand_logo(height: Pixels, cx: &App) -> Stateful<Svg> {
    svg()
        .path("brand/queri-logo.svg")
        .text_color(brand_color(cx))
        .id("brand-logo")
        .role(Role::Image)
        .aria_label("Queri")
        .flex_shrink_0()
        .w(height * (232. / 64.))
        .h(height)
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

pub fn menu_item(label: impl Into<SharedString>, icon: impl Into<Icon>) -> PopupMenuItem {
    // GPUI Kit adds a 4px gap; another spacing step keeps menu labels at 8px.
    PopupMenuItem::new(label).icon(icon.into().mr_1())
}

pub fn destructive_menu_item(label: impl Into<SharedString>, disabled: bool) -> PopupMenuItem {
    let label = label.into();
    PopupMenuItem::element(move |_, cx| {
        // Reclaim Kit's 8px inset and 20px icon slot so hover fills the native row.
        // The menu still owns activation, dismissal, and disabled state.
        h_flex()
            .id("destructive-menu-item")
            .test_support()
            .flex_1()
            .ml_neg_7()
            .mr_neg_2()
            .px_2()
            .min_h(px(26.))
            .gap_2()
            .rounded(cx.theme().radius.min(px(8.)))
            .text_color(if disabled {
                cx.theme().muted_foreground
            } else {
                cx.theme().button_danger_foreground
            })
            .when(!disabled, |row| {
                row.hover(|style| style.bg(cx.theme().button_danger_hover))
            })
            .child(Icon::new(IconName::Trash).xsmall())
            .child(
                div()
                    .id("destructive-menu-label")
                    .test_support()
                    .role(Role::Label)
                    .aria_label(label.clone())
                    .aria_value(label.clone())
                    .child(label.clone()),
            )
    })
    .icon(Icon::empty().mr_1())
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
