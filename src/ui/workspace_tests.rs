use super::{ConnectionState, Workspace};
use crate::ui::test_support::click_label;
use crate::{database::runtime, model::*, storage::Store, ui::Services};
use gpui_kit::test::{TestAppContextExt, TestEventExt, TestQueryExt, TestWindowExt};
use gpui_kit::{
    AppContext, Bounds, MouseButton, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use std::time::Duration;
use uuid::Uuid;

#[gpui_kit::test]
async fn middle_click_closes_tabs_without_selecting_them(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("catalog.sqlite");
    rusqlite::Connection::open(&path).expect("database");
    let profile = ConnectionProfile {
        ssh: None,
        environment: None,
        policy: ExecutionPolicy::Normal,
        id: Uuid::new_v4(),
        name: "Catalog".into(),
        notes: String::new(),
        color: String::new(),
        remember_password: false,
        config: ConnectionConfig::Sqlite {
            path,
            read_only: false,
            create: false,
            wal: true,
            foreign_keys: true,
        },
    };
    let store = Store::open(directory.path().join("app.db")).expect("store");
    let services = Services::new(runtime().expect("runtime"), store);
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Workspace::new(
                    services,
                    Preferences {
                        active_connection: Some(profile.id),
                        ..Default::default()
                    },
                    vec![profile],
                    window,
                    cx,
                )
            })
        })
        .expect("workspace")
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        for _ in 0..3 {
            window.click("new-query", cx);
        }
    })
    .expect("open three query tabs");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).documents.len() == 3
            && workspace
                .read(cx)
                .documents
                .iter()
                .all(|doc| !doc.read(cx).busy())
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        let active = workspace.read(cx).documents[2].clone();
        let tab = window
            .within("document-tabs")
            .find(0_usize)
            .bounds()
            .center();
        window.pointer_move(tab, None, cx);
        window.pointer_down(tab, MouseButton::Right, cx);
        window.pointer_up(tab, MouseButton::Right, cx);
        assert_eq!(workspace.read(cx).documents.len(), 3);
        assert_eq!(workspace.read(cx).active, 2);

        window.pointer_down(tab, MouseButton::Middle, cx);
        assert_eq!(workspace.read(cx).documents.len(), 3);
        let outside = window.find("new-query").bounds().center();
        window.pointer_move(outside, Some(MouseButton::Middle), cx);
        window.pointer_up(outside, MouseButton::Middle, cx);
        assert_eq!(workspace.read(cx).documents.len(), 3);

        window.pointer_move(tab, None, cx);
        window.pointer_down(tab, MouseButton::Middle, cx);
        window.pointer_up(tab, MouseButton::Middle, cx);
        assert_eq!(workspace.read(cx).documents.len(), 2);
        assert_eq!(workspace.read(cx).active, 1);
        assert_eq!(workspace.read(cx).documents[1], active);

        let tab = window
            .within("document-tabs")
            .find(1_usize)
            .bounds()
            .center();
        window.pointer_move(tab, None, cx);
        window.pointer_down(tab, MouseButton::Middle, cx);
        window.pointer_up(tab, MouseButton::Middle, cx);
        assert_eq!(workspace.read(cx).documents.len(), 1);
        assert_eq!(workspace.read(cx).active, 0);

        let close = window.find(("close-tab", 0_usize)).bounds().center();
        window.pointer_move(close, None, cx);
        window.pointer_down(close, MouseButton::Middle, cx);
        window.pointer_up(close, MouseButton::Middle, cx);
        assert!(workspace.read(cx).documents.is_empty());
        assert!(window.find("new-query").visible());
    })
    .expect("close inactive, active, and final tabs with middle-click");
}

#[gpui_kit::test]
async fn find_unopened_table_and_use_history_in_compact_workspace(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("catalog.sqlite");
    rusqlite::Connection::open(&path).expect("database")
        .execute_batch("CREATE TABLE products(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO products VALUES(1,'Keyboard'); CREATE TABLE orders(id INTEGER PRIMARY KEY);")
        .expect("tables");
    let profile = ConnectionProfile {
        ssh: None,
        environment: None,
        policy: ExecutionPolicy::Normal,
        id: Uuid::new_v4(),
        name: "Catalog".into(),
        notes: String::new(),
        color: String::new(),
        remember_password: false,
        config: ConnectionConfig::Sqlite {
            path,
            read_only: false,
            create: false,
            wal: true,
            foreign_keys: true,
        },
    };
    let store = Store::open(directory.path().join("app.db")).expect("store");
    store.save_profile(&profile).expect("profile");
    store
        .record(&HistoryEntry {
            id: 0,
            connection_id: profile.id,
            database: "catalog.sqlite".into(),
            sql: "SELECT 'draft only';".into(),
            executed_at: "2026-10-28T12:00:00Z".into(),
            elapsed_ms: 1,
            affected: 1,
            error: None,
        })
        .expect("history entry");
    let services = Services::new(runtime().expect("runtime"), store.clone());
    let connection_id = profile.id;
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(900.), px(600.)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        services,
                        Preferences {
                            active_connection: Some(profile.id),
                            ..Default::default()
                        },
                        vec![profile],
                        window,
                        cx,
                    )
                })
            },
        )
        .expect("workspace")
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert!(
            workspace.read(cx).branches.is_empty(),
            "finder must work without opening the tree"
        );
        window.press("ctrl-p", cx);
    })
    .expect("open finder");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Open main.products").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| window.input("missing", cx))
        .expect("search no results");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Open main.products").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.press("enter", cx);
        assert!(workspace.read(cx).documents.is_empty());
        window.press("ctrl-a", cx);
        window.input("MAIN.PROD", cx);
    })
    .expect("search schema and name");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Open main.products").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .expect("open search result by keyboard");
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        window.try_find_by_label("Open main.products").is_none()
            && workspace
                .read(cx)
                .documents
                .first()
                .is_some_and(|doc| !doc.read(cx).busy())
    })
    .await;
    cx.update_window(window, |_, window, cx| window.press("ctrl-p", cx))
        .expect("find an existing table tab");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Open main.products").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Open main.products", cx);
    })
    .expect("open existing table from finder");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Open main.products").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            workspace.read(cx).documents.len(),
            1,
            "reuse the existing table tab"
        );
        assert_eq!(
            workspace.read(cx).documents[0]
                .read(cx)
                .table_ref()
                .expect("table")
                .name,
            "products"
        );
        window.press("ctrl-h", cx);
        assert!(!workspace.read(cx).history_docked(window));
        assert!(window.find("close-history").visible());
        assert_eq!(window.find("history-search").focused(), Some(true));
    })
    .expect("compact history");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window
            .try_find_by_label("Open query from 2026-10-28T12:00:00Z: SELECT 'draft only';")
            .is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        click_label(
            window,
            "Open query from 2026-10-28T12:00:00Z: SELECT 'draft only';",
            cx,
        );
    })
    .expect("replay history into editor");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).documents.len() == 2 && !workspace.read(cx).documents[1].read(cx).busy()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert!(
            workspace.read(cx).history.is_none(),
            "replaying a query must reveal its editor"
        );
        assert_eq!(
            workspace.read(cx).documents[1].read(cx).draft(cx).sql,
            "SELECT 'draft only';"
        );
        assert!(window.find("run-query").visible());
    })
    .expect("replayed query is visible");
    cx.update_window(window, |_, window, cx| window.press("ctrl-p", cx))
        .expect("find a table from the editor");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Open main.products").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| window.press("escape", cx))
        .expect("cancel the finder");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Open main.products").is_none()
    })
    .await;
    cx.update_window(window, |_, window, _| {
        assert_eq!(window.find_by_label("SQL editor").focused(), Some(true));
    })
    .expect("restore editor focus");
    assert_eq!(
        store.history(connection_id, "", 0).expect("history").len(),
        1,
        "replaying must not execute SQL"
    );
}

#[gpui_kit::test]
async fn explorer_switching_keyboard_navigation_and_restart(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().expect("temporary directory");
    let store = Store::open(directory.path().join("app.db")).expect("application store");
    let profiles = ["first", "second"].map(|name| {
        let path = directory.path().join(format!("{name}.db"));
        rusqlite::Connection::open(&path).expect("fixture database").execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY, label TEXT); INSERT INTO items VALUES(1,'fixture');").expect("fixture table");
        let profile = ConnectionProfile { ssh: None,
environment: None, policy: ExecutionPolicy::Normal, id: Uuid::new_v4(), name: name.into(), notes: String::new(), color: String::new(), remember_password: false,
            config: ConnectionConfig::Sqlite { path, read_only: false, create: false, wal: true, foreign_keys: true } };
        store.save_profile(&profile).expect("save profile");
        profile
    });
    let services = Services::new(runtime().expect("runtime"), store.clone());
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        services.clone(),
                        Preferences::default(),
                        profiles.to_vec(),
                        window,
                        cx,
                    )
                })
            },
        )
        .expect("workspace window")
    });
    cx.update_window(window, |_, window, cx| {
        window.click("connection-picker", cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .expect("choose first connection");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).selected == Some(profiles[0].id)
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("connection-picker", cx);
        window.input("SECOND.DB", cx);
    })
    .expect("search connections by file path");
    let second_option = format!(
        "second · SQLite · {}",
        directory.path().join("second.db").display()
    );
    let first_option = format!(
        "first · SQLite · {}",
        directory.path().join("first.db").display()
    );
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label(&second_option).is_some()
            && window.try_find_by_label(&first_option).is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert!(window.try_find("connection-menu").is_none());
        window.press("enter", cx);
    })
    .expect("inactive connections have no actions; switch to a filtered connection by keyboard");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).selected == Some(profiles[1].id)
            && workspace.read(cx).connection_state == ConnectionState::Ready
    })
    .await;
    let generation = cx.update(|cx| workspace.read(cx).generation);
    cx.update_window(window, |_, window, cx| {
        assert_eq!(window.find("connection-picker").focused(), Some(true));
        window.press("space", cx);
        assert!(window.find("picker-new-connection").visible());
        window.input("no-such-connection", cx);
    })
    .expect("open switcher by keyboard and search without a match");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label(&second_option).is_none()
            && window.try_find("picker-new-connection").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.press("enter", cx);
        assert_eq!(workspace.read(cx).generation, generation);
        window.press("escape", cx);
    })
    .expect("empty search cannot change the connection; Escape restores focus");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("picker-new-connection").is_none()
            && window.find("connection-picker").focused() == Some(true)
    })
    .await;
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .expect("reopen connection picker by keyboard");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label(&first_option).is_some()
            && window.try_find_by_label(&second_option).is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        click_label(window, &second_option, cx);
    })
    .expect("reopening resets search; choosing the active connection does not reconnect");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("picker-new-connection").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(workspace.read(cx).generation, generation);
        window.click("connection-picker", cx);
        window.press("tab", cx);
        assert_eq!(window.find("connection-menu").focused(), Some(true));
        window.press("enter", cx);
        assert!(window.find_by_label("Edit connection…").visible());
        window.press("escape", cx);
    })
    .expect("open and dismiss the active connection's options by keyboard");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Edit connection…").is_none()
            && window.try_find("picker-new-connection").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("connection-menu", cx);
    })
    .expect("reopen connection options");
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Refresh connection", cx);
    })
    .expect("refresh from the active connection's options");
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        workspace.read(cx).generation != generation
            && workspace.read(cx).connection_state == ConnectionState::Ready
            && window.try_find("picker-new-connection").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(workspace.read(cx).selected, Some(profiles[1].id));
        assert_eq!(workspace.read(cx).databases, vec!["second.db"]);
        let tree = workspace.read(cx).tree.clone();
        tree.update(cx, |tree, cx| {
            tree.set_selected_index(Some(0), cx);
            tree.focus(window, cx);
        });
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .expect("expand database with keyboard");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).branches.contains_key("second.db")
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
    })
    .expect("expand schema");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .nodes
            .values()
            .any(|node| matches!(node, super::Node::Table { .. }))
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
    })
    .expect("open table");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .documents
            .first()
            .is_some_and(|doc| !doc.read(cx).busy())
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Indexes", cx);
        window.click("empty-metadata-query", cx);
    })
    .expect("create a query from empty indexes");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).documents.len() == 2
            && workspace
                .read(cx)
                .documents
                .iter()
                .all(|doc| !doc.read(cx).busy())
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        let first = workspace.read(cx).documents[1].read(cx).title();
        window.press("ctrl-n", cx);
        window.input("SELECT 'retained draft';", cx);
        let second = workspace.read(cx).documents[2].read(cx).title();
        assert_ne!(
            first, second,
            "queries on the same database must have distinct titles"
        );
    })
    .expect("open another query with a distinct title");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .documents
            .iter()
            .all(|doc| !doc.read(cx).busy())
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let menu_size = window.find("document-tabs-menu").bounds().size;
        assert_eq!(
            menu_size.height,
            window.find("new-query").bounds().size.height
        );
        assert_eq!(menu_size.width, menu_size.height);
        assert_eq!(workspace.read(cx).active, 2);
        window.click("document-tabs-menu", cx);
        window.within("popup-menu").click(1_usize, cx);
        assert_eq!(workspace.read(cx).active, 1);
    })
    .expect("match the new-query button size and switch queries through the tab menu");
    cx.update_window(window, |_, window, cx| {
        window.click("document-tabs-menu", cx);
        window.within("popup-menu").click(2_usize, cx);
        assert_eq!(workspace.read(cx).active, 2);
        window.click(("close-tab", 1_usize), cx);
    })
    .expect("reopen the tab menu, switch back, and close the first query");
    let preferences = cx.update(|cx| workspace.read(cx).snapshot(cx));
    assert_eq!(preferences.drafts.len(), 2);
    assert!(preferences.drafts[0].table.is_some());
    assert_eq!(preferences.drafts[1].sql, "SELECT 'retained draft';");
    services
        .runtime
        .block_on(services.save_preferences(preferences))
        .expect("save session");
    let restored = store.preferences().expect("read saved session");
    let (reopened_window, reopened) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(services.clone(), restored, profiles.to_vec(), window, cx))
        })
        .expect("reopen workspace")
    });
    cx.update(|cx| {
        assert_eq!(reopened.read(cx).documents.len(), 2);
        assert_eq!(reopened.read(cx).active, 1);
        assert_eq!(reopened.read(cx).documents[1].read(cx).query_number, 2);
        assert_eq!(
            reopened.read(cx).snapshot(cx).drafts[1].sql,
            "SELECT 'retained draft';"
        );
    });
    cx.wait_for(reopened_window, Duration::from_secs(5), |_, cx| {
        reopened.read(cx).connection_state == ConnectionState::Ready
            && reopened
                .read(cx)
                .documents
                .iter()
                .all(|doc| !doc.read(cx).busy())
    })
    .await;
    cx.update_window(reopened_window, |_, window, cx| {
        window.press("ctrl-w", cx);
        click_label(window, "Indexes", cx);
        window.click("new-query", cx);
    })
    .expect("open a query from an empty metadata section");
    cx.wait_for(reopened_window, Duration::from_secs(5), |_, cx| {
        reopened
            .read(cx)
            .documents
            .iter()
            .all(|doc| !doc.read(cx).busy())
    })
    .await;
    cx.update_window(reopened_window, |_, window, cx| {
        window.press("ctrl-w", cx);
        window.press("ctrl-w", cx);
        assert!(reopened.read(cx).documents.is_empty());
        window.click("browse-databases", cx);
        assert!(reopened.read(cx).expanded.contains("db:second.db"));
        assert_eq!(reopened.read(cx).tree.read(cx).selected_index(), Some(0));
        window.click("new-query", cx);
        window.input("SELECT 'fresh start';", cx);
        assert_eq!(
            reopened.read(cx).snapshot(cx).drafts[0].sql,
            "SELECT 'fresh start';"
        );
    })
    .expect("create a focused query after closing the last document");
}

#[gpui_kit::test]
async fn connection_create_edit_and_remove_keeps_sqlite_file(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("created.sqlite");
    let store = Store::open(directory.path().join("app.db")).expect("store");
    let services = Services::new(runtime().expect("runtime"), store.clone());
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(900.), px(600.)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| Workspace::new(services, Preferences::default(), vec![], window, cx))
            },
        )
        .expect("workspace")
    });
    cx.update_window(window, |_, window, cx| {
        window.click("sidebar-add-connection", cx);
        click_label(window, "SQLite", cx);
        click_label(window, "Create database", cx);
        window.click("save-connection", cx);
        assert!(workspace.read(cx).profiles.is_empty());
        assert!(!path.exists());
        window.input("Local database", cx);
        window.click("New file path", cx);
        window.input(&path.to_string_lossy(), cx);
        window.click("save-connection", cx);
    })
    .expect("create through connection dialog");
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        workspace.read(cx).profiles.len() == 1
            && workspace.read(cx).connection_state == ConnectionState::Ready
            && window.try_find("save-connection").is_none()
    })
    .await;
    let database = rusqlite::Connection::open(&path).expect("created database");
    database
        .execute_batch("CREATE TABLE retained(value TEXT); INSERT INTO retained VALUES('kept');")
        .expect("fixture data");
    cx.update_window(window, |_, window, cx| {
        window.click("connection-picker", cx);
        window.click("picker-new-connection", cx);
    })
    .expect("new-connection action in the switcher opens the form");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("save-connection").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("cancel-connection", cx)
    })
    .expect("cancel the new connection");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("save-connection").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("connection-picker", cx);
        window.click("connection-menu", cx);
    })
    .expect("open the active connection's menu");
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Edit connection…", cx);
    })
    .expect("open the active connection's edit action");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("save-connection").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("Connection name", cx);
        window.press("ctrl-a", cx);
        window.input("Renamed connection", cx);
        window.click("save-connection", cx);
    })
    .expect("edit through connection menu");
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        workspace.read(cx).profiles[0].name == "Renamed connection"
            && workspace.read(cx).connection_state == ConnectionState::Ready
            && window.try_find("save-connection").is_none()
    })
    .await;
    assert_eq!(
        store.profiles().expect("saved profiles")[0].name,
        "Renamed connection"
    );
    cx.update_window(window, |_, window, cx| {
        window.click("connection-picker", cx);
        window.click("connection-menu", cx);
    })
    .expect("open the active connection's menu for removal");
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Remove connection…", cx);
    })
    .expect("open connection removal");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("ok").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("cancel", cx);
        assert_eq!(workspace.read(cx).profiles.len(), 1);
    })
    .expect("cancel connection removal");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("ok").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("connection-picker", cx);
        window.click("connection-menu", cx);
    })
    .expect("reopen the active connection's menu for removal");
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Remove connection…", cx);
    })
    .expect("open connection removal again");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("ok").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("ok", cx);
    })
    .expect("cancel then confirm connection removal");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).profiles.is_empty()
    })
    .await;
    assert!(store.profiles().expect("remaining profiles").is_empty());
    assert!(path.exists());
    assert_eq!(
        database
            .query_row("SELECT value FROM retained", [], |row| row
                .get::<_, String>(0))
            .expect("retained data"),
        "kept"
    );
}

#[gpui_kit::test]
async fn sidebar_retry_recovers_after_dismissing_the_connection_error(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("restored.sqlite");
    let profile = ConnectionProfile {
        ssh: None,
        environment: None,
        policy: ExecutionPolicy::Normal,
        id: Uuid::new_v4(),
        name: "Archive".into(),
        notes: String::new(),
        color: String::new(),
        remember_password: false,
        config: ConnectionConfig::Sqlite {
            path: path.clone(),
            read_only: false,
            create: false,
            wal: true,
            foreign_keys: true,
        },
    };
    let store = Store::open(directory.path().join("app.db")).expect("store");
    store.save_profile(&profile).expect("profile");
    let services = Services::new(runtime().expect("runtime"), store);
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Workspace::new(
                    services,
                    Preferences {
                        active_connection: Some(profile.id),
                        sidebar_width: 220.,
                        ..Default::default()
                    },
                    vec![profile],
                    window,
                    cx,
                )
            })
        })
        .expect("workspace")
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Failed
    })
    .await;
    let picker_label = format!("Switch connection: Archive · SQLite · {}", path.display());
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            window.find("connection-picker").label(),
            Some(format!("{picker_label} · Connection failed").as_str())
        );
        window.click("dismiss-error", cx);
    })
    .expect("selector announces the failure after the connection attempt");
    rusqlite::Connection::open(&path).expect("restore the missing database file");
    cx.update_window(window, |_, window, cx| {
        assert!(workspace.read(cx).error.is_empty());
        window.click("sidebar-retry-connection", cx);
        assert_eq!(
            window.find("connection-picker").label(),
            Some(format!("{picker_label} · Connecting…").as_str())
        );
    })
    .expect("retry remains available in the sidebar");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
            && workspace.read(cx).databases == ["restored.sqlite"]
    })
    .await;
    cx.update_window(window, |_, window, _| {
        assert_eq!(
            window.find("connection-picker").label(),
            Some(format!("{picker_label} · Connected").as_str())
        );
        assert!(window.try_find("sidebar-retry-connection").is_none());
    })
    .expect("selector announces recovery and the error-state retry is removed");
}

#[gpui_kit::test]
async fn missing_passwords_prompt_before_connecting_and_on_document_retry(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for previously_saved in [false, true] {
        let directory = tempfile::tempdir().expect("directory");
        let store = Store::open(directory.path().join("app.db")).expect("store");
        let profile = ConnectionProfile {
            ssh: None,
            environment: None,
            policy: ExecutionPolicy::Normal,
            id: Uuid::new_v4(),
            name: "Needs a password".into(),
            notes: String::new(),
            color: String::new(),
            remember_password: true,
            config: ConnectionConfig::Postgres {
                host: "127.0.0.1".into(),
                port: 9,
                database: "postgres".into(),
                user: "postgres".into(),
                tls: TlsMode::Disable,
                timeout_seconds: 1,
            },
        };
        if previously_saved {
            store
                .save_connection(&profile, "saved before losing the key")
                .expect("save password");
            std::fs::remove_file(directory.path().join("app.db.key")).expect("remove key");
        } else {
            store.save_profile(&profile).expect("legacy connection");
        }
        let services = Services::new(runtime().expect("runtime"), store.clone());
        let (window, workspace) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        services,
                        Preferences {
                            active_connection: Some(profile.id),
                            drafts: vec![Draft {
                                parameters: vec![],
                                saved_table_view: None,
                                table_view: None,
                                statement_timeout: StatementTimeout::Inherit,
                                relationship: None,
                                saved_query: None,
                                connection_id: profile.id,
                                database: "postgres".into(),
                                sql: "SELECT 'retained draft'".into(),
                                query_number: None,
                                table: None,
                                editor_height: 260.,
                            }],
                            ..Default::default()
                        },
                        vec![profile.clone()],
                        window,
                        cx,
                    )
                })
            })
            .expect("workspace")
        });
        cx.wait_for(window, Duration::from_secs(5), |window, cx| {
            workspace.read(cx).connection_state == ConnectionState::Disconnected
                && window
                    .try_find("save-connection")
                    .is_some_and(|button| button.disabled() != Some(true))
        })
        .await;
        cx.update_window(window, |_, window, cx| {
            assert!(workspace.read(cx).passwords.is_empty());
            assert!(workspace.read(cx).error.is_empty());
            assert!(
                workspace
                    .read(cx)
                    .documents
                    .iter()
                    .all(|doc| !doc.read(cx).busy() && doc.read(cx).session.is_none())
            );
            assert_eq!(window.find("remember").checked(), Some(true));
            window.click("cancel-connection", cx);
        })
        .expect("request re-entry without attempting an empty password");
        cx.wait_for(window, Duration::from_secs(5), |window, _| {
            window.try_find("save-connection").is_none()
        })
        .await;
        cx.update_window(window, |_, window, cx| window.click("reconnect", cx))
            .expect("retry restored query");
        cx.wait_for(window, Duration::from_secs(5), |window, cx| {
            workspace.read(cx).connection_state == ConnectionState::Disconnected
                && window
                    .try_find("save-connection")
                    .is_some_and(|button| button.disabled() != Some(true))
        })
        .await;
        cx.update_window(window, |_, _, cx| {
            assert!(
                workspace
                    .read(cx)
                    .documents
                    .iter()
                    .all(|doc| !doc.read(cx).busy() && doc.read(cx).session.is_none())
            );
        })
        .expect("document retry also requests the password");
        if previously_saved {
            cx.update_window(window, |_, window, cx| {
                click_label(window, "Password", cx);
                window.input("re-entered password", cx);
                window.click("Connection name", cx);
                window.press("ctrl-a", cx);
                window.input("Recovered connection", cx);
                window.click("save-connection", cx);
            })
            .expect("try saving after losing the key");
            cx.wait_for(window, Duration::from_secs(5), |window, _| {
                window
                    .try_find("save-connection")
                    .is_some_and(|button| button.disabled() != Some(true))
            })
            .await;
            assert_eq!(
                store.profiles().expect("rolled-back profile")[0].name,
                "Needs a password"
            );
            assert!(store.load_password(profile.id).is_err());
            cx.update_window(window, |_, window, cx| {
                window.click("remember", cx);
                window.click("save-connection", cx);
            })
            .expect("recover by removing the saved password");
            cx.wait_for(window, Duration::from_secs(5), |window, cx| {
                workspace.read(cx).connection_state == ConnectionState::Failed
                    && window.try_find("save-connection").is_none()
            })
            .await;
            assert_eq!(
                store.load_password(profile.id).expect("removed credential"),
                None
            );
            assert_eq!(
                store.profiles().expect("saved profile")[0].name,
                "Recovered connection"
            );
            cx.update(|cx| {
                assert_eq!(
                    workspace
                        .read(cx)
                        .passwords
                        .get(&profile.id)
                        .map(String::as_str),
                    Some("re-entered password")
                );
            });
        }
        cx.update_window(window, |_, window, _| window.remove_window())
            .expect("close workspace");
    }
}

fn postgres_test_profile() -> (ConnectionProfile, String) {
    let config = std::env::var("QUERI_TEST_POSTGRES")
        .expect("PostgreSQL test URL")
        .parse::<tokio_postgres::Config>()
        .expect("PostgreSQL config");
    let host = match config.get_hosts().first() {
        Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
        _ => panic!("test requires a TCP PostgreSQL host"),
    };
    let password =
        String::from_utf8(config.get_password().unwrap_or_default().to_vec()).expect("password");
    (
        ConnectionProfile {
            ssh: None,
            environment: None,
            policy: ExecutionPolicy::Normal,
            id: Uuid::new_v4(),
            name: "Test PostgreSQL".into(),
            notes: String::new(),
            color: String::new(),
            remember_password: false,
            config: ConnectionConfig::Postgres {
                host,
                port: config.get_ports().first().copied().unwrap_or(5432),
                database: config.get_dbname().unwrap_or("postgres").into(),
                user: config.get_user().unwrap_or("postgres").into(),
                tls: TlsMode::Disable,
                timeout_seconds: 5,
            },
        },
        password,
    )
}

#[gpui_kit::test]
#[ignore = "requires QUERI_TEST_POSTGRES"]
async fn postgres_remembered_password_reconnects_after_restart_and_remember_off(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (mut profile, password) = postgres_test_profile();
    profile.remember_password = true;
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("app.db");
    let store = Store::open(&path).expect("store");
    store
        .save_profile(&profile)
        .expect("legacy connection without a local password");
    let services = Services::new(runtime().expect("runtime"), store.clone());
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Workspace::new(
                    services.clone(),
                    Preferences {
                        active_connection: Some(profile.id),
                        ..Default::default()
                    },
                    vec![profile.clone()],
                    window,
                    cx,
                )
            })
        })
        .expect("workspace")
    });
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window
            .try_find("save-connection")
            .is_some_and(|button| button.disabled() != Some(true))
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Password", cx);
        window.input(&password, cx);
        window.click("test-connection", cx);
    })
    .expect("test the re-entered password");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window
            .try_find("test-connection")
            .is_some_and(|button| button.disabled() != Some(true))
    })
    .await;
    assert_eq!(
        store.load_password(profile.id).expect("unsaved password"),
        None
    );
    assert!(!directory.path().join("app.db.key").exists());
    cx.update_window(window, |_, window, cx| window.click("save-connection", cx))
        .expect("save and connect");
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
            && window.try_find("save-connection").is_none()
    })
    .await;
    assert_eq!(
        store.load_password(profile.id).expect("saved password"),
        Some(password.clone())
    );
    let preferences = cx.update(|cx| workspace.read(cx).snapshot(cx));
    store
        .save_preferences(&preferences)
        .expect("save workspace");
    cx.update_window(window, |_, window, _| window.remove_window())
        .expect("close application window");
    drop(workspace);
    drop(services);
    drop(store);

    let store = Store::open(&path).expect("reopen store");
    let services = Services::new(runtime().expect("fresh runtime"), store.clone());
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Workspace::new(
                    services.clone(),
                    store.preferences().expect("preferences"),
                    store.profiles().expect("profiles"),
                    window,
                    cx,
                )
            })
        })
        .expect("restart workspace")
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            workspace.read(cx).passwords.get(&profile.id),
            Some(&password)
        );
        assert!(window.try_find("save-connection").is_none());
        std::fs::remove_file(directory.path().join("app.db.key"))
            .expect("lose key while session is cached");
        workspace.update(cx, |workspace, cx| {
            workspace.connection_form(Some(profile.clone()), window, cx)
        });
        window.click("remember", cx);
        window.click("save-connection", cx);
    })
    .expect("turn off remembering while retaining the cached password");
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
            && window.try_find("save-connection").is_none()
    })
    .await;
    assert_eq!(
        store
            .load_password(profile.id)
            .expect("password removed without a key"),
        None
    );
    assert!(!store.profiles().expect("profiles")[0].remember_password);
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            workspace.read(cx).passwords.get(&profile.id),
            Some(&password)
        );
        workspace.update(cx, |workspace, cx| {
            workspace.activate(profile.id, window, cx)
        });
    })
    .expect("reconnect using only the session password");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
    })
    .await;
    cx.update_window(window, |_, window, _| window.remove_window())
        .expect("close session-only workspace");
    drop(workspace);
    drop(services);

    let services = Services::new(runtime().expect("fresh runtime"), store.clone());
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Workspace::new(
                    services,
                    store.preferences().expect("preferences"),
                    store.profiles().expect("profiles"),
                    window,
                    cx,
                )
            })
        })
        .expect("restart session-only workspace")
    });
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("save-connection").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert!(workspace.read(cx).passwords.is_empty());
        assert_eq!(window.find("remember").checked(), Some(false));
        window.remove_window();
    })
    .expect("session-only password does not survive restart");
}

#[gpui_kit::test]
#[ignore = "requires QUERI_TEST_POSTGRES with CREATEDB"]
async fn postgres_safeguards_contextual_database_lifecycle_retains_drafts(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (mut profile, password) = postgres_test_profile();
    profile.policy = ExecutionPolicy::ConfirmExecutions;
    profile.environment = Some(Environment::Production);
    let directory = tempfile::tempdir().expect("directory");
    let store = Store::open(directory.path().join("app.db")).expect("store");
    store.save_profile(&profile).expect("save profile");
    let services = Services::new(runtime().expect("runtime"), store);
    let observer = services
        .runtime
        .block_on(crate::database::Session::connect(
            profile.clone(),
            password.clone(),
        ))
        .expect("observer session");
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(
                    services.clone(),
                    Preferences::default(),
                    vec![profile.clone()],
                    window,
                    cx,
                );
                workspace.passwords.insert(profile.id, password);
                workspace.activate(profile.id, window, cx);
                workspace
            })
        })
        .expect("workspace")
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
    })
    .await;
    let name = format!("queri_ui_{}", Uuid::new_v4().simple());
    let renamed = format!("{name}_new");
    let cloned = format!("{name}_copy");
    cx.update_window(window, |_, window, cx| {
        window.click("create-database", cx);
        window.click("Database name", cx);
        window.input(&name, cx);
        window.click("cancel-admin", cx);
        assert!(!workspace.read(cx).databases.contains(&name));
        window.click("create-database", cx);
        window.click("Database name", cx);
        window.input(&name, cx);
        window.click("database-operation", cx);
    })
    .expect("create database");
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        workspace.read(cx).databases.contains(&name)
            && workspace.read(cx).loading.is_empty()
            && window.try_find("database-operation").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("connection-picker", cx);
        window.click("connection-menu", cx);
    })
    .expect("open the PostgreSQL connection's menu");
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Edit connection…", cx);
    })
    .expect("edit the PostgreSQL connection from its row menu");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("save-connection").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("Database", cx);
        window.press("ctrl-a", cx);
        window.input(&name, cx);
        window.click("save-connection", cx);
    })
    .expect("make the created database the saved default");
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        workspace.read(cx).profiles[0].database() == name
            && workspace.read(cx).connection_state == ConnectionState::Ready
            && window.try_find("save-connection").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-n", cx);
        assert_eq!(
            workspace.read(cx).documents.len(),
            1,
            "New query shortcut works after closing the connection dialog"
        );
        window.input("SELECT 'preserved draft';", cx);
    })
    .expect("open a query on created database");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .documents
            .first()
            .is_some_and(|doc| !doc.read(cx).busy())
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click(
            gpui_kit::SharedString::from(format!("database-menu:{name}")),
            cx,
        );
        click_label(window, "Rename database", cx);
        window.click("Database name", cx);
        window.press("ctrl-a", cx);
        window.input(&renamed, cx);
        window.click("database-operation", cx);
    })
    .expect("rename selected database");
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        workspace.read(cx).databases.contains(&renamed)
            && workspace.read(cx).loading.is_empty()
            && window.try_find("database-operation").is_none()
    })
    .await;
    cx.update(|cx| {
        let draft = workspace.read(cx).documents[0].read(cx).draft(cx);
        assert_eq!(draft.database, renamed);
        assert_eq!(draft.sql, "SELECT 'preserved draft';");
        assert_eq!(workspace.read(cx).profiles[0].database(), renamed);
    });
    cx.update_window(window, |_, window, cx| {
        window.click(
            gpui_kit::SharedString::from(format!("database-menu:{renamed}")),
            cx,
        );
        click_label(window, "Clone database", cx);
        window.click("Database name", cx);
        window.press("ctrl-a", cx);
        window.input(&cloned, cx);
        window.click("database-operation", cx);
    })
    .expect("clone selected database");
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        workspace.read(cx).databases.contains(&cloned)
            && workspace.read(cx).loading.is_empty()
            && window.try_find("database-operation").is_none()
    })
    .await;
    for database in [&renamed, &cloned] {
        cx.update_window(window, |_, window, cx| {
            window.click(
                gpui_kit::SharedString::from(format!("database-menu:{database}")),
                cx,
            );
            click_label(window, "Delete database", cx);
            window.click("database-operation", cx);
            assert!(workspace.read(cx).databases.contains(database));
            window.click(
                gpui_kit::SharedString::from(format!("Type {database} to confirm")),
                cx,
            );
            window.input(database, cx);
            window.click("database-operation", cx);
        })
        .expect("require database name before deletion");
        cx.wait_for(window, Duration::from_secs(5), |window, cx| {
            !workspace.read(cx).databases.contains(database)
                && window.try_find("database-operation").is_none()
        })
        .await;
    }
    let databases = services
        .runtime
        .block_on(observer.databases())
        .expect("server databases");
    assert!(
        !databases.contains(&name) && !databases.contains(&renamed) && !databases.contains(&cloned)
    );
    assert_eq!(
        services.store.profiles().expect("saved connection default")[0].database(),
        "postgres"
    );
    cx.update(|cx| {
        let doc = workspace.read(cx).documents[0].read(cx);
        assert!(doc.session.is_none());
        assert_eq!(doc.draft(cx).sql, "SELECT 'preserved draft';");
    });
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.profiles[0].policy = ExecutionPolicy::ReadOnlyByDefault;
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(window.find("create-database").disabled(), Some(true));
        workspace.update(cx, |workspace, cx| {
            for action in [
                super::AdminAction::Create,
                super::AdminAction::Rename("postgres".into()),
                super::AdminAction::Clone("postgres".into()),
                super::AdminAction::Delete("postgres".into()),
            ] {
                workspace.admin(action, window, cx);
            }
        });
        window.render_frame(cx);
        assert!(window.try_find("database-operation").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn saved_library_sql_files_and_related_tabs_keep_work_separate(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shop.sqlite");
    let database = rusqlite::Connection::open(&path).unwrap();
    database
        .execute_batch(
            "CREATE TABLE parent(id INTEGER PRIMARY KEY,name TEXT);
        CREATE TABLE child(id INTEGER PRIMARY KEY,parent_id INTEGER REFERENCES parent(id));
        INSERT INTO parent VALUES(1,'Alice'),(2,'Bob'); INSERT INTO child VALUES(10,1);",
        )
        .unwrap();
    let profile = ConnectionProfile {
        ssh: None,
        environment: None,
        policy: ExecutionPolicy::Normal,
        id: Uuid::new_v4(),
        name: "Test".into(),
        notes: String::new(),
        color: String::new(),
        remember_password: false,
        config: ConnectionConfig::Sqlite {
            path,
            read_only: false,
            create: false,
            wal: true,
            foreign_keys: true,
        },
    };
    let store = Store::open(directory.path().join("app.db")).unwrap();
    store.save_profile(&profile).unwrap();
    let query = store
        .save_query(&SavedQuery {
            parameters: vec![],
            id: Uuid::new_v4(),
            name: "Never execute on open".into(),
            sql: "CREATE TABLE never_run(id INTEGER);".into(),
            favorite: true,
            connection_id: Some(profile.id),
            database: profile.database(),
            revision: 0,
        })
        .unwrap();
    let services = Services::new(runtime().unwrap(), store.clone());
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(800.), px(700.)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        services,
                        Preferences {
                            active_connection: Some(profile.id),
                            ..Default::default()
                        },
                        vec![profile.clone()],
                        window,
                        cx,
                    )
                })
            },
        )
        .unwrap()
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("saved-queries", cx);
    })
    .unwrap();
    let open_id = gpui_kit::SharedString::from(format!("open-{}", query.id));
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find(open_id.clone()).is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| window.click(open_id.clone(), cx))
        .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .documents
            .last()
            .is_some_and(|doc| !doc.read(cx).busy())
    })
    .await;
    cx.update(|cx| {
        let workspace = workspace.read(cx);
        let doc = workspace.documents.last().unwrap().read(cx);
        assert_eq!(doc.draft(cx).sql, query.sql);
        assert_eq!(doc.saved_query.as_ref().map(|q| q.id), Some(query.id));
    });
    assert_eq!(
        database
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='never_run'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    // Let the library dialog finish its exit transition before using the toolbar under it.
    cx.executor().timer(Duration::from_millis(500)).await;
    let file = directory.path().join("import.sql");
    std::fs::write(&file, "SELECT 'imported';").unwrap();
    cx.update_window(window, |_, window, cx| {
        window.click("query-files", cx);
        click_label(window, "Open SQL file…", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![file]));
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .documents
            .last()
            .is_some_and(|doc| doc.read(cx).draft(cx).sql == "SELECT 'imported';")
    })
    .await;
    let source = TableRef {
        schema: "main".into(),
        name: "child".into(),
        kind: RelationKind::Table,
    };
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |this, cx| {
            this.open_table(profile.clone(), source, window, cx)
        })
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !workspace.read(cx).documents.last().unwrap().read(cx).busy()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("view-columns", cx);
        window.click("column-earlier-1", cx);
        window.click("column-visible-1", cx);
        window.click("table-cell-0:1", cx);
        window.click("row-actions", cx);
        click_label(window, "Open referenced row…", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find(("relationship", 0_usize)).is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click(("relationship", 0_usize), cx)
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .documents
            .last()
            .is_some_and(|doc| doc.read(cx).relationship().is_some() && !doc.read(cx).busy())
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        let state = workspace.read(cx);
        let target = state.documents.last().unwrap().read(cx);
        assert_eq!(target.table_ref().unwrap().name, "parent");
        let draft = target.draft(cx);
        let restored: Draft =
            serde_json::from_str(&serde_json::to_string(&draft).unwrap()).unwrap();
        assert_eq!(restored.relationship, target.relationship().cloned());
        window.render_frame(cx);
        window.click("open-table-sql", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .documents
            .last()
            .is_some_and(|doc| doc.read(cx).table_ref().is_none() && !doc.read(cx).busy())
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        let doc = workspace.read(cx).documents.last().unwrap().clone();
        assert_eq!(doc.read(cx).profile.id, profile.id);
        assert_eq!(doc.read(cx).profile.database(), profile.database());
        assert!(doc.read(cx).draft(cx).sql.contains("LIMIT 100 OFFSET 0"));
        assert!(doc.read(cx).parameter_seed.is_some());
        assert!(store.history(profile.id, "", 0).unwrap().is_empty());
        doc.update(cx, |doc, cx| doc.focus(window, cx));
        window.press("ctrl-enter", cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !workspace.read(cx).documents.last().unwrap().read(cx).busy()
    })
    .await;
    assert_eq!(store.history(profile.id, "", 0).unwrap().len(), 1);
    cx.update_window(window, |_, window, cx| {
        assert!(
            window
                .find("table-cell-0:1")
                .label()
                .unwrap()
                .contains("Alice")
        );
        workspace.update(cx, |this, cx| {
            this.active = this.documents.len() - 2;
            this.documents[this.active].update(cx, |doc, cx| doc.focus(window, cx));
            cx.notify();
        });
        window.render_frame(cx);
        window.click("clear-relationship", cx);
    })
    .unwrap();
    cx.update(|cx| {
        assert!(
            workspace
                .read(cx)
                .documents
                .get(workspace.read(cx).active)
                .unwrap()
                .read(cx)
                .relationship()
                .is_none()
        )
    });
}

#[gpui_kit::test]
async fn workspace_search_live_drafts_selects_text_offline_and_drops_closed_tabs(
    cx: &mut TestAppContext,
) {
    use gpui_kit::Focusable;
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let profile = ConnectionProfile {
        ssh: None,
        environment: None,
        policy: ExecutionPolicy::Normal,
        id: Uuid::new_v4(),
        name: "Offline drafts".into(),
        notes: String::new(),
        color: String::new(),
        remember_password: false,
        config: ConnectionConfig::Sqlite {
            path: directory.path().join("never-opened.db"),
            read_only: true,
            create: false,
            wal: false,
            foreign_keys: true,
        },
    };
    let drafts = (1..=2)
        .map(|number| Draft {
            parameters: vec![],
            saved_table_view: None,
            table_view: None,
            statement_timeout: StatementTimeout::custom(number * 1000).unwrap(),
            relationship: None,
            saved_query: None,
            connection_id: profile.id,
            database: profile.database(),
            sql: "persisted text".into(),
            query_number: Some(number),
            table: None,
            editor_height: default_editor_height(),
        })
        .collect();
    let services = Services::new(
        runtime().unwrap(),
        Store::open(directory.path().join("app.db")).unwrap(),
    );
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        services.clone(),
                        Preferences {
                            drafts,
                            ..Default::default()
                        },
                        vec![profile.clone()],
                        window,
                        cx,
                    )
                })
            },
        )
        .unwrap()
    });
    cx.update(|cx| {
        for (index, doc) in workspace.read(cx).documents.iter().enumerate() {
            assert_eq!(
                doc.read(cx).draft(cx).statement_timeout,
                StatementTimeout::custom((index as u32 + 1) * 1000).unwrap()
            );
        }
    });
    let target = cx.update(|cx| workspace.read(cx).documents[1].clone());
    let editor = cx.update(|cx| target.read(cx).sql_editor().unwrap());
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.active = 1;
            cx.notify();
        });
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        window.render_frame(cx);
        window.press("ctrl-a", cx);
        window.input("-- unsaved é\nSELECT 'Ä\nneedle', 'Ä\nneedle';", cx);
        window.press("ctrl-shift-s", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            workspace
                .read(cx)
                .draft_search
                .as_ref()
                .and_then(|v| v.upgrade())
                .is_some(),
            "shortcut opened draft search"
        );
        window.press("escape", cx);
        assert!(
            editor.read(cx).focus_handle(cx).is_focused(window),
            "closing search restores the originating editor"
        );
        window.press("ctrl-shift-s", cx);
        window.input("ä\nneedle", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .draft_search
            .as_ref()
            .and_then(|v| v.upgrade())
            .is_some_and(|v| v.read(cx).match_count(cx) == 2)
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, _| workspace.active = 0);
        window.press("tab", cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(workspace.read(cx).active, 1);
        assert_eq!(editor.read(cx).selected_text().to_string(), "Ä\nneedle");
        assert_eq!(
            editor.read(cx).selected_range().start,
            editor.read(cx).value().rfind("Ä\nneedle").unwrap()
        );
        assert!(editor.read(cx).focus_handle(cx).is_focused(window));
        assert!(
            target.read(cx).session.is_none(),
            "activating search must not connect"
        );
        window.press("ctrl-shift-s", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.input("needle", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .draft_search
            .as_ref()
            .and_then(|v| v.upgrade())
            .is_some_and(|v| v.read(cx).match_count(cx) == 2)
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_selected_range(0..editor.text().len(), cx);
            editor.replace("changed draft", window, cx);
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .draft_search
            .as_ref()
            .and_then(|v| v.upgrade())
            .is_some_and(|v| v.read(cx).match_count(cx) == 0)
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        editor.update(cx, |editor, cx| {
            editor.select_all(window, cx);
            editor.replace("needle", window, cx);
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .draft_search
            .as_ref()
            .and_then(|v| v.upgrade())
            .is_some_and(|v| v.read(cx).match_count(cx) == 1)
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_document(1, window, cx));
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .draft_search
            .as_ref()
            .and_then(|v| v.upgrade())
            .is_some_and(|v| v.read(cx).match_count(cx) == 0)
    })
    .await;
    assert!(!directory.path().join("never-opened.db").exists());
    assert!(
        services
            .store
            .history(profile.id, "", 0)
            .unwrap()
            .is_empty()
    );
}

#[gpui_kit::test]
async fn safeguards_transaction_close_reconnect_profile_and_quit_guards(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let profile = ConnectionProfile {
        ssh: None,
        environment: None,
        policy: ExecutionPolicy::Normal,
        id: Uuid::new_v4(),
        name: "Transactions".into(),
        notes: String::new(),
        color: String::new(),
        remember_password: false,
        config: ConnectionConfig::Sqlite {
            path: directory.path().join("db.sqlite"),
            read_only: false,
            create: true,
            wal: true,
            foreign_keys: true,
        },
    };
    let store = Store::open(directory.path().join("app.db")).unwrap();
    store.save_profile(&profile).unwrap();
    let services = Services::new(runtime().unwrap(), store);
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Workspace::new(
                    services.clone(),
                    Preferences {
                        active_connection: Some(profile.id),
                        ..Default::default()
                    },
                    vec![profile.clone()],
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("new-query", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace
            .read(cx)
            .documents
            .first()
            .is_some_and(|doc| !doc.read(cx).busy())
    })
    .await;
    let doc = cx.update(|cx| workspace.read(cx).documents[0].clone());
    cx.update_window(window, |_, window, cx| {
        window.click("begin-transaction", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    let original = cx.update(|cx| doc.read(cx).session.clone().unwrap());
    // Each loss path offers keeping the same session alive. Repeated requests
    // while the guard is open must not stack another transaction prompt.
    for action in 0..4 {
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |workspace, cx| match action {
                0 => {
                    workspace.close_document(0, window, cx);
                    workspace.close_document(0, window, cx);
                }
                1 => workspace.request_quit(window, cx),
                2 => workspace.connection_form(Some(profile.clone()), window, cx),
                _ => workspace.delete_connection(profile.id, window, cx),
            });
        })
        .unwrap();
        cx.wait_for(window, Duration::from_secs(5), |window, _| {
            window.try_find_by_label("Keep open").is_some()
        })
        .await;
        cx.update_window(window, |_, window, cx| {
            click_label(window, "Keep open", cx);
        })
        .unwrap();
        cx.wait_for(window, Duration::from_secs(5), |window, cx| {
            !doc.read(cx).busy() && window.try_find_by_label("Keep open").is_none()
        })
        .await;
        cx.update(|cx| {
            assert_eq!(workspace.read(cx).documents.len(), 1);
            assert!(std::sync::Arc::ptr_eq(
                doc.read(cx).session.as_ref().unwrap(),
                &original
            ));
            assert_eq!(doc.read(cx).transaction, TransactionStatus::Open);
        });
    }
    cx.update_window(window, |_, _, cx| {
        doc.update(cx, |_, cx| {
            cx.emit(super::super::document::ReconnectRequested)
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Roll back and close").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Roll back and close", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !doc.read(cx).busy()
            && doc
                .read(cx)
                .session
                .as_ref()
                .is_some_and(|session| !std::sync::Arc::ptr_eq(session, &original))
    })
    .await;
    assert_eq!(
        services
            .runtime
            .block_on(original.transaction_status())
            .unwrap(),
        TransactionStatus::Idle
    );
    cx.update_window(window, |_, window, cx| {
        window.click("begin-transaction", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.connection_form(Some(profile.clone()), window, cx)
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Roll back and close").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Roll back and close", cx)
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("save-connection").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("connection-environment", cx);
        click_label(window, "Production", cx);
        window.click("connection-policy", cx);
        click_label(window, "Confirm executions", cx);
        window.click("save-connection", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        window.try_find("save-connection").is_none()
            && doc
                .read(cx)
                .session
                .as_ref()
                .is_some_and(|session| session.profile.policy == ExecutionPolicy::ConfirmExecutions)
            && !doc.read(cx).busy()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            doc.read(cx).profile.environment,
            Some(Environment::Production)
        );
        assert_eq!(
            doc.read(cx).profile.policy,
            ExecutionPolicy::ConfirmExecutions
        );
        assert!(
            window
                .find("connection-picker")
                .label()
                .unwrap()
                .contains("Production")
        );
        window.click("begin-transaction", cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_document(0, window, cx));
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find_by_label("Roll back and close").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        click_label(window, "Roll back and close", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).documents.is_empty()
    })
    .await;
}

#[gpui_kit::test]
async fn staged_edits_guard_idle_tab_close_and_save_before_closing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("edits.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY,name TEXT);INSERT INTO items VALUES(1,'Original')").unwrap();
    let profile = ConnectionProfile {
        ssh: None,
        id: Uuid::new_v4(),
        name: "Edits".into(),
        notes: String::new(),
        color: String::new(),
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
    };
    let store = Store::open(directory.path().join("app.sqlite")).unwrap();
    store.save_profile(&profile).unwrap();
    let services = Services::new(runtime().unwrap(), store);
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1400.), px(950.)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        services.clone(),
                        Preferences {
                            active_connection: Some(profile.id),
                            ..Default::default()
                        },
                        vec![profile.clone()],
                        window,
                        cx,
                    )
                })
            },
        )
        .unwrap()
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_table(
                profile.clone(),
                TableRef {
                    schema: "main".into(),
                    name: "items".into(),
                    kind: RelationKind::Table,
                },
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, cx| {
        workspace.read(cx).documents.len() == 1
            && !workspace.read(cx).documents[0].read(cx).busy()
            && window.try_find("table-cell-0:2").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("table-cell-0:2", cx);
        window.press("f2", cx);
        window.press("ctrl-a", cx);
        window.input("Saved before close", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).documents[0].read(cx).has_edits(cx)
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_document(0, window, cx))
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("guard-cancel-edits").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("guard-cancel-edits", cx);
        assert_eq!(workspace.read(cx).documents.len(), 1);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_document(0, window, cx))
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("guard-save-edits").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| window.click("guard-save-edits", cx))
        .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).documents.is_empty()
    })
    .await;
    assert_eq!(
        db.query_row("SELECT name FROM items", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "Saved before close"
    );
}

#[gpui_kit::test]
async fn schema_draft_saves_restores_reviews_and_clears_after_apply(cx: &mut TestAppContext) {
    use crate::ui::schema_designer::SchemaDesignerEvent;
    use crate::ui::schema_review::SchemaReviewEvent;
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("catalog.sqlite");
    rusqlite::Connection::open(&path)
        .expect("database")
        .execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT)")
        .expect("table");
    let profile = ConnectionProfile {
        ssh: None,
        environment: None,
        policy: ExecutionPolicy::Normal,
        id: Uuid::new_v4(),
        name: "Catalog".into(),
        notes: String::new(),
        color: String::new(),
        remember_password: false,
        config: ConnectionConfig::Sqlite {
            path,
            read_only: false,
            create: false,
            wal: true,
            foreign_keys: true,
        },
    };
    let draft_key = serde_json::json!([profile.id, "catalog.sqlite", "main", "items"]).to_string();
    let store = Store::open(directory.path().join("app.db")).expect("store");
    let services = Services::new(runtime().expect("runtime"), store);
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Workspace::new(
                    services,
                    Preferences {
                        active_connection: Some(profile.id),
                        ..Default::default()
                    },
                    vec![profile],
                    window,
                    cx,
                )
            })
        })
        .expect("workspace")
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
    })
    .await;
    let open_designer = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |this, cx| {
                this.open_schema_designer(
                    "catalog.sqlite",
                    "main",
                    Some("items".into()),
                    window,
                    cx,
                )
            })
        })
        .expect("open designer");
    };
    open_designer(cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).schema_draft.is_some()
    })
    .await;
    let stored = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            workspace
                .read(cx)
                .services
                .store
                .schema_draft(&draft_key)
                .expect("read draft")
        })
    };
    assert!(stored(cx).is_none(), "opening alone does not persist");

    cx.update(|cx| {
        let designer = workspace
            .read(cx)
            .schema_draft
            .as_ref()
            .unwrap()
            .designer
            .clone();
        designer.update(cx, |_, cx| cx.emit(SchemaDesignerEvent::Changed));
    });
    assert!(stored(cx).is_some(), "edits autosave under the draft key");

    cx.update(|cx| workspace.update(cx, |this, cx| this.save_close_schema_draft(cx)));
    cx.update(|cx| assert!(workspace.read(cx).schema_draft.is_none()));
    assert!(
        stored(cx).is_some(),
        "save and close keeps the stored draft"
    );

    open_designer(cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).schema_draft.is_some()
    })
    .await;
    cx.update(|cx| {
        assert!(
            workspace
                .read(cx)
                .status
                .starts_with("Saved schema draft restored")
        )
    });

    cx.update(|cx| {
        let designer = workspace
            .read(cx)
            .schema_draft
            .as_ref()
            .unwrap()
            .designer
            .clone();
        designer.update(cx, |designer, cx| {
            let intent = designer.intent(cx);
            cx.emit(SchemaDesignerEvent::Review(Box::new(intent)))
        });
    });
    let review = cx.update(|cx| {
        workspace
            .read(cx)
            .schema_draft
            .as_ref()
            .unwrap()
            .review
            .clone()
            .expect("review opens")
    });
    review.update(cx, |_, cx| cx.emit(SchemaReviewEvent::Back));
    cx.update(|cx| {
        let draft = workspace
            .read(cx)
            .schema_draft
            .as_ref()
            .expect("back keeps the draft");
        assert!(draft.review.is_none());
    });

    cx.update(|cx| {
        let designer = workspace
            .read(cx)
            .schema_draft
            .as_ref()
            .unwrap()
            .designer
            .clone();
        designer.update(cx, |designer, cx| {
            let intent = designer.intent(cx);
            cx.emit(SchemaDesignerEvent::Review(Box::new(intent)))
        });
    });
    let review = cx.update(|cx| {
        workspace
            .read(cx)
            .schema_draft
            .as_ref()
            .unwrap()
            .review
            .clone()
            .expect("review reopens")
    });
    review.update(cx, |_, cx| cx.emit(SchemaReviewEvent::Applied));
    cx.update(|cx| assert!(workspace.read(cx).schema_draft.is_none()));
    assert!(stored(cx).is_none(), "applying removes the stored draft");
}
