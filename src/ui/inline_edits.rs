use super::*;
use crate::row_updates::{PendingEdits, UpdateBatch};

impl Document {
    pub(super) fn selected_data_cell(&self, cx: &App) -> Option<(usize, usize)> {
        let grid = self.grid.read(cx);
        let (row, col) = grid.selected_cell()?;
        let index = col.checked_sub(usize::from(grid.delegate().checkable))?;
        Some((row, *grid.delegate().display_columns.get(index)?))
    }
    pub(super) fn begin_cell_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        if self.grid.read(cx).delegate().editor.is_some() {
            self.accept_cell_edit(window, cx);
            if self.grid.read(cx).delegate().editor.is_some() {
                return;
            }
        }
        let Some((row, column)) = self.selected_data_cell(cx) else {
            return;
        };
        let DocumentKind::Table {
            page: Some(page),
            table,
            section: Section::Data,
            ..
        } = &self.kind
        else {
            return;
        };
        if !page.metadata.editable(table)
            || self.session.as_ref().is_none_or(|s| s.read_only())
            || page.metadata.columns[column].generated
        {
            return;
        }
        let Some(original) = page.data.rows.get(row) else {
            return;
        };
        if crate::row_updates::validate_original(&page.metadata, table, original).is_err() {
            return;
        }
        let value = match self.edits.field(original, column) {
            Some(FieldValue::Value(value)) => value.optional_text().unwrap_or_default(),
            Some(_) => String::new(),
            None => original[column].optional_text().unwrap_or_default(),
        };
        if let DocumentKind::Table { columns_open, .. } = &mut self.kind {
            *columns_open = false;
        }
        self.edit_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        self.grid.update(cx, |grid, cx| {
            grid.delegate_mut().editor = Some((row, column, self.edit_input.clone()));
            cx.notify();
        });
    }
    pub(super) fn accept_cell_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((row, column, _)) = self.grid.read(cx).delegate().editor.clone() else {
            return;
        };
        let DocumentKind::Table {
            page: Some(page), ..
        } = &self.kind
        else {
            return;
        };
        let original = &page.data.rows[row];
        match parse_field(
            &self.edit_input.read(cx).value(),
            &page.metadata.columns[column],
            matches!(self.profile.config, ConnectionConfig::Sqlite { .. }),
            Some(&original[column]),
        ) {
            Ok(value) => {
                self.edits.set(original, column, FieldValue::Value(value));
                self.cancel_cell_edit(window, cx);
                self.sync_edits(cx);
                self.error.clear();
            }
            Err(error) => self.error = format!("{error:#}"),
        }
        cx.notify();
    }
    pub(super) fn stage_mode(
        &mut self,
        value: FieldValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_cell_edit(window, cx);
        let Some((row, column, _)) = self.grid.read(cx).delegate().editor.clone() else {
            return;
        };
        if let DocumentKind::Table {
            page: Some(page), ..
        } = &self.kind
        {
            self.edits.set(&page.data.rows[row], column, value);
        }
        self.cancel_cell_edit(window, cx);
        self.sync_edits(cx);
    }
    pub(super) fn cancel_cell_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.grid.update(cx, |grid, cx| {
            grid.delegate_mut().editor = None;
            grid.focus_handle(cx).focus(window, cx);
            cx.notify();
        });
    }
    pub(super) fn sync_edits(&mut self, cx: &mut Context<Self>) {
        let mut cells = std::collections::BTreeMap::new();
        if let DocumentKind::Table {
            page: Some(page), ..
        } = &self.kind
        {
            for (row, original) in page.data.rows.iter().enumerate() {
                if let Some(edit) = self.edits.rows.iter().find(|r| &r.original == original) {
                    for (column, value) in &edit.fields {
                        cells.insert((row, *column), value.clone());
                    }
                }
            }
        }
        self.grid.update(cx, |grid, cx| {
            grid.delegate_mut().edits = cells;
            cx.notify();
        });
        cx.notify();
    }
    pub(super) fn discard_edits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edits = PendingEdits::default();
        self.cancel_cell_edit(window, cx);
        self.sync_edits(cx);
    }
    pub(in crate::ui) fn has_edits(&self, cx: &App) -> bool {
        !self.edits.rows.is_empty() || self.grid.read(cx).delegate().editor.is_some()
    }

    pub(in crate::ui) fn resolve_edits(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        complete: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if !self.has_edits(cx) {
            complete(self, window, cx);
            return;
        }
        let complete = std::rc::Rc::new(std::cell::RefCell::new(Some(complete)));
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let save = complete.clone();
            let discard = complete.clone();
            let save_view = view.clone();
            let discard_view = view.clone();
            dialog
                .title("Pending cell edits")
                .child("Save or discard your edits before continuing.")
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("guard-save-edits")
                                .label("Save changes")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    if let Some(done) = save.borrow_mut().take() {
                                        save_view.update(cx, |this, cx| {
                                            this.save_edits(window, cx, done)
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("guard-discard-edits")
                                .label("Discard")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    if let Some(done) = discard.borrow_mut().take() {
                                        discard_view.update(cx, |this, cx| {
                                            this.discard_edits(window, cx);
                                            done(this, window, cx);
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("guard-cancel-edits")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
        });
    }
    pub(super) fn save_edits(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        complete: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.busy() {
            return;
        }
        if self.grid.read(cx).delegate().editor.is_some() {
            self.accept_cell_edit(window, cx);
            if self.grid.read(cx).delegate().editor.is_some() {
                return;
            }
        }
        if self.edits.rows.is_empty() {
            complete(self, window, cx);
            return;
        }
        if self.profile.policy == ExecutionPolicy::ConfirmExecutions {
            let done = std::rc::Rc::new(std::cell::RefCell::new(Some(complete)));
            let view = cx.entity();
            let context = self.profile.context();
            let generation = self.session_generation;
            window.open_alert_dialog(cx, move |dialog, _, _| {
                let done = done.clone();
                let view = view.clone();
                dialog.title("Save cell edits?")
                    .child(format!("Apply pending updates to {context}. All updates succeed together or are rolled back."))
                    .button_props(DialogButtonProps::default().show_cancel(true).ok_text("Save changes"))
                    .on_ok(move |_, window, cx| {
                        if let Some(done) = done.borrow_mut().take() {
                            view.update(cx, |this, cx| {
                                if this.session_generation == generation {
                                    this.save_edits_confirmed(window, cx, done);
                                }
                            });
                        }
                        true
                    })
            });
        } else {
            self.save_edits_confirmed(window, cx, complete);
        }
    }
    fn save_edits_confirmed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        complete: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.busy() {
            return;
        }
        let (
            Some(session),
            DocumentKind::Table {
                table,
                page: Some(page),
                ..
            },
        ) = (self.session.clone(), &self.kind)
        else {
            return;
        };
        let table = table.clone();
        let options = self.applied_browse.clone().unwrap_or_default();
        let batch = UpdateBatch {
            columns: page.metadata.columns.clone(),
            rows: self.edits.rows.clone(),
        };
        let generation = self.session_generation;
        let timeout = self.statement_timeout;
        self.operation = Operation::Running;
        self.error.clear();
        task(
            &self.services,
            window,
            cx,
            async move {
                let result = match session.set_statement_timeout(timeout) {
                    Ok(()) => session.apply_updates(table.clone(), batch).await,
                    Err(error) => Err(error),
                };
                let state = session
                    .transaction_status()
                    .await
                    .unwrap_or(TransactionStatus::Unknown);
                let refreshed = if result.is_ok() && state != TransactionStatus::Unknown {
                    Some(session.browse(table, options).await)
                } else {
                    None
                };
                Ok((result, state, refreshed))
            },
            move |this, result, window, cx| {
                if generation != this.session_generation {
                    return;
                }
                this.operation = Operation::Idle;
                match result {
                    Ok((result, state, refreshed)) => {
                        this.observe_transaction(state, cx);
                        match result {
                            Ok(count) if state != TransactionStatus::Unknown => {
                                this.discard_edits(window, cx);
                                this.status = format!(
                                    "{count} rows applied{}",
                                    if state == TransactionStatus::Open {
                                        " · commit pending"
                                    } else {
                                        ""
                                    }
                                );
                                match refreshed {
                                    Some(Ok(page)) => {
                                        if let DocumentKind::Table {page: current,..}=&mut this.kind { *current=Some(Box::new(page)); }
                                        this.update_grid(cx);
                                    }
                                    Some(Err(error)) => {
                                        this.error=format!("Changes were applied, but reloading failed: {error:#}. Refresh before editing again.");
                                        if let DocumentKind::Table {page,..}=&mut this.kind { *page=None; }
                                        this.grid.update(cx,|grid,cx|{grid.delegate_mut().replace(Arc::new(ResultSet::default()));cx.notify();});
                                    }
                                    None=>{}
                                }
                                complete(this, window, cx);
                            }
                            Ok(_) => this.error = "Save returned but transaction status is unknown. Edits were retained; verify the outcome before retrying.".into(),
                            Err(error) => {
                                this.error = format!(
                                    "{error:#}. Pending edits were retained.{}",
                                    if state == TransactionStatus::Unknown {
                                        " Verify the database outcome before retrying."
                                    } else {
                                        ""
                                    }
                                )
                            }
                        }
                    }
                    Err(error) => this.error = format!("{error:#}"),
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    pub(super) fn edit_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let pending = self.edits.rows.len();
        let editable = !self.busy()
            && self.can_insert()
            && self
                .selected_data_cell(cx)
                .is_some_and(|(row, column)| match &self.kind {
                    DocumentKind::Table {
                        table,
                        page: Some(page),
                        ..
                    } => page.data.rows.get(row).is_some_and(|original| {
                        crate::row_updates::validate_original(&page.metadata, table, original)
                            .is_ok()
                            && page
                                .metadata
                                .columns
                                .get(column)
                                .is_some_and(|c| !c.generated)
                    }),
                    _ => false,
                });
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                Button::new("edit-cell")
                    .small()
                    .label("Edit cell · F2")
                    .disabled(!editable)
                    .on_click(cx.listener(|this, _, w, cx| this.begin_cell_edit(w, cx))),
            )
            .child(
                Button::new("cell-null")
                    .small()
                    .label("Set NULL")
                    .disabled(!editable)
                    .on_click(
                        cx.listener(|this, _, w, cx| this.stage_mode(FieldValue::Null, w, cx)),
                    ),
            )
            .child(
                Button::new("cell-default")
                    .small()
                    .label("Use default")
                    .disabled(!editable)
                    .on_click(
                        cx.listener(|this, _, w, cx| this.stage_mode(FieldValue::Default, w, cx)),
                    ),
            )
            .child(
                Button::new("save-edits")
                    .small()
                    .label(format!("Save changes ({pending})"))
                    .disabled(self.busy() || !self.has_edits(cx))
                    .on_click(cx.listener(|this, _, w, cx| this.save_edits(w, cx, |_, _, _| {}))),
            )
            .child(
                Button::new("revert-row")
                    .small()
                    .label("Revert row")
                    .disabled(self.busy() || pending == 0)
                    .on_click(cx.listener(|this, _, w, cx| {
                        if let Some(row) = this.selected_row(cx)
                            && let DocumentKind::Table {
                                page: Some(page), ..
                            } = &this.kind
                        {
                            this.edits.revert(&page.data.rows[row]);
                        }
                        this.cancel_cell_edit(w, cx);
                        this.sync_edits(cx);
                    })),
            )
            .child(
                Button::new("discard-edits")
                    .small()
                    .label("Discard all")
                    .disabled(self.busy() || !self.has_edits(cx))
                    .on_click(cx.listener(|this, _, w, cx| this.discard_edits(w, cx))),
            )
            .child(
                Button::new("column-statistics")
                    .small()
                    .label("Column statistics")
                    .disabled(self.busy())
                    .on_click(cx.listener(|this, _, w, cx| {
                        if let Some((_, column)) = this.selected_data_cell(cx) {
                            this.show_statistics(column, false, w, cx);
                        }
                    })),
            )
            .into_any_element()
    }
}
