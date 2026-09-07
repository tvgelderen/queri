use super::{Document, DocumentKind};
use crate::{
    database::{Session, runtime},
    model::*,
    storage::Store,
    ui::Services,
};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Focusable, TestAppContext, WindowBounds,
    WindowOptions, point, px, size,
};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

struct Fixture {
    _directory: tempfile::TempDir,
    services: Services,
    profile: ConnectionProfile,
    session: Arc<Session>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("temporary data directory");
        let store = Store::open(directory.path().join("app.db")).expect("open app database");
        let profile = ConnectionProfile {
            remember_password: false,
            id: Uuid::new_v4(),
            name: "UI fixture".into(),
            notes: String::new(),
            color: String::new(),
            config: ConnectionConfig::Sqlite {
                path: directory.path().join("fixture.db"),
                read_only: false,
                create: true,
                wal: true,
                foreign_keys: true,
            },
        };
        store.save_profile(&profile).expect("save profile");
        let services = Services::new(runtime().expect("runtime"), store);
        let session = services
            .runtime
            .block_on(Session::connect(profile.clone(), String::new()))
            .expect("connect");
        services.runtime.block_on(session.query("CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT NOT NULL DEFAULT 'New'); INSERT INTO items VALUES(1,'Alpha');".into())).expect("create fixture");
        Self {
            _directory: directory,
            services,
            profile,
            session,
        }
    }

    fn open(&self, table: bool, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Document>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            Document::init(cx);
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
                        let table = table.then(|| TableRef {
                            schema: "main".into(),
                            name: "items".into(),
                            kind: "table".into(),
                        });
                        let mut doc = Document::new(
                            self.services.clone(),
                            self.profile.clone(),
                            String::new(),
                            table,
                            String::new(),
                            window,
                            cx,
                        );
                        doc.session = Some(self.session.clone());
                        doc.focus(window, cx);
                        if table_is_document(&doc) {
                            doc.refresh(window, cx);
                        }
                        doc
                    })
                },
            )
            .expect("open document")
        })
    }
}

fn table_is_document(doc: &Document) -> bool {
    matches!(doc.kind, DocumentKind::Table { .. })
}

#[gpui_kit::test]
async fn keyboard_execution_selection_cancellation_and_history(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.input("SELECT 41 + 1 AS answer;", cx);
        assert_eq!(window.find("run-query").label(), Some("Run query"));
        window.press("ctrl-enter", cx);
    })
    .expect("run SQL by keyboard");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !doc.read(cx).results.is_empty()
    })
    .await;
    cx.update(|cx| {
        assert_eq!(doc.read(cx).results[0].rows[0][0], Cell::Integer(42));
        assert_eq!(doc.read(cx).draft(cx).sql, "SELECT 41 + 1 AS answer;");
    });
    assert_eq!(
        fixture
            .services
            .store
            .history(fixture.profile.id, "", 0)
            .expect("history")
            .len(),
        1
    );
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("SELECT 9; SELECT 10;", cx);
        window.press("ctrl-home", cx);
        for _ in 0..9 {
            window.press("shift-right", cx);
        }
        assert_eq!(window.find("run-query").label(), Some("Run selection"));
        window.press("ctrl-enter", cx);
    })
    .expect("run selection");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        assert_eq!(doc.read(cx).results.len(), 1);
        assert_eq!(doc.read(cx).results[0].rows[0][0], Cell::Integer(9));
    });
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n;",
            cx,
        );
        window.press("ctrl-enter", cx);
    })
    .expect("start long query");
    // SQLite's interrupt applies to an executing statement, not one still queued on a worker.
    std::thread::sleep(Duration::from_millis(80));
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-c", cx))
        .expect("cancel by keyboard");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| assert!(!doc.read(cx).error.is_empty()));
    assert_eq!(
        fixture
            .services
            .runtime
            .block_on(fixture.session.query("SELECT 1".into()))
            .expect("session recovered")
            .sets[0]
            .rows[0][0],
        Cell::Integer(1)
    );
}

#[gpui_kit::test]
async fn select_cells_toggle_checkboxes_across_the_entire_cell(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let grid = doc.read(cx).grid.clone();
        let cell = window.find("table-cell:0:0").bounds();
        let width = cell.size.width;
        let height = cell.size.height;
        for offset in [
            point(width / 2., height - px(1.)),
            point(width / 2., px(1.)),
            point(px(1.), height / 2.),
            point(width - px(1.), height / 2.),
            point(width - px(1.), height - px(1.)),
            point(px(16.), height / 2.),
            point(px(36.), height / 2.),
        ] {
            for checked in [true, false] {
                window.click_at("table-cell:0:0", offset, cx);
                assert_eq!(
                    grid.read(cx).delegate().checked.contains(&0),
                    checked,
                    "click at {offset:?} must toggle the row checkbox"
                );
                assert_eq!(
                    window.find(("select-row", 0_usize)).checked(),
                    Some(checked)
                );
                assert_eq!(grid.read(cx).selected_cell(), None);
                assert_eq!(grid.read(cx).selected_row(), None);
            }
        }

        window.click("table-cell-0:2", cx);
        assert_eq!(grid.read(cx).selected_cell(), Some((0, 2)));
        assert!(grid.read(cx).delegate().checked.is_empty());
        window.click_at("table-cell:0:0", point(px(1.), height - px(1.)), cx);
        assert!(grid.read(cx).delegate().checked.contains(&0));
        assert_eq!(grid.read(cx).selected_cell(), Some((0, 2)));

        window.focus_next(cx);
        window.render_frame(cx);
        assert_eq!(window.find(("select-row", 0_usize)).focused(), Some(true));
        for (key, checked) in [("space", false), ("enter", true)] {
            window.press(key, cx);
            assert_eq!(grid.read(cx).delegate().checked.contains(&0), checked);
            assert_eq!(
                window.find(("select-row", 0_usize)).checked(),
                Some(checked)
            );
            assert_eq!(grid.read(cx).selected_cell(), Some((0, 2)));
        }

        window.double_click(("select-row", 0_usize), cx);
        assert!(grid.read(cx).delegate().checked.contains(&0));
        assert!(window.try_find("save-row").is_none());
        window.click(("col-header", 0_usize), cx);
        assert_eq!(grid.read(cx).selected_cell(), Some((0, 2)));
    })
    .expect("toggle rows through the Select cells");
}

#[gpui_kit::test]
async fn row_inspector_inserts_edits_and_deletes_real_rows(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("insert", cx);
        window.click(("field-mode", 0_usize), cx);
        crate::ui::test_support::click_label(window, "Value", cx);
    })
    .expect("enter a primary key value");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click(("row-field", 0_usize), cx);
        window.input("not an integer", cx);
        window.click("save-row", cx);
        assert_eq!(window.find(("row-field", 0_usize)).focused(), Some(true));
        assert_eq!(doc.read(cx).grid.read(cx).delegate().data.rows.len(), 1);
        window.press("ctrl-a", cx);
        window.input("2", cx);
        window.click(("field-mode", 1_usize), cx);
        crate::ui::test_support::click_label(window, "Value", cx);
        window.click(("row-field", 1_usize), cx);
        window.input("Inserted in the inspector", cx);
        window.click("save-row", cx);
    })
    .expect("insert via inspector");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).grid.read(cx).delegate().data.rows.len() == 2
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        let grid = doc.read(cx).grid.clone();
        grid.update(cx, |grid, cx| grid.set_selected_row(1, cx));
        window.render_frame(cx);
        window.click("edit", cx);
        window.click(("row-field", 1_usize), cx);
        window.press("ctrl-a", cx);
        window.input("Edited in the inspector", cx);
        window.click("save-row", cx);
    })
    .expect("edit via inspector");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx)
            .grid
            .read(cx)
            .delegate()
            .data
            .rows
            .get(1)
            .is_some_and(|row| row[1].text() == "Edited in the inspector")
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        let grid = doc.read(cx).grid.clone();
        grid.update(cx, |grid, cx| grid.set_selected_row(1, cx));
        window.render_frame(cx);
        window.click("delete", cx);
        window.click("ok", cx);
    })
    .expect("confirm deletion");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).grid.read(cx).delegate().data.rows.len() == 1
    })
    .await;
    let result = fixture
        .services
        .runtime
        .block_on(fixture.session.query("SELECT name FROM items".into()))
        .expect("read surviving row");
    assert_eq!(result.sets[0].rows, vec![vec![Cell::Text("Alpha".into())]]);
}

#[gpui_kit::test]
async fn filtering_and_page_size_use_explicit_choices(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("toggle-filter", cx);
        window.click("filter-column", cx);
        window.press("up", cx);
        window.press("enter", cx);
        window.click("filter-value", cx);
        window.input("Missing", cx);
        window.press("enter", cx);
    })
    .expect("filter by selected column using Enter");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !doc.read(cx).busy() && doc.read(cx).grid.read(cx).delegate().data.rows.is_empty()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("empty-clear-filter", cx);
    })
    .expect("recover from empty filtered results");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).grid.read(cx).delegate().data.rows.len() == 1
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("filter-value", cx);
        window.input("Alpha", cx);
        window.click("apply-filter", cx);
    })
    .expect("apply a matching filter");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("toggle-filter", cx);
        window.click("clear-applied-filter", cx);
    })
    .expect("clear the filter without reopening its editor");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("page-size", cx);
        window.press("up", cx);
        window.press("enter", cx);
    })
    .expect("select page size directly");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        let DocumentKind::Table { options, .. } = &doc.read(cx).kind else {
            panic!("table document");
        };
        assert_eq!(options.limit, 1000);
        assert!(options.filter.is_none());
        assert_eq!(
            doc.read(cx).grid.read(cx).delegate().data.rows[0][1].text(),
            "Alpha"
        );
    });
}

#[gpui_kit::test]
async fn filter_operators_follow_column_types_and_distinguish_empty_and_null(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    fixture.services.runtime.block_on(fixture.session.query(
        "ALTER TABLE items ADD COLUMN note TEXT;
         ALTER TABLE items ADD COLUMN happened DATE;
         UPDATE items SET note='first',happened='2026-01-02';
         INSERT INTO items VALUES(2,'Alpine',NULL,'2026-01-10'),(10,'Beta','memo','2026-02-01'),(11,'',NULL,NULL);".into()
    )).expect("create filter fixtures");
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("toggle-filter", cx);
        assert_eq!(
            window.find("filter-operator").label(),
            Some("Filter operator: equals")
        );
        window.click("filter-operator", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(
            window.find("filter-operator").label(),
            Some("Filter operator: greater than")
        );
        window.click("filter-value", cx);
        window.input("2", cx);
        window.press("enter", cx);
    })
    .expect("choose numeric greater-than by keyboard");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        let document = doc.read(cx);
        let rows = &document.grid.read(cx).delegate().data.rows;
        assert_eq!(
            rows.iter().map(|row| row[0].text()).collect::<Vec<_>>(),
            ["10", "11"]
        );
    });
    cx.update_window(window, |_, window, cx| {
        window.click("filter-column", cx);
        window
            .within("dropdown-menu:Name(\"filter-column\")")
            .within("popup-menu")
            .click(1_usize, cx);
        assert_eq!(
            window.find("filter-operator").label(),
            Some("Filter operator: contains")
        );
        window.click("filter-operator", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(
            window.find("filter-operator").label(),
            Some("Filter operator: equals")
        );
        window.click("filter-value", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.press("enter", cx);
    })
    .expect("switch to text and match an empty string with equals");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        let document = doc.read(cx);
        let rows = &document.grid.read(cx).delegate().data.rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].text(), "11");
    });
    cx.update_window(window, |_, window, cx| {
        window.click("filter-column", cx);
        window
            .within("dropdown-menu:Name(\"filter-column\")")
            .within("popup-menu")
            .click(2_usize, cx);
        window.click("filter-operator", cx);
        window
            .within("dropdown-menu:Name(\"filter-operator\")")
            .within("popup-menu")
            .click(6_usize, cx);
        assert!(window.try_find("filter-value").is_none());
        if let DocumentKind::Table { filter, .. } = &doc.read(cx).kind {
            assert!(
                !filter.read(cx).focus_handle(cx).is_focused(window),
                "NULL selection must not leave focus on its hidden input"
            );
        }
        assert_eq!(window.find("apply-filter").focused(), Some(true));
        window.press("enter", cx);
    })
    .expect("apply a NULL filter without a value input");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        let document = doc.read(cx);
        let rows = &document.grid.read(cx).delegate().data.rows;
        assert_eq!(
            rows.iter().map(|row| row[0].text()).collect::<Vec<_>>(),
            ["2", "11"]
        );
    });
    cx.update_window(window, |_, window, cx| {
        window.click("toggle-filter", cx);
        window.click("clear-applied-filter", cx);
    })
    .expect("clear a NULL filter from its collapsed summary");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("toggle-filter", cx);
        assert!(window.try_find("filter-value").is_none());
        window.click("filter-column", cx);
        window
            .within("dropdown-menu:Name(\"filter-column\")")
            .within("popup-menu")
            .click(3_usize, cx);
        window.click("filter-operator", cx);
        window
            .within("dropdown-menu:Name(\"filter-operator\")")
            .within("popup-menu")
            .click(4_usize, cx);
        assert_eq!(
            window.find("filter-operator").label(),
            Some("Filter operator: before")
        );
        window.click("filter-value", cx);
        window.input("invalid date", cx);
        window.press("enter", cx);
    })
    .expect("apply an invalid date filter");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        assert!(doc.read(cx).error.contains("Enter a date"));
        assert_eq!(doc.read(cx).grid.read(cx).delegate().data.rows.len(), 4);
    });
    cx.update_window(window, |_, window, cx| {
        window.click("filter-value", cx);
        window.press("ctrl-a", cx);
        window.input("2026-01-15", cx);
        window.press("enter", cx);
    })
    .expect("correct the invalid date without losing the filter editor");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        let document = doc.read(cx);
        assert!(document.error.is_empty());
        let rows = &document.grid.read(cx).delegate().data.rows;
        assert_eq!(
            rows.iter().map(|row| row[0].text()).collect::<Vec<_>>(),
            ["1", "2"]
        );
        let DocumentKind::Table { options, .. } = &document.kind else {
            panic!("table document")
        };
        assert_eq!(options.offset, 0);
    });
}

#[gpui_kit::test]
async fn empty_table_actions_respect_read_only_connections(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mut fixture = Fixture::new();
    let writer = fixture.session.clone();
    fixture
        .services
        .runtime
        .block_on(writer.query("DELETE FROM items".into()))
        .expect("empty table");
    if let ConnectionConfig::Sqlite {
        read_only, create, ..
    } = &mut fixture.profile.config
    {
        *read_only = true;
        *create = false;
    }
    fixture.session = fixture
        .services
        .runtime
        .block_on(Session::connect(fixture.profile.clone(), String::new()))
        .expect("read-only session");
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert!(window.try_find("empty-insert-row").is_none());
        window.click("insert", cx);
        assert!(
            window.try_find("save-row").is_none(),
            "read-only tables cannot open the row editor"
        );
    })
    .expect("read-only empty actions");
    fixture
        .services
        .runtime
        .block_on(writer.query("INSERT INTO items(name) VALUES('Added elsewhere')".into()))
        .expect("external change");
    cx.update_window(window, |_, window, cx| {
        window.click("empty-refresh-table", cx)
    })
    .expect("refresh the empty read-only table");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).grid.read(cx).delegate().data.rows.len() == 1
    })
    .await;
    cx.update(|cx| {
        assert_eq!(
            doc.read(cx).grid.read(cx).delegate().data.rows[0][1].text(),
            "Added elsewhere"
        )
    });
}
