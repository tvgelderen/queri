# Local GPUI patches

These are the published crates.io sources for `gpui-pre` **0.3.10**,
`gpui-pre-wgpu` **0.3.10**, and `gpui-component` **0.8.1**. Queri patches them
together through `[patch.crates-io]`, retaining the existing GPUI type family and
versions. Original licenses and notices are preserved.

Local changes:

- `gpui-pre/src/window/content_zoom.rs`, `src/window.rs`, and `src/platform.rs`:
  application content zoom independent of display density. Layout uses a smaller
  logical viewport while painting uses display density multiplied by zoom, so
  fixed pixel dimensions and text scale together. Platform pointer, scroll,
  touch, and native text-input geometry are converted at the window boundary;
  synthetic GPUI events remain in content coordinates. Resizing retains zoom
  without changing the native window dimensions.
- `gpui-component/src/window_border.rs`, `src/menu/`, and `src/input/popovers/`:
  decorations retain their native shadow inset and resize areas, while menus
  and editor popovers use the content viewport for placement and clipping.
  Run `cargo test --locked --features test-support --test ui_zoom` for native
  geometry and input checks, and `cargo test --locked --features test-support
  --test ui_zoom_render` for real GPU scaling and small-window dialog checks.
  Remove these patches together when upstream provides uniform content zoom.

- `gpui-pre/src/scene.rs` and `src/window.rs`: a clipped, opacity-aware backdrop
  paint operation with scene replay and sampling boundaries. The batching approach
  is adapted from the Apache-2.0 [Sailry material port, revision
  b9b7df79854ba971dfea77650825014cdfdb12c0](https://github.com/dux-web/gpui/commit/b9b7df79854ba971dfea77650825014cdfdb12c0)
  (itself selectively ported from `crabtalk/zed`).
- `gpui-pre-wgpu/src/backdrop.{rs,wgsl}` and the renderer integration: a separate
  render target, prefiltered quarter-resolution separable Gaussian blur, and
  opacity-correct compositing before the dialog foreground. No CPU screenshots,
  desktop capture permissions, compositor effects, or glass/lens effects.
- `gpui-component/src/dialog/dialog.rs` and `src/theme/mod.rs`: opt-in
  `Theme::dialog_backdrop_blur`, defaulting to zero. Ordinary dialogs and alerts
  share it, including their closing backdrop. Only the existing overlay owner
  paints the effect, so stacking dialogs does not compound the blur.
- `gpui-component/src/menu/{popup_menu,menu_item}.rs`: an opt-in destructive
  style for standard menu items. The menu owns foreground, hover, selected, and
  disabled styling; Queri's connection, database, and history deletion actions
  use native rows without duplicating menu geometry.

- `gpui-component/src/table/{state,data_table}.rs`: opt-in rectangular cell
  selection through `range_selectable(true)` and `selected_range()`. Dragging,
  Shift-clicking, and Shift+arrow keys extend the range; ordinary selection and
  row checkboxes retain their behavior. Queri resets ranges when results change.
  Native interaction tests cover keyboard, pointer, clipboard, and refresh behavior.

Queri enables an 8-logical-pixel Gaussian sigma in `src/ui/theme.rs`. These
patches implement blur on Linux's WGPU renderer (Wayland, X11, and headless).
Other renderers retain the ordinary dimmed overlay. Textures are allocated on
first use, reused across frames, and replaced on resize/device recovery. Frames
without blur retain the original direct rendering path.

Run `cargo test --locked --features test-support --test dialog_backdrop` for
real GPU pixel and dialog lifecycle checks. A Vulkan/OpenGL adapter is required;
Mesa lavapipe works for headless testing. `bash scripts/check.sh` includes it.

Upstream tracking: [GPUI Kit #3598](https://github.com/longbridge/gpui-kit/issues/3598).
When upstream provides equivalent backdrop support, remove the three backdrop
patches together after verifying the rendering tests. Remove the vendored sources
once upstream also provides destructive menu styling, rectangular table
selection, and uniform content zoom, and their interaction tests pass.

## PostgreSQL execution foundations

`tokio-postgres-0.7.18` is the published crates.io source, upstream revision
`f1cb6ec0d5766b136cbd68f3010d64142a5daa66`. Original licenses are retained.
Only `src/client.rs`, `src/query.rs`, and `src/simple_query.rs` are modified:

- `Client::transaction_status()` sends the existing protocol Sync request and
  returns the ReadyForQuery status byte. No SQL parsing, extra connection,
  cached-state inference, or transaction-changing SQL is involved.
- `Client::query_text_raw()` sends unnamed Parse/Bind/Describe-portal/Execute/Sync
  in one request. Bind parameters and results both use text format. PostgreSQL
  infers parameter types; NULL and empty text remain distinct. It reuses
  `SimpleQueryStream` and does not alter ordinary binary query methods.
- The text stream defers database errors until ReadyForQuery and then terminates.
  Dropped receivers retain the upstream connection driver's drainage behavior.

The application serializes these operations with its existing session mutex.
The text path supports one statement; the existing simple-query path remains
the script API. COPY streaming is outside both application query APIs.

To refresh this vendor, compare these three files with the pinned registry
source and rerun `tests/database.rs` against disposable PostgreSQL, including
ignored tests. Remove the patch only when upstream offers equivalent text binds
and authoritative status access with the same drainage guarantees.
