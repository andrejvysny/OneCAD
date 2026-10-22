# iPadOS host testing

Status: test layout and automation identifiers for the iPad host (Plane IP18
ONECAD-465). Evidence classes and their separation are defined in
[ARCHITECTURE.md](ARCHITECTURE.md#qualification-policy); build steps in
[BUILD.md](BUILD.md). This file asserts no test result.

## Scheme, plan and targets

| Item | Value |
|---|---|
| Shared scheme | `OneCADiPad` (`platforms/ios/OneCAD.xcodeproj/xcshareddata/xcschemes`) |
| Test configuration | `Acceptance` (Rust `test-hooks`, `ONECAD_ACCEPTANCE`) |
| Test plan | `platforms/ios/iPadAcceptance.xctestplan`: one configuration, both targets, default (non-random) order, automatic screen capture as screenshots (`preferredScreenCaptureFormat`), which Xcode keeps for failing tests |
| `OneCADNativeTests` | XCTest bundle hosted in the app. Calls the C ABI with the app's own `onecad_ios_app` (one per process); test-hook tests compile only under `ONECAD_ACCEPTANCE` |
| `OneCADUITests` | XCUITest of the real app through public UI only. Shared helpers: `OneCADUITests/UITestSupport.swift` |

```sh
xcodebuild build-for-testing -project platforms/ios/OneCAD.xcodeproj \
  -scheme OneCADiPad -testPlan iPadAcceptance -configuration Acceptance \
  -destination 'platform=iOS Simulator,id=<UDID>' -derivedDataPath build/DerivedData
xcodebuild test-without-building -project platforms/ios/OneCAD.xcodeproj \
  -scheme OneCADiPad -testPlan iPadAcceptance \
  -destination 'platform=iOS Simulator,id=<UDID>' -derivedDataPath build/DerivedData \
  -resultBundlePath <fresh path>.xcresult
```

Rules:

- Readiness is awaited with `XCTNSPredicateExpectation` (`waitUntil`), never
  with sleeps.
- Native and UI tests pass only against the integrated Rust staticlib and
  the real in-process kernel. A build linked with `ONECAD_IOS_STUB_LIB_DIR`
  proves compilation and linkage only.
- Process termination in the simulator is labeled termination, never jetsam.

## Accessibility identifiers

Identifiers are stable, never localized and dot-separated. **Native** ones are
set by Swift in `platforms/ios`. **Snapshot** ones must be exported by Rust
through `onecad_ios_a11y_snapshot` (`identifier` field) from the egui
widgets; they are the planned catalogue and are not implemented by the host.
UI tests address elements by identifier with
`descendants(matching: .any)`, because a snapshot node's XCUITest element type
follows its role (button, static text or other).

| Identifier | Source | Element | Used by |
|---|---|---|---|
| `onecad.canvas` | Native | `MetalCanvasView`, accessibility container of all snapshot nodes | `LaunchTests`, `OrientationTests` |
| `onecad.error.startup` | Native | Full-screen error when `onecad_ios_app_create` fails or the ABI version differs | `LaunchTests` (must be absent) |
| `onecad.error.relaunchRequired` | Native | Full-screen error after `ERR_TERMINAL`/`ERR_PANIC` | `LaunchTests` (must be absent) |
| `onecad.error.title`, `onecad.error.message`, `onecad.error.hint` | Native | Labels inside either error view | — |
| `onecad.save.name` | Native | Name field of the SAVE_DOCUMENT prompt | planned document tests |
| `onecad.import.failed` | Native | Alert when an OPEN_DOCUMENT/"Open in" copy into `Documents/Inbox` fails | planned document tests |
| `onecad.keyInputProxy` | Native | Hidden first responder for the keyboard; not an accessibility element | — |
| `onecad.viewport` | Snapshot (egui viewport, role CANVAS) | 3D viewport region | planned |
| `onecad.menu.file` | Snapshot (egui "File" menu button) | In-app File menu | `LaunchTests`, `OrientationTests` |
| `onecad.menu.file.new`, `.open`, `.save`, `.saveAs`, `.closeTab`, `.import`, `.insertStep`, `.export` | Snapshot (egui File menu items, `crates/onecad-ui/src/touch_menu.rs`) | File commands (iPad has no native menu bar for them) | planned document tests |
| `onecad.menu.edit.undo`, `onecad.menu.edit.redo` | Snapshot (egui Edit items) | Undo/redo | planned |
| `onecad.tab.<n>`, `onecad.tab.new` | Snapshot (egui document tabs, role TAB) | Document tabs, 0-based | planned multi-document tests |
| `onecad.tool.<tool-id>` | Snapshot (egui tool buttons; `<tool-id>` = tool registry id) | Modeling and sketch tools | planned tool tests |
| `onecad.dimension.input` | Snapshot (egui numeric field, role TEXT_FIELD) | Dimension/parameter entry | planned numeric-entry tests |

## Test-hook commands (`onecad_ios_test.h`)

Acceptance-only (`test-hooks`), main thread, JSON in and out:
`onecad_ios_test_command` returns `{"ok":true,...}` or
`{"ok":false,"error":"..."}`, freed with `onecad_ios_test_string_free`.
`ping`, `replay` and `prediction` are catalogued in
[HOST.md](HOST.md#test-hooks). The semantic commands below
(`crates/onecad-ios/src/test_hooks_semantic.rs`) give native tests the
verifiable artifacts `cargo xtask ios-simulator-gate` checks
([SIMULATOR_GATE.md](SIMULATOR_GATE.md)).

| Command | Request | Response fields |
|---|---|---|
| `run_id` | `{"cmd":"run_id","set"?:id}` | `runId`: `set` replaces the current id; otherwise the one read at launch from `ONECAD_RUN_ID` or `TEST_RUNNER_ONECAD_RUN_ID`; `null` when none |
| `kernel_identity` | `{"cmd":"kernel_identity"}` | `kernelIdentity`: `backend` (`in-process`, or `null` before a document host runs), `embedded` (manifest the binary verifies against), `linked` (manifest of the linked bundle), worker `hello`, `epoch`, `workerState`, `journalWriteFailures`, `relaunchRequired` |
| `document_summary` | `{"cmd":"document_summary"}` | ticket; then `summary` (`tabCount`, `document`: path, dirty, revision, undo/redo depth; `geometrySource`; `features`: id, opType, label, valueText, primaryValue, stored `params` (as a re-edit reads them), status; `bodies`: solids, faces, kernel `volume`, `surfaceArea`, `centroid`, tessellation `min`/`max`; `sketches`: status and `dof` of the last solve in this session, entity and constraint counts) and `kernelIdentity` |
| `save_artifact` | `{"cmd":"save_artifact","name"}` | ticket; then `saved`, `path`, `clean`, `status`, `provenance` |
| `export_step_artifact` | `{"cmd":"export_step_artifact","name"}` | ticket; then `path`, `provenance` |
| `write_semantic` | `{"cmd":"write_semantic","name"}` | ticket; then `path` of `<name>.json` |
| `seal_artifact` | `{"cmd":"seal_artifact","name"}` | `provenance` (kind `trace`) for a trace record a test wrote into the run directory; `.onecad`, `.step` and `.json` names are refused |
| `fault` | `{"cmd":"fault","kind":"journal_write"\|"save_failure"\|"clear"}` | `journalWrite`, `saveFailureArmed` |
| `result` | `{"cmd":"result","ticket"}` | `pending: true` until the ticket's command finished, then its fields plus `runId`; a failure is `ok:false` once |

Rules:

- Commands that wait on document services answer `{"pending":true,"ticket":n}`
  and never block the main thread; poll `result` from the run loop. At most 64
  unread tickets.
- Artifacts go to `Documents/acceptance/<run id>/`; names are 1–64 of
  `[A-Za-z0-9._-]`. Without a run id every artifact command fails. Every
  artifact carries the run id and `kernelIdentity`: JSON files inside
  (kind `semantic`, from `write_semantic`), `.onecad`, STEP and trace files
  in `<file>.provenance.json` (run id, kind, SHA-256 of the file, kernel
  identity). The gate accepts kind `document` only for `.onecad`
  (`save_artifact`), `step` only for `.step` (`export_step_artifact`) and
  `trace` for anything else (`seal_artifact`). Anything else in the
  directory, subdirectories and symlinks included, is rejected by the gate.
- Every required simulator model (`plate`, `plate-fillet`, `plate-chamfer`,
  `mac-plate-resaved`) needs `save_artifact` and then `write_semantic` with
  the same name: the gate checks the in-process kernel's own summary (history,
  one healthy solid, volume, bounds, live geometry) against the expectations
  as well as the Mac regeneration of the saved file.
- `save_artifact` runs the editor's own File › Save As (document flow, document
  services, `api::save_document`, path claim, recents, Saved status); only the
  name prompt is answered by the hook (`Queue::save_answer` in
  `crates/onecad-ios/src/host.rs`). The document is then bound to the
  artifact path, as after a user's Save As, so save the artifact last or save
  it again after later edits.
- `export_step_artifact` calls `api::export_step_file` with the artifact path
  and publishes nothing (no `EXPORT_FILE` request).
- `fault save_failure` arms the next `save_artifact` only: a directory takes
  the target's name, the editor's Save As fails in the container write and
  reports it; the result is `saved:false, injectedFailure:true` with the
  editor's status line, and only when that status names the container
  write's `Is a directory` (EISDIR). A Save the editor refused (another file
  action running) or one that failed elsewhere is an error. `fault journal_write` replaces the kernel journal
  directory by a regular file, so journal writes fail and are counted
  (`journalWriteFailures`); `clear`, the bridge's destruction and the next
  launch restore it without the fault window's `inflight.json`. Nothing else
  is injectable.
- XCUITest runs in another process and cannot call hooks: UI tests prove UI
  behavior and screenshots; model artifacts come from native tests.
  `launchOneCAD` passes the run id on as `ONECAD_RUN_ID`.

Swift helpers: `platforms/ios/OneCADNativeTests/SemanticHooks.swift`
(`SemanticHooks.attach`, `call`, `run` with ticket polling, `runID`,
`kernelIdentity`, `documentSummary`, `saveArtifact`, `exportStepArtifact`,
`writeSemantic`, `fault`; `XCTestCase.waitFor` polls on the main run loop).
`SemanticHookTests.swift` uses them for the in-process identity, the Mac
fixture round trip and both faults.

Release and Debug archives must not export any `onecad_ios_test_` symbol; the
Xcode build phase refuses them, and `cargo xtask ios-simulator-gate` checks
the Release products.
