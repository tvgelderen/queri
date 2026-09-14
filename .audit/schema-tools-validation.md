# Schema tools verification

The integrated implementation provides ER diagrams, table design, schema comparison, and reviewed target apply for supported PostgreSQL and SQLite definitions.

## Verified

- `scripts/check.sh` passed with both `QUERI_SCHEMA_TEST_POSTGRES` and `QUERI_TEST_POSTGRES` pointing to a disposable local PostgreSQL instance. Includes formatting, strict all-target Clippy, library/integration tests, native GPU harnesses, PostgreSQL-specific suites, and desktop-file validation. Optional SSH tests remain environment-gated.
- Nine schema integration tests exercised planning, selections, blocked SQL, actual SQLite stale-baseline rejection and rollback, PostgreSQL alterations, constraint/index replacement, data preservation, and explicit namespace mapping.
- Designer test persists and reloads a draft from a reopened Store, rejects malformed restoration, verifies composite primary-key ordering, then applies designer-generated primary-key/index SQL through Session and independently checks SQLite integrity and index creation.
- PostgreSQL catalog test rejects unmodeled NULLS NOT DISTINCT, deferred unique, MATCH FULL and NOT VALID semantics rather than silently changing them.
- Native GPU harness exercised review/apply against temporary SQLite databases, independent post-apply inspection, stale failure, read-only rejection, comparison selection/review/Back, ER pointer zoom and layout round-trip. Captured seven screens/states in light and dark themes.
- Final designer action-row layout and primary-key ordering received strict Clippy, focused designer testing and a fresh GPU harness run after the full suite.

## Independent review

A read-only code-reviewer inspected the integrated diff and reported four actionable issues. Fixed: leaving an in-flight apply through Open SQL; unmodeled PostgreSQL constraint semantics; SQLite ordinary-index normalization; designer primary-key construction. Findings and verification are recorded in the conversation. Review was source-based; runtime evidence comes from the checks above.

## Deliberate limits

Unsupported definitions remain visible and blocked for mutation. These include complex index recreation and several SQLite rebuild features (AUTOINCREMENT, STRICT/WITHOUT ROWID, deferred foreign keys, view dependencies, collations/conflict policies). Namespace mapping rejects unresolved expressions/dependencies. Cross-engine conversion is excluded. Selection is by table difference, not each individual clause.

ER obstacle avoidance after arbitrary dragging, full screen-reader behavior, and cancellation under sustained database load were not verified. Captured components are native GPUI windows; screenshots are not browser mockups.

Draft restoration occurs when reopening the same designer context. It preserves the original baseline; external changes therefore cause stale-baseline rejection. Destructive consent resets on restoration. Save errors retain the open draft.

## Artifacts and cleanup

Generated artifacts are under `dist/schema-tools/` (ignored build output): full check log, final focused-check log and screenshots. The final response embeds the main screens and links the screenshot set.

All 18 completed task worktrees and their branches were removed after archiving branch history, dirty patches and untracked files under `/tmp/queri-schema-worker-recovery/`. The unrelated preexisting OpenCode worktree registration was left untouched. Only disposable test databases were mutated.
