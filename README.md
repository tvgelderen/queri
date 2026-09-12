# Queri

A native Linux database workspace built in Rust with GPUI Kit. PostgreSQL and SQLite run directly in the desktop application.

## Run

Requires Rust 1.97 or newer, a Linux desktop (Wayland or X11), and an OpenGL/Vulkan-capable graphics driver. Software rendering works for testing. SQLite is bundled. PostgreSQL connections use the system OpenSSL trust store.

On Debian/Ubuntu, install the build dependencies:

```sh
sudo apt install build-essential clang cmake pkg-config libssl-dev \
  libfontconfig1-dev libfreetype6-dev libwayland-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libxcb1-dev libxcb-xkb-dev libvulkan1 libegl1 \
  desktop-file-utils
cargo run --locked
```

On Arch Linux: `base-devel clang cmake pkgconf openssl dbus fontconfig freetype2 wayland libxkbcommon libxkbcommon-x11 libxcb vulkan-icd-loader mesa desktop-file-utils`.

**Remember password** saves PostgreSQL passwords encrypted locally and reconnects automatically, without a desktop keyring or master password. **Test connection** checks database access without saving. Turn off **Remember password** to keep the entered password only for the current application session.

The native file picker uses `xdg-desktop-portal` and your desktop's file-chooser backend (for example, `xdg-desktop-portal-gtk`). You can also enter a SQLite path directly.

## Workspace

- Switch connections from the sidebar, with database type, host or file, and a connection-status icon beside the selected name. Search saved connections by name, type, host, or file path. Choose **New connection** at the bottom of the selector to add one, or use the active connection's **…** menu to edit, refresh, or remove it; failed connections offer **Retry** in the database explorer. PostgreSQL accepts a connection string or individual fields; advanced settings include TLS and timeout. SQLite offers **Open existing file** and **Create database** separately. Creation never overwrites an existing file; removing a connection keeps the database file.
- Use **Find table…** (`Ctrl+P`) to search tables and views across every schema in the selected database, including unopened explorer branches. You can also expand a database and schema in the sidebar. Use the Data, Structure, Indexes, and Constraints tabs to explore a table. **Filter** opens column and operator selectors; press Enter or choose **Apply** to filter. Text supports case-insensitive, literal contains, does not contain, equals, does not equal, starts with, and ends with. Numbers and dates support equality and ordered comparisons; every column supports **is NULL** and **is not NULL**. Negative comparisons exclude NULL values. **Equals** with an empty value matches empty strings, not NULL. SQLite date comparisons support ISO-formatted text in declared DATE/DATETIME/TIMESTAMP columns, not numeric timestamps or dates inferred from generic text columns. Applied filters remain visible when the filter editor closes. Pagination and an explicit rows-per-page menu sit below the data. Use **Row actions** for editing, deletion, inspection, and relationship navigation.
- Table filters support nested **All (AND)** and **Any (OR)** groups. Select a group and use **Add / replace condition**, or select an existing condition to replace it. **Add All group** / **Add Any group** adds a nested group; **Switch All/Any** changes its logic. Edits remain a draft until **Apply**. Invalid applies keep the previous applied filter and rows. **Filter from cell** adds an equality or NULL condition to a draft; Apply runs it. Relationship scope always combines with the entire user filter using AND.
- **Open SQL** opens an editable draft for the **currently loaded page**, including its applied grouped filters, relationship scope, ordered sorts, primary-key tie-breaks, limit and offset. It uses the backend’s actual browse compiler. Unapplied edits and failed reloads do not replace that successful page’s settings. Hidden/reordered columns are presentation-only; generated SQL returns all source columns. The draft opens on the same connection/database and does not execute automatically. Bind values are prefilled in memory for its first parameter form; editing the SQL or reconnecting clears them, and they are never restored after restart. Run uses the same confirmations, timeouts, transaction handling and cancellation as other SQL. A new draft has a separate session and cannot see the table tab’s uncommitted changes. Missing columns, tables or changed relationships produce a repair error when generating SQL.
- Click column headers to add or change sorts. Numbered headers and sort controls show precedence and direction; **Earlier** changes precedence and **Remove sort** removes a term. Queri appends missing primary-key columns for stable pagination. Equal rows in keyless tables have no guaranteed order.
- **Columns** hides/shows and reorders columns with **Earlier**. This changes presentation only: copy/export use visible column order, while editing, inspection and relationship actions keep complete source rows. Grid search defaults to visible columns; **Include hidden** searches all loaded cells and names hidden matches. Layout changes invalidate search results and cell selections.
- Applied table filters, sorts, column layout and page size survive restart. **Saved views → Load view** replaces the tab’s settings and relationship scope; **Save as…** creates a named view and **Save** explicitly updates its loaded revision. Unapplied condition edits are not saved. Saving from a relationship-scoped tab requires choosing reusable settings only or including that record’s scope. A conflicting save keeps your settings and asks you to reopen the saved view or use Save as. Views retain their connection/database/table association; removing a connection retains its views as unavailable entries in Saved views, without reassignment. Missing schema columns produce a repair error: replace/remove the affected condition or sort, or repair/reset Columns. Queri never silently removes predicates to broaden a query.
- Insert rows with **Value**, **NULL**, or **Default** modes. Edit or delete rows from tables with a primary key, including composite keys. Mutations use parameters and compare the original row; conflicting edits fail without overwriting newer data. Bulk deletion is transactional. Views and keyless tables cannot be updated or deleted through the inspector.
- Open independent SQL tabs. **Run query** changes to **Run selection** when SQL is selected; each statement has its own result tab. Execution status, duration, and row counts appear above the results. Transactions stay on that tab's session until committed, rolled back, reconnected, or closed. Cancel stops a running query. Closing a tab with active work requests cancellation first.
- Use **`:name` parameters** on either engine, PostgreSQL **`$1`**, or SQLite **`?` / `?NNN`**. Run (`Ctrl+Enter`) opens a parameter form before the usual execution confirmation. Repeated names share one value. Use one style per statement; numbered positions must be contiguous from 1 (up to 1,000). PostgreSQL array slice colons keep their native meaning; wrap a named subscript parameter in parentheses, such as `items[(:index)]`. Parameterized scripts are rejected before any statement executes; ordinary scripts keep their existing behavior. PostgreSQL accepts server text and explicit NULL, infers types, and supports SQL casts such as `:id::uuid`. SQLite offers Text, Integer (signed 64-bit), Real (finite), and Blob (hexadecimal pairs). NULL is a separate toggle; blank Text is an empty string, and blank Blob is zero bytes. Invalid inputs keep the form open; Cancel sends nothing.
- Parameter values exist only in memory. Saved queries and restored drafts retain parameter names and type choices, never entered values. History keeps the original SQL placeholders and a safe summary for parameterized errors; detailed server errors remain visible in the live tab because they may echo values. SQL literals you write yourself are still persisted. Editing SQL or changing sessions invalidates pending confirmations and generated parameter values.
- SQL tabs start in **Autocommit**. **Begin**, **Commit**, and **Rollback** use that tab's session, just like manual transaction SQL. The status shows **Transaction open**, **Transaction failed**, or **Transaction outcome unknown**, separately from query progress. Elapsed time starts when an open transaction is first observed; a long script may open its transaction before that observation. PostgreSQL failed transactions offer Rollback; you can still run manual `ROLLBACK TO SAVEPOINT`. SQLite errors use its actual autocommit state, since some errors leave a transaction open and others roll it back.
- SQL tabs have a **Timeout** menu: **Inherit connection/server** (the default), a custom positive duration in milliseconds, or **No application timeout**. The preference is saved with the draft and restored on reconnect. The limit applies separately to each statement, including computation and lock waits, rather than to the entire script or the displayed rows. A timeout is recorded as **Statement timed out** in the tab and history; transaction status is still read from the database. Roll back or recover a savepoint after a PostgreSQL transaction fails. Timeout settings are disabled while busy or while a transaction is failed.
- PostgreSQL uses only that session's `statement_timeout`. Inherit restores the policy observed when that session connected; None sets `statement_timeout` to zero. Other server policies can still stop work. Queri reapplies the preference before each submission, including text parameter binds, because rollback and manual `SET` can change the session setting. It does not prepend configuration SQL in a failed transaction or before a standalone Rollback; recovery uses the session’s existing policy. Raw scripts are sent unchanged: a manual `SET statement_timeout` inside a script controls subsequent statements in that script until the next submission. SQLite uses a scoped progress handler and deadline-aware lock wait for custom limits; its existing busy timeout remains a lock policy, not an execution timeout. Manual `PRAGMA busy_timeout` within a script can replace that script's lock handler. No application timeout removes Queri's execution deadline, but does not remove database lock policies. These controls apply to SQL execution and its transaction commands, not explorer metadata or administration.
- Closing or reconnecting a tab, editing/removing its connection, or quitting (`Ctrl+Q` or window close) checks for uncommitted work. Choose **Keep open** or **Roll back and close**; Queri never commits on close. Active work is cancelled first; wait for it to settle and retry the close. Database administration uses its existing rollback acknowledgment. A lost connection, especially during Commit, can leave the outcome unknown: Queri does not retry or report success. Verify the result independently before repeating the operation. Transaction state is never restored after restart. Forced process termination or operating-system shutdown cannot display an interactive confirmation.
- Select a rectangle of cells by dragging, Shift-clicking, or using Shift+arrow keys. **Copy** offers TSV with optional headers or typed JSON; `Ctrl+C` copies TSV. Row checkboxes remain a separate selection. **Export…** saves the loaded page/result, selected rows, or selected cells as CSV, TSV, or typed JSON. The dialog shows the exact scope and any truncation; exporting never reruns SQL. CSV/TSV include headers and a configurable NULL marker (default `\N`). JSON preserves column order, duplicate names, NULLs, numeric text, and binary values in a versioned envelope with ordered columns, row arrays, and tagged cells. Saves replace the destination atomically.
- Open **Saved queries** from the sidebar to search, favorite, rename, duplicate, or delete reusable SQL. Save from an editor or a history entry. Opening a saved query creates an editable draft without executing it; changes only update the library when explicitly saved. Connection/database associations are retained; deleting a connection preserves its saved queries and asks for a connection when reopening them. The SQL tab's **…** menu opens UTF-8 `.sql` files in a new draft and saves SQL to disk.
- Use `Ctrl+F` in the SQL editor, result grid, or cell inspector to search that focused surface. Grid search matches literal, complete cell text in visible columns of the **loaded result/page only**, with a case toggle, matching-cell count, and wrapping Next/Previous navigation (`Enter` / `Shift+Enter`). It scrolls and highlights the current cell without changing row checkboxes or the rectangle used for copying. Paste multiline search text; Escape returns focus to the grid.
- Use **Search SQL drafts** (`Ctrl+Shift+S`) to search all open SQL tabs, including live unsaved edits, across connections and databases. Results show the tab, connection/database, and a snippet. Enter opens a match and selects its text without connecting or executing SQL; Tab moves to the results for arrow-key navigation. Search is literal and case-insensitive, accepts multiline text, and shows the first 1,000 matches for broad queries. Results refresh when drafts change or tabs close.
- SQL completion opens with **Ctrl+Space** or automatically after typing an identifier prefix or qualifier (`alias.`). Use the arrow keys and Enter to accept, Escape to dismiss, and Ctrl+Z to undo the acceptance. Suggestions include SQL keywords, schemas, tables/views, columns with types and available PostgreSQL comments, aliases, and common CTE/subquery outputs. Queri quotes database identifiers and replaces the whole token, including its existing quotes. Completion works with unfinished SQL and stays closed inside comments, strings, and dollar-quoted bodies.
- **Refresh completions** (the refresh icon beside Format, **F5**, or the SQL tab’s **…** menu) reloads metadata for that document’s session. Hover the icon for metadata status. Names load first; columns load only for referenced relations. Caches are isolated by session and invalidated on reconnect, explicit refresh, and SQL submissions. Autocommit temporary tables and changed search paths become available on refresh. Busy, open, failed, or unavailable sessions use cached metadata; finish the transaction and refresh to retry. Completion errors never block editing or execution. Uncommitted DDL discovery is intentionally unsupported. Completion is bounded to 100 suggestions, 20,000 relation names, 16 column sources per request, 2,000 columns per relation, and SQL drafts up to 256 KiB. Complex expression output names and recursive CTE inference may be incomplete; completion does not validate SQL.
- Use **Format** (`Ctrl+Shift+F`) to format the selection or the entire SQL editor as one undoable edit. Formatting uses the connection's SQL dialect and preserves comments and literals. Unsupported or incomplete SQL is left unchanged with an explanation.
- Use **Row actions → Open referenced row…** or **Find referencing rows…** to follow foreign keys, including composite keys. Choose the relationship to open a scoped table tab with a visible, removable filter. Normal filters, sorting, and pagination work within that scope. Each tab has its own session: a new relationship tab cannot see uncommitted changes in the source tab.
- Double-click a cell or press `Alt+Enter` to inspect it. Switch between raw text, wrapped text, a collapsible JSON tree, formatted JSON, and binary hex where available. Text search uses `Ctrl+F`. PostgreSQL values retain the server's exact text representation; SQLite integers and binary values retain their native representation. **Copy raw** always copies the complete original value. Text previews stop at 1 MiB; JSON previews support up to 2 MiB, 64 nesting levels, and 20,000 nodes, preserving duplicate keys and numeric spelling.
- Open **Query history** from the sidebar, search with Enter, and select an entry to open it against its recorded connection and database without executing it. Entry menus offer copying, saving to the query library, and confirmed deletion. On narrower windows, history uses the document area; opening a query returns to the editor.
- Use **Create** beside Databases for PostgreSQL creation, or a database's **…** / right-click menu to rename, clone, delete, or inspect sessions. Each operation has a focused form; deletion requires the database name. Queri disconnects its affected sessions before administration, and explains that uncommitted transactions will roll back. Renaming updates open tabs and saved defaults; deleting keeps SQL drafts available for copying. Maintenance database and external-session termination are under Connection options.
- Open application preferences with the gear beside Query history or the **…** menu beside Queri. Document tabs support keyboard navigation, close buttons, and an overflow menu. With no tabs open, use **Browse tables** or **New query** to continue; the tab bar keeps its new-query action available.
- SQL drafts, open table tabs, window dimensions, sidebar/editor sizes, appearance, and destructive-SQL preferences survive a restart. Each document has its own database session; temporary objects and uncommitted transactions do not survive a restart.

SQL results are limited to **10,000 rows / 32 MiB per script** and marked when truncated. Table pages hold up to 1,000 rows. These are display limits, not database statement limits; statements still execute in full. SQL errors can follow earlier successful statements in a batch. Use explicit transactions when you need atomic scripts.

### Keyboard

| Shortcut | Action |
| --- | --- |
| Ctrl+N | New SQL tab |
| Ctrl+P | Find table in the current database |
| Ctrl+Shift+N | New connection |
| Ctrl+Enter | Run selected SQL or entire script |
| Ctrl+Shift+C | Cancel query |
| Ctrl+Shift+F | Format selected SQL or entire script |
| Ctrl+Space | Show SQL completions |
| Ctrl+F | Search the focused SQL editor, grid, or cell inspector |
| Ctrl+Shift+S | Search all live SQL drafts |
| Enter / Shift+Enter (grid search) | Next / previous matching cell |
| Ctrl+C (grid) | Copy selected cells as TSV |
| Shift+arrow keys (grid) | Extend cell selection |
| F5 | Refresh table / SQL completion metadata |
| Ctrl+E | Edit selected row |
| Alt+Enter | Inspect / copy selected cell or row |
| Ctrl+H | Toggle history |
| Ctrl+Tab | Next document |
| Ctrl+W | Close document |
| Ctrl+Q | Quit with transaction checks |
| Arrow keys / Enter | Navigate / open explorer items |

## Connection safeguards

Connection forms offer an optional **Development**, **Staging**, or **Production** label, shown as text beside the connection name and in document and confirmation context. The existing connection color remains a visual cue. Labels and policy are independent: selecting Production never changes execution. Older profiles default to no label and **Normal** policy.

- **Normal** preserves ordinary execution and existing destructive-SQL warnings.
- **Confirm executions** asks before each SQL submission, including selections and the shared submission path for bound parameters. Begin and Commit also ask. Toolbar Rollback and rollback during session-loss recovery remain available without an extra execution prompt; manually submitted rollback SQL still asks. Generated inserts and updates ask before saving. Row deletion and database administration use their existing operation-specific confirmation with connection, environment, and database context. Cancel sends no operation and makes no database changes. Explorer reads, refreshes, connection tests, and session inspection do not ask.
- **PostgreSQL Read-only by default** sets `default_transaction_read_only=on` when each session connects, including reconnects. PostgreSQL rejects ordinary persistent writes by default. Queri also blocks generated row mutations and database administration in the UI and Session API. **This is an accident-prevention default, not a security boundary.** Raw SQL can use `BEGIN READ WRITE` or change the default; PostgreSQL permits writes to existing temporary tables in read-only transactions. Queri observes current read-only settings after submissions when the transaction is not failed, and reports observed read-write overrides. It cannot report every transient change inside a script or promise immutable state. Generated changes remain blocked even after a raw SQL override. Use PostgreSQL role permissions for hard enforcement.
- SQLite retains its existing read-only file-opening option; the PostgreSQL default policy is not offered for SQLite. Normal policy does not reset PostgreSQL server, database, or role defaults.

Editing a connection checks its affected document sessions using the transaction-loss guard. The prompt identifies the sessions; keep them open or acknowledge rollback before editing. Saving refreshes profiles on all affected documents, and sessions reconnect with the saved policy. Uncommitted work is never silently discarded. No environment or transaction state is inferred from connection color.

## Storage

Application data lives in `$XDG_DATA_HOME/queri/queri.sqlite3` (normally `~/.local/share/queri/queri.sqlite3`). Set `QUERI_DATA_DIR` to choose another directory. Connection metadata, preferences, drafts, saved queries, named table views, and SQL history are stored locally. Remembered passwords use XChaCha20-Poly1305 encryption in the same database, with a random key in `queri.sqlite3.key` beside it. The key file has owner-only permissions (`0600`). Anyone who can read both files can decrypt the passwords. SQL text itself may contain sensitive values and is retained in drafts, saved queries, and history.

Back up both the database and its `.key` file to restore remembered passwords. Losing or damaging the key makes those passwords unrecoverable; Queri will not replace it while saved passwords remain. Restore the original key, or turn off **Remember password** for the affected connections to remove their saved passwords. Connections can also be removed without the key. If a damaged key file remains after all saved passwords are removed, delete that file before remembering passwords again.

Connections saved by older desktop versions retain their settings, drafts, and history. Enter previously remembered passwords once after updating. Queri no longer accesses desktop-keyring entries and leaves them untouched.

This desktop rewrite uses fresh storage. The former web application's `data/` and `.env` files are not imported.

## Verify

```sh
bash scripts/check.sh
```

This checks formatting, strict Clippy, native GPUI interaction tests, real-GPU dialog backdrop rendering, SQLite integration tests, and the desktop launcher. Rendering tests need a working Vulkan/OpenGL adapter; Mesa lavapipe supports headless testing. PostgreSQL integration tests need a disposable database and a role with `CREATEDB`:

```sh
docker run --rm --detach --name queri-test-postgres \
  -p 127.0.0.1:55439:5432 \
  -e POSTGRES_USER=queri_test -e POSTGRES_PASSWORD=queri-test \
  -e POSTGRES_DB=queri_test postgres:17-alpine
QUERI_TEST_POSTGRES='postgresql://queri_test:queri-test@127.0.0.1:55439/queri_test' \
  bash scripts/check.sh
docker stop queri-test-postgres
```

## Linux bundle

```sh
bash scripts/package-linux.sh
tar -xf dist/queri-0.1.0-linux-x86_64.tar.gz
./queri-0.1.0-linux-x86_64/install.sh
```

The archive contains the release binary, desktop launcher, icon, and a user-local installer (Python 3 required). It uses the build machine's system ABI; build on the oldest distribution you intend to support. The installer accepts an optional prefix instead of `~/.local`. The binary can also run directly from the extracted archive.

## Code

`src/database/` owns database I/O and mutations; `src/storage.rs` owns application persistence and credentials; `src/ui/` owns GPUI views and UI-thread completions. Tokio handles PostgreSQL I/O and dispatches SQLite and credential storage to blocking workers. GPUI Kit is pinned to `0.8.1`; `Cargo.lock` pins its GPUI snapshot and the remaining dependencies. `vendor/README.md` documents the pinned Linux renderer and shared-dialog patches for true backdrop blur.
