# Advanced table views — decisions and validation

## Predicates and sorting

- Recursive All/Any expressions compile with bound values; count and data reuse the same compiled predicate. Relationship scope is appended with AND outside the parenthesized expression.
- Ordered sorts append omitted primary-key columns. Keyless tables cannot promise stable order among equal values.
- Apply stages a candidate filter. Failed execution retains the applied filter and loaded rows; draft conditions remain repairable. Header sorting adds/updates columns; controls expose precedence, direction, removal, and earlier placement.
- Native condition controls are extracted into `src/ui/table_views.rs` under the document module.

Commands and outcomes:
- `pg_ctl -D /tmp/queri-foundations-pg -l /tmp/queri-views-pg.log -o '-h 127.0.0.1 -p 55483 -k /tmp/queri-foundations-pg' start` — passed.
- `cargo test --locked --features test-support filter` — passed (3 unit/native, 3 SQLite; PostgreSQL ignored in this invocation; backdrop executable also passed).
- `QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --test database grouped_views -- --include-ignored` — 2 passed, SQLite and PostgreSQL. Nested logic/count, NULL exclusion, multiple sorts/composite PK pagination, injection-shaped values, missing columns and empty groups.
- `cargo test --locked --features test-support --lib grouped_filter_draft_apply_and_sort_precedence` — 1 passed. Draft isolation, apply, invalid candidate recovery and sort precedence.

## Layout and restart restoration

- Full source data stays in the grid. An explicit display-to-source map drives rendering, selection copy, inspector, and export projection; row mutation and relationship callers retain original rows.
- Search defaults to visible columns in display order, invalidates when layout changes, and explicitly includes hidden cells on request (hidden matches identify their column).
- Draft JSON gains an optional settings object; old drafts default to no settings. Restored invalid predicates remain intact. Metadata-only recovery makes repairs possible without an unfiltered data query.
- Layout names are validated; missing names produce a repair message and empty display, with explicit remove/reset controls.

Commands and outcomes:
- `cargo test --locked --features test-support --lib table_layout_maps` — 1 passed (native hide/reorder, copy source mapping, exact export projection, search visibility/invalidation, draft round-trip).
- `cargo test --locked --features test-support --lib restored_missing_predicate` — 1 passed (restoration repair without unfiltered rows).
- `cargo test --locked --test storage table_view_drafts` — 1 passed after correcting the test fixture's serialized relation kind to `table`.
- `cargo test --locked --features test-support --lib row_inspector_inserts_edits_and_deletes_real_rows` — 1 passed with hidden/reordered columns before editing/deleting.
- `cargo test --locked --features test-support --lib saved_library_sql_files_and_related_tabs_keep_work_separate` — 1 passed with hidden/reordered columns before relationship navigation.
