# Next features: integrated validation

Validated in `/home/timen/projects/queri` on 2026-11-11 at implementation commit `d0de943`.

All planned milestones are integrated. Each implementation received focused validation and an independent read-only source review; accepted search and timeout findings were corrected and rechecked before proceeding. The final parameter/generated-SQL review found no actionable issues.

| Feature | Integrated commits | Feature evidence |
| --- | --- | --- |
| PostgreSQL execution foundations | `42a1cbe` | [Foundations](next-features-foundations-validation.md) |
| Loaded-result and live-draft search | `517668b`, `ffe2694` | [Search](next-features-search-validation.md) |
| Transaction controls and lifecycle guards | `1d8f44a` | [Transactions](next-features-transactions-validation.md) |
| Per-tab statement timeouts | `073255c`, `8199647` | [Timeouts](next-features-timeouts-validation.md) |
| Environment labels and execution safeguards | `8beb21b` | [Safeguards](next-features-safeguards-validation.md) |
| Grouped filters, sorting, column layouts, named views | `1391c8b`, `1c6a22b`, `28cacc7` | [Table views](next-features-table-views-validation.md) |
| Session-aware SQL autocomplete | `dded863` | [Autocomplete](next-features-autocomplete-validation.md) |
| Query parameters and current-page SQL drafts | `1168cb1`, `009b072`, `d0de943` | [Parameters and generated SQL](next-features-parameters-validation.md) |

## Final parent verification

The parent independently ran the full integrated suite, using the designated disposable PostgreSQL cluster on loopback port 55483:

```sh
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55483/postgres' bash scripts/check.sh
```

Exit **0**; local output `/tmp/queri-parent-final-check.log`.

- Formatting and strict all-target Clippy passed.
- Ordinary library/native tests: 72 passed; PostgreSQL cases separately enabled below.
- SQLite/ordinary database tests: 22 passed.
- Storage tests: 14 passed.
- Backdrop scene tests: 3 passed; real-GPU dialog rendering passed.
- PostgreSQL library/native tests: 5 passed.
- PostgreSQL database tests: 19 passed, including parameter execution, generated SQL equivalence, completion, safeguards, timeout, cancellation, and transaction regressions.
- Desktop launcher validation passed.

This is native GPUI, actual GPU rendering, SQLite, and PostgreSQL integration verification; no manual desktop walkthrough is claimed. Parameter history privacy includes errors that echo entered values. Generated-query tests compare returned rows with actual table browsing and exercise the native draft/form handoff.

## Scope limits

- Search operates on loaded result cells and live drafts.
- Parameterized execution accepts one statement and at most 1,000 distinct bindings. Values stay in memory; PostgreSQL types use server inference and explicit casts when needed.
- Generated SQL describes the last successfully loaded page's settings, not a data snapshot. A new draft has an independent session; temporary objects and uncommitted changes may therefore be unavailable there.
- Autocomplete uses cached metadata during active/failed transactions and supports common SQL scopes rather than complete semantic inference.
- Read-only-by-default PostgreSQL policy is SQL-overridable accident prevention. OFFSET pagination and keyless ordering limitations remain.
- Feature records detail further boundaries, including forced shutdown and localized PostgreSQL timeout classification.

The designated PostgreSQL cluster was stopped after verification. Completed implementation worktrees and branches were removed after confirming integrated content matched. No PR was opened. The unrelated pre-existing prunable `/tmp/opencode/queri-dialog-blur-validation` worktree record was preserved.
