---
name: ipad-host-lane
description: onecad-ios / onecad-ui desktop-feature gotchas — feature inheritance trap, release manifest prerequisite, simulator Metal limits, sim smoke harness recipe
metadata:
  type: project
---

Facts from the iPadOS UI host lane (feat/ipad-uihost, 2026-10-22). Verify against code before relying on them.

- `onecad-ui` `desktop` feature (default) gates eframe/rfd/muda/arboard/`run()`; other hosts use `onecad_ui::host::Editor` (App stays crate-private: `pub use app::App` triggers 7 `private_interfaces` warnings).
- `default-features = false` is silently ignored on `{ workspace = true }` deps whose workspace entry keeps defaults → onecad-ios uses path deps.
- `cargo test -p onecad-ui --no-default-features` compiles with ~12 dead-code warnings (test helpers used only by desktop-gated suites); not a gate.
- `onecad-app/build.rs` panics in ANY release build (incl. `cargo check --target aarch64-apple-ios --release`) without `packaging/binaries/onecad-worker-manifest.json`; stage with `ONECAD_WORKER_BUILD_DIR=$PWD/target/worker-build scripts/build-worker.sh Release` (~10 min).
- iOS Simulator Metal adapter: `max_inter_stage_shader_variables` 15 < wgpu default 16 (device request fails unless limits use `or_worse_values_from(adapter.limits())`); also lacks BASE_VERTEX, COMPARISON_SAMPLERS, CUBE_ARRAY_TEXTURES, INDIRECT_EXECUTION downlevel flags — CAD viewport on sim unverified.
- Sim smoke recipe: own device via `xcrun simctl create`, link an ObjC CLI with `xcrun -sdk iphonesimulator clang -target arm64-apple-ios18.0-simulator -fobjc-arc ... libonecad_ios.a -framework Foundation Metal QuartzCore CoreText CoreGraphics CoreFoundation UIKit Security -lc++ -liconv`, run with `xcrun simctl spawn <udid> <path>`; CAMetalLayer without a window presents fine.
- Simulator processes see the Mac filesystem (SF Mono resolved from /System/Library/Fonts) and debug logging writes to the compile-time `<repo>/logs`; neither holds on device.
- egui AccessKit: label widgets keep text in `value`; `TextRun` children duplicate it; title-bar drag region is an unnamed clickable node. egui never sets `author_id`; set it with `ctx.accesskit_node_builder(id, |n| n.set_author_id(..))` (touch menu: `onecad.menu.*`, matching the Xcode lane's TESTING.md ids).
- Touch cancel must never send a release (egui counts a release within 0.8 s/6 pt as a click): `onecad_ui::host::cancel_contact` resets egui's pointer state + `stop_dragging`, next frame carries `PointerGone`.
- epaint `TexturesDelta` drop `debug_assert`s `set`/`free` are empty: `mem::take` both or debug builds panic.
- macOS shell has no `timeout`; `xcrun simctl spawn` is fine without it.
- Shell commands containing `rm -rf` get held by a Blast Radius approval hook in unattended runs — avoid them.
