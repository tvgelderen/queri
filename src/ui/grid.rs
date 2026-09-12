use crate::model::{Cell, ResultSet};
use gpui_kit::component::{
    ActiveTheme,
    checkbox::Checkbox,
    table::{Column, ColumnSort, TableDelegate, TableState},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone)]
pub enum GridEvent {
    Sort(String, ColumnSort),
}
impl EventEmitter<GridEvent> for TableState<Grid> {}

pub struct Grid {
    pub data: Arc<ResultSet>,
    pub display_columns: Vec<usize>,
    pub checkable: bool,
    pub search_cell: Option<(usize, usize)>,
    pub checked: BTreeSet<usize>,
    pub sortable: bool,
    pub sort: Vec<(String, ColumnSort)>,
    numeric_columns: BTreeSet<usize>,
}

impl Grid {
    pub fn new(data: Arc<ResultSet>) -> Self {
        let mut grid = Self {
            data: Arc::new(ResultSet::default()),
            display_columns: vec![],
            checkable: false,
            search_cell: None,
            checked: BTreeSet::new(),
            sortable: false,
            sort: vec![],
            numeric_columns: BTreeSet::new(),
        };
        grid.replace(data);
        grid
    }
    pub fn replace(&mut self, data: Arc<ResultSet>) {
        self.display_columns = (0..data.columns.len()).collect();
        self.data = data;
        self.search_cell = None;
        self.checked.clear();
        self.numeric_columns = (0..self.data.columns.len())
            .filter(|index| {
                let mut cells = self
                    .data
                    .rows
                    .iter()
                    .filter_map(|row| row.get(*index))
                    .filter(|cell| !matches!(cell, Cell::Null))
                    .peekable();
                cells.peek().is_some()
                    && cells.all(|cell| matches!(cell, Cell::Integer(_) | Cell::Real(_)))
            })
            .collect();
    }

    pub fn displayed_data(&self) -> ResultSet {
        let mut data = (*self.data).clone();
        data.columns = self
            .display_columns
            .iter()
            .map(|index| self.data.columns[*index].clone())
            .collect();
        data.rows = self
            .data
            .rows
            .iter()
            .map(|row| {
                self.display_columns
                    .iter()
                    .map(|index| row[*index].clone())
                    .collect()
            })
            .collect();
        data
    }

    pub fn set_column_types(&mut self, columns: &[crate::model::Column]) {
        for (index, column) in columns.iter().enumerate() {
            let name = column
                .data_type
                .split('(')
                .next()
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            if matches!(
                name.as_str(),
                "integer"
                    | "bigint"
                    | "smallint"
                    | "int"
                    | "int2"
                    | "int4"
                    | "int8"
                    | "real"
                    | "numeric"
                    | "decimal"
                    | "float"
                    | "double"
                    | "double precision"
            ) {
                self.numeric_columns.insert(index);
            }
        }
    }
    fn value(&self, row: usize, col: usize) -> String {
        if self.checkable && col == 0 {
            return if self.checked.contains(&row) {
                "Selected".into()
            } else {
                "Not selected".into()
            };
        }
        let Some(col) = col
            .checked_sub(usize::from(self.checkable))
            .and_then(|index| self.display_columns.get(index).copied())
        else {
            return String::new();
        };
        self.data
            .rows
            .get(row)
            .and_then(|row| row.get(col))
            .map(|cell| cell.text())
            .unwrap_or_default()
    }
}

impl TableDelegate for Grid {
    fn columns_count(&self, _: &App) -> usize {
        self.display_columns.len() + usize::from(self.checkable)
    }
    fn rows_count(&self, _: &App) -> usize {
        self.data.rows.len()
    }
    fn column(&self, index: usize, _: &App) -> Column {
        if self.checkable && index == 0 {
            return Column::new("selection", "#")
                .width(px(70.))
                .fixed_left()
                .selectable(false)
                .movable(false);
        }
        let i = self.display_columns[index.saturating_sub(usize::from(self.checkable))];
        let name = self.data.columns.get(i).cloned().unwrap_or_default();
        let content_width = self
            .data
            .rows
            .iter()
            .take(40)
            .filter_map(|row| row.get(i))
            .map(|cell| match cell {
                Cell::Text(value) => value.chars().take(36).count(),
                Cell::Blob(value) => value.len().saturating_mul(2).min(36),
                _ => cell.text().chars().take(36).count(),
            })
            .max()
            .unwrap_or(0);
        let characters = u16::try_from(
            content_width
                .max(name.chars().take(36).count() + 2)
                .clamp(10, 36),
        )
        .unwrap_or(36);
        let label = self
            .sort
            .iter()
            .position(|(column, _)| column == &name)
            .map(|position| format!("{} · {}", position + 1, name))
            .unwrap_or(name);
        let mut col = Column::new(format!("column-{i}"), label)
            .width(px(f32::from(characters) * 7.5 + 24.))
            .min_width(px(80.))
            .movable(false);
        if self.numeric_columns.contains(&i) {
            col = col.text_right();
        }
        if self.sortable {
            col.sort(
                self.sort
                    .iter()
                    .find(|(column, _)| Some(column) == self.data.columns.get(i))
                    .map(|(_, sort)| *sort)
                    .unwrap_or(ColumnSort::Default),
            )
        } else {
            col
        }
    }
    fn perform_sort(
        &mut self,
        index: usize,
        sort: ColumnSort,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if let Some(name) = self
            .data
            .columns
            .get(self.display_columns[index.saturating_sub(usize::from(self.checkable))])
        {
            cx.emit(GridEvent::Sort(name.clone(), sort));
        }
    }
    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if self.checkable && col == 0 {
            return Checkbox::new(("select-row", row))
                // Include the table cell's padding in the checkbox hit area.
                .absolute()
                .inset_0()
                .px_2()
                .items_center()
                .label(format!("{}", row + 1))
                .checked(self.checked.contains(&row))
                .on_change(cx.listener(move |state, checked, _, cx| {
                    cx.stop_propagation();
                    if *checked {
                        state.delegate_mut().checked.insert(row);
                    } else {
                        state.delegate_mut().checked.remove(&row);
                    }
                    cx.notify();
                }))
                .into_any_element();
        }
        let index = self.display_columns[col.saturating_sub(usize::from(self.checkable))];
        let cell = self.data.rows.get(row).and_then(|row| row.get(index));
        div()
            .when(self.search_cell == Some((row, index)), |view| {
                view.bg(cx.theme().accent)
                    .border_1()
                    .border_color(cx.theme().primary)
            })
            .text_ellipsis()
            .when(matches!(cell, Some(Cell::Null)), |view| {
                view.italic().text_color(cx.theme().muted_foreground)
            })
            .when(
                self.numeric_columns.contains(&index) || matches!(cell, Some(Cell::Blob(_))),
                |view| view.font_family(cx.theme().mono_font_family.clone()),
            )
            .child(self.value(row, col))
            .into_any_element()
    }
    fn cell_label(&self, row: usize, col: usize, _: &App) -> Option<SharedString> {
        Some(self.value(row, col).into())
    }
    fn cell_text(&self, row: usize, col: usize, _: &App) -> String {
        self.value(row, col)
    }
}
