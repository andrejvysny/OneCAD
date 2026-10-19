---
name: real-arc-slot-suite-gotchas
description: real_arc_slot.rs worker-backed suite facts - redo/enter re-solve ulp drift, free-arc radius edit nondeterminism, welds count
metadata:
  type: project
---
Reuses real_point_center/support.rs via #[path]; mods in tests/real_arc_slot/{geom,flows}.rs need explicit #[path] (rustfmt hoists `mod`).
Redo/re-enter re-solve differs ~1e-16 from first solve: compare with json tolerance, not assert_eq (Undo is bit-exact).
Free (unpinned) arc radius edit moves center AND angles, differs run-to-run (PlaneGCS); never assert angles/center on unpinned geometry.
Closed tangent chain has 2 Coincident (shared point entities carry the rest); bare slot 4; DOF slot 5, arc 5, +Diameter/Radius -1.
