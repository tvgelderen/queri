use super::{
    Services,
    controls::{choice, disclosure_button, setting_switch},
    input, task,
};
use crate::{database::Session, model::*};
use anyhow::Result;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    form::Field,
    h_flex,
    input::{Input, InputState},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use uuid::Uuid;

pub struct SavedConnection {
    pub profile: ConnectionProfile,
    pub password: String,
}

pub struct ConnectionForm {
    services: Services,
    id: Uuid,
    sqlite: bool,
    existing: bool,
    advanced: bool,
    uri_mode: bool,
    name: Entity<InputState>,
    uri: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    database: Entity<InputState>,
    user: Entity<InputState>,
    password: Entity<InputState>,
    path: Entity<InputState>,
    notes: Entity<InputState>,
    color: Entity<InputState>,
    timeout: Entity<InputState>,
    tls: TlsMode,
    environment: Option<Environment>,
    policy: ExecutionPolicy,
    read_only: bool,
    create: bool,
    wal: bool,
    foreign_keys: bool,
    remember: bool,
    busy: bool,
    message: String,
    error: bool,
    field_error: Option<(Entity<InputState>, String)>,
}
impl EventEmitter<SavedConnection> for ConnectionForm {}

impl ConnectionForm {
    pub fn new(
        services: Services,
        existing: Option<ConnectionProfile>,
        cached_password: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut form = Self {
            existing: existing.is_some(),
            advanced: false,
            uri_mode: false,
            services,
            id: existing
                .as_ref()
                .map(|profile| profile.id)
                .unwrap_or_else(Uuid::new_v4),
            sqlite: existing
                .as_ref()
                .is_some_and(|profile| matches!(profile.config, ConnectionConfig::Sqlite { .. })),
            name: input(
                existing
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_default(),
                window,
                cx,
            ),
            uri: input("", window, cx),
            host: input("localhost", window, cx),
            port: input("5432", window, cx),
            database: input("postgres", window, cx),
            user: input("postgres", window, cx),
            password: cx.new(|cx| InputState::new(window, cx).masked(true)),
            path: input("", window, cx),
            notes: input(
                existing
                    .as_ref()
                    .map(|p| p.notes.clone())
                    .unwrap_or_default(),
                window,
                cx,
            ),
            color: input(
                existing
                    .as_ref()
                    .map(|p| p.color.clone())
                    .unwrap_or_default(),
                window,
                cx,
            ),
            timeout: input("5", window, cx),
            environment: existing.as_ref().and_then(|p| p.environment),
            policy: existing.as_ref().map(|p| p.policy).unwrap_or_default(),
            tls: TlsMode::Verify,
            read_only: false,
            create: false,
            wal: true,
            foreign_keys: true,
            remember: existing
                .as_ref()
                .is_none_or(|profile| profile.remember_password),
            busy: false,
            message: String::new(),
            error: false,
            field_error: None,
        };
        if let Some(profile) = existing {
            match profile.config {
                ConnectionConfig::Postgres {
                    host,
                    port,
                    database,
                    user,
                    tls,
                    timeout_seconds,
                } => {
                    for (state, value) in [
                        (&form.host, host),
                        (&form.port, port.to_string()),
                        (&form.database, database),
                        (&form.user, user),
                        (&form.timeout, timeout_seconds.to_string()),
                    ] {
                        state.update(cx, |state, cx| state.set_value(value, window, cx));
                    }
                    form.tls = tls;
                    if let Some(password) = cached_password {
                        form.password
                            .update(cx, |state, cx| state.set_value(password, window, cx));
                        return form;
                    }
                    if !form.remember {
                        form.message = "Enter your password to reconnect.".into();
                        return form;
                    }
                    let id = form.id;
                    let store = form.services.store.clone();
                    form.busy = true;
                    task(
                        &form.services,
                        window,
                        cx,
                        async move {
                            tokio::task::spawn_blocking(move || store.load_password(id)).await?
                        },
                        move |this, result, window, cx| {
                            this.busy = false;
                            match result {
                                Ok(Some(password)) => this
                                    .password
                                    .update(cx, |state, cx| state.set_value(password, window, cx)),
                                Ok(None) => {
                                    this.message = "Enter your password again to reconnect.".into();
                                }
                                Err(error) => {
                                    eprintln!("Could not load the connection password: {error:#}");
                                    this.message = "Could not read the saved password. Enter it again; turn off Remember password if saving fails.".into();
                                    this.error = true;
                                }
                            }
                            cx.notify();
                        },
                    );
                }
                ConnectionConfig::Sqlite {
                    path,
                    read_only,
                    create: _,
                    wal,
                    foreign_keys,
                } => {
                    form.path.update(cx, |state, cx| {
                        state.set_value(path.to_string_lossy().to_string(), window, cx)
                    });
                    form.read_only = read_only;
                    form.create = false;
                    form.wal = wal;
                    form.foreign_keys = foreign_keys;
                }
            }
        }
        form
    }

    fn config(&self, cx: &App) -> Result<ConnectionProfile> {
        let config = if self.sqlite {
            ConnectionConfig::Sqlite {
                path: self.path.read(cx).value().to_string().into(),
                read_only: self.read_only,
                create: self.create,
                wal: self.wal,
                foreign_keys: self.foreign_keys,
            }
        } else {
            ConnectionConfig::Postgres {
                host: self.host.read(cx).value().to_string(),
                port: self.port.read(cx).value().parse().map_err(|_| {
                    ConnectionValidationError {
                        field: ConnectionField::Port,
                        message: "Enter a port between 1 and 65535.",
                    }
                })?,
                database: self.database.read(cx).value().to_string(),
                user: self.user.read(cx).value().to_string(),
                tls: self.tls.clone(),
                timeout_seconds: self.timeout.read(cx).value().parse().map_err(|_| {
                    ConnectionValidationError {
                        field: ConnectionField::Timeout,
                        message: "Enter a timeout in seconds.",
                    }
                })?,
            }
        };
        let profile = ConnectionProfile {
            environment: self.environment,
            policy: self.policy,
            remember_password: !self.sqlite && self.remember,
            id: self.id,
            name: self.name.read(cx).value().to_string(),
            notes: self.notes.read(cx).value().to_string(),
            color: self.color.read(cx).value().to_string(),
            config,
        };
        profile.validate()?;
        Ok(profile)
    }

    pub fn is_existing(&self) -> bool {
        self.existing
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    fn field(&self, label: &str, input: &Entity<InputState>) -> Field {
        self.field_with_content(label, input, |input| input)
    }

    fn field_with_content<E: IntoElement>(
        &self,
        label: &str,
        input: &Entity<InputState>,
        content: impl FnOnce(Input) -> E,
    ) -> Field {
        let error = self
            .field_error
            .as_ref()
            .filter(|(field, _)| field == input);
        let label = SharedString::from(label.to_owned());
        Field::new()
            .label(label.clone())
            .child(content(
                Input::new(input)
                    .id(label.clone())
                    .aria_label(label)
                    .disabled(self.busy),
            ))
            .when_some(error, |field, (_, message)| {
                let message = message.clone();
                field.description_fn(move |_, cx| {
                    div()
                        .id("connection-field-error")
                        .role(Role::Alert)
                        .aria_label(message.clone())
                        .text_color(cx.theme().danger)
                        .child(message.clone())
                })
            })
    }

    fn invalid_field(
        &mut self,
        input: Entity<InputState>,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.field_error = Some((input, message));
        self.message.clear();
        self.error = true;
        cx.notify();
    }

    fn parse_uri(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.uri.read(cx).value().parse::<tokio_postgres::Config>();
        match result {
            Ok(config) => {
                let host = match config.get_hosts().first() {
                    Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
                    _ => "localhost".into(),
                };
                let password =
                    String::from_utf8(config.get_password().unwrap_or_default().to_vec());
                match password {
                    Ok(password) => {
                        let values = [
                            (&self.host, host),
                            (
                                &self.port,
                                config
                                    .get_ports()
                                    .first()
                                    .copied()
                                    .unwrap_or(5432)
                                    .to_string(),
                            ),
                            (
                                &self.database,
                                config.get_dbname().unwrap_or("postgres").into(),
                            ),
                            (&self.user, config.get_user().unwrap_or("postgres").into()),
                            (&self.password, password),
                        ];
                        for (state, value) in values {
                            state.update(cx, |state, cx| state.set_value(value, window, cx));
                        }
                        self.tls = match config.get_ssl_mode() {
                            tokio_postgres::config::SslMode::Disable => TlsMode::Disable,
                            _ => TlsMode::Verify,
                        };
                        self.uri
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        self.message = "Connection string applied.".into();
                        self.uri_mode = false;
                        self.error = false;
                    }
                    Err(error) => {
                        self.invalid_field(self.uri.clone(), error.to_string(), window, cx);
                    }
                }
            }
            Err(error) => {
                self.invalid_field(self.uri.clone(), error.to_string(), window, cx);
            }
        }
        cx.notify();
    }

    fn submit(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.field_error = None;
        if !self.sqlite && self.uri_mode {
            self.parse_uri(window, cx);
            if self.error {
                return;
            }
        }
        let mut profile = match self.config(cx) {
            Ok(profile) => profile,
            Err(error) => {
                if let Some(validation) = error.downcast_ref::<ConnectionValidationError>() {
                    let input = match validation.field {
                        ConnectionField::Name => self.name.clone(),
                        ConnectionField::Color => {
                            self.advanced = true;
                            self.color.clone()
                        }
                        ConnectionField::Host => self.host.clone(),
                        ConnectionField::Port => self.port.clone(),
                        ConnectionField::User => self.user.clone(),
                        ConnectionField::Timeout => {
                            self.advanced = true;
                            self.timeout.clone()
                        }
                        ConnectionField::Path => self.path.clone(),
                    };
                    self.invalid_field(input, validation.to_string(), window, cx);
                    return;
                }
                self.message = format!("{error:#}");
                self.error = true;
                cx.notify();
                return;
            }
        };
        let password = self.password.read(cx).value().to_string();
        let store = self.services.store.clone();
        let sqlite = self.sqlite;
        let creating = sqlite && self.create && !self.existing;
        self.busy = true;
        self.error = false;
        self.message = if save {
            "Saving…"
        } else {
            "Testing connection…"
        }
        .into();
        task(
            &self.services,
            window,
            cx,
            async move {
                if save {
                    if let ConnectionConfig::Sqlite { path, create, .. } = &mut profile.config {
                        if creating {
                            crate::database::create_sqlite_file(path.clone()).await?;
                        }
                        *create = false;
                    }
                    let profile_copy = profile.clone();
                    let password_copy = password.clone();
                    tokio::task::spawn_blocking(move || {
                        store.save_connection(&profile_copy, &password_copy)
                    })
                    .await??;
                } else {
                    let session = Session::connect(profile.clone(), password.clone()).await?;
                    session.query("SELECT 1".into()).await?;
                }
                Ok(SavedConnection { profile, password })
            },
            move |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok(saved) => {
                        this.message = "Connection successful.".into();
                        if save {
                            cx.emit(saved);
                            window.close_dialog(cx);
                        }
                    }
                    Err(error) => {
                        this.message = if save {
                            eprintln!("Could not save the connection: {error:#}");
                            if this.sqlite || !this.remember {
                                "Could not save the connection. Check file permissions and retry."
                                    .into()
                            } else {
                                "Could not save the connection. Check file permissions, or turn off Remember password and retry.".into()
                            }
                        } else {
                            format!("Connection test failed.\n\n{error:#}")
                        };
                        this.error = true;
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.create && !self.existing {
            let receiver =
                cx.prompt_for_new_path(std::path::Path::new("."), Some("database.sqlite"));
            cx.spawn_in(window, async move |view, cx| {
                let result = receiver.await;
                let _ = view.update_in(cx, |this, window, cx| {
                    match result {
                        Ok(Ok(Some(path))) => this.path.update(cx, |state, cx| state.set_value(path.to_string_lossy().to_string(), window, cx)),
                        Ok(Err(error)) => { this.message = format!("Could not open the file picker: {error}. Enter a file path instead."); this.error = true; }
                        _ => {}
                    }
                    cx.notify();
                });
            }).detach();
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open SQLite database".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = receiver.await;
            let _ = view.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            this.path.update(cx, |state, cx| {
                                state.set_value(path.to_string_lossy().to_string(), window, cx)
                            });
                        }
                    }
                    Ok(Err(error)) => {
                        this.message = error.to_string();
                        this.error = true;
                    }
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("cancel-connection")
                    .label("Cancel")
                    .disabled(self.busy)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .when(!(self.sqlite && self.create && !self.existing), |footer| {
                footer.child(
                    Button::new("test-connection")
                        .label("Test connection")
                        .disabled(self.busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.submit(false, window, cx)),
                        ),
                )
            })
            .child(
                Button::new("save-connection")
                    .primary()
                    .label(if self.sqlite && self.create && !self.existing {
                        "Create database"
                    } else {
                        "Save and connect"
                    })
                    .loading(self.busy)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(true, window, cx))),
            )
            .into_any_element()
    }
}

impl Render for ConnectionForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut form = v_flex()
            .gap_4()
            .child(self.field("Connection name", &self.name));
        if self.sqlite {
            form = form
                .when(!self.existing, |form| {
                    form.child(
                        TabBar::new("sqlite-mode")
                            .segmented()
                            .selected_index(usize::from(self.create))
                            .child(Tab::new().flex_1().label("Open existing file"))
                            .child(Tab::new().flex_1().label("Create database"))
                            .on_click(cx.listener(|this, index, _, cx| {
                                this.create = *index == 1;
                                if this.create {
                                    this.read_only = false;
                                }
                                cx.notify();
                            })),
                    )
                })
                .child(self.field_with_content(
                    if self.create && !self.existing {
                        "New file path"
                    } else {
                        "SQLite file"
                    },
                    &self.path,
                    |input| {
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(input))
                            .child(
                                Button::new("browse-file")
                                    .icon(IconName::FolderOpen)
                                    .label("Browse…")
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.browse(window, cx)),
                                    ),
                            )
                    },
                ))
                .when(!self.create, |form| {
                    form.child(setting_switch(
                        "read-only",
                        "Read-only",
                        self.read_only,
                        cx.listener(|this, on, _, cx| {
                            this.read_only = *on;
                            if *on {
                                this.create = false;
                            }
                            cx.notify();
                        }),
                    ))
                });
        } else {
            form = form
                .child(
                    TabBar::new("postgres-entry-mode")
                        .segmented()
                        .selected_index(usize::from(self.uri_mode))
                        .child(Tab::new().flex_1().label("Connection details"))
                        .child(Tab::new().flex_1().label("Connection string"))
                        .on_click(cx.listener(|this, index, _, cx| {
                            this.uri_mode = *index == 1;
                            cx.notify();
                        })),
                )
                .when(self.uri_mode, |form| {
                    form.child(self.field("PostgreSQL connection string", &self.uri))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("postgresql://user:password@host:5432/database"),
                        )
                })
                .when(!self.uri_mode, |form| {
                    form.child(
                        h_flex()
                            .gap_3()
                            .child(div().flex_1().child(self.field("Host", &self.host)))
                            .child(div().w(px(110.)).child(self.field("Port", &self.port))),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(div().flex_1().child(self.field("Database", &self.database)))
                            .child(div().flex_1().child(self.field("User", &self.user))),
                    )
                    .child(
                        gpui_kit::component::form::Field::new()
                            .label("Password")
                            .child(
                                Input::new(&self.password)
                                    .aria_label("Password")
                                    .mask_toggle(),
                            ),
                    )
                })
                .child(setting_switch(
                    "remember",
                    "Remember password",
                    self.remember,
                    cx.listener(|this, on, _, cx| {
                        this.remember = *on;
                        cx.notify();
                    }),
                ));
        }
        form = form
            .child(h_flex().gap_2().flex_wrap()
                .child(choice(
                    Button::new("connection-environment").label(format!("Environment: {}", self.environment.map(Environment::label).unwrap_or("None")))
                        .tooltip("A label only; choosing Production never changes policy.").disabled(self.busy),
                    self.environment,
                    [(None, "None".into()), (Some(Environment::Development), "Development".into()), (Some(Environment::Staging), "Staging".into()), (Some(Environment::Production), "Production".into())],
                    cx.listener(|this, value, _, cx| { this.environment = *value; cx.notify(); }),
                ))
                .child(choice(
                    Button::new("connection-policy").label(format!("Policy: {}", self.policy.label())).disabled(self.busy),
                    self.policy,
                    [ExecutionPolicy::Normal, ExecutionPolicy::ConfirmExecutions, ExecutionPolicy::ReadOnlyByDefault].into_iter()
                        .filter(|policy| !self.sqlite || *policy != ExecutionPolicy::ReadOnlyByDefault)
                        .map(|policy| (policy, policy.label().into())),
                    cx.listener(|this, value, _, cx| { this.policy = *value; cx.notify(); }),
                )))
            .when(self.policy == ExecutionPolicy::ReadOnlyByDefault, |form| form.child(div().text_sm().text_color(cx.theme().muted_foreground).child("PostgreSQL read-only is a session default: SQL can override it and temporary objects have exceptions. Generated changes and administration stay blocked. Use database role permissions for enforcement.")))
            .child(
                disclosure_button("connection-advanced", "Advanced settings", self.advanced)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced = !this.advanced;
                        cx.notify();
                    })),
            )
            .when(self.advanced, |form| {
                form.child(
                    v_flex()
                        .gap_3()
                        .when(self.sqlite, |advanced| {
                            advanced
                                .child(setting_switch(
                                    "wal",
                                    "Write-ahead logging (WAL)",
                                    self.wal,
                                    cx.listener(|this, on, _, cx| {
                                        this.wal = *on;
                                        cx.notify();
                                    }),
                                ))
                                .child(setting_switch(
                                    "foreign-keys",
                                    "Enforce foreign keys",
                                    self.foreign_keys,
                                    cx.listener(|this, on, _, cx| {
                                        this.foreign_keys = *on;
                                        cx.notify();
                                    }),
                                ))
                        })
                        .when(!self.sqlite, |advanced| {
                            advanced
                                .child(div().text_sm().child("TLS encryption"))
                                .child(choice(
                                    Button::new("tls").label(match self.tls {
                                        TlsMode::Disable => "Disabled",
                                        TlsMode::Verify => "Verify certificate and hostname",
                                        TlsMode::Require => "Encrypt without verification",
                                    }),
                                    self.tls.clone(),
                                    [
                                        (TlsMode::Verify, "Verify certificate and hostname".into()),
                                        (TlsMode::Require, "Encrypt without verification".into()),
                                        (TlsMode::Disable, "Disabled".into()),
                                    ],
                                    cx.listener(|this, tls: &TlsMode, _, cx| {
                                        this.tls = tls.clone();
                                        cx.notify();
                                    }),
                                ))
                                .child(self.field("Connection timeout (seconds)", &self.timeout))
                        })
                        .child(self.field("Notes", &self.notes))
                        .child(self.field("Color (#RRGGBB, optional)", &self.color)),
                )
            });
        v_flex()
            .gap_4()
            .child(
                div().text_sm().line_height(relative(1.5)).text_color(cx.theme().muted_foreground)
                    .child("Choose a database type, then enter its details. Everything is saved locally."),
            )
            .child(
                TabBar::new("connection-provider")
                    .segmented()
                    .selected_index(usize::from(self.sqlite))
                    .child(
                        Tab::new()
                            .flex_1()
                            .aria_label("PostgreSQL")
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(Icon::new(IconName::Database).small())
                                    .child("PostgreSQL"),
                            )
                            .disabled(self.busy || self.existing),
                    )
                    .child(
                        Tab::new()
                            .flex_1()
                            .aria_label("SQLite")
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(Icon::new(IconName::File).small())
                                    .child("SQLite"),
                            )
                            .disabled(self.busy || self.existing),
                    )
                    .on_click(cx.listener(|this, index, _, cx| {
                        this.sqlite = *index == 1;
                        if this.sqlite && this.policy == ExecutionPolicy::ReadOnlyByDefault {
                            this.policy = ExecutionPolicy::Normal;
                        }
                        this.error = false;
                        this.field_error = None;
                        this.message.clear();
                        cx.notify();
                    })),
            )
            .child(form)
            .when(!self.message.is_empty(), |view| {
                view.child(
                    v_flex()
                        .id("connection-feedback")
                        .role(if self.error { Role::Alert } else { Role::Status })
                        .gap_2().p_3().rounded(cx.theme().radius)
                        .bg(cx.theme().muted)
                        .text_sm().line_height(relative(1.5))
                        .text_color(if self.error {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(h_flex().items_start().gap_2()
                            .child(div().flex_shrink_0().child(Icon::new(if self.error { IconName::CircleAlert } else if self.busy { IconName::RefreshCw } else { IconName::Check }).small()))
                            .child(div().flex_1().min_w_0().whitespace_normal().child(self.message.clone()))),
                )
            })
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::ConnectionForm;
    use crate::{database::runtime, storage::Store, ui::test_support::click_label};
    use crate::{
        model::{Environment, ExecutionPolicy},
        ui::Services,
    };
    use gpui_kit::test::{TestAppContextExt, TestWindowExt};
    use gpui_kit::{AppContext, TestAppContext, WindowOptions};

    #[gpui_kit::test]
    async fn safeguards_form_labels_and_policy_are_independent(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let directory = tempfile::tempdir().unwrap();
        let services = Services::new(
            runtime().unwrap(),
            Store::open(directory.path().join("app.db")).unwrap(),
        );
        let (window, form) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| ConnectionForm::new(services, None, None, window, cx))
            })
            .unwrap()
        });
        cx.update_window(window, |_, window, cx| {
            window.click("connection-environment", cx);
            click_label(window, "Production", cx);
            assert_eq!(form.read(cx).environment, Some(Environment::Production));
            assert_eq!(form.read(cx).policy, ExecutionPolicy::Normal);
            window.click("connection-policy", cx);
            click_label(window, "Confirm executions", cx);
            assert_eq!(form.read(cx).environment, Some(Environment::Production));
            assert_eq!(form.read(cx).policy, ExecutionPolicy::ConfirmExecutions);
            form.update(cx, |form, cx| {
                form.name.update(cx, |name, cx| {
                    name.set_value("Production fixture", window, cx)
                })
            });
            let profile = form.read(cx).config(cx).unwrap();
            assert_eq!(profile.environment, Some(Environment::Production));
            assert_eq!(profile.policy, ExecutionPolicy::ConfirmExecutions);
        })
        .unwrap();
        cx.wait_for(window, std::time::Duration::from_secs(5), |window, _| {
            window.try_find("popup-menu").is_none()
        })
        .await;
        cx.update_window(window, |_, window, cx| {
            window.click("connection-policy", cx);
            click_label(window, "PostgreSQL Read-only by default", cx);
            assert!(form.read(cx).config(cx).unwrap().blocks_mutations());
        })
        .unwrap();
    }
}
