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

/// The bar's bottom hairline. Render it before the tabs so the active tab can
/// paint over it and open into the document below.
pub fn tab_bar_baseline(cx: &App) -> Div {
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h_px()
        .bg(cx.theme().border)
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
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    context: impl Into<SharedString>,
    icon: IconName,
    selected: bool,
    cx: &App,
) -> Tab {
    let context = context.into();
    Tab::new(id)
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
                    // Hover drops the flares so it never overlaps a neighbour, and stops
                    // above the baseline, which stays visible.
                    view.left_0()
                        .right_0()
                        .bottom(px(1.))
                        .invisible()
                        .group_hover("document-tab", |style| style.visible())
                        .child(div().size_full().rounded_t(radius(cx)).bg(
                            if cx.theme().is_dark() {
                                // Move toward the active tab without outshining it.
                                cx.theme().tab_active.opacity(0.75)
                            } else {
                                cx.theme().secondary_hover
                            },
                        ))
                })
                .when(selected, |view| {
                    view.child(folder_tab_fill(
                        radius(cx),
                        cx.theme().tab_active,
                        cx.theme().border,
                    ))
                }),
        )
        .child(
            Icon::new(icon)
                .small()
                .flex_shrink_0()
                .when(selected, |icon| {
                    icon.text_color(super::theme::accent_text(cx))
                }),
        )
        .child(
            div()
                .id("document-tab-title")
                .min_w_0()
                .text_ellipsis()
                .child(title.into())
                .tooltip(move |window, cx| Tooltip::new(context.clone()).build(window, cx)),
        )
}

// The selected tab's surface, with flared bottom corners that open into the
// document. Its hairline edge joins the bar's baseline on both sides.
fn folder_tab_fill(radius: Pixels, background: Hsla, outline: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (left, right) = (bounds.left(), bounds.right());
            let (tab_left, tab_right) = (left + radius, right - radius);
            let radii = point(radius, radius);
            let edge = |mut path: PathBuilder, top: Pixels, bottom: Pixels| {
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
                path
            };
            let mut fill = edge(
                PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::tolerance(0.02))),
                bounds.top(),
                bounds.bottom(),
            );
            fill.close();
            if let Ok(path) = fill.build() {
                window.paint_path(path, background);
            }
            // Centre the hairline half a pixel inside the edge so its top is not
            // clipped and its feet sit exactly on the 1px baseline.
            if let Ok(path) = edge(
                PathBuilder::stroke(px(1.)),
                bounds.top() + px(0.5),
                bounds.bottom() - px(0.5),
            )
            .build()
            {
                window.paint_path(path, outline);
            }
        },
    )
    .size_full()
}
