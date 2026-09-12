use super::*;
use crate::parameters::{Definition, ParameterType, Parameters};

struct ParameterField {
    definition: Definition,
    input: Entity<InputState>,
    null: bool,
}
struct ParameterForm {
    fields: Vec<ParameterField>,
    postgres: bool,
    error: String,
}
impl ParameterForm {
    fn values(&self, cx: &App) -> anyhow::Result<Vec<Cell>> {
        use anyhow::Context;
        self.fields
            .iter()
            .map(|field| {
                field
                    .definition
                    .kind
                    .parse(&field.input.read(cx).value(), field.null)
                    .with_context(|| format!("Parameter {}", field.definition.name))
            })
            .collect()
    }
}
impl Render for ParameterForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let help = if self.postgres {
            "PostgreSQL infers types. Use SQL casts where needed. Values stay in memory."
        } else {
            "Choose a SQLite value type. Empty text and NULL are different. Values stay in memory."
        };
        let fields = self.fields.iter().enumerate().map(|(index, field)| {
            h_flex()
                .gap_2()
                .child(div().w(px(100.)).child(field.definition.name.clone()))
                .when(!self.postgres, |row| {
                    row.child(choice(
                        Button::new(("parameter-type", index))
                            .label(format!("Type: {:?}", field.definition.kind)),
                        field.definition.kind,
                        vec![
                            (ParameterType::Text, "Text".into()),
                            (ParameterType::Integer, "Integer".into()),
                            (ParameterType::Real, "Real".into()),
                            (ParameterType::Blob, "Blob (hex)".into()),
                        ],
                        cx.listener(move |this, kind, _, cx| {
                            this.fields[index].definition.kind = *kind;
                            cx.notify();
                        }),
                    ))
                })
                .child(
                    Button::new(("parameter-null", index))
                        .label("NULL")
                        .selected(field.null)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.fields[index].null = !this.fields[index].null;
                            cx.notify();
                        })),
                )
                .child(
                    Input::new(&field.input)
                        .id(SharedString::from(format!("parameter-value-{index}")))
                        .disabled(field.null)
                        .aria_label(format!("Value for {}", field.definition.name)),
                )
        });
        v_flex()
            .gap_3()
            .child(help)
            .child(
                v_flex()
                    .id("parameter-fields")
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .gap_2()
                    .children(fields),
            )
            .child(
                div()
                    .id("parameter-error")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(self.error.clone())
                    .text_color(cx.theme().danger)
                    .child(self.error.clone()),
            )
    }
}

impl Document {
    pub(in crate::ui) fn set_parameter_definitions(&mut self, definitions: Vec<Definition>) {
        self.parameter_definitions = definitions;
    }
    pub(super) fn submission_snapshot(&self, cx: &App) -> Option<(String, String)> {
        self.sql_editor().map(|editor| {
            let editor = editor.read(cx);
            (
                editor.value().to_string(),
                editor.selected_value().to_string(),
            )
        })
    }
    pub(super) fn parameter_dialect(&self) -> sql::SqlDialect {
        if matches!(self.profile.config, ConnectionConfig::Sqlite { .. }) {
            sql::SqlDialect::Sqlite
        } else {
            sql::SqlDialect::Postgres
        }
    }
    pub(super) fn parameter_definitions(&self, cx: &App) -> Vec<Definition> {
        let Some(editor) = self.sql_editor() else {
            return vec![];
        };
        let previous = if self.parameter_definitions.is_empty() {
            self.saved_query
                .as_ref()
                .map(|q| q.parameters.as_slice())
                .unwrap_or(&[])
        } else {
            &self.parameter_definitions
        };
        crate::parameters::discover(&editor.read(cx).value(), self.parameter_dialect())
            .map(|p| p.definitions(previous))
            .unwrap_or_else(|_| previous.to_vec())
    }
    pub(super) fn enter_parameters(
        &mut self,
        sql: String,
        parameters: Parameters,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let definitions = parameters.definitions(&self.parameter_definitions(cx));
        let postgres = matches!(self.parameter_dialect(), sql::SqlDialect::Postgres);
        let seed = self.parameter_seed.take().filter(|seed| seed.sql == sql);
        let form = cx.new(|cx| ParameterForm {
            fields: definitions
                .into_iter()
                .enumerate()
                .map(|(index, mut definition)| {
                    let value = seed.as_ref().and_then(|s| s.values.get(index));
                    if postgres {
                        definition.kind = ParameterType::Text;
                    } else if let Some(value) = value {
                        definition.kind = ParameterType::for_cell(value);
                    }
                    let text = match value {
                        Some(Cell::Blob(bytes)) => {
                            bytes.iter().map(|b| format!("{b:02x}")).collect()
                        }
                        Some(Cell::Null) | None => String::new(),
                        Some(value) => value.text(),
                    };
                    ParameterField {
                        definition,
                        input: cx.new(|cx| InputState::new(window, cx).default_value(text)),
                        null: matches!(value, Some(Cell::Null)),
                    }
                })
                .collect(),
            postgres,
            error: String::new(),
        });
        let view = cx.entity();
        let generation = self.session_generation;
        let editor_sql = self.submission_snapshot(cx);
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let (view, form, sql, editor_sql) =
                (view.clone(), form.clone(), sql.clone(), editor_sql.clone());
            let cancel_view = view.clone();
            dialog
                .on_cancel(move |_, window, cx| {
                    cancel_view.update(cx, |this, cx| this.focus(window, cx));
                    true
                })
                .title("Query parameters")
                .child(form.clone())
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Continue")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let values = form.read(cx).values(cx);
                    match values {
                        Err(error) => {
                            form.update(cx, |form, cx| {
                                form.error = format!("{error:#}");
                                cx.notify();
                            });
                            false
                        }
                        Ok(values) => {
                            let definitions: Vec<Definition> = form
                                .read(cx)
                                .fields
                                .iter()
                                .map(|f| f.definition.clone())
                                .collect();
                            // Close this dialog before the shared boundary can open its confirmation.
                            window.close_dialog(cx);
                            view.update(cx, |this, cx| {
                                if this.session_generation == generation
                                    && this.submission_snapshot(cx) == editor_sql
                                {
                                    let mut all_definitions = this.parameter_definitions(cx);
                                    for definition in definitions {
                                        if let Some(existing) = all_definitions
                                            .iter_mut()
                                            .find(|d| d.name == definition.name)
                                        {
                                            *existing = definition;
                                        }
                                    }
                                    this.parameter_definitions = all_definitions;
                                    cx.emit(DocumentChanged);
                                    this.focus(window, cx);
                                    this.submit_sql(sql.clone(), Some(values), window, cx);
                                }
                            });
                            false
                        }
                    }
                })
        });
    }
}
