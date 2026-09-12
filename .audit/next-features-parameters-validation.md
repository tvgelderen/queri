# Query parameters and generated table SQL validation

Worktree: `/home/timen/.t3/worktrees/queri/t3-3a64375b-64f64591cdb0a53c`.
Base: `dded863`. No agents or PRs. Only the designated cluster is used.

## Slice 1 — discovery and execution

- Reuse sqlparser dialect tokenization and source spans; replace only placeholders, retaining all other source bytes. PostgreSQL operators, casts and dollar bodies stay opaque. Repeated named placeholders share an index; positional numbering must be contiguous. Reject mixed styles and parameterized scripts before execution.
- `Session::query_params(BoundQuery)` supports both engines. PostgreSQL retains `query_postgres_params` and exact text protocol. SQLite shares its existing collector and deadline guard. BoundQuery has no serialization implementation; persisted definitions contain only name/type.

Commands/outcomes:
- `pg_ctl -D /tmp/queri-foundations-pg -l /tmp/queri-params-pg.log -o '-h 127.0.0.1 -p 55483 -k /tmp/queri-foundations-pg' start` — server started.
- `cargo test --locked --lib parameters --quiet` — 1 passed.
- `QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --test database parameter_execution -- --include-ignored` — 2 passed. Repeated names, NULL/empty, injection-shaped Unicode, atomic script/count rejection, transaction rollback, SQLite native types and bound timeout.
- Commit: `363304a`.

## Slice 2 — form, definitions, privacy

- Native parameter dialog feeds the shared Document confirmation/execution path. Captured editor SQL and session generation guard both form continuation and confirmation. SQLite validates typed inputs before submitting; PostgreSQL uses inferred server types/casts.
- Schema 5 adds saved-query definitions with existing optimistic revision checks; old rows default to empty definitions. Draft definitions use optional/defaulted JSON. No values enter these storage models. Parameterized history errors use a static summary, retaining actionable backend errors in the live tab.
- `cargo check --locked --tests --features test-support` — passed after correcting a module visibility error.
- `cargo test --locked --test storage` — 14 passed, including actual schema-4 migration, revision conflicts and definition reopening.
- Native test iterations caught an ambiguous type-button/menu label and two test API/type errors; fixed the button label and test calls.
- `cargo test --locked --features test-support --lib parameter_form -- --nocapture` — 1 passed: Ctrl+Enter, Cancel, invalid Integer correction, explicit NULL, empty text, repeated values, execution confirmation, stale editor rejection, draft/history value absence.
- `QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --features test-support --lib postgres_safeguards_parameter -- --include-ignored` — 1 passed, including a real PostgreSQL integer conversion error echoing its input in the live tab but absent from persisted history.
