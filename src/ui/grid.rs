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
    pub checkable: bool,
    pub checked: BTreeSet<usize>,
    pub sortable: bool,
    pub sort: Option<(String, ColumnSort)>,
    numeric_columns: BTreeSet<usize>,
}

impl Grid {
    pub fn new(data: Arc<ResultSet>) -> Self {
        let mut grid = Self {
            data: Arc::new(ResultSet::default()),
            checkable: false,
            checked: BTreeSet::new(),
            sortable: false,
            sort: None,
            numeric_columns: BTreeSet::new(),
        };
        grid.replace(data);
        grid
    }
    pub fn replace(&mut self, data: Arc<ResultSet>) {
        self.data = data;
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
        let col = col.saturating_sub(usize::from(self.checkable));
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
        self.data.columns.len() + usize::from(self.checkable)
    }
    fn rows_count(&self, _: &App) -> usize {
        self.data.rows.len()
    }
    fn column(&self, index: usize, _: &App) -> Column {
        if self.checkable && index == 0 {
            return Column::new("selection", "Select")
                .width(px(70.))
                .fixed_left()
                .selectable(false)
                .movable(false);
        }
        let i = index.saturating_sub(usize::from(self.checkable));
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
        let mut col = Column::new(format!("column-{i}"), name)
            .width(px(f32::from(characters) * 7.5 + 24.))
            .min_width(px(80.))
            .movable(false);
        if self.numeric_columns.contains(&i) {
            col = col.text_right();
        }
        if self.sortable {
            col.sort(
                self.sort
                    .as_ref()
                    .filter(|(column, _)| Some(column) == self.data.columns.get(i))
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
            .get(index.saturating_sub(usize::from(self.checkable)))
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
        let index = col.saturating_sub(usize::from(self.checkable));
        let cell = self.data.rows.get(row).and_then(|row| row.get(index));
        div()
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
