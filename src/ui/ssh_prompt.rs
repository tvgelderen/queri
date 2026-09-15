use super::{
    Services,
    controls::{Tone, dialog_icon},
};
use crate::ssh::{HostDecision, Prompt};
use gpui_kit::assets::IconName;
use gpui_kit::{
    component::{
        ActiveTheme, WindowExt,
        button::{Button, ButtonVariant, ButtonVariants},
        dialog::DialogButtonProps,
        h_flex,
        input::{Input, InputState},
        v_flex,
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

/// Labelled, monospaced key facts so fingerprints can be compared character by character.
fn facts(rows: Vec<(&'static str, String, Option<Hsla>)>, cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .gap_2p5()
        .p_3()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.muted)
        .children(rows.into_iter().map(|(label, value, color)| {
            h_flex()
                .items_start()
                .gap_3()
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(84.))
                        .text_xs()
                        .line_height(relative(1.6))
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal()
                        .text_xs()
                        .line_height(relative(1.6))
                        .font_family(theme.mono_font_family.clone())
                        .text_color(color.unwrap_or(theme.foreground))
                        .child(value),
                )
        }))
}

fn note(text: &'static str, cx: &App) -> Div {
    div()
        .text_sm()
        .line_height(relative(1.5))
        .text_color(cx.theme().muted_foreground)
        .child(text)
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
            window.open_alert_dialog(cx, move |dialog, _, cx| {
                let cancel_done = done.clone();
                let close_done = done.clone();
                let cancel = reply.clone();
                let close = reply.clone();
                let mut footer = h_flex().w_full().justify_end().gap_2().child(
                    Button::new("ssh-reject")
                        .label("Cancel")
                        .on_click(move |_, window, cx| {
                            answer(&cancel, &cancel_done, HostDecision::Reject);
                            window.close_dialog(cx);
                        }),
                );
                let danger = cx.theme().danger;
                let details = if let Some(old) = &previous {
                    let reply = reply.clone();
                    let done = done.clone();
                    let replacement = (
                        endpoint.clone(),
                        algorithm.clone(),
                        old.clone(),
                        fingerprint.clone(),
                    );
                    footer = footer.child(
                        Button::new("ssh-replace")
                            .danger()
                            .label("Replace trusted key…")
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let reply = reply.clone();
                                let done = done.clone();
                                let (endpoint, algorithm, old, new) = replacement.clone();
                                window.open_alert_dialog(cx, move |dialog, _, cx| {
                                    let accepted = reply.clone();
                                    let rejected = reply.clone();
                                    let accept_done = done.clone();
                                    let cancel_done = done.clone();
                                    let danger = cx.theme().danger;
                                    dialog
                                        .width(px(540.))
                                        .icon(dialog_icon(IconName::ShieldAlert, Tone::Danger, cx))
                                        .title("Replace the trusted host key?")
                                        .description(format!(
                                            "Queri will trust the new key for {endpoint} from now on."
                                        ))
                                        .child(
                                            v_flex()
                                                .gap_3()
                                                .child(facts(
                                                    vec![
                                                        ("Key type", algorithm.clone(), None),
                                                        ("Trusted", old.clone(), None),
                                                        ("New", new.clone(), Some(danger)),
                                                    ],
                                                    cx,
                                                ))
                                                .child(note(
                                                    "Only continue if your server administrator confirmed the new fingerprint.",
                                                    cx,
                                                )),
                                        )
                                        .button_props(
                                            DialogButtonProps::default()
                                                .show_cancel(true)
                                                .ok_text("Replace key")
                                                .ok_variant(ButtonVariant::Danger),
                                        )
                                        .on_ok(move |_, _, _| {
                                            answer(&accepted, &accept_done, HostDecision::Replace);
                                            true
                                        })
                                        .on_cancel(move |_, _, _| {
                                            answer(&rejected, &cancel_done, HostDecision::Reject);
                                            true
                                        })
                                });
                            }),
                    );
                    v_flex()
                        .gap_3()
                        .child(facts(
                            vec![
                                ("Host", endpoint.clone(), None),
                                ("Key type", algorithm.clone(), None),
                                ("Trusted", old.clone(), None),
                                ("Presented", fingerprint.clone(), Some(danger)),
                            ],
                            cx,
                        ))
                        .child(note(
                            "Authentication is blocked. A changed key can mean the server was reinstalled, or that someone is impersonating it.",
                            cx,
                        ))
                } else {
                    let session = reply.clone();
                    let persist = reply.clone();
                    let session_done = done.clone();
                    let persist_done = done.clone();
                    footer = footer
                        .child(
                            Button::new("ssh-trust-session")
                                .label("Trust for this session")
                                .on_click(move |_, window, cx| {
                                    answer(&session, &session_done, HostDecision::Session);
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("ssh-trust-persist")
                                .primary()
                                .label("Trust and remember")
                                .on_click(move |_, window, cx| {
                                    answer(&persist, &persist_done, HostDecision::Persist);
                                    window.close_dialog(cx);
                                }),
                        );
                    v_flex()
                        .gap_3()
                        .child(facts(
                            vec![
                                ("Host", endpoint.clone(), None),
                                ("Key type", algorithm.clone(), None),
                                ("Fingerprint", fingerprint.clone(), None),
                            ],
                            cx,
                        ))
                        .child(note(
                            "Compare the fingerprint with one from your server administrator before allowing authentication.",
                            cx,
                        ))
                };
                // Explicit buttons own dismissal so a replacement confirmation can retain its reply.
                dialog
                    .width(px(540.))
                    .icon(if previous.is_some() {
                        dialog_icon(IconName::ShieldAlert, Tone::Danger, cx)
                    } else {
                        dialog_icon(IconName::ShieldCheck, Tone::Neutral, cx)
                    })
                    .title(if previous.is_some() {
                        "SSH host key changed"
                    } else {
                        "Verify SSH host key"
                    })
                    .description(if previous.is_some() {
                        "The bastion presented a different key than the one you trusted."
                    } else {
                        "Queri has not connected to this bastion before."
                    })
                    .child(details)
                    .keyboard(false)
                    .close_button(false)
                    .footer(footer)
                    .on_cancel(move |_, _, _| {
                        answer(&close, &close_done, HostDecision::Reject);
                        true
                    })
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
            let value = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Passphrase")
            });
            value.read(cx).focus_handle(cx).focus(window, cx);
            window.open_alert_dialog(cx, move |dialog, _, cx| {
                let accept_done = done.clone();
                let cancel_done = done.clone();
                let value = value.clone();
                let accepted = reply.clone();
                let cancelled = reply.clone();
                dialog
                    .width(px(460.))
                    .icon(dialog_icon(IconName::LockKeyhole, Tone::Neutral, cx))
                    .title("Unlock SSH key")
                    .description(format!(
                        "{name} uses an encrypted private key. Enter its passphrase to continue."
                    ))
                    .child(
                        Input::new(&value)
                            .aria_label("SSH key passphrase")
                            .mask_toggle(),
                    )
                    .button_props(
                        DialogButtonProps::default()
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
