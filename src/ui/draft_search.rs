use super::{
    document::{Document, DocumentChanged},
    search::literal_ranges,
};
use gpui_kit::{
    component::{
        ActiveTheme, IndexPath, WindowExt,
        input::{InputEvent, Rope, Textarea, TextareaState},
        list::{List, ListDelegate, ListItem, ListState},
        v_flex,
    },
    *,
};
use std::ops::Range;

const MATCH_LIMIT: usize = 1000;
actions!(queri_draft_search, [FocusDraftMatches, FocusDraftQuery]);

#[derive(Clone)]
pub struct DraftMatch {
    pub document: EntityId,
    pub text: Rope,
    pub range: Range<usize>,
    label: String,
    context: String,
    snippet: String,
}
pub struct OpenDraftMatch(pub DraftMatch);
impl EventEmitter<OpenDraftMatch> for DraftSearch {}
impl EventEmitter<OpenDraftMatch> for ListState<DraftMatches> {}

struct DraftSnapshot {
    document: EntityId,
    text: Rope,
    label: String,
    context: String,
}
fn search_drafts(drafts: Vec<DraftSnapshot>, query: &str) -> Vec<DraftMatch> {
    let mut matches = vec![];
    for draft in drafts {
        let text = draft.text.to_string();
        for range in literal_ranges(&text, query, false, MATCH_LIMIT + 1 - matches.len()) {
            let start = text[..range.start]
                .char_indices()
                .rev()
                .nth(40)
                .map_or(0, |(i, _)| i);
            matches.push(DraftMatch {
                document: draft.document,
                text: draft.text.clone(),
                range,
                label: draft.label.clone(),
                context: draft.context.clone(),
                snippet: text[start..]
                    .chars()
                    .take(160)
                    .collect::<String>()
                    .replace('\n', " ↵ "),
            });
        }
        if matches.len() > MATCH_LIMIT {
            break;
        }
    }
    matches
}

pub struct DraftSearch {
    input: Entity<TextareaState>,
    list: Entity<ListState<DraftMatches>>,
    documents: Vec<WeakEntity<Document>>,
    generation: u64,
    task: Option<Task<()>>,
    searching: bool,
    _subscriptions: Vec<Subscription>,
    document_subscriptions: Vec<Subscription>,
}
impl DraftSearch {
    pub fn new(
        documents: &[Entity<Document>],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new("tab", FocusDraftMatches, Some("QueriDraftSearch > Input")),
            KeyBinding::new(
                "shift-tab",
                FocusDraftQuery,
                Some("QueriDraftSearch > List"),
            ),
        ]);
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 3)
                .submit_on_enter(true)
                .placeholder("Search live SQL drafts…")
        });
        let list =
            cx.new(|cx| ListState::new(DraftMatches::default(), window, cx).searchable(false));
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Change => this.search(window, cx),
                        InputEvent::PressEnter { .. } => this.list.update(cx, |list, cx| {
                            list.delegate_mut().confirm(false, window, cx)
                        }),
                        _ => {}
                    }
                }),
                cx.subscribe(&list, |_, _, event: &OpenDraftMatch, cx| {
                    cx.emit(OpenDraftMatch(event.0.clone()))
                }),
            ];
        let mut this = Self {
            input,
            list,
            documents: vec![],
            generation: 0,
            task: None,
            searching: false,
            _subscriptions: subscriptions,
            document_subscriptions: vec![],
        };
        this.set_documents(documents, window, cx);
        this
    }
    pub(super) fn match_count(&self, cx: &App) -> usize {
        self.list.read(cx).delegate().matches.len().min(MATCH_LIMIT)
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.read(cx).focus_handle(cx).focus(window, cx);
    }
    pub fn set_documents(
        &mut self,
        documents: &[Entity<Document>],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.documents = documents
            .iter()
            .filter(|doc| doc.read(cx).sql_editor().is_some())
            .map(Entity::downgrade)
            .collect();
        self.document_subscriptions = documents
            .iter()
            .map(|doc| {
                cx.subscribe_in(doc, window, |this, _, _: &DocumentChanged, window, cx| {
                    this.search(window, cx)
                })
            })
            .collect();
        self.search(window, cx);
    }
    fn search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.task = None;
        self.list.update(cx, |list, cx| {
            list.delegate_mut().matches.clear();
            list.set_selected_index(None, window, cx);
            cx.notify();
        });
        let query = self.input.read(cx).value().to_string();
        self.searching = !query.is_empty();
        cx.notify();
        if !self.searching {
            return;
        }
        let drafts = self
            .documents
            .iter()
            .filter_map(|weak| {
                let doc = weak.upgrade()?;
                let state = doc.read(cx);
                let editor = state.sql_editor()?;
                Some(DraftSnapshot {
                    document: doc.entity_id(),
                    text: editor.read(cx).text().clone(),
                    label: state.title(),
                    context: format!("{} / {}", state.profile.name, state.profile.database()),
                })
            })
            .collect();
        let generation = self.generation;
        // Dropping the previous task cancels this delay before a new scan starts.
        let debounce = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(100));
        let work = cx.background_executor().spawn(async move {
            debounce.await;
            search_drafts(drafts, &query)
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let matches = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.searching = false;
                this.list.update(cx, |list, cx| {
                    list.delegate_mut().matches = matches;
                    let selection =
                        (!list.delegate().matches.is_empty()).then_some(IndexPath::default());
                    list.set_selected_index(selection, window, cx);
                    cx.notify();
                });
                cx.notify();
            });
        }));
    }
}
impl Render for DraftSearch {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("draft-search")
            .key_context("QueriDraftSearch")
            .on_action(cx.listener(|this, _: &FocusDraftMatches, window, cx| {
                this.list.update(cx, |list, cx| list.focus(window, cx))
            }))
            .on_action(cx.listener(|this, _: &FocusDraftQuery, window, cx| this.focus(window, cx)))
            .gap_3()
            .child(Textarea::new(&self.input).aria_label("Search live SQL drafts"))
            .child(
                div()
                    .id("draft-search-count")
                    .text_sm()
                    .child(if self.searching {
                        "Searching…".into()
                    } else {
                        format!(
                            "{} matches · live SQL drafts · case-insensitive literal text",
                            if self.list.read(cx).delegate().matches.len() > MATCH_LIMIT {
                                "First 1,000".into()
                            } else {
                                self.match_count(cx).to_string()
                            }
                        )
                    }),
            )
            .child(div().h(px(320.)).child(List::new(&self.list)))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Enter to open · Tab then ↑ ↓ to navigate · Esc to close"),
            )
    }
}
#[derive(Default)]
struct DraftMatches {
    matches: Vec<DraftMatch>,
    selected: Option<IndexPath>,
}
impl ListDelegate for DraftMatches {
    type Item = ListItem;
    fn items_count(&self, _: usize, _: &App) -> usize {
        self.matches.len().min(MATCH_LIMIT)
    }
    fn set_selected_index(
        &mut self,
        index: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected = index;
    }
    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        if let Some(item) = self.selected.and_then(|index| self.matches.get(index.row)) {
            cx.emit(OpenDraftMatch(item.clone()));
        }
    }
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        window.close_dialog(cx);
    }
    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let item = self.matches.get(index.row)?;
        Some(
            ListItem::new(index)
                .selected(self.selected == Some(index))
                .accessibility_label(format!("{}: {}", item.label, item.snippet))
                .child(
                    v_flex()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .child(format!("{} · {}", item.label, item.context)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_ellipsis()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(item.snippet.clone()),
                        ),
                ),
        )
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .p_4()
            .child("No matches. Enter text to search open SQL drafts.")
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::{DraftSnapshot, MATCH_LIMIT, search_drafts};
    use gpui_kit::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn draft_search_preserves_identity_offsets_and_bounds_broad_results(cx: &mut TestAppContext) {
        let first = cx.new(|_| ());
        let second = cx.new(|_| ());
        let snapshot = |document, text: &str| DraftSnapshot {
            document,
            text: text.into(),
            label: "Same tab title".into(),
            context: "Connection / database".into(),
        };
        let matches = search_drafts(
            vec![
                snapshot(first.entity_id(), "é Ä\nneedle"),
                snapshot(second.entity_id(), "Ä\nneedle"),
            ],
            "ä\nneedle",
        );
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].document, first.entity_id());
        assert_eq!(matches[1].document, second.entity_id());
        assert_eq!(matches[0].range, 3..12);
        assert_eq!(matches[1].range, 0..9);
        assert_eq!(matches[0].label, "Same tab title");
        assert_eq!(matches[0].context, "Connection / database");
        assert!(matches[0].snippet.contains("Ä ↵ needle"));
        let matches = search_drafts(
            vec![snapshot(first.entity_id(), &"needle ".repeat(10_000))],
            "needle",
        );
        assert_eq!(
            matches.len(),
            MATCH_LIMIT + 1,
            "one extra hit detects truncation without allocating every hit"
        );
        assert!(matches.iter().all(|hit| hit.snippet.chars().count() <= 160));
    }
}
