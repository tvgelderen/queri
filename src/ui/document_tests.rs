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
        window.click("view-columns", cx);
        window.click("column-earlier-1", cx);
        window.click("column-visible-1", cx);
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
                Some(vec![Cell::Text("confirmed parameter".into())]),
                window,
                cx,
            )
        });
        window.click("cancel", cx);
        assert!(doc.read(cx).results.is_empty());
        doc.update(cx, |doc, cx| {
            doc.submit_sql(
                "SELECT $1::text".into(),
                Some(vec![Cell::Text("confirmed parameter".into())]),
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
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.submit_sql(
                "SELECT :private::integer".into(),
                Some(vec![Cell::Text("sensitive-invalid-number".into())]),
                window,
                cx,
            )
        });
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| assert!(doc.read(cx).error.contains("sensitive-invalid-number")));
    let history = fixture
        .services
        .store
        .history(fixture.profile.id, "", 0)
        .unwrap();
    assert!(
        !serde_json::to_string(&history)
            .unwrap()
            .contains("sensitive-invalid-number")
    );
    assert!(
        history[0]
            .error
            .as_ref()
            .unwrap()
            .contains("Parameterized query failed")
    );
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

#[gpui_kit::test]
async fn grouped_filter_draft_apply_and_sort_precedence(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("toggle-filter", cx);
        window.click("add-filter-condition", cx);
        doc.update(cx, |doc, cx| {
            if let DocumentKind::Table { filter_draft, .. } = &mut doc.kind {
                *filter_draft = Some(Box::new(FilterExpression::Any(vec![
                    TableFilter {
                        column: "id".into(),
                        predicate: FilterPredicate::Equals("1".into()),
                    }
                    .into(),
                    TableFilter {
                        column: "id".into(),
                        predicate: FilterPredicate::Equals("2".into()),
                    }
                    .into(),
                ])));
            }
            cx.notify();
        });
        assert_eq!(doc.read(cx).grid.read(cx).delegate().data.rows.len(), 1);
        let DocumentKind::Table { options, .. } = &doc.read(cx).kind else {
            panic!()
        };
        assert!(options.filter.is_none());
        window.click("apply-filter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            if let DocumentKind::Table {
                filter_draft,
                options,
                ..
            } = &mut doc.kind
            {
                assert!(options.filter.is_some());
                *filter_draft = Some(Box::new(
                    TableFilter {
                        column: "id".into(),
                        predicate: FilterPredicate::Equals("invalid".into()),
                    }
                    .into(),
                ));
                options.sort = vec![("name".into(), false), ("id".into(), true)];
            }
            cx.notify();
        });
        window.click("apply-filter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        let DocumentKind::Table { options, .. } = &doc.read(cx).kind else {
            panic!()
        };
        assert!(matches!(options.filter, Some(FilterExpression::Any(_))));
        assert!(doc.read(cx).error.contains("Invalid filter"));
        window.click("sort-earlier-1", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        let DocumentKind::Table { options, .. } = &doc.read(cx).kind else {
            panic!()
        };
        assert_eq!(
            options.sort,
            vec![("id".into(), true), ("name".into(), false)]
        );
        assert!(doc.read(cx).error.is_empty());
    });
}

#[gpui_kit::test]
async fn table_layout_maps_copy_export_search_and_restores_settings(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("view-columns", cx);
        window.click("column-earlier-1", cx);
        window.click("column-visible-1", cx);
        doc.update(cx, |doc, cx| {
            let grid = doc.grid.read(cx).delegate();
            assert_eq!(grid.display_columns, vec![1]);
            assert_eq!(
                grid.data.rows[0].len(),
                2,
                "original row retained for mutation/conflict checking"
            );
            let displayed = grid.displayed_data();
            assert_eq!(displayed.columns, ["name"]);
            assert_eq!(displayed.rows[0], [Cell::Text("Alpha".into())]);
            let mut output = Vec::new();
            crate::export::serialize(
                &displayed,
                &crate::export::Selection::all(&displayed),
                crate::export::Format::Tsv,
                true,
                "\\N",
                &mut output,
            )
            .unwrap();
            assert_eq!(String::from_utf8(output).unwrap(), "name\nAlpha\n");
            doc.grid
                .update(cx, |grid, cx| grid.set_selected_cell(0, 1, cx));
            assert_eq!(doc.cell_selection(cx).unwrap().columns, [1]);
            doc.copy_cells(crate::export::Format::Tsv, false, cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "Alpha");
            let settings = doc.draft(cx).table_view.unwrap();
            doc.restore_table_view(Some(settings.clone()));
            assert_eq!(doc.table_view_settings().unwrap(), settings);
            doc.grid_search
                .update(cx, |search, cx| search.open(window, cx));
        });
        window.input("1", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).grid_search.read(cx).status() == "0/0 cells"
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("grid-search-hidden", cx)
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx)
            .grid_search
            .read(cx)
            .status()
            .contains("hidden: id")
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("column-visible-1", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).grid_search.read(cx).status() == "1/1 cells"
    })
    .await;
}

#[gpui_kit::test]
async fn restored_missing_predicate_remains_repairable_without_unfiltered_rows(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.restore_table_view(Some(TableViewSettings {
                filter: Some(
                    TableFilter {
                        column: "removed".into(),
                        predicate: FilterPredicate::IsNull,
                    }
                    .into(),
                ),
                sort: vec![],
                columns: vec![],
                page_size: 50,
            }));
            if let DocumentKind::Table { page, .. } = &mut doc.kind {
                *page = None;
            }
            doc.refresh(window, cx);
        })
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert!(doc.read(cx).error.contains("filter column"));
        assert!(doc.read(cx).grid.read(cx).delegate().data.rows.is_empty());
        window.click("toggle-filter", cx);
        window.click("filter-node-[]-remove", cx);
        window.click("clear-filter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| assert_eq!(doc.read(cx).grid.read(cx).delegate().data.rows.len(), 1));
}

#[gpui_kit::test]
async fn named_views_explicit_save_load_conflict_and_save_as(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("view-columns", cx);
        window.click("column-earlier-1", cx);
        window.click("save-table-view-as", cx);
        window.click("saved-view-name", cx);
        window.input("By name", cx);
        window.click("confirm-save-view", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).draft(cx).saved_table_view.is_some()
    })
    .await;
    let saved = fixture
        .services
        .store
        .saved_table_views()
        .unwrap()
        .remove(0);
    assert_eq!(saved.settings.columns[0].name, "name");
    cx.update_window(window, |_, window, cx| {
        window.click("column-visible-0", cx);
    })
    .unwrap();
    assert!(
        !fixture.services.store.saved_table_views().unwrap()[0]
            .settings
            .columns[0]
            .hidden,
        "unsaved changes do not update library"
    );
    let mut changed = saved.clone();
    changed.name = "Changed elsewhere".into();
    fixture.services.store.save_table_view(&changed).unwrap();
    cx.update_window(window, |_, window, cx| {
        window.click("save-table-view", cx);
        window.click("confirm-save-view", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window
            .find("saved-view-error")
            .label()
            .is_some_and(|text| text.contains("changed"))
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("close", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("saved-view-name").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("save-table-view-as", cx);
        window.click("saved-view-name", cx);
        window.input("My changes", cx);
        window.click("confirm-save-view", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx)
            .draft(cx)
            .saved_table_view
            .as_ref()
            .is_some_and(|view| view.name == "My changes")
    })
    .await;
    assert_eq!(fixture.services.store.saved_table_views().unwrap().len(), 2);
    cx.update_window(window, |_, window, cx| {
        window.click("saved-table-views", cx)
    })
    .unwrap();
    let id = format!("load-view-{}", saved.id);
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find(id.clone()).is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| window.click(id.clone(), cx))
        .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        assert_eq!(
            doc.read(cx).draft(cx).saved_table_view.unwrap().name,
            "Changed elsewhere"
        );
        assert_eq!(
            doc.read(cx).grid.read(cx).delegate().display_columns,
            [1, 0]
        );
    });
}

#[gpui_kit::test]
async fn named_relationship_view_requires_explicit_scope_choice(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    fixture.services.runtime.block_on(fixture.session.query("ALTER TABLE items ADD COLUMN parent INTEGER REFERENCES items(id); UPDATE items SET parent=1".into())).unwrap();
    let table = TableRef {
        schema: "main".into(),
        name: "items".into(),
        kind: RelationKind::Table,
    };
    let key = fixture
        .services
        .runtime
        .block_on(fixture.session.foreign_keys(table, false))
        .unwrap()
        .remove(0);
    let scope = RelationshipScope {
        key,
        incoming: false,
        values: vec![Cell::Integer(1)],
    };
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, _| doc.set_relationship(Some(scope.clone())));
        window.click("save-table-view-as", cx);
        window.click("saved-view-name", cx);
        window.input("Reusable", cx);
        window.click("confirm-save-view", cx);
        assert!(
            window
                .find("saved-view-error")
                .label()
                .unwrap()
                .contains("Choose whether")
        );
        assert!(
            fixture
                .services
                .store
                .saved_table_views()
                .unwrap()
                .is_empty()
        );
        window.click("saved-view-scope", cx);
        crate::ui::test_support::click_label(window, "Reusable settings only", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("confirm-save-view", cx)
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).draft(cx).saved_table_view.is_some()
    })
    .await;
    assert!(
        fixture.services.store.saved_table_views().unwrap()[0]
            .relationship
            .is_none()
    );
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            doc.read(cx).relationship(),
            Some(&scope),
            "saving reusable settings does not change active scope"
        );
        window.click("save-table-view-as", cx);
        window.click("saved-view-name", cx);
        window.input("Record", cx);
        window.click("saved-view-scope", cx);
        crate::ui::test_support::click_label(
            window,
            "Include this record’s relationship scope",
            cx,
        );
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("confirm-save-view", cx)
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx)
            .draft(cx)
            .saved_table_view
            .as_ref()
            .is_some_and(|view| view.name == "Record")
    })
    .await;
    assert!(
        fixture
            .services
            .store
            .saved_table_views()
            .unwrap()
            .iter()
            .any(|view| view.relationship == Some(scope.clone()))
    );
}

#[gpui_kit::test]
async fn nested_filter_controls_and_null_cell_filter_are_drafts(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    fixture
        .services
        .runtime
        .block_on(
            fixture
                .session
                .query("ALTER TABLE items ADD COLUMN note TEXT".into()),
        )
        .unwrap();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("toggle-filter", cx);
        window.click("filter-value", cx);
        window.input("1", cx);
        window.click("add-any-group", cx);
        window.click("filter-node-[0]", cx);
        window.click("add-filter-condition", cx);
        window.click("filter-node-[]", cx);
        window.click("add-filter-condition", cx);
        let DocumentKind::Table { options, .. } = &doc.read(cx).kind else {
            panic!()
        };
        assert!(options.filter.is_none());
        window.click("apply-filter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert!(doc.read(cx).error.is_empty());
        let DocumentKind::Table { options, .. } = &doc.read(cx).kind else {
            panic!()
        };
        let Some(FilterExpression::All(children)) = &options.filter else {
            panic!()
        };
        assert!(matches!(children[0], FilterExpression::Any(_)));
        assert_eq!(children.len(), 2);
        doc.read(cx)
            .grid
            .clone()
            .update(cx, |grid, cx| grid.set_selected_cell(0, 3, cx));
        window.click("filter-from-cell", cx);
        let DocumentKind::Table { filter_draft, .. } = &doc.read(cx).kind else {
            panic!()
        };
        let Some(FilterExpression::All(children)) = filter_draft.as_deref() else {
            panic!()
        };
        assert!(matches!(
            &children[1],
            FilterExpression::Predicate(TableFilter {
                predicate: FilterPredicate::IsNull,
                ..
            })
        ));
        window.click("apply-filter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| assert_eq!(doc.read(cx).grid.read(cx).delegate().data.rows.len(), 1));
}

#[gpui_kit::test]
async fn adding_condition_preserves_previously_applied_predicate(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("toggle-filter", cx);
        window.click("filter-value", cx);
        window.input("1", cx);
        window.click("apply-filter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        window.click("filter-value", cx);
        window.press("ctrl-a", cx);
        window.input("2", cx);
        window.click("add-filter-condition", cx);
        let DocumentKind::Table { filter_draft, .. } = &doc.read(cx).kind else {
            panic!()
        };
        let Some(FilterExpression::All(children)) = filter_draft.as_deref() else {
            panic!()
        };
        assert_eq!(children.len(), 2);
        window.click("apply-filter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| assert!(doc.read(cx).grid.read(cx).delegate().data.rows.is_empty()));
}

#[gpui_kit::test]
async fn missing_layout_column_requires_explicit_repair(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.restore_table_view(Some(TableViewSettings {
                filter: None,
                sort: vec![],
                columns: vec![ViewColumn {
                    name: "removed".into(),
                    hidden: true,
                }],
                page_size: 100,
            }));
            doc.refresh(window, cx);
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert!(doc.read(cx).error.contains("Layout column ‘removed’"));
        assert!(
            doc.read(cx)
                .grid
                .read(cx)
                .delegate()
                .display_columns
                .is_empty()
        );
        assert_eq!(
            doc.read(cx).table_view_settings().unwrap().columns[0].name,
            "removed"
        );
        window.click("view-columns", cx);
        window.click("column-remove-0", cx);
        assert!(doc.read(cx).error.is_empty());
        assert_eq!(
            doc.read(cx).grid.read(cx).delegate().display_columns,
            [0, 1]
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn sql_completion_manual_typing_accept_undo_and_refresh(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    let editor = cx.update(|cx| doc.read(cx).sql_editor().unwrap());
    cx.update_window(window, |_, window, cx| {
        doc.read(cx)
            .completion
            .set_session(Some(fixture.session.clone()));
        editor.update(cx, |e, cx| {
            e.set_value("SELECT * FROM it", window, cx);
            e.set_selected_range(e.value().len()..e.value().len(), cx);
            e.focus(window, cx);
        });
        window.render_frame(cx);
        window.press("ctrl-space", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        editor.read(cx).completion_menu_state().open
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        editor.read(cx).value() == "SELECT * FROM \"items\""
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert!(editor.read(cx).focus_handle(cx).is_focused(window));
        window.press("ctrl-z", cx);
        assert_eq!(editor.read(cx).value().to_string(), "SELECT * FROM it");
        editor.update(cx, |e, cx| {
            e.set_value("SELECT * FROM items u WHERE u", window, cx);
            e.set_selected_range(e.value().len()..e.value().len(), cx)
        });
        window.input(".", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        editor.read(cx).completion_menu_state().open
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert!(
            editor
                .read(cx)
                .completion_menu_state()
                .items
                .iter()
                .any(|i| i.label == "name")
        );
        window.press("escape", cx);
        editor.update(cx, |e, cx| {
            e.set_value("SELECT * FROM items u WHERE u.na", window, cx);
            e.set_selected_range(e.value().len()..e.value().len(), cx)
        });
        window.input("m", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        editor.read(cx).completion_menu_state().open
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        editor.read(cx).value().to_string().ends_with("u.\"name\"")
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-z", cx);
        assert!(editor.read(cx).value().to_string().ends_with("u.nam"));
        window.press("f5", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).completion.status.borrow().contains("ready")
    })
    .await;
}

#[gpui_kit::test]
async fn sql_completion_discards_stale_editor_focus_and_session(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    let editor = cx.update(|cx| doc.read(cx).sql_editor().unwrap());
    cx.update_window(window, |_, window, cx| {
        doc.read(cx)
            .completion
            .set_session(Some(fixture.session.clone()));
        editor.update(cx, |e, cx| {
            e.set_value("SELECT * FROM it", window, cx);
            e.set_selected_range(e.value().len()..e.value().len(), cx);
            e.focus(window, cx);
        });
        window.render_frame(cx);
        window.press("ctrl-space", cx);
        editor.update(cx, |e, cx| e.set_value("-- no suggestions", window, cx));
    })
    .unwrap();
    cx.executor().timer(Duration::from_millis(250)).await;
    cx.update_window(window, |_, window, cx| {
        assert!(!editor.read(cx).completion_menu_state().open);
        editor.update(cx, |e, cx| e.set_value("SELECT * FROM it", window, cx));
        window.press("ctrl-space", cx);
        doc.update(cx, |d, cx| d.disconnect(cx));
    })
    .unwrap();
    cx.executor().timer(Duration::from_millis(250)).await;
    cx.update_window(window, |_, window, cx| {
        assert!(!editor.read(cx).completion_menu_state().open);
        doc.read(cx)
            .completion
            .set_session(Some(fixture.session.clone()));
        window.press("ctrl-space", cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
    })
    .unwrap();
    cx.executor().timer(Duration::from_millis(250)).await;
    cx.update(|cx| assert!(!editor.read(cx).completion_menu_state().open));
}

#[gpui_kit::test]
async fn sql_completion_quoted_replacement_unicode_and_literals(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    fixture.services.runtime.block_on(fixture.session.query("ALTER TABLE items ADD COLUMN \"Mixed name\" TEXT; ALTER TABLE items ADD COLUMN \"日本語\" TEXT;".into())).unwrap();
    let (window, doc) = fixture.open(false, cx);
    let editor = cx.update(|cx| doc.read(cx).sql_editor().unwrap());
    let original = "SELECT '😀', u.\"Mixed no\" FROM items u";
    cx.update_window(window, |_, window, cx| {
        doc.read(cx)
            .completion
            .set_session(Some(fixture.session.clone()));
        editor.update(cx, |e, cx| {
            e.set_value(original, window, cx);
            let cursor = original.find("Mixed").unwrap() + 2;
            e.set_selected_range(cursor..cursor, cx);
            e.focus(window, cx);
        });
        window.render_frame(cx);
        window.press("ctrl-space", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        editor.read(cx).completion_menu_state().open
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        editor.read(cx).value() == "SELECT '😀', u.\"Mixed name\" FROM items u"
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-z", cx);
        assert_eq!(editor.read(cx).value(), original);
    })
    .unwrap();
    for sql in [
        "SELECT 'unfinished",
        "-- comment",
        "SELECT /* comment",
        "DO $body$ code",
    ] {
        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |e, cx| {
                e.set_value(sql, window, cx);
                e.set_selected_range(sql.len()..sql.len(), cx);
            });
            window.press("ctrl-space", cx);
        })
        .unwrap();
        cx.executor().timer(Duration::from_millis(150)).await;
        cx.update(|cx| assert!(!editor.read(cx).completion_menu_state().open));
    }
}

#[gpui_kit::test]
async fn sql_completion_open_popup_preserves_run_and_search_shortcuts(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(false, cx);
    let editor = cx.update(|cx| doc.read(cx).sql_editor().unwrap());
    cx.update_window(window, |_, window, cx| {
        doc.read(cx)
            .completion
            .set_session(Some(fixture.session.clone()));
        editor.update(cx, |e, cx| {
            e.set_value("SELECT * FROM items", window, cx);
            e.set_selected_range(19..19, cx);
            e.focus(window, cx);
        });
        window.render_frame(cx);
        window.press("ctrl-space", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        editor.read(cx).completion_menu_state().open
    })
    .await;
    cx.update_window(window, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !doc.read(cx).results.is_empty()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(editor.read(cx).value(), "SELECT * FROM items");
        assert!(!editor.read(cx).completion_menu_state().open);
        window.press("ctrl-space", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        editor.read(cx).completion_menu_state().open
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.render_frame(cx);
        assert!(editor.read(cx).search_session().open);
        window.input("items", cx);
        window.press("escape", cx);
        assert!(editor.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn parameter_form_keyboard_validation_cancel_and_privacy(cx: &mut TestAppContext) {
    use crate::ui::test_support::click_label;
    cx.executor().allow_parking();
    let mut fixture = Fixture::new();
    fixture.profile.policy = ExecutionPolicy::ConfirmExecutions;
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.input("SELECT :secret, :secret, :empty, :nothing", cx);
        window.press("ctrl-enter", cx);
        window.click("cancel", cx);
        assert!(doc.read(cx).results.is_empty());
        window.press("ctrl-enter", cx);
        window.click("parameter-value-0", cx);
        window.input("private 雪'; DELETE FROM items; --", cx);
        window.click(("parameter-type", 0usize), cx);
        click_label(window, "Integer", cx);
    })
    .unwrap();
    cx.executor().run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.click("ok", cx);
        assert!(
            window
                .find("parameter-error")
                .label()
                .unwrap()
                .contains("integer")
        );
        window.click(("parameter-type", 0usize), cx);
        click_label(window, "Text", cx);
    })
    .unwrap();
    cx.executor().run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.click(("parameter-null", 2usize), cx);
        window.click("ok", cx);
        assert!(doc.read(cx).results.is_empty());
        assert_eq!(window.find("ok").label(), Some("Run SQL"));
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update(|cx| {
        assert_eq!(
            doc.read(cx).results[0].rows[0],
            vec![
                Cell::Text("private 雪'; DELETE FROM items; --".into()),
                Cell::Text("private 雪'; DELETE FROM items; --".into()),
                Cell::Text("".into()),
                Cell::Null
            ]
        );
        let draft = serde_json::to_string(&doc.read(cx).draft(cx)).unwrap();
        assert!(!draft.contains("private"));
        assert_eq!(doc.read(cx).draft(cx).parameters.len(), 3);
    });
    let history = fixture
        .services
        .store
        .history(fixture.profile.id, "", 0)
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].sql, "SELECT :secret, :secret, :empty, :nothing");
    assert!(!serde_json::to_string(&history).unwrap().contains("private"));
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| doc.focus(window, cx));
        window.press("ctrl-enter", cx);
        window.click("ok", cx);
        // Editing while a confirmation is open invalidates its captured submission.
        doc.update(cx, |doc, cx| {
            doc.sql_editor()
                .unwrap()
                .update(cx, |editor, cx| editor.set_value("SELECT 99", window, cx))
        });
        window.click("ok", cx);
        assert!(!doc.read(cx).busy());
        assert_eq!(doc.read(cx).results[0].rows[0][3], Cell::Null);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn generated_sql_uses_successful_options_and_reports_schema_drift(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let (window, doc) = fixture.open(true, cx);
    let captured = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = captured.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&doc, move |_, event: &super::OpenGeneratedSql, _| {
            *output.borrow_mut() = Some(event.0.clone())
        })
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            if let DocumentKind::Table { options, .. } = &mut doc.kind {
                options.filter = Some(
                    TableFilter {
                        column: "name".into(),
                        predicate: FilterPredicate::Equals("Alpha".into()),
                    }
                    .into(),
                );
                options.limit = 1;
            }
            doc.refresh(window, cx);
        });
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, _| {
            if let DocumentKind::Table {
                filter_draft,
                options,
                ..
            } = &mut doc.kind
            {
                *filter_draft = Some(Box::new(
                    TableFilter {
                        column: "name".into(),
                        predicate: FilterPredicate::Equals("unapplied".into()),
                    }
                    .into(),
                ));
                options.sort = vec![("missing".into(), false)];
            }
        });
        window.click("refresh-table", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| window.click("open-table-sql", cx))
        .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, _| {
        captured.borrow().is_some()
    })
    .await;
    let query = captured.borrow_mut().take().unwrap();
    assert_eq!(query.values, vec![Cell::Text("Alpha".into())]);
    assert!(query.sql.ends_with("LIMIT 1 OFFSET 0"));
    assert!(!query.sql.contains("missing"));
    assert!(
        fixture
            .services
            .store
            .history(fixture.profile.id, "", 0)
            .unwrap()
            .is_empty()
    );
    fixture
        .services
        .runtime
        .block_on(
            fixture
                .session
                .query("ALTER TABLE items RENAME COLUMN name TO renamed".into()),
        )
        .unwrap();
    cx.update_window(window, |_, window, cx| window.click("open-table-sql", cx))
        .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    assert!(captured.borrow().is_none());
    cx.update(|cx| assert!(doc.read(cx).error.contains("Repair the table view")));
}

#[gpui_kit::test]
async fn parameter_definitions_reopen_edit_save_and_stale_session(cx: &mut TestAppContext) {
    use crate::parameters::{Definition, ParameterType};
    cx.executor().allow_parking();
    let fixture = Fixture::new();
    let saved = fixture
        .services
        .store
        .save_query(&SavedQuery {
            parameters: vec![Definition {
                name: "count".into(),
                kind: ParameterType::Integer,
            }],
            id: Uuid::new_v4(),
            name: "Typed query".into(),
            sql: "SELECT :count".into(),
            favorite: false,
            connection_id: Some(fixture.profile.id),
            database: fixture.profile.database(),
            revision: 0,
        })
        .unwrap();
    let (window, doc) = fixture.open(false, cx);
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.sql_editor().unwrap().update(cx, |editor, cx| {
                editor.set_value(saved.sql.clone(), window, cx)
            });
            doc.saved_query = Some(saved.clone());
            doc.focus(window, cx);
        });
        window.press("ctrl-enter", cx);
        assert_eq!(
            window.find(("parameter-type", 0usize)).label(),
            Some("Type: Integer")
        );
        window.click("parameter-value-0", cx);
        window.input("42", cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(doc.read(cx).results[0].rows[0][0], Cell::Integer(42));
        doc.update(cx, |doc, cx| {
            doc.sql_editor().unwrap().update(cx, |editor, cx| {
                editor.set_value("SELECT :count, :new", window, cx)
            });
        });
        window.click("save-query", cx);
        window.click("confirm-save-query", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        doc.read(cx).saved_query.as_ref().unwrap().revision == 2
    })
    .await;
    let reopened = fixture
        .services
        .store
        .saved_queries("", None)
        .unwrap()
        .remove(0);
    assert_eq!(
        reopened.parameters,
        vec![
            Definition {
                name: "count".into(),
                kind: ParameterType::Integer
            },
            Definition {
                name: "new".into(),
                kind: ParameterType::Text
            }
        ]
    );
    assert_eq!(reopened.sql, "SELECT :count, :new");
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| doc.focus(window, cx));
        window.press("ctrl-enter", cx);
        window.click("parameter-value-0", cx);
        window.input("99", cx);
        doc.update(cx, |doc, _| doc.session_generation += 1);
        window.click("ok", cx);
        assert!(!doc.read(cx).busy());
        assert_eq!(doc.read(cx).results[0].rows[0][0], Cell::Integer(42));
        doc.update(cx, |doc, cx| {
            doc.sql_editor()
                .unwrap()
                .update(cx, |editor, cx| editor.set_value("SELECT :new", window, cx))
        });
        assert_eq!(
            doc.read(cx).draft(cx).parameters,
            vec![Definition {
                name: "new".into(),
                kind: ParameterType::Text
            }]
        );
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        doc.update(cx, |doc, cx| {
            doc.execute(
                "CREATE TABLE raw_partial_effect(x); SELECT 'unfinished".into(),
                window,
                cx,
            )
        });
        window.click("ok", cx);
    })
    .unwrap();
    cx.wait_for(window, Duration::from_secs(5), |_, cx| !doc.read(cx).busy())
        .await;
    assert!(
        fixture
            .services
            .runtime
            .block_on(
                fixture
                    .session
                    .query("SELECT * FROM raw_partial_effect".into())
            )
            .is_ok()
    );
}
