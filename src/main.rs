use gpui_kit::*;
use queri::{
    assets::Assets,
    database,
    storage::Store,
    ui::{Services, Workspace},
};

fn main() -> anyhow::Result<()> {
    let store = Store::open(Store::default_path()?)?;
    let preferences = store.preferences()?;
    let profiles = store.profiles()?;
    let services = Services::new(database::runtime()?, store);
    application().with_assets(Assets).run(move |cx| {
        init(cx);
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Queri".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(
                    px(preferences.window_width.max(900.)),
                    px(preferences.window_height.max(600.)),
                ),
                cx,
            ))),
            window_min_size: Some(size(px(900.), px(600.))),
            app_id: Some("dev.queri.Queri".into()),
            ..Default::default()
        };
        if let Err(error) = open_window(options, cx, |window, cx| {
            cx.new(|cx| Workspace::new(services, preferences, profiles, window, cx))
        }) {
            eprintln!("Could not open Queri: {error:#}");
            cx.quit();
        }
        cx.activate(true);
    });
    Ok(())
}
