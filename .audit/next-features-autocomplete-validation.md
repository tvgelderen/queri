# SQL autocomplete validation

Implementation worktree: `/home/timen/.t3/worktrees/queri/t3-af2ccb7a-486158c656b23c56`.
Base: `28cacc7` (parent transaction/timeout/safeguard/search/table-view milestone).

## Implementation

- Reuses the installed GPUI `CompletionProvider`, `ShowCompletions`, completion popup, and undo/edit machinery. Adds the already-transitive `lsp-types` 0.97 dependency explicitly; no LSP server or overlay replacement.
- Feature-owned analysis in `src/completion/analysis.rs`; catalog access/cache in `src/database/completion.rs`; editor adapter in `src/ui/completion.rs`. Document changes install/reset the provider, bind Ctrl+Space, expose refresh/status, and dismiss stale menus on session changes and execution.
- Tolerant lexical analysis is separate from execution parsing: the existing sqlparser tokenizer rejects unfinished quotes, while completion must retain ranges and suppress suggestions inside unfinished strings/comments/dollar bodies. No successful whole-script parse is required.
- Catalog names precede on-demand columns. Queries do not load indexes or constraints. PostgreSQL exposes only accessible objects/columns, including column-level SELECT grants, types/comments, search-path visibility, and this session's temporary schema (as `pg_temp`). SQLite includes temp/main/attached schemas and generated columns.
- Each Session owns its cache. Every SQL submission invalidates before and after execution, including errors and bound submissions. This deliberately conservative policy covers DDL, SET/search_path, role changes, ATTACH/DETACH, and functions with schema side effects. Explicit refresh and reconnect also invalidate.
- The execution lock is acquired with try_lock; the idle check and catalog reads occur while retaining that same lock. Active/failed/busy sessions never receive speculative catalog SQL. Errors preserve cached data and suppress retries until invalidation. Editing/execution remain independent of metadata failures.
- The provider debounces 60 ms before starting background analysis/I/O. Superseded editor tasks are cancelled, with at most one active catalog job per provider/session generation and one cancellable latest request. Responses check session generation and catalog epoch; installed GPUI also checks document revision, caret, focus, and editing state.
- Identifiers are always safely double quoted; a completion replaces the complete token, including existing quotes and text after the caret. GPUI 0.8.1's completion Position columns are Unicode scalars, not UTF-16; the adapter follows its actual RopeExt contract. Native acceptance tests include supplementary Unicode before a quoted token.

## Commands and outcomes

Commands were run from the worktree above. PostgreSQL was started only at the user-designated stopped cluster:

```sh
pg_ctl -D /tmp/queri-foundations-pg -l /tmp/queri-completion-pg.log -o '-h 127.0.0.1 -p 55483 -k /tmp/queri-foundations-pg' start
```

Outcome: `server started`. No other PostgreSQL server was started/stopped.

Focused catalog slice:

```sh
cargo check --quiet
cargo test --locked --test database sqlite_completion -- --nocapture
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --test database postgres_completion -- --include-ignored --nocapture
```

- Check passed after correcting an initial duplicate dependency-key editing error.
- SQLite: 1 passed. Covers names without eagerly loaded columns, quoted/reserved/Unicode columns and types, temp shadowing, attached schemas, cache isolation, autocommit DDL refresh, and unchanged open transactions.
- Final focused PostgreSQL slice: 3 passed. Covers quoted/Unicode names, temp tables, column types/comments, explicit search_path precedence, views, column-level permissions, open/failed transaction preservation, rollback, isolation between sessions, connection termination, and immediate cached completion while execution holds the lock. Connection-loss test intentionally logs a closed-connection error. Temporary roles and schemas are cleaned up.
- One intermediate test compile failed because a new assertion called `Cell::display`; corrected to the existing `Cell::text` API before the passing run.

Analysis/provider slices:

```sh
cargo test --locked --lib completion::analysis -- --nocapture
cargo test --locked --features test-support --lib sql_completion -- --nocapture
cargo test --locked --features test-support --lib completion -- --nocapture
cargo clippy --locked --all-targets --features test-support -- -D warnings
```

- Initial pure analysis slice: 6 passed. Expanded final completion filter: 13 passed (includes one existing transaction-completion regression test).
- Pure cases include incomplete SQL, joins/comma sources, quoted and Unicode token ranges, comments/nested comments/strings/dollar bodies, aliases, nested correlation, shadowing, statement isolation, CTE explicit/projection/star outputs, derived tables, recursive termination, CTE/non-lateral isolation versus LATERAL correlation, dialect case handling, and input/result bounds.
- Native GPUI tests exercise actual Ctrl+Space, typing `.` and prefixes, Enter acceptance, one-step undo, focus retention, F5 refresh, whole quoted-token replacement with supplementary Unicode earlier in the line, literal/comment suppression, stale text/focus/session requests, and Ctrl+Enter/Ctrl+F while the popup is open.
- The first manual native test failed because `EditorState::set_value` resets multiline editor selection to offset zero. Its setup now positions the caret at the intended prefix. The corrected test passed. A supplementary-Unicode test also verifies the adapter's scalar-column conversion.
- Initial strict Clippy reported an owned-string comparison in a test; corrected. Subsequent strict Clippy passed.

Full verification:

```sh
cargo fmt --all
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' bash scripts/check.sh
```

The first full run passed. A final full run follows the expanded scope, permissions/busy-session, and shortcut coverage; its result is recorded below. The script includes formatting, strict all-target Clippy, native tests, GPU backdrop rendering, SQLite/storage/integration tests, PostgreSQL-native and database tests, and desktop-file validation. Local output logs: `/tmp/queri-completion-focused.log`, `/tmp/queri-completion-catalog.log`, `/tmp/queri-completion-check-final.log`.

## Limits

- Comprehensive discovery of uncommitted DDL is intentionally excluded. During transactions, existing cached columns remain available; finish/recover the transaction and refresh to discover new metadata.
- Invalidation after any SQL submission is conservative and can reload names after read-only SELECTs. External schema changes require explicit refresh.
- Bounds: 256 KiB SQL, 30,000 lexical tokens, 100 displayed suggestions, 20,000 relation/schema names, 16 referenced column sources per request, 2,000 columns per relation, 1,024 characters per PostgreSQL column comment, and eight levels of derived-column inference.
- This is completion inference, not full semantic SQL validation. Complex expression output names, recursive inference, uncommon dialect constructs, and UNION operand isolation are not comprehensive. Ordinary aliases, common CTEs/derived tables, correlated nested queries, and LATERAL boundaries are covered.
- Native tests use GPUI's native test harness and existing rendering checks; no manual desktop session or replacement browser overlay was used.
- No parameter UI, PR, unrelated cleanup, or agents were created.

## Final outcome

The final `QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' bash scripts/check.sh` exited **0**. Before this passing run, strict Clippy caught a collapsible nested `if` in the last dependency-selection change; it was collapsed and the complete script rerun.

- Formatting and all-target strict Clippy: passed.
- Normal library/native suite: 68 passed, 4 PostgreSQL cases ignored in this phase.
- Backdrop scene tests: 3 passed; actual GPU dialog-backdrop binary passed.
- Normal database suite: 19 passed, 17 PostgreSQL cases ignored in this phase.
- Storage suite: 13 passed.
- PostgreSQL library/native phase: 5 passed, no ignored tests.
- PostgreSQL database phase: 17 passed, no ignored tests (includes all 3 completion-specific PostgreSQL tests).
- Desktop-file validation: passed.
- `git diff --check`: passed.

Cluster shutdown command:

```sh
pg_ctl -D /tmp/queri-foundations-pg stop -m fast
```

Outcome: `server stopped` (exit 0). Only the designated cluster was touched.
