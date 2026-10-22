---
name: ipad-xcuitest-lane
description: Public XCUITest of the iPad app — typeText vs egui fields, persisted UI state and recovery prompt, a11y id collisions, predicate pitfalls, canvas coordinate mapping
metadata:
  type: project
---

Facts from the iPad public-XCUITest acceptance lane (feat/ipad-accept, 2026-10-22). Verify before relying on them.

- `typeText`/`typeKey` fail on egui fields ("Neither element nor any descendant has keyboard focus"): KeyInputProxy is not an a11y element. Tap software-keyboard keys instead (labels `0-9`, `.`, `delete`; id `tab`, `Done`). Native UIKit fields (`onecad.save.name`) take `typeText`.
- Inspector tab (Properties/History) and other UI prefs persist across launches; a killed/failed run leaves an autosave "Recover unsaved work" prompt (`button.discard-recovery`). Tests must normalize both.
- Derived ids are `<role>.<label>`; same label in several places (toolbar `button.sketch` vs explorer `other.sketch`) → an OR-predicate `firstMatch` picks tree order. Resolve ids by priority, labels last.
- Explorer lists sketches/bodies from `projection.sketches` (BTreeMap keyed by UUID string): order is random per document, and a11y `#n` suffixes follow tree order, not visual order. Reach features via History rows (feature order) + "Edit feature".
- `XCTNSPredicateExpectation` with a block `NSPredicate` and `object: nil` never fulfilled here; wait on element queries instead.
- `egui::WidgetInfo::text_edit(enabled, prev, text, hint)`: 4th arg is a placeholder, not the a11y name; set `info.label`.
- New document default camera: world origin at the viewport a11y frame centre; ~1.285 pt/mm on iPad Pro 13 (M5) portrait sketch view; XY quad hit at origin + (0, 41) pt in the plane chooser.
- Geometry proof without measurements: tap in model view, read Properties title `Face`/`Edge` (+ `on Body 1`) or `Nothing selected`. Edges within 8 pt beat faces (`EDGE_PICK_LOGICAL_PIXELS`), so small holes pick their rim; TOP view + Fit all centres the bbox at the viewport centre (~15 pt/mm for 60x40) for through-hole checks.
- A reopened document keeps an isometric camera, not the new-doc top camera; mm→pt mapping is invalid after reopen or Fit all.
- Plain viewport tap = Replace selection (no toggle); regen/camera have no UI completion signal, so retry tap sequences boundedly.
- `TEST_RUNNER_ONECAD_RUN_ID` in xcodebuild's env reaches the test as `ONECAD_RUN_ID`; native save prompt strips `/` (no subfolders).
- Full required workflow test ≈ 7 min; whole iPadAcceptance plan ≈ 8 min plus build.

**Why:** each cost a failed probe iteration.
**How to apply:** before extending OneCADUITests. See [[ios-xcode-lane]], [[ipad-host-lane]].
