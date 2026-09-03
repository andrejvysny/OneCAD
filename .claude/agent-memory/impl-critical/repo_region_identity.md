---
name: repo-region-identity
description: How OneCAD sketch-region ids are minted and matched, and the shipped-kernel facts a WP-B probe measured (V3 hashing, curve-pair ceiling, arc sampling, solver verdicts)
metadata:
  type: project
---

Sketch-region identity and detection facts measured on the shipped kernel at `78daecb`.

**Why:** these are the numbers a probe or fix in this area has to start from; each one was
measured, not inferred, and each is easy to get wrong from reading the code alone.

**How to apply:**

- **Region ids are GEOMETRY hashes, not anchors.** `RegionIdentityVersion::V3` (`r_<16hex>`)
  hashes analytic provenance plus normalized parameter intervals. Changing a bounding curve's
  RADIUS remints every id in the sketch, so a stored `SketchRegionRef` stops matching and the
  step fails `profile: regionId '<id>' matched no selectable region (available: [...])`. The
  regen still reports `outcome=published`; the loss shows up only in `RegenReport::failed_steps`.
- **Two different configs, one function.** `SolverLane::on_regions` (publication) uses V3 +
  `V3PhysicalProximity`; `ops::build_profile_face` derives its config from the ref's
  `regionIdentityVersion` and OVERRIDES `exactAnalyticFragments = (v == 2 || v == 3)` — so a
  ref with NO version silently gets the legacy tessellated detector. Bind an extrude with
  `region_identity_version: Some(finished.region_identity_version)` or the ids will not match.
- **`buildRegionTable` does NOT guarantee disjoint cells.** It enforces id uniqueness only.
  Cell disjointness comes from `LoopDetector::findFaces`, and hole parenting is a tessellation
  containment test that CAN fail (that is exactly probe P-B2). Never write a comment or an
  algorithm that assumes "the table forbids overlapping cells" — if two cells could contain the
  same point, refuse rather than rank them.
- **Curve-pair ceiling.** `LoopDetectorConfig::maxPlanarizedCurvePairs = 4096` is compared
  against the pairs that SURVIVE a bbox broad phase (2026-09-03), not C(sources, 2). Boxes are
  conservative supersets — exact for a line, the FULL circle box for an arc, the major-radius
  box for an ellipse — grown by the intersection tolerance on both sides, so the cull cannot
  drop a real crossing. A closed 100-line profile went 4950 enumerated -> 100 surviving.
  Before the cull it refused with `profile refinement exceeds curve-pair limit`.
- **Fragment sampling is chord-tolerance based** (2026-09-03, was a fixed 16 segments):
  `n = clamp(ceil(sweep / (2*acos(1 - 0.01mm/r))), 16, 1024)`, so the polygon never dips more
  than 0.01 mm inside the curve. R=100/240° -> 149 segments (was 16, sagitta 0.856 mm);
  R=100/120° -> 75; r=8/180° -> 32; r=1/360° -> 23. Containment is still decided against the
  polygon, so a near-rim fixture must sit at a segment MIDPOINT (a sample vertex sees zero
  sagitta and passes at any clearance). Sampling does NOT feed region identity — ids stayed
  byte-identical across the change; only a cell that genuinely GAINED a hole was reminted.
- **A closed curve with no splits never reaches that code.** It goes to
  `AdjacencyGraph::closedCurves` with EMPTY `fragment.samples` and is tessellated later by
  `computeLoopProperties` (`minCircleSegments = 32`). Only SPLIT fragments use the chord count.
- **A degenerate entity is dropped, not fatal** (2026-09-03): a line shorter than, or a
  circle/arc with radius under, `kAuthoringResolutionMm` (1e-3) becomes a
  `loop::DetectionWarning` and the rest of the sketch still detects. It used to refuse the
  whole graph under V3 with `profile refinement could not preserve analytic provenance`.
  `RegionTable::warnings` remaps each `entityId` through `mapBaseEdge` into the WIRE id space;
  `build_profile_face` then emits the §7.2 warning `SKETCH_ENTITY_DEGENERATE`. `SketchRegions`
  has no diagnostics channel, so that verb stays silent about it. Ellipses are NOT covered.
- **The solver is honest about a flat contradiction.** Two `Distance` constraints on the same
  point pair (10 and 20) makes DogLeg fail, LM fail, and `ConstraintSolver::solve` return
  `status=Diverged, success=false, conflicting=2`, with `undoSolution()` restoring the
  pre-solve positions. `SolverResult::residual` is NEVER populated — read residuals off the
  constraints with `getError(sketch)` instead.
- Exact volumes come from `ElementQuery::query_mass_properties` (SCHEMA §7.5), not the mesh:
  a prism over a rect-minus-half-disc matched its analytic value to 9.1e-13 absolute.
