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
            ..
        } = &mut self.kind
        {
            let root = filter_draft.get_or_insert_with(|| FilterExpression::All(vec![]));
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
            *filter_draft = Some(FilterExpression::All(
                options.filter.clone().into_iter().collect(),
            ));
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
        v_flex().gap_1().mt_2()
            .child(div().text_xs().child("Draft conditions · select a group to add; select a condition to replace. Apply runs the draft."))
            .children(rows)
            .child(h_flex().gap_2()
                .children([(false, "Add All group"), (true, "Add Any group")].into_iter().map(|(any, label)| Button::new(if any { "add-any-group" } else { "add-all-group" }).small().label(label).disabled(self.busy() || page.is_none()).on_click(cx.listener(move |this, _, _, cx| this.add_filter_node(if any { FilterExpression::Any(vec![]) } else { FilterExpression::All(vec![]) }, cx)))))
                .child(Button::new("reset-filter-draft").small().label("Reset draft").disabled(self.busy()).on_click(cx.listener(|this, _, _, cx| {
                    if let DocumentKind::Table { filter_draft, filter_target, options, .. } = &mut this.kind { *filter_draft = options.filter.clone().map(|filter| FilterExpression::All(vec![filter])); filter_target.clear(); }
                    cx.notify();
                }))))
            .child(div().text_xs().child(format!("Adding to group {}", if filter_target.is_empty() { "root".into() } else { filter_target.iter().map(|i| (i+1).to_string()).collect::<Vec<_>>().join(".") })))
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
                .gap_2()
                .pl(px(path.len() as f32 * 16.))
                .child(
                    Button::new(SharedString::from(id.clone()))
                        .small()
                        .label(label)
                        .selected(selected && group)
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
                            {
                                if let Some(node) = node_mut(root, &select_path) {
                                    match node {
                                        FilterExpression::Predicate(predicate) => {
                                            *filter_target = select_path.clone();
                                            if let Some(index) = page.as_ref().and_then(|page| {
                                                page.metadata.columns.iter().position(|column| {
                                                    column.name == predicate.column
                                                })
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
                            }
                            cx.notify();
                        })),
                )
                .when(group, |row| {
                    row.child(
                        Button::new(SharedString::from(format!("{id}-toggle")))
                            .small()
                            .label("Switch All/Any")
                            .disabled(self.busy())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let DocumentKind::Table {
                                    filter_draft: Some(root),
                                    ..
                                } = &mut this.kind
                                {
                                    if let Some(node) = node_mut(root, &toggle_path) {
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
                                }
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new(SharedString::from(format!("{id}-remove")))
                        .small()
                        .label("Remove")
                        .disabled(self.busy())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let DocumentKind::Table {
                                filter_draft,
                                filter_target,
                                ..
                            } = &mut this.kind
                            {
                                if remove_path.is_empty() {
                                    *filter_draft = None;
                                } else if let Some(root) = filter_draft {
                                    if let Some(
                                        FilterExpression::All(children)
                                        | FilterExpression::Any(children),
                                    ) = node_mut(root, &remove_path[..remove_path.len() - 1])
                                    {
                                        children.remove(*remove_path.last().unwrap());
                                    }
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
        h_flex()
            .gap_2()
            .flex_wrap()
            .children(
                options
                    .sort
                    .iter()
                    .enumerate()
                    .map(|(index, (column, descending))| {
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!("sort-direction-{index}")))
                                    .small()
                                    .label(format!(
                                        "{} · {} {}",
                                        index + 1,
                                        column,
                                        if *descending { "DESC" } else { "ASC" }
                                    ))
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
                            .child(
                                Button::new(SharedString::from(format!("sort-earlier-{index}")))
                                    .small()
                                    .label("Earlier")
                                    .disabled(self.busy() || index == 0)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let DocumentKind::Table { options, .. } = &mut this.kind
                                        {
                                            options.sort.swap(index, index - 1);
                                            options.offset = 0;
                                        }
                                        this.refresh(window, cx);
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("sort-remove-{index}")))
                                    .small()
                                    .label("Remove sort")
                                    .disabled(self.busy())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let DocumentKind::Table { options, .. } = &mut this.kind
                                        {
                                            options.sort.remove(index);
                                            options.offset = 0;
                                        }
                                        this.refresh(window, cx);
                                    })),
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
                .map(|filter| FilterExpression::All(vec![filter]));
            options.sort = settings.sort;
            options.limit = settings.page_size;
            options.offset = 0;
            *columns = settings.columns;
        }
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
        v_flex().gap_1().child(Button::new("view-columns").small().label("Columns").selected(*columns_open).on_click(cx.listener(|this, _, _, cx| {
            if let DocumentKind::Table { columns_open, .. } = &mut this.kind { *columns_open = !*columns_open; } cx.notify();
        })))
        .when(*columns_open, |view| view
            .child(div().text_xs().child("Presentation only · copy, export and search use visible column order. Row actions retain complete source rows."))
            .children(all.iter().enumerate().map(|(index, column)| {
                let name = column.name.clone(); let current = all.clone(); let previous = all.clone(); let remove = all.clone();
                let missing = page.as_ref().is_some_and(|page| !page.metadata.columns.iter().any(|col| col.name == name));
                h_flex().gap_2()
                    .child(Button::new(SharedString::from(format!("column-visible-{index}"))).small().label(format!("{} {}{}", if column.hidden { "Show" } else { "Hide" }, name, if missing { " (missing)" } else { "" })).disabled(self.busy() || missing).on_click(cx.listener(move |this, _, _, cx| {
                        if let DocumentKind::Table { columns, .. } = &mut this.kind { *columns = current.clone(); columns[index].hidden = !columns[index].hidden; } this.update_grid(cx); cx.emit(DocumentChanged); cx.notify();
                    })))
                    .child(Button::new(SharedString::from(format!("column-earlier-{index}"))).small().label("Earlier").disabled(self.busy() || index == 0).on_click(cx.listener(move |this, _, _, cx| {
                        if let DocumentKind::Table { columns, .. } = &mut this.kind { *columns = previous.clone(); columns.swap(index, index-1); } this.update_grid(cx); cx.emit(DocumentChanged); cx.notify();
                    })))
                    .when(missing, |row| row.child(Button::new(SharedString::from(format!("column-remove-{index}"))).small().label("Remove missing column").on_click(cx.listener(move |this, _, _, cx| {
                        if let DocumentKind::Table { columns, .. } = &mut this.kind { *columns = remove.clone(); columns.remove(index); } this.error.clear(); this.update_grid(cx); cx.emit(DocumentChanged); cx.notify();
                    }))))
            }))
            .child(Button::new("reset-columns").small().label("Reset column layout").disabled(self.busy()).on_click(cx.listener(|this, _, _, cx| {
                if let DocumentKind::Table { columns, .. } = &mut this.kind { columns.clear(); } this.error.clear(); this.update_grid(cx); cx.emit(DocumentChanged); cx.notify();
            }))))
        .into_any_element()
    }
}
