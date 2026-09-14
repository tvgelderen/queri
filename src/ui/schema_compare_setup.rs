//! Dedicated connection and credential lifecycle for schema comparison.
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
    component::{
        button::Button,
        input::{InputEvent, InputState},
        scroll::ScrollableElement,
        v_flex,
    },
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
    status: String,
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
            status: String::new(),
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
        self.generation += 1;
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        self.pending = false;
        self.comparison = None;
        self.status = "Capture cancelled or endpoints changed. Compare to capture again.".into();
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
                self.status = format!("{label}: select a connection.");
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
                self.status = format!("{label}: enter a schema.");
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
            self.status = "Source and Target must use the same engine.".into();
            cx.notify();
            return;
        }
        self.pending = true;
        self.status = "Connecting and capturing Source and Target…".into();
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
                this.status = match &result {
                    Ok(_) => format!(
                        "Source and Target captured at {}",
                        chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z")
                    ),
                    Err(error) => format!("{error:#}"),
                };
                if let (Some(child_generation), Some(child)) = (child_generation, &this.comparison)
                {
                    child.update(cx, |child, cx| {
                        child.complete_capture(child_generation, result, cx);
                    });
                } else if let Ok((source, target, session)) = result {
                    let child = cx.new(|_| {
                        SchemaComparison::new(this.services.clone(), source, target, session)
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
impl Render for SchemaCompareSetup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut view = v_flex()
            .id("schema-compare-setup")
            .size_full()
            .overflow_y_scrollbar()
            .p_4()
            .gap_3()
            .child("Compare database schemas")
            .child(self.status.clone());
        if let Some(child) = &self.comparison {
            for (index, endpoint) in self.endpoints.iter().enumerate() {
                let label = if index == 0 { "Source" } else { "Target" };
                if let Some(profile) = &endpoint.profile {
                    view = view.child(format!(
                        "{label}: {} / {} / {}",
                        profile.display_name(),
                        endpoint.database.read(cx).value(),
                        endpoint.namespace.read(cx).value()
                    ));
                }
            }
            view = view.child(child.clone());
        } else {
            for index in 0..2 {
                let label = if index == 0 {
                    "Source (capture only)"
                } else {
                    "Target (apply destination)"
                };
                view = view.child(label);
                for (choice, profile) in self.profiles.iter().enumerate() {
                    let profile = profile.clone();
                    view = view.child(
                        Button::new((
                            if index == 0 {
                                "source-profile"
                            } else {
                                "target-profile"
                            },
                            choice,
                        ))
                        .label(profile.context())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.select(index, profile.clone(), window, cx)
                            },
                        )),
                    );
                }
                let endpoint = &self.endpoints[index];
                view = view
                    .child(
                        endpoint
                            .profile
                            .as_ref()
                            .map(|p| format!("Selected: {}", p.context()))
                            .unwrap_or_else(|| "No connection selected".into()),
                    )
                    .child(field(
                        format!("{label} database / SQLite file"),
                        &endpoint.database,
                    ))
                    .child(field(format!("{label} schema"), &endpoint.namespace))
                    .child(field(format!("{label} password"), &endpoint.password));
            }
            view = view.child(
                Button::new("compare")
                    .label("Compare")
                    .on_click(cx.listener(|this, _, window, cx| this.capture(None, window, cx))),
            );
        }
        if self.pending {
            view = view.child(
                Button::new("cancel-capture")
                    .label("Cancel capture")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            );
        }
        view.child(
            Button::new("close-setup")
                .label("Back to workspace")
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.can_close(cx) {
                        this.cancel(cx);
                        cx.emit(SchemaCompareSetupEvent::Back);
                    }
                })),
        )
    }
}
