---
name: extrude-handle-viewport-drag
description: extrude_handle.rs conventions: gate, sign convention, re-exports from gizmo, flaky ignored test
metadata:
  type: project
---

Viewport extrude arrow lives in crates/onecad-ui/src/extrude_handle.rs; reuses gizmo::{along,project,arrow} (pub(crate) re-exports), transform_ui::snap_step, extrude_ui::params. Positive distance = +plane.normal (worker ExtrudeOp.cpp:987-988,1215). Profile.uv+triangles give exact area centroid (vertices are fill-triangle verts, not ordered polygon). `arc_slot_delivery_tests::tangent_chain_*` ignored test is flaky once in full-suite run (passes alone/rerun). `cargo fmt --all --check` can fail from other agents' onecad-render edits; use `cargo fmt -p onecad-ui --check`.
