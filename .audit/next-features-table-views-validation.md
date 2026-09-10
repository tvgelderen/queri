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

## Named views and final validation

- Storage schema 4 adds named table views. Updates and deletes check revisions; updates also check immutable connection/database/table association. Connection deletion sets the association to NULL and keeps the view. Database rename updates associations and revisions.
- Save/Save as capture applied settings. Relationship-scoped tabs require an explicit reusable-settings/include-record-scope choice on every save. Saving does not change the active scope. The library identifies scoped views, other-table views, and unavailable orphans; only the original live connection/table can load a view.
- Drafts retain the loaded saved-view revision. Invalid schema references remain serialized and editable. No autocomplete, parameter-entry UI, or generated-SQL placeholder was added.
- The existing OFFSET/page-number interaction is retained; primary-key tie-breakers stabilize ties but do not provide snapshot isolation across page requests. Deep offsets still have the existing scan cost. Keyless-table ties remain unspecified.

Commands and outcomes:
- `cargo test --locked --features test-support --test storage` — 13 passed, including v1/v3 migration, old drafts, restart, save conflicts, association changes, database rename, and orphans.
- `cargo test --locked --features test-support --lib named_ -- --nocapture` — 2 passed: Save/Save as/load, stale revision recovery, and explicit relationship-scope choices. Initial test runs exposed missing test registration and dialog/dropdown transition timing; corrected and rerun successfully.
- `QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --features test-support --test database grouped_views -- --include-ignored` — 2 passed, now also proving composite relationship scope AND an OR expression on SQLite and PostgreSQL.
- `cargo test --locked --features test-support --lib nested_filter_controls -- --nocapture` — 1 passed: native nested-group construction and NULL filter-from-cell.
- `cargo test --locked --features test-support --lib adding_condition_preserves` — 1 passed: adding to an already applied predicate retains it.
- `cargo test --locked --features test-support --lib missing_layout_column` — 1 passed: schema drift retains the missing layout entry until explicit repair.
- Initial full check stopped at Clippy's large-enum and collapsible-if findings. Boxed the large optional document state and applied the suggested control-flow simplifications; no lint suppressions.
- Final `QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' bash scripts/check.sh` — **exit 0**. Formatting, strict Clippy, 56 unit/native tests, 3 backdrop-scene tests, 18 SQLite/database tests, 13 storage tests, light/dark GPU rendering, 5 PostgreSQL-enabled unit/native tests, 14 PostgreSQL integration tests, and desktop launcher validation passed. The initial default test phase's ignored PostgreSQL tests ran in the script's PostgreSQL-enabled phases.
- `pg_ctl -D /tmp/queri-foundations-pg stop` — passed; designated server stopped.
- `git diff --check` — passed.

## Later parameterized SQL handoff

Reuse `BrowseOptions` (relationship scope, recursive `FilterExpression`, ordered sorts, pagination) and `TableViewSettings`; layout must remain presentation-only. `FilterExpression::compile` is the shared tree walker, and each backend already compiles one predicate/parameter sequence reused by count and data. The later dependent slice should extract each backend's statement-plus-bind construction into a reusable result, including dialect-specific scalar casts and relationship binds, then hand SQL plus typed bindings to the parameter editor. Preserve connection/database association and existing submission safeguards/timeouts. No public generated-query API was needed by this milestone.
