use gpui_kit::component::WindowExt;
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Context, HeadlessAppContext, Render, Window, WindowBounds, WindowOptions,
    div, px, rgb, size,
};
use queri::{
    assets::Assets,
    database,
    model::*,
    storage::Store,
    ui::{Services, Workspace},
};
use std::sync::Arc;

struct FixedPixels;

impl Render for FixedPixels {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0xffffff))
            .p(px(64.))
            .child(div().w(px(80.)).h(px(40.)).bg(rgb(0xff0000)))
    }
}

fn main() {
    let mut fixture_directories = Vec::new();
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.allow_parking();
    cx.update(gpui_kit::init);
    let options = || WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(900.), px(600.)),
        })),
        show: false,
        focus: false,
        ..Default::default()
    };
    let (handle, _) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |_, cx| cx.new(|_| FixedPixels)).unwrap()
    });
    let dimensions = cx.capture_screenshot(handle).unwrap().dimensions();
    let dpi = dimensions.0 as f32 / 900.;
    for zoom in [1., 1.25, 2.] {
        cx.update_window(handle, |_, window, cx| {
            window.set_content_zoom(zoom);
            window.render_frame(cx);
        })
        .unwrap();
        let image = cx.capture_screenshot(handle).unwrap();
        assert_eq!(
            image.dimensions(),
            dimensions,
            "zoom preserves native window dimensions"
        );
        let y = (84. * zoom * dpi) as u32;
        let red_pixels = (0..image.width())
            .filter(|&x| image.get_pixel(x, y).0 == [255, 0, 0, 255])
            .count();
        assert_eq!(
            red_pixels,
            (80. * zoom * dpi) as usize,
            "actual rendered pixels at {zoom}"
        );
    }
    for (appearance, name) in [(Appearance::Light, "light"), (Appearance::Dark, "dark")] {
        let directory = tempfile::tempdir().unwrap();
        let services = Services::new(
            database::runtime().unwrap(),
            Store::open(directory.path().join("app.db")).unwrap(),
        );
        let (handle, _) = cx.update(|cx| {
            gpui_kit::open_window(options(), cx, |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        services,
                        Preferences {
                            appearance,
                            ui_zoom_percent: UiZoom::from(200),
                            ..Default::default()
                        },
                        vec![],
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        save_screenshot(&mut cx, handle, &format!("workspace-200-{name}"));
        cx.update_window(handle, |_, window, cx| {
            window.scroll(
                "workspace-viewport",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-1000.))),
                cx,
            );
            window.click("preferences", cx);
        })
        .unwrap();
        cx.advance_clock(std::time::Duration::from_secs(1));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("ui-zoom-out").visible());
            assert!(window.find("ui-zoom-reset").visible());
            let dialog = window.find("surface").bounds();
            let viewport = Bounds::new(Default::default(), window.viewport_size());
            assert!(
                viewport.contains(&dialog.origin) && viewport.contains(&dialog.bottom_right()),
                "dialog fits at 200%: {dialog:?} in {viewport:?}"
            );
            assert_eq!(window.content_zoom(), 2.);
        })
        .unwrap();
        save_screenshot(&mut cx, handle, &format!("preferences-200-{name}"));
        cx.update_window(handle, |_, window, cx| window.close_dialog(cx))
            .unwrap();
        fixture_directories.push(directory);
    }
}

fn save_screenshot(cx: &mut HeadlessAppContext, handle: gpui_kit::AnyWindowHandle, name: &str) {
    if let Ok(directory) = std::env::var("QUERI_ZOOM_SCREENSHOT_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        cx.capture_screenshot(handle)
            .unwrap()
            .save(std::path::Path::new(&directory).join(format!("{name}.png")))
            .unwrap();
    }
}
