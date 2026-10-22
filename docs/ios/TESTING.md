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
that pill tools, the multi-select switch, operation fields and footer,
document tabs and the sketch strip actions appear in an egui AccessKit
`TreeUpdate` (`onecad-ui` `automation::tests`, `pill::automation_tests`,
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
| `onecad.save.name` | Native | Name field of the SAVE_DOCUMENT prompt | planned document tests |
| `onecad.import.failed` | Native | Alert when an OPEN_DOCUMENT/"Open in" copy into `Documents/Inbox` fails | planned document tests |
| `onecad.keyInputProxy` | Native | Hidden first responder for the keyboard; not an accessibility element | — |
| `onecad.viewport` | Snapshot (egui viewport, role CANVAS) | 3D viewport region | implemented |
| `onecad.menu.file` | Snapshot (egui "File" menu button) | In-app File menu | `LaunchTests`, `OrientationTests` |
| `onecad.menu.file.new`, `.open`, `.save`, `.saveAs`, `.saveAll`, `.export`, `.importStep` | Snapshot (egui File menu items) | File commands (iPad has no native menu bar for them) | planned document tests |
| `onecad.menu.edit.undo`, `onecad.menu.edit.redo` | Snapshot (egui Edit items) | Undo/redo | planned |
| `onecad.tab.<n>`, `onecad.tab.new` | Snapshot (egui document tabs, role TAB; "+" button) | Document tabs, 0-based in strip order | implemented; planned multi-document tests |
| `onecad.tool.<tool-id>` | Snapshot (egui pill tool buttons; `<tool-id>` = `ToolId::automation_name`: `select`, `sketch`, `line`, `rectangle`, `circle`, `arc`, `slot`, `point`, `dimension`, `constraints`, `extrude`, `fillet`, `chamfer`, `offsetFace`, `moveRotate`, `edit`, `import`, `more`, …) | Tools shown in the pill; rows of the More catalog carry no id | implemented; planned tool tests |
| `onecad.tool.finishSketch`, `onecad.tool.discardSketch` | Snapshot (sketch session strip buttons) | Finish or discard the open sketch | implemented |
| `onecad.dimension.input` | Snapshot (egui numeric field, role TEXT_FIELD) | First declared field of the running operation: fillet radius, chamfer distance, offset distance, extrude distance, translation X | implemented; planned numeric-entry tests |
| `onecad.param.<name>` | Snapshot (egui numeric fields) | Every other field of the running operation: `translationY`, `translationZ`, `rotationAngle`, `extrudeSecondDistance`, … (`automation.rs`) | implemented |
| `onecad.op.confirm`, `onecad.op.cancel` | Snapshot (operation card or History editor buttons) | Create/Apply and Cancel of the running operation | implemented |
| `onecad.plane.xy`, `.xz`, `.yz` | Snapshot (sketch support chooser buttons) | Sketch on an origin plane | implemented |
| `onecad.history.<index>` | Snapshot (Inspector History rows) | Feature row, 0-based in history order | implemented |
| `onecad.select.additive` | Snapshot (pill multi-select switch; toggled sets `A11Y_SELECTED`) | Plain viewport picks toggle items instead of replacing the selection | implemented |

## Test-hook commands (`onecad_ios_test.h`)

Acceptance-only, main thread, JSON in and out. The bridge lane owns the full
catalogue; the host tests currently use:

| Command | Request | Expected response |
|---|---|---|
| `ping` | `{"cmd":"ping"}` | `{"ok":true, ...}` |

Responses are freed with `onecad_ios_test_string_free`. Release and Debug
archives must not export any `onecad_ios_test_` symbol; the Xcode build phase
refuses them, and `cargo xtask ios-simulator-gate` checks the shipped archive.
