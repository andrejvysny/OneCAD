---
name: repo-worker-geometry-lane
description: Composing multi-op worker probes in-process — the direct op-executor lane, the non-standard XY sketch frame, un-oriented descriptor normals, and which upstream feature reorders a box's face map
metadata:
  type: project
---

Facts for writing an in-process C++ probe that chains several kernel ops.

**Why:** the Session + `handle_execute_plan` lane needs a full plan envelope and minted element
ids; the direct executor lane composes the same shipped ops in ~20 lines, and two of the coordinate
conventions below are non-obvious enough to cost a build cycle each.

**How to apply:**

- Direct op-executor lane (`test_wp6_ops.cpp`, `test_hole_op.cpp`, `test_hole_extrude.cpp`): one
  `BodyStore` + one `ElementMapPartition` + a `Ctx{sketches, last_sketch, cancel}` whose `make()`
  returns the `OpContext`; then call `ops::execute_extrude` / `execute_hole` / `execute_chamfer` in
  sequence on the same store. `execute_extrude` NewBody publishes `body_<opId>`; `execute_hole`
  modifies `params.targetBodyId` in place. Sketches ride `Ctx::sketches` as `{sketchId, params}`.
- An `inputs[]` semantic ref whose `elementId` was never minted always takes the descriptor+anchor
  ladder, so `{primary:{bodyId, elementId, kind}, intent:{kind, descriptor}, anchor:{worldPoint}}`
  built from a live sub-shape binds by geometry. The `anchor.worldPoint` must be the element's real
  position — an anchor tens of mm off scores low and the op returns NeedsRepair with an EMPTY
  `error_message` (the status is Ok), which reads like a silent failure if you only print the message.
- `plane:{kind:"XY"}` is NON-STANDARD by design (`worker/src/sketch/Sketch.h:59-69`): sketch u maps
  to world **+Y** and sketch v to world **-X**. A rect authored u∈[0,40], v∈[0,20] extrudes to a box
  at world x∈[-20,0], y∈[0,40]. Author u∈[0,h], v∈[-w,0] to land x∈[0,w], y∈[0,h].
- `ElementMapPartition::describe(shape).normal` is the UN-oriented surface normal — a box's z=0 and
  z=10 faces both report (0,0,1). Outwardness is the separate `outward` field, filled only by the
  two-arg `describe(shape, body)`.
- Measured (OCCT 8.0.1, 2026-09-03) on a 40×20×10 extruded box: the plain box's face map orders the
  -Y wall f:2 before the +Z top f:6; inserting a Ø6 Hole in the **-X wall** (blind or through)
  reorders that pair to top f:4 before front f:5. A hole in the +X wall or the bottom does NOT
  reorder it. That -X hole is the cheapest upstream feature for exercising any face-ordinal rule.
- Need a TANGENT CHAIN in a worker test without arcs in the sketch? Extrude the rect box, then
  `execute_fillet` a VERTICAL corner edge (r=5). The three top edges around that corner (line, arc,
  line) become ONE contour that BOTH `BRepFilletAPI_MakeChamfer::Contour` and `EdgeChainer` agree
  on, so `analyze_edge_contours` accepts it (measured 2026-09-04, OCCT 8.0.1). A v1 record must
  then list ALL THREE in `inputs`/`edgeIds` or `enforce_frozen_closure` refuses closure-drift.
- ctest binaries land in `worker/build/tests/<name>`, not `worker/build/<name>`.
- `session::shape_volume` of an extruded 40x20x10 box is NOT bit-exactly 8000 — compare with a
  tolerance, never `==`.
- `ctx.post_upstream_edit` is a per-PLAN claim, NOT a reliable "geometry may have moved" signal: every
  `ToEnd(0)` lane (open, import, recovery, worker restart) sets it while `RevertToEnd` (undo/redo) and
  `RegenToStep` previews omit it. A correctness gate keyed on it fires on open and VANISHES on the next
  redo. Use it only for the SCHEMA §10 descriptor tie-veto it was built for; never to decide whether a
  record may still use a legacy fallback (WP-F review, 2026-09-04 — the chamfer ordinal rule was deleted
  outright instead).
