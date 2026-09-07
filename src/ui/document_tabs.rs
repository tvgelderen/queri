use gpui_kit::assets::IconName;
use gpui_kit::base::{Tab, Tabs};
use gpui_kit::component::{
    ActiveTheme, Icon, InteractiveElementExt, Sizable, h_flex, scroll::ScrollableMask,
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

const TAB_HEIGHT: f32 = 32.;

fn radius(cx: &App) -> Pixels {
    cx.theme().radius_lg.min(px(TAB_HEIGHT) / 3.)
}

pub fn document_tab_bar(
    scroll: &ScrollHandle,
    tabs: impl IntoIterator<Item = Tab>,
    cx: &App,
) -> Tabs {
    let overhang = radius(cx);
    Tabs::new("document-tabs")
        .flex()
        .items_center()
        .min_w_0()
        .bg(cx.theme().tab_bar)
        .child(
            h_flex()
                .relative()
                .flex_1()
                .min_w_0()
                .overflow_x_hidden()
                .child(
                    h_flex()
                        .id("document-tabs-scroll")
                        .flex_1()
                        .min_w_0()
                        .pt_1()
                        .overflow_x_scroll()
                        .lock_scroll_axis()
                        .track_scroll(scroll)
                        .children(tabs.into_iter().enumerate().map(move |(index, tab)| {
                            // Include the shoulders in each scroll target so the
                            // first and last tabs keep their full outline when revealed.
                            div()
                                .flex_shrink_0()
                                .px(overhang)
                                .when(index > 0, |view| view.ml(px(4.) - overhang * 2.))
                                .child(tab)
                        })),
                )
                .child(ScrollableMask::vertical_wheel(scroll)),
        )
}

pub fn document_tab(
    index: usize,
    title: impl Into<SharedString>,
    context: impl Into<SharedString>,
    icon: IconName,
    selected: bool,
    cx: &App,
) -> Tab {
    let context = context.into();
    Tab::new(index)
        .selected(selected)
        .accessibility_label(context.clone())
        .group("document-tab")
        .relative()
        .flex_shrink_0()
        .max_w(px(240.))
        .h(px(TAB_HEIGHT))
        .px_3()
        .pb_1()
        .gap_2()
        .text_sm()
        .text_color(if selected {
            cx.theme().tab_active_foreground
        } else {
            cx.theme().tab_foreground
        })
        .hover(|style| style.text_color(cx.theme().tab_active_foreground))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(-radius(cx))
                .right(-radius(cx))
                .when(!selected, |view| {
                    view.invisible()
                        .group_hover("document-tab", |style| style.visible())
                })
                .child(folder_tab_fill(
                    radius(cx),
                    if selected {
                        cx.theme().tab_active
                    } else {
                        cx.theme().secondary_hover
                    },
                )),
        )
        .child(Icon::new(icon).small().flex_shrink_0())
        .child(
            div()
                .id("document-tab-title")
                .min_w_0()
                .text_ellipsis()
                .child(title.into())
                .tooltip(move |window, cx| Tooltip::new(context.clone()).build(window, cx)),
        )
}

// One outline for selection and hover, including the flared bottom corners.
fn folder_tab_fill(radius: Pixels, background: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (left, right) = (bounds.left(), bounds.right());
            let (top, bottom) = (bounds.top(), bounds.bottom());
            let (tab_left, tab_right) = (left + radius, right - radius);
            let radii = point(radius, radius);
            let mut path =
                PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::tolerance(0.02)));
            path.move_to(point(left, bottom));
            path.arc_to(
                radii,
                px(0.),
                false,
                false,
                point(tab_left, bottom - radius),
            );
            path.line_to(point(tab_left, top + radius));
            path.arc_to(radii, px(0.), false, true, point(tab_left + radius, top));
            path.line_to(point(tab_right - radius, top));
            path.arc_to(radii, px(0.), false, true, point(tab_right, top + radius));
            path.line_to(point(tab_right, bottom - radius));
            path.arc_to(radii, px(0.), false, false, point(right, bottom));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, background);
            }
        },
    )
    .size_full()
}
