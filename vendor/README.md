# Dialog backdrop blur patches

These are the published crates.io sources for `gpui-pre` **0.3.10**,
`gpui-pre-wgpu` **0.3.10**, and `gpui-component` **0.8.1**. Queri patches them
together through `[patch.crates-io]`, retaining the existing GPUI type family and
versions. Original licenses and notices are preserved.

Local changes:

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

Queri enables an 8-logical-pixel Gaussian sigma in `src/ui/theme.rs`. These
patches implement blur on Linux's WGPU renderer (Wayland, X11, and headless).
Other renderers retain the ordinary dimmed overlay. Textures are allocated on
first use, reused across frames, and replaced on resize/device recovery. Frames
without blur retain the original direct rendering path.

Run `cargo test --locked --features test-support --test dialog_backdrop` for
real GPU pixel and dialog lifecycle checks. A Vulkan/OpenGL adapter is required;
Mesa lavapipe works for headless testing. `bash scripts/check.sh` includes it.

Upstream tracking: [GPUI Kit #3598](https://github.com/longbridge/gpui-kit/issues/3598).
When upstream provides equivalent support, remove all three local patches and
the corresponding vendored sources together after verifying the rendering tests.
