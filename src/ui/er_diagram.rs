//! Native schema relationship inspector. Relationships are declared constraints only.
use crate::schema::{Constraint, Snapshot};
use gpui_kit::{
    component::{ActiveTheme, button::Button, h_flex, v_flex},
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
            selected: 0,
        }
    }
}
const WIDTH: f32 = 240.;
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
    fn edges(&self) -> Vec<Vec<(f32, f32)>> {
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
                        edges.push(route(
                            (
                                self.positions[i].0 + WIDTH,
                                self.positions[i].1 + row(i, source),
                            ),
                            (
                                self.positions[j].0 + WIDTH,
                                self.positions[j].1 + row(j, dest),
                            ),
                            edges.len(),
                        ));
                    }
                }
            }
        }
        edges
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
    fn canvas_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let z = self.zoom;
        let pan = self.pan;
        let edges = self.edges();
        let color = cx.theme().foreground;
        let viewport = self.viewport.clone();
        let mut surface = div()
            .id("er-canvas")
            .relative()
            .flex_1()
            .w_full()
            .min_h(px(200.))
            .overflow_hidden()
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
                        for edge in edges {
                            let mut path = PathBuilder::stroke(px(1.5));
                            for (i, (x, y)) in edge.iter().enumerate() {
                                let p = bounds.origin + point(px(x * z + pan.0), px(y * z + pan.1));
                                if i == 0 {
                                    path.move_to(p);
                                } else {
                                    path.line_to(p);
                                }
                            }
                            if let Some(&(x, y)) = edge.last() {
                                let p = bounds.origin + point(px(x * z + pan.0), px(y * z + pan.1));
                                path.move_to(p + point(px(7.), px(-4.)));
                                path.line_to(p);
                                path.line_to(p + point(px(7.), px(4.)));
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, color);
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            );
        for (i, table) in self.snapshot.tables.iter().enumerate() {
            let mut node = v_flex()
                .id(("er-node", i))
                .absolute()
                .left(px(self.positions[i].0 * z + pan.0))
                .top(px(self.positions[i].1 * z + pan.1))
                .w(px(WIDTH * z))
                .h(px(height(table.columns.len()) * z))
                .text_size(px(12. * z))
                .bg(cx.theme().background)
                .border_1()
                .border_color(color)
                .overflow_hidden()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.selected = i;
                        this.drag = Some((Some(i), e.position));
                        cx.notify();
                    }),
                )
                .child(div().h(px(36. * z)).child(format!(
                    "{}{}",
                    if i == self.selected { "● " } else { "" },
                    table.name
                )));
            for c in &table.columns {
                let fk = table.constraints.iter().any(|k| matches!(k, Constraint::ForeignKey { columns, .. } if columns.contains(&c.name)));
                node = node.child(div().h(px(24. * z)).child(format!(
                    "{}{}{}  {}",
                    if primary(table, &c.name, c.primary_key_position) {
                        "PK "
                    } else {
                        ""
                    },
                    if fk { "FK " } else { "" },
                    c.name,
                    c.data_type
                )));
            }
            surface = surface.child(node);
        }
        surface
    }
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
            let search = cx.new(|cx| {
                gpui_kit::component::input::InputState::new(window, cx)
                    .placeholder("Search tables…")
            });
            self.search_subscription = Some(cx.subscribe(
                &search,
                |_, _, _: &gpui_kit::component::input::InputEvent, cx| cx.notify(),
            ));
            self.search = Some(search);
        }
        let query = self
            .search
            .as_ref()
            .unwrap()
            .read(cx)
            .value()
            .to_lowercase();
        let tables = self
            .snapshot
            .tables
            .iter()
            .enumerate()
            .filter(|(_, table)| table.name.to_lowercase().contains(&query))
            .map(|(index, table)| {
                Button::new(("er-table", index))
                    .label(table.name.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = index;
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        let mut details = v_flex().gap_2().p_4();
        if let Some(table) = self.snapshot.tables.get(self.selected) {
            let table_ref = crate::model::TableRef {
                schema: self.snapshot.scope.namespace.clone(),
                name: table.name.clone(),
                kind: crate::model::RelationKind::Table,
            };
            details = details
                .child(div().text_xl().child(table.name.clone()))
                .child(Button::new("er-open-table").label("Open table").on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(OpenDiagramTable(table_ref.clone()))),
                ));
            for column in &table.columns {
                let fk = table.constraints.iter().any(|c| matches!(c, Constraint::ForeignKey { columns, .. } if columns.contains(&column.name)));
                details = details.child(
                    h_flex()
                        .gap_4()
                        .child(format!(
                            "{}{}",
                            if primary(table, &column.name, column.primary_key_position) {
                                "PK "
                            } else {
                                ""
                            },
                            if fk { "FK" } else { "" }
                        ))
                        .child(column.name.clone())
                        .child(column.data_type.clone())
                        .child(if column.nullable { "NULL" } else { "NOT NULL" }),
                );
            }
            for constraint in &table.constraints {
                if let Some(label) = relationship(constraint) {
                    details = details.child(div().child(label));
                }
            }
            for warning in &table.unsupported {
                details = details.child(div().child(format!("Coverage warning: {warning}")));
            }
        }
        let toolbar =
            h_flex()
                .gap_2()
                .child(
                    Button::new("er-zoom-in")
                        .label("Zoom +")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.zoom = (this.zoom * 1.2).min(3.);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("er-zoom-out")
                        .label("Zoom −")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.zoom = (this.zoom / 1.2).max(0.1);
                            cx.notify();
                        })),
                )
                .child(Button::new("er-fit").label("Fit").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.fit();
                        cx.notify();
                    },
                )))
                .child(
                    Button::new("er-arrange")
                        .label("Arrange")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.positions = layout(&this.snapshot);
                            this.fit();
                            cx.notify();
                        })),
                );
        let toolbar = toolbar
            .child(
                Button::new("er-refresh")
                    .label("Refresh")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(RefreshDiagram))),
            )
            .child(
                Button::new("er-export")
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
                    })),
            )
            .child(self.export_message.clone());
        let mut controls = h_flex().gap_2();
        for (id, label, dx, dy) in [
            ("left", "Move ←", -20., 0.),
            ("right", "Move →", 20., 0.),
            ("up", "Move ↑", 0., -20.),
            ("down", "Move ↓", 0., 20.),
        ] {
            controls = controls.child(Button::new(id).label(label).on_click(cx.listener(
                move |this, _, _, cx| {
                    if let Some(p) = this.positions.get_mut(this.selected) {
                        p.0 += dx;
                        p.1 += dy;
                    }
                    cx.notify();
                },
            )));
        }
        let canvas = self.canvas_view(cx);
        h_flex()
            .items_start()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .w(px(220.))
                    .gap_2()
                    .p_3()
                    .child(format!("Schema: {}", self.snapshot.scope.namespace))
                    .child(
                        gpui_kit::component::input::Input::new(self.search.as_ref().unwrap())
                            .aria_label("Search tables"),
                    )
                    .children(tables),
            )
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .child(toolbar)
                    .child(controls)
                    .child(canvas)
                    .child(
                        div()
                            .id("er-details")
                            .flex_1()
                            .h(px(220.))
                            .overflow_y_scroll()
                            .child(details),
                    ),
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
