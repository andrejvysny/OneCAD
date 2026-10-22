---
name: ipad-render-lane
description: Render profile / display budget / surface lifecycle seams (feat/ipad-render) — renderer test idling, Host::ui skips logic, Mobile host tests + zoom, preparation cache in Shared, sim background state
metadata:
  type: project
---

Facts from the iPad rendering lane (feat/ipad-render, 2026-10-22). Verify against code first.

- `RenderProfile` is fixed at `Renderer::with_profile` (pipelines + IBL built once); `Renderer::new` = Desktop. `Editor::new`/`App::new_with` take the profile; `DisplayBudget::for_profile` gives the renderer's `RenderLimits`.
- `limits::quality_target_bytes(size, options, profile)` must mirror `Targets::new` exactly (Desktop 94 B/px, Mobile 52/80); change both together.
- A fresh `Renderer` is `Flight::Submitted` (IBL prefilter): call the tests' `idle()` before `set_options`/`resize` or get `Busy`.
- Delivery-test `Host::ui` runs route + ui only, never `logic`/`poll`: service replies (e.g. `Prepared` after `activate_tab`) need `pump_until` (logic ticks) before `redraw`.
- A host-driven viewport resize can pass through a zero-size suspend, so `MemoryEstimate::peak_gpu_bytes` after it shows no overlap; assert overlap with a direct `renderer.resize` after `input::fence_previous_frame` + device wait.
- Renderer output readback in onecad-ui tests: `renderer.output_view().unwrap().texture()` (wgpu 30 `TextureView::texture`, output has COPY_SRC); `Thumbnail` textures lack COPY_SRC outside onecad-render's own cfg(test).
- zsh: `grep --include=*.rs` unquoted fails ("no matches found"); quote the glob. `cargo test -- --ignored --include-ignored` is an error (mutually exclusive).
- Mobile host-route tests: `boxed_with(root, RenderProfile::Mobile)` (builds via `App::new_with`). iPad 13" = `set_zoom_factor(2.0)`, one `host.ui` pass, then `host.size = 1366x1024` (egui screen_rect is in points; the zoom-applying pass rescales the old screen, so a size set before it gives a stale half-size view).
- A document's `PreparationCache` lives in `bridge::Shared` (take/restore around each preparation); `PreparedScene::cpu_bytes()` is a peak charge, `cpu_bytes_beyond(cache)` is resident.
- iOS simulator never reports `.runningBackgroundSuspended` after Home; poll for `.runningBackground` too.
- Disk on this machine swings fast with concurrent lanes (30 → 8.5 GiB in one session, not from this worktree's 13G target); recheck `df` right before Xcode builds.

**Why:** each cost a failed iteration or a blocked gate in this lane.
**How to apply:** before touching onecad-render limits/targets, onecad-ui display_budget or iOS lifecycle. See [[ipad-host-lane]], [[touch-lane]].
