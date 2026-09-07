use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::{Theme, ThemeMode, WindowExt};
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Context, HeadlessAppContext, Render, TestSupportExt, Window, WindowBounds,
    WindowOptions, div, px, rgb, size,
};
use std::{sync::Arc, time::Duration};

struct BackdropFixture {
    reversed: bool,
}

impl Render for BackdropFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(if self.reversed { 0xff0000 } else { 0x0000ff }))
            .child(div().w(px(64.)).h_full().bg(rgb(if self.reversed {
                0x0000ff
            } else {
                0xff0000
            })))
    }
}

fn main() {
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        verify_dialog_backdrop(mode);
    }
}

fn verify_dialog_backdrop(mode: ThemeMode) {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(queri::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(mode, None, cx);
        Theme::update(cx, |theme| {
            theme.dialog_backdrop_blur = px(8.);
        });
    });
    let (handle, fixture) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(640.), px(480.)),
                    })),
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                cx,
                |_, cx| cx.new(|_| BackdropFixture { reversed: false }),
            )
        })
        .expect("offscreen window");
    cx.update_window(handle, |_, window, cx| {
        window.open_dialog(cx, |dialog, _, _| {
            dialog
                .entrance(DialogEntrance::None)
                .title("Foreground stays sharp")
                .child(
                    div()
                        .flex()
                        .h(px(48.))
                        .child(
                            div()
                                .id("sharp-red")
                                .test_support()
                                .w(px(48.))
                                .bg(rgb(0xff0000)),
                        )
                        .child(div().w(px(48.)).bg(rgb(0x0000ff))),
                )
        });
        window.render_frame(cx);
        assert_eq!(
            window.painted_backdrop_blurs().len(),
            1,
            "one shared blur operation"
        );
    })
    .expect("open dialog");
    let image = cx.capture_screenshot(handle).expect("real GPU rendering");
    let scale = image.width() as f32 / 640.;
    let mixed = image
        .get_pixel(image.width() * 60 / 640, image.height() * 360 / 480)
        .0;
    assert!(
        mixed[0] > 20 && mixed[2] > 20,
        "{mode:?}: backdrop must mix neighboring colors, not just dim them: {mixed:?}"
    );
    let bounds = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("sharp-red").bounds()
        })
        .expect("foreground bounds");
    let x = (f32::from(bounds.right()) * scale) as u32;
    let y = (f32::from(bounds.center().y) * scale) as u32;
    assert_eq!(image.get_pixel(x - 2, y).0, [255, 0, 0, 255]);
    assert_eq!(image.get_pixel(x + 2, y).0, [0, 0, 255, 255]);

    // A second dialog must hand over the single sampling operation, not blur
    // an already blurred workspace again.
    cx.update_window(handle, |_, window, cx| {
        window.open_alert_dialog(cx, |dialog, _, _| {
            dialog.entrance(DialogEntrance::None).title("Confirmation")
        });
        window.render_frame(cx);
        assert_eq!(window.painted_backdrop_blurs().len(), 1);
    })
    .expect("stack confirmation");
    let stacked = cx.capture_screenshot(handle).expect("stacked GPU frame");
    let sample_x = image.width() * 60 / 640;
    let sample_y = image.height() * 360 / 480;
    assert_eq!(
        stacked.get_pixel(sample_x, sample_y),
        image.get_pixel(sample_x, sample_y)
    );

    cx.update_window(handle, |_, window, cx| {
        window.close_dialog(cx);
        window.render_frame(cx);
        assert_eq!(window.painted_backdrop_blurs().len(), 1);
        fixture.update(cx, |fixture, cx| {
            fixture.reversed = true;
            cx.notify();
        });
        window.resize(size(px(639.), px(477.)));
    })
    .expect("close top dialog, update workspace, and resize");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .expect("resized frame");
    let resized = cx.capture_screenshot(handle).expect("resized GPU frame");
    let scale = resized.width() as f32 / 639.;
    let mixed = resized
        .get_pixel((60. * scale) as u32, (360. * scale) as u32)
        .0;
    assert!(mixed[0] > 20 && mixed[2] > 20);
    assert!(
        resized
            .get_pixel((8. * scale) as u32, (360. * scale) as u32)
            .0[2]
            > 100,
        "blur must sample live workspace changes, not a stale screenshot"
    );

    cx.update_window(handle, |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!window.has_active_dialog(cx));
        assert!(window.painted_backdrop_blurs().is_empty());
    })
    .expect("Escape closes the remaining dialog");
    let closed = cx.capture_screenshot(handle).expect("closed GPU frame");
    assert_eq!(
        closed
            .get_pixel((60. * scale) as u32, (360. * scale) as u32)
            .0,
        [0, 0, 255, 255]
    );

    // Reduced motion keeps a static blur and does not leave it behind on close.
    cx.update(|cx| cx.set_reduce_motion(true));
    cx.update_window(handle, |_, window, cx| {
        window.open_alert_dialog(cx, |dialog, _, _| {
            dialog.title("Reduced-motion confirmation")
        });
        window.render_frame(cx);
        assert_eq!(window.painted_backdrop_blurs().len(), 1);
    })
    .expect("reduced-motion alert");
    let reduced = cx
        .capture_screenshot(handle)
        .expect("reduced-motion GPU frame");
    assert!(
        reduced
            .get_pixel((60. * scale) as u32, (360. * scale) as u32)
            .0[0]
            > 20
    );
    cx.update_window(handle, |_, window, cx| {
        window.close_dialog(cx);
        window.render_frame(cx);
        assert!(window.painted_backdrop_blurs().is_empty());
    })
    .expect("close reduced-motion alert");

    // Normal entrance/exit keeps the backdrop through the existing transition.
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| {
        window.open_dialog(cx, |dialog, _, _| dialog.title("Animated lifecycle"));
        window.render_frame(cx);
    })
    .expect("animated dialog");
    cx.advance_clock(Duration::from_millis(300));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.close_dialog(cx);
        window.render_frame(cx);
        assert_eq!(
            window.painted_backdrop_blurs().len(),
            1,
            "closing backdrop stays mounted"
        );
    })
    .expect("begin exit");
    cx.advance_clock(Duration::from_millis(300));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.painted_backdrop_blurs().is_empty());
    })
    .expect("finish exit");

    println!(
        "{mode:?}: blur, sharp foreground, stacking, resize, live updates, Escape, reduced motion, and exit verified"
    );
}
