---
name: ios-gate-lane
description: iPad simulator gate / oracle / semantic hooks gotchas — case-insensitive log names, Xcode 26 xcresult JSON shape, reopened sketches unsolved, Swift 6 Timer capture, editor Save As from hooks
metadata:
  type: project
---

Facts from the ios-simulator-gate lane (feat/ipad-gate, 2026-10-22). Verify against code first.

- macOS volumes are case-insensitive: two logs named `otool-l.log` / `otool-L.log` are ONE file. Gate logs use `otool-load-commands` / `otool-libraries`.
- Xcode 26 `xcrun xcresulttool get test-results tests` JSON: `testNodes[]` → `Test Plan` → `Unit test bundle` / `UI test bundle` (name = target) → `Test Suite` → `Test Case` (`nodeIdentifier` = `Suite/test()`, `result` `Passed`); test cases may carry `Runtime Warning` children (not runs). `summary` has `totalTestCount`, `passedTests`, `failedTests`, `skippedTests`, `expectedFailures`, `devicesAndConfigurations[].device.deviceId`.
- A reopened `.onecad`'s projection carries no sketch solve (`SketchDto.status`/`dof` null); the oracle solves via `api::enter_sketch` + `cancel_sketch(.., Some(false))`. Solve status serializes PascalCase (`FullyConstrained`).
- Swift 6 (project uses `-swift-version 6`): a `Timer.scheduledTimer` block that calls `timer.invalidate()` inside `MainActor.assumeIsolated` fails ("sending 'timer' risks causing data races"); use a `@MainActor` flag object and invalidate after the wait. `XCTAssert*(try await ...)` does not compile (autoclosure).
- Hooks drive the editor's real File › Save As by pre-answering the save dialog (`onecad-ios` `Queue::save_answer`, test-hooks only) and polling `Editor::save_outcome` (`semantic-probe`); a directory at the target makes the save fail in the container rename (EISDIR).
- Sim smoke without the kernel lane: copy `target/ios-worker/<sdk>-<cfg>[-test-hooks]` from the integration worktree into your own `target/ios-worker/`; Acceptance build-for-testing on a fresh own simulator, then `TEST_RUNNER_ONECAD_RUN_ID=.. TEST_RUNNER_ONECAD_MAC_FIXTURE=.. xcodebuild test-without-building`.
- MESH1 positions are f32: bounds checks need a half-f32-ulp term per coordinate (2.86e-6 on a 60 mm span at 13.7); 1e-7 abs alone false-rejects off-origin plates.
- `Editor::request_save_as` must check `latest(File(Save)) == Pending`: `documents.running` may be another file action's job that refused the Save.
- Gate evidence dir must be new/empty and outside the tree (gate re-runs `git status` at the end); provenance kind is enforced per extension (document/.onecad, step/.step, trace else).
- `packaging/binaries/` is gitignored; `native-host-gate`/`ios-simulator-gate` need the staged manifest there (copy the kernel lane's, whose `binarySha256` matches its `worker/build/onecad-worker`).

**Why:** each cost a failed run or a false verdict in this lane.
**How to apply:** before touching xtask/src/evidence/ios_*, the oracle test, or OneCADNativeTests. See [[ios-xcode-lane]], [[ipad-host-lane]].
