//! One guard for operations that discard document sessions. Documents remain locked
//! from observation through acknowledgement and rollback, including dialog dismissal.
use super::{Services, document::Document, task};
use crate::{database::Session, model::TransactionStatus};
use gpui_kit::{
    component::{WindowExt, dialog::DialogButtonProps},
    *,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub(super) fn release<V: 'static>(
    services: &Services,
    documents: Vec<Entity<Document>>,
    window: &mut Window,
    cx: &mut Context<V>,
    complete: impl FnOnce(&mut V, &mut Window, &mut Context<V>) + 'static,
) {
    if documents.iter().any(|doc| doc.read(cx).releasing) {
        return;
    }
    if documents.iter().any(|doc| doc.read(cx).busy()) {
        for doc in &documents {
            doc.update(cx, |doc, cx| doc.cancel(window, cx));
        }
        return;
    }
    if documents.is_empty() {
        cx.defer_in(window, complete);
        return;
    }
    let sessions = documents
        .iter()
        .map(|doc| doc.read(cx).session.clone())
        .collect::<Vec<_>>();
    for doc in &documents {
        doc.update(cx, |doc, cx| {
            doc.releasing = true;
            cx.notify();
        });
    }
    let observed = sessions.clone();
    let services = services.clone();
    task(
        &services.clone(),
        window,
        cx,
        async move { Ok(observe(&observed).await) },
        move |this, result, window, cx| {
            let states =
                result.unwrap_or_else(|_| vec![TransactionStatus::Unknown; documents.len()]);
            for (doc, state) in documents.iter().zip(&states) {
                doc.update(cx, |doc, cx| doc.observe_transaction(*state, cx));
            }
            if states.iter().all(|state| *state == TransactionStatus::Idle) {
                unlock(&documents, cx);
                complete(this, window, cx);
                return;
            }
            let affected = documents
                .iter()
                .map(|doc| {
                    let doc = doc.read(cx);
                    format!("{} — {}", doc.profile.context(), doc.title())
                })
                .collect::<Vec<_>>()
                .join("\n");
            let unknown = states.contains(&TransactionStatus::Unknown);
            let complete = Rc::new(RefCell::new(Some(complete)));
            let view = cx.entity().downgrade();
            window.open_alert_dialog(cx, move |dialog, _, _| {
            let documents = documents.clone();
            let dismissed = documents.clone();
            let sessions = sessions.clone();
            let complete = complete.clone();
            let view = view.clone();
            let services = services.clone();
            dialog.title("Close database sessions?")
                .child(affected.clone())
                .child(if unknown {
                    "A transaction outcome is unknown. A COMMIT may already have taken effect. Closing cannot undo or confirm it; Queri will not retry it. Other open transactions will be rolled back."
                } else {
                    "These sessions have uncommitted changes. Keep them open, or roll back their transactions and continue."
                })
                .button_props(DialogButtonProps::default().show_cancel(true).cancel_text("Keep open")
                    .ok_text(if unknown { "Close with unknown outcome" } else { "Roll back and close" }))
                .on_cancel(move |_, _, cx| { unlock(&dismissed, cx); true })
                .on_ok(move |_, window, cx| {
                    let Some(complete) = complete.borrow_mut().take() else { return false; };
                    let documents = documents.clone();
                    let sessions = sessions.clone();
                    let _ = view.update(cx, |_, cx| {
                        task(&services, window, cx, async move {
                            let result = rollback(&sessions, unknown).await;
                            let states = observe(&sessions).await;
                            Ok((result, states))
                        }, move |this, observed, window, cx| {
                            let (result, states) = observed.unwrap_or_else(|error| (Err(error), vec![TransactionStatus::Unknown; documents.len()]));
                            for (doc, state) in documents.iter().zip(states) {
                                doc.update(cx, |doc, cx| doc.observe_transaction(state, cx));
                            }
                            unlock(&documents, cx);
                            match result {
                                Ok(()) => complete(this, window, cx),
                                Err(error) => { let message = format!("{error:#}"); window.open_alert_dialog(cx, move |dialog, _, _| dialog.title("Sessions kept open").child(message.clone())); }
                            }
                        });
                    });
                    true
                })
        });
        },
    );
}

fn unlock(documents: &[Entity<Document>], cx: &mut App) {
    for doc in documents {
        doc.update(cx, |doc, cx| {
            doc.releasing = false;
            cx.notify();
        });
    }
}

async fn observe(sessions: &[Option<Arc<Session>>]) -> Vec<TransactionStatus> {
    let mut states = Vec::with_capacity(sessions.len());
    for session in sessions {
        states.push(match session {
            Some(session) => session
                .transaction_status()
                .await
                .unwrap_or(TransactionStatus::Unknown),
            None => TransactionStatus::Idle,
        });
    }
    states
}

async fn rollback(
    sessions: &[Option<Arc<Session>>],
    acknowledged_unknown: bool,
) -> anyhow::Result<()> {
    for session in sessions.iter().flatten() {
        match session
            .transaction_status()
            .await
            .unwrap_or(TransactionStatus::Unknown)
        {
            TransactionStatus::Open | TransactionStatus::Failed => {
                session.query("ROLLBACK".into()).await?;
                anyhow::ensure!(
                    session.transaction_status().await? == TransactionStatus::Idle,
                    "Rollback could not be confirmed. The session remains open."
                );
            }
            TransactionStatus::Unknown if !acknowledged_unknown => anyhow::bail!(
                "Connection lost. Transaction outcome is unknown. Review and try closing again to acknowledge it."
            ),
            _ => {}
        }
    }
    Ok(())
}
