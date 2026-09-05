---
name: repo-worker-geometry-lane
description: Composing worker probes in-process — the direct op-executor lane, the Session/mint lane, the non-standard XY sketch frame, un-oriented descriptor normals, and which upstream feature reorders a box's face map
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

- SESSION lane (needed when the probe must go through the REAL mint or a wire verb handler;
  pattern `test_element_identity_gate.cpp`): `session.open(...)` → `fence_and_clone(jobId, rev,
  epoch, session::kEmptyPrefixHash)` (that constant lives in `session/HistoryHash.h`, NOT
  `Session.h`) → fill a `ScratchJob` from the fence → run the op executor against
  `job.bodies`/`job.partition` so publish-time per-body state rides along → `store_prepared(std::
  move(job))` → `accept_prepared`. Then call `handle_bind_element_ids` / `handle_query_element`
  directly with `Envelope::request(...)`. Name the bind helper anything but `bind` — ADL picks up
  `std::bind` and the call fails to convert to `Envelope`.
- `AcquireElementIds` mints NOTHING (`ElementIdentity.cpp:136-163` returns an empty `elementId`;
  Rust mints it). The mint that installs a durable binding is `Session::bind_element_ids` →
  `stage_binding`. A refusal wired only into `AcquireElementIds` is ADVISORY — `BindElementIds` on
  the same topoKey still succeeds. (Fixed for gear faces by WP-I 2026-09-04: `stage_binding` now
  consults `Session::gear_bodies_` and refuses a tooth face `REF_UNRESOLVED` /
  `GEAR_FACE_NOT_REFERENCEABLE`; Acquire stays a pass-through by design.)
- `session::classify_shape(shape)` (`session/ClassifyElement.h`) runs the ClassifyElement
  classification on a bare shape in-process. Cheapest way to locate a face by GEOMETRY instead of
  a topokey ordinal in a probe: `{surfaceType, frame:{origin,normal}}` for a plane,
  `{origin,axis,radius}` for a cylinder; a gear's involute flanks come back `surfaceType:"other"`.
  Its plane normal FOLDS `face.Orientation()`, so a box's top face reads +Z and its bottom -Z —
  unlike `ElementMapPartition::describe`, which is un-oriented.
- A `Gear` op with `axleHole:true` + `axleHoleDiameter` gives a bore face to reference; m=2 z=20
  gives root radius 17.5 and bore radius 5, so `radius < rootRadius` separates bore from tip.
- To test the PLAN-STEP ref rung in-process (the `CandidateFilter` / tracked-entry lane), call
  `session::execute_candidate_op(job, op, opId, lastSketch, cancel)` on a hand-filled `ScratchJob`
  — no Envelope, no HandlerContext. Assert a successful bind on `result.delta.added`, NOT on the
  surviving partition: a Fillet CONSUMES the edge it rounds, so the id it just minted appears in
  `delta.removed` in the same step.
- SCHEMA §7.3 gear referenceability covers EDGES and VERTICES too: one is referenceable iff EVERY
  adjacent face is (`ops::gear_element_referenceable`; reuse one `ops::GearAdjacency` per pool —
  building it per candidate is O(candidates x body size)). Bind evidence carries `surfaceType` for a
  face and `kind` for an edge/vertex. The bore threshold is `radius < rootRadius - 1e-3` (one
  authoring resolution) — an exact compare would decide a root-land cylinder by float luck.
- `ScratchJob` carries `plan` (`{"ops":[...]}`, set by `handle_execute_plan`) and `gear_bodies`
  (the head's map, fence-cloned). SCHEMA §7.3 gear referenceability is PLAN-derived, so a probe
  that fills a `ScratchJob` by hand must set `job.plan` or no body is treated as a gear body.
- `session::classify_shape_in_body(shape, body)` is the ONE producer of §7.5 `frame.sidedness`
  (`pin`/`hole`) — it needs the face's instance in its body; the bare `classify_shape` omits the key.
- `elementmap::resolve_descriptor_stage` takes an optional trailing `CandidateFilter`
  (`bool(const TopoDS_Shape&)`) applied while the pool is ENUMERATED, so a rejected sub-shape is
  neither scored nor reported. Excluding faces does NOT renumber TopoKeys (they stay body ordinals).
- `build_gear_solid` bounds (SCHEMA §7.3): teeth ∈ [3,400], sampleCount ∈ [2,256], height ≤ 1000 mm.
  Any harness that sweeps a gear by scale must clamp height (`test_micro_topology_census` does).
- Rust side: `ElementInfoDto::normal` (`query_element_by_topo_key`) is the UN-oriented normal, so
  both caps of an extrusion report `(0,0,1)` — discriminate by `center[2]`. `center` is the bbox
  centre, which for a PLANAR face lies on the plane, so it is a usable ladder anchor.
