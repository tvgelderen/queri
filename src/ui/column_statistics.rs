use super::*;
impl Document {
    pub(super) fn show_statistics(
        &mut self,
        column: usize,
        exact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        if self.has_edits(cx) {
            self.resolve_edits(window, cx, move |this, w, cx| {
                this.show_statistics(column, exact, w, cx)
            });
            return;
        }
        let DocumentKind::Table {
            table,
            page: Some(page),
            section: Section::Data,
            ..
        } = &self.kind
        else {
            return;
        };
        let Some(metadata) = page.metadata.columns.get(column) else {
            return;
        };
        let name = metadata.name.clone();
        let table = table.clone();
        if !exact {
            match crate::statistics::ColumnStatistics::loaded(
                &page.data,
                column,
                matches!(self.profile.config, ConnectionConfig::Postgres { .. }),
            ) {
                Ok(stats) => self.statistics_dialog(column, name, stats, false, window, cx),
                Err(error) => {
                    self.error = format!("{error:#}");
                    cx.notify();
                }
            }
            return;
        }
        let (Some(session), Some(options)) = (self.session.clone(), self.applied_browse.clone())
        else {
            return;
        };
        let generation = self.session_generation;
        let timeout = self.statement_timeout;
        let output_name = name.clone();
        self.operation = Operation::Running;
        self.status = "Calculating column statistics…".into();
        self.error.clear();
        task(
            &self.services,
            window,
            cx,
            async move {
                let result = match session.set_statement_timeout(timeout) {
                    Ok(()) => session.column_statistics(table, options, name).await,
                    Err(error) => Err(error),
                };
                let transaction = session
                    .transaction_status()
                    .await
                    .unwrap_or(TransactionStatus::Unknown);
                Ok((result, transaction))
            },
            move |this, result, window, cx| {
                if this.session_generation != generation {
                    return;
                }
                this.operation = Operation::Idle;
                match result {
                    Ok((result, transaction)) => {
                        this.observe_transaction(transaction, cx);
                        match result {
                            Ok(stats) => {
                                this.status = "Column statistics calculated".into();
                                this.statistics_dialog(
                                    column,
                                    output_name,
                                    stats,
                                    true,
                                    window,
                                    cx,
                                );
                            }
                            Err(error) => {
                                this.error = format!("Could not calculate statistics: {error:#}")
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
    fn statistics_dialog(
        &self,
        column: usize,
        name: String,
        stats: crate::statistics::ColumnStatistics,
        exact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        let generation = self.session_generation;
        let context = format!(
            "{} · {} · {}",
            self.profile.context(),
            if exact {
                "All matching rows"
            } else {
                "Loaded rows"
            },
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        let text = stats.text();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let text = text.clone();
            dialog
                .title(format!("Statistics · {name}"))
                .child(context.clone())
                .child(
                    div()
                        .id("statistics-results")
                        .test_support()
                        .role(Role::Status)
                        .aria_label(text.clone())
                        .max_h(px(400.))
                        .overflow_y_scroll()
                        .whitespace_normal()
                        .child(text.clone()),
                )
                .child("Snapshot at the time shown; reopen after changing filters or data.")
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("copy-statistics")
                                .label("Copy statistics")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                                }),
                        )
                        .when(!exact, |bar| {
                            bar.child(
                                Button::new("exact-statistics")
                                    .label("Calculate all matching rows…")
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |this, cx| {
                                            if this.session_generation == generation {
                                                this.confirm_statistics(column, window, cx);
                                            }
                                        });
                                    }),
                            )
                        }),
                )
        });
    }
    fn confirm_statistics(&mut self, column: usize, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let generation = self.session_generation;
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog.title("Calculate all matching rows?")
                .child("This scans the filtered dataset and can be expensive. The tab timeout and Cancel action apply.")
                .button_props(DialogButtonProps::default().show_cancel(true).ok_text("Calculate"))
                .on_ok(move |_, window, cx| {
                    view.update(cx, |this, cx| {
                        if this.session_generation == generation {
                            this.show_statistics(column, true, window, cx);
                        }
                    });
                    true
                })
        });
    }
}
