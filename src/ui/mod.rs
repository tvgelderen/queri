mod admin;
mod connection_form;
mod connection_picker;
mod controls;
mod document;
mod document_tabs;
mod grid;
mod history;
mod row_editor;
mod table_finder;
mod theme;
mod workspace;

#[cfg(all(test, feature = "test-support"))]
mod test_support;

use crate::storage::Store;
use anyhow::Result;
use gpui_kit::ParentElement;
use gpui_kit::component::{
    form::Field,
    input::{Input, InputState},
};
use gpui_kit::{App, Context, Entity, Window};
use std::{future::Future, sync::Arc};
pub use workspace::Workspace;

#[derive(Clone)]
pub struct Services {
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub store: Store,
    preferences: tokio::sync::mpsc::UnboundedSender<PreferenceWrite>,
}

type PreferenceWrite = (
    crate::model::Preferences,
    tokio::sync::oneshot::Sender<Result<()>>,
);

impl Services {
    pub fn new(runtime: Arc<tokio::runtime::Runtime>, store: Store) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<PreferenceWrite>();
        let writer = store.clone();
        runtime.spawn(async move {
            while let Some((preferences, reply)) = receiver.recv().await {
                let store = writer.clone();
                let result =
                    tokio::task::spawn_blocking(move || store.save_preferences(&preferences))
                        .await
                        .map_err(anyhow::Error::from)
                        .and_then(|result| result);
                let _ = reply.send(result);
            }
        });
        Self {
            runtime,
            store,
            preferences: sender,
        }
    }

    pub async fn save_preferences(&self, preferences: crate::model::Preferences) -> Result<()> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.preferences
            .send((preferences, sender))
            .map_err(|_| anyhow::anyhow!("Workspace storage worker stopped"))?;
        receiver.await?
    }
}

/// Tokio owns database I/O; only this completion runs on GPUI's UI thread.
pub fn task<V: 'static, T: Send + 'static>(
    services: &Services,
    window: &mut Window,
    cx: &mut Context<V>,
    future: impl Future<Output = Result<T>> + Send + 'static,
    complete: impl FnOnce(&mut V, Result<T>, &mut Window, &mut Context<V>) + 'static,
) {
    let job = services.runtime.spawn(future);
    cx.spawn_in(window, async move |view, cx| {
        let result = job
            .await
            .map_err(anyhow::Error::from)
            .and_then(|value| value);
        let _ = view.update_in(cx, |view, window, cx| complete(view, result, window, cx));
    })
    .detach();
}

pub fn field(label: impl Into<gpui_kit::SharedString>, input: &Entity<InputState>) -> Field {
    let label = label.into();
    Field::new()
        .label(label.clone())
        .child(Input::new(input).id(label.clone()).aria_label(label))
}

pub fn input(
    value: impl Into<gpui_kit::SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    use gpui_kit::AppContext;
    let value = value.into();
    cx.new(|cx| InputState::new(window, cx).default_value(value))
}
