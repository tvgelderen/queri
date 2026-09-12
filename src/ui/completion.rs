use crate::{
    completion::{Catalog, analysis::Context},
    database::Session,
};
use gpui_kit::{
    App, Task, Window,
    component::input::{CompletionProvider, Rope},
};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Position, TextEdit,
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct SqlCompletion {
    session: RefCell<Option<Arc<Session>>>,
    runtime: Arc<tokio::runtime::Runtime>,
    busy: RefCell<Arc<AtomicBool>>,
    generation: Rc<std::cell::Cell<u64>>,
    pub status: Rc<RefCell<String>>,
}
impl SqlCompletion {
    pub fn new(runtime: Arc<tokio::runtime::Runtime>) -> Self {
        Self {
            session: RefCell::new(None),
            runtime,
            busy: RefCell::new(Arc::new(AtomicBool::new(false))),
            generation: Rc::new(std::cell::Cell::new(0)),
            status: Rc::new(RefCell::new("Connect for database completions".into())),
        }
    }
    pub fn set_session(&self, session: Option<Arc<Session>>) {
        *self.session.borrow_mut() = session;
        *self.busy.borrow_mut() = Arc::new(AtomicBool::new(false));
        self.generation.set(self.generation.get() + 1);
        *self.status.borrow_mut() = "Ctrl+Space for completions; F5 refreshes metadata".into();
    }
}
struct Busy(Arc<AtomicBool>);
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn position(sql: &str, offset: usize) -> Position {
    let before = &sql[..offset];
    Position::new(
        before.bytes().filter(|b| *b == b'\n').count() as u32,
        before
            .rsplit('\n')
            .next()
            .unwrap_or("")
            // gpui-base 0.8.1 uses Unicode scalar columns for completion edits.
            .chars()
            .count() as u32,
    )
}

impl CompletionProvider for SqlCompletion {
    fn is_completion_trigger(&self, _: usize, text: &str, _: &mut App) -> bool {
        text.chars()
            .last()
            .is_some_and(|c| c == '.' || c == '"' || c == '_' || c.is_alphanumeric())
    }
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        if text.len() > crate::completion::analysis::MAX_SQL {
            *self.status.borrow_mut() = "Completions support SQL drafts up to 256 KiB".into();
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }
        let busy = self.busy.borrow().clone();
        let executor = cx.background_executor().clone();
        let runtime = self.runtime.clone();
        let sql = text.to_string();
        let session = self.session.borrow().clone();
        let expected_epoch = session.as_ref().map(|s| s.completion_epoch());
        let check_session = session.clone();
        let generation = self.generation.clone();
        let expected = generation.get();
        let status = self.status.clone();
        cx.spawn(async move |_| {
            // The editor drops superseded requests. Debounce before spawning work so rapid
            // typing leaves at most one database job and one cancellable latest request.
            executor.timer(std::time::Duration::from_millis(60)).await;
            while busy.load(Ordering::Acquire) {
                executor.timer(std::time::Duration::from_millis(25)).await;
            }
            if generation.get() != expected {
                return Ok(CompletionResponse::Array(vec![]));
            }
            busy.store(true, Ordering::Release);
            let job = runtime.spawn(async move {
                let _busy = Busy(busy);
                let Some(context) = Context::at(&sql, offset) else {
                    return (vec![], String::new());
                };
                let context =
                    context.case_insensitive_identifiers(session.as_ref().is_some_and(|s| {
                        matches!(
                            s.profile.config,
                            crate::model::ConnectionConfig::Sqlite { .. }
                        )
                    }));
                let catalog = if let Some(session) = session {
                    let catalog = session.completion_catalog(vec![]).await;
                    let wanted = context.wanted(&catalog);
                    if wanted.is_empty() {
                        catalog
                    } else {
                        session.completion_catalog(wanted).await
                    }
                } else {
                    Catalog::default()
                };
                let range = lsp_types::Range::new(
                    position(&sql, context.replacement.start),
                    position(&sql, context.replacement.end),
                );
                let items = context
                    .suggestions(&catalog)
                    .into_iter()
                    .map(|s| CompletionItem {
                        label: s.label,
                        detail: Some(s.detail),
                        kind: Some(if s.keyword {
                            CompletionItemKind::KEYWORD
                        } else {
                            CompletionItemKind::FIELD
                        }),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range,
                            new_text: s.insert,
                        })),
                        ..Default::default()
                    })
                    .collect();
                (items, catalog.status)
            });
            let (items, message) = job.await?;
            if generation.get() != expected
                || check_session.as_ref().map(|s| s.completion_epoch()) != expected_epoch
            {
                return Ok(CompletionResponse::Array(vec![]));
            }
            if !message.is_empty() {
                *status.borrow_mut() = message;
            }
            Ok(CompletionResponse::Array(items))
        })
    }
}
