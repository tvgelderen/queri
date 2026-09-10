use super::{grid::Grid, search::literal_ranges};
use crate::model::ResultSet;
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Selectable, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::{InputEvent, Textarea, TextareaState},
        table::TableState,
        v_flex,
    },
    *,
};
use std::sync::Arc;

actions!(
    queri_grid_search,
    [FindGrid, CloseGridSearch, PreviousGridMatch]
);

pub struct GridSearch {
    grid: Entity<TableState<Grid>>,
    input: Entity<TextareaState>,
    data: Arc<ResultSet>,
    open: bool,
    case_sensitive: bool,
    include_hidden: bool,
    display_columns: Vec<usize>,
    matches: Vec<(usize, usize)>,
    current: usize,
    generation: u64,
    searching: bool,
    task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

fn matching_cells(data: &ResultSet, query: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    let mut matches = vec![];
    if query.is_empty() {
        return matches;
    }
    // These are the displayed data columns; the checkbox is not searchable data.
    for (row_index, row) in data.rows.iter().enumerate() {
        for (column, cell) in row.iter().take(data.columns.len()).enumerate() {
            if !literal_ranges(&cell.text(), query, case_sensitive, 1).is_empty() {
                matches.push((row_index, column));
            }
        }
    }
    matches
}

impl GridSearch {
    pub fn new(
        grid: Entity<TableState<Grid>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new(
                "shift-enter",
                PreviousGridMatch,
                Some("QueriGridSearch > Input"),
            ),
            KeyBinding::new("escape", CloseGridSearch, Some("QueriGridSearch")),
            KeyBinding::new("ctrl-f", FindGrid, Some("QueriGridSearch")),
        ]);
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 3)
                .submit_on_enter(true)
                .placeholder("Find in loaded cells…")
        });
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => this.search(cx),
                InputEvent::PressEnter { shift, .. } => this.step(*shift, cx),
                _ => {}
            }),
            cx.observe(&grid, |this, grid, cx| {
                let data = grid.read(cx).delegate().data.clone();
                let columns = grid.read(cx).delegate().display_columns.clone();
                if !Arc::ptr_eq(&this.data, &data) || this.display_columns != columns {
                    this.display_columns = columns;
                    this.data = data;
                    this.search(cx);
                }
            }),
        ];
        Self {
            data: grid.read(cx).delegate().data.clone(),
            display_columns: grid.read(cx).delegate().display_columns.clone(),
            include_hidden: false,
            grid,
            input,
            open: false,
            case_sensitive: false,
            matches: vec![],
            current: 0,
            generation: 0,
            searching: false,
            task: None,
            _subscriptions: subscriptions,
        }
    }
    pub(super) fn status(&self) -> String {
        if self.searching {
            "Searching…".into()
        } else {
            format!(
                "{}/{} cells",
                if self.matches.is_empty() {
                    0
                } else {
                    self.current + 1
                },
                self.matches.len()
            ) + &self
                .matches
                .get(self.current)
                .filter(|(_, column)| !self.display_columns.contains(column))
                .map(|(_, column)| format!(" · hidden: {}", self.data.columns[*column]))
                .unwrap_or_default()
        }
    }
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.search(cx);
    }
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.search(cx);
        self.grid.read(cx).focus_handle(cx).focus(window, cx);
    }
    fn search(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.task = None;
        self.matches.clear();
        self.current = 0;
        self.highlight(cx);
        let query = self.input.read(cx).value().to_string();
        self.searching = self.open && !query.is_empty();
        cx.notify();
        if !self.searching {
            return;
        }
        let generation = self.generation;
        let data = self.data.clone();
        let case_sensitive = self.case_sensitive;
        let columns = self.display_columns.clone();
        let include_hidden = self.include_hidden;
        // Dropping the previous task cancels this delay before a new scan starts.
        let debounce = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(100));
        let work = cx.background_executor().spawn(async move {
            debounce.await;
            let mut matches = matching_cells(&data, &query, case_sensitive);
            if !include_hidden {
                matches.retain(|(_, column)| columns.contains(column));
            }
            matches.sort_by_key(|(row, column)| {
                (
                    *row,
                    columns
                        .iter()
                        .position(|index| index == column)
                        .unwrap_or(columns.len() + column),
                )
            });
            matches
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let matches = work.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation
                    || !Arc::ptr_eq(&this.data, &this.grid.read(cx).delegate().data)
                {
                    return;
                }
                this.matches = matches;
                this.searching = false;
                this.highlight(cx);
                cx.notify();
            });
        }));
    }
    fn step(&mut self, previous: bool, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        self.current = if previous {
            (self.current + self.matches.len() - 1) % self.matches.len()
        } else {
            (self.current + 1) % self.matches.len()
        };
        self.highlight(cx);
        cx.notify();
    }
    fn highlight(&self, cx: &mut Context<Self>) {
        let current = self.matches.get(self.current).copied();
        self.grid.update(cx, |grid, cx| {
            grid.delegate_mut().search_cell = current;
            if let Some((row, column)) = current {
                grid.scroll_to_row(row, cx);
                if let Some(display) = grid
                    .delegate()
                    .display_columns
                    .iter()
                    .position(|index| *index == column)
                {
                    grid.scroll_to_col(display + usize::from(grid.delegate().checkable), cx);
                }
            }
            cx.notify();
        });
    }
}
impl Render for GridSearch {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return Empty.into_any_element();
        }
        let count = self.status();
        v_flex().id("grid-search").key_context("QueriGridSearch").p_2().gap_1()
            .on_action(cx.listener(|this, _: &PreviousGridMatch, _, cx| this.step(true, cx)))
            .on_action(cx.listener(|this, _: &CloseGridSearch, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &FindGrid, window, cx| this.open(window, cx)))
            .child(h_flex().gap_2()
                .child(Textarea::new(&self.input).small().flex_1())
                .child(Button::new("grid-search-case").small().label("Aa").accessibility_label("Match case").selected(self.case_sensitive).toggled(self.case_sensitive).on_click(cx.listener(|this, _, _, cx| { this.case_sensitive = !this.case_sensitive; this.search(cx); })))
                .child(Button::new("grid-search-hidden").small().label("Include hidden").selected(self.include_hidden).toggled(self.include_hidden).on_click(cx.listener(|this, _, _, cx| { this.include_hidden = !this.include_hidden; this.search(cx); })))
                .child(div().id("grid-search-count").role(Role::Status).aria_label(count.clone()).text_sm().child(count))
                .child(Button::new("grid-search-previous").small().label("Previous").disabled(self.matches.is_empty()).on_click(cx.listener(|this, _, _, cx| this.step(true, cx))))
                .child(Button::new("grid-search-next").small().label("Next").disabled(self.matches.is_empty()).on_click(cx.listener(|this, _, _, cx| this.step(false, cx))))
                .child(Button::new("grid-search-close").small().ghost().label("Close").on_click(cx.listener(|this, _, window, cx| this.close(window, cx)))))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Loaded result / page only · visible columns by default · complete cell text · Enter / Shift+Enter to navigate"))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::matching_cells;
    use crate::model::{Cell, ResultSet};
    #[test]
    fn complete_cells_and_displayed_columns() {
        let data = ResultSet {
            columns: vec!["text".into()],
            rows: vec![
                vec![
                    Cell::Text(format!("{}Ä\nend", "x".repeat(1024 * 1024))),
                    Cell::Text("hidden".into()),
                ],
                vec![Cell::Null],
            ],
            ..Default::default()
        };
        assert_eq!(matching_cells(&data, "ä\nend", false), vec![(0, 0)]);
        assert!(matching_cells(&data, "ä\nend", true).is_empty());
        assert!(matching_cells(&data, "hidden", false).is_empty());
        assert_eq!(matching_cells(&data, "NULL", true), vec![(1, 0)]);
    }
}
