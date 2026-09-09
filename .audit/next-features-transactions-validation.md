# Transaction controls validation

Date: 2026-11-11. Scope: transaction UI, session lifecycle, tests, and documentation only. Foundations/search were preserved. No timeout, safeguard, parameter, table-feature, or completion work; no agents or PRs.

## Implementation

- SQL tabs expose Begin, Commit, and Rollback through the existing query/history path. Operation state is independent of Idle/Open/Failed/Unknown transaction state. Autocommit remains the default.
- Query completion awaits `Session::transaction_status()` before enabling controls, on success and failure. SQLite autocommit and PostgreSQL Sync/ReadyForQuery remain authoritative. No SQL-text inference. Connections and table refreshes also observe status asynchronously.
- PostgreSQL Failed disables toolbar Commit and offers Rollback. Manual savepoint recovery remains available. Unknown never reports a successful query or retries Commit.
- Elapsed time starts at the first observed Open/Failed state, clears on Idle/session replacement, and updates while open. Session generations reject stale connect/query/read/cancel completions. Cancellation delivery itself keeps controls locked until settled.
- One shared loss guard covers close, reconnect, connection editing/removal, Ctrl+Q, and window close. It locks affected documents through asynchronous observation, acknowledgement, and rollback. Keep open retains the sessions. Rollback must be confirmed before continuing; a newly lost connection keeps the session open and requires acknowledgement of Unknown. Already-unknown outcomes have an explicitly named close option, with no claim that a completed Commit can be undone.
- Idle documents retain immediate close/form behavior: their last completed operation already included authoritative observation, and no busy/cancelling work may pass this path. Active work requests cancellation and requires retrying the intended close after settlement. Duplicate transaction guard requests are ignored while a guard owns the documents.
- Profile edits disconnect acknowledged sessions before opening the editing form. Connection removal retains its separate existing deletion confirmation. Existing admin rollback acknowledgement is reused; busy work and known unknown outcomes block admin disconnect until resolved/acknowledged in the tab.
- Normal application quit uses the guard before `cx.quit`; `on_app_quit` is final best-effort cleanup and persistence, not an interactive veto. Explicit quit mode prevents accidental automatic shutdown. Transaction state/timestamps are not persisted.

## Commands and results

Executed in this worktree. Final verification commands exited 0 unless stated otherwise.

```sh
cargo check --locked --features test-support
cargo fmt --all
pg_ctl -D /tmp/queri-foundations-pg -l /tmp/queri-transactions-pg.log -o '-h 127.0.0.1 -p 55483 -k /tmp/queri-foundations-pg' start
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --features test-support --lib transaction_ -- --include-ignored
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --features test-support --test database transaction_ -- --include-ignored
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --features test-support --lib -- --include-ignored
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' bash scripts/check.sh
pg_ctl -D /tmp/queri-foundations-pg stop -m fast
git diff --check
```

Final focused native run: **5 passed**. Coverage: toolbar/manual SQL and SQLite error observation; cancellation settlement; PostgreSQL Failed/manual savepoint recovery/Unknown Commit UI; close/reconnect/profile/quit keep-open and rollback decisions; stale old-session completion rejection.

Focused database run: **5 passed**. Coverage: authoritative backend status, committed/rolled-back effects, independent session visibility, SQLite automatic rollback, and PostgreSQL disconnect during Commit. The dedicated Commit-loss test installs a deferred trigger, waits until the committing backend is actually sleeping in that trigger, terminates only that backend, and asserts error plus Unknown rather than success. The observer confirms the row was not committed in this particular fault scenario. Test objects are dropped.

The all-native run passed **43 tests** before the final stale-completion test was added. The subsequent final full check includes that test: **41 normal library tests passed**, 3 PostgreSQL-dependent tests deferred to the enabled PostgreSQL pass; **3 backdrop-scene tests**, **15 normal database tests**, and **9 storage tests** passed. Real-GPU light/dark dialog verification passed. PostgreSQL pass: **4 filtered library tests** and **10 integration tests** passed, including existing cancellation/abandoned-work and admin lifecycle coverage. Formatting, strict Clippy, doc tests, and desktop-file validation passed. Full script exit: **0**.

Local command output: `/tmp/queri-transactions-focused.log`, `/tmp/queri-transactions-database.log`, `/tmp/queri-transactions-native.log`, `/tmp/queri-transactions-full-check.log`. The cluster start and stop both succeeded; no unrelated server was touched.

## Failures found and corrected during implementation

- Initial compilation caught moved/borrowed callback captures and an invalid test-query helper; corrected before successful checks.
- Early focused UI attempts exposed test clicks against a dialog still exiting. Tests now wait for dismissal before the next request; an obsolete run was terminated while revising this test. A real runtime timer supplies elapsed-time updates without repeatedly advancing GPUI's synthetic test timer.
- The first PostgreSQL isolation fixture put CREATE TABLE and BEGIN in one protocol message, making the new table invisible to the second connection. Setup now creates the table in a separate completed call before beginning the transaction. The corrected integration run passed.
- The first full script passed Clippy but failed three existing native tests because universally asynchronous guards delayed ordinary idle closes and new-connection forms. The guard now preserves immediate behavior for already-observed Idle sessions with no pending work. The unchanged existing tests and final full script passed.

## Limits

- Native tests exercise GPUI actions, controls, document lifecycle, and the workspace quit request. They do not drive a real desktop window manager's close button or terminate the production application. Forced process termination/OS shutdown cannot offer an interactive confirmation; final cleanup is best effort under GPUI's shutdown deadline.
- The Commit fault test covers backend termination while Commit is executing. It does not inject a network loss after durable commit but before receipt of ReadyForQuery. Both use the same Unknown/no-retry UI contract; the latter outcome cannot be inferred from this test.
- Elapsed time measures time since first observation, not the exact server BEGIN timestamp inside a long-running multi-statement script. Transaction state is refreshed at operation boundaries, not polled while a query is executing.
- Busy close/quit requests cancel and require a second request after settlement; they do not automatically close when cancellation completes. Rollback failure keeps documents open. Connection removal still separately confirms deletion of its saved metadata/drafts/history.
