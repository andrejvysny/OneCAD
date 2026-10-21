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
| Test plan | `platforms/ios/iPadAcceptance.xctestplan`: one configuration, both targets, default (non-random) order, default attachment policy (automatic UI screenshots kept only for failing tests) |
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
| `onecad.keyInputProxy` | Native | Hidden first responder for the keyboard; not an accessibility element | — |
| `onecad.viewport` | Snapshot (egui viewport, role CANVAS) | 3D viewport region | planned |
| `onecad.menu.file` | Snapshot (egui "File" menu button) | In-app File menu | `LaunchTests`, `OrientationTests` |
| `onecad.menu.file.new`, `.open`, `.save`, `.saveAs`, `.saveAll`, `.export`, `.importStep` | Snapshot (egui File menu items) | File commands (iPad has no native menu bar for them) | planned document tests |
| `onecad.menu.edit.undo`, `onecad.menu.edit.redo` | Snapshot (egui Edit items) | Undo/redo | planned |
| `onecad.tab.<n>`, `onecad.tab.new` | Snapshot (egui document tabs, role TAB) | Document tabs, 0-based | planned multi-document tests |
| `onecad.tool.<tool-id>` | Snapshot (egui tool buttons; `<tool-id>` = tool registry id) | Modeling and sketch tools | planned tool tests |
| `onecad.dimension.input` | Snapshot (egui numeric field, role TEXT_FIELD) | Dimension/parameter entry | planned numeric-entry tests |

## Test-hook commands (`onecad_ios_test.h`)

Acceptance-only, main thread, JSON in and out. The bridge lane owns the full
catalogue; the host tests currently use:

| Command | Request | Expected response |
|---|---|---|
| `ping` | `{"cmd":"ping"}` | `{"ok":true, ...}` |

Responses are freed with `onecad_ios_test_string_free`. Release and Debug
archives must not export any `onecad_ios_test_` symbol; the Xcode build phase
refuses them, and `cargo xtask ios-simulator-gate` checks the shipped archive.
