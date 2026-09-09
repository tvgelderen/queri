# Statement timeout validation

Worktree: `/home/timen/.t3/worktrees/queri/t3-7d4718a1-0d26ca1500d51b2e`

Scope: per-SQL-tab Inherit / positive custom milliseconds / No application timeout; backward-compatible Draft JSON; reconnect; PostgreSQL simple and text-bind execution; SQLite statement deadlines and lock waits; outcomes, native controls, history, documentation. No agents, PRs, or other roadmap features.

## Implementation decisions

- Inherit is the default for missing Draft fields. Custom durations are positive milliseconds, bounded to PostgreSQL's signed 32-bit millisecond range. Settings are persisted through the existing preferences JSON, without a schema migration.
- PostgreSQL captures `SHOW statement_timeout` at connection creation. Before SQL submissions it applies the selected policy with session-local `set_config(..., false)`, under the existing session mutex. This is a session setting, not a global configuration change. It reapplies after rollback/manual settings changes. Failed transactions and standalone Rollback/Abort bypass configuration so recovery can run. Commit uses the selected timeout. Raw scripts are not parsed, split, rewritten, or retried.
- SQLite uses the rusqlite hooks feature. A progress callback observes a deadline reset before each Batch statement, including preparation. A worker-local busy callback waits in bounded increments until that deadline. An RAII guard removes the progress callback and restores the prior busy timeout on success/error. No detached timeout task or future-dropping cancellation is used. SQLite transaction state continues to come from `is_autocommit`.
- Timed-out errors have a distinct marker and UI outcome; history retains the timeout description and original backend error. Existing result collection, exact values, display caps, full execution, cancellation, and transaction guards remain in place.
- The Timeout menu uses existing choice and confirmation-dialog controls. It is disabled during work and in a failed transaction, with recovery guidance. Reconnect retains the preference and installs it on the replacement session; each SQL execution uses the current draft policy.

## Commands and results

All commands ran in the worktree above. PostgreSQL was used only at the designated disposable cluster and port.

```sh
pg_ctl -D /tmp/queri-foundations-pg -l /tmp/queri-timeouts-pg.log -o '-h 127.0.0.1 -p 55483 -k /tmp/queri-foundations-pg' start
```

Result: server started successfully.

```sh
cargo check --locked --tests --features test-support
cargo test --locked --test storage statement_timeout
cargo test --locked --features test-support --lib statement_timeout
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' cargo test --locked --test database statement_timeout -- --include-ignored
```

- Initial compilation exposed hook mutability/Result handling and dialog API mismatches; corrected before the passing checks.
- Storage: 1 passed. Missing preference defaults to Inherit; all three modes round-trip through Store; zero and out-of-range custom input are rejected.
- Focused native UI: 2 passed. Real menu selection and custom dialog Apply; draft serialization; blocked changes during work; distinct timed-out outcome and history; reconnect and next short query; disabled settings with Rollback available in Failed state.
- Initial native interaction testing found that the plain dialog had no confirmation footer. Switched to the existing alert/confirmation dialog. The test waits for the native menu's deferred dismissal before reopening it.
- Focused database: 4 passed, including two real PostgreSQL tests and two SQLite tests. The initial nonzero inherited-policy fixture creates a uniquely named disposable login role with an 800 ms role default and drops it afterward; this test requires role creation privileges on the disposable cluster.
- PostgreSQL covers CPU work, sleep, text parameters, advisory-lock waits, Failed status, savepoint recovery, rollback, transaction-rolled-back settings, manual SET overridden on the next submission, NoTimeout, initial nonzero role policy restoration, reconnect, independent sessions, and two 600 ms statements under a 1000 ms per-statement limit.
- SQLite covers an unbounded recursive computation, an interrupted INSERT that automatically rolls back its transaction, a read interruption retaining an open transaction, rollback, a synchronized held write lock, a short query after each failure, reconnect, and 200,000 short statements whose total runtime exceeds a 200 ms limit while preserving result caps.
- Workspace restoration coverage additionally verifies that two restored drafts retain distinct custom timeout values.

```sh
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' bash scripts/check.sh
```

The first full-suite attempt exposed a test-lifecycle mistake: the synthetic Failed-state control test started an elapsed-transaction timer and exited without settling it. The test now sets its synthetic state directly; production transaction timer behavior was not changed. Subsequent complete runs passed. The final run includes formatting, strict Clippy, native GPUI tests, real-GPU backdrop rendering, SQLite/storage tests, PostgreSQL-enabled native and database tests, and desktop launcher validation.

Final result: exit 0. Main native/library tests: 43 passed, 3 PostgreSQL tests deferred to the enabled pass. Backdrop-scene tests: 3 passed; standalone GPU dialog test succeeded. Database default pass: 17 passed, 12 PostgreSQL tests deferred. Storage: 10 passed. PostgreSQL-enabled native/library pass: 4 passed; PostgreSQL database pass: 12 passed.

```sh
git diff --check
pg_ctl -D /tmp/queri-foundations-pg stop -m fast
```

Final diff check clean; designated cluster stopped after all test work settled.

## Limits and explicit semantics

Parent integration validation passed all four database timeout tests, both native
timeout tests, and the storage round-trip test. Independent review found silent
invalid-duration rejection. The parent added a live inline validation message and
native coverage for empty, zero, negative, and excessive values, preserving the
previous preference and correcting to a valid value. Initial UI assertions could
not observe the plain div; adding the existing GPUI test-support observation hook
made the rendered alert testable. Both native timeout tests then passed.

- These settings cover SQL execution, including binds and transaction commands; explorer metadata, table mutation APIs, and administration have no new timeout controls.
- PostgreSQL manual `SET statement_timeout` within a raw script controls later statements in that script; the selected preference is reapplied on the next submission. Recovery in Failed state cannot accept configuration SQL, so it uses the current backend policy until recovery completes. Standalone Rollback likewise uses the active policy.
- SQLite manual `PRAGMA busy_timeout` within a script can replace its busy handler; the outer scope restores the pre-execution busy policy. This explicit manual override is documented. Computation deadlines remain active. Inherit/NoTimeout preserve SQLite's existing lock policy (normally five seconds), which is not an execution deadline.
- NoTimeout sets PostgreSQL statement_timeout to zero or removes SQLite's application execution deadline. Other server/database policies still apply.
- PostgreSQL distinguishes timeout from user cancellation using SQLSTATE 57014 plus the server's `statement timeout` message. The supplied cluster uses English messages; localized server messages were not tested and may appear as ordinary errors rather than the timeout-specific outcome.
- Timing checks use long/infinite work, a pre-held database lock, generous short-query margins, or many short statements. They do not assume precise wall-clock timeout delivery. SQLite cannot preempt arbitrary non-cooperative native extension functions between progress callbacks.
