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
- Insert rows with **Value**, **NULL**, or **Default** modes. Edit or delete rows from tables with a primary key, including composite keys. Mutations use parameters and compare the original row; conflicting edits fail without overwriting newer data. Bulk deletion is transactional. Views and keyless tables cannot be updated or deleted through the inspector.
- Open independent SQL tabs. **Run query** changes to **Run selection** when SQL is selected; each statement has its own result tab. Execution status, duration, and row counts appear above the results. Transactions stay on that tab's session until committed, rolled back, reconnected, or closed. Cancel stops a running query. Closing a tab with active work requests cancellation first.
- Select a rectangle of cells by dragging, Shift-clicking, or using Shift+arrow keys. **Copy** offers TSV with optional headers or typed JSON; `Ctrl+C` copies TSV. Row checkboxes remain a separate selection. **Export…** saves the loaded page/result, selected rows, or selected cells as CSV, TSV, or typed JSON. The dialog shows the exact scope and any truncation; exporting never reruns SQL. CSV/TSV include headers and a configurable NULL marker (default `\N`). JSON preserves column order, duplicate names, NULLs, numeric text, and binary values in a versioned envelope with ordered columns, row arrays, and tagged cells. Saves replace the destination atomically.
- Open **Saved queries** from the sidebar to search, favorite, rename, duplicate, or delete reusable SQL. Save from an editor or a history entry. Opening a saved query creates an editable draft without executing it; changes only update the library when explicitly saved. Connection/database associations are retained; deleting a connection preserves its saved queries and asks for a connection when reopening them. The SQL tab's **…** menu opens UTF-8 `.sql` files in a new draft and saves SQL to disk.
- Use `Ctrl+F` in the SQL editor, result grid, or cell inspector to search that focused surface. Grid search matches literal, complete cell text in visible columns of the **loaded result/page only**, with a case toggle, matching-cell count, and wrapping Next/Previous navigation (`Enter` / `Shift+Enter`). It scrolls and highlights the current cell without changing row checkboxes or the rectangle used for copying. Paste multiline search text; Escape returns focus to the grid.
- Use **Search SQL drafts** (`Ctrl+Shift+S`) to search all open SQL tabs, including live unsaved edits, across connections and databases. Results show the tab, connection/database, and a snippet. Enter opens a match and selects its text without connecting or executing SQL; Tab moves to the results for arrow-key navigation. Search is literal and case-insensitive, accepts multiline text, and shows the first 1,000 matches for broad queries. Results refresh when drafts change or tabs close.
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
| Ctrl+F | Search the focused SQL editor, grid, or cell inspector |
| Ctrl+Shift+S | Search all live SQL drafts |
| Enter / Shift+Enter (grid search) | Next / previous matching cell |
| Ctrl+C (grid) | Copy selected cells as TSV |
| Shift+arrow keys (grid) | Extend cell selection |
| F5 | Refresh table |
| Ctrl+E | Edit selected row |
| Alt+Enter | Inspect / copy selected cell or row |
| Ctrl+H | Toggle history |
| Ctrl+Tab | Next document |
| Ctrl+W | Close document |
| Arrow keys / Enter | Navigate / open explorer items |

## Storage

Application data lives in `$XDG_DATA_HOME/queri/queri.sqlite3` (normally `~/.local/share/queri/queri.sqlite3`). Set `QUERI_DATA_DIR` to choose another directory. Connection metadata, preferences, drafts, saved queries, and SQL history are stored locally. Remembered passwords use XChaCha20-Poly1305 encryption in the same database, with a random key in `queri.sqlite3.key` beside it. The key file has owner-only permissions (`0600`). Anyone who can read both files can decrypt the passwords. SQL text itself may contain sensitive values and is retained in drafts, saved queries, and history.

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
