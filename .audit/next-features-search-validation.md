# Milestone 2: loaded-grid and live SQL-draft search

Validated 2026-11-11 in `/home/timen/.t3/worktrees/queri/t3-1bd6330c-c90d6af7529ecebe`, based on foundation commit `42a1cbe`. Scope is UI/search models/tests/documentation only; no driver, database, dependency, or vendor changes.

## Decisions

- Inspected README, document/grid/workspace/cell-inspector/table-finder implementations, existing native tests, and the pinned GPUI input search, selection, list, and scrolling APIs. Used the feedback-loop skill. No subagents or PRs.
- Enabled the existing GPUI SQL-editor search panel. Kept inspector search intact. Grid Ctrl+F has a separate search cursor: scroll/highlight never changes checked rows or rectangle/copy selection. Counts and navigation are explicitly **matching cells**, not occurrences within a cell.
- Grid scope is the current loaded result/page and its displayed data columns (all data columns today), excluding checkbox controls. It searches complete `Cell::text()` values, including text beyond inspector preview limits. No database call is involved.
- Extracted `grid_search.rs`, `draft_search.rs`, and a small pure literal matcher instead of growing document/workspace with the search implementation. Reused GPUI textarea, button, editor search, and virtualized list components. The built-in search panel is private to GPUI and tied to an editor, so the grid uses those public primitives.
- Ctrl+Shift+S / **Search SQL drafts** avoids the existing Ctrl+Shift+F formatting shortcut. Results carry stable document identity, an immutable live Rope snapshot, UTF-8 byte range, tab title, connection/database, and bounded snippet. Activation validates the document and text, switches tabs, selects the original range, and focuses the editor without connecting or executing SQL.
- Both new scans run on GPUI's background executor, with a cancellable 100 ms delay and generation checks. Grid completion additionally checks result-set identity. Editor events and tab changes invalidate draft results. Activation checks the source snapshot again to reject stale offsets.
- Draft searches show the first 1,000 matches, clearly labelled when capped; one extra result detects truncation. Snippets are bounded. Grid counts are uncapped within the already-loaded dataset.
- The new matcher uses literal text and Unicode lowercase matching with original byte-offset mapping, not regex or locale-specific/full Unicode case folding. Multiline query text is preserved by GPUI textarea controls. Grid Shift+Enter navigates backwards; draft Shift+Enter can insert a newline, and Tab moves into the existing list for arrow navigation.

## Focused validation

Final command:

```sh
cargo test --locked --features test-support --lib search -- --nocapture
```

Exit 0: **7 passed, 0 failed, 32 filtered out**. Final output: `/tmp/queri-search-debounce-focused.log`.

- `literal_unicode_multiline_offsets`: literal metacharacters, empty/no matches, case sensitivity, Unicode lowercase expansion, multiline UTF-8 offsets, non-overlapping matches.
- `complete_cells_and_displayed_columns`: complete value beyond 1 MiB, Unicode/multiline query, case toggle, NULL text, exclusion of columns outside the displayed data schema.
- `draft_search_preserves_identity_offsets_and_bounds_broad_results`: distinct document identities despite identical titles, source ranges, connection/database context, snippets, and bounded broad-query results. Uses GPUI only to allocate entity IDs; this is a model test, not a GUI interaction claim.
- `grid_search_preserves_selection_wraps_and_invalidates`: native Ctrl+F/text/Enter/Shift+Enter/button/Escape events; matching-cell counts, wrapping, case toggle, checked rows, actual rectangle selection and clipboard content, grid focus recovery, rapid query changes, and result-set replacement.
- `editor_search_and_format_shortcuts_remain_separate`: native SQL-editor Ctrl+F, match navigation, Escape focus recovery, and Ctrl+Shift+F formatting. Checks the existing search match cursor, not an invented editor text-selection behavior.
- `inspector_search_keeps_document_search_and_sql_untouched`: native inspector focus/click/search/escape; validates inspector match state, originating editor focus, and unchanged SQL/document search state.
- `workspace_search_live_drafts_selects_text_offline_and_drops_closed_tabs`: native shortcut, multiline query, cancel/focus restoration, Tab/Down/Enter list navigation; verifies correct tab identity, actual selected text and byte offset, actual editor focus, edit/closed-tab invalidation, and no connection/history/database-file creation.

During implementation, initial native checks exposed a real dialog-focus issue and missing textarea-to-list Tab routing; both were fixed and retested. Initial test assertions also incorrectly assumed synchronous InputEvent delivery and that the existing editor search selects text; those checks were corrected to observe delivered events and its actual active-match API. Compilation caught the InputEvent shift field and a focus borrow, both corrected. Temporary debugging output was removed.

## Repository and backend validation

Commands run:

```sh
cargo check --locked --features test-support
cargo clippy --locked --all-targets --features test-support -- -D warnings
pg_ctl -D /tmp/queri-foundations-pg -l /tmp/queri-search-pg.log \
  -o '-h 127.0.0.1 -p 55483 -k /tmp/queri-foundations-pg' start
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' \
  bash scripts/check.sh
pg_ctl -D /tmp/queri-foundations-pg -m fast stop
git diff --check
```

All final commands exited 0. The full script was rerun after final implementation/test changes; authoritative final output is `/tmp/queri-search-release-check.log`:

- Formatting and strict all-target Clippy passed.
- Library/native suite: **37 passed, 2 ignored** (PostgreSQL-only tests subsequently run explicitly).
- Backdrop scene tests: **3 passed**.
- SQLite/database default suite: **14 passed, 8 ignored** (PostgreSQL tests subsequently run explicitly).
- Real-GPU dialog executable passed light/dark blur, foreground, stacking, resize, live updates, Escape, reduced motion, and exit checks.
- Storage: **9 passed**.
- PostgreSQL-filtered library suite with ignored tests enabled: **3 passed**, including both PostgreSQL workspace interaction tests.
- PostgreSQL backend integration suite with ignored tests enabled: **8 passed**, including authoritative transaction status, text-bind fidelity/errors/limits, cancellation/abandoned work, formatting, relationships, filters, database lifecycle/auth failure, and real workflows.
- Desktop launcher validation passed; doc tests passed (none defined).
- Only the designated disposable PostgreSQL cluster was started, and it was stopped after checks. No unrelated server was touched.

## Limits

Parent integration review found inconsistent contextual lowercasing of Greek
sigma between the query and searched text. Added a regression asserting that
case-insensitive search for `ΟΣ` finds identical text. It failed before the fix.
Both sides now use per-character Unicode lowercasing. Parent reran
`cargo test --locked --features test-support --lib search` (7 passed) and
`cargo fmt --all -- --check` (passed). The independent review found no other
actionable issues. The implementation worktree and temporary branch were removed
after verifying the integrated commit has identical contents.

Search interactions were validated through native GPUI event dispatch and state/focus/selection assertions, not static markup checks. No manual physical-desktop or search-specific pixel-layout review was performed. The existing real-GPU check validates shared dialogs, not every new search layout. Grid navigation counts cells; draft search intentionally caps broad results at 1,000. Unicode matching uses lowercase semantics, not language-specific collation. Existing SQL-editor/inspector search behavior remains GPUI's implementation.
