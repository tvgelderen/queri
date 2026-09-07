use super::History;
use crate::{
    database::runtime,
    model::*,
    storage::Store,
    ui::{Services, test_support::click_label},
};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AppContext, SharedString, TestAppContext, WindowOptions};
use std::time::Duration;
use uuid::Uuid;

#[gpui_kit::test]
async fn history_delete_requires_confirmation(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().expect("directory");
    let store = Store::open(directory.path().join("app.db")).expect("store");
    let id = Uuid::new_v4();
    store
        .save_profile(&ConnectionProfile {
            id,
            name: "History fixture".into(),
            notes: String::new(),
            color: String::new(),
            remember_password: false,
            config: ConnectionConfig::Sqlite {
                path: directory.path().join("sample.db"),
                read_only: false,
                create: false,
                wal: true,
                foreign_keys: true,
            },
        })
        .expect("profile");
    store
        .record(&HistoryEntry {
            id: 0,
            connection_id: id,
            database: "sample.db".into(),
            sql: "SELECT 42".into(),
            executed_at: "2026-10-28T12:00:00Z".into(),
            elapsed_ms: 1,
            affected: 1,
            error: None,
        })
        .expect("history");
    let entry_id = store.history(id, "", 0).expect("entry")[0].id;
    let services = Services::new(runtime().expect("runtime"), store.clone());
    let (window, history) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let mut history = History::new(services, id, window, cx);
                history.load(window, cx);
                history
            })
        })
        .expect("history window")
    });
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !history.read(cx).entries.is_empty()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("history-search", cx);
        window.input("unmatched text", cx);
        window.press("enter", cx);
    })
    .expect("search history");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !history.read(cx).busy && history.read(cx).entries.is_empty()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click("empty-clear-history-search", cx);
    })
    .expect("clear an empty search using its recovery action");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        !history.read(cx).busy && history.read(cx).entries.len() == 1
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.click(
            SharedString::from(format!("history-actions-{entry_id}")),
            cx,
        );
        click_label(window, "Delete from history…", cx);
        window.click("cancel", cx);
    })
    .expect("cancel deletion");
    cx.wait_for(window, Duration::from_secs(5), |window, _| {
        window.try_find("ok").is_none()
    })
    .await;
    assert_eq!(store.history(id, "", 0).expect("retained history").len(), 1);
    cx.update_window(window, |_, window, cx| {
        window.click(
            SharedString::from(format!("history-actions-{entry_id}")),
            cx,
        );
        click_label(window, "Delete from history…", cx);
        window.click("ok", cx);
    })
    .expect("confirm deletion");
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        history.read(cx).entries.is_empty() && !history.read(cx).busy
    })
    .await;
    assert!(
        store
            .history(id, "", 0)
            .expect("deleted history")
            .is_empty()
    );
}
