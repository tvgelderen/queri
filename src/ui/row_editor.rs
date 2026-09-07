use super::{Services, controls::choice, input, task};
use crate::{database::Session, model::*};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Value,
    Null,
    Default,
}

pub struct RowSaved;
impl EventEmitter<RowSaved> for RowEditor {}

pub struct RowEditor {
    services: Services,
    session: Arc<Session>,
    table: TableRef,
    metadata: TableMetadata,
    original: Option<Vec<Cell>>,
    fields: Vec<Entity<gpui_kit::component::input::InputState>>,
    modes: Vec<Mode>,
    busy: bool,
    error: String,
    field_error: Option<(usize, String)>,
}

impl RowEditor {
    pub fn new(
        services: Services,
        session: Arc<Session>,
        table: TableRef,
        metadata: TableMetadata,
        original: Option<Vec<Cell>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fields = metadata
            .columns
            .iter()
            .enumerate()
            .map(|(i, _)| {
                input(
                    original
                        .as_ref()
                        .and_then(|row| row.get(i))
                        .and_then(Cell::optional_text)
                        .unwrap_or_default(),
                    window,
                    cx,
                )
            })
            .collect();
        let modes = metadata
            .columns
            .iter()
            .enumerate()
            .map(
                |(i, _)| match original.as_ref().and_then(|row| row.get(i)) {
                    Some(Cell::Null) => Mode::Null,
                    Some(_) => Mode::Value,
                    None => Mode::Default,
                },
            )
            .collect();
        Self {
            services,
            session,
            table,
            metadata,
            original,
            fields,
            modes,
            busy: false,
            error: String::new(),
            field_error: None,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let sqlite = matches!(self.session.profile.config, ConnectionConfig::Sqlite { .. });
        self.field_error = None;
        self.error.clear();
        let values = self
            .metadata
            .columns
            .iter()
            .enumerate()
            .map(|(i, column)| {
                Ok(match self.modes[i] {
                    Mode::Default => FieldValue::Default,
                    Mode::Null => FieldValue::Null,
                    Mode::Value => FieldValue::Value(
                        parse_field(
                            &self.fields[i].read(cx).value(),
                            column,
                            sqlite,
                            self.original.as_ref().and_then(|row| row.get(i)),
                        )
                        .map_err(|error| (i, error))?,
                    ),
                })
            })
            .collect::<Result<Vec<_>, (usize, anyhow::Error)>>();
        let values = match values {
            Ok(values) => values,
            Err((index, error)) => {
                self.field_error = Some((index, format!("{error:#}")));
                self.fields[index]
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.notify();
                return;
            }
        };
        let change = match &self.original {
            Some(original) => RowChange::Update {
                original: original.clone(),
                values,
            },
            None => RowChange::Insert(values),
        };
        let session = self.session.clone();
        let table = self.table.clone();
        self.busy = true;
        self.error.clear();
        task(
            &self.services,
            window,
            cx,
            async move { session.change_rows(table, change).await },
            |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok(_) => {
                        cx.emit(RowSaved);
                        window.close_dialog(cx);
                    }
                    Err(error) => {
                        this.error = format!(
                            "Could not save the row: {error:#}\nCheck the field values and try again. If the row changed elsewhere, cancel and refresh the table first."
                        )
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }
}

impl Render for RowEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self
            .metadata
            .columns
            .iter()
            .enumerate()
            .map(|(i, column)| {
                let mode = self.modes[i];
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(column.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{}{}{}",
                                                column.data_type,
                                                if column.primary_key {
                                                    " · Primary key"
                                                } else {
                                                    ""
                                                },
                                                if column.nullable { " · Optional" } else { "" }
                                            )),
                                    ),
                            )
                            .child(choice(
                                Button::new(("field-mode", i))
                                    .small()
                                    .accessibility_label(format!("Value mode for {}", column.name))
                                    .label(if column.generated {
                                        "Generated"
                                    } else {
                                        match mode {
                                            Mode::Value => "Value",
                                            Mode::Null => "NULL",
                                            Mode::Default => "Default",
                                        }
                                    })
                                    .disabled(self.busy || column.generated),
                                mode,
                                [
                                    (Mode::Value, "Value".into()),
                                    (Mode::Null, "NULL".into()),
                                    (Mode::Default, "Default".into()),
                                ]
                                .into_iter()
                                .filter(|(mode, _)| *mode != Mode::Null || column.nullable),
                                cx.listener(move |this, mode, _, cx| {
                                    this.modes[i] = *mode;
                                    cx.notify();
                                }),
                            )),
                    )
                    .when_some(
                        self.field_error.as_ref().filter(|(index, _)| *index == i),
                        |view, (_, message)| {
                            view.child(
                                div()
                                    .id(("row-field-error", i))
                                    .role(Role::Alert)
                                    .text_sm()
                                    .text_color(cx.theme().danger)
                                    .child(message.clone()),
                            )
                        },
                    )
                    .child(
                        gpui_kit::component::input::Input::new(&self.fields[i])
                            .id(("row-field", i))
                            .aria_label(column.name.clone())
                            .disabled(
                                self.busy || column.generated || !matches!(mode, Mode::Value),
                            ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        v_flex().gap_4()
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Choose Value, NULL, or Default for each field. Binary SQLite values use hexadecimal."))
            .child(div().id("row-fields").max_h((window.viewport_size().height - px(280.)).clamp(px(120.),px(470.))).child(v_flex().gap_4().children(rows)).overflow_y_scrollbar())
            .when(!self.error.is_empty(), |view| view.child(div().id("row-save-error").role(Role::Alert).text_sm().text_color(cx.theme().danger).child(self.error.clone())))
            .child(h_flex().gap_2().justify_end()
                .child(Button::new("cancel-row").label("Cancel").disabled(self.busy).on_click(|_, window, cx| window.close_dialog(cx)))
                .child(Button::new("save-row").primary().label(if self.original.is_some(){"Save changes"}else{"Insert row"}).loading(self.busy).disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.save(window,cx)))))
    }
}
