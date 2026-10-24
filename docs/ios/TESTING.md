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
set by Swift in `platforms/ios`. **Snapshot** ones are AccessKit author ids set
by the shared egui editor (`crates/onecad-ui/src/automation.rs`; the File menu
in `touch_menu.rs`) and exported through `onecad_ios_a11y_snapshot`
(`identifier` field); macOS carries the same ids. GPU-free Rust tests assert
that pill tools and More-catalog rows, the multi-select switch, operation
fields and footer, sketch creation chips, the sketch dimension editor,
document tabs and the sketch strip actions appear in an egui AccessKit
`TreeUpdate` (`onecad-ui` `automation::tests`, `pill::automation_tests`,
`sketch_numeric::editor::tests`, `constraint_authoring::tests`,
`session_strip::tests`). The ignored Metal + real-worker host tests in
`model_operation_delivery_tests` (`ONECAD_REQUIRE_GPU=1 ONECAD_REQUIRE_WORKER=1`)
tap the running editor by these ids, including the viewport, history rows,
document tabs and plane buttons that only the full App draws; they are macOS
evidence, not simulator acceptance. Repeats of one id in a snapshot
get `#2`, `#3` (HOST.md). UI tests address elements by identifier with
`descendants(matching: .any)`, because a snapshot node's XCUITest element type
follows its role (button, static text or other).

| Identifier | Source | Element | Used by |
|---|---|---|---|
| `onecad.canvas` | Native | `MetalCanvasView`, accessibility container of all snapshot nodes | `LaunchTests`, `OrientationTests` |
| `onecad.error.startup` | Native | Full-screen error when `onecad_ios_app_create` fails or the ABI version differs | `LaunchTests` (must be absent) |
| `onecad.error.relaunchRequired` | Native | Full-screen error after `ERR_TERMINAL`/`ERR_PANIC` | `LaunchTests` (must be absent) |
| `onecad.error.title`, `onecad.error.message`, `onecad.error.hint` | Native | Labels inside either error view | — |
| `onecad.save.name` | Native | Name field of the SAVE_DOCUMENT prompt | `RequiredWorkflowTests`, `DocumentLifecycleTests`, `DocumentPickerTests` |
| `onecad.import.failed` | Native | Alert when an OPEN_DOCUMENT/"Open in" copy into `Documents/Imported` fails | planned document tests |
| `onecad.keyInputProxy` | Native | Editable accessibility target for the focused field; mirrors its full draft and selection | `TextInputTests` |
| `onecad.viewport` | Snapshot (egui viewport, role CANVAS) | 3D viewport region | `SurfaceLifecycleTests` |
| `onecad.menu.file` | Snapshot (egui "File" menu button) | In-app File menu | `LaunchTests`, `OrientationTests`, `SurfaceLifecycleTests`, `RequiredWorkflowTests` |
| `onecad.menu.file.new`, `.open`, `.save`, `.saveAs`, `.saveAll`, `.closeTab`, `.import`, `.insertStep`, `.export`, `.sendCopy` | Snapshot (egui File menu items, `crates/onecad-ui/src/touch_menu.rs`) | File commands (iPad has no native menu bar for them) | `.new`: `SurfaceLifecycleTests`; document native and UI tests (below) |
| `onecad.menu.edit.undo`, `onecad.menu.edit.redo` | Snapshot (egui Edit items) | Undo/redo | `MultiDocumentTests` (undo) |
| `onecad.tab.<n>`, `onecad.tab.new` | Snapshot (egui document tabs, role TAB; "+" button) | Document tabs, 0-based in strip order | `MultiDocumentTests` (tab switches through the strip; no tab-selection hook exists), document UI tests |
| `onecad.tool.<tool-id>` | Snapshot (egui pill tool buttons; `<tool-id>` = `ToolId::automation_name`: `select`, `sketch`, `line`, `rectangle`, `circle`, `arc`, `slot`, `point`, `dimension`, `constraints`, `extrude`, `fillet`, `chamfer`, `offsetFace`, `moveRotate`, `edit`, `import`, `more`, …) | Tools shown in the pill and rows of its More catalog (`onecad.tool.more`); a narrow pill draws only the first tool and More, the rest stay addressable as catalog rows, each id once | implemented; planned tool tests |
| `onecad.tool.finishSketch`, `onecad.tool.discardSketch` | Snapshot (sketch session strip buttons) | Finish or discard the open sketch | implemented |
| `onecad.dimension.input` | Snapshot (egui numeric field, role TEXT_FIELD) | Primary value entry: the running operation's first declared field (fillet radius, chamfer distance, offset distance, extrude distance, translation X), a sketch creation chip's first field (line length, rectangle width, circle Ø, arc radius, point X) or the sketch dimension editor's value (Distance/Diameter/Dimension tools, dimension edit). Return applies a sketch chip or dimension | implemented; planned numeric-entry tests |
| `onecad.param.<name>` | Snapshot (egui numeric fields) | Every other field of the running operation (`translationY`, `translationZ`, `rotationAngle`, `extrudeSecondDistance`, …) or of a sketch creation chip (`height`, `y`, `sweep`) (`automation.rs`) | implemented |
| `onecad.op.confirm`, `onecad.op.cancel` | Snapshot (operation card or History editor buttons) | Create/Apply and Cancel of the running operation | implemented |
| `onecad.plane.xy`, `.xz`, `.yz` | Snapshot (sketch support chooser buttons) | Sketch on an origin plane | implemented |
| `onecad.history.<index>` | Snapshot (Inspector History rows) | Feature row, 0-based in history order | `MultiDocumentTests` (selection), `DocumentLifecycleTests`, `DocumentPickerTests` (row text) |
| `onecad.select.additive` | Snapshot (pill multi-select switch; toggled sets `A11Y_SELECTED`) | Plain viewport picks on an item toggle it instead of replacing the selection; a tap on empty space still clears and a double tap still selects the whole body alone | implemented |

## Test inventory

| Test identifier | Target | Evidence |
|---|---|---|
| `OneCADNativeTests/TestHookPingTests/testPingThroughTestCommand` | native | class 3 |
| `OneCADUITests/LaunchTests/testLaunchShowsCanvasAndFileMenu` | UI | class 4 |
| `OneCADUITests/OrientationTests/testRotationKeepsAppResponsive` | UI | class 4 |
| `OneCADUITests/RequiredWorkflowTests/testPencilOnlyCanvasWithNumericControlsHistoryUndoSaveReopen` | UI | class 4, public numeric controls, history, undo/redo and file workflow |

The exact checked-in inventory also includes three `FrameLoopTests` covering
reentrant wakes, short retries, suspension, idle and finite timer delays, and two
`AccessibilityBridgeTests` covering unchanged-tree reuse, changed text and native
text-proxy focus transitions. These are scheduler/cache regressions, not frame-rate
or physical touch-latency evidence; see [PERFORMANCE.md](PERFORMANCE.md).

### Synthetic Pencil modeling

`PencilModelingTests/testSyntheticPencilStrokeRectangleExtrudeAndHistoryInLandscape`
requests landscape through the canvas's `UIWindowScene.requestGeometryUpdate`,
propagates rotation errors, and waits for the scene orientation and stable actual
landscape viewport before delivery. Teardown restores the prior supported scene
orientation.

In windowing modes that reject programmatic orientation, rotate the dedicated
simulator to landscape through Device > Rotate Left before the native target.
The test avoids an unnecessary request only when both the actual scene
orientation and canvas are already landscape; it still requires a stable
landscape viewport. The dedicated UI orientation test still exercises all four
rotations and restores an actual landscape canvas in teardown.

Confirmed `kind: pencil` samples traverse the production normalizer/router with
pressure and hover absent. Public
controls choose Pencil Stroke, Rectangle and Extrude; semantic commands only
observe accepted geometry and seal evidence. The test accepts a continuous
straight stroke (three entities), undoes it, then drags Circle and Line. Each
keeps zero authoritative entities/history changes before lift, commits exactly
two or three entities after lift, and returns to zero with one Undo. It completes a rectangle with a
Pencil press-drag-lift, then drags the extrude distance handle with Pencil samples and
commits. It checks draft-only history before commit, live solid volume/bounds,
and model Undo/Redo. `acceptance/<run-id>/pencil-modeling.trace` records synthetic
input and summaries. Visual evidence is captured separately: hosted native XCTest
does not have UI automation authorization for `XCUIScreen` screenshots.
The test creates and closes only its own unsaved tab and preserves existing
document files. This is simulator input evidence, not physical Pencil, pressure,
hover, prediction appearance or haptic qualification.

The exact Acceptance inventory includes this test, the six native text-session
tests and six document-inbox tests. No skip allowance is added for them.

## Required public workflows (`RequiredWorkflowTests`)

These two tests use public XCUITest only (evidence class 4): finger controls,
keyboard keys, Files pickers, native prompts, and process termination/relaunch.
They do not synthesize Pencil input or call app test hooks. Canvas authoring and
picking require Pencil or an external pointer; fingers operate controls and
navigation. The former finger-authored plate workflow is superseded by that
policy, not retained as Pencil evidence.

`testPencilOnlyCanvasWithNumericControlsHistoryUndoSaveReopen` checks:

1. New part, Sketch, stable `onecad.plane.xy`, Rectangle: two finger taps and a drag
   leave `Entities: 0` and a fully constrained empty sketch.
2. Point and Numeric input: the public keyboard enters X=30 mm and Y=20 mm.
   Acknowledged values, the point plus its fixed construction origin (two
   entities), and exactly three constraints (Fixed, horizontal30, vertical20)
   prove controls remain usable with fingers. Undo restores zero entities and
   constraints; Redo restores two entities, three constraints and zero DOF.
3. Discard that sketch, close its tab, then File > Open through the real Files
   picker: On My iPad > OneCAD Fixtures > the gate's current Mac-authored plate fixture.
4. Require exact history labels `Sketch, Plate`, the selected Plate row's
   `Extrude · 4.0 mm` operation detail, one healthy body, width60 mm,
   height40 mm and diameter6 mm. Edit the first sketch's width to70 mm through
   its constraint control, Undo to60, Redo to70, Undo to60.
5. Save As `Plate-<run>`, terminate, relaunch, reopen from Recent and check the
   same history, health and exact sketch dimensions.

`testOpenedPlateSaveAsAndStepExportThroughNativePrompts` opens the same fixture
through Files, saves as `PromptPlate-<run>`, then exercises Save As defaults,
case-insensitive extension normalization, overwrite Cancel/Replace, STEP prompt
Cancel/Save, the native export picker and the completed export's normalized
path. Relaunch/reopen retains exact `Sketch, Extrude` history and one healthy
body. The independent Mac oracle checks saved geometry and STEP output.

The fixture is the current gate's Mac-authored 60×40×4 mm plate with centered
Ø6 mm through hole, fourteen sketch entities, thirteen constraints, zero DOF
and exact history `Sketch, Extrude`. The public tests open existing geometry;
from-new Pencil authoring, fillet/chamfer, handle drags, exact geometry and
model history have separate synthetic native tests and physical acceptance.
A public workflow pass must not be reported as physical Pencil qualification.

Before these tests, the gate fresh-installs OneCAD and the separate
simulator-only `OneCADFixtureSource` app on its dedicated simulator. It copies
the freshly authored Mac fixture into the source app's Documents directory.
The source exposes that directory through the public Files provider; the
shipping OneCAD app retains its copy-only document policy. The source target
has no dependency from OneCAD and is excluded from shipping archives. Record
the fixture hash/provenance with the gate; never substitute a stale fixture
or a semantic creation command. After building the source's dedicated scheme:

```sh
xcrun simctl install "$ONECAD_SIM_UDID" \
  build/ios-air4-public/Build/Products/Acceptance-iphonesimulator/OneCAD.app
xcrun simctl install "$ONECAD_SIM_UDID" \
  "$ONECAD_FIXTURE_DERIVED/Build/Products/Acceptance-iphonesimulator/OneCADFixtureSource.app"
fixture_data="$(xcrun simctl get_app_container "$ONECAD_SIM_UDID" com.onecad.fixture-source data)"
mkdir -p "$fixture_data/Documents"
cp "$ONECAD_MAC_FIXTURE" "$fixture_data/Documents/$ONECAD_PUBLIC_FIXTURE_NAME"
xcrun simctl launch "$ONECAD_SIM_UDID" com.onecad.fixture-source
```

Pass `TEST_RUNNER_ONECAD_PUBLIC_FIXTURE_NAME` to Xcode alongside
`TEST_RUNNER_ONECAD_RUN_ID` and `TEST_RUNNER_ONECAD_MAC_FIXTURE`. It names that
file, for example `MacInput-<run>.onecad`; it is not an alternate opening path.
The test still selects the file through the native picker. Both tests keep
screenshots and attach the actual accessibility tree on failure. Public saved
files live at `Documents/Plate-<run>.onecad` and
`Documents/PromptPlate-<run>.onecad`, outside `acceptance/<run>`.

Targets include Air4 (820×1180 pt portrait) and Pro13 (1032×1376 pt portrait);
source portability does not establish a destination's result. These workflows
retain the actual window orientation. On the Air4 iPadOS26.5 simulator's current
windowing mode, forcing portrait after the native landscape test produced
inconsistent app/keyboard window transforms and offscreen keyboard hit targets.
Removing that test precondition does not qualify production rotation; dedicated
`OrientationTests` retain their separate rotation assertions. The finger refusal probe
uses the actual public viewport and source default-camera projection, refreshed
after layout. XY support uses its stable identifier, not glyph coordinates.

The software keyboard must appear (simulator Connect Hardware Keyboard off).
Each field is focused explicitly; no Tab key is assumed on the smaller keyboard.
The focused virtual field becomes `onecad.keyInputProxy`, a custom UIKit
`UITextInput` exposing Rust's acknowledged draft, queried by identifier across
accessibility roles. Other virtual fields and all keyboard keys remain subject
to hittability checks. The workflow asserts exact acknowledged values, taps
Done and awaits keyboard dismissal. When a locale presents an alphabet layout,
the test taps its public `more` key before a digit. A first-use alternate-letter
tutorial is dismissed only when its exact message appears, using its public
`OK` button; the requested digit must then be hittable.

Controls use stable automation identifiers where provided, then current derived
role/label identifiers or public labels. Sketches are reached by History order,
not Explorer's random sketch-ID order. Canvas picks are absent from these
public control workflows; the synthetic native helpers explicitly distinguish
Pencil authoring from finger control/refusal probes.

Constraint-list searches use at most eight finger drags inside the observed
Edit/Remove content bounds, starting clear of buttons. They require visible row
movement and tap Edit only when the whole row and button lie inside the capped
scroll area. Export checks expand the
Details disclosure belonging to the latest completed Export entry before
asserting the full normalized filename; the Activity summary truncates long paths.

## Simulator-authored geometry artifacts

`ModelArtifactTests` authors three independent models through accessibility
controls and the public touch, key and text ABI. These hosted native tests are
class 3 evidence; confirmed synthetic Pencil samples are not physical input evidence.
`ModelDriver` draws a constrained 60 × 40 mm rectangle and centered Ø6 mm circle
in one sketch, then extrudes 4 mm. Separate cases apply a 1 mm fillet or chamfer
to one 4 mm vertical outer edge. The tests save `plate`, `plate-fillet` and
`plate-chamfer` plus matching semantic records for the independent Mac oracle.
After saving its original baseline, `testPlateArtifact` also picks the top face
with confirmed synthetic Pencil input, previews/cancels Offset Face4 mm,
applies it (exact height8 mm and twice the original volume), then undoes it.
Move / Rotate applies translation(10,5,0) mm and rotation90° about Z through
public controls, preserving mass and requiring world bounds
[-40,15,0]..[20,55,4]. Undo restores the original bounds, volume and history.
These checks preserve the former public operation coverage as explicit native
input evidence; they do not claim public XCUITest Pencil delivery.
Native card fields are tapped only after their complete rectangles fit between
the observed card header and footer, inside the keyboard-safe viewport. Up to
eight confirmed synthetic finger drags scroll the card's label column; each tap
uses freshly settled field bounds and surface generation. Scrolling does not
dismiss the keyboard or submit the operation.

Assertions cover live geometry, exact feature order, one healthy solid,
analytic volume/extents, fully constrained sketches, saved hashes and run/kernel
provenance. Semantic sketch status uses canonical solver names; an unsolved
sketch remains absent rather than being reported as successful. The Mac fixture
round-trip test enters and finishes its sketch through public controls before
recording solver evidence, checking that document, feature and sketch identities
remain unchanged. These test definitions do not assert a successful gate run.

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
| `input_state` | `{"cmd":"input_state","contact","estimationIndex"}` | Read-only `synthetic:true`, `liveContacts`, normalizer `stats`, retained `pose`, current `prediction` and `pencilPosition`; never drains or resets input |
| `run_id` | `{"cmd":"run_id","set"?:id}` | `runId`: `set` replaces the current id; otherwise the one read at launch from `ONECAD_RUN_ID` or `TEST_RUNNER_ONECAD_RUN_ID`; `null` when none |
| `kernel_identity` | `{"cmd":"kernel_identity"}` | `kernelIdentity`: `backend` (`in-process`, or `null` before a document host runs), `embedded` (manifest the binary verifies against), `linked` (manifest of the linked bundle), worker `hello`, `epoch`, `workerState`, `journalWriteFailures`, `relaunchRequired` |
| `document_summary` | `{"cmd":"document_summary"}` | ticket; then `summary` (`tabCount`, `document`: path, title, documentId, dirty, revision, undo/redo depth; `geometrySource`; `features`: id, opType, label, valueText, primaryValue, stored `params` (as a re-edit reads them), status; `bodies`: solids, faces, kernel `volume`, `surfaceArea`, `centroid`, tessellation `min`/`max`; `sketches`: status and `dof` of the last solve in this session, entity and constraint counts; `view`: the active tab's editor state when the command was sent, `tabIndex` (0-based strip position), `selected` (selection size), `cameraOffset`, `cameraTarget`, `status` (status line), `busy`) and `kernelIdentity`. Everything describes the active tab. |
| `save_artifact` | `{"cmd":"save_artifact","name","hold"?}` | ticket; then `saved`, `path`, `clean`, `status`, `provenance`. The result is read from the tab that started the save, whichever tab is shown. `hold: true` keeps the save pending (its dialog unanswered, the editor busy) until `release_save` |
| `release_save` | `{"cmd":"release_save","ticket"}` | `released`: a held save continues; `false` when it was not held or already released |
| `answer_save` | `{"cmd":"answer_save","name"}` | `path`: `Documents/acceptance-exports/<name>`, which answers the next save dialog (Save As or Export) in place of the name prompt; nothing is attributed to the run and the directory is outside every run directory |
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
- `fault save_failure` arms the next `save_artifact` only. A new target:
  a directory takes its name and the editor's Save As fails in the container
  write's rename (`Is a directory`, EISDIR). An existing target (an
  overwrite): its run directory is made read-only for that save, so the
  container write fails creating its temp file (`Permission denied`,
  EACCES) and the previous file must stay byte-identical. The directory's
  previous mode is recorded in Application Support
  (`save-fault.read-only.json`) and restored when the result is read, when
  an unread ticket is dropped, by `fault clear`, by the bridge's destruction
  and at the next launch; only one overwrite fault can be armed at once. The result is `saved:false,
  injectedFailure:true, blocked:"directory"|"readOnly"` with the editor's
  status line, and only when that status names the expected error. A Save the editor refused (another file
  action running) or one that failed elsewhere is an error. `fault journal_write` replaces the kernel journal
  directory by a regular file, so journal writes fail and are counted
  (`journalWriteFailures`); `clear`, the bridge's destruction and the next
  launch restore it without the fault window's `inflight.json`. `clear`
  also releases every held save. Nothing else is injectable.
- XCUITest runs in another process and cannot call hooks: UI tests prove UI
  behavior and screenshots; model artifacts come from native tests.
  `launchOneCAD` passes the run id on as `ONECAD_RUN_ID`.

Swift helpers: `platforms/ios/OneCADNativeTests/SemanticHooks.swift`
(`SemanticHooks.attach`, `call`, `run` with ticket polling, `runID`,
`kernelIdentity`, `documentSummary`, `saveArtifact`, `exportStepArtifact`,
`writeSemantic`, `fault`; `XCTestCase.waitFor` polls on the main run loop).
`SemanticHookTests.swift` uses them for the in-process identity, the Mac
fixture round trip and both faults. `DocumentDriver.swift` drives documents
through public paths only: it activates snapshot nodes by automation id or
label (`onecad_ios_a11y_activate`, as a tap), opens files with
`onecad_ios_open_path`, and reads state with `document_summary`; its
`closeEveryTab` (discarding unsaved work and a leftover recovery offer)
starts and ends each document test, which also removes the files it
created, because the gate rejects run-directory files it did not expect.

## Synthetic Pencil replay

`PencilReplayTests/testSyntheticPencilReplayPreservesInputAndDocumentContracts`
uses the actual canvas generation and the production normalizer/router. It
checks chronological delivery and coalesced deduplication, prediction replacement,
retained force correction without position changes, and public cancellation
followed by a rejected late end. Document summaries must remain unchanged.
`pencil-replay.trace` records requests and observed responses for all four
scenarios; `seal_artifact` binds its bytes to the current run and kernel.
The simulator gate rechecks these observations and requires the sealed trace.
This is synthetic input evidence, not physical Pencil qualification.

## Document tests (IP16 ONECAD-463, IP17 ONECAD-464)

Simulator evidence only; the Mac plate is `ONECAD_MAC_FIXTURE`.

| Test | Class | Proves |
|---|---|---|
| `OneCADNativeTests/DocumentInboxTests/*` (6) | 3 (host only) | The security scope covers the whole coordinated copy into `Documents/Imported` and is released on success, failure and in async batches; an ungranted scope is never stopped |
| `OneCADNativeTests/DocumentPersistenceTests/testFailedOverwriteKeepsThePreviousFileByteIdentical()` | 3 | A Save over an existing file that fails in the container write (`fault save_failure`, read-only) leaves its SHA-256 unchanged, the document dirty, at its revision and bound to that file, with the error in the status line; the next Save replaces it whole |
| `OneCADNativeTests/DocumentPersistenceTests/testMalformedStepFromFilesLeavesTheDocumentUnchanged()` | 3 | A malformed STEP in `Documents/Inbox` opened with `open_path` is refused; the open document keeps identity, history, revision and dirty state, and the status line reports the failure |
| `OneCADNativeTests/DocumentExportTests/testStepExportReachesTheExportPicker()` | 3 | File › Export STEP (name answered by `answer_save`) writes a STEP and presents the real export picker |
| `OneCADNativeTests/DocumentExportTests/testSendACopyOffersOnlyTheSavedDocument()` | 3 | File › Send a Copy presents the export picker for a saved document and refuses one with unsaved changes |
| `OneCADNativeTests/MultiDocumentTests/testThreeDocumentsKeepIndependentHistorySelectionCameraAndDirtyState()` | 3 | Three tabs: a sketch, an undo, a camera turn and a selection in one leave the other two's history, undo depth, dirty state, selection and camera unchanged, and the edited one keeps them across switches |
| `OneCADNativeTests/MultiDocumentTests/testPendingSaveAndImportLandOnlyInTheirOwnDocuments()` | 3 | A Save held pending (`hold`) keeps the plate busy, so a tap on another tab is refused for 10 polls; after it finishes and another tab is shown, its result is still read from, and lands only in, the plate. A tab tapped while a STEP import opens its new tab does not receive the import, which keeps its own tab |
| `OneCADNativeTests/MultiDocumentTests/testATabSwitchCancelsALiveTouchContact()` | 2+3 (synthetic Pencil replay) | Choosing a tab while a stroke is down cancels it (`DOCUMENT_SWITCH`): its prediction is dropped and its lift edits neither document |
| `OneCADUITests/DocumentLifecycleTests/testSavedDocumentReopensFromRecentsInAFreshProcess()` | 4 | New part, sketch, Save through the name prompt, `terminate()`, relaunch, reopen from Home's recents; the History row reads `Sketch` |
| `OneCADUITests/DocumentLifecycleTests/testUnsavedWorkIsOfferedAfterProcessTermination()` | 4 | Unsaved work flushed on backgrounding is offered after `terminate()` and relaunch; Restore brings its History back (process termination, not jetsam) |
| `OneCADUITests/DocumentPickerTests/testCancelledImportPickerLeavesTheDocumentUnchanged()` | 4 | Cancel in the import picker adds no tab, keeps the history and shows `Activity · Cancelled` / `File action cancelled` |
| `OneCADUITests/DocumentPickerTests/testSendACopyOfASavedDocumentOpensTheExportPicker()` | 4 | File › Send a Copy of a document saved through the prompt opens the export picker; the document keeps its file |
| `OneCADUITests/SaveAllTests/testSaveAllAsksForEachUntitledDocumentAndReportsACancel()` | 4 | File › Save All with two dirty untitled tabs asks for each in turn; after a name and a Cancel the first is saved, the second stays untitled and Activity reads `Save All left unsaved: Untitled (cancelled)` |

Not covered here: `WILL_TERMINATE` (XCUITest's `terminate()` kills the
process without it), a live preview pending across a switch (Rust delivery
tests in `onecad-ui` `document_tab_delivery_tests` cover unfinished tools
across a switch on macOS), and the physical-only provider behaviour listed in
[HOST.md](HOST.md#files-and-documents).

Release and Debug archives must not export any `onecad_ios_test_` symbol; the
Xcode build phase refuses them, and `cargo xtask ios-simulator-gate` checks
the Release products.
