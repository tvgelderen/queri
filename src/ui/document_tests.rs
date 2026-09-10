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
            environment: None,
            policy: ExecutionPolicy::Normal,
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
                            kind: RelationKind::Table,
                        });
                        let mut doc = Document::new(
                            self.services.clone(),
                            self.profile.clone(),
                            None,
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
        window.click("row-actions", cx);
        crate::ui::test_support::click_label(window, "Edit row…", cx);
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
        window.click("row-actions", cx);
        crate::ui::test_support::click_label(window, "Delete row…", cx);
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

#[gpui_kit::test]
async fn rectangular_selection_copies_and_refresh_clears_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    fixture
        .services
        .runtime
        .block_on(
            fixture
                .session
                .query("INSERT INTO items VALUES(2,'Beta');".into()),
        )
        .unwrap();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("table-cell-0:1", cx);
        let grid = doc.read(cx).grid.clone();
        assert_eq!(grid.read(cx).selected_cell(), Some((0, 1)));
        window.drag_to("table-cell-0:1", "refresh-table", cx);
        let selection_after_release = grid.read(cx).selected_range();
        window.drag_to("export-data", "table-cell-1:2", cx);
        assert_eq!(grid.read(cx).selected_range(), selection_after_release);
        window.click("table-cell-0:1", cx);
        window.press("shift-right", cx);
        window.press("shift-down", cx);
        assert_eq!(grid.read(cx).selected_range(), Some(((0, 1), (1, 2))));
        assert!(grid.read(cx).delegate().checked.is_empty());
        window.click("table-cell-0:1", cx);
        window.click_with_modifiers(
            "table-cell-1:2",
            gpui_kit::Modifiers {
                shift: true,
                ..Default::default()
            },
            cx,
        );
        assert_eq!(grid.read(cx).selected_range(), Some(((0, 1), (1, 2))));
        window.drag_to("table-cell-0:1", "table-cell-1:2", cx);
        assert_eq!(grid.read(cx).selected_range(), Some(((0, 1), (1, 2))));
        doc.update(cx, |doc, cx| {
            doc.copy_cells(crate::export::Format::Tsv, true, cx)
        });
        assert_eq!(
            cx.read_from_clipboard().and_then(|v| v.text()),
            Some("id\tname\n1\tAlpha\n2\tBeta\n".into())
        );
        window.click("copy-selection", cx);
        crate::ui::test_support::click_label(window, "Copy as JSON", cx);
        let copied = cx.read_from_clipboard().and_then(|v| v.text()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&copied).unwrap();
        assert_eq!(json["columns"], serde_json::json!(["id", "name"]));
        assert_eq!(json["rows"][1][1]["value"], "Beta");
        doc.update(cx, |doc, cx| doc.refresh(window, cx));
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| assert_eq!(doc.read(cx).grid.read(cx).selected_range(), None));
}

#[gpui_kit::test]
async fn saved_query_is_explicit_and_formatting_is_one_undo(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    let original = "select 1 as answer, 'é' as label;";
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.input(original, cx);
        window.click("save-query", cx);
        window.click("Name", cx);
        window.input("Daily check", cx);
        window.click("confirm-save-query", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).saved_query.is_some()
    })
    .await;
    assert_eq!(
        fixture.services.store.saved_queries("", None).unwrap()[0].sql,
        original
    );
    assert!(
        fixture
            .services
            .store
            .history(fixture.profile.id, "", 0)
            .unwrap()
            .is_empty()
    );
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| doc.focus(window, cx));
        window.press("ctrl-shift-f", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !doc.read(cx).formatting
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert_ne!(doc.read(cx).draft(cx).sql, original);
        assert!(doc.read(cx).draft(cx).sql.contains("'é'"));
        window.press("ctrl-z", cx);
        assert_eq!(doc.read(cx).draft(cx).sql, original);
        window.press("ctrl-y", cx);
        assert_ne!(doc.read(cx).draft(cx).sql, original);
    })
    .unwrap();
    // Formatting changes only the draft until the user explicitly saves again.
    assert_eq!(
        fixture.services.store.saved_queries("", None).unwrap()[0].sql,
        original
    );
}

#[gpui_kit::test]
async fn cell_inspector_copies_original_json_after_switching_views(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    let raw = r#"{"n":12345678901234567890,"n":1.2300}"#;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        super::super::cell_inspector::open(
            fixture.services.clone(),
            Cell::Text(raw.into()),
            "JSON value".into(),
            None,
            window,
            cx,
        );
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("copy-formatted-cell").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("copy-formatted-cell", cx);
        let formatted = cx.read_from_clipboard().and_then(|v| v.text()).unwrap();
        assert_eq!(formatted.matches("\"n\"").count(), 2);
        window.click("copy-raw-cell", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|v| v.text()),
            Some(raw.into())
        );
        window.press("escape", cx);
        doc.update(cx, |doc, cx| doc.focus(window, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn export_dialog_writes_captured_page_without_running_sql(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    let destination = fixture._directory.path().join("export.csv");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("export-data", cx);
        window.click("save-export", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| Some(destination.clone()));
    cx.wait_for(window, Duration::from_secs(5), |_, _| destination.exists())
        .await;
    assert_eq!(
        std::fs::read_to_string(destination).unwrap(),
        "id,name\n1,Alpha\n"
    );
    assert!(
        fixture
            .services
            .store
            .history(fixture.profile.id, "", 0)
            .unwrap()
            .is_empty()
    );
}

#[gpui_kit::test]
async fn format_selection_keeps_surrounding_sql(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    let original = "-- before\nselect 'é', 1;\n-- after";
    let start = original.find("select").unwrap();
    let end = original.find(";\n").unwrap() + 1;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.input(original, cx);
        doc.update(cx, |doc, cx| {
            if let DocumentKind::Sql { editor } = &doc.kind {
                editor.update(cx, |editor, cx| editor.set_selected_range(start..end, cx));
            }
            doc.format_sql(window, cx);
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !doc.read(cx).formatting
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        let value = doc.read(cx).draft(cx).sql;
        assert!(value.starts_with("-- before\n"));
        assert!(value.ends_with("\n-- after"));
        assert_ne!(value, original);
        window.press("ctrl-z", cx);
        assert_eq!(doc.read(cx).draft(cx).sql, original);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn selecting_a_cell_keeps_the_grid_stable_and_double_click_inspects_it(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let before = window.find("table-cell-0:2").bounds();
        window.click("table-cell-0:2", cx);
        window.render_frame(cx);
        assert_eq!(window.find("table-cell-0:2").bounds(), before);
        window.double_click("table-cell-0:2", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("copy-raw-cell").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("copy-raw-cell", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|v| v.text()),
            Some("Alpha".into())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn grid_search_preserves_selection_wraps_and_invalidates(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    let grid = cx.update(|cx| doc.read(cx).grid.clone());
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.results = vec![
                Arc::new(ResultSet {
                    columns: vec!["value".into(), "other".into()],
                    rows: vec![
                        vec![Cell::Text("Ä\nmatch".into()), Cell::Text("keep".into())],
                        vec![
                            Cell::Text("ä\nmatch".into()),
                            Cell::Text("selection".into()),
                        ],
                    ],
                    ..Default::default()
                }),
                Arc::new(ResultSet {
                    columns: vec!["value".into()],
                    rows: vec![vec![Cell::Text("different page".into())]],
                    ..Default::default()
                }),
            ];
            doc.update_grid(cx);
        });
        grid.update(cx, |grid, cx| {
            grid.delegate_mut().checked.insert(1);
            grid.set_selected_cell(0, 0, cx);
            grid.extend_selection(1, 1, cx);
            grid.focus_handle(cx).focus(window, cx);
        });
        window.render_frame(cx);
        window.press("ctrl-f", cx);
        window.input("ä\nmatch", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        grid.read(cx).delegate().search_cell == Some((0, 0))
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(doc.read(cx).grid_search.read(cx).status(), "1/2 cells");
        assert_eq!(grid.read(cx).selected_range(), Some(((0, 0), (1, 1))));
        assert_eq!(
            grid.read(cx)
                .delegate()
                .checked
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![1]
        );
        window.press("enter", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(grid.read(cx).delegate().search_cell, Some((1, 0)));
        window.press("enter", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(grid.read(cx).delegate().search_cell, Some((0, 0)));
        window.press("shift-enter", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(grid.read(cx).delegate().search_cell, Some((1, 0)));
        window.click("grid-search-case", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        grid.read(cx).delegate().search_cell == Some((1, 0))
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("grid-search-next", cx);
        assert_eq!(
            grid.read(cx).delegate().search_cell,
            Some((1, 0)),
            "case-sensitive search has one matching cell"
        );
        window.click("grid-search-close", cx);
        assert!(grid.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(grid.read(cx).selected_range(), Some(((0, 0), (1, 1))));
        window.press("ctrl-c", cx);
        assert!(
            cx.read_from_clipboard()
                .and_then(|v| v.text())
                .unwrap()
                .contains("selection")
        );
        window.press("ctrl-f", cx);
        window.input("missing", cx);
        window.press("ctrl-a", cx);
        window.input("ä\nmatch", cx);
        doc.update(cx, |doc, cx| {
            doc.result_index = 1;
            doc.update_grid(cx);
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).grid_search.read(cx).status() == "0/0 cells"
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(grid.read(cx).delegate().search_cell, None);
        window.press("escape", cx);
        assert!(grid.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
    assert!(
        fixture
            .services
            .store
            .history(fixture.profile.id, "", 0)
            .unwrap()
            .is_empty()
    );
}

#[gpui_kit::test]
async fn editor_search_and_format_shortcuts_remain_separate(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    let editor = cx.update(|cx| doc.read(cx).sql_editor().unwrap());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.input("select 'needle', 'needle';", cx);
        window.press("ctrl-f", cx);
        window.render_frame(cx);
        assert!(editor.read(cx).search_session().open);
        assert!(!editor.read(cx).focus_handle(cx).is_focused(window));
        window.input("needle", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(editor.read(cx).search_session().matcher.len(), 2);
        window.press("enter", cx);
        assert_eq!(
            editor
                .read(cx)
                .search_session()
                .matcher
                .current_match_index(),
            1
        );
        window.press("escape", cx);
        assert!(editor.read(cx).focus_handle(cx).is_focused(window));
        assert!(!editor.read(cx).search_session().open);
        window.press("ctrl-a", cx);
        window.press("ctrl-shift-f", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !doc.read(cx).formatting
    })
    .await;
    cx.update(|cx| {
        assert!(!editor.read(cx).search_session().open);
        assert_ne!(
            editor.read(cx).value().as_ref(),
            "select 'needle', 'needle';"
        );
    });
}

#[gpui_kit::test]
async fn inspector_search_keeps_document_search_and_sql_untouched(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt;
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    let sql_editor = cx.update(|cx| doc.read(cx).sql_editor().unwrap());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.input("select 'untouched';", cx);
        super::super::cell_inspector::open(
            fixture.services.clone(),
            Cell::Text("needle 世界 needle".into()),
            "Search cell".into(),
            None,
            window,
            cx,
        );
    })
    .unwrap();
    let inspector_editor = cx
        .update_window(window, |_, window, cx| {
            crate::ui::test_support::click_label(window, "Cell contents; Ctrl+F to search", cx);
            window
                .focused_input(cx)
                .unwrap()
                .as_editor()
                .unwrap()
                .clone()
        })
        .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        inspector_editor.read(cx).value().as_ref() == "needle 世界 needle"
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("needle", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        assert!(inspector_editor.read(cx).search_session().open);
        assert_eq!(inspector_editor.read(cx).search_session().matcher.len(), 2);
        assert!(!sql_editor.read(cx).search_session().open);
        assert_eq!(sql_editor.read(cx).value().as_ref(), "select 'untouched';");
        window.press("escape", cx);
        assert!(
            inspector_editor
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.press("escape", cx);
        assert!(sql_editor.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn transaction_controls_manual_sql_and_error_observation(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        window.click("begin-transaction", cx);
        assert!(doc.read(cx).busy());
        // A second action cannot race the pending BEGIN/status observation.
        doc.update(cx, |doc, cx| doc.transaction_command("COMMIT", window, cx));
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Open);
        assert!(doc.read(cx).transaction_started.is_some());
        doc.update(cx, |doc, cx| {
            doc.execute(
                "INSERT INTO items VALUES(2,'pending'); SELECT * FROM missing".into(),
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Open);
        assert!(!doc.read(cx).error.is_empty());
        window.click("rollback-transaction", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    assert_eq!(
        fixture
            .services
            .runtime
            .block_on(fixture.session.query("SELECT count(*) FROM items".into()))
            .unwrap()
            .sets[0]
            .rows[0][0],
        Cell::Integer(1)
    );
    cx.update_window(window, |_, window, cx| {
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Idle);
        assert!(doc.read(cx).transaction_started.is_none());
        doc.update(cx, |doc, cx| {
            doc.execute(
                "SAVEPOINT manual; INSERT INTO items VALUES(3,'saved')".into(),
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Open);
        window.click("commit-transaction", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| assert_eq!(doc.read(cx).transaction, TransactionStatus::Idle));
    assert_eq!(
        fixture
            .services
            .runtime
            .block_on(fixture.session.query("SELECT count(*) FROM items".into()))
            .unwrap()
            .sets[0]
            .rows[0][0],
        Cell::Integer(2)
    );
}

#[gpui_kit::test]
#[ignore = "requires QUERI_TEST_POSTGRES"]
async fn postgres_transaction_failed_recovery_and_unknown_commit_ui(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mut fixture = Fixture::new();
    let config = std::env::var("QUERI_TEST_POSTGRES")
        .unwrap()
        .parse::<tokio_postgres::Config>()
        .unwrap();
    fixture.profile.config = ConnectionConfig::Postgres {
        host: match &config.get_hosts()[0] {
            tokio_postgres::config::Host::Tcp(host) => host.clone(),
            _ => panic!("TCP required"),
        },
        port: config.get_ports()[0],
        database: config.get_dbname().unwrap_or("postgres").into(),
        user: config.get_user().unwrap().into(),
        tls: TlsMode::Disable,
        timeout_seconds: 5,
    };
    let password = String::from_utf8(config.get_password().unwrap_or_default().to_vec()).unwrap();
    fixture.session = fixture
        .services
        .runtime
        .block_on(Session::connect(fixture.profile.clone(), password.clone()))
        .unwrap();
    let observer = fixture
        .services
        .runtime
        .block_on(Session::connect(fixture.profile.clone(), password))
        .unwrap();
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.execute("BEGIN; SAVEPOINT recovery; SELECT 1/0".into(), window, cx)
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Failed);
        doc.update(cx, |doc, cx| doc.transaction_command("COMMIT", window, cx));
        assert!(
            !doc.read(cx).busy(),
            "Failed transaction cannot be committed by the toolbar"
        );
        doc.update(cx, |doc, cx| {
            doc.execute("ROLLBACK TO recovery".into(), window, cx)
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| assert_eq!(doc.read(cx).transaction, TransactionStatus::Open));
    let pid = fixture
        .services
        .runtime
        .block_on(fixture.session.query("SELECT pg_backend_pid()".into()))
        .unwrap()
        .sets[0]
        .rows[0][0]
        .text();
    fixture
        .services
        .runtime
        .block_on(observer.query(format!("SELECT pg_terminate_backend({pid})")))
        .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.click("commit-transaction", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Unknown);
        assert!(doc.read(cx).error.contains("may have taken effect"));
        assert!(matches!(
            doc.read(cx).query_outcome,
            super::QueryOutcome::Failed
        ));
    });
}

#[gpui_kit::test]
async fn transaction_cancellation_settles_before_controls_enable(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| doc.execute("BEGIN; WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n".into(), window, cx));
    }).unwrap();
    // Let SQLite enter the recursive statement before interrupting its worker.
    cx.executor().timer(Duration::from_millis(100)).await;
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| doc.cancel(window, cx));
        assert!(doc.read(cx).busy());
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    let observed = fixture
        .services
        .runtime
        .block_on(fixture.session.transaction_status())
        .unwrap();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(doc.read(cx).transaction, observed);
        assert!(matches!(
            doc.read(cx).query_outcome,
            super::QueryOutcome::Cancelled
        ));
        if observed == TransactionStatus::Open {
            window.click("rollback-transaction", cx);
        }
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| assert_eq!(doc.read(cx).transaction, TransactionStatus::Idle));
}

#[gpui_kit::test]
async fn transaction_completion_from_old_session_cannot_replace_reconnect_state(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.execute("BEGIN; SELECT 99".into(), window, cx);
            // Simulate session replacement before the old UI completion is delivered.
            doc.disconnect(cx);
            doc.connect(window, cx);
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    fixture
        .services
        .runtime
        .block_on(fixture.session.transaction_status())
        .unwrap();
    cx.executor().timer(Duration::from_millis(100)).await;
    cx.update(|cx| {
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Idle);
        assert!(doc.read(cx).results.is_empty());
        assert!(doc.read(cx).transaction_started.is_none());
        assert!(!Arc::ptr_eq(
            doc.read(cx).session.as_ref().unwrap(),
            &fixture.session
        ));
    });
    fixture
        .services
        .runtime
        .block_on(fixture.session.query("ROLLBACK".into()))
        .unwrap();
}

#[gpui_kit::test]
async fn statement_timeout_controls_outcome_history_and_reconnect(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("statement-timeout").label(),
            Some("Timeout: inherit")
        );
        window.click("statement-timeout", cx);
        window
            .within("dropdown-menu:Name(\"statement-timeout\")")
            .within("popup-menu")
            .click(1_usize, cx);
        assert_eq!(doc.read(cx).statement_timeout, StatementTimeout::NoTimeout);
    })
    .unwrap();
    // Allow the menu's deferred dismissal to settle before opening it again.
    cx.executor().timer(Duration::from_millis(100)).await;
    cx.update_window(window, |_, window, cx| {
        window.click("statement-timeout", cx);
        window
            .within("dropdown-menu:Name(\"statement-timeout\")")
            .within("popup-menu")
            .click(2_usize, cx);
    })
    .unwrap();
    for invalid in ["", "0", "-1", "2147483648"] {
        cx.update_window(window, |_, window, cx| {
            let input = doc.read(cx).timeout_input.clone();
            input.update(cx, |input, cx| input.set_value(invalid, window, cx));
            window.click("ok", cx);
            assert_eq!(doc.read(cx).statement_timeout, StatementTimeout::NoTimeout);
        })
        .unwrap();
        cx.wait_for(window, Duration::from_secs(2), |window, _| {
            window
                .try_find("timeout-validation")
                .is_some_and(|element| {
                    element.label()
                        == Some("Enter a whole number from 1 to 2147483647 milliseconds.")
                })
        })
        .await;
    }
    cx.update_window(window, |_, window, cx| {
        let input = doc.read(cx).timeout_input.clone();
        input.update(cx, |input, cx| input.set_value("200", window, cx));
        window.click("ok", cx);
        assert_eq!(doc.read(cx).statement_timeout, StatementTimeout::custom(200).unwrap());
        assert_eq!(doc.read(cx).draft(cx).statement_timeout, StatementTimeout::custom(200).unwrap());
        doc.update(cx, |doc, cx| {
            doc.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n".into(), window, cx);
            doc.set_timeout(StatementTimeout::NoTimeout, cx);
            assert_eq!(doc.statement_timeout, StatementTimeout::custom(200).unwrap());
        });
    }).unwrap();
    cx.wait_for(window, Duration::from_secs(10), |_, cx| {
        !doc.read(cx).busy()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert!(matches!(
            doc.read(cx).query_outcome,
            super::QueryOutcome::TimedOut
        ));
        assert_eq!(doc.read(cx).status, "Statement timed out");
        doc.update(cx, |doc, cx| {
            doc.disconnect(cx);
            doc.connect(window, cx);
        });
    })
    .unwrap();
    let history = fixture
        .services
        .store
        .history(fixture.profile.id, "", 0)
        .unwrap();
    assert!(
        history[0]
            .error
            .as_ref()
            .unwrap()
            .contains("Statement timed out")
    );
    cx.wait_for(window, Duration::from_secs(10), |_, cx| {
        !doc.read(cx).busy()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            doc.read(cx).statement_timeout,
            StatementTimeout::custom(200).unwrap()
        );
        doc.update(cx, |doc, cx| doc.execute("SELECT 42".into(), window, cx));
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(10), |_, cx| {
        !doc.read(cx).busy()
    })
    .await;
    cx.update(|cx| {
        assert!(matches!(
            doc.read(cx).query_outcome,
            super::QueryOutcome::Succeeded
        ))
    });
}

#[gpui_kit::test]
async fn statement_timeout_settings_preserve_failed_transaction_recovery(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.transaction = TransactionStatus::Failed;
            doc.set_timeout(StatementTimeout::NoTimeout, cx);
        });
        window.render_frame(cx);
        assert_eq!(doc.read(cx).statement_timeout, StatementTimeout::Inherit);
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Failed);
        assert_eq!(window.find("statement-timeout").disabled(), Some(true));
        assert_ne!(window.find("rollback-transaction").disabled(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn safeguards_confirmation_selection_transactions_and_cancel(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mut fixture = Fixture::new();
    fixture.profile.environment = Some(Environment::Production);
    fixture.profile.policy = ExecutionPolicy::ConfirmExecutions;
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("document-context")
                .label()
                .unwrap()
                .contains("Production")
        );
        window.input("INSERT INTO items VALUES(2,'confirmed');", cx);
        window.press("ctrl-enter", cx);
        window.render_frame(cx);
        let confirmation = window.find("sql-confirmation-context");
        let message = confirmation.label().unwrap();
        assert!(message.contains("UI fixture · Production"));
        assert!(message.contains("INSERT INTO items"));
        assert!(message.contains(&fixture.profile.database()));
        window.click("cancel", cx);
        assert!(!doc.read(cx).busy());
    })
    .unwrap();
    assert_eq!(
        fixture
            .services
            .runtime
            .block_on(fixture.session.query("SELECT count(*) FROM items".into()))
            .unwrap()
            .sets[0]
            .rows[0][0],
        Cell::Integer(1)
    );
    assert!(
        fixture
            .services
            .store
            .history(fixture.profile.id, "", 0)
            .unwrap()
            .is_empty()
    );
    cx.update_window(window, |_, window, cx| {
        window.click("run-query", cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    assert_eq!(
        fixture
            .services
            .runtime
            .block_on(fixture.session.query("SELECT count(*) FROM items".into()))
            .unwrap()
            .sets[0]
            .rows[0][0],
        Cell::Integer(2)
    );
    cx.update_window(window, |_, window, cx| {
        window.click("begin-transaction", cx);
        window.click("cancel", cx);
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Idle);
        window.click("begin-transaction", cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Open);
        window.click("commit-transaction", cx);
        window.click("cancel", cx);
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Open);
        window.click("rollback-transaction", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(doc.read(cx).transaction, TransactionStatus::Idle);
        assert!(window.try_find("ok").is_none());
        // Selected SQL and destructive warnings share exactly one confirmation.
        doc.update(cx, |doc, cx| {
            let DocumentKind::Sql { editor } = &doc.kind else {
                panic!()
            };
            editor.update(cx, |editor, cx| {
                editor.set_value("DELETE FROM items; SELECT 99;", window, cx);
                editor.set_selected_range(0..18, cx);
            });
            doc.run(window, cx);
        });
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert!(window.try_find("ok").is_none());
        assert_eq!(doc.read(cx).results.len(), 1);
    })
    .unwrap();
    assert_eq!(
        fixture
            .services
            .runtime
            .block_on(fixture.session.query("SELECT count(*) FROM items".into()))
            .unwrap()
            .sets[0]
            .rows[0][0],
        Cell::Integer(0)
    );
}

#[gpui_kit::test]
async fn safeguards_row_editor_confirm_cancel_and_delete(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mut fixture = Fixture::new();
    fixture.profile.policy = ExecutionPolicy::ConfirmExecutions;
    fixture.profile.environment = Some(Environment::Staging);
    fixture.session = fixture
        .services
        .runtime
        .block_on(Session::connect(fixture.profile.clone(), String::new()))
        .unwrap();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("insert", cx);
        window.click("save-row", cx);
        window.click("cancel", cx);
        assert!(window.find("save-row").visible());
    })
    .unwrap();
    assert_eq!(
        fixture
            .services
            .runtime
            .block_on(fixture.session.query("SELECT count(*) FROM items".into()))
            .unwrap()
            .sets[0]
            .rows[0][0],
        Cell::Integer(1)
    );
    cx.update_window(window, |_, window, cx| {
        window.click("save-row", cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).grid.read(cx).delegate().data.rows.len() == 2
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        doc.read(cx)
            .grid
            .clone()
            .update(cx, |grid, cx| grid.set_selected_row(1, cx));
        doc.update(cx, |doc, cx| doc.delete_rows(window, cx));
        window.click("cancel", cx);
        doc.update(cx, |doc, cx| doc.delete_rows(window, cx));
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).grid.read(cx).delegate().data.rows.len() == 1
    })
    .await;
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("ok").is_none())
    })
    .unwrap();
}

#[gpui_kit::test]
async fn safeguards_read_only_row_actions_cannot_bypass(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mut fixture = Fixture::new();
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
        .unwrap();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("insert").disabled(), Some(true));
        doc.update(cx, |doc, cx| {
            doc.edit_row(true, window, cx);
            doc.delete_rows(window, cx);
        });
        assert!(window.try_find("save-row").is_none());
        assert!(window.try_find("ok").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
#[ignore = "requires QUERI_TEST_POSTGRES"]
async fn postgres_safeguards_parameter_submission_and_read_only_ui(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mut fixture = Fixture::new();
    let config = std::env::var("QUERI_TEST_POSTGRES")
        .unwrap()
        .parse::<tokio_postgres::Config>()
        .unwrap();
    fixture.profile.config = ConnectionConfig::Postgres {
        host: match &config.get_hosts()[0] {
            tokio_postgres::config::Host::Tcp(host) => host.clone(),
            _ => panic!("TCP required"),
        },
        port: config.get_ports()[0],
        database: config.get_dbname().unwrap_or("postgres").into(),
        user: config.get_user().unwrap().into(),
        tls: TlsMode::Disable,
        timeout_seconds: 5,
    };
    fixture.profile.policy = ExecutionPolicy::ConfirmExecutions;
    let password = String::from_utf8(config.get_password().unwrap_or_default().to_vec()).unwrap();
    fixture.session = fixture
        .services
        .runtime
        .block_on(Session::connect(fixture.profile.clone(), password.clone()))
        .unwrap();
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.submit_sql(
                "SELECT $1::text".into(),
                Some(vec![Some("confirmed parameter".into())]),
                window,
                cx,
            )
        });
        window.click("cancel", cx);
        assert!(doc.read(cx).results.is_empty());
        doc.update(cx, |doc, cx| {
            doc.submit_sql(
                "SELECT $1::text".into(),
                Some(vec![Some("confirmed parameter".into())]),
                window,
                cx,
            )
        });
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        assert_eq!(
            doc.read(cx).results[0].rows[0][0],
            Cell::Text("confirmed parameter".into())
        )
    });
    fixture.profile.policy = ExecutionPolicy::ReadOnlyByDefault;
    let session = fixture
        .services
        .runtime
        .block_on(Session::connect(fixture.profile.clone(), password))
        .unwrap();
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.profile = fixture.profile.clone();
            doc.session = Some(session.clone());
            doc.execute("SET default_transaction_read_only=off".into(), window, cx);
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    assert!(session.read_write_observed());
    assert!(
        session.read_only(),
        "Generated changes remain blocked after a raw SQL override"
    );
}
