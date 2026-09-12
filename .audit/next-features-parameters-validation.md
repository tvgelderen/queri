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
- Slice 2 commit: `4485e68`.

## Slice 3 — shared browse compiler and editable handoff

- Each backend's existing browse builder now returns a single internal plan containing metadata, count SQL and page SQL/binds. Actual browsing and `Session::browse_sql` consume this same plan. PostgreSQL keeps its FK operator/collation and json_populate_record semantics; SQLite keeps native affinity/collation and blob values.
- Document records options only after successful page loads. Open SQL uses that snapshot, recompiles against current metadata, and emits a draft-opening event. Workspace retains connection/database; no SQL execution or history entry occurs until Run. Column layout never changes query projection. Parameter values are seeded only in memory and consumed by the first form.
- Added selection snapshots alongside SQL/session checks so changing the selection invalidates pending confirmation too. Explicit PostgreSQL array-slice handling distinguishes native `a[lower:upper]` from `ARRAY[:value]` and parenthesized subscript parameters.

Focused commands/outcomes:
- `QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --test database grouped_views -- --include-ignored` — 2 passed; each successful browse is compared against generated-bound execution, including group/scope conjunction, sorting and page offsets.
- `QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --test database relationship_navigation -- --include-ignored` — 2 passed; generated execution also compared for composite keys, SQLite affinity/NOCASE and PostgreSQL UUID/bigint keys, both directions.
- `cargo test --locked --lib formatting_preserves -- --nocapture` — passed, including named PG casts and native SQLite placeholders with token equality.
- `cargo test --locked --features test-support --lib generated_sql_ -- --nocapture` — passed; unapplied filter edits and failed sorts retain successful options, no history on Open SQL, renamed filter column fails repairably.
- `cargo test --locked --features test-support --lib saved_library_sql_files -- --nocapture` — passed at an 800px native window; actual relationship-tab Open SQL → new draft → Ctrl+Enter → prefilled parameter form → correct Alice row, same connection/database, no execution on opening.
- `QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --test database generated_quoted -- --include-ignored` — 2 passed after using the actual PostgreSQL temporary schema catalog name (metadata APIs do not resolve the `pg_temp` alias). Quoted identifiers, NULL, empty text, Unicode/quote/injection-shaped filter values, stable sorting/page offset, and SQLite blob relationship bindings compare exactly.
- `cargo test --locked --test database sqlite_parameter_limits_and_cancel` — passed; 11,000 bound result rows are fully drained with 10,000 displayed, an unbounded bound computation cancels, and a subsequent bound query succeeds on the same session.
- `cargo test --locked --features test-support --lib parameter_definitions_reopen -- --nocapture` — passed; reopened Integer type, actual run/save, updated definitions, no value in SQL/definitions, removed-name synchronization, and stale-session form rejection. Also confirms the existing unparameterized partial-script behavior with an unfinished literal, including its destructive-SQL reminder. The initial test forgot to accept that existing reminder; corrected the interaction. An earlier privacy assertion looked for `42` anywhere in JSON and could match a random UUID; replaced it with exact SQL/definition assertions (the separate long-secret/error tests verify absence of entered values).
- Strict Clippy initially caught two tightly spaced assignments in the new form renderer; expanded the renderer and corrected spacing. No lint suppressions.
- Final refinements preserve unselected parameter type choices when running a selection, initialize generated definitions from native binding types before saving, and cover nested PostgreSQL ARRAY constructors versus quoted identifier subscripts.

## Full verification and limits

`QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' bash scripts/check.sh` passed on the first complete run (71 ordinary library/native tests, 22 ordinary database tests, 14 storage tests, 3 backdrop scene tests, light/dark GPU harness, 5 PostgreSQL library/native tests, 19 PostgreSQL database tests, strict Clippy/formatting and desktop launcher validation). A subsequent full run with the reopening test also passed. The final run below includes the raw-script compatibility refinement.

Explicit behavior:
- One parameterized statement, at most 1,000 distinct binds, contiguous positional indexes. Mixed named/native styles are rejected. SQL tokenizer failures in parameterized input are rejected; an unparameterized unfinished literal/comment stays on the original raw-script path.
- PostgreSQL uses server text, with casts for ambiguous/custom types; there is no client-side emulation of PostgreSQL type parsers. Array slice colons retain native meaning. Use `array[(:index)]` for named subscript parameters.
- Generated SQL represents the last successfully loaded page's settings, recompiled against current schema. It is not a data snapshot; concurrent data changes or the new draft's independent session can change returned rows. Keyless ties remain unordered as before.
- New SQL drafts keep generated values only until the first parameter form, SQL editing, or reconnect. Saved definitions retain native type choices, but values never survive reopening/restart. Parameterized result cells and detailed live errors can of course display the values; neither is serialized in draft/history models.
- Verification is native GPUI and real database/GPU integration; no browser or manual desktop walkthrough is claimed.

Final command:

```sh
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' bash scripts/check.sh
```

**Exit 0**, final log `/tmp/queri-params-final-verified.log`:
- Formatting and strict all-target Clippy passed.
- Ordinary library/native tests: 72 passed, 4 PostgreSQL tests deferred to the enabled phase.
- Ordinary database tests: 22 passed, 19 PostgreSQL tests deferred to the enabled phase.
- Storage: 14 passed. Backdrop scene: 3 passed. Light/Dark real-GPU dialog executable passed.
- PostgreSQL library/native phase: 5 passed; PostgreSQL database phase: 19 passed. No ignored cases in either enabled phase.
- Desktop launcher validation passed.

Cleanup:

```sh
pg_ctl -D /tmp/queri-foundations-pg -m fast stop
pg_ctl -D /tmp/queri-foundations-pg status
git diff --check
```

Stop succeeded (`server stopped`); status returned exit 3 (`no server running`). Diff check passed. No other cluster was touched. No delegated agents, extra worktrees, dependencies or PRs were created.
