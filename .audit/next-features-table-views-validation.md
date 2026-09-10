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
