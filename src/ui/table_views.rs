use super::*;

fn node_mut<'a>(
    expression: &'a mut FilterExpression,
    path: &[usize],
) -> Option<&'a mut FilterExpression> {
    if path.is_empty() {
        return Some(expression);
    }
    match expression {
        FilterExpression::All(children) | FilterExpression::Any(children) => {
            node_mut(children.get_mut(path[0])?, &path[1..])
        }
        _ => None,
    }
}

impl Document {
    pub(super) fn filter_input(&self, cx: &App) -> Option<TableFilter> {
        let DocumentKind::Table {
            page: Some(page),
            filter,
            filter_column,
            filter_operator,
            ..
        } = &self.kind
        else {
            return None;
        };
        Some(TableFilter {
            column: page.metadata.columns.get(*filter_column)?.name.clone(),
            predicate: filter_operator.predicate(filter.read(cx).value().to_string()),
        })
    }

    pub(super) fn add_filter_node(&mut self, node: FilterExpression, cx: &mut Context<Self>) {
        if let DocumentKind::Table {
            filter_draft,
            filter_target,
            options,
            ..
        } = &mut self.kind
        {
            let root = filter_draft.get_or_insert_with(|| {
                Box::new(FilterExpression::All(
                    options.filter.clone().into_iter().collect(),
                ))
            });
            if let Some(target) = node_mut(root, filter_target) {
                match target {
                    FilterExpression::All(children) | FilterExpression::Any(children) => {
                        children.push(node)
                    }
                    FilterExpression::Predicate(_) => *target = node,
                }
            }
        }
        cx.notify();
    }

    pub(super) fn add_filter_condition(&mut self, cx: &mut Context<Self>) {
        if let Some(filter) = self.filter_input(cx) {
            self.add_filter_node(filter.into(), cx);
        }
    }

    pub(super) fn filter_from_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.grid.read(cx);
        let Some((row, column)) = state.selected_cell() else {
            return;
        };
        let Some(column) = column.checked_sub(usize::from(state.delegate().checkable)) else {
            return;
        };
        let Some(column) = state.delegate().display_columns.get(column).copied() else {
            return;
        };
        let data = &state.delegate().data;
        let Some(value) = data.rows.get(row).and_then(|row| row.get(column)) else {
            return;
        };
        let filter = TableFilter {
            column: data.columns[column].clone(),
            predicate: if matches!(value, Cell::Null) {
                FilterPredicate::IsNull
            } else {
                FilterPredicate::Equals(value.text())
            },
        };
        if let DocumentKind::Table {
            options,
            filter_draft,
            filter_target,
            filter_open,
            ..
        } = &mut self.kind
        {
            *filter_draft = Some(Box::new(FilterExpression::All(
                options.filter.clone().into_iter().collect(),
            )));
            filter_target.clear();
            *filter_open = true;
        }
        self.add_filter_node(filter.into(), cx);
        self.filter_focus(window, cx);
    }

    pub(super) fn filter_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let DocumentKind::Table {
            filter_apply_focus, ..
        } = &self.kind
        {
            filter_apply_focus.focus(window, cx);
        }
    }

    pub(super) fn filter_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        let DocumentKind::Table {
            filter_draft,
            filter_target,
            page,
            ..
        } = &self.kind
        else {
            return Empty.into_any_element();
        };
        let mut rows = vec![];
        if let Some(root) = filter_draft {
            self.filter_tree_rows(root, vec![], &mut rows, cx);
        }
        let target = if filter_target.is_empty() {
            "the top-level group".to_string()
        } else {
            format!(
                "group {}",
                filter_target
                    .iter()
                    .map(|i| (i + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(".")
            )
        };
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().muted_foreground)
                            .child("Conditions"),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .children(
                                [(false, "Add All group"), (true, "Add Any group")]
                                    .into_iter()
                                    .map(|(any, label)| {
                                        Button::new(if any {
                                            "add-any-group"
                                        } else {
                                            "add-all-group"
                                        })
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Plus)
                                        .label(if any { "Any group" } else { "All group" })
                                        .accessibility_label(label)
                                        .tooltip(if any {
                                            "Add a group where any condition matches (OR)"
                                        } else {
                                            "Add a group where all conditions match (AND)"
                                        })
                                        .disabled(self.busy() || page.is_none())
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.add_filter_node(
                                                    if any {
                                                        FilterExpression::Any(vec![])
                                                    } else {
                                                        FilterExpression::All(vec![])
                                                    },
                                                    cx,
                                                )
                                            }),
                                        )
                                    }),
                            )
                            .child(
                                Button::new("reset-filter-draft")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::RotateCcw)
                                    .label("Reset")
                                    .accessibility_label("Reset draft")
                                    .tooltip("Discard unapplied changes")
                                    .disabled(self.busy())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let DocumentKind::Table {
                                            filter_draft,
                                            filter_target,
                                            options,
                                            ..
                                        } = &mut this.kind
                                        {
                                            *filter_draft = options.filter.clone().map(|filter| {
                                                Box::new(FilterExpression::All(vec![filter]))
                                            });
                                            filter_target.clear();
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(v_flex().gap_0p5().children(rows))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "New conditions go into {target}. Select a condition to replace it."
                    )),
            )
            .into_any_element()
    }

    pub(super) fn filter_tree_rows(
        &self,
        node: &FilterExpression,
        path: Vec<usize>,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let DocumentKind::Table {
            page,
            filter_target,
            ..
        } = &self.kind
        else {
            return;
        };
        let id = format!("filter-node-{path:?}");
        let group = !matches!(node, FilterExpression::Predicate(_));
        let label = match node {
            FilterExpression::All(_) => "All (AND)".into(),
            FilterExpression::Any(_) => "Any (OR)".into(),
            FilterExpression::Predicate(filter) => filter.summary(
                page.as_ref()
                    .and_then(|p| p.metadata.columns.iter().find(|c| c.name == filter.column))
                    .map(|c| {
                        c.filter_kind(matches!(
                            self.profile.config,
                            ConnectionConfig::Sqlite { .. }
                        ))
                    })
                    .unwrap_or(FilterKind::Text),
            ),
        };
        let selected = filter_target == &path;
        let select_path = path.clone();
        let remove_path = path.clone();
        let toggle_path = path.clone();
        rows.push(
            h_flex()
                .gap_1()
                .pl(px(path.len() as f32 * 20.))
                .child(
                    Button::new(SharedString::from(id.clone()))
                        .xsmall()
                        .when(group, |button| button.outline())
                        .when(!group, |button| button.ghost())
                        .label(label)
                        .selected(selected)
                        .disabled(self.busy())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let DocumentKind::Table {
                                filter_draft: Some(root),
                                filter_target,
                                filter_column,
                                filter_operator,
                                filter,
                                page,
                                ..
                            } = &mut this.kind
                                && let Some(node) = node_mut(root, &select_path)
                            {
                                match node {
                                    FilterExpression::Predicate(predicate) => {
                                        *filter_target = select_path.clone();
                                        if let Some(index) = page.as_ref().and_then(|page| {
                                            page.metadata
                                                .columns
                                                .iter()
                                                .position(|column| column.name == predicate.column)
                                        }) {
                                            *filter_column = index;
                                            *filter_operator = predicate.predicate.operator();
                                            filter.update(cx, |input, cx| {
                                                input.set_value(
                                                    predicate.predicate.value().unwrap_or(""),
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }
                                    }
                                    _ => *filter_target = select_path.clone(),
                                }
                            }
                            cx.notify();
                        })),
                )
                .when(group, |row| {
                    row.child(
                        icon_button(
                            SharedString::from(format!("{id}-toggle")),
                            IconName::ArrowLeftRight,
                            "Switch All/Any",
                        )
                        .xsmall()
                        .disabled(self.busy())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let DocumentKind::Table {
                                filter_draft: Some(root),
                                ..
                            } = &mut this.kind
                                && let Some(node) = node_mut(root, &toggle_path)
                            {
                                *node = match node {
                                    FilterExpression::All(children) => {
                                        FilterExpression::Any(std::mem::take(children))
                                    }
                                    FilterExpression::Any(children) => {
                                        FilterExpression::All(std::mem::take(children))
                                    }
                                    _ => return,
                                };
                            }
                            cx.notify();
                        })),
                    )
                })
                .child(
                    icon_button(
                        SharedString::from(format!("{id}-remove")),
                        IconName::X,
                        "Remove",
                    )
                    .xsmall()
                    .disabled(self.busy())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let DocumentKind::Table {
                            filter_draft,
                            filter_target,
                            ..
                        } = &mut this.kind
                        {
                            if remove_path.is_empty() {
                                *filter_draft = Some(Box::new(FilterExpression::All(vec![])));
                            } else if let Some(root) = filter_draft
                                && let Some(
                                    FilterExpression::All(children)
                                    | FilterExpression::Any(children),
                                ) = node_mut(root, &remove_path[..remove_path.len() - 1])
                            {
                                children.remove(*remove_path.last().unwrap());
                            }
                            filter_target.clear();
                        }
                        cx.notify();
                    })),
                )
                .into_any_element(),
        );
        if let FilterExpression::All(children) | FilterExpression::Any(children) = node {
            for (index, child) in children.iter().enumerate() {
                let mut path = path.clone();
                path.push(index);
                self.filter_tree_rows(child, path, rows, cx);
            }
        }
    }
}

impl Document {
    pub(super) fn sort_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let DocumentKind::Table { options, .. } = &self.kind else {
            return Empty.into_any_element();
        };
        let many = options.sort.len() > 1;
        h_flex()
            .gap_1p5()
            .flex_wrap()
            .children(
                options
                    .sort
                    .iter()
                    .enumerate()
                    .map(|(index, (column, descending))| {
                        chip(cx)
                            .pl_0p5()
                            .gap_0p5()
                            .child(
                                Button::new(SharedString::from(format!("sort-direction-{index}")))
                                    .ghost()
                                    .xsmall()
                                    .rounded_full()
                                    .accessibility_label(format!(
                                        "{} · {} {}",
                                        index + 1,
                                        column,
                                        if *descending { "DESC" } else { "ASC" }
                                    ))
                                    .tooltip("Reverse direction")
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .when(many, |row| {
                                                row.child(
                                                    div()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child((index + 1).to_string()),
                                                )
                                            })
                                            .child(column.clone())
                                            .child(
                                                Icon::new(if *descending {
                                                    IconName::ArrowDown
                                                } else {
                                                    IconName::ArrowUp
                                                })
                                                .xsmall()
                                                .text_color(super::super::theme::accent_text(cx)),
                                            ),
                                    )
                                    .disabled(self.busy())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let DocumentKind::Table { options, .. } = &mut this.kind
                                        {
                                            options.sort[index].1 = !options.sort[index].1;
                                            options.offset = 0;
                                        }
                                        this.refresh(window, cx);
                                    })),
                            )
                            .when(index > 0, |chip| {
                                chip.child(
                                    icon_button(
                                        SharedString::from(format!("sort-earlier-{index}")),
                                        IconName::ChevronLeft,
                                        "Earlier",
                                    )
                                    .xsmall()
                                    .tooltip("Sort by this first")
                                    .disabled(self.busy())
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            if let DocumentKind::Table { options, .. } =
                                                &mut this.kind
                                            {
                                                options.sort.swap(index, index - 1);
                                                options.offset = 0;
                                            }
                                            this.refresh(window, cx);
                                        },
                                    )),
                                )
                            })
                            .child(
                                icon_button(
                                    SharedString::from(format!("sort-remove-{index}")),
                                    IconName::X,
                                    "Remove sort",
                                )
                                .xsmall()
                                .disabled(self.busy())
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        if let DocumentKind::Table { options, .. } = &mut this.kind
                                        {
                                            options.sort.remove(index);
                                            options.offset = 0;
                                        }
                                        this.refresh(window, cx);
                                    },
                                )),
                            )
                    }),
            )
            .into_any_element()
    }
}

impl Document {
    pub(super) fn table_view_settings(&self) -> Option<TableViewSettings> {
        let DocumentKind::Table {
            options, columns, ..
        } = &self.kind
        else {
            return None;
        };
        Some(TableViewSettings {
            filter: options.filter.clone(),
            sort: options.sort.clone(),
            columns: columns.clone(),
            page_size: options.limit,
        })
    }

    pub fn restore_table_view(&mut self, settings: Option<TableViewSettings>) {
        if let (
            Some(settings),
            DocumentKind::Table {
                options,
                columns,
                filter_draft,
                ..
            },
        ) = (settings, &mut self.kind)
        {
            options.filter = settings.filter.clone();
            *filter_draft = settings
                .filter
                .map(|filter| Box::new(FilterExpression::All(vec![filter])));
            options.sort = settings.sort;
            options.limit = settings.page_size;
            options.offset = 0;
            *columns = settings.columns;
        }
    }

    pub(super) fn column_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        let DocumentKind::Table {
            columns_open,
            columns,
            ..
        } = &self.kind
        else {
            return Empty.into_any_element();
        };
        let hidden = columns.iter().filter(|column| column.hidden).count();
        Button::new("view-columns")
            .ghost()
            .small()
            .icon(IconName::Columns3)
            .accessibility_label("Columns")
            .selected(*columns_open)
            .child(
                h_flex()
                    .gap_1p5()
                    .child("Columns")
                    .when(hidden > 0, |row| row.child(count_badge(hidden, cx))),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if let DocumentKind::Table { columns_open, .. } = &mut this.kind {
                    *columns_open = !*columns_open;
                }
                cx.notify();
            }))
            .into_any_element()
    }

    pub(super) fn column_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let DocumentKind::Table {
            columns_open,
            columns,
            page,
            ..
        } = &self.kind
        else {
            return Empty.into_any_element();
        };
        let mut all = columns.clone();
        if let Some(page) = page {
            for column in &page.metadata.columns {
                if !all.iter().any(|entry| entry.name == column.name) {
                    all.push(ViewColumn {
                        name: column.name.clone(),
                        hidden: false,
                    });
                }
            }
        }
        if !*columns_open {
            return Empty.into_any_element();
        }
        let hidden = all.iter().filter(|column| column.hidden).count();
        v_flex()
            .p_3()
            .gap_2()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(cx.theme().muted_foreground).child("Columns"))
                            .child(
                                div()
                                    .id("column-layout-note")
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground.opacity(0.8))
                                    .child(if hidden == 0 { "All visible".to_string() } else { format!("{hidden} hidden") })
                                    .tooltip(|window, cx| Tooltip::new("Presentation only · copy, export and search use visible column order. Row actions retain complete source rows.").build(window, cx)),
                            ),
                    )
                    .child(Button::new("reset-columns").ghost().xsmall().icon(IconName::RotateCcw).label("Reset").accessibility_label("Reset column layout").disabled(self.busy()).on_click(cx.listener(|this, _, _, cx| {
                        if let DocumentKind::Table { columns, .. } = &mut this.kind { columns.clear(); } this.error.clear(); this.update_grid(cx); cx.emit(DocumentChanged); cx.notify();
                    }))),
            )
            .child(h_flex().flex_wrap().gap_x_4().gap_y_0p5().children(all.iter().enumerate().map(|(index, column)| {
                let name = column.name.clone(); let current = all.clone(); let previous = all.clone(); let remove = all.clone();
                let missing = page.as_ref().is_some_and(|page| !page.metadata.columns.iter().any(|col| col.name == name));
                h_flex().w(px(220.)).gap_1()
                    .child(icon_button(SharedString::from(format!("column-visible-{index}")), if column.hidden { IconName::EyeOff } else { IconName::Eye }, format!("{} {}{}", if column.hidden { "Show" } else { "Hide" }, name, if missing { " (missing)" } else { "" })).xsmall().disabled(self.busy() || missing).on_click(cx.listener(move |this, _, _, cx| {
                        if let DocumentKind::Table { columns, .. } = &mut this.kind { *columns = current.clone(); columns[index].hidden = !columns[index].hidden; } this.update_grid(cx); cx.emit(DocumentChanged); cx.notify();
                    })))
                    .child(div().flex_1().min_w_0().text_sm().text_ellipsis()
                        .when(column.hidden || missing, |label| label.text_color(cx.theme().muted_foreground))
                        .when(missing, |label| label.text_color(cx.theme().danger))
                        .child(if missing { format!("{name} (missing)") } else { name.clone() }))
                    .when(missing, |row| row.child(icon_button(SharedString::from(format!("column-remove-{index}")), IconName::Trash, "Remove missing column").xsmall().on_click(cx.listener(move |this, _, _, cx| {
                        if let DocumentKind::Table { columns, .. } = &mut this.kind { *columns = remove.clone(); columns.remove(index); } this.error.clear(); this.update_grid(cx); cx.emit(DocumentChanged); cx.notify();
                    }))))
                    .child(icon_button(SharedString::from(format!("column-earlier-{index}")), IconName::ChevronUp, "Earlier").xsmall().tooltip(format!("Move {name} earlier")).disabled(self.busy() || index == 0).on_click(cx.listener(move |this, _, _, cx| {
                        if let DocumentKind::Table { columns, .. } = &mut this.kind { *columns = previous.clone(); columns.swap(index, index-1); } this.update_grid(cx); cx.emit(DocumentChanged); cx.notify();
                    })))
            })))
        .into_any_element()
    }
}
