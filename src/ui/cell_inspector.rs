use super::{Services, task};
use crate::{
    cell_view::{JsonNode, hex_line},
    model::Cell,
};
use gpui_kit::component::{
    ActiveTheme, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Editor, EditorState},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Raw,
    Text,
    Json,
    Formatted,
    Hex,
}
struct TreeRow {
    path: Vec<usize>,
    depth: usize,
    label: String,
    branch: bool,
}
struct Inspector {
    cell: Cell,
    binary: Option<Vec<u8>>,
    editor: Entity<EditorState>,
    mode: Mode,
    raw: String,
    json: Option<Arc<JsonNode>>,
    expanded: HashSet<Vec<usize>>,
    rows: Vec<TreeRow>,
    message: String,
    loading: bool,
}
pub fn open(
    services: Services,
    cell: Cell,
    title: String,
    data_type: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let binary_cell = matches!(&cell, Cell::Blob(_)) || data_type.as_deref() == Some("bytea");
    let inspector = cx.new(|cx| Inspector {
        cell: cell.clone(),
        binary: None,
        editor: cx.new(|cx| {
            EditorState::new(window, cx)
                .searchable(true)
                .soft_wrap(false)
        }),
        mode: Mode::Raw,
        raw: String::new(),
        json: None,
        expanded: HashSet::from([vec![]]),
        rows: vec![],
        message: String::new(),
        loading: true,
    });
    inspector.update(cx, |_, cx| {
        task(
            &services,
            window,
            cx,
            async move {
                Ok(tokio::task::spawn_blocking(move || {
                    let raw = cell.text();
                    let json = if matches!(cell, Cell::Text(_)) {
                        match JsonNode::parse(&raw) {
                            Ok(json) => Some(Ok(json)),
                            Err(error) if raw.trim_start().starts_with(['{', '[']) => {
                                Some(Err(error))
                            }
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                    let binary = match &cell {
                        Cell::Blob(bytes) => Some(Ok(bytes.clone())),
                        Cell::Text(text) if data_type.as_deref() == Some("bytea") => {
                            Some(crate::cell_view::postgres_binary(text))
                        }
                        _ => None,
                    };
                    (raw, json, binary)
                })
                .await?)
            },
            |this, result, window, cx| {
                this.loading = false;
                match result {
                    Ok((raw, json, binary)) => {
                        match binary {
                            Some(Ok(bytes)) => this.binary = Some(bytes),
                            Some(Err(e)) => {
                                this.message = format!("Binary preview unavailable: {e}")
                            }
                            None => {}
                        }
                        this.raw = raw;
                        match json {
                            Some(Ok(json)) => this.json = Some(Arc::new(json)),
                            Some(Err(e)) => this.message = format!("Raw value shown: {e}"),
                            None => {}
                        }
                        this.rebuild();
                        this.set_mode(this.mode, window, cx);
                    }
                    Err(e) => this.message = format!("Could not prepare preview: {e:#}"),
                }
                cx.notify();
            },
        );
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .when(binary_cell, |dialog| dialog.w(px(720.)))
            .child(inspector.clone())
    });
}
impl Inspector {
    fn rebuild(&mut self) {
        fn walk(
            node: &JsonNode,
            path: Vec<usize>,
            name: String,
            expanded: &HashSet<Vec<usize>>,
            rows: &mut Vec<TreeRow>,
        ) {
            let (summary, branch) = match node {
                JsonNode::Scalar(value) => (value.clone(), false),
                JsonNode::Array(v) => (format!("[{} items]", v.len()), true),
                JsonNode::Object(v) => (format!("{{{} fields}}", v.len()), true),
            };
            let label = format!("{name}{summary}");
            rows.push(TreeRow {
                depth: path.len(),
                path: path.clone(),
                label,
                branch,
            });
            if !expanded.contains(&path) {
                return;
            }
            match node {
                JsonNode::Object(values) => {
                    for (i, (key, node)) in values.iter().enumerate() {
                        let mut p = path.clone();
                        p.push(i);
                        walk(node, p, format!("{key}: "), expanded, rows);
                    }
                }
                JsonNode::Array(values) => {
                    for (i, node) in values.iter().enumerate() {
                        let mut p = path.clone();
                        p.push(i);
                        walk(node, p, format!("{i}: "), expanded, rows);
                    }
                }
                _ => {}
            }
        }
        self.rows.clear();
        if let Some(json) = &self.json {
            walk(json, vec![], String::new(), &self.expanded, &mut self.rows);
        }
    }
    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        let limit = 1024 * 1024;
        let mut end = self.raw.len().min(limit);
        while !self.raw.is_char_boundary(end) {
            end -= 1;
        }
        let value = if mode == Mode::Formatted {
            self.json
                .as_ref()
                .map(|json| json.formatted())
                .unwrap_or_default()
        } else {
            self.raw[..end].to_owned()
        };
        self.editor.update(cx, |editor, cx| {
            editor.set_soft_wrap(mode == Mode::Text, window, cx);
            editor.set_value(value, window, cx);
        });
        cx.notify();
    }
}
impl Render for Inspector {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut modes = vec![(Mode::Raw, "Raw"), (Mode::Text, "Text")];
        if self.json.is_some() {
            modes.push((Mode::Json, "JSON"));
            modes.push((Mode::Formatted, "Formatted"));
        }
        if self.binary.is_some() {
            modes.push((Mode::Hex, "Hex"));
        }
        let selected = modes
            .iter()
            .position(|(mode, _)| *mode == self.mode)
            .unwrap_or(0);
        let choices = modes.clone();
        let details = match &self.cell {
            Cell::Null => "NULL · no value".into(),
            Cell::Blob(b) => format!("{} bytes", b.len()),
            Cell::Text(s) if s.is_empty() => "Empty string · 0 characters".into(),
            _ => format!("{} bytes", self.raw.len()),
        };
        v_flex()
            .gap_3()
            .child(
                TabBar::new("cell-view-mode")
                    .segmented()
                    .selected_index(selected)
                    .children(modes.iter().map(|(_, label)| Tab::new().label(*label)))
                    .on_click(cx.listener(move |this, index, window, cx| {
                        if let Some((mode, _)) = choices.get(*index) {
                            this.set_mode(*mode, window, cx)
                        }
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(details),
            )
            .child(
                div().h(px(360.)).min_h_0().child(match self.mode {
                    Mode::Json => uniform_list(
                        "json-tree",
                        self.rows.len(),
                        cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|i| {
                                    let row = &this.rows[i];
                                    let path = row.path.clone();
                                    let expanded = this.expanded.contains(&path);
                                    let depth = f32::from(u16::try_from(row.depth).unwrap_or(64));
                                    h_flex().pl(px(depth * 12.)).child(
                                        Button::new(("json-node", i))
                                            .ghost()
                                            .small()
                                            .accessibility_label(format!(
                                                "{} · {}",
                                                row.label,
                                                if row.branch {
                                                    if expanded { "expanded" } else { "collapsed" }
                                                } else {
                                                    "value"
                                                }
                                            ))
                                            .label(format!(
                                                "{}{}",
                                                if row.branch {
                                                    if expanded { "▾ " } else { "▸ " }
                                                } else {
                                                    ""
                                                },
                                                row.label
                                            ))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if !this.expanded.remove(&path) {
                                                    this.expanded.insert(path.clone());
                                                }
                                                this.rebuild();
                                                cx.notify();
                                            })),
                                    )
                                })
                                .collect()
                        }),
                    )
                    .size_full()
                    .into_any_element(),
                    Mode::Hex => {
                        let count = self
                            .binary
                            .as_ref()
                            .map_or(0, |bytes| bytes.len().div_ceil(16));
                        uniform_list(
                            "hex-lines",
                            count,
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|i| {
                                        let line = this
                                            .binary
                                            .as_ref()
                                            .map(|bytes| hex_line(bytes, i))
                                            .unwrap_or_default();
                                        div()
                                            .text_sm()
                                            .whitespace_nowrap()
                                            .font_family(cx.theme().mono_font_family.clone())
                                            .child(line)
                                    })
                                    .collect()
                            }),
                        )
                        .with_horizontal_sizing_behavior(
                            ListHorizontalSizingBehavior::Unconstrained,
                        )
                        .size_full()
                        .into_any_element()
                    }
                    _ => Editor::new(&self.editor)
                        .h(relative(1.))
                        .readonly(true)
                        .aria_label("Cell contents; Ctrl+F to search")
                        .into_any_element(),
                }),
            )
            .child(
                div()
                    .id("cell-preview-status")
                    .role(Role::Status)
                    .text_sm()
                    .child(if self.loading {
                        "Preparing preview…".into()
                    } else if self.raw.len() > 1024 * 1024 {
                        "Text preview limited to 1 MiB. Copy raw includes the complete value."
                            .into()
                    } else {
                        self.message.clone()
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("copy-raw-cell")
                            .label("Copy raw")
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(this.cell.text()))
                            })),
                    )
                    .when(self.json.is_some(), |bar| {
                        bar.child(
                            Button::new("copy-formatted-cell")
                                .ghost()
                                .label("Copy formatted JSON")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(json) = &this.json {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            json.formatted(),
                                        ));
                                    }
                                })),
                        )
                    }),
            )
    }
}
