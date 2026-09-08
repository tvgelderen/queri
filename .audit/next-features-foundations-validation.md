# Execution foundations validation

Only milestone one is implemented. No UI features, parameter editor, other
feature work, agents, or PRs were added. Worktree:
`/home/timen/.t3/worktrees/queri/t3-6e7454ae-2fbdcb09b5ce3545`.

## Integration contract

- `Session::transaction_status().await -> Result<TransactionStatus>` acquires
  the same execution lock as other session work. PostgreSQL sends protocol Sync
  and maps ReadyForQuery I/T/E to Idle/Open/Failed. Failed communication or an
  unrecognized status returns Unknown. SQLite reads `is_autocommit()` under
  its connection lock (Idle/Open); worker/lock failures remain errors.
- This is an authoritative observation at the time of the call, not a cached
  property or a subscription. It waits behind running work and costs one
  PostgreSQL round trip. Call it after both successful and failed operations;
  use the existing independent cancel method to interrupt running work.
  A later concurrent operation can change state after observation.
- `Session::query_postgres_params(sql: String, params: Vec<Option<String>>)`
  returns the existing `QueryOutput`. Parameters are positional `$1`, etc.;
  None is SQL NULL, Some("") is empty text. PostgreSQL infers types from context,
  with explicit SQL casts when needed. Both parameter and result formats are
  PostgreSQL text, including custom types. SQL is never interpolated.
- Bound execution accepts one statement; the server rejects multiple commands.
  SQLite returns an unsupported-backend error. The existing `query()` remains
  the raw-script API. Both paths use the existing 10,000-row / 32-MiB collector
  and drain successful execution even after truncation. Database errors in
  text streams are delivered after ReadyForQuery. Earlier script effects and
  the existing error/result API remain unchanged.
- Reconnect means creating a new Session. No implicit reconnect or transaction
  recovery is introduced. Unknown intentionally includes disconnected state.
  COPY streaming is not supported by these application query APIs. Text output
  follows server settings (timezone, bytea_output, etc.), as raw query does.

## Evidence

Baseline: `cargo test --locked --test database`: **13 passed, 5 ignored**.

Final focused run:

```sh
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' \
  cargo test --locked --test database -- --include-ignored
```

**22 passed, 0 failed, 0 ignored** on a newly initialized PostgreSQL **18.6**
cluster, bound only to loopback with a private temporary socket directory.

Coverage includes BEGIN/START TRANSACTION, COMMIT, ROLLBACK, savepoint recovery,
failed transactions, independent sessions, cancellation outside/inside a
transaction, dropped query futures, queued status observations, backend
termination and fresh-session state. Bind checks compare server text against
simple-query output for NULL, empty text, high-precision numeric, arrays,
timestamps, bytea, custom enum and Unicode/injection-shaped text. Tests also
cover no-row column metadata, inferred text types, repeated parameters, command
counts, local encoding failure, SQL/type/count errors, multi-statement rejection,
both display limits and persisted effects beyond the row limit.

Required check:

```sh
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' \
  bash scripts/check.sh
```

**Exit 0**: formatting, strict all-target Clippy, native tests, real-GPU dialog
test executable, PostgreSQL integration/UI tests and desktop launcher validation.
Final test additions were also rechecked with:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --features test-support -- -D warnings
```

Both passed. Actual test-summary outputs and the initial socket setup failure
and resolution are recorded in `next-features-foundations.tsv`.
The vendor differs from published 0.7.18 in only three Rust source files;
`vendor/README.md` documents provenance, changes and removal criteria.

The staged whitespace check reported two unchanged upstream vendor artifacts:
LICENSE-MIT's blank final line and generated sqlstate.rs trailing whitespace.
Both are retained verbatim from the published crate; application edits are clean.
The disposable PostgreSQL server was stopped after validation.
