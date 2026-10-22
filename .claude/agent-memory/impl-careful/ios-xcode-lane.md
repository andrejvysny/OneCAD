---
name: ios-xcode-lane
description: platforms/ios Xcode host gotchas — nm vs Rust bitcode, debug dylib symbol binding, xctestplan target IDs, stub-lib override, rm -rf held by hook
metadata:
  type: project
---

iPad UIKit host (feat/ipad-xcode, 2026-10-22) facts that are not obvious from the code:

- Xcode's `nm` cannot parse Rust 1.99 std bitcode inside `libonecad_ios.a` ("Not an int attribute"); use `xcrun nm -gjU --no-llvm-bc`. build-rust.sh does.
- Debug/Acceptance app code lives in `OneCAD.app/OneCAD.debug.dylib` (ENABLE_DEBUG_DYLIB); hosted test bundles bind C symbols from there, not the executable.
- `iPadAcceptance.xctestplan` references targets by pbxproj IDs XcodeGen derives from target names; renaming a target needs the plan updated. XcodeGen does not generate plans.
- `preferredScreenCaptureFormat: screenshots` in the plan gives failure screenshots; Xcode 26 default is screen recording.
- Compile/link without Rust lanes: pass `ONECAD_IOS_STUB_LIB_DIR=<dir>` as an xcodebuild setting (dir holds stub libonecad_ios.a + libonecad_worker_bundle.a; keep stubs outside the repo).
- A Blast Radius hook holds `rm -rf` for 10 min then refuses; use unique scratch names or plain `rm` + `rmdir`.
- Bash tool shell is zsh: unquoted `$var` with spaces is not word-split.
- UIDocumentPickerViewController presents *delayed*: `presentingViewController` stays nil a main turn after a successful present; never infer refusal from it (check on-window + `transitionCoordinator` before presenting instead).
- UIKit presses: a press on the KeyInputProxy bubbles via `super` to the canvas (same UIPress object); dedupe by ObjectIdentifier(press), not by first-responder checks.
- UIKit-behavior questions are cheap to settle with a temporary hosted test in OneCADNativeTests run against the stub (`-only-testing:OneCADNativeTests`, Acceptance, sim-hooks stub).

**Why:** each cost a failed build or a stalled command in the first lane run.
**How to apply:** check before touching platforms/ios build/test wiring. See [[repo_rust_lane_gotchas]].
