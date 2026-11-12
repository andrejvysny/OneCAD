---
name: native-gate-lane
description: Native host gate / shared CARGO_TARGET_DIR traps, model-handle camera-flake repro, onecad-ui test prerequisites, CI CTest summary variant, reason-code presentation seams
metadata:
  type: project
---

Facts from the 2026-11-11 hardening-worker lane (Package 1-3). Verify against code first.

- A CARGO_TARGET_DIR shared by several worktrees of this repo clobbers artifacts: cargo hashes path packages relative to the workspace root, so every worktree writes the SAME `debug/xtask` and `deps/onecad_ui-<hash>`, and cargo can report `Fresh` for another worktree's binary. `cargo xtask` then `set_current_dir`s into THAT worktree (`env!("CARGO_MANIFEST_DIR")`). Recipe: touch a source, `cargo build -p xtask`, copy `debug/xtask` out, check `strings | grep <worktree>/xtask`, run the copy. Touch `crates/onecad-ui/src/lib.rs` before gate runs.
- `native_hole_scalar_click_drag_release_and_losses...` camera-loss flake reproduced only under GPU contention: 8 parallel loops x15 of the ignored test gave 3/120 failures, 0/30 idle. Cause was `app.doc.view` (last RENDERED view) staying stale when `viewport::render` early-returns (renderer busy / sampling fence open).
- Deterministic stale-view trick in host tests: `app.sampled = false; app.sampling_done = Arc::new(AtomicBool::new(false))` before a frame makes `render` return before drawing.
- Non-ignored `cargo test -p onecad-ui` needs `ONECAD_WORKER_PATH` (datum/regression tests panic without it) and, with `ONECAD_REQUIRE_WORKER=1`, the staged Bambu addon (`scripts/build-addons.sh addons/bambu-studio` + `ONECAD_TEST_ADDON_COMPONENT=.../extension.wasm`).
- GitHub macos-15-arm64 CTest prints `100% tests passed out of N`; local CTest 4.1.3 prints `100% tests passed, 0 tests failed out of N`. Fixture: `xtask/src/qualification/fixtures/ctest-macos-15-success.txt`.
- Native UI has NO worker-capability gating (`op.sweep`/`op.gear`/`op.featurePattern` never reach onecad-ui; hello caps stay in onecad-app worker manager).
- Edit-preview failures are stringified inside pure `onecad-interaction::edit_preview` (`error.to_string()`), so onecad-ui reason text cannot reach them without an interaction change.

**Why:** each cost a wasted gate run or a wrong-binary result in this lane.
**How to apply:** before running any gate or xtask in a /private/tmp worktree with a shared target, and before touching diagnostics presentation. See [[ipad-render-lane]].
