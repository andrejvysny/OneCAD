# sketch-host-face-transport
date: 2026-09-15
mode: derive
model: gpt-6-astra
effort: xhigh
access: grounded: worker/src/ops/ComponentOp.cpp, worker/src/session/FaceProjection.cpp, worker/src/session/PlanExecutor.cpp, worker/src/session/ElementIdentity.cpp, worker/src/elementmap/Ladder.cpp, worker/src/ops/ExtrudeOp.cpp, worker/src/kernel/validation/GeometryPrecision.h, src-tauri/crates/onecad-core/src/sketch/plane.rs, src-tauri/crates/onecad-core/src/sketch/mod.rs, protocol/SCHEMA.md. Transcript audit: Astra also read worker/tests/{test_component_mate_reseat,test_wp6_ladder,test_cross_body_element_ref,test_feature_pattern}.cpp, worker/tests/fixtures/executeplan_shell.ndjson, src-tauri/tests/{component_ops,topology_rebind}.rs, protocol/fixtures/ and the legacy oracle Sketch.h under corpus (recorded, not re-run; none is corpus/expected-values and none carries the answer to this derivation)
packet: sha256 e401dc316c07
calls: 1 of ~3 for WP-1 (session 01a0a3ea-9d99-7863-8ff6-8087e5e2bc86 recorded for followup)
verdict: n/a (derive)
verified by: Fable recomputed G4 (d = 20·sin10° = 3.472963553 mm; o1 = (103.4202, 30, 19.3969); y1 = n1 × Y = (sin10°, 0, cos10°)), G8 (y1 = (0,0,−1) × (0,1,0) = (1,0,0)), the 0.01° = 1.7453e-4 rad conversion, G10 volume 100·60·25 = 150 000 mm³, G11 deltas 6000·t, and the "no deadband" argument (a 0.0005 mm shift under a 1e-3 deadband would be left behind). OCCT fact added by Fable for the unknown "volume measurement error bound": `BRepGProp::VolumeProperties(shape, props, Eps)` RETURNS the achieved relative error, so U = err_before·V_before + err_after·V_after is measurable per call (to be pinned by the worker test).
rejected: none. Decisions taken on Astra's open questions: (1) PLANE RE-SEATING is the accepted semantics (rigid following needs transform evidence the sketch does not carry; G9 spin ⇒ F1 = F0 by definition); (2) `frameTransportVersion` is carried as an optional Sketch param (additive) rather than in policyVersions; (3) the diagnostic code family is `SKETCH_HOST_RESEATED` / `SKETCH_HOST_NORMAL_REVERSED` (info), repair reasons `sketchHostNonPlanar` / `sketchSeatOffFace` / `sketchFrameIllConditioned` / `sketchFrameInvalid` / `sketchSeatUnmeasurable`, and the Cut family is `CUT_NO_EFFECT` / `CUT_VOLUME_INCREASED` / `CUT_VOLUME_UNMEASURABLE` on EVERY subtracting tail (Extrude, Revolve, Boolean) per plan amendment A-3; (4) the strict host-resolution gate (tracked rung, or AutoBind with score ≥ 0.85 AND margin ≥ 0.10 — Ladder.cpp:342 admits anchor-decided binds below margin) is adopted for the sketch host.

## 1. Problem restatement in your own terms; surface any misframing in one line.

**[Inference]** Define a deterministic **plane re-seating** policy and a Cut predicate that requires demonstrable material removal.

**Misframing:** a resolved plane does not determine tangential translation or rotation about its normal; its `gp_Pln` origin is not necessarily an attachment point inside the face.

Labels below: **[Proved]** mathematics; **[Numeric]** shown computation; **[Source]** cited evidence; **[Inference]** proposed policy or conditional conclusion.

**[Source]** Two implementation gaps matter:

- Sketch attachment currently stays in Rust; it does not cross the Sketch wire. [Sketch schema](/Users/andrejvysny/workspace/OneCAD/protocol/SCHEMA.md:1172)
- The shipped ladder permits some anchor-decided bindings below margin `0.10`. The requested stricter invariant therefore requires an explicit gate; accepting every `AutoBind` is insufficient. [Ladder.cpp](/Users/andrejvysny/workspace/OneCAD/worker/src/elementmap/Ladder.cpp:342)

## 2. Properties to guarantee: the invariants the policy must hold, each as a checkable statement.

- **[Proved] Plane membership:** `(o1 − p1)·n1 = 0`.
- **[Proved] Local geometry:** every stored `(u,v)` remains unchanged. With orthonormal frames, local distances and angles remain unchanged.
- **[Proved] Pure normal translation:** if `n1 = n0` and the plane displacement is `d`, then `F1 = (o0 + d n0, x0, y0, n0)`, including arbitrarily small nonzero `d`.
- **[Proved] Continuity:** the formula below is continuous wherever its projected X axis is nonzero. A Gram–Schmidt fallback cannot extend it continuously through every singular orientation; those orientations require repair.
- **[Inference] Never silently move:** every applied frame change carries its derived writeback and an info diagnostic. Positive reporting epsilons must not suppress transport.
- **[Inference] Failure:** retain the sketch and its local geometry, display `F0`, emit `NeedsRepair`, and stop dependent execution.
- **[Inference] Determinism:** use immutable `F0`, the same resolved input, fixed numerical policy, and fixed serialization. Never transport from the previously published frame.
- **[Inference] Same face:** identity means accepted lineage or accepted semantic resolution—not similarity of normals. A world-normal reversal alone neither destroys identity nor determines the face’s complete rigid motion.

**[Source]** The authored basis must be carried forward: Rust deliberately preserves even its non-standard named-plane bases. [plane.rs](/Users/andrejvysny/workspace/OneCAD/src-tauri/crates/onecad-core/src/sketch/plane.rs:35)

## 3. Predicates and epsilon derivation

### a. Frame transport

**[Inference]** Adopt **projected-origin, projected-X transport** as the explicit plane attachment convention.

**[Proved]** For valid unit `n1`, let:

```text
d  = (p1 − o0) · n1
oc = o0 + d n1

qx = x0 − (x0 · n1)n1
h  = ||qx||
xc = qx / h
yc = n1 × xc
Fc = (oc, xc, yc, n1)
```

`oc` is the nearest point on the resolved plane to `o0`. For `h > 0`, `xc` is the unit tangent maximizing `xc·x0`. Thus the convention preserves the old X direction as far as the new plane permits.

**[Proved]** Replacing `p1` with any other point on the same plane leaves `d` unchanged. Tangential movement of the kernel’s plane origin therefore cannot drag the sketch.

**[Inference] Numerical guard:** propose

```text
τGS = sqrt(DBL_EPSILON) = 1.4901161193847656e−8
```

for IEEE binary64. This is dimensionless, separating numerical conditioning from modelling tolerance. Normalizing `qx` amplifies perturbations approximately by `1/h`; this guard is **not** a measured bound on OCCT normal uncertainty.

**[Proved] Gram–Schmidt fallback when `h ≤ τGS`:**

```text
qy = y0 − (y0 · n1)n1
yf = normalize(qy)
xf = yf × n1
```

For an orthonormal `F0`, `||qy||² ≥ 1 − h²`, so the fallback is well-conditioned.

**[Inference]** Compute this fallback as repair evidence, but **do not auto-publish it** under the requested continuity guarantee. Return `sketchFrameIllConditioned`, retaining `F0`. Section 7 shows the unavoidable jump.

**[Inference]** When `n1` equals `n0`, carry `x0/y0/n0` verbatim; do not rebuild them using Rust’s world-axis seed.

### b. Movement and reporting epsilons

**[Proved]** A positive transport deadband contradicts exact translation: displacement `0.0005 mm ≤ 0.001 mm` would incorrectly leave the sketch behind.

**[Inference]** Separate two predicates:

```text
moved = Fc differs from F0 in any frame component

significant =
    ||oc − o0|| > 1e−3 mm
    OR θframe > π/18000 rad
```

Publish the complete `Fc` and an info diagnostic whenever `moved`; `significant` is diagnostic evidence only.

**[Numeric]** `0.01° × π/180 = 0.00017453292519943294 rad`.

**[Proved]** Measure the **whole frame**, not just its normal. With `Bi = [xi yi ni]`, `Q = Bc B0ᵀ`:

```text
θframe = atan2(
    ||(Q32−Q23, Q13−Q31, Q21−Q12)|| / 2,
    (trace(Q)−1) / 2
)
```

**[Inference]** Keep the mate epsilons for significance; they need no size scaling because they no longer control geometric accuracy. A point at radius `R` can move by `2R sin(θframe/2)`, so a constant angular epsilon is unsuitable as a positional accuracy guarantee.

**[Source]** Mate translation/rotation thresholds and their original rationale are explicit in [ComponentOp.cpp](/Users/andrejvysny/workspace/OneCAD/worker/src/ops/ComponentOp.cpp:165).

### c. Planarity

**[Inference]**

```text
face.ShapeType() == FACE
AND adaptor.GetType() == GeomAbs_Plane
AND plane/frame values are finite
AND orientation is FORWARD or REVERSED
```

Use the plane normal, reversed for `REVERSED`. Reject other surface types; do not fit an approximately planar cylinder or spline.

**[Proved]** This is a topology/surface-type predicate, with no distance epsilon. The creation-time `1e−3 mm` coplanarity distance cannot serve as an angular or planarity test.

### d. “Origin still on the face”

**[Proved]** `Classify(face, oc) == OUT` does **not** imply broken attachment: an unchanged annular face can have its plane origin inside its central hole.

**[Inference]** Therefore, **do not make origin OUT a repair condition** under the supplied definition of `F0`.

Use the frozen anchor as the actual attachment witness:

```text
ua = (anchor0 − o0) · x0
va = (anchor0 − o0) · y0
anchor1 = oc + ua xc + va yc
```

Classify `anchor1` at `τface = BRep_Tool::Tolerance(face)`:

- `IN` or `ON`: accept.
- `OUT`: `sketchSeatOffFace`.
- `UNKNOWN`, exception, or unusable anchor: repair; do not claim successful seating.

Evaluate this **before** movement reporting, including unchanged-plane cases.

**[Source]** This borrows the mate’s face-owned classification tolerance. The mate implementation only refuses explicit `OUT`; refusing unmeasurable classification is a proposed tightening. [ComponentOp.cpp](/Users/andrejvysny/workspace/OneCAD/worker/src/ops/ComponentOp.cpp:501)

### e. Cut no-effect predicate

**[Source]** The documented v1 authoring resolution is absolute:

```text
a = authoring_resolution() = 1e−3 mm
vmin = a³ = 1e−9 mm³
```

It intentionally does not scale with body size. [GeometryPrecision.h](/Users/andrejvysny/workspace/OneCAD/worker/src/kernel/validation/GeometryPrecision.h:113)

**[Inference]** Separate this semantic floor from measurement uncertainty:

```text
Dhat = V_before_measured − V_after_measured
U    = E_before + E_after + E_subtraction
εV   = a³ + U
```

where `|Dhat − Dtrue| ≤ U`.

```text
|Dhat| ≤ εV  → refuse: no demonstrable material removal
Dhat > εV    → passes the material-removal predicate
Dhat < −εV   → refuse: Cut increased volume
unbounded/nonfinite measurement → refuse: measurement unavailable
```

**[Proved]** Passing implies `Dtrue ≥ Dhat − U > a³`. A true zero-volume imprint always refuses because `|Dhat| ≤ U ≤ εV`.

**[Inference] Size law:** if body-centred volume measurements have established dimensionless error bounds `ηi`, use `Ei = ηi Li³`, with `Li` the body diagonal. Otherwise the bound must also account for coordinate magnitude and integration conditioning. **Neither `ηi` nor a certified interpretation of the optional integration epsilon is supplied. A production numerical `εV` cannot be fixed from this packet.**

**[Numeric]** Reject the tempting semantic rule `εV = a L²`: at `L = 1000 mm`, it gives `1000 mm³`, wrongly refusing G12’s `5 mm³`.

**[Proved] Similarity and units:**

| Predicate | Physical scaling by `s` | Numerical unit conversion by `k` |
|---|---|---|
| Transport | Origins/displacements multiply by `s`; axes unchanged | Lengths multiply by `k` |
| Significant movement | Absolute length threshold unchanged; angular threshold unchanged | Length threshold multiplies by `k`; convert angular units together |
| `τGS`, frame validity | Dimensionless; unchanged | Unchanged |
| Planarity/type | Unchanged | Unchanged |
| Seat classification | Invariant if face tolerance scales with geometry | Face tolerance multiplies by `k` |
| Volume | `D` and relative error terms scale as `s³`; fixed product floor `a³` does not | `D`, `U`, and `a³` all multiply by `k³` |

## 4. Degeneracy classes

**[Inference]** Proposed branches; formulas are mathematical detection predicates.

| Class | Detection | Action |
|---|---|---|
| Normal reversed, G8 | `n0·n1 < 0`; near-antiparallel when `||n0+n1|| ≤ τGS` | Follow resolved outward normal under the stated convention; report reversal. Never silently re-sign it toward `n0`. |
| Spin about normal, G9 | **Undetectable from plane inputs** when plane unchanged | `F1 = F0`; rigid spin-following requires additional tangent/transform evidence. |
| Tilt, G4 | `atan2(||n0×n1||, n0·n1) > 0` | Generic transport if well-conditioned. |
| Split/tie, G5 | Multiple successors fail score/margin or descriptor-tie gate | `ambiguous`; preserve all candidate evidence. |
| Consumed, G6 | Authoritative deletion without successor | Terminal resolution refusal; never descriptor-rebind to a replacement. |
| No candidate | Candidate set empty | `no-candidates`. This alone does not establish historical consumption. |
| Non-planar, G7 | Surface type differs from `GeomAbs_Plane` | `sketchHostNonPlanar`. |
| Origin outside | Classifier of `oc` returns `OUT` | Advisory only; classify transported anchor for attachment validity. |
| Attachment witness outside | Classifier of `anchor1` returns `OUT` | `sketchSeatOffFace`. |
| X nearly parallel to normal | `h ≤ τGS` | `sketchFrameIllConditioned`; fallback is a repair proposal. |
| Legacy basis | Basis differs from today’s seed convention but remains orthonormal/right-handed | Preserve it. |
| Invalid frame | Nonfinite, non-unit, nonorthogonal, or `x×y ≠ n` beyond the declared numerical guard | `sketchFrameInvalid`; do not silently “correct” stored geometry. |
| Classification unmeasurable | `UNKNOWN`/exception | `sketchSeatUnmeasurable`. |

**[Source]** Post-edit descriptor separation below `0.02` refuses even if the stale anchor lies exactly on a candidate. [Resolution policy](/Users/andrejvysny/workspace/OneCAD/protocol/SCHEMA.md:5078)

## 5. Algorithm sketch

**[Inference]**

Inputs: immutable `F0`, optional typed host reference/anchor, current scratch body and identity state, edit/replay context, tolerance policy.

Outputs: effective frame, `moved`, `significant`, diagnostics, optional `NeedsRepair`, optional derived writeback.

1. No host reference: retain existing frozen path verbatim.
2. Resolve against the body **at this Sketch step**. Honor consumption, ownership, topology kind, ambiguity, and strict scored-binding gates.
3. Validate `F0`; extract the exact planar surface and orientation-corrected normal.
4. Compute candidate origin and basis; stop on conditioning failure.
5. Classify the transported attachment witness.
6. On success, materialize sketch params using the complete candidate plane. Emit writeback plus info whenever changed.
7. On repair, retain/display `F0`, emit repair state, stop dependents.

**[Source]** The current Sketch step merely stores raw params. Ordinary `NeedsRepair` stops the plan; the successful-step repair exception is specifically for component mates. [PlanExecutor.cpp](/Users/andrejvysny/workspace/OneCAD/worker/src/session/PlanExecutor.cpp:429), [schema](/Users/andrejvysny/workspace/OneCAD/protocol/SCHEMA.md:985)

**[Inference] Complexity:** one resolution, constant frame arithmetic, one face-classification query. Resolution/classification costs depend on candidate count and face complexity. Each Cut adds two volume measurements.

**[Proved]** A feature executed **after** this Sketch step cannot affect its same-step host input. G5–G7 apply here only when their changed topology is already upstream.

## 6. Test vectors

### Frames

**[Numeric]** For the supplied top normal, Rust authors:

```text
x0 = (0,1,0), y0 = (−1,0,0), n0 = (0,0,1)
```

The packet does not supply the actual plane-origin X/Y coordinates. Therefore the exact G1/G2 answers are:

```text
G1: ((o0x,o0y,25), x0,y0,n0); d = 25−40 = −15 mm
G2: ((o0x,o0y,60), x0,y0,n0); d = 60−40 = +20 mm
```

**[Numeric]** The following are **explicit additional test inputs**, not assumed measurements of G4/G8/G9. All require accepted identity and an inside/on-face attachment witness.

| Vector | Fully specified input | Expected frame |
|---|---|---|
| G1 instance | `o0=(0,0,40)`, current plane `z=25` | `((0,0,25), (0,1,0), (−1,0,0), (0,0,1))` |
| G2 instance | Same `F0`, plane `z=60` | `((0,0,60), (0,1,0), (−1,0,0), (0,0,1))` |
| G4 instance | `o0=(100,30,20)`, `x0=Y`, `y0=Z`, `n0=X`; `p1=(100,0,0)`, `n1=(cos10°,0,−sin10°)` | `o1=(103.420201433257,30,19.396926207859)`; `x1=(0,1,0)`; `y1=(0.173648177667,0,0.984807753012)`; `n1=(0.984807753012,0,−0.173648177667)` |
| G8 instance | Top `F0` at `(0,0,40)`; same plane, `n1=−Z` | `((0,0,40), (0,1,0), (1,0,0), (0,0,−1))` |
| G9 instance | Top `F0`; rotate face `90°` about Z through origin | `F1=F0`: the oriented plane is unchanged |

**[Numeric]** G4 computation: `d=20 sin10°=3.472963553339 mm`; displacement is `(20 sin10° cos10°,0,−20 sin²10°)`.

### Degeneracies and thresholds

| Test | Computation / expected result |
|---|---|
| **[Numeric] G5 symmetric tie** | Scores `0.91,0.91`; margin `0` → `ambiguous`. |
| **[Inference] G6** | No candidate → `no-candidates`; known consumption additionally forbids fallback. |
| **[Inference] G7** | Resolved cylinder → `sketchHostNonPlanar`. |
| **[Numeric] G3** | Same oriented plane → `F0`; changed boundary still requires witness classification. |
| **[Numeric] Origin outside** | Annulus radii `5,10 mm`, origin at centre, anchor radius `7 mm`: origin OUT, anchor IN; accept when tolerance is below the `2 mm` boundary clearance. |
| **[Numeric] Seat outside** | Boundary shrinks from `[-3,3]²` to `[-1,1]² mm`; anchor `(2,0)` is `1 mm` outside → repair when face tolerance is below `1 mm`. |
| **[Numeric] Exact parallel** | Top basis, `n1=Y`: `h=0`; fallback `x=−Z,y=−X,n=Y`; published frame remains `F0` with repair. |
| **[Numeric] Conditioning boundary** | `n1=(0,√(1−b²),b)` gives `h=|b|`. `b=τGS/2` repairs; `b=2τGS` uses generic transport. |
| **[Numeric] Legacy valid basis** | `x=X,y=Y,n=Z`, translation `+1 mm`: preserve those axes exactly. |
| **[Numeric] Invalid handedness** | `x=Y,y=X,n=Z`; `(x×y)·n=−1` → invalid-frame repair. |
| **[Numeric] Translation significance** | `0.000999,0.001,0.001001 mm`: all transported; significance `false,false,true`. |
| **[Numeric] Rotation significance** | `0.009999°,0.01°,0.010001°`: all transported; significance `false,false,true`. |
| **[Inference] Unknown classifier state** | Repair, irrespective of zero movement. |

### Cut volumes

**[Numeric] Analytical oracle:** with exact measurements, `U=0`, hence `εV=10⁻⁹ mm³`.

| Case | Volume computation | Effect predicate |
|---|---|---|
| G10 | `Vbefore=Vafter=100×60×25=150000 mm³`; `D=0` | Refuse |
| G11, `t=10⁻⁴ mm` | `D=6000×10⁻⁴=0.6 mm³` | Pass |
| G11, `t=10⁻³ mm` | `D=6000×10⁻³=6 mm³` | Pass |
| G11, `t=10⁻² mm` | `D=6000×10⁻²=60 mm³` | Pass |
| G12 | `D=10×10×0.05=5 mm³` | Pass |

**[Proved]** These are volume-effect decisions, not guarantees that separate topology/tolerance audits accept a sliver.

**[Numeric]** G11 crossover:

```text
t* = εV / 6000
   = 1.6666666666666667e−13 mm   when U=0
```

At `t=t*`, refuse; above it, pass for exact measurements.

**[Proved]** With error bound `U`, guaranteed acceptance of true removal `D` requires `D > a³ + 2U`. Thus G12 is guaranteed to pass when:

```text
U < (5 − 10⁻⁹)/2 = 2.4999999995 mm³
```

The equivalent G11 bounds are `0.2999999995`, `2.9999999995`, and `29.9999999995 mm³`. Actual OCCT acceptance remains conditional on the missing measurement bound.

## 7. Counterexample search

**[Proved, Numeric] Strongest geometric ambiguity: G9.** Unchanged and `90°`-rotated faces can supply identical plane inputs. For the top basis, local point `(10,0)` remains at `(0,10,40)` under this policy; rigid following would place it at `(-10,0,40)`:

```text
error = √(10²+10²) = 14.1421356237 mm
```

No plane-only algorithm can distinguish them. The policy is sound only as explicitly defined plane re-seating.

**[Proved] Gram–Schmidt discontinuity:** with `n1=(0,√(1−b²),b)`, projected X tends to `−Z` as `b→0+` and `+Z` as `b→0−`. One fallback cannot equal both limits. Automatic fallback therefore defeats unconditional continuity; the repair branch makes the limitation explicit.

**[Numeric] Deadband failure:** a `0.009°` tilt at radius `10000 mm` moves a point by:

```text
2×10000×sin(0.009°/2) = 1.57079632518 mm
```

A normal-angle deadband would suppress a substantial displacement. Always applying the complete candidate avoids this, including combined translation/rotation.

**[Source, Inference] Identity attack:** an anchor sitting on a congruent decoy must not establish continuity after an edit. The shipped post-edit descriptor-tie veto handles this class. Scores and margins alone remain heuristic evidence, not identity proof. [Ladder.cpp](/Users/andrejvysny/workspace/OneCAD/worker/src/elementmap/Ladder.cpp:296)

**[Proved]** If each candidate score changes by at most `δ`, an existing margin `m` can shrink to `m−2δ`. Stable auto-binding requires both `score1−δ≥0.85` and `m−2δ≥0.10`. No score-perturbation bound is supplied, so arbitrary-small-edit binding stability cannot be promised.

## 8. Implementation handoff

**[Inference] Proposed worker interfaces:**

```cpp
struct SketchFrame {
    gp_Pnt origin;
    gp_Vec x_axis, y_axis, normal;
};

struct FrameChange {
    bool moved;
    bool significant;
    double translation_mm;
    double rotation_rad;
};

struct TransportResult {
    SketchFrame effective;
    FrameChange change;
    std::optional<RepairItem> repair;
    std::vector<Diagnostic> diagnostics;
};

TransportResult transport_sketch_frame(
    const SketchFrame& frozen,
    const ResolvedPlane& plane,
    const TransportPolicy& policy);

SeatResult classify_sketch_seat(
    const TopoDS_Face& face,
    const SketchFrame& frozen,
    const SketchFrame& candidate,
    const gp_Pnt& frozen_anchor);

CutEffectResult classify_cut_effect(
    const VolumeMeasurement& before,
    const VolumeMeasurement& after,
    double minimum_volume_mm3);
```

**[Inference] Placement of responsibilities:**

- Worker Sketch execution: plan-scoped resolution, conditioning, transport, face classification.
- Worker Extrude Cut: signed volume-effect check before publishing successor geometry/history.
- Rust planner: supplies host identity and immutable authoring frame; consumes derived results.
- Any range/preview path: share the predicate if it predicts this same operation; do not create a second tolerance policy.

**[Source]** The Cut insertion point is after boolean construction and before `publish_boolean_result`; existing Add/empty-result refusals already run there. [ExtrudeOp.cpp](/Users/andrejvysny/workspace/OneCAD/worker/src/ops/ExtrudeOp.cpp:1287)

**[Inference] Additive input/output contract:**

```text
Sketch params:
  plane: existing frame shape, carrying immutable F0
  hostFace?: typed semantic ref
  frameTransportVersion?: 1

planStep:
  sketchPlane?: { origin, xAxis, yAxis, normal }
```

`sketchPlane` appears only for an applied change. For a processed host sketch, absence means effective `F0`, not “retain the previous derived frame.”

**[Inference] Diagnostics:**

| Outcome | Code |
|---|---|
| Any applied movement | `SKETCH_HOST_RESEATED` — info |
| Reversed world normal | `SKETCH_HOST_NORMAL_REVERSED` — info |
| Non-planar/invalid/ill-conditioned/off-face/unmeasurable | Matching `SKETCH_*` code plus typed repair reason |
| Cut no demonstrable removal | `EXTRUDE_CUT_NO_EFFECT` |
| Cut volume increase | `EXTRUDE_CUT_VOLUME_INCREASED` |
| Missing/nonfinite volume evidence | `EXTRUDE_CUT_VOLUME_UNMEASURABLE` |

**[Inference] Rust adoption:** preserve immutable `F0` separately before updating effective `Sketch.plane`. Adopt only accepted, matching-step results; no undo entry for derived placement. Never adopt rewritten local geometry, a replacement frozen anchor, worker-computed fencing hashes, or the prior derived frame as the next transport seed.

**[Source]** Mate writeback provides the transactional precedent, but its existing hash adoption must not be mistaken for immutable-frame storage. [schema](/Users/andrejvysny/workspace/OneCAD/protocol/SCHEMA.md:698)

**[Inference] Log:** op/sketch/ref IDs; resolution rung, candidates, scores, margins and veto; plane/frame values; `h`; displacement/angle and significance; witness classification/tolerance; volume measurements, error bounds, `εV`, and decision. Geometry diagnostics must contain finite values. Cut refusal travels in terminal step diagnostics because failed steps emit no `planStep`. [PlanExecutor.cpp](/Users/andrejvysny/workspace/OneCAD/worker/src/session/PlanExecutor.cpp:884)

## 9. Confidence and the unknowns that would change the answer

**High:** projection mathematics, exact translation, fallback discontinuity, G9 underdetermination, analytical volume deltas.

**Medium:** the proposed plane attachment convention and production numerical guards.

**Unresolved inputs/decisions:**

- Accept plane re-seating, or require rigid following with additional transform/tangent evidence?
- Actual `F0` origins and G4/G8/G9 resolved planes/pivots.
- Verified volume measurement error bounds across the stated scales and coordinates.
- Availability of authoritative consumed-element state before descriptor fallback.
- Normal uncertainty near the conditioning guard.
- Persistent immutable-frame storage and strict host-resolution gate semantics.

**[Inference]** These gaps prevent an unconditional production epsilon and globally continuous rigid transport from being derived honestly from the supplied inputs.

---

## Break record (2026-09-15, call 2 of ~3 for WP-1, mode break, effort xhigh, grounded on the 12 paths named in the packet, packet sha256 4dfc61a7880f, session 01a0a46d-bfb9-7ef2-8ee5-2c2d49cdcbda)

**Verdict returned: defective.** Fable's rulings:

- **F1 accepted** (info-only `SKETCH_HOST_UNTRACKED` lets a low-confidence host execute dependents at the authored frame — a 40→35 edit with a 15 mm cut removes 1000 instead of 1500 mm³ and passes the Cut predicate). Closed by WP-1b: the halt is restored for every below-gate resolution, with the candidates carried.
- **F2 accepted** (`wire.rs` lowering overwrote a stored `frameTransportVersion` with 1, so the §13 refusal could never fire). Closed in WP-1b: default only when absent; no narrowing compare.
- **F3 accepted** (the origin-anchor seat exemption skips the witness exactly when the anchor is the origin; a face trimmed from [−3,3]² to [−1,1]² with the anchor at (2,0) resolved "successfully"). Closed in WP-1b: exemption dropped; on-face anchor frozen at creation/re-pick.
- **F4 rejected as a defect, recorded as a residual:** the tracked rung's bypass of descriptor scoring is the same trust every op places in the element-map partition (mate: `ComponentOp.cpp`, hole: `HoleOp.cpp::resolve_host_face`); the partition removes consumed elements through `elementMapDelta`, which the allowlist did not include. Follow-up test recorded: consume the host face by a through-cut, add a congruent replacement ⇒ `NeedsRepair`, never a bind.
- **F5 rejected:** the planner refuses any checkpoint that swallows a `Sketch` op a later executed step consumes (`onecad-core/src/regen/planner.rs` `a_checkpoint_that_swallows_a_consumed_sketch_op_is_refused`, outside the allowlist), so a suffix can never lack its profile.
- **F6 accepted in part:** the a³ floor is below binary64 spacing at 1e9 mm³ (2^(29−52) ≈ 1.19e-7 mm³), so a one-ULP integration difference on a 1 m body would pass as removal. Closed in WP-1b: εV gains a κ·ulp(V) floor (κ = 8) and the e/(1−e) form. The 1e-10 mm³ "real removal refuses" case is the accepted semantic floor.
- **F7 accepted as a consequence, not a defect:** a legacy hosted record with an off-face anchor halts after an upstream edit until re-picked (WP-1b makes re-pick freeze the on-face anchor). Strict refusal over silent replay, per the identity law.
- **F8 accepted** (b = 2e-8 passes the τGS guard but the projected x fails the 1e-9 orthogonality check ⇒ a valid authored frame is blamed). Closed in WP-1b: cross-product re-orthogonalisation after projection; stored-frame validity checked before transport, transport residuals after, with distinct reasons.
- **F9 accepted** (a malformed `sketchPlacement` parsed as "absent"). Closed in WP-1b: malformed ⇒ planStep parse failure; basis validated.
- **F10 recorded as an accepted residual:** `moved` is exact by design (no deadband); Rust's equality on `resolved_plane` prevents repeated adoption; representation jitter would show as a stable, harmless first adoption.
- Also recorded from § 4 "Tests required": the fixture's follow round replays a previously bound host without `editedFrom`; an empty-base + explicit-edit-context round is owed; Revolve ring and Boolean disjoint-Cut vectors are owed; `BOOLEAN_DISJOINT_RESULT` is a Union code in §7.3, so the Cut refusal keeps its own `CUT_NO_EFFECT` name.

Raw output: scratchpad `astra-break-sketch-host-face-20260915-113730.md` (not copied here; accepted findings above are the record).
