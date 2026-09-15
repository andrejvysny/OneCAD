# sketch-fillet-fill-and-tangency
date: 2026-09-15
mode: derive
model: gpt-6-astra
effort: xhigh
access: grounded: worker/src/loop/PolygonFill.cpp, worker/src/loop/LoopDetector.{cpp,h}, worker/src/protocol/SolverLane.cpp, worker/src/sketch/solver/ConstraintSolver.cpp, worker/third_party/planegcs/{GCS.h,Constraints.cpp}, worker/tests/test_sketch_arc_endpoints.cpp, src/tools/sketch/{sketchFilletMath,sketchService}.ts, protocol/SCHEMA.md. Transcript audit: also read src/tools/sketch/toolMachine.ts, protocol/fixtures/ and the legacy oracle LoopDetector under corpus (recorded; no corpus/expected-values read).
packet: sha256 fe279e335a22
calls: 1 of ~3 for WP-3 (session 01a0a3f3-508e-7cd0-8bbc-fcefaadc70f8 recorded for followup)
verdict: n/a (derive)
verified by: Fable recomputed F1 (m = 8 ⇒ N = 12 ⇒ T = 10), F3 10° (d = 5/tan 5° = 57.15 mm > 20 ⇒ radiusTooLarge, maxRadius = 20·tan 5° = 1.7498 mm), T1's DOF baseline (8 + 9 − 4 ArcRules − 4 weld equations = 9), the first-order-singular distance-tangency argument (r(1−cos δ) = r δ²/2 ⇒ f'(0) = 0; at r = 5, δ = 1e-3: 2.5e-6 mm) and the f32 spacing at 10 000 mm (2^(13−23) = 0.000977 mm). The synthetic F4 (29/29 complete) is Astra's own Bun computation — recorded as a hypothesis about B4, NOT a reproduction (A-15 stands: the fix site is chosen after the exact repro).
rejected: (1) "add endpoint semantics to the existing Tangent kind" — REJECTED per the protocol pre-audit Q8 (positions are aligned with entities and Tangent's slots are curve slots; an older reader would silently solve entity-level tangency): the distinct kind `TangentAtPoint` is used (plan P-8); the lowering, branch policy and residual unit (rad) are accepted unchanged. (2) The full occurrence-aware bridge-tree rewrite and the O(n²) blocker-count clipper are recorded as the target design but scoped: this program implements the certified orientation predicate (filtered + exact fallback), inclusive containment via the same predicate, exact-collinear chain simplification with restoration, closure-duplicate removal, the post-fill verification (count, positive orientation, boundary edges single-use, bridges two-incident, area equality, f32 encoding check) and the named refusal codes; the bridge construction stays as shipped but is verified by the same checks. (3) OCCT BRepMesh fallback: not approved (accepted as Astra states it).
Decisions on Astra's unknowns: the effective detector config is read at run time and passed into the fill policy (no duplicated 1e-4 constant); the angular acceptance tolerance for TangentAtPoint follows A2's local rule (τ_len / max(line span, arc radius)); a minimum net-material-area policy is NOT introduced (the existing loop area floor 1e-6 mm² is made one shared named policy).

## 1. Problem restatement

Labels: **[P]** proved mathematically; **[N]** computed numerically; **[S]** source-cited; **[I]** inference or proposed policy.

**[P] Misframing:** a positive geometric ear-area cutoff cannot guarantee triangulation of every simple polygon; deleting a collinear vertex preserves the boundary’s geometric locus, but changes its indexed edges.

**[I] B4 hypothesis:** a sub-tolerance closing-seam mismatch or duplicate is more plausible than ordinary collinear splits; a faithful F4 probe must distinguish that from an orientation-predicate error.

**[S] Three source corrections affect the derivation:**

- Legacy arc tessellation has **at least eight segments**. Exact-fragment tessellation has a separate chord policy and **at least 16**. The active configuration returned by `makeRegionDetectionConfig()` is outside the allowlist. [Tessellation](/Users/andrejvysny/workspace/OneCAD/worker/src/loop/LoopDetector.cpp:194), [defaults](/Users/andrejvysny/workspace/OneCAD/worker/src/loop/LoopDetector.h:242).
- Polygon assembly already removes **consecutive** vertices at distance ≤ configured coincidence tolerance, but does not compare the last retained vertex with the first. The fill’s closing-point check instead uses coordinate differences < `1e-12`. [Assembly](/Users/andrejvysny/workspace/OneCAD/worker/src/loop/LoopDetector.cpp:2050), [closing-point check](/Users/andrejvysny/workspace/OneCAD/worker/src/loop/PolygonFill.cpp:16).
- The measured slot constrains **equal cap radii**, not equal wall lengths. That distinction changes the endpoint-tangency rank result. [Slot fixture](/Users/andrejvysny/workspace/OneCAD/worker/tests/test_sketch_arc_endpoints.cpp:112).

**[N] Synthetic F4 results:** a standalone Bun transcription of the existing ear predicates, with exact shared endpoint coordinates, completed every cyclic start tested: **29/29 triangles** with legacy sampling; **59/59** with fragment sampling. Appending a closing representative `(0.00005, 0) mm` instead of `(0,0)` produced **29/30**, leaving a collinear backtracking remainder. This demonstrates a possible failure mechanism, **not B4’s measured cause**.

## 2. Properties to guarantee

**[I] Publication requirements:**

| Property | Check |
|---|---|
| Completeness | Every accepted simple, positive-area working polygon triangulates completely using certified predicates. Publication additionally passes area, topology and encoding checks. |
| Boundary preservation | Single-use triangle edges equal the required outer/hole boundary edges, including protected collinear splits. |
| Interior edges | Every other triangle edge, including every bridge, has exactly two incident triangles with opposite directed uses. |
| Geometry | Triangles have positive orientation, interiors do not overlap, and their union equals the polygonal material region. |
| Determinism | Fixed normalization, bridge ordering, tie-breaks and ear ordering produce identical indices for identical inputs and policy. |
| Fail closed | Missing holes, unresolved topology, stalled clipping or failed verification publish no fill. |
| Tangency | Each **independent** endpoint-tangency row increases Jacobian rank by one; accepted drag results satisfy its angular residual and endpoint weld. |
| Blame | ArcRules remain internal, tag `0`; authored endpoint tangency receives its own user constraint tag. |

**[P] Triangle accounting:** for `N` boundary vertices and `H` holes, with no interior vertices:

\[
T=N+2H-2.
\]

A bridge-cut loop has `n = N + 2H` occurrences, hence `T = n−2`. **`n` is not `vertexCount`.**

**[P] Collinear-chain accounting:** if clipping temporarily removes `k` protected collinear vertices, restore them by subdividing their boundary-adjacent triangles. This adds exactly `k` triangles and restores `T=N+2H−2`.

**[P] Qualification:** “every authored Tangent removes one DOF” is impossible when existing constraints already imply it. The guarantee must concern an independent new row; T5 with equal walls supplies a concrete counterexample.

## 3. Predicates and epsilon derivation

### Ear orientation and containment

**[I] Use an arithmetic uncertainty bound, not a geometric flattening tolerance.**

For `x=b−a`, `y=c−a`, compute:

\[
D=x_x y_y-x_y y_x,\quad
S=|x_x y_y|+|x_y y_x|,
\]
\[
u=2^{-53},\qquad
\varepsilon_{\rm area}=\gamma_8 S,\qquad
\gamma_8=\frac{8u}{1-8u}\approx8.8818\cdot10^{-16}.
\]

**[P] Acceptance policy:**

- `D > ε_area`: certified convex.
- `D < −ε_area`: certified reflex.
- Otherwise: compute the **exact determinant sign of the input binary64 coordinates**.
- Exact sign `>0`: convex; `=0`: collinear.

The conservative bound follows by accumulating subtraction, multiplication and subtraction roundoff, assuming ordinary IEEE rounding and no overflow/underflow. Exceptional ranges require exact fallback or verified power-of-two scaling.

**[P] Why no positive geometric cutoff:** a simple polygon can contain an essential convex vertex with arbitrarily small positive determinant. Rejecting every `0 < D ≤ ε` therefore defeats the universal completeness claim. A bbox-relative positive cutoff has the same problem.

**[S] Existing `point_in_triangle` is inclusive:** points on triangle edges and vertices count as contained. [Predicate](/Users/andrejvysny/workspace/OneCAD/worker/src/loop/PolygonFill.cpp:94).

**[I] Preserve that inclusivity using the same certified orientation predicate:**

\[
p\in\triangle abc
\iff
\operatorname{orient}(a,b,p)\ge0,\ 
\operatorname{orient}(b,c,p)\ge0,\ 
\operatorname{orient}(c,a,p)\ge0
\]

for a positive triangle. A different boundary vertex on proposed diagonal `ac` blocks the ear. Ignore repeated occurrences of the ear’s own canonical endpoint IDs only with the corresponding bridge-sector/topology checks; coordinate proximity alone is insufficient.

### Merge and collinear pre-pass

**[I] Merge reach:**

\[
\varepsilon_{\rm merge}=\tau_{\rm graph}\le10^{-4}\ {\rm mm},
\]

where `τ_graph` is the **effective tolerance used for this detected region**, not merely the configuration default. Chord tolerance does not enter this decision.

**[S] The exact-fragment graph derives a size-dependent coincidence tolerance capped by the configured maximum. Thus the packet’s default `1e-4 mm` is not necessarily the effective graph tolerance. [Policy](/Users/andrejvysny/workspace/OneCAD/worker/src/loop/LoopDetector.cpp:240).

**[I] Distance is a candidate test, not permission to merge:**

- Remove an exact redundant closing representation or consecutive exact duplicate.
- Merge distinct nearby coordinates only with provenance proving they represent the same graph endpoint and no boundary interval is being removed.
- Use the graph’s canonical representative; do not average.
- Do not perform transitive proximity clustering or merge across rings.
- Otherwise retain a valid distinct vertex; refuse if its unresolved identity creates overlap or a pinch.

**[P] Exact collinear simplification requires**

\[
\operatorname{orient}(a,b,c)=0,\qquad
(b-a)\cdot(c-b)>0.
\]

Then `[a,b] ∪ [b,c] = [a,c]`. Backtracking fails the second predicate.

**[P] This proves equality of geometric boundary loci, not indexed edge sets.** Record the chain `a,b,c`; after clipping replace the triangle adjacent to `ac` by two triangles incident to `ab` and `bc`. Retain protected source junctions and tessellated curvature samples. Near-collinearity never licenses deletion.

### Area and retry policy

**[S] The detector already has `kMinArea = 1e-6 mm²`; its paths differ between `<` and `≤` rejection. The fill has no separate minimum-area gate. [Detector](/Users/andrejvysny/workspace/OneCAD/worker/src/loop/LoopDetector.cpp:41).

**[I] Make the loop publication policy explicit and consistent:** require loop area `> 1e-6 mm²`; reject a required sub-floor hole rather than silently filling it. A separate minimum **net material area** is not specified by the packet and cannot be inferred from `ε_merge²`.

**[P] Normalize outer CCW and holes CW once using certified signed area. Reversing a stalled CCW loop while retaining a CCW ear test is incorrect. Reversing both orientation and predicate merely restates the same geometry.**

**[I] A deterministic restart from a different start occurrence is permissible only from the original working loop and with full verification. It is a diagnostic or implementation fallback, not a reason to accept an incomplete fill. Certified clipping of a valid simple polygon should not require it.**

### Endpoint tangency

Let `T` be the welded endpoint, `q=T−C`, and `d=L.end−L.start`.

**[P] Geometric endpoint tangency is**

\[
d\cdot q=0,\qquad |d|>0,\quad |q|=r>0.
\]

A dimensionless equivalent is `(d·q)/(|d|r)=0`; it has two orientation branches.

**[I] Lower to `addConstraintAngleViaPoint(curve1, curve2, T, &α, userTag, true)`, with fixed `α∈{0,π}`.** Keep the Coincident weld separately.

**[S] PlaneGCS’s actual error is**

\[
e=s\operatorname{atan2}
\left(
\operatorname{cross}(R_\alpha n_1,n_2),
(R_\alpha n_1)\cdot n_2
\right),
\]

where `n1`, `n2` come from `CalculateNormal(T)` and `s` is its internal scale. The unscaled residual is **radians**, not millimetres. [Implementation](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/Constraints.cpp:2355).

**[I] Branch handling:**

- At the valid fillet seed, evaluate the actual PlaneGCS normals. Choose `α=0` when aligned, `π` when opposed.
- Keep this target constant through the solve/gesture; do not reselect the nearest branch every iteration.
- Store its scalar in stable memory and exclude it from unknown parameters.
- Reversing a line’s parameterization changes the branch mapping. Arc Start/End and traversal direction must be handled explicitly.
- Reject zero normals, an ambiguous branch seed, or an accepted solution violating the selected branch.

**[P] For a CCW arc, its geometric tangent is `Jq/r`, where `J(x,y)=(-y,x)`. Tangents pointing *away* from the joint are**

\[
t_{\rm line,away}=\sigma_L\,d/|d|,
\qquad
t_{\rm arc,away}=\sigma_A\,Jq/r,
\]

with `σStart=+1`, `σEnd=−1`. A smooth fillet continuation requires these to be opposite. This handles either line orientation without assuming PlaneGCS’s normal-sign convention.

**[P] Why entity-level tangency fails at a single welded corner:** perturb the line by angle `δ` from tangency while preserving the weld and ArcRules:

\[
f_{\rm distance}=r|\cos\delta|-r
=-\frac r2\delta^2+O(\delta^4),\qquad f'(0)=0.
\]

Thus the distance row adds **zero first-order rank** on the welded manifold. It still expresses a nonlinear restriction; the weld alone does **not** imply tangency. Endpoint angle instead has `e=±δ+O(δ²)`, giving rank one.

**[N] At `r=5 mm`, `δ=0.001 rad`, the distance residual is only `−2.4999998e−6 mm`, whereas the endpoint-angle residual is `0.001 rad`.**

**[S] `TangentCircumf` is unsuitable for endpoint lowering:** its ordinary error is center-distance squared minus squared sum/difference of radii, in mm²; its near-concentric branch switches to a radius difference in mm. It does not identify the welded endpoint. [Implementation](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/Constraints.cpp:1495).

### Scaling laws

**[P] For positive similarity scale `λ`, and coordinate-unit conversion factor `k`:**

| Quantity/predicate | Similarity | Unit conversion |
|---|---|---|
| Orientation determinant and arithmetic bound | Both scale by `λ²`; sign unchanged | Both scale by `k²` |
| Exact collinearity, incidence | Preserved mathematically | Preserved mathematically |
| Graph merge tolerance, mm | Fixed physical threshold: similarity can change classification | Multiply numeric tolerance by `k` |
| Area floor, mm² | Fixed physical threshold: similarity can change classification | Multiply by `k²` |
| Chord tolerance `min(0.05 mm, .02r)` | Curvature-relative until absolute cap applies | Convert `0.05 mm` by `k` |
| Endpoint angular residual | Unchanged | Unchanged; angular tolerance stays radians |
| Angular coordinate derivatives | Scale as `1/λ` | Scale as `1/k` |

**[I] Floating-point conversion itself may perturb exact collinearity; certified predicates must classify the supplied coordinates, without guessing their intended exact geometry.**

## 4. Degeneracy classes

**[I] Required branches:**

| Class | Detection | Action |
|---|---|---|
| Collinear triple | Exact orientation zero; forward dot product positive | Temporary chain simplification; restore protected splits |
| Backtracking/overlapping run | Collinear with reversed traversal or overlapping nonadjacent edges | `FILL_BOUNDARY_OVERLAP` |
| Short genuine leg | Distinct endpoints; positive length, however short relative to arc chords | Preserve; chord tolerance is irrelevant |
| Sub-tolerance endpoint representation | Distance ≤ effective graph tolerance plus same-endpoint provenance | Canonicalize only the redundant representation |
| Duplicate/pinch | Repeated nonadjacent boundary position/identity outside certified bridges | `FILL_BOUNDARY_PINCH` or unresolved-identity refusal |
| Bridge-collapsed ear | Repeated canonical IDs among `(a,b,c)` | Skip; retain bridge occurrences and their sectors |
| Invalid hole contact | Hole touches/crosses outer boundary, another hole or bridge improperly | Refuse; do not invent a bridge topology |
| Too-small loop | Certified area ≤ explicit loop floor | `FILL_AREA_BELOW_MINIMUM` |
| Arithmetic uncertainty | `|D|≤ε_area` | Exact fallback, never automatic flattening |
| Wire-coordinate collapse | f32 conversion collapses an edge/triangle or creates an intersection | `FILL_F32_TOPOLOGY_LOSS` |
| Sweep collapse | Solved extent `<1e−3` or `>2π−1e−3 rad` | Restore previous pose; partial drag result |
| Zero-length tangent leg | `|d|=0`, nonfinite coordinates or undefined normal | `TANGENT_DEGENERATE_OPERAND` |
| Short tangent leg, T2 | `0<|d|≪` other lengths | Regular equation; inspect conditioning and angular residual |
| T-junction, T3 | More than two incident curves at selected endpoint | Bind the explicit operand pair; never choose “the other leg” |
| Arc–arc, T4 | Both selected endpoints welded | Angle between their endpoint normals; rank-one if independent |
| Coincident arc supports | Equal centers/radii | Tangency can be valid for adjacent intervals; overlapping intervals remain a region-topology refusal |
| Slot/global redundancy, T5 | New row lies in existing Jacobian row span | Report redundancy against user constraints; never ArcRules |

**[S] PlaneGCS’s angle error intentionally evaluates to zero for zero normals, while its gradient divides by normal length squared. Therefore solver “zero residual” does not validate a collapsed leg. [Error and gradient](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/Constraints.cpp:2370).

**[P] Arc–arc entity-level tangency has the same rank defect at a weld.** With radial angle difference `φ`:

\[
|C_1-C_2|^2=r_1^2+r_2^2-2r_1r_2\cos\phi.
\]

Both internal tangency at `φ=0` and external tangency at `φ=π` have zero first derivative in their distance-squared formulation.

## 5. Algorithm sketch

### Fill

**[I] Input:** ordered outer/hole coordinates, source/node provenance, effective detector policy.  
**Output:** complete verified indexed fill, or one named refusal with evidence.

1. **Validate input:** finite coordinates, legal ring sizes, provenance consistency.
2. **Pre-pass each ring before bridging:** remove redundant closure representations; perform only certified merges; record exact forward-collinear chains.
3. **Validate topology:** certified segment intersection/contact classification, ring simplicity, hole nesting and area policy. Refuse crossings, overlaps and pinches.
4. **Normalize winding and deterministic ordering.**
5. **Bridge holes:** construct a noncrossing visible bridge tree, preserving separate loop occurrences at shared bridge endpoints. Reject bridge-through-vertex and collinear-overlap cases.
6. **Clip positive ears:** certified orientation, inclusive containment and valid occurrence sectors. Clip in deterministic order.
7. **Restore protected collinear boundary vertices** by subdividing boundary-adjacent triangles.
8. **Verify:** expected count, index ranges, positive triangles, edge incidences, exact required boundary edges, area equality and absence of overlaps.
9. **Convert to f32 and verify the geometry actually published.**
10. **Publish atomically** only after all holes and checks pass.

**[P] Completeness argument:** a simple polygon has a boundary-preserving triangulation; its dual tree has an ear. Inclusive containment prevents skipping mandatory boundary vertices. Removing a certified ear preserves simplicity, so induction terminates. Exact-collinear chain restoration preserves the boundary and adds the required triangles.

**[P] For holes, the same argument applies after a valid bridge cut, with occurrences distinguished by their incident sectors. Arbitrarily deduplicating bridge occurrences destroys that argument.**

**[I] Complexity target:** `O(n²)` predicate work, `O(n)` clipping state:

- Maintain blocker counts for candidate ears.
- Removing one vertex updates unchanged candidates in `O(n)`.
- Recompute only the two changed neighbor triangles in `O(n)`.
- Select ears deterministically.
- Pairwise final intersection verification remains `O(n²)`.

**[P] The present nested “all candidates × all other vertices” clipper is cubic in the worst case; its guard does not change that.** The existing exhaustive shortest-bridge search also needs replacement if a strict overall quadratic bound is required.

**[I] One bridge construction meeting that target is a deterministic rightmost-hole ray/visibility method: process holes from right to left, locate the nearest ray intersection, select a visible existing endpoint using the obstructing reflex vertices, and certify the chosen segment and endpoint sectors. This avoids adding boundary vertices and scans `O(n)` data per hole.**

**[I] OCCT fallback remains unapproved:** no allowlisted evidence establishes its output’s correspondence to required boundary vertices or bridge incidence. Any fallback must pass the same checks; mesh generation alone proves neither contract.

### Tangency

**[I] Lowering sequence:**

1. Detect endpoint form from `Tangent.positions`.
2. Resolve both owner curves and their actual Start/End points.
3. Require an existing structural weld/shared point—not merely nearby coordinates.
4. Validate finite geometry, positive radius, nonzero line direction and legal arc extent.
5. Construct complete GCS curves and select the welded point.
6. Add one fixed-angle `AngleViaPoint` row under the authored constraint’s tag.
7. Diagnose rank/redundancy; retain ArcRules under tag `0`.
8. After solving and dragging, validate weld residual, angular residual, branch and degeneracy guards; roll back a rejected step.

**[S] The current `makeArc` intentionally leaves Start/End unset because existing entity-level constraints use center/radius/angles. Endpoint lowering should construct a complete arc rather than silently rely on that helper’s old assumption. [Helper](/Users/andrejvysny/workspace/OneCAD/worker/src/sketch/solver/ConstraintSolver.cpp:108).

## 6. Test vectors

### Fill vectors

**[P] Concrete corner fixture:** `P=(0,0)`, `u=(1,0)`, `v=(cosθ,sinθ)`. Close the two legs through `La·u+Lb·v`, then replace the corner by the specified fillet. This defines F2/F3 closures explicitly rather than assuming missing geometry.

**[N] Sampling calculation:**

\[
m=\operatorname{clamp}
\left(
\left\lceil\frac{s}{2\arccos(1-t/r)}\right\rceil,
m_{\min},m_{\max}
\right).
\]

The fragment column below uses `t=0.01 mm`—ten times the schema’s `0.001 mm` authoring resolution—and the source’s 16-segment floor. These are synthetic lane comparisons, not an assertion about the live detector configuration.

| Vector | Concrete input/calculation | Expected output |
|---|---|---|
| **F1** | `La=30`, `Lb=20`, `θ=90°`, `r=5`; `m=8 / 16`; `N=m+4` | **10 / 18 triangles** |
| **F2** | `La=5+L`, `Lb=20`, `L=.2,.05,.01 mm`; same arc | **10 / 18** for every `L`; preserve the short leg |
| **F3, 170°** | `d=5/tan85°=.437443318 mm`; sweep `10°`; `m=8 / 16` | **10 / 18** |
| **F3, 10°** | `d=5/tan5°=57.1502615 mm > min(30,20)` | Frontend `radiusTooLarge`; `maxRadius=20tan5°=1.74977327 mm` |
| **F3, feasible acute variant** | Both original legs `100 mm`; `m=11 / 24` | **13 / 26** |
| **F4, upper cell** | Rectangle `[0,30]×[0,20]`; inward semicircle `C=(15,0), r=12.5`; top-right fillet `C=(25,15),r=5` | Semicircle `k=18 / 40`; `N=m+k+5=31 / 61`; **29 / 59** |
| **F4, lower cell** | Middle bottom-edge segment retained, bounded by inward semicircle | `N=k+1`; **17 / 39** |
| **F4, bottom-corner fillet** | Circle intersections `(2.5,0)`, `(27.5,0)` leave only `2.5 mm` bottom legs | `r=5` exceeds available corner radius: refuse |
| **F5** | F4’s filleted rectangle without notch; square hole `[26.9,27.1]×[15.9,16.1]` | `N=16 / 24`, `H=1`; **16 / 24 triangles** |
| **F6** | Rectangle with extra `(δ,0)` after `(0,0)`, `δ∈{.9,1,1.1}·10−4 mm` | Distinct protected boundary split: `N=5`, **3 triangles**. Certified duplicate representation: `N=4`, **2**. Distance alone does not choose between them. |

**[P] F4’s two circle intersections do not necessarily create consecutive collinear triples in the selected cell:** the upper cell follows the inward arc between them. Retaining the original middle straight segment creates the separate lower cell. The actual selected region must be recorded.

**[N] F2 ear determinants at the tangent point:** for angular step `Δ`,

\[
D=Lr(1-\cos\Delta).
\]

For legacy `m=8`, these are **0.0192147, 0.00480368, 0.000960736 mm²**. For fragment `m=16`: **0.00481527, 0.00120382, 0.000240764 mm²**. These are positive, well-resolved ears despite the short legs.

**[N] F5’s legacy shortest bridge lands on fillet sample**
`(29.6193977,16.9134172)`, from hole vertex `(27.1,16.1)`, length `2.64745388 mm`. The bridge must have exactly two incident triangles.

**[I/N] Additional threshold tests:**

| Class | Input and expected result |
|---|---|
| Exact collinear chain | `(0,0),(1,0),(2,0),(2,1),(0,1)`: skeleton 4 vertices/2 triangles; restore split → **5 vertices/3 triangles** |
| Closure duplicate | Append the first point exactly: remove redundant representation; triangle count unchanged |
| Near closure | F4 plus closing `(5e−5,0)`: current clip transcription gives **29/30**; canonicalize only with closure provenance, otherwise `FILL_BOUNDARY_OVERLAP` |
| Area floor | Triangle `(0,0),(.125,0),(.0625,h)`; `A=h/16`. Use `h=16·Amin·{.99,1,1.01}`: proposed gate refuses first two, accepts third |
| Sweep floor | Test `s=.000999,.001,.001001 rad`: first violates the normative sweep guard; equality is permitted by that guard |
| Floor sampling | `r=5,s=.001,m=8`: chord `0.000625 mm`; arc-triple determinant `4.88281e−11 mm²`. Preserve inside a larger valid region |
| Radius/sweep interaction | `r=.01,s=.001`: entire endpoint separation ≈`1e−5 mm`, below `1e−4 mm`; solver-valid extent need not yield a graph-resolvable region |
| Pinch | Two otherwise valid rings touching at one vertex: refuse touching topology; do not treat as a hole bridge |

### Tangency vectors

**[P] Counts below assume free geometry and only the explicitly listed constraints.** They are mathematical rank expectations; the new lowering has not been executed in PlaneGCS.

| Vector | Baseline count | Endpoint Tangent result | Redundancy |
|---|---:|---:|---|
| **T1**: two legs, one arc, two welds | `8+9−4 ArcRules−4 welds=9` | First: **8**; second: **7**; Radius additionally: **6** | Neither tangent redundant |
| **T2**: same, one leg `.01 mm` | **9** | **8 → 7** | Same rank; worse conditioning |
| **T3**: one arc endpoint welded to two lines | `5+4+4−2−2=9` | Explicit first pair: **8**; second pair, if independent: **7** | Depends on existing line-direction constraints |
| **T4**: two arcs, one endpoint weld | `5+5−2=8` | **7** | Nonredundant unless other constraints already impose tangent direction |
| **T5**, measured equal-cap-radius slot | `26−8 ArcRules−8 welds−1 Equal=9` | **9→8→7→6→5** | All four endpoint rows independent |
| **T5**, equal walls instead | `10−1 Equal=9` after eliminating welds/ArcRules | **9→8→7→6→6** | One row dependent |
| **T5**, both Equal constraints | **8** | **8→7→6→5→5** | One row dependent |

**[P/N] Slot rank calculation:** after eliminating welds and ArcRules, use two centers, two radii and four endpoint angles: **10 variables**. At wall length `L=100 mm`:

\[
\delta\beta_{\rm top}=
\frac{\delta C_{Ry}-\delta C_{Ly}+\delta r_R-\delta r_L}{L},
\]
\[
\delta\beta_{\rm bottom}=
\frac{\delta C_{Ry}-\delta C_{Ly}-\delta r_R+\delta r_L}{L}.
\]

Each endpoint row is `δβwall−δαendpoint=0`. Gaussian elimination gives the rank sequences in the table. Equal wall length adds the row proportional to

\[
r(\delta\alpha_{LT}+\delta\alpha_{LB}
-\delta\alpha_{RT}-\delta\alpha_{RB}),
\]

which is already implied by the four tangent rows.

**[N] T2 conditioning:** angular derivatives scale like `1/L`; `.01 mm` gives approximately `100 rad/mm`, versus `.04 rad/mm` for `25 mm`: **2500× larger**, without a rank loss.

**[I] Additional solver tests:** reverse each line’s endpoint order; swap arc Start/End roles consistently; perturb tangency by ±`0.001 rad`; drag through zero leg length; test T3 with fixed nonparallel branches; test T4 with already-concentric equal-radius supports. Expect invariant physical tangency, explicit rollback on collapse, conflict for incompatible fixed branches, and genuine redundancy when already implied.

## 7. Counterexample search

### Real 0.02 mm step

**[N] Consider the boundary run**

`(0,0) → (10,0) → (10,.02) → (10.02,.02) → (10.02,0) → (30,0)`,

closed by `(30,20),(0,20)`.

The step edges are `0.02 mm`; the small-corner determinant is

\[
0.02^2=0.0004\ {\rm mm^2}.
\]

A `0.05 mm` merge/chord policy can erase it. The proposed policy cannot: `0.02 mm = 200·10−4 mm`, and the step is not collinear.

### Positive orientation inside arithmetic uncertainty

**[N] For**

`a=(0,0)`, `b=(3000,3000)`,  
`c=(6000,6000+2−40)` mm,

the exact determinant of those binary inputs is

\[
3000\cdot2^{-40}=2.7284841\cdot10^{-9}\ {\rm mm^2}.
\]

Ordinary evaluation gives `3.7252903e−9`, while the filter bound is `3.1974423e−8 mm²`. Treating that bound as a geometric zero would erase a positive turn. Exact fallback classifies it correctly.

### f32 defeats an otherwise correct double-precision fill

**[N] At `10000 mm`, binary32 spacing is**

\[
2^{13-23}=0.0009765625\ {\rm mm}.
\]

Both `10000` and `10000.0002 mm` convert to the same f32 value, despite separation exceeding `1e−4 mm`. Edge-count verification alone misses the resulting zero-length boundary. The proposed encoding check refuses it.

### Tangency’s two roots and drag flips

**[P/N] With `T=(0,0)`, line endpoint `F=(10,0)`, `r=5`:**

- `C=(0,5)` satisfies endpoint tangency.
- `C=(0,−5)` also satisfies it.

A dot-product-zero formulation accepts both. Their arc normals are opposite; a fixed `AngleViaPoint` target accepts only the selected relative-normal branch.

**[P] A remaining trap is collapse:** changing `F=(10,0)` to `F=(−10,0)` reverses the line direction through `F=T` along the straight interpolation. There the normal is undefined.

**[I] Freeze the branch, monitor nonzero directions and endpoint/sweep continuity, and subdivide or reject steps that cannot establish continuity. A residual check alone cannot prove that an arbitrary nonlinear solver jump remained in the intended solution component.**

## 8. Implementation handoff

**[I] Proposed C++ interfaces—not existing declarations:**

```cpp
struct FillPolicy {
    double graphCoincidenceMm;
    double minimumLoopAreaMm2;
};

struct BoundaryInput {
    std::span<const sk::Vec2d> points;
    std::span<const VertexOrigin> origins;
};

struct FillFailure {
    FillFailureCode code;
    FillEvidence evidence;
};

using PrepassResult = std::variant<PreparedLoop, FillFailure>;
using FillResult = std::variant<VerifiedRegionFill, FillFailure>;

PrepassResult prepass_loop(
    const BoundaryInput& input,
    const FillPolicy& policy);

Orientation orient2d_certified(
    const sk::Vec2d& a,
    const sk::Vec2d& b,
    const sk::Vec2d& c);

FillResult fill_region_checked(
    const BoundaryInput& outer,
    std::span<const BoundaryInput> holes,
    const FillPolicy& policy);

LoweringResult lower_endpoint_tangent(
    const EndpointTangentSpec& spec,
    SolverContext& context);
```

**[I] Required data responsibilities:**

- `VertexOrigin`: source interval/endpoint identity, graph-node identity where applicable, and whether the sample carries required boundary.
- `PreparedLoop`: retained output vertices, clipping occurrences, deferred boundary chains and expected boundary edges.
- `FillEvidence`: ring/occurrence/source IDs, coordinates, effective tolerance, determinant/bound/exact sign, candidate blocker, area, bridge IDs and edge-incidence mismatch.

**[I] Predicate ownership:**

| Location | Responsibility |
|---|---|
| `LoopDetector.cpp` | Carry effective tolerances and sample/node provenance; avoid losing required boundary before fill |
| `PolygonFill.cpp` | Certified predicates, pre-pass, bridge topology, clipping and verification |
| `SolverLane.cpp::on_regions` | Map refusal to `OP_FAILED` diagnostics; verify f32 output; publish atomically |
| `ConstraintSolver.cpp` | Endpoint resolution, GCS lowering, branch lifetime, rank and post-solve guards |
| Frontend authoring | Explicit endpoint roles, welds, tangents, radius and trim remapping |
| Wire readers/core planner | Preserve and validate endpoint semantics; no geometric epsilon decisions belong in a range analyzer or planner |

**[I] Constants:** keep the orientation error-bound factor beside the certified predicate. Pass graph coincidence from detection; do not duplicate `1e−4` as an unrelated fill constant. Give the existing area floor one shared named policy. Keep chord tolerance exclusively in tessellation.

**[I] Proposed refusal codes:**

- `FILL_NONFINITE_INPUT`
- `FILL_BOUNDARY_IDENTITY_UNRESOLVED`
- `FILL_BOUNDARY_OVERLAP`
- `FILL_BOUNDARY_PINCH`
- `FILL_INVALID_HOLE`
- `FILL_AREA_BELOW_MINIMUM`
- `FILL_NO_VISIBLE_BRIDGE`
- `FILL_EAR_STALLED`
- `FILL_RING_CONTRACT_FAILED`
- `FILL_F32_TOPOLOGY_LOSS`
- `TANGENT_ENDPOINT_UNRESOLVED`
- `TANGENT_ENDPOINT_NOT_WELDED`
- `TANGENT_DEGENERATE_OPERAND`
- `TANGENT_BRANCH_AMBIGUOUS`
- `TANGENT_RESIDUAL_EXCEEDED`

Emit bounded evidence on stderr and structured diagnostics. Never expose a partial triangle list as a successful region.

### Wire form

**[I] Add endpoint semantics to the existing Tangent kind:**

```json
{
  "id": "tangentA",
  "type": "Tangent",
  "entities": ["trimmedA", "filletArc"],
  "positions": ["End", "Start"]
}
```

- Absent/all-empty positions retain legacy entity-level meaning.
- Endpoint form requires two valid endpoint roles on supported curve owners.
- Partial, unsupported or unresolved roles fail loudly.
- Its arity is **one**; the two-equation weld remains a separate Coincident.
- Readers unable to honor this form must reject it. Additive syntax alone cannot guarantee that older readers will do so; compatibility gating must be verified.

**[S/I] Residual reporting also needs an amendment:** the schema currently describes Tangent residuals as mm. Endpoint Tangent must report radians while legacy Tangent retains its existing interpretation. [Residual contract](/Users/andrejvysny/workspace/OneCAD/protocol/SCHEMA.md:2941).

### Frontend authoring

**[I] Fillet must author, atomically:**

1. Two endpoint Coincidents.
2. Two `Tangent` constraints with matching endpoint `positions`.
3. One `Radius(filletArc, requestedRadius)`.
4. Correctly remapped surviving constraints on both legs.

**[P] The Radius removes one additional independent DOF; tangency alone permits radius changes.**

**[S/I] Trim currently deletes every constraint referencing the replaced entity, and surviving pieces inherit none. [Trim](/Users/andrejvysny/workspace/OneCAD/src/tools/sketch/sketchService.ts:372). Change this to provenance-based remapping:**

- Preserve endpoint tangency/weld only on the piece retaining that exact endpoint.
- Remove a tangent whose contact endpoint was consumed; never move it to a new intersection.
- Preserve radius intent on surviving pieces of the same circle support.
- Author explicit incidence at new trim intersections where connectivity is intended.
- Do not discard a real short boundary because a screen-scaled trim threshold exceeds it.

## 9. Confidence and unknowns

**High — [P/S]:**

- Entity-level distance tangency is first-order singular at any welded tangent corner.
- A regular endpoint-angle row removes one independent DOF.
- The measured equal-cap-radius slot accepts four independent endpoint rows.
- Chord-based merging and positive geometric ear cutoffs cannot meet the requested guarantees.
- Collinear simplification needs boundary-chain restoration to preserve indexed edges.

**Medium — [I]:** the proposed occurrence-aware fill and lowering design; implementation and real PlaneGCS drag tests remain necessary.

**Low — [I]:** attribution of B4. The synthetic F4 did not reproduce it.

**Required F4 probe evidence:**

- Frontend entity/constraint JSON **and solved coordinates**.
- Selected corner, selected region, and whether the middle circle-edge segment remains.
- Actual detector flags, effective coincidence tolerance, sampling policy/counts.
- Polygon order before/after filtering; first–last distance; near-pair node/source identities.
- Every stalled candidate’s determinant, error bound, exact sign and blocking vertex.
- Hole bridges, boundary incidence, and f32-converted coordinates.

**Unresolved inputs:** the live `makeRegionDetectionConfig()` result; PlaneGCS normal-sign implementations; the intended angular acceptance tolerance; minimum net-material-area policy; reader capability handling for endpoint Tangent.

**[N] Validation performed:** allowlisted source inspection and standalone Bun geometry/rank computations. No repository edits, native worker reproduction or new PlaneGCS constraint execution.