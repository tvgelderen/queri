//! Dedicated connection and credential lifecycle for schema comparison.
use super::controls::{Tone, card, choice, notice, page_header};
use super::{
    Services, field, input,
    schema_compare::{SchemaComparison, SchemaComparisonEvent},
    task,
};
use crate::{
    model::{ConnectionConfig, ConnectionProfile},
    schema::Scope,
};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::{InputEvent, InputState},
        scroll::ScrollableElement,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::collections::HashMap;
use uuid::Uuid;

pub enum SchemaCompareSetupEvent {
    Back,
    OpenSQL(String),
    Applied,
}
impl EventEmitter<SchemaCompareSetupEvent> for SchemaCompareSetup {}
struct Endpoint {
    profile: Option<ConnectionProfile>,
    database: Entity<InputState>,
    namespace: Entity<InputState>,
    password: Entity<InputState>,
}
pub struct SchemaCompareSetup {
    services: Services,
    profiles: Vec<ConnectionProfile>,
    passwords: HashMap<Uuid, String>,
    endpoints: [Endpoint; 2],
    generation: u64,
    pending: bool,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    comparison: Option<Entity<SchemaComparison>>,
    status: Option<(Tone, String)>,
}
impl SchemaCompareSetup {
    pub fn target_namespace(&self, cx: &App) -> String {
        self.endpoints[1].namespace.read(cx).value().to_string()
    }

    pub fn target_profile(&self, cx: &App) -> Option<ConnectionProfile> {
        self.endpoints[1]
            .profile
            .as_ref()
            .map(|p| p.for_database(&self.endpoints[1].database.read(cx).value()))
    }

    pub fn new(
        services: Services,
        profiles: Vec<ConnectionProfile>,
        session_passwords: HashMap<Uuid, String>,
        initial_target: Option<ConnectionProfile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let endpoints = std::array::from_fn(|_| Endpoint {
            profile: None,
            database: input("", window, cx),
            namespace: input("public", window, cx),
            password: cx.new(|cx| InputState::new(window, cx).masked(true)),
        });
        for endpoint in &endpoints {
            for field in [&endpoint.database, &endpoint.namespace, &endpoint.password] {
                cx.subscribe(field, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.cancel(cx);
                    }
                })
                .detach();
            }
        }
        let mut this = Self {
            services,
            profiles,
            passwords: session_passwords,
            endpoints,
            generation: 0,
            pending: false,
            cancel: None,
            comparison: None,
            status: None,
        };
        if let Some(profile) = initial_target {
            this.select(1, profile, window, cx);
        }
        this
    }
    pub fn busy(&self, cx: &App) -> bool {
        self.pending
            || self
                .comparison
                .as_ref()
                .is_some_and(|child| child.read(cx).busy(cx))
    }
    pub fn can_close(&self, cx: &App) -> bool {
        !self
            .comparison
            .as_ref()
            .is_some_and(|child| child.read(cx).busy(cx))
    }
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self
            .comparison
            .as_ref()
            .is_some_and(|child| child.read(cx).busy(cx))
            && !self.pending
        {
            return;
        }
        let interrupted = self.pending || self.comparison.is_some();
        self.generation += 1;
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        self.pending = false;
        self.comparison = None;
        self.status = interrupted.then(|| {
            (
                Tone::Neutral,
                "Capture cancelled or endpoints changed. Compare to capture again.".into(),
            )
        });
        cx.notify();
    }
    fn select(
        &mut self,
        index: usize,
        profile: ConnectionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel(cx);
        let namespace = if matches!(profile.config, ConnectionConfig::Sqlite { .. }) {
            "main"
        } else {
            "public"
        };
        let endpoint = &mut self.endpoints[index];
        endpoint.database.update(cx, |input, cx| {
            let database = match &profile.config {
                ConnectionConfig::Sqlite { path, .. } => path.to_string_lossy().into_owned(),
                _ => profile.database(),
            };
            input.set_value(database, window, cx)
        });
        endpoint
            .namespace
            .update(cx, |input, cx| input.set_value(namespace, window, cx));
        endpoint
            .password
            .update(cx, |input, cx| input.set_value("", window, cx));
        endpoint.profile = Some(profile);
    }
    fn capture(
        &mut self,
        child_generation: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        self.generation += 1;
        let generation = self.generation;
        let mut requests = Vec::new();
        for (index, endpoint) in self.endpoints.iter().enumerate() {
            let label = if index == 0 { "Source" } else { "Target" };
            let Some(mut profile) = endpoint.profile.clone() else {
                self.status = Some((Tone::Danger, format!("{label}: choose a connection.")));
                cx.notify();
                return;
            };
            if index == 0 {
                match &mut profile.config {
                    ConnectionConfig::Sqlite { read_only, .. } => *read_only = true,
                    ConnectionConfig::Postgres { .. } => {
                        profile.policy = crate::model::ExecutionPolicy::ReadOnlyByDefault
                    }
                }
            }
            let database = endpoint.database.read(cx).value().to_string();
            match &mut profile.config {
                ConnectionConfig::Postgres { database: name, .. } => *name = database,
                ConnectionConfig::Sqlite { path, create, .. } => {
                    *path = database.into();
                    *create = false;
                }
            }
            let namespace = endpoint.namespace.read(cx).value().to_string();
            if namespace.trim().is_empty() {
                self.status = Some((Tone::Danger, format!("{label}: enter a schema.")));
                cx.notify();
                return;
            }
            let entered = endpoint.password.read(cx).value().to_string();
            let password = if entered.is_empty() {
                self.passwords.get(&profile.id).cloned()
            } else {
                Some(entered)
            };
            requests.push((label, profile, namespace, password));
        }
        if std::mem::discriminant(&requests[0].1.config)
            != std::mem::discriminant(&requests[1].1.config)
        {
            self.status = Some((
                Tone::Danger,
                "Source and Target must use the same engine.".into(),
            ));
            cx.notify();
            return;
        }
        self.pending = true;
        self.status = Some((
            Tone::Neutral,
            "Connecting and capturing Source and Target…".into(),
        ));
        let services = self.services.clone();
        let (send, mut receive) = tokio::sync::oneshot::channel();
        self.cancel = Some(send);
        task(
            &self.services,
            window,
            cx,
            async move {
                tokio::select! {
                    _ = &mut receive => anyhow::bail!("Capture cancelled"),
                    result = async move {
                        let mut captures = Vec::new();
                        for (label, profile, namespace, password) in requests {
                            let result = async {
                                let password = match password {
                                    Some(password) => password,
                                    None if matches!(profile.config, ConnectionConfig::Sqlite { .. }) => String::new(),
                                    None => {
                                        let store = services.store.clone(); let id = profile.id;
                                        let stored = if profile.remember_password { tokio::task::spawn_blocking(move || store.load_password(id)).await?? } else { None };
                                        stored.ok_or_else(|| anyhow::anyhow!("Enter a password (used only for this comparison)."))?
                                    }
                                };
                                let session = services.connector.connect(profile, password).await?;
                                let snapshot = session.schema_snapshot(Scope { namespace }).await?;
                                Ok::<_, anyhow::Error>((snapshot, session))
                            }.await.map_err(|error| anyhow::anyhow!("{label}: {error:#}"))?;
                            captures.push(result);
                        }
                        let (target, session) = captures.pop().unwrap();
                        let (source, _) = captures.pop().unwrap();
                        Ok((source, target, session))
                    } => result
                }
            },
            move |this, result, window, cx| {
                if this.generation != generation || !this.pending {
                    return;
                }
                this.pending = false;
                this.cancel = None;
                this.status = Some(match &result {
                    Ok(_) => (
                        Tone::Success,
                        format!(
                            "Source and Target captured at {}",
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z")
                        ),
                    ),
                    Err(error) => (Tone::Danger, format!("{error:#}")),
                });
                if let (Some(child_generation), Some(child)) = (child_generation, &this.comparison)
                {
                    child.update(cx, |child, cx| {
                        child.complete_capture(child_generation, result, cx);
                    });
                } else if let Ok((source, target, session)) = result {
                    let source_label = this.endpoints[0].profile.as_ref().map(|profile| {
                        profile
                            .for_database(&this.endpoints[0].database.read(cx).value())
                            .context()
                    });
                    let child = cx.new(|_| {
                        let mut child =
                            SchemaComparison::new(this.services.clone(), source, target, session);
                        if let Some(label) = source_label {
                            child.set_source_label(label);
                        }
                        child
                    });
                    cx.subscribe_in(&child, window, |this, _, event, window, cx| match event {
                        SchemaComparisonEvent::Back
                        | SchemaComparisonEvent::ChangeSource
                        | SchemaComparisonEvent::ChangeTarget => this.cancel(cx),
                        SchemaComparisonEvent::Capture { generation } => {
                            this.capture(Some(*generation), window, cx)
                        }
                        SchemaComparisonEvent::OpenSQL(sql) => {
                            cx.emit(SchemaCompareSetupEvent::OpenSQL(sql.clone()))
                        }
                        SchemaComparisonEvent::Applied => cx.emit(SchemaCompareSetupEvent::Applied),
                    })
                    .detach();
                    this.comparison = Some(child);
                }
                cx.notify();
            },
        );
        cx.notify();
    }
}
impl SchemaCompareSetup {
    fn endpoint_card(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (role, description, id) = if index == 0 {
            (
                "Source",
                "Read only. Captured, never changed.",
                "source-profile",
            )
        } else {
            (
                "Target",
                "Reviewed changes are applied here.",
                "target-profile",
            )
        };
        let endpoint = &self.endpoints[index];
        let selected = endpoint
            .profile
            .as_ref()
            .and_then(|profile| self.profiles.iter().position(|p| p.id == profile.id));
        let sqlite = endpoint
            .profile
            .as_ref()
            .is_some_and(|p| matches!(p.config, ConnectionConfig::Sqlite { .. }));
        let options = self
            .profiles
            .iter()
            .enumerate()
            .map(|(i, p)| (Some(i), SharedString::from(p.context())))
            .collect::<Vec<_>>();
        card(cx)
            .flex_1()
            .min_w_0()
            .p_4()
            .gap_4()
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(role),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(description),
                    ),
            )
            .child(
                v_flex()
                    .gap_1p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Connection"),
                    )
                    .child(choice(
                        Button::new(id)
                            .outline()
                            .w_full()
                            .justify_between()
                            .accessibility_label(format!("{role} connection"))
                            .label(
                                endpoint
                                    .profile
                                    .as_ref()
                                    .map(|p| p.context())
                                    .unwrap_or_else(|| "Choose a connection".into()),
                            ),
                        selected,
                        options,
                        cx.listener(move |this, choice: &Option<usize>, window, cx| {
                            if let Some(profile) =
                                choice.and_then(|i| this.profiles.get(i)).cloned()
                            {
                                this.select(index, profile, window, cx);
                                cx.notify();
                            }
                        }),
                    )),
            )
            .child(field(
                if sqlite { "SQLite file" } else { "Database" },
                &endpoint.database,
            ))
            .child(field(
                if sqlite {
                    "Attached database"
                } else {
                    "Schema"
                },
                &endpoint.namespace,
            ))
            .when(!sqlite, |card| {
                card.child(
                    v_flex()
                        .gap_1()
                        .child(field("Password", &endpoint.password))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    "Optional. Leave empty to use the saved or session password.",
                                ),
                        ),
                )
            })
    }
}
impl Render for SchemaCompareSetup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(child) = &self.comparison {
            return v_flex()
                .size_full()
                .child(div().flex_1().min_h_0().child(child.clone()))
                .when(self.pending, |view| {
                    view.child(
                        h_flex()
                            .flex_shrink_0()
                            .h_11()
                            .px_4()
                            .gap_3()
                            .justify_between()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().muted)
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Capturing Source and Target…"),
                            )
                            .child(
                                Button::new("cancel-capture")
                                    .outline()
                                    .small()
                                    .label("Cancel capture")
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                            ),
                    )
                })
                .into_any_element();
        }
        let mut actions = vec![
            Button::new("close-setup")
                .ghost()
                .small()
                .label("Close")
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.can_close(cx) {
                        this.cancel(cx);
                        cx.emit(SchemaCompareSetupEvent::Back);
                    }
                }))
                .into_any_element(),
        ];
        if self.pending {
            actions.push(
                Button::new("cancel-capture")
                    .outline()
                    .small()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
                    .into_any_element(),
            );
        }
        actions.push(
            Button::new("compare")
                .primary()
                .small()
                .icon(IconName::GitCompareArrows)
                .label("Compare")
                .loading(self.pending)
                .disabled(self.pending)
                .on_click(cx.listener(|this, _, window, cx| this.capture(None, window, cx)))
                .into_any_element(),
        );
        let source = self.endpoint_card(0, cx);
        let target = self.endpoint_card(1, cx);
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(page_header(
                "Compare schemas",
                "Plan changes that make the target schema match the source",
                actions,
                cx,
            ))
            .child(
                div()
                    .id("schema-compare-setup")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(px(960.))
                            .mx_auto()
                            .p_6()
                            .gap_4()
                            .children(
                                self.status.clone().map(|(tone, message)| {
                                    notice("compare-status", tone, message, cx)
                                }),
                            )
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_3()
                                    .child(source)
                                    .child(
                                        div()
                                            .pt(px(18.))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(Icon::new(IconName::ArrowRight).small()),
                                    )
                                    .child(target),
                            ),
                    ),
            )
            .into_any_element()
    }
}
