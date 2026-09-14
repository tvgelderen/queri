use super::{Services, input};
use crate::ssh::{HostDecision, Prompt};
use gpui_kit::{
    component::{
        WindowExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::Input,
    },
    *,
};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

pub(super) fn install<V: 'static>(services: &Services, window: &mut Window, cx: &mut Context<V>) {
    let Some(mut prompts) = services.connector.take_prompts() else {
        return;
    };
    cx.spawn_in(window, async move |view, cx| {
        while let Some(prompt) = prompts.recv().await {
            let Ok((done, cancelled)) = view.update_in(cx, |_, window, cx| show(prompt, window, cx)) else { break; };
            tokio::select! {
                biased;
                _ = done.cancelled() => {},
                _ = cancelled.cancelled() => {
                    let _ = view.update_in(cx, |_, window, cx| { if !done.is_cancelled() { window.close_dialog(cx); } });
                }
            }
        }
    }).detach();
}

fn answer<T>(
    reply: &Arc<Mutex<Option<tokio::sync::oneshot::Sender<T>>>>,
    done: &CancellationToken,
    value: T,
) {
    if let Some(reply) = reply.lock().unwrap().take() {
        let _ = reply.send(value);
    }
    done.cancel();
}

pub(super) fn show(
    prompt: Prompt,
    window: &mut Window,
    cx: &mut App,
) -> (CancellationToken, CancellationToken) {
    let finished = CancellationToken::new();
    let done = finished.clone();
    match prompt {
        Prompt::Host {
            endpoint,
            algorithm,
            fingerprint,
            previous,
            reply,
            cancelled,
        } => {
            if reply.is_closed() || cancelled.is_cancelled() {
                done.cancel();
                return (done, cancelled);
            }
            let reply = Arc::new(Mutex::new(Some(reply)));
            window.open_alert_dialog(cx, move |dialog, _, _| {
                let cancel_done = done.clone(); let close_done = done.clone();
                let cancel = reply.clone();
                let close = reply.clone();
                let mut footer = h_flex().gap_2().child(Button::new("ssh-reject").label("Cancel").on_click(move |_, window, cx| { answer(&cancel, &cancel_done, HostDecision::Reject); window.close_dialog(cx); }));
                if previous.is_some() {
                    let reply = reply.clone(); let done = done.clone();
                    let details = format!("{endpoint}\n{algorithm}\nPrevious: {}\nNew: {fingerprint}", previous.as_deref().unwrap_or_default());
                    footer = footer.child(Button::new("ssh-replace").label("Replace trusted key…").on_click(move |_, window, cx| {
                        let reply = reply.clone();
                        let details = details.clone();
                        window.close_dialog(cx);
                        let cancelled = reply.clone(); let done = done.clone();
                        window.open_alert_dialog(cx, move |dialog, _, _| {
                            let reply = reply.clone(); let cancelled = cancelled.clone(); let accept_done = done.clone(); let cancel_done = done.clone();
                            dialog.title("Replace SSH host key?").child(details.clone()).child("Verify the new fingerprint with your server administrator before replacing it.")
                                .button_props(gpui_kit::component::dialog::DialogButtonProps::default().show_cancel(true).ok_text("Replace key"))
                                .on_ok(move |_, _, _| { answer(&reply, &accept_done, HostDecision::Replace); true })
                                .on_cancel(move |_, _, _| { answer(&cancelled, &cancel_done, HostDecision::Reject); true })
                        });
                    }));
                } else {
                    let session = reply.clone(); let persist = reply.clone(); let session_done = done.clone(); let persist_done = done.clone();
                    footer = footer.child(Button::new("ssh-trust-session").label("Trust for this session").on_click(move |_, window, cx| { answer(&session, &session_done, HostDecision::Session); window.close_dialog(cx); }))
                        .child(Button::new("ssh-trust-persist").primary().label("Trust and remember").on_click(move |_, window, cx| { answer(&persist, &persist_done, HostDecision::Persist); window.close_dialog(cx); }));
                }
                // Explicit buttons own dismissal so a replacement confirmation can retain its reply.
                dialog.title(if previous.is_some() { "SSH host key changed — connection blocked" } else { "Verify SSH host key" })
                    .child(format!("Host: {endpoint}\nAlgorithm: {algorithm}\nSHA256 fingerprint: {fingerprint}"))
                    .child(previous.as_ref().map(|old| format!("Previously trusted: {old}\nA changed key can indicate impersonation. Authentication is blocked until you explicitly replace it.")).unwrap_or_else(|| "Verify this fingerprint through a trusted channel before allowing authentication.".into()))
                    .keyboard(false).close_button(false).footer(footer)
                    .on_cancel(move |_, _, _| { answer(&close, &close_done, HostDecision::Reject); true })
            });
            (finished, cancelled)
        }
        Prompt::Passphrase {
            name,
            reply,
            cancelled,
        } => {
            if reply.is_closed() || cancelled.is_cancelled() {
                done.cancel();
                return (done, cancelled);
            }
            let reply = Arc::new(Mutex::new(Some(reply)));
            let value = input("", window, cx);
            value.update(cx, |state, cx| state.set_masked(true, window, cx));
            window.open_alert_dialog(cx, move |dialog, _, _| {
                let accept_done = done.clone();
                let cancel_done = done.clone();
                let value = value.clone();
                let accepted = reply.clone();
                let cancelled = reply.clone();
                dialog
                    .title(format!("SSH private key passphrase · {name}"))
                    .child(
                        Input::new(&value)
                            .aria_label("SSH key passphrase")
                            .mask_toggle(),
                    )
                    .button_props(
                        gpui_kit::component::dialog::DialogButtonProps::default()
                            .show_cancel(true)
                            .ok_text("Unlock key"),
                    )
                    .on_ok(move |_, _, cx| {
                        answer(
                            &accepted,
                            &accept_done,
                            Some(value.read(cx).value().to_string()),
                        );
                        true
                    })
                    .on_cancel(move |_, _, _| {
                        answer(&cancelled, &cancel_done, None);
                        true
                    })
            });
            (finished, cancelled)
        }
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::super::test_support::click_label;
    use super::show;
    use crate::ssh::{HostDecision, Prompt};
    use gpui_kit::test::{TestAppContextExt, TestWindowExt};
    use gpui_kit::{
        AppContext, Context, IntoElement, ParentElement, Render, TestAppContext, Window,
        WindowOptions, div,
    };
    use tokio_util::sync::CancellationToken;

    struct Root;
    impl Render for Root {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().child("SSH prompt test")
        }
    }

    #[gpui_kit::test]
    async fn ssh_trust_dialog_requires_explicit_decisions(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let (window, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Root)).unwrap()
        });
        for (button, expected) in [
            ("ssh-reject", HostDecision::Reject),
            ("ssh-trust-session", HostDecision::Session),
            ("ssh-trust-persist", HostDecision::Persist),
        ] {
            let (reply, response) = tokio::sync::oneshot::channel();
            cx.update_window(window, |_, window, cx| {
                show(
                    Prompt::Host {
                        endpoint: "[bastion]:22".into(),
                        algorithm: "ssh-ed25519".into(),
                        fingerprint: "SHA256:actual-handshake-key".into(),
                        previous: None,
                        reply,
                        cancelled: CancellationToken::new(),
                    },
                    window,
                    cx,
                );
                window.render_frame(cx);
                window.click(button, cx);
            })
            .unwrap();
            assert_eq!(response.await.unwrap(), expected);
            cx.wait_for(window, std::time::Duration::from_secs(5), |window, _| {
                window.try_find(button).is_none()
            })
            .await;
        }
        let (reply, mut response) = tokio::sync::oneshot::channel();
        cx.update_window(window, |_, window, cx| {
            show(
                Prompt::Host {
                    endpoint: "[bastion]:22".into(),
                    algorithm: "ssh-ed25519".into(),
                    fingerprint: "SHA256:new".into(),
                    previous: Some("SHA256:old".into()),
                    reply,
                    cancelled: CancellationToken::new(),
                },
                window,
                cx,
            );
            window.render_frame(cx);
            assert!(window.try_find("ssh-trust-session").is_none());
            assert!(window.try_find("ssh-trust-persist").is_none());
            window.click("ssh-replace", cx);
        })
        .unwrap();
        assert!(matches!(
            response.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        cx.update_window(window, |_, window, cx| {
            click_label(window, "Replace key", cx);
        })
        .unwrap();
        assert_eq!(response.await.unwrap(), HostDecision::Replace);
    }

    #[gpui_kit::test]
    async fn ssh_passphrase_dialog_cancels_without_credentials(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let (window, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Root)).unwrap()
        });
        let (reply, response) = tokio::sync::oneshot::channel();
        cx.update_window(window, |_, window, cx| {
            show(
                Prompt::Passphrase {
                    name: "Encrypted fixture".into(),
                    reply,
                    cancelled: CancellationToken::new(),
                },
                window,
                cx,
            );
            click_label(window, "Cancel", cx);
        })
        .unwrap();
        assert_eq!(response.await.unwrap(), None);
    }
}
