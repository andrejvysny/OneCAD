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
| `onecad.save.name` | Native | Name field of the SAVE_DOCUMENT prompt | `RequiredWorkflowTests` |
| `onecad.import.failed` | Native | Alert when an OPEN_DOCUMENT/"Open in" copy into `Documents/Inbox` fails | planned document tests |
| `onecad.keyInputProxy` | Native | Hidden first responder for the keyboard; not an accessibility element | — |
| `onecad.viewport` | Snapshot (egui viewport, role CANVAS) | 3D viewport region | planned |
| `onecad.menu.file` | Snapshot (egui "File" menu button) | In-app File menu | `LaunchTests`, `OrientationTests`, `RequiredWorkflowTests` |
| `onecad.menu.file.new`, `.open`, `.save`, `.saveAs`, `.saveAll`, `.export`, `.importStep` | Snapshot (egui File menu items) | File commands (iPad has no native menu bar for them) | `.save`: `RequiredWorkflowTests` |
| `onecad.menu.edit.undo`, `onecad.menu.edit.redo` | Snapshot (egui Edit items) | Undo/redo | planned |
| `onecad.tab.<n>`, `onecad.tab.new` | Snapshot (egui document tabs, role TAB) | Document tabs, 0-based | planned multi-document tests |
| `onecad.tool.<tool-id>` | Snapshot (egui tool buttons; `<tool-id>` = tool registry id) | Modeling and sketch tools | planned tool tests |
| `onecad.dimension.input` | Snapshot (egui numeric field, role TEXT_FIELD) | Dimension/parameter entry | planned numeric-entry tests |

## Test inventory

| Test identifier | Target | Evidence |
|---|---|---|
| `OneCADNativeTests/TestHookPingTests/testPingThroughTestCommand` | native | class 3 |
| `OneCADUITests/LaunchTests/testLaunchShowsCanvasAndFileMenu` | UI | class 4 |
| `OneCADUITests/OrientationTests/testRotationKeepsAppResponsive` | UI | class 4 |
| `OneCADUITests/RequiredWorkflowTests/testPlateWithHoleEditUndoSaveReopen` | UI | class 4, IP18/IP21 required workflow |

## Required workflow (`RequiredWorkflowTests`)

One test drives the real app (embedded OCCT/PlaneGCS kernel) through public
XCUITest only: element and coordinate taps, the system software keyboard for
egui fields, `typeText` for the native save prompt, `terminate`/`launch`.
Each step keeps a screenshot attachment named after it.

| Step | Actions | Asserted UI state |
|---|---|---|
| `01-new-document` | Start page (discarding any recovery prompt a killed run left), New part | viewport, Sketch tool, no body |
| `02-sketch-on-xy` | Sketch, tap the XY quad below the origin | plane chooser, then Finish; `Sketch · Fully constrained`, `Entities: 0` |
| `03-rectangle-60x40` | Rectangle, tap the origin, Numeric input, `60` Tab `40` Return | `Distance (mm): 60` and `40`, `Fixed`, `Entities: 8`, `Constraints: 7`, fully constrained |
| `04-finish-sketch` | Finish | `1 closed region(s) returned` |
| `05-extrude-4mm` | tap the profile, Extrude, distance `4`, Create | `Solve: 0 degrees of freedom`, `Body 1`, history `Sketch, Extrude` |
| `06-hole-sketch-on-top-face` | Sketch on the top face; Point numeric X `30` Y `20`; Circle snapped to the point, Ø `6` | H/V distances 30/20, `Coincident`, `Diameter (mm): 6`, fully constrained; `2 closed region(s) returned` |
| `07-cut-through` | tap the disk, Extrude, Operation Cut, target Body 1, distance `5`, Flip, Create | `Reverse`; history `Sketch, Extrude, Sketch, Extrude`; `Health: Healthy` |
| `08-edit-width-70` | History row 1 (Sketch), Edit feature, Edit beside `Distance (mm): 60`, `70` Return, Finish | `Distance (mm): 70`, same four history rows, `Health: Healthy` |
| `09-undo-redo` | Undo, Undo, Redo | width back to 60 (read in the sketch, then Discard); three rows after the second Undo, four after Redo |
| `10-save` | File, Save, name `Plate-<run>` in `onecad.save.name`, Save | prompt dismissed, tab titled with the name |
| `11-relaunch-and-reopen` | `terminate`, `launch`, open the document from Recent | four history rows, `Body 1` healthy, width 60 |

Preconditions and conventions:

- iPad Pro 13-inch simulator in portrait (the test sets portrait). Canvas
  taps are sketch millimetres mapped from the viewport centre (the world
  origin of a new document) with `pointsPerMillimetre` measured for that
  device's default camera; another device needs a new measurement.
- The software keyboard must appear (simulator "Connect Hardware Keyboard"
  off). egui fields have no UIKit element with keyboard focus, so `typeText`
  cannot target them; the test taps keyboard keys, including Tab and Done.
- `Plate-<run>` uses the last 8 alphanumerics of `ONECAD_RUN_ID`
  (`TEST_RUNNER_ONECAD_RUN_ID`), else of a UUID, so saves never overwrite.
- Controls are found through `UIRef` (`OneCADUITests/WorkflowSupport.swift`):
  the planned stable id first (`onecad.tool.<id>`, `onecad.param.<name>`,
  `onecad.op.confirm`, `onecad.dimension.input`, ...), then today's derived
  `<role>.<label>` id, then the label. Unnamed egui controls (Operation
  combo, dimension field) are found by their value until they get ids.
- Sketches are reached through History rows (feature order), not the
  explorer, which lists sketches in id order (random per document).
- Fillet, chamfer, offset face and move/rotate (workflow steps 5–6) are not
  in this suite.

## Test-hook commands (`onecad_ios_test.h`)

Acceptance-only, main thread, JSON in and out. The bridge lane owns the full
catalogue; the host tests currently use:

| Command | Request | Expected response |
|---|---|---|
| `ping` | `{"cmd":"ping"}` | `{"ok":true, ...}` |

Responses are freed with `onecad_ios_test_string_free`. Release and Debug
archives must not export any `onecad_ios_test_` symbol; the Xcode build phase
refuses them, and `cargo xtask ios-simulator-gate` checks the shipped archive.
