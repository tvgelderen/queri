# Environment safeguards validation

Worktree: `/home/timen/.t3/worktrees/queri/t3-c0d3261f-dcd7080860a27399`.
Base: `8199647` (integrated transaction, loss-guard, timeout, and search work).
No agents, PRs, dependencies, or later roadmap features were added.

## Implementation contract

- Profiles persist optional Development/Staging/Production and an independent
  Normal/ConfirmExecutions/ReadOnlyByDefault policy. Missing fields deserialize
  as None/Normal. Environment changes never select an execution policy.
- Selected connection and document context show the environment as text.
  The picker uses the existing connection color, without environment-specific
  colors. SQL, row, admin, connection removal, and transaction-loss context
  retain the affected connection/environment/database identity.
- `Document::submit_sql` is the common confirmation boundary for raw SQL and
  optional text binds. No parameter-entry UI was added. Begin/Commit use it;
  toolbar Rollback and acknowledged rollback during session release bypass the
  extra execution confirmation for practical recovery. Typed rollback SQL still
  follows the policy. A captured session generation prevents a pending SQL
  confirmation from running against a reconfigured session.
- Destructive SQL uses one combined prompt. Inserts/updates confirm the captured
  row change; delete uses its existing specific confirmation. Admin forms are
  the specific confirmation: operation, connection/environment, source/target
  fields, maintenance database, rollback/termination consequences, and Cancel.
  Their submit button is the acknowledgement; there is no redundant extra modal.
- PostgreSQL startup options set `default_transaction_read_only=on` for every
  protected session. Normal never sets it off, preserving server/database/role
  defaults. Generated mutations/admin are also rejected by Session, and blocked
  by UI controls and action handlers. SQLite retains OS read-only opening.
- PostgreSQL `SHOW` observes default/current transaction read-only settings after
  successful and failed submissions when the transaction is usable; it avoids a
  data SELECT/snapshot just to inspect policy. Observed read-write overrides are
  displayed; generated mutations stay blocked regardless of overrides.
- Connection edits reuse the transaction-loss guard. Affected tabs are listed;
  saving updates every document profile, not just the sidebar, and reconnects
  using the updated policy. Uncommitted work requires explicit acknowledgement.

## Commands and results

Cluster start (succeeded):

```sh
pg_ctl -D /tmp/queri-foundations-pg -l /tmp/queri-safeguards-pg.log \
  -o '-h 127.0.0.1 -p 55483 -k /tmp/queri-foundations-pg' start
```

Focused feature validation, run before the full check:

```sh
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' \
  cargo test --locked --features test-support safeguards -- --include-ignored
```

Passed: 8 native GPUI tests, 1 PostgreSQL integration test, and 1 storage test.
The harness-free dialog executable also ran and passed Light/Dark rendering
checks. Log: `/tmp/queri-safeguards-focused-final.log`.

Coverage includes native form choices without policy coupling, text environment
in document/confirmation/picker context, SQL cancel/run and selected destructive
SQL without double prompts, Begin/Commit confirmation and practical Rollback,
parameter-submission cancel/run, row editor cancel/run and deletion, read-only
row action bypass attempts, direct admin bypass attempts, complete confirmed
PostgreSQL admin lifecycle and disabled admin controls, and policy editing through
an active transaction with fresh document/session profiles. Storage checks load
an actual old JSON profile from SQLite and round-trip every new field variant.

Real PostgreSQL checks reject ordinary persistent writes and bound writes after
reconnect; verify environment alone permits writes; exercise BEGIN READ WRITE
and a changed session default; reject generated writes/admin even after an SQL
override; test the existing-temporary-table exception; and verify Normal respects
a database-level read-only default. Existing SQLite read-only tests run below.

Required complete check:

```sh
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' \
  bash scripts/check.sh
```

Passed with exit 0: formatting, strict all-target Clippy, 48 library/native tests
(4 PostgreSQL tests deferred to the enabled pass), 3 backdrop-scene tests,
17 ordinary database/SQLite tests, 11 storage tests, real-GPU Light/Dark dialog
rendering, then 5 PostgreSQL-filtered library/native tests and 13 PostgreSQL
integration tests, and desktop launcher validation.
Log: `/tmp/queri-safeguards-full.log`.

During implementation, `cargo check --locked --all-targets --features test-support`
first caught a missing new field in a legacy fixture. Early focused iterations
caught a missing test DSN, a private GPUI selection method, a wildcard test-macro
import collision, missing test observation registration, and a menu-dismissal
wait in a test. These were corrected and rerun. An affected-tabs scroll element
initially needed an element ID to use GPUI scrolling. The first full native suite
also caught a real 900×600 form regression (SQLite file path outside the visible
area); controls were moved below connection details, grouped compactly, and long
policy help made conditional. The original small-window test then passed without
weakening it. Final table wording explicitly describes generated-change blocking.

## Limits

This is accident prevention, not immutable enforcement or an authorization layer.
Raw PostgreSQL SQL can override defaults; temporary objects have PostgreSQL's
own exceptions. Use database role permissions for hard enforcement. Observation
is best effort after a submission, not a parser/security filter or a complete
history of transient settings inside scripts. Failed/disconnected sessions retain
the last observation. Confirm policy is a UI submission policy: low-level Session
query APIs intentionally do not display UI. Future parameter UI must call the
shared Document submission boundary tested here. Explorer reads, refreshes,
connection tests, and session inspection do not prompt.

Validation uses native GPUI test windows and the existing real-GPU dialog harness;
no manual desktop/browser walkthrough or new visual snapshot was claimed.
Only the designated disposable PostgreSQL cluster was started/stopped.

Final focused-plus-full rerun after the wording correction also exited 0.
`git diff --check` passed. Cleanup:

```sh
pg_ctl -D /tmp/queri-foundations-pg stop -m fast
pg_ctl -D /tmp/queri-foundations-pg status
```

Stop succeeded (`server stopped`); status returned exit 3, `no server running`.
No other PostgreSQL server was touched.
