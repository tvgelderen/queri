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
        let profile = ConnectionProfile { id: Uuid::new_v4(), name: name.into(), notes: String::new(), color: String::new(), remember_password: false,
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
    cx.update_window(window, |_, window, cx| window.click("dismiss-error", cx))
        .expect("dismiss the detailed connection error");
    rusqlite::Connection::open(path).expect("restore the missing database file");
    cx.update_window(window, |_, window, cx| {
        assert!(workspace.read(cx).error.is_empty());
        window.click("sidebar-retry-connection", cx);
    })
    .expect("retry remains available in the sidebar");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        workspace.read(cx).connection_state == ConnectionState::Ready
            && workspace.read(cx).databases == ["restored.sqlite"]
    })
    .await;
}

#[gpui_kit::test]
#[ignore = "requires QUERI_TEST_POSTGRES with CREATEDB"]
async fn postgres_contextual_database_lifecycle_retains_drafts(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
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
    let profile = ConnectionProfile {
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
    };
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
}
