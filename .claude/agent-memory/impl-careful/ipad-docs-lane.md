---
name: ipad-docs-lane
description: iPad document/Files/multi-tab simulator test gotchas — shared hosted process, gate-clean run dir, duplicate plate identity, sim-only content paths, picker detection, background state
metadata:
  type: project
---

Facts from feat/ipad-docs (2026-10-22). Verify against code first.

- Hosted native tests share ONE app process: tabs, recovery offers and recents leak between test classes. Start and end each document test with `DocumentDriver.closeEveryTab` (discards "Don't save", "Discard recovery").
- The gate's oracle fails on any `.onecad` in `Documents/acceptance/<run>/` it does not expect (`unexpected`): delete test-only artifacts and their `.provenance.json`; use `answer_save` (Documents/acceptance-exports) for non-model files.
- Every copy of the Mac plate has the same DocumentId: opening a second copy answers AlreadyOpen and focuses the first tab. Close tabs before opening it.
- Sim: Edit › Undo stays disabled right after opening a file (undo depth 0). A dirty edit without canvas drawing: Sketch tool → `onecad.plane.xy` → `onecad.tool.finishSketch` (empty Sketch row; adds 2 undo steps).
- Here a finger drag with Line created nothing and Rectangle showed no numeric chip before a first corner (see [[ipad-xcuitest-lane]] for software-keyboard typing and canvas mapping). STEP export needs bodies, so it is tested natively (Mac plate + `answer_save`).
- `XCUIApplication.state` does not reach `.runningBackgroundSuspended` within 30 s after `press(.home)`; wait for `.runningBackground`, then `activate()` before `terminate()`.
- Presented `UIDocumentPickerViewController` is found by walking each window's `rootViewController.presentedViewController` chain; cancel via `delegate?.documentPickerWasCancelled` + `dismiss`.
- The Activity button prefers any older Failed/Refused/Warning entry over the newest; check messages in its popover, not its label, unless the run has no failures.
- Fastest way to learn snapshot ids: a scratch XCUITest printing `app.debugDescription` per step (delete it and re-run `xcodegen generate` before committing).

**Why:** each cost a failed run in this lane.
**How to apply:** before writing iPad document/tab tests or touching `DocumentDriver`. See [[ios-gate-lane]], [[touch-lane]], [[ipad-host-lane]].
