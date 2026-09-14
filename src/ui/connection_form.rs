use super::{
    Services,
    controls::{choice, disclosure_button, setting_switch},
    input, task,
};
use crate::model::*;
use anyhow::Result;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    form::Field,
    h_flex,
    input::{Input, InputEvent, InputState},
    switch::Switch,
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
    ssh_enabled: bool,
    ssh_host: Entity<InputState>,
    ssh_port: Entity<InputState>,
    ssh_user: Entity<InputState>,
    ssh_key: Entity<InputState>,
    ssh_passphrase: Entity<InputState>,
    ssh_remember: bool,
    ssh_agent: bool,
    ssh_identity: String,
    ssh_identities: Vec<String>,
    test_cancel: Option<futures::future::AbortHandle>,
    key_subscription: Option<Subscription>,
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

fn hinted(
    value: String,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}

impl ConnectionForm {
    pub fn new(
        services: Services,
        existing: Option<ConnectionProfile>,
        cached_password: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut form = Self {
            ssh_enabled: existing.as_ref().is_some_and(|p| p.ssh.is_some()),
            ssh_host: hinted(
                existing
                    .as_ref()
                    .and_then(|p| p.ssh.as_ref())
                    .map(|s| s.host.clone())
                    .unwrap_or_default(),
                "bastion.example.com",
                window,
                cx,
            ),
            ssh_port: input(
                existing
                    .as_ref()
                    .and_then(|p| p.ssh.as_ref())
                    .map(|s| s.port)
                    .unwrap_or(22)
                    .to_string(),
                window,
                cx,
            ),
            ssh_user: hinted(
                existing
                    .as_ref()
                    .and_then(|p| p.ssh.as_ref())
                    .map(|s| s.user.clone())
                    .unwrap_or_default(),
                "user",
                window,
                cx,
            ),
            ssh_key: hinted(
                existing
                    .as_ref()
                    .and_then(|p| p.ssh.as_ref())
                    .and_then(|s| {
                        if let SshAuth::PrivateKey { path } = &s.auth {
                            Some(path.to_string_lossy().to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default(),
                "/home/you/.ssh/id_ed25519",
                window,
                cx,
            ),
            ssh_passphrase: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Optional")
            }),
            ssh_remember: existing
                .as_ref()
                .and_then(|p| p.ssh.as_ref())
                .is_some_and(|s| s.remember_passphrase),
            ssh_agent: existing
                .as_ref()
                .and_then(|p| p.ssh.as_ref())
                .is_some_and(|s| matches!(s.auth, SshAuth::Agent { .. })),
            ssh_identity: existing
                .as_ref()
                .and_then(|p| p.ssh.as_ref())
                .and_then(|s| {
                    if let SshAuth::Agent { fingerprint } = &s.auth {
                        Some(fingerprint.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_default(),
            ssh_identities: vec![],
            test_cancel: None,
            key_subscription: None,
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
        form.key_subscription = Some(cx.subscribe_in(
            &form.ssh_key,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) && !this.busy {
                    this.ssh_passphrase
                        .update(cx, |state, cx| state.set_value("", window, cx));
                    this.services.connector.clear_passphrase(this.id);
                }
            },
        ));
        if form.ssh_remember {
            let store = form.services.store.clone();
            let id = form.id;
            let expected_key = form.ssh_key.read(cx).value().to_string();
            task(
                &form.services,
                window,
                cx,
                async move { tokio::task::spawn_blocking(move || store.load_ssh_passphrase(id)).await? },
                move |this, result, window, cx| {
                    match result {
                        Ok(Some(passphrase))
                            if this.ssh_remember
                                && !this.ssh_agent
                                && this.ssh_key.read(cx).value().as_str() == expected_key
                                && this.ssh_passphrase.read(cx).value().is_empty() =>
                        {
                            this.ssh_passphrase
                                .update(cx, |state, cx| state.set_value(passphrase, window, cx))
                        }
                        Err(error) => {
                            this.error = true;
                            this.message = format!("Could not read SSH passphrase: {error:#}");
                        }
                        _ => {}
                    }
                    cx.notify();
                },
            );
        }
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
            ssh: if self.ssh_enabled && !self.sqlite {
                Some(SshConfig {
                    host: self.ssh_host.read(cx).value().to_string(),
                    port: self
                        .ssh_port
                        .read(cx)
                        .value()
                        .parse()
                        .map_err(|_| anyhow::anyhow!("SSH port must be between 1 and 65535"))?,
                    user: self.ssh_user.read(cx).value().to_string(),
                    auth: if self.ssh_agent {
                        SshAuth::Agent {
                            fingerprint: self.ssh_identity.clone(),
                        }
                    } else {
                        SshAuth::PrivateKey {
                            path: self.ssh_key.read(cx).value().to_string().into(),
                        }
                    },
                    remember_passphrase: self.ssh_remember && !self.ssh_agent,
                })
            } else {
                None
            },
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
        let passphrase = self.ssh_passphrase.read(cx).value().to_string();
        let connector = self.services.connector.clone();
        let (progress, mut stages) = tokio::sync::watch::channel("Starting connection test…");
        if !save {
            cx.spawn_in(window, async move |view, cx| {
                while stages.changed().await.is_ok() {
                    let message = *stages.borrow_and_update();
                    if view
                        .update_in(cx, |this, _, cx| {
                            if this.busy {
                                this.message = message.into();
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
        let (abort, cancellation) = futures::future::AbortHandle::new_pair();
        if !save {
            self.test_cancel = Some(abort);
        }
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
                futures::future::Abortable::new(
                    async move {
                        if save {
                            if let ConnectionConfig::Sqlite { path, create, .. } =
                                &mut profile.config
                            {
                                if creating {
                                    crate::database::create_sqlite_file(path.clone()).await?;
                                }
                                *create = false;
                            }
                            let profile_copy = profile.clone();
                            let password_copy = password.clone();
                            let passphrase_copy = passphrase.clone();
                            tokio::task::spawn_blocking(move || {
                                store.save_connection_with_ssh(
                                    &profile_copy,
                                    &password_copy,
                                    Some(&passphrase_copy),
                                )
                            })
                            .await??;
                            connector.invalidate(profile.id);
                            if profile
                                .ssh
                                .as_ref()
                                .is_some_and(|s| matches!(s.auth, SshAuth::PrivateKey { .. }))
                            {
                                connector.supply_passphrase(profile.id, passphrase);
                            }
                        } else {
                            connector
                                .test_progress(
                                    profile.clone(),
                                    password.clone(),
                                    passphrase,
                                    Some(progress),
                                )
                                .await?;
                        }
                        Ok(SavedConnection { profile, password })
                    },
                    cancellation,
                )
                .await
                .map_err(|_| anyhow::anyhow!("Connection test cancelled"))?
            },
            move |this, result, window, cx| {
                this.busy = false;
                this.test_cancel = None;
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

    fn clear_passphrase(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.ssh_passphrase
            .update(cx, |state, cx| state.set_value("", window, cx));
        let store = self.services.store.clone();
        let id = self.id;
        self.services.connector.clear_passphrase(id);
        task(
            &self.services,
            window,
            cx,
            async move { tokio::task::spawn_blocking(move || store.clear_ssh_passphrase(id)).await? },
            |this, result, _, cx| {
                this.busy = false;
                if let Err(error) = result {
                    this.error = true;
                    this.message = format!("Could not clear saved SSH passphrase: {error:#}");
                }
                cx.notify();
            },
        );
    }

    fn browse_ssh_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose SSH private key".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = receiver.await;
            let _ = view.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            this.ssh_key.update(cx, |state, cx| {
                                state.set_value(path.to_string_lossy().to_string(), window, cx)
                            });
                        }
                    }
                    Ok(Err(error)) => {
                        this.error = true;
                        this.message =
                            format!("File picker failed: {error}. Enter a path manually.");
                    }
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        task(
            &self.services,
            window,
            cx,
            crate::ssh::agent_identities(),
            |this, result, _, cx| {
                match result {
                    Ok(identities) => {
                        this.ssh_identities = identities;
                        this.message = if this.ssh_identities.is_empty() {
                            "No public key identities in SSH_AUTH_SOCK.".into()
                        } else {
                            "Choose an SSH agent identity.".into()
                        };
                    }
                    Err(error) => {
                        this.error = true;
                        this.message = format!("{error:#}");
                    }
                }
                cx.notify();
            },
        );
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

    fn forget_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let endpoint = SshConfig {
            host: self.ssh_host.read(cx).value().to_string(),
            port: self.ssh_port.read(cx).value().parse().unwrap_or(22),
            user: String::new(),
            auth: SshAuth::Agent {
                fingerprint: String::new(),
            },
            remember_passphrase: false,
        }
        .endpoint();
        let connector = self.services.connector.clone();
        task(
            &self.services,
            window,
            cx,
            async move { tokio::task::spawn_blocking(move || connector.forget_host(&endpoint)).await? },
            |this, result, _, cx| {
                this.error = result.is_err();
                this.message = result
                    .map(|_| {
                        "SSH host trust removed. The next handshake requires verification.".into()
                    })
                    .unwrap_or_else(|e| format!("{e:#}"));
                cx.notify();
            },
        );
    }

    fn set_ssh_enabled(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.ssh_enabled = on;
        if !on {
            self.ssh_remember = false;
            self.clear_passphrase(window, cx);
        }
        cx.notify();
    }

    /// The tunnel lives in one card so its settings read as a single, optional route.
    fn ssh_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let header = h_flex()
            .id("ssh-section-header")
            .test_support()
            .w_full()
            .gap_3()
            .px_4()
            .py_3()
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.set_ssh_enabled(!this.ssh_enabled, window, cx);
            }))
            .child(
                div()
                    .flex_shrink_0()
                    .size_8()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme.radius)
                    .bg(if self.ssh_enabled {
                        super::theme::accent_soft(cx)
                    } else {
                        theme.muted
                    })
                    .child(Icon::new(IconName::Waypoints).small().text_color(
                        if self.ssh_enabled {
                            super::theme::accent_text(cx)
                        } else {
                            theme.muted_foreground
                        },
                    )),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Connect through SSH"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .line_height(relative(1.4))
                            .text_color(theme.muted_foreground)
                            .child(
                                "Tunnel through a bastion. The database host above is resolved from the bastion.",
                            ),
                    ),
            )
            .child(
                div().flex_shrink_0().child(
                    Switch::new("ssh-enabled")
                        .accessibility_label("Connect through SSH")
                        .checked(self.ssh_enabled)
                        .disabled(self.busy)
                        .on_change(cx.listener(|this, on: &bool, window, cx| {
                            cx.stop_propagation();
                            this.set_ssh_enabled(*on, window, cx);
                        })),
                ),
            );
        let body = self.ssh_enabled.then(|| {
            let auth = TabBar::new("ssh-auth")
                .segmented()
                .selected_index(usize::from(self.ssh_agent))
                .child(
                    Tab::new().flex_1().aria_label("Private key").child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(IconName::FileKey).small())
                            .child("Private key"),
                    ),
                )
                .child(
                    Tab::new().flex_1().aria_label("SSH agent").child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(IconName::KeyRound).small())
                            .child("SSH agent"),
                    ),
                )
                .on_click(cx.listener(|this, index, window, cx| {
                    if this.busy {
                        return;
                    }
                    this.ssh_agent = *index == 1;
                    this.ssh_remember = false;
                    this.clear_passphrase(window, cx);
                    if this.ssh_agent {
                        this.load_agent(window, cx);
                    }
                    cx.notify();
                }));
            let credentials = if self.ssh_agent {
                let identity = Field::new()
                    .label("Agent identity")
                    .description(
                        "Queri signs with this identity only and never forwards the agent.",
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div().flex_1().min_w_0().child(choice(
                                    Button::new("ssh-agent-identity")
                                        .outline()
                                        .w_full()
                                        .disabled(self.busy)
                                        .child(
                                            h_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .gap_2()
                                                .child(
                                                    Icon::new(IconName::FingerprintPattern)
                                                        .small()
                                                        .text_color(theme.muted_foreground),
                                                )
                                                .child(
                                                    div()
                                                        .min_w_0()
                                                        .text_ellipsis()
                                                        .when(
                                                            self.ssh_identity.is_empty(),
                                                            |label| {
                                                                label
                                                                    .text_color(
                                                                        theme.muted_foreground,
                                                                    )
                                                                    .child("Choose an identity")
                                                            },
                                                        )
                                                        .when(
                                                            !self.ssh_identity.is_empty(),
                                                            |label| {
                                                                label
                                                                    .font_family(
                                                                        theme
                                                                            .mono_font_family
                                                                            .clone(),
                                                                    )
                                                                    .text_xs()
                                                                    .child(
                                                                        self.ssh_identity.clone(),
                                                                    )
                                                            },
                                                        ),
                                                ),
                                        ),
                                    self.ssh_identity.clone(),
                                    self.ssh_identities
                                        .iter()
                                        .map(|fp| (fp.clone(), fp.clone().into())),
                                    cx.listener(|this, fp: &String, _, cx| {
                                        if this.busy {
                                            return;
                                        }
                                        this.ssh_identity = fp.clone();
                                        cx.notify();
                                    }),
                                )),
                            )
                            .child(
                                Button::new("ssh-agent-refresh")
                                    .outline()
                                    .icon(IconName::RefreshCw)
                                    .accessibility_label("Refresh agent identities")
                                    .tooltip("Refresh agent identities")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.load_agent(window, cx)
                                    })),
                            ),
                    );
                v_flex().gap_4().child(identity)
            } else {
                v_flex()
                    .gap_4()
                    .child(self.field_with_content(
                        "SSH private key path",
                        &self.ssh_key,
                        |input| {
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().min_w_0().child(input))
                                .child(
                                    Button::new("ssh-key-picker")
                                        .icon(IconName::FolderOpen)
                                        .label("Browse…")
                                        .disabled(self.busy)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.browse_ssh_key(window, cx)
                                        })),
                                )
                        },
                    ))
                    .child(
                        self.field_with_content(
                            "SSH key passphrase",
                            &self.ssh_passphrase,
                            |input| input.mask_toggle(),
                        )
                        .when(self.field_error.is_none(), |field| {
                            field.description(
                                "Leave empty for unencrypted keys. Queri asks when one is needed.",
                            )
                        }),
                    )
                    .child(setting_switch(
                        "ssh-remember-passphrase",
                        "Remember SSH passphrase",
                        self.ssh_remember,
                        cx.listener(|this, on, window, cx| {
                            if this.busy {
                                return;
                            }
                            this.ssh_remember = *on;
                            if !on {
                                this.clear_passphrase(window, cx);
                            }
                            cx.notify();
                        }),
                    ))
            };
            v_flex()
                .child(
                    v_flex()
                        .gap_4()
                        .p_4()
                        .border_t_1()
                        .border_color(theme.border)
                        .child(
                            h_flex()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(148.))
                                        .child(self.field("SSH user", &self.ssh_user)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(self.field("SSH host", &self.ssh_host)),
                                )
                                .child(
                                    div()
                                        .w(px(88.))
                                        .child(self.field("SSH port", &self.ssh_port)),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("Authentication"),
                                )
                                .child(auth),
                        )
                        .child(credentials),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .justify_between()
                        .pl_4()
                        .pr_2()
                        .py_1p5()
                        .border_t_1()
                        .border_color(theme.border)
                        .bg(theme.muted)
                        .rounded_b(theme.radius_lg)
                        .child(
                            h_flex()
                                .min_w_0()
                                .gap_2()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(Icon::new(IconName::ShieldCheck).xsmall())
                                .child(
                                    div()
                                        .min_w_0()
                                        .text_ellipsis()
                                        .child("Host keys are verified before authentication."),
                                ),
                        )
                        .child(
                            Button::new("ssh-forget-host")
                                .ghost()
                                .xsmall()
                                .icon(IconName::ShieldOff)
                                .label("Forget trust for this SSH host")
                                .disabled(self.busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.forget_host(window, cx)),
                                ),
                        ),
                )
        });
        v_flex()
            .id("ssh-section")
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(if self.ssh_enabled {
                theme.input
            } else {
                theme.border
            })
            .child(header)
            .children(body)
    }

    pub fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("cancel-connection")
                    .label("Cancel")
                    .disabled(self.busy && self.test_cancel.is_none())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(abort) = this.test_cancel.take() {
                            abort.abort();
                            this.message = "Cancelling connection test…".into();
                            cx.notify();
                        } else {
                            window.close_dialog(cx);
                        }
                    })),
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
        if !self.sqlite {
            form = form.child(self.ssh_section(cx));
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
    use gpui_kit::{AppContext, Bounds, TestAppContext, WindowBounds, WindowOptions, px, size};

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
    #[gpui_kit::test]
    async fn ssh_form_defaults_and_credentials_follow_authentication(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("app.db")).unwrap();
        let services = Services::new(runtime().unwrap(), store.clone());
        let (window, form) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1000.), px(1800.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| ConnectionForm::new(services, None, None, window, cx)),
            )
            .unwrap()
        });
        cx.update_window(window, |_, window, cx| {
            form.update(cx, |form, cx| {
                form.name
                    .update(cx, |s, cx| s.set_value("SSH connection", window, cx))
            });
            assert!(form.read(cx).config(cx).unwrap().ssh.is_none());
            assert!(!form.read(cx).ssh_remember);
            window.click("ssh-enabled", cx);
            form.update(cx, |form, cx| {
                for (input, value) in [
                    (&form.ssh_host, "bastion"),
                    (&form.ssh_user, "tunnel"),
                    (&form.ssh_key, "/keys/id_ed25519"),
                ] {
                    input.update(cx, |s, cx| s.set_value(value, window, cx));
                }
            });
        })
        .unwrap();
        cx.wait_for(window, std::time::Duration::from_secs(5), |_, cx| {
            !form.read(cx).busy()
        })
        .await;
        cx.update_window(window, |_, window, cx| {
            form.update(cx, |form, cx| {
                form.ssh_passphrase
                    .update(cx, |s, cx| s.set_value("private-secret", window, cx))
            });
            let profile = form.read(cx).config(cx).unwrap();
            assert_eq!(profile.ssh.as_ref().unwrap().port, 22);
            assert!(
                !serde_json::to_string(&profile)
                    .unwrap()
                    .contains("private-secret")
            );
            window.click("ssh-remember-passphrase", cx);
            assert!(
                form.read(cx)
                    .config(cx)
                    .unwrap()
                    .ssh
                    .unwrap()
                    .remember_passphrase
            );
            store
                .save_connection_with_ssh(
                    &form.read(cx).config(cx).unwrap(),
                    "db",
                    Some("private-secret"),
                )
                .unwrap();
            window.click("ssh-remember-passphrase", cx);
            assert!(form.read(cx).ssh_passphrase.read(cx).value().is_empty());
        })
        .unwrap();
        let id = form.read_with(cx, |f, _| f.id);
        cx.wait_for(window, std::time::Duration::from_secs(5), |_, cx| {
            !form.read(cx).busy() && store.load_ssh_passphrase(id).unwrap().is_none()
        })
        .await;
        cx.update_window(window, |_, window, cx| {
            window.click("ssh-enabled", cx);
            assert!(form.read(cx).config(cx).unwrap().ssh.is_none());
        })
        .unwrap();
        cx.wait_for(window, std::time::Duration::from_secs(5), |_, cx| {
            !form.read(cx).busy()
        })
        .await;
        cx.update_window(window, |_, window, cx| {
            window.click("ssh-section-header", cx);
            assert!(
                form.read(cx).ssh_enabled,
                "the whole card header toggles SSH once"
            );
        })
        .unwrap();
    }
    #[gpui_kit::test]
    async fn ssh_connection_test_cancel_closes_socket_and_does_not_save(cx: &mut TestAppContext) {
        use gpui_kit::{ParentElement, component::WindowExt};
        use tokio::io::AsyncReadExt;
        cx.executor().allow_parking();
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("app.db")).unwrap();
        let services = Services::new(runtime().unwrap(), store.clone());
        let listener = services
            .runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let socket_closed = services.runtime.spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 256];
            tokio::time::timeout(std::time::Duration::from_secs(5), async move {
                while socket.read(&mut buffer).await.unwrap_or(0) > 0 {}
            })
            .await
            .unwrap();
        });
        let (window, form) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| ConnectionForm::new(services, None, None, window, cx))
            })
            .unwrap()
        });
        cx.update_window(window, |_, window, cx| {
            form.update(cx, |form, cx| {
                form.name
                    .update(cx, |s, cx| s.set_value("Cancelled SSH test", window, cx));
                form.ssh_enabled = true;
                form.ssh_agent = true;
                form.ssh_identity = "SHA256:fixture".into();
                for (input, value) in [
                    (&form.ssh_host, "127.0.0.1".to_owned()),
                    (&form.ssh_port, port.to_string()),
                    (&form.ssh_user, "fixture".into()),
                ] {
                    input.update(cx, |s, cx| s.set_value(value, window, cx));
                }
            });
            let form = form.clone();
            window.open_dialog(cx, move |dialog, _, cx| {
                dialog
                    .footer(form.update(cx, |f, cx| f.render_footer(cx)))
                    .child(form.clone())
            });
            window.click("test-connection", cx);
        })
        .unwrap();
        cx.wait_for(window, std::time::Duration::from_secs(5), |_, cx| {
            form.read(cx).message.contains("SSH handshake")
        })
        .await;
        cx.update_window(window, |_, window, cx| {
            window.click("cancel-connection", cx);
        })
        .unwrap();
        cx.wait_for(window, std::time::Duration::from_secs(5), |_, cx| {
            !form.read(cx).busy()
        })
        .await;
        assert!(form.read_with(cx, |form, _| form.message.contains("cancelled")));
        assert!(store.profiles().unwrap().is_empty());
        socket_closed.await.unwrap();
    }
}

impl Drop for ConnectionForm {
    fn drop(&mut self) {
        if let Some(abort) = self.test_cancel.take() {
            abort.abort();
        }
    }
}
