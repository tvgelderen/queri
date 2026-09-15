//! Native schema relationship inspector. Relationships are declared constraints only.
use super::controls::{
    Tone, count_badge, icon_button, notice, page_header, section_label, toolbar_divider,
};
use super::theme::accent_text;
use crate::schema::{Constraint, Snapshot};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Icon, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    prelude::*,
    *,
};

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct DiagramLayout {
    positions: std::collections::BTreeMap<String, (f32, f32)>,
    zoom: f32,
    pan: (f32, f32),
}
pub struct RefreshDiagram;
impl EventEmitter<RefreshDiagram> for ErDiagram {}

pub struct OpenDiagramTable(pub crate::model::TableRef);
impl EventEmitter<OpenDiagramTable> for ErDiagram {}

pub struct CloseDiagram;
impl EventEmitter<CloseDiagram> for ErDiagram {}

pub struct ErDiagram {
    export_message: String,
    search: Option<Entity<gpui_kit::component::input::InputState>>,
    search_subscription: Option<Subscription>,
    snapshot: Snapshot,
    selected: usize,
    positions: Vec<(f32, f32)>,
    zoom: f32,
    pan: (f32, f32),
    drag: Option<(Option<usize>, Point<Pixels>)>,
    viewport: std::rc::Rc<std::cell::Cell<(f32, f32)>>,
    /// Fit the first frame unless a saved layout was restored.
    pending_fit: bool,
}
impl ErDiagram {
    pub fn saved_layout(&self) -> DiagramLayout {
        DiagramLayout {
            positions: self
                .snapshot
                .tables
                .iter()
                .zip(&self.positions)
                .map(|(t, p)| (t.name.clone(), *p))
                .collect(),
            zoom: self.zoom,
            pan: self.pan,
        }
    }
    pub fn restore_layout(&mut self, layout: DiagramLayout) {
        self.pending_fit = false;
        for (table, position) in self.snapshot.tables.iter().zip(&mut self.positions) {
            if let Some(saved) = layout
                .positions
                .get(&table.name)
                .filter(|p| p.0.is_finite() && p.1.is_finite())
            {
                *position = *saved;
            }
        }
        if layout.zoom.is_finite() && layout.zoom > 0. {
            self.zoom = layout.zoom.clamp(0.1, 3.);
        }
        if layout.pan.0.is_finite() && layout.pan.1.is_finite() {
            self.pan = layout.pan;
        }
    }
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            export_message: String::new(),
            search: None,
            search_subscription: None,
            positions: layout(&snapshot),
            snapshot,
            zoom: 1.,
            pan: (20., 20.),
            drag: None,
            viewport: Default::default(),
            pending_fit: true,
            selected: 0,
        }
    }
}
const WIDTH: f32 = 240.;
/// Source table, destination table and the polyline between their columns.
type Route = (usize, usize, Vec<(f32, f32)>);
fn layout(snapshot: &Snapshot) -> Vec<(f32, f32)> {
    let mut order: Vec<_> = (0..snapshot.tables.len()).collect();
    order.sort_by_key(|&i| &snapshot.tables[i].name);
    let mut positions = vec![(0., 0.); order.len()];
    let mut y = 60.;
    for row in order.chunks(3) {
        for (col, &i) in row.iter().enumerate() {
            positions[i] = (col as f32 * 330., y);
        }
        y += row
            .iter()
            .map(|&i| height(snapshot.tables[i].columns.len()))
            .fold(0., f32::max)
            + 90.;
    }
    positions
}
fn height(columns: usize) -> f32 {
    36. + columns as f32 * 24.
}
fn primary(table: &crate::schema::Table, name: &str, position: i64) -> bool {
    position > 0
        || table.constraints.iter().any(
            |c| matches!(c, Constraint::Primary { columns } if columns.iter().any(|c| c == name)),
        )
}
// Compare complete quoted identifiers, so dots inside identifiers remain literal.
fn resolve(snapshot: &Snapshot, target: &str) -> Option<usize> {
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let matches: Vec<_> = snapshot
        .tables
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            target == t.name
                || target == quote(&t.name)
                || target == format!("{}.{}", quote(&snapshot.scope.namespace), quote(&t.name))
                || (!snapshot.scope.namespace.contains('.')
                    && !t.name.contains('.')
                    && target == format!("{}.{}", snapshot.scope.namespace, t.name))
        })
        .map(|(i, _)| i)
        .collect();
    (matches.len() == 1).then(|| matches[0])
}
fn route(a: (f32, f32), b: (f32, f32), lane: usize) -> Vec<(f32, f32)> {
    let x = a.0 + 20. + lane as f32 * 8.;
    let destination_x = b.0 + 20. + lane as f32 * 8.;
    let corridor = 10. + lane as f32 * 12.;
    if a == b {
        vec![a, (x, a.1), (x, a.1 - 18.), (a.0, a.1 - 18.), a]
    } else {
        vec![
            a,
            (x, a.1),
            (x, corridor),
            (destination_x, corridor),
            (destination_x, b.1),
            b,
        ]
    }
}
impl ErDiagram {
    fn fit(&mut self) {
        let (w, h) = self.viewport.get();
        if self.positions.is_empty() || w <= 0. || h <= 0. {
            return;
        }
        let min_x = self
            .positions
            .iter()
            .map(|p| p.0)
            .fold(f32::INFINITY, f32::min);
        let min_y = self
            .positions
            .iter()
            .map(|p| p.1)
            .fold(f32::INFINITY, f32::min);
        let max_x = self
            .positions
            .iter()
            .map(|p| p.0 + WIDTH + 80.)
            .fold(0., f32::max);
        let max_y = self
            .positions
            .iter()
            .enumerate()
            .map(|(i, p)| p.1 + height(self.snapshot.tables[i].columns.len()))
            .fold(0., f32::max);
        self.zoom = ((w - 40.) / (max_x - min_x))
            .min((h - 40.) / (max_y - min_y))
            .clamp(0.1, 2.);
        self.pan = (20. - min_x * self.zoom, 20. - min_y * self.zoom);
    }
    /// Brings a table into the middle of the visible canvas without changing zoom.
    fn reveal(&mut self, index: usize) {
        let (w, h) = self.viewport.get();
        let (Some(position), Some(table)) =
            (self.positions.get(index), self.snapshot.tables.get(index))
        else {
            return;
        };
        if w <= 0. || h <= 0. {
            return;
        }
        let center = (
            position.0 + WIDTH / 2.,
            position.1 + height(table.columns.len()) / 2.,
        );
        self.pan = (w / 2. - center.0 * self.zoom, h / 2. - center.1 * self.zoom);
    }
    /// Each route with its source and destination table indexes.
    fn routes(&self) -> Vec<Route> {
        let mut edges = Vec::new();
        for (i, table) in self.snapshot.tables.iter().enumerate() {
            for constraint in &table.constraints {
                if let Constraint::ForeignKey {
                    columns,
                    target,
                    referenced_columns,
                    ..
                } = constraint
                    && let Some(j) = resolve(&self.snapshot, target)
                {
                    for (source, dest) in columns.iter().zip(referenced_columns) {
                        let row = |idx: usize, name: &str| {
                            self.snapshot.tables[idx]
                                .columns
                                .iter()
                                .position(|c| c.name == name)
                                .map(|r| 48. + r as f32 * 24.)
                                .unwrap_or(18.)
                        };
                        let lane = edges.len();
                        let (from, to) = (self.positions[i], self.positions[j]);
                        let (ya, yb) = (from.1 + row(i, source), to.1 + row(j, dest));
                        let offset = (lane % 3) as f32 * 8. - 8.;
                        // Straight elbows only when no other table sits between the ports.
                        let clear = |x0: f32, x1: f32| {
                            let (top, bottom) = (ya.min(yb), ya.max(yb));
                            !self.positions.iter().enumerate().any(|(k, p)| {
                                k != i
                                    && k != j
                                    && p.0 < x1
                                    && p.0 + WIDTH > x0
                                    && p.1 < bottom + 12.
                                    && p.1 + height(self.snapshot.tables[k].columns.len())
                                        > top - 12.
                            })
                        };
                        let corridor = self
                            .positions
                            .iter()
                            .map(|p| p.1)
                            .fold(f32::INFINITY, f32::min)
                            - 24.
                            - lane as f32 * 8.;
                        let detour = |a: (f32, f32), out: f32, b: (f32, f32), into: f32| {
                            let spread = lane as f32 * 6.;
                            vec![
                                a,
                                (a.0 + out * (16. + spread), a.1),
                                (a.0 + out * (16. + spread), corridor),
                                (b.0 + into * (16. + spread), corridor),
                                (b.0 + into * (16. + spread), b.1),
                                b,
                            ]
                        };
                        let points = if i == j {
                            route((from.0 + WIDTH, ya), (to.0 + WIDTH, yb), lane)
                        } else if to.0 > from.0 + WIDTH + 32. {
                            // Target to the right: leave right, enter left.
                            if clear(from.0 + WIDTH, to.0) {
                                let x = (from.0 + WIDTH + to.0) / 2. + offset;
                                vec![(from.0 + WIDTH, ya), (x, ya), (x, yb), (to.0, yb)]
                            } else {
                                detour((from.0 + WIDTH, ya), 1., (to.0, yb), -1.)
                            }
                        } else if to.0 + WIDTH + 32. < from.0 {
                            if clear(to.0 + WIDTH, from.0) {
                                let x = (to.0 + WIDTH + from.0) / 2. + offset;
                                vec![(from.0, ya), (x, ya), (x, yb), (to.0 + WIDTH, yb)]
                            } else {
                                detour((from.0, ya), -1., (to.0 + WIDTH, yb), 1.)
                            }
                        } else {
                            // Stacked tables: loop around their shared right edge.
                            let x = from.0.max(to.0) + WIDTH + 24. + lane as f32 * 8.;
                            vec![(from.0 + WIDTH, ya), (x, ya), (x, yb), (to.0 + WIDTH, yb)]
                        };
                        edges.push((i, j, points));
                    }
                }
            }
        }
        edges
    }
    fn edges(&self) -> Vec<Vec<(f32, f32)>> {
        self.routes().into_iter().map(|(_, _, edge)| edge).collect()
    }
    pub fn svg(&self) -> String {
        let escape = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        };
        let (w, h) = self.viewport.get();
        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}"><g transform="translate({} {}) scale({})" font-family="sans-serif" font-size="12">"#,
            w.max(800.),
            h.max(600.),
            self.pan.0,
            self.pan.1,
            self.zoom
        );
        for edge in self.edges() {
            let points = edge
                .iter()
                .map(|(x, y)| format!("{x},{y}"))
                .collect::<Vec<_>>()
                .join(" ");
            svg.push_str(&format!(
                r#"<polyline points="{points}" fill="none" stroke="black"/>"#
            ));
        }
        for (table, (x, y)) in self.snapshot.tables.iter().zip(&self.positions) {
            svg.push_str(&format!(r#"<rect x="{x}" y="{y}" width="{WIDTH}" height="{}" fill="white" stroke="black"/><text x="{}" y="{}">{}</text>"#, height(table.columns.len()), x+4., y+18., escape(&table.name)));
            for (i, c) in table.columns.iter().enumerate() {
                svg.push_str(&format!(
                    r#"<text x="{}" y="{}">{} {}</text>"#,
                    x + 4.,
                    y + 48. + i as f32 * 24.,
                    escape(&c.name),
                    escape(&c.data_type)
                ));
            }
        }
        svg.push_str("</g></svg>");
        svg
    }
    fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.1, 3.);
    }
    fn canvas_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let z = self.zoom;
        let pan = self.pan;
        let selected = self.selected;
        let edges = self.routes();
        let quiet = cx.theme().muted_foreground.opacity(0.55);
        let accent = accent_text(cx);
        let dot = cx.theme().muted_foreground.opacity(0.18);
        let viewport = self.viewport.clone();
        let mut surface = div()
            .id("er-canvas")
            .relative()
            .flex_1()
            .min_w_0()
            .min_h(px(200.))
            .overflow_hidden()
            .bg(cx.theme().sidebar)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, _| {
                    this.drag = Some((None, e.position));
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag = None),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if !e.dragging() {
                    this.drag = None;
                    return;
                }
                if let Some((node, last)) = this.drag {
                    let dx = f32::from(e.position.x - last.x);
                    let dy = f32::from(e.position.y - last.y);
                    if let Some(i) = node {
                        this.positions[i].0 += dx / this.zoom;
                        this.positions[i].1 += dy / this.zoom;
                    } else {
                        this.pan.0 += dx;
                        this.pan.1 += dy;
                    }
                    this.drag = Some((node, e.position));
                    cx.notify();
                }
            }))
            .child(
                canvas(
                    move |bounds, _, _| {
                        viewport.set((bounds.size.width.into(), bounds.size.height.into()));
                    },
                    move |bounds, _, window, _| {
                        // A quiet dot grid that moves with the canvas.
                        let spacing = 24. * z;
                        if spacing >= 8. {
                            let width: f32 = bounds.size.width.into();
                            let height: f32 = bounds.size.height.into();
                            let mut x = pan.0.rem_euclid(spacing);
                            while x < width {
                                let mut y = pan.1.rem_euclid(spacing);
                                while y < height {
                                    window.paint_quad(
                                        fill(
                                            Bounds::new(
                                                bounds.origin + point(px(x - 0.75), px(y - 0.75)),
                                                size(px(1.5), px(1.5)),
                                            ),
                                            dot,
                                        )
                                        .corner_radii(px(0.75)),
                                    );
                                    y += spacing;
                                }
                                x += spacing;
                            }
                        }
                        // Relationships of the selected table are drawn last, on top.
                        let mut ordered = edges;
                        ordered.sort_by_key(|(from, to, _)| *from == selected || *to == selected);
                        for (from, to, edge) in ordered {
                            let color = if from == selected || to == selected {
                                accent
                            } else {
                                quiet
                            };
                            let at = |(x, y): (f32, f32)| {
                                bounds.origin + point(px(x * z + pan.0), px(y * z + pan.1))
                            };
                            let mut path = PathBuilder::stroke(px(1.25));
                            for (i, &p) in edge.iter().enumerate() {
                                if i == 0 {
                                    path.move_to(at(p));
                                } else {
                                    path.line_to(at(p));
                                }
                            }
                            if let [.., before, end] = edge.as_slice() {
                                // Arrowhead along the final segment.
                                let (dx, dy) = (end.0 - before.0, end.1 - before.1);
                                let length = (dx * dx + dy * dy).sqrt().max(f32::EPSILON);
                                let (ux, uy) = (dx / length, dy / length);
                                let p = at(*end);
                                path.move_to(
                                    p + point(px(-ux * 7. - uy * 4.), px(-uy * 7. + ux * 4.)),
                                );
                                path.line_to(p);
                                path.line_to(
                                    p + point(px(-ux * 7. + uy * 4.), px(-uy * 7. - ux * 4.)),
                                );
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, color);
                            }
                            if let Some(&start) = edge.first() {
                                let p = at(start);
                                window.paint_quad(
                                    fill(
                                        Bounds::new(
                                            p - point(px(3.), px(3.)),
                                            size(px(6.), px(6.)),
                                        ),
                                        color,
                                    )
                                    .corner_radii(px(3.)),
                                );
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            );
        let key = cx.theme().warning;
        let link = accent_text(cx);
        for (i, table) in self.snapshot.tables.iter().enumerate() {
            let active = i == self.selected;
            let mut node = v_flex()
                .id(("er-node", i))
                .absolute()
                .left(px(self.positions[i].0 * z + pan.0))
                .top(px(self.positions[i].1 * z + pan.1))
                .w(px(WIDTH * z))
                .h(px(height(table.columns.len()) * z))
                .text_size(px(12. * z))
                .bg(cx.theme().popover)
                .rounded(px(8. * z))
                .border_1()
                .border_color(if active {
                    cx.theme().ring
                } else {
                    cx.theme().border
                })
                .when(active, |node| node.shadow_md())
                .when(!active, |node| node.shadow_xs())
                .overflow_hidden()
                .cursor_grab()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.selected = i;
                        this.drag = Some((Some(i), e.position));
                        cx.notify();
                    }),
                )
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .h(px(36. * z))
                        .px(px(10. * z))
                        .gap(px(6. * z))
                        .bg(cx.theme().muted)
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(Icon::new(IconName::Table2).size(px(13. * z)).text_color(
                            if active {
                                accent_text(cx)
                            } else {
                                cx.theme().muted_foreground
                            },
                        ))
                        .child(
                            div()
                                .min_w_0()
                                .text_ellipsis()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(table.name.clone()),
                        ),
                );
            for c in &table.columns {
                let pk = primary(table, &c.name, c.primary_key_position);
                let fk = is_foreign(table, &c.name);
                node =
                    node.child(
                        h_flex()
                            .flex_shrink_0()
                            .h(px(24. * z))
                            .px(px(10. * z))
                            .gap(px(6. * z))
                            .child(
                                div()
                                    .w(px(13. * z))
                                    .flex_shrink_0()
                                    .children((pk || fk).then(|| {
                                        Icon::new(if pk {
                                            IconName::KeyRound
                                        } else {
                                            IconName::Link
                                        })
                                        .size(px(11. * z))
                                        .text_color(if pk { key } else { link })
                                    })),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .when(pk, |name| name.font_weight(FontWeight::MEDIUM))
                                    .child(c.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_size(px(11. * z))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(c.data_type.to_lowercase()),
                            ),
                    );
            }
            surface = surface.child(node);
        }
        let zoom_label = format!("{:.0}%", self.zoom * 100.);
        surface.child(
            h_flex()
                .id("er-zoom-controls")
                .absolute()
                .right_3()
                .bottom_3()
                .p_0p5()
                .gap_0p5()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .shadow_sm()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    icon_button("er-zoom-out", IconName::ZoomOut, "Zoom out")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.zoom_by(1. / 1.2);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .w(px(44.))
                        .text_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .font_features(FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])))
                        .child(zoom_label),
                )
                .child(
                    icon_button("er-zoom-in", IconName::ZoomIn, "Zoom in")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.zoom_by(1.2);
                            cx.notify();
                        })),
                )
                .child(toolbar_divider(cx))
                .child(
                    icon_button("er-fit", IconName::Maximize, "Fit to view")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.fit();
                            cx.notify();
                        })),
                )
                .child(
                    icon_button("er-arrange", IconName::LayoutGrid, "Arrange tables")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.positions = layout(&this.snapshot);
                            this.fit();
                            cx.notify();
                        })),
                ),
        )
    }
    fn table_list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let search = self.search.as_ref().unwrap();
        let query = search.read(cx).value().to_lowercase();
        let rows = self
            .snapshot
            .tables
            .iter()
            .enumerate()
            .filter(|(_, table)| table.name.to_lowercase().contains(&query))
            .map(|(index, table)| {
                let active = index == self.selected;
                h_flex()
                    .id(("er-table", index))
                    .role(Role::Button)
                    .aria_label(table.name.clone())
                    .h_8()
                    .px_2()
                    .gap_2()
                    .rounded(cx.theme().radius)
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |row| row.bg(cx.theme().list_active))
                    .when(!active, |row| {
                        row.hover(|row| row.bg(cx.theme().list_hover))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = index;
                        this.reveal(index);
                        cx.notify();
                    }))
                    .child(Icon::new(IconName::Table2).small().text_color(if active {
                        accent_text(cx)
                    } else {
                        cx.theme().muted_foreground
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .when(active, |name| name.font_weight(FontWeight::MEDIUM))
                            .child(table.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(table.columns.len().to_string()),
                    )
            })
            .collect::<Vec<_>>();
        let empty = rows.is_empty();
        v_flex()
            .w(px(220.))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(section_label("Tables", cx))
                            .child(count_badge(self.snapshot.tables.len(), cx)),
                    )
                    .child(
                        Input::new(search)
                            .small()
                            .prefix(
                                Icon::new(IconName::Search)
                                    .small()
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .cleanable(true)
                            .aria_label("Search tables"),
                    ),
            )
            .child(
                v_flex()
                    .id("er-table-list")
                    .flex_1()
                    .min_h_0()
                    .px_2()
                    .pb_2()
                    .gap_px()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(empty, |list| {
                        list.child(
                            div()
                                .px_2()
                                .py_4()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No tables match"),
                        )
                    }),
            )
    }
    fn details(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut details = v_flex()
            .id("er-details")
            .w(px(280.))
            .flex_shrink_0()
            .h_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .overflow_y_scroll();
        let Some(table) = self.snapshot.tables.get(self.selected) else {
            return details.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("This schema has no tables."),
            );
        };
        let table_ref = crate::model::TableRef {
            schema: self.snapshot.scope.namespace.clone(),
            name: table.name.clone(),
            kind: crate::model::RelationKind::Table,
        };
        let outgoing: Vec<_> = table.constraints.iter().filter_map(link_parts).collect();
        let incoming: Vec<_> = self
            .snapshot
            .tables
            .iter()
            .flat_map(|other| {
                other.constraints.iter().filter_map(move |c| match c {
                    Constraint::ForeignKey {
                        columns, target, ..
                    } if resolve(&self.snapshot, target) == Some(self.selected) => {
                        Some(format!("{}.{}", other.name, columns.join(", ")))
                    }
                    _ => None,
                })
            })
            .collect();
        let relationships = outgoing.len() + incoming.len();
        let muted = cx.theme().muted_foreground;
        details = details.child(
            v_flex()
                .gap_3()
                .p_4()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_ellipsis()
                                .child(table.name.clone()),
                        )
                        .child(div().text_xs().text_color(muted).child(format!(
                            "{} {} · {} {}",
                            table.columns.len(),
                            plural(table.columns.len(), "column", "columns"),
                            relationships,
                            plural(relationships, "relationship", "relationships"),
                        ))),
                )
                .child(
                    Button::new("er-open-table")
                        .outline()
                        .small()
                        .icon(IconName::ArrowUpRight)
                        .label("Open table")
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(OpenDiagramTable(table_ref.clone()))
                        })),
                ),
        );
        let section = |title: &'static str| {
            v_flex()
                .gap_1()
                .px_4()
                .py_3()
                .child(div().pb_1().child(section_label(title, cx)))
        };
        let mut columns = section("Columns");
        for column in &table.columns {
            let pk = primary(table, &column.name, column.primary_key_position);
            let fk = is_foreign(table, &column.name);
            columns = columns.child(
                h_flex()
                    .h_7()
                    .gap_2()
                    .text_sm()
                    .child(div().w_4().flex_shrink_0().children((pk || fk).then(|| {
                        Icon::new(if pk {
                            IconName::KeyRound
                        } else {
                            IconName::Link
                        })
                        .xsmall()
                        .text_color(if pk {
                            cx.theme().warning
                        } else {
                            accent_text(cx)
                        })
                    })))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .child(column.name.clone()),
                    )
                    .when(!column.nullable, |row| {
                        row.child(div().text_xs().text_color(muted).child("not null"))
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_xs()
                            .text_color(muted)
                            .child(column.data_type.to_lowercase()),
                    ),
            );
        }
        details = details.child(columns);
        if relationships > 0 {
            let mut links = section("Relationships");
            for (index, (label, from, to, actions)) in outgoing.into_iter().enumerate() {
                links = links.child(
                    v_flex()
                        .id(("er-relationship", index))
                        .aria_label(label)
                        .gap_0p5()
                        .py_1()
                        .child(
                            h_flex()
                                .gap_1p5()
                                .text_sm()
                                .child(div().child(from))
                                .child(Icon::new(IconName::ArrowRight).xsmall().text_color(muted))
                                .child(div().min_w_0().text_ellipsis().child(to)),
                        )
                        .child(div().text_xs().text_color(muted).child(actions)),
                );
            }
            for source in incoming {
                links = links.child(
                    h_flex()
                        .gap_1p5()
                        .py_1()
                        .text_sm()
                        .child(Icon::new(IconName::ArrowLeft).xsmall().text_color(muted))
                        .child(div().text_color(muted).child("Referenced by"))
                        .child(div().min_w_0().text_ellipsis().child(source)),
                );
            }
            details = details.child(links);
        }
        if !table.unsupported.is_empty() {
            details = details.child(
                section("Coverage").children(table.unsupported.iter().enumerate().map(
                    |(index, warning)| {
                        notice(("er-coverage", index), Tone::Warning, warning.clone(), cx)
                    },
                )),
            );
        }
        let mut nudge = h_flex().gap_1();
        for (id, label, icon, dx, dy) in [
            ("left", "Move left", IconName::ArrowLeft, -20., 0.),
            ("up", "Move up", IconName::ArrowUp, 0., -20.),
            ("down", "Move down", IconName::ArrowDown, 0., 20.),
            ("right", "Move right", IconName::ArrowRight, 20., 0.),
        ] {
            nudge = nudge.child(icon_button(id, icon, label).small().outline().on_click(
                cx.listener(move |this, _, _, cx| {
                    if let Some(p) = this.positions.get_mut(this.selected) {
                        p.0 += dx;
                        p.1 += dy;
                    }
                    cx.notify();
                }),
            ));
        }
        details.child(section("Position").child(nudge))
    }
}
fn is_foreign(table: &crate::schema::Table, name: &str) -> bool {
    table.constraints.iter().any(|k| {
        matches!(k, Constraint::ForeignKey { columns, .. } if columns.iter().any(|c| c == name))
    })
}
fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}
/// Accessible summary, source columns, destination and referential actions of a foreign key.
fn link_parts(constraint: &Constraint) -> Option<(String, String, String, String)> {
    let label = relationship(constraint)?;
    let Constraint::ForeignKey {
        columns,
        target,
        referenced_columns,
        on_delete,
        on_update,
        ..
    } = constraint
    else {
        return None;
    };
    Some((
        label,
        columns.join(", "),
        format!("{target}.{}", referenced_columns.join(", ")),
        format!(
            "On delete {} · on update {}",
            on_delete.to_lowercase(),
            on_update.to_lowercase()
        ),
    ))
}
fn relationship(constraint: &Constraint) -> Option<String> {
    if let Constraint::ForeignKey {
        columns,
        target,
        referenced_columns,
        on_delete,
        on_update,
        ..
    } = constraint
    {
        Some(format!(
            "({}) → {} ({}) · DELETE {} · UPDATE {}",
            columns.join(", "),
            target,
            referenced_columns.join(", "),
            on_delete,
            on_update
        ))
    } else {
        None
    }
}
impl Render for ErDiagram {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.search.is_none() {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search tables"));
            self.search_subscription =
                Some(cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()));
            self.search = Some(search);
        }
        if self.pending_fit {
            cx.on_next_frame(window, |this, _, cx| {
                if this.pending_fit && this.viewport.get().0 > 0. {
                    this.pending_fit = false;
                    this.fit();
                    cx.notify();
                }
            });
        }
        let tables = self.snapshot.tables.len();
        let relationships = self.routes().len();
        let subtitle = format!(
            "{} · {} {} · {} {}",
            self.snapshot.scope.namespace,
            tables,
            plural(tables, "table", "tables"),
            relationships,
            plural(relationships, "relationship", "relationships"),
        );
        let mut actions: Vec<AnyElement> = Vec::new();
        if !self.export_message.is_empty() {
            actions.push(
                div()
                    .id("er-export-status")
                    .role(Role::Status)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.export_message.clone())
                    .into_any_element(),
            );
        }
        actions.push(
            icon_button("er-refresh", IconName::RefreshCw, "Refresh schema")
                .small()
                .on_click(cx.listener(|_, _, _, cx| cx.emit(RefreshDiagram)))
                .into_any_element(),
        );
        actions.push(
            Button::new("er-export")
                .ghost()
                .small()
                .icon(IconName::Download)
                .label("Export SVG")
                .on_click(cx.listener(|this, _, window, cx| {
                    let svg = this.svg();
                    let receiver =
                        cx.prompt_for_new_path(std::path::Path::new("."), Some("schema.svg"));
                    cx.spawn_in(window, async move |view, cx| {
                        let result = receiver.await;
                        let _ = view.update_in(cx, |this, _, cx| {
                            this.export_message = match result {
                                Ok(Ok(Some(path))) => {
                                    match crate::export::atomic_write(&path, |file| {
                                        use std::io::Write;
                                        file.write_all(svg.as_bytes())?;
                                        Ok(())
                                    }) {
                                        Ok(()) => "SVG saved".into(),
                                        Err(error) => format!("Export failed: {error}"),
                                    }
                                }
                                Ok(Err(error)) => format!("Export failed: {error}"),
                                _ => String::new(),
                            };
                            cx.notify();
                        });
                    })
                    .detach();
                }))
                .into_any_element(),
        );
        actions.push(toolbar_divider(cx).into_any_element());
        actions.push(
            icon_button("close-diagram", IconName::X, "Close schema relationships")
                .small()
                .on_click(cx.listener(|_, _, _, cx| cx.emit(CloseDiagram)))
                .into_any_element(),
        );
        let list = self.table_list(cx);
        let canvas = self.canvas_view(cx);
        let details = self.details(cx);
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(page_header("Relationships", subtitle, actions, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(list)
                    .child(v_flex().h_full().flex_1().min_w_0().child(canvas))
                    .child(details),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::{Constraint, relationship};
    #[test]
    fn composite_qualified_relationship_preserves_identity_and_order() {
        let c = Constraint::ForeignKey {
            columns: vec!["tenant".into(), "parent".into()],
            target: "\"sales\".\"orders\"".into(),
            referenced_columns: vec!["tenant".into(), "id".into()],
            on_delete: "CASCADE".into(),
            on_update: "NO ACTION".into(),
            deferral: None,
        };
        assert_eq!(
            relationship(&c).unwrap(),
            "(tenant, parent) → \"sales\".\"orders\" (tenant, id) · DELETE CASCADE · UPDATE NO ACTION"
        );
    }
}

#[cfg(test)]
mod canvas_tests {
    use super::{Constraint, Snapshot, WIDTH, height, layout, primary, resolve, route};
    fn snapshot() -> Snapshot {
        Snapshot {
            engine: crate::schema::Engine::Postgres,
            engine_version: String::new(),
            scope: crate::schema::Scope {
                namespace: "sales.eu".into(),
            },
            tables: ["z", "orders.old", "a", "b"]
                .into_iter()
                .map(|name| crate::schema::Table {
                    constraint_names: Vec::new(),
                    name: name.into(),
                    columns: vec![],
                    definition: None,
                    indexes: vec![],
                    constraints: vec![],
                    dependencies: vec![],
                    options: vec![],
                    unsupported: vec![],
                })
                .collect(),
        }
    }
    #[test]
    fn moved_layout_restores_by_name_after_refresh() {
        let mut diagram = super::ErDiagram::new(snapshot());
        diagram.selected = 1;
        diagram.positions[1] = (123., 456.);
        diagram.zoom = 1.7;
        diagram.pan = (-30., 80.);
        let saved = serde_json::to_string(&diagram.saved_layout()).unwrap();
        let mut refreshed = snapshot();
        refreshed.tables.reverse();
        let mut restored = super::ErDiagram::new(refreshed);
        restored.restore_layout(serde_json::from_str(&saved).unwrap());
        let index = restored
            .snapshot
            .tables
            .iter()
            .position(|t| t.name == "orders.old")
            .unwrap();
        assert_eq!(restored.positions[index], (123., 456.));
        assert_eq!(restored.zoom, 1.7);
        assert_eq!(restored.pan, (-30., 80.));
        assert!(restored.svg().contains("123"));
    }
    #[test]
    fn layout_is_stable_and_nonoverlapping() {
        let s = snapshot();
        let p = layout(&s);
        assert_eq!(p, layout(&s));
        assert_eq!(p[2], (0., 60.));
        for i in 0..p.len() {
            for j in i + 1..p.len() {
                assert!((p[i].0 - p[j].0).abs() >= WIDTH || (p[i].1 - p[j].1).abs() >= height(0));
            }
        }
    }
    #[test]
    fn quoted_dots_and_schema_identity() {
        let s = snapshot();
        assert_eq!(resolve(&s, "\"sales.eu\".\"orders.old\""), Some(1));
        assert_eq!(resolve(&s, "\"other\".\"orders.old\""), None);
        assert_eq!(resolve(&s, "orders.old"), Some(1));
    }
    #[test]
    fn self_cycles_and_composite_ports() {
        let a = (240., 48.);
        let b = (570., 72.);
        let self_edge = route(a, a, 0);
        assert!(self_edge.len() > 4);
        assert_eq!(self_edge.first(), self_edge.last());
        for (source, target) in [(a, b), (b, a), ((240., 72.), (570., 96.))] {
            let edge = route(source, target, 1);
            assert_eq!(edge[0], source);
            assert_eq!(*edge.last().unwrap(), target);
        }
    }
    #[test]
    fn postgres_primary_constraint_marks_zero_position() {
        let mut s = snapshot();
        s.tables[0].constraints.push(Constraint::Primary {
            columns: vec!["id".into()],
        });
        assert!(primary(&s.tables[0], "id", 0));
        assert!(!primary(&s.tables[0], "other", 0));
    }
}
