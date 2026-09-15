use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, px, size};
use queri::schema::{Scope, Snapshot};
use std::sync::Arc;

fn fixture() -> (
    tempfile::TempDir,
    queri::ui::Services,
    Arc<queri::database::Session>,
    Snapshot,
    Snapshot,
) {
    use queri::{database::Session, model::*, storage::Store};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE customers(id INTEGER PRIMARY KEY, name TEXT, email TEXT); CREATE TABLE orders(id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id), total TEXT); CREATE TABLE order_items(id INTEGER PRIMARY KEY, order_id INTEGER REFERENCES orders(id), product TEXT, quantity INTEGER);").unwrap();
    let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
    let services = queri::ui::Services::new(
        runtime.clone(),
        Store::open(directory.path().join("store.db")).unwrap(),
    );
    let session = runtime
        .block_on(Session::connect(
            ConnectionProfile {
                id: uuid::Uuid::new_v4(),
                name: "Screenshot SQLite".into(),
                notes: String::new(),
                color: String::new(),
                ssh: None,
                environment: None,
                policy: ExecutionPolicy::Normal,
                remember_password: false,
                config: ConnectionConfig::Sqlite {
                    path,
                    read_only: false,
                    create: false,
                    wal: true,
                    foreign_keys: true,
                },
            },
            String::new(),
        ))
        .unwrap();
    let before = runtime
        .block_on(session.schema_snapshot(Scope {
            namespace: "main".into(),
        }))
        .unwrap();
    db.execute_batch(
        "CREATE TABLE shipments(id INTEGER PRIMARY KEY, order_id INTEGER REFERENCES orders(id));",
    )
    .unwrap();
    let after = runtime
        .block_on(session.schema_snapshot(Scope {
            namespace: "main".into(),
        }))
        .unwrap();
    db.execute_batch("DROP TABLE shipments").unwrap();
    (directory, services, session, before, after)
}

fn main() {
    let (_directory, services, session, before, after) = fixture();
    for (mode, name) in [(ThemeMode::Dark, "dark"), (ThemeMode::Light, "light")] {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(queri::assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            Theme::change(mode, None, cx);
            queri::ui::apply_theme(cx);
        });
        let (handle, diagram) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Default::default(),
                            size: size(px(1440.), px(900.)),
                        })),
                        show: false,
                        focus: false,
                        ..Default::default()
                    },
                    cx,
                    |_, cx| cx.new(|_| queri::ui::ErDiagram::new(before.clone())),
                )
            })
            .expect("schema window");
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .expect("render");
        cx.update(|cx| {
            let layout = serde_json::to_string(&diagram.read(cx).saved_layout()).unwrap();
            diagram.update(cx, |diagram, _| {
                diagram.restore_layout(serde_json::from_str(&layout).unwrap())
            });
            assert_eq!(
                serde_json::to_string(&diagram.read(cx).saved_layout()).unwrap(),
                layout
            );
            let svg = diagram.read(cx).svg();
            assert_eq!(
                svg.matches("<polyline ").count(),
                2,
                "only declared relationships"
            );
            assert_eq!(svg.matches("<rect ").count(), 3);
        });
        // Exercise the actual pointer handlers, then restore the fixture for capture.
        let layout_before =
            cx.update(|cx| serde_json::to_value(diagram.read(cx).saved_layout()).unwrap());
        cx.update_window(handle, |_, window, cx| {
            window.click("er-zoom-in", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.update(|cx| {
            let after = serde_json::to_value(diagram.read(cx).saved_layout()).unwrap();
            assert!(
                after["zoom"].as_f64().unwrap() > layout_before["zoom"].as_f64().unwrap(),
                "zoom button changes canvas transform"
            );
            diagram.update(cx, |diagram, _| {
                diagram.restore_layout(serde_json::from_value(layout_before).unwrap())
            });
        });
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        let image = cx.capture_screenshot(handle).expect("GPU screenshot");
        assert!(image.width() >= 1440 && image.height() >= 900);
        if let Ok(directory) = std::env::var("QUERI_SCREENSHOT_DIR") {
            std::fs::create_dir_all(&directory).expect("screenshot directory");
            image
                .save(std::path::Path::new(&directory).join(format!("er-diagram-{name}.png")))
                .expect("save screenshot");
        }
        macro_rules! capture {
            ($name:expr, $build:expr) => {{
                let (screen, entity) = cx
                    .update(|cx| {
                        gpui_kit::open_window(
                            WindowOptions {
                                window_bounds: Some(WindowBounds::Windowed(Bounds {
                                    origin: Default::default(),
                                    size: size(px(1440.), px(900.)),
                                })),
                                show: false,
                                focus: false,
                                ..Default::default()
                            },
                            cx,
                            $build,
                        )
                    })
                    .unwrap();
                cx.update_window(screen, |_, window, cx| window.render_frame(cx))
                    .unwrap();
                let image = cx.capture_screenshot(screen).unwrap();
                let directory = std::env::var("QUERI_SCREENSHOT_DIR")
                    .unwrap_or_else(|_| "/tmp/queri-schema-screens".into());
                std::fs::create_dir_all(&directory).unwrap();
                image
                    .save(std::path::Path::new(&directory).join(format!("{}-{name}.png", $name)))
                    .unwrap();
                (screen, entity)
            }};
        }
        capture!("designer", |window, cx| cx.new(|cx| {
            queri::ui::schema_designer::SchemaDesigner::new(
                before.clone(),
                Some(before.tables[0].clone()),
                window,
                cx,
            )
        }));
        let (review_window, review) = capture!("sql-review", |_, cx| cx.new(|_| {
            queri::ui::schema_review::SchemaReview::new(
                queri::schema::plan(&before, &after).unwrap(),
                services.clone(),
                session.clone(),
            )
        }));
        let (comparison_window, _) = capture!("comparison", |_, cx| cx.new(|_| {
            queri::ui::schema_compare::SchemaComparison::new(
                services.clone(),
                after.clone(),
                before.clone(),
                session.clone(),
            )
        }));
        cx.update_window(comparison_window, |_, window, cx| {
            window.click(("select-table", 0usize), cx);
            window.click("review-selection", cx);
            assert!(
                window.try_find("apply-schema").is_none(),
                "empty selection cannot enter review"
            );
            window.click(("select-table", 0usize), cx);
            window.click("review-selection", cx);
            assert!(
                window.try_find("apply-schema").is_some(),
                "selected table opens shared review"
            );
            window.click("back-schema", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(comparison_window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("review-selection").is_some());
        })
        .unwrap();
        capture!("endpoint-setup", |window, cx| cx.new(|cx| {
            queri::ui::schema_compare_setup::SchemaCompareSetup::new(
                services.clone(),
                vec![session.profile.clone()],
                Default::default(),
                Some(session.profile.clone()),
                window,
                cx,
            )
        }));
        cx.update_window(review_window, |_, window, cx| {
            window.click("apply-schema", cx)
        })
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            let state = cx.update(|cx| review.read(cx).state.clone());
            if state != queri::ui::schema_review::ReviewState::Applying {
                assert_eq!(state, queri::ui::schema_review::ReviewState::Succeeded);
                break;
            }
            assert!(std::time::Instant::now() < deadline, "apply timed out");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let db = rusqlite::Connection::open(_directory.path().join("fixture.db")).unwrap();
        let columns: i64 = db
            .query_row(
                "SELECT count(*) FROM pragma_table_info('shipments')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(columns, 2, "UI apply created the reviewed table");
        let (failed_window, failed) = capture!("stale-review", |_, cx| cx.new(|_| {
            queri::ui::schema_review::SchemaReview::new(
                queri::schema::plan(&before, &after).unwrap(),
                services.clone(),
                session.clone(),
            )
        }));
        cx.update_window(failed_window, |_, window, cx| {
            window.click("apply-schema", cx)
        })
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            let state = cx.update(|cx| failed.read(cx).state.clone());
            if state != queri::ui::schema_review::ReviewState::Applying {
                assert!(matches!(
                    state,
                    queri::ui::schema_review::ReviewState::Failed(_)
                ));
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        cx.update_window(failed_window, |_, window, cx| {
            window.render_frame(cx);
            window.click("back-schema", cx);
        })
        .unwrap();
        db.execute_batch("DROP TABLE shipments").unwrap();
        let mut profile = session.profile.clone();
        if let queri::model::ConnectionConfig::Sqlite { read_only, .. } = &mut profile.config {
            *read_only = true;
        }
        let readonly = services
            .runtime
            .block_on(queri::database::Session::connect(profile, String::new()))
            .unwrap();
        let (readonly_window, readonly_review) =
            capture!("readonly-review", |_, cx| cx.new(|_| {
                queri::ui::schema_review::SchemaReview::new(
                    queri::schema::plan(&before, &after).unwrap(),
                    services.clone(),
                    readonly.clone(),
                )
            }));
        cx.update_window(readonly_window, |_, window, cx| {
            window.click("apply-schema", cx)
        })
        .unwrap();
        assert_eq!(
            cx.update(|cx| readonly_review.read(cx).state.clone()),
            queri::ui::schema_review::ReviewState::Review
        );
    }
}
