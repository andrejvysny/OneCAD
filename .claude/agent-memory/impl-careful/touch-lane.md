---
name: touch-lane
description: onecad-ui sketch-canvas/overlay traps and iPad touch routing facts from the wave-2 touch lane (status overwrite, hover-gated viewport, occluded test pumps, primitive op gate)
metadata:
  type: project
---

Facts from feat/ipad-touch (2026-10-22). Verify against code before relying on them.

- `app.doc.status` is recomputed every logic tick from `SketchStatus` while a sketch is open: a UI-set message vanishes next tick. Route refusals through the interaction (`SketchStatus::Refused`), e.g. `SketchAction::CommitStroke { stroke: Recognition }`.
- `sketch_ui::viewport` runs only while the pointer hovers the bare canvas (viewport.rs gate): an overlay drawn there disappears exactly when the user presses it. Draw such bars from the always-run sketch branch (see `sketch_pen_stroke::bar`).
- A resolved sketch undo clears `accepted_capture`; dispatch `SketchAction::AcceptView` before committing outside `sketch_ui::viewport`, or `valid_point` silently drops the action.
- `onecad-app` `validate_primitive_ops` accepts only AddEntity/AddConstraint in an Upsert; a remove+add "replace" is refused by the worker path. Stroke override = sketch undo, then commit.
- Delivery-test `pump_until`/`count()` run occluded logic only (no UI pass); work the UI does on a frame needs `host.ui(vec![])` (count() does one at its end).
- AccessKit labels in delivery tests are not unique (pill tool labels repeat in palettes/bars); disambiguate by rect.
- iOS touch routing: `onecad_ui::host::over_control` (canvas rect+layer captured per pass), `Editor::drag_authoring`; orbit = Shift trackpad `MouseWheel` (navigation.rs latch).
- egui has one pointer: a control tap while another contact holds it is `RoutedEvent::Activate` -> iOS `a11y::Tree::click_target` -> `AccessKitActionRequest(Click)` (egui sets FAKE_PRIMARY_CLICKED regardless of pointer state).
- `arc_slot_delivery_tests::tangent_chain_keyboard_drag_seed_cancel_and_numeric_radius` flaked once (Tab numeric editor not open) in the full `--ignored` run; passed alone 3/3 and in 2 more full runs.
- A PostToolUse hook reformats Rust imports in non-2024 order after Write; run `cargo fmt -p <crate>` before clippy/commit.
- In Bash, never end a command with `cat > /dev/null` (waits on stdin, times out).

**Why:** each cost a failed test iteration in this lane.
**How to apply:** before adding canvas overlays, sketch-side refusals or delivery tests in onecad-ui. See [[ipad-host-lane]], [[frontend-gotchas]].
