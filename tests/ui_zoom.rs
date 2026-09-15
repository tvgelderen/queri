#![cfg(feature = "test-support")]

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Context, Entity, Modifiers, MouseMoveEvent, Render, ScrollDelta, ScrollEvent,
    ScrollHandle, TestAppContext, TestSupportExt, VisualTestContext, Window, WindowOptions, div,
    init, open_window, point, px, rgb, size,
};

struct ZoomFixture {
    input: Entity<InputState>,
    clicks: usize,
    scroll: ScrollHandle,
}

impl Render for ZoomFixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .p(px(64.))
            .gap_4()
            .child(
                div()
                    .id("zoom-target")
                    .test_support()
                    .flex_shrink_0()
                    .w(px(80.))
                    .h(px(40.))
                    .bg(rgb(0xff0000))
                    .on_click(cx.listener(|this, _, _, _| this.clicks += 1)),
            )
            .child(
                div()
                    .id("zoom-input")
                    .test_support()
                    .child(Input::new(&self.input)),
            )
            .child(
                div()
                    .id("zoom-scroll")
                    .test_support()
                    .w(px(100.))
                    .h(px(80.))
                    .flex_shrink_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(div().h(px(800.)).w_full().bg(rgb(0x00ff00))),
            )
    }
}

#[gpui_kit::test]
fn content_zoom_scales_fixed_pixels_and_maps_native_pointer_input(cx: &mut TestAppContext) {
    let (handle, fixture) = cx.update(|cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value("SELECT 123"));
            cx.new(|_| ZoomFixture {
                input,
                clicks: 0,
                scroll: ScrollHandle::new(),
            })
        })
        .unwrap()
    });
    let mut visual = VisualTestContext::from_window(handle, cx);
    for dpi in [1., 1.25, 2.] {
        cx.simulate_window_scale_factor_change(handle, dpi);
        for zoom in [0.8, 1., 1.25, 2.] {
            let target = cx
                .update_window(handle, |_, window, cx| {
                    window.set_content_zoom(zoom);
                    window.render_frame(cx);
                    let bounds = window.find("zoom-target").bounds();
                    let painted = window
                        .painted_quads()
                        .into_iter()
                        .find(|quad| quad.background == rgb(0xff0000).into())
                        .unwrap();
                    assert!((painted.bounds.size.width.0 - 80. * dpi * zoom).abs() <= 1.);
                    let platform_size = window.bounds().size;
                    assert!(
                        (f32::from(window.viewport_size().width) * zoom
                            - f32::from(platform_size.width))
                        .abs()
                            < 0.01
                    );
                    bounds.center() * zoom
                })
                .unwrap();
            let before = cx.read(|cx| fixture.read(cx).clicks);
            visual.simulate_event(MouseMoveEvent {
                position: target,
                ..Default::default()
            });
            visual.simulate_click(target, Modifiers::default());
            assert_eq!(cx.read(|cx| fixture.read(cx).clicks), before + 1);
        }
    }
    cx.simulate_window_resize(handle, size(px(900.), px(600.)));
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.content_zoom(), 2.);
        assert_eq!(window.viewport_size(), size(px(450.), px(300.)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn content_zoom_maps_pixel_scrolling_without_scaling_line_counts(cx: &mut TestAppContext) {
    let (handle, fixture) = cx.update(|cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx));
            cx.new(|_| ZoomFixture {
                input,
                clicks: 0,
                scroll: ScrollHandle::new(),
            })
        })
        .unwrap()
    });
    let position = cx
        .update_window(handle, |_, window, cx| {
            window.set_content_zoom(2.);
            window.render_frame(cx);
            window.find("zoom-scroll").bounds().center() * 2.
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle, cx);
    visual.simulate_event(ScrollEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
        ..Default::default()
    });
    let offset = cx.read(|cx| fixture.read(cx).scroll.offset().y);
    assert_eq!(offset, px(-20.));
    visual.simulate_event(ScrollEvent {
        position,
        delta: ScrollDelta::Lines(point(0., -1.)),
        ..Default::default()
    });
    assert!(cx.read(|cx| fixture.read(cx).scroll.offset().y) < offset);
}

#[gpui_kit::test]
fn content_zoom_converts_native_text_input_geometry(cx: &mut TestAppContext) {
    use gpui_kit::{ElementInputHandler, InputHandler, PlatformInputHandler};
    let (handle, fixture) = cx.update(|cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value("SELECT 123"));
            cx.new(|_| ZoomFixture {
                input,
                clicks: 0,
                scroll: ScrollHandle::new(),
            })
        })
        .unwrap()
    });
    let (mut native, mut content) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let bounds = window.find("zoom-input").bounds();
            let input = fixture.read(cx).input.clone();
            (
                PlatformInputHandler::new(
                    window.to_async(cx),
                    Box::new(ElementInputHandler::new(bounds, input.clone())),
                ),
                ElementInputHandler::new(bounds, input),
            )
        })
        .unwrap();
    for zoom in [0.8, 1.25, 2.] {
        let (caret, rectangles, index) = cx
            .update_window(handle, |_, window, cx| {
                window.set_content_zoom(zoom);
                window.render_frame(cx);
                let caret = content
                    .bounds_for_range(0..0, window, cx)
                    .expect("laid-out caret");
                (
                    caret,
                    content.selection_bounds_for_range(0..6, window, cx),
                    content.character_index_for_point(caret.origin, window, cx),
                )
            })
            .unwrap();
        assert_eq!(native.bounds_for_range(0..0), Some(caret.map(|v| v * zoom)));
        assert_eq!(
            native.selection_bounds_for_range(0..6),
            rectangles
                .into_iter()
                .map(|b| b.map(|v| v * zoom))
                .collect::<Vec<_>>()
        );
        assert_eq!(native.character_index_for_point(caret.origin * zoom), index);
    }
}
