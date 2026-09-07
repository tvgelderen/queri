use super::{Services, controls::disclosure_button, field, input, task};
use crate::{database::Session, model::*};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::InputState,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

#[derive(Clone)]
pub enum AdminAction {
    Create,
    Rename(String),
    Clone(String),
    Delete(String),
    Sessions(String),
}

impl AdminAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Create => "Create database",
            Self::Rename(_) => "Rename database",
            Self::Clone(_) => "Clone database",
            Self::Delete(_) => "Delete database",
            Self::Sessions(_) => "Active sessions",
        }
    }

    pub fn source(&self) -> Option<&str> {
        match self {
            Self::Create => None,
            Self::Rename(name) | Self::Clone(name) | Self::Delete(name) | Self::Sessions(name) => {
                Some(name)
            }
        }
    }
}

pub enum AdminEvent {
    Execute(DatabaseAction),
    Completed {
        action: DatabaseAction,
        maintenance: String,
    },
}

pub struct DatabaseAdmin {
    services: Services,
    profile: ConnectionProfile,
    password: String,
    action: AdminAction,
    maintenance: Entity<InputState>,
    name: Entity<InputState>,
    confirmation: Entity<InputState>,
    advanced: bool,
    force: bool,
    busy: bool,
    message: String,
    error: String,
}

impl EventEmitter<AdminEvent> for DatabaseAdmin {}

impl DatabaseAdmin {
    pub fn new(
        services: Services,
        profile: ConnectionProfile,
        password: String,
        action: AdminAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let maintenance = if action.source() == Some("postgres") {
            "template1"
        } else {
            "postgres"
        };
        let name = match &action {
            AdminAction::Rename(source) => source.clone(),
            AdminAction::Clone(source) => format!("{source}_copy"),
            _ => String::new(),
        };
        Self {
            services,
            profile,
            password,
            action,
            maintenance: input(maintenance, window, cx),
            name: input(name, window, cx),
            confirmation: input("", window, cx),
            advanced: false,
            force: false,
            busy: false,
            message: String::new(),
            error: String::new(),
        }
    }

    pub fn connection_id(&self) -> uuid::Uuid {
        self.profile.id
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn show_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.error = message;
        cx.notify();
    }

    pub fn load_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(database) = self.action.source().map(str::to_owned) else {
            return;
        };
        let profile = self
            .profile
            .for_database(&self.maintenance.read(cx).value());
        let password = self.password.clone();
        self.busy = true;
        self.error.clear();
        task(
            &self.services,
            window,
            cx,
            async move {
                Session::connect(profile, password)
                    .await?
                    .database_connections(database)
                    .await
            },
            |this, result, _, cx| {
                this.busy = false;
                match result {
                    Ok(data) => {
                        this.message = if data.rows.is_empty() {
                            "No active sessions.".into()
                        } else {
                            data.rows
                                .iter()
                                .map(|row| {
                                    row.iter().map(Cell::text).collect::<Vec<_>>().join(" · ")
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    }
                    Err(error) => {
                        this.error = format!(
                            "Could not inspect sessions: {error:#}\nCheck the maintenance database and your permissions, then retry."
                        )
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.error.clear();
        let name = self.name.read(cx).value().to_string();
        let maintenance = self.maintenance.read(cx).value();
        if maintenance.trim().is_empty() || self.action.source() == Some(maintenance.as_ref()) {
            self.error =
                "Choose a maintenance database different from the database being changed.".into();
            self.advanced = true;
            self.maintenance.read(cx).focus_handle(cx).focus(window, cx);
        } else if !matches!(
            self.action,
            AdminAction::Delete(_) | AdminAction::Sessions(_)
        ) && (name.trim().is_empty() || name.len() > 63 || name.contains('\0'))
        {
            self.error = "Enter a database name of 1–63 bytes, without null characters.".into();
            self.name.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            let action = match &self.action {
                AdminAction::Create => DatabaseAction::Create { name },
                AdminAction::Rename(source) if source == &name => {
                    self.error = "Enter a different database name.".into();
                    cx.notify();
                    return;
                }
                AdminAction::Rename(source) => DatabaseAction::Rename {
                    source: source.clone(),
                    target: name,
                    force: self.force,
                },
                AdminAction::Clone(source) => DatabaseAction::Clone {
                    source: source.clone(),
                    target: name,
                    force: self.force,
                },
                AdminAction::Delete(source) => {
                    if self.confirmation.read(cx).value().as_ref() != source {
                        self.error = format!("Type {source} exactly to confirm deletion.");
                        self.confirmation
                            .read(cx)
                            .focus_handle(cx)
                            .focus(window, cx);
                        cx.notify();
                        return;
                    }
                    DatabaseAction::Delete {
                        name: source.clone(),
                        force: self.force,
                    }
                }
                AdminAction::Sessions(_) => {
                    self.load_sessions(window, cx);
                    return;
                }
            };
            cx.emit(AdminEvent::Execute(action));
        }
        cx.notify();
    }

    pub fn execute(&mut self, action: DatabaseAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let maintenance = self.maintenance.read(cx).value().to_string();
        let profile = self.profile.for_database(&maintenance);
        let password = self.password.clone();
        let completed = action.clone();
        self.busy = true;
        self.error.clear();
        task(
            &self.services,
            window,
            cx,
            async move {
                Session::connect(profile, password)
                    .await?
                    .database_action(action)
                    .await
            },
            move |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        cx.emit(AdminEvent::Completed {
                            action: completed,
                            maintenance,
                        });
                        window.close_dialog(cx);
                    }
                    Err(error) => {
                        this.error = format!(
                            "Could not complete the operation: {error:#}\nCheck the name, permissions, and active sessions, then retry."
                        );
                        this.advanced = true;
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    pub fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let destructive = matches!(self.action, AdminAction::Delete(_));
        h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("cancel-admin")
                    .label("Cancel")
                    .disabled(self.busy)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                Button::new("database-operation")
                    .label(if matches!(self.action, AdminAction::Sessions(_)) {
                        "Refresh sessions"
                    } else {
                        self.action.label()
                    })
                    .when(destructive, |button| button.danger().icon(IconName::Trash))
                    .when(!destructive, |button| button.primary())
                    .loading(self.busy)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            )
            .into_any_element()
    }
}

impl Render for DatabaseAdmin {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sessions = matches!(self.action, AdminAction::Sessions(_));
        let delete = matches!(self.action, AdminAction::Delete(_));
        v_flex().gap_4()
            .child(h_flex().gap_2().text_sm().text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Database).small())
                .child(format!("{}{}", self.profile.name, self.action.source().map(|name| format!(" / {name}")).unwrap_or_default())))
            .when(delete, |view| view
                .child("This permanently deletes the database and all of its tables and data. This cannot be undone.")
                .child(field(format!("Type {} to confirm", self.action.source().unwrap_or_default()), &self.confirmation)))
            .when(!delete && !sessions, |view| view.child(field("Database name", &self.name)))
            .when(self.action.source().is_some() && !sessions, |view| view.child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child("Queri will disconnect its sessions for this database. Uncommitted transactions will be rolled back; SQL drafts are kept.")))
            .child(disclosure_button("admin-advanced", "Connection options", self.advanced)
                .on_click(cx.listener(|this, _, _, cx| { this.advanced = !this.advanced; cx.notify(); })))
            .when(self.advanced, |view| view.child(v_flex().gap_3()
                .child(field("Maintenance database", &self.maintenance))
                .when(self.action.source().is_some() && !sessions, |view| view
                    .child(Checkbox::new("force").label("Also disconnect sessions from other applications")
                        .checked(self.force).disabled(self.busy)
                        .on_change(cx.listener(|this, on, _, cx| { this.force = *on; cx.notify(); })))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Disconnecting other sessions rolls back their uncommitted work.")))))
            .when(!self.message.is_empty(), |view| view.child(div().id("database-message").role(Role::Status).max_h(px(200.)).overflow_y_scroll().p_3().rounded(cx.theme().radius).bg(cx.theme().muted).text_sm().font_family(cx.theme().mono_font_family.clone()).child(self.message.clone())))
            .when(!self.error.is_empty(), |view| view.child(div().id("database-error").role(Role::Alert).text_sm().text_color(cx.theme().danger).child(self.error.clone())))
    }
}
