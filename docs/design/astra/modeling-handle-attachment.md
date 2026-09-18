# modeling-handle-attachment
date: 2026-09-17
mode: derive
model: gpt-6-astra
effort: xhigh
access: grounded — src/tools/preview/{handleProjection,filletRadius,faceOffset,shellThickness,alignSolve}.ts, src/viewport/mesh/faceEdges.ts, src/ipc/types.ts, src/viewport/engine/DragHandle.ts, src/tools/modelTools/ModelToolController.ts, protocol/SCHEMA.md (§7.6 + OffsetFace/Shell). Transcript audited: every read was inside this list.
packet: sha256 d778c09125a9
calls: 1 of ~3 for Boundary 2 (session 01a0b0aa-1482-7961-a64c-0e3629c97772 kept for a followup; a `break` is planned after H8–H10)
verdict: n/a (derive)
verified by: scratch recomputation of every load-bearing number — convex 90° blend midpoint displacement −(√2−1)q·b = −0.828427 mm at q = 2 mm; perspective inverse Δq = 8.93471 mm vs 9.622504 mm for a constant gain (FOV 60, g = 5.196152 px/mm, k = −0.008/mm, p = 50 px); off-centre P76 conditioning 0.775404 vs sin ∠ 0.612771 and maximum 1.265406; β = 0.5 blend overstating scalar response by exactly 1.2×; full rejection at a 1° screen angle multiplying gain by 57.2987. Repository claims checked in source: `MIN_CLIP_W = 1e-6` and the `pxPerWorld > 1e-9` absolute floors (handleProjection.ts:54, :156), `axisConditioning`'s `Math.min(c, 1)` clamp, `ClassifyFrame.sidedness?: "pin" | "hole"` (types.ts:719), and SCHEMA's `d = σ(distance − R)` / `d = σ(distance/2 − R)` with σ = sign(n_out·r̂) (protocol/SCHEMA.md §7.3 OffsetFace).
rejected: none of the derivation's findings. Two deliberate scope reductions by the orchestrator, recorded rather than rejected: (1) the full interval-arithmetic certification (γ7 dot-product bounds, a `numeric-uncertain` verdict) is not implemented in this frontend package — the refusal SHAPE is adopted (refuse when a denominator is not provably safe under relative guards), the machinery is not; (2) the open product question "keep a tangent-perpendicular appearance as a separate decoration" is answered NO — the glyph draws the true projected motion direction of the point that moves, and the edge may be highlighted separately.

## 1. Problem restatement

**[Inference — recommendation]** Define a parameter-dependent handle point H(q), draw its actual projected motion, and invert that same projection when dragging. Treat attachment provenance and witness meaning separately.

**Misframing:** absolute dimensions need a starting value, not a meaningful zero; material-outward is not always increasing-dimension motion; off-centre perspective conditioning is not generally a sine.

Labels below: **P** = proved mathematically; **N** = numeric computation; **S** = source-cited; **I** = proposed policy/inference.

**[S]** Two implementation details differ from the packet: `projectAxis` currently has absolute floors `c.w > 1e-6` and `pxPerWorld > 1e-9`; `axisConditioning` clamps its result to 1. These require correction before claiming general scale invariance. [handleProjection.ts](/Users/andrejvysny/workspace/OneCAD/src/tools/preview/handleProjection.ts:42)

## 2. Properties to guarantee

1. **[P] Pointer tracking:** inside the admissible, unclamped drag interval, the handle’s displacement along its drawn direction equals the pointer displacement projected onto that direction.
2. **[I] One mapping:** rendering, picking and sampling consume the same mapping object. Freeze its reference geometry, camera, coefficients, scale and direction at grab—not merely its strategy name.
3. **[P] Positive response:** positive sampled travel increases the bound scalar. Radius/Diameter use their own derivatives; material-normal orientation cannot reverse them.
4. **[I] Honest witnesses:** every witness declares `measured reference`, `target construction`, or `parameter construction`. A successful prepare does not establish that a newly requested value has already been built.
5. **[I] Honest attachment:** a mesh-derived attachment identifies a display location only. No helper producing a handle may produce persistent topology references.
6. **[P] Similarity invariance:** jointly transform geometry and camera by P′ = λRP + T, q′ = λq. Screen positions and classifications remain unchanged; gains in world units multiply by λ.
7. **[P] Unit invariance:** changing display units changes formatting only. If internal coordinates were converted, every length, uncertainty and dimensional bound must convert together.
8. **[I] Explicit refusal:** missing evidence, arithmetic uncertainty, poor screen sensitivity and an invalid operation value have distinct reasons.
9. **[I] Saturation exception:** domain/range limits stop tracking. Rebase at saturation so the first reversing sample changes the value; show the limit.

**[P/S] Qualification:** the mapping can satisfy similarity invariance; the complete operation domain cannot while retaining a fixed `0.1 mm` floor. Scaling `0.1 mm` by 0.1 produces `0.01 mm`, which is outside that domain. Likewise, the existing tessellation deflection clamps are not homogeneous under physical model scaling. [filletRadius.ts](/Users/andrejvysny/workspace/OneCAD/src/tools/preview/filletRadius.ts:53), [faceEdges.ts](/Users/andrejvysny/workspace/OneCAD/src/viewport/mesh/faceEdges.ts:40)

## 3. Predicates and epsilon derivation

### Tangent rejection: remove it from the motion mapping

**[I] Choice:** β = 0: do not reject the projected tangent from a moving handle’s derivative. Keep tangent information for edge location and optional decoration.

**[P] Reason:** let j be the derivative of the point that actually moves. A rejected direction generally differs from j, so that point cannot follow the pointer along the rejected direction. Full rejection is legitimate for a *quotient coordinate* that ignores motion along an edge; it is not the forward motion of the attachment point.

For a proposed blend:

```text
u = unit(projected tangent)
b = j − β(j·u)u
d = b/|b|
```

The scalar response along d is **j·d**, not generally |b|. Those coincide for β = 0 and full orthogonal rejection β = 1, but not for intermediate blends.

**[P] Error bounds for the chosen β = 0:**

- Tangent-induced direction error: **0 rad**.
- Tangent-induced gain multiplier: **1**.
- Direction is continuous wherever j is continuous and nonzero.
- Crossing tangent conditioning 0.08 causes no change.
- Screen-parallel tangent/outward directions remain usable if j itself is usable.

**[I]** A handle moving along an edge’s screen image can be visually awkward. Label the scalar clearly; this is not a numerical failure.

### Projection and gain

**[P]** Use a linear world path:

```text
H(q) = H0 + v(q − q0)
v = dH/dq                         // world mm per scalar mm
j = J(H0)v                       // CSS px per scalar mm
g = |j|
d = j/g
s = worldPerPx(H0)
Cq = g·s
```

Accept a world mapping when the numerically certified lower bound of **Cq ≥ τ**, with **τ = 0.08**.

This measures **scalar sensitivity**, including semantic factors such as Diameter’s v = r̂/2. It bounds the initial scalar gain:

```text
dq/dp = 1/g ≤ s/τ = 12.5s
```

**[I]** Retain 0.08 as the existing usability budget. Its value is not derived from floating-point accuracy or OCCT tolerance.

**[P]** A constant `p/g` mapping is exact only for orthographic projection or constant-depth motion. For perspective, freeze:

```text
c0 = M[H0,1]
dv = M[v,0]
k = dv.w/c0.w

p = g·Δq / (1 + k·Δq)
Δq = p / (g − k·p)
dq/dp = g / (g − k·p)² > 0
```

Thus a frozen strategy can track exactly without a constant gain.

Reject samples outside the connected valid interval containing Δq = 0:

- camera/frustum visibility permits the world point;
- `1 + kΔq > 0`;
- the inverse denominator is certified positive;
- the frozen sensitivity envelope remains acceptable.

For a standard perspective camera:

```text
s(q) = s0(1 + kΔq)
Cq(q) = Cq(0)/(1 + kΔq)
```

**[I]** Compute this usable interval at grab. Reaching it produces a “View limits this drag” stop, not a mid-gesture switch to proxy or a new parameter-domain ceiling.

### Conditioning is not generally a sine

**[P]** In camera coordinates, with depth D > 0, focal length f in CSS pixels and s = D/f:

```text
j·s = [ ax + (x/D)az, −ay − (y/D)az ]
```

Its nonzero singular values are:

```text
1, sqrt(1 + (x/D)² + (y/D)²)
```

Therefore conditioning can exceed 1. Equality with a sine holds for orthographic projection and the central perspective ray, not arbitrary viewport positions.

**[I]** Log the unclamped value. Clamping to 1 does not change the existing 0.08 decision, but conceals the actual sensitivity.

### Predicate and tolerance policy

| Decision | Predicate / epsilon | Scaling and interpretation |
|---|---|---|
| **[I] Tangent rejection** | β = 0; no tangent threshold | Dimensionless; similarity/unit invariant |
| **[I] World versus screen mapping** | Certified `g·s ≥ 0.08` | Dimensionless scalar sensitivity; invariant |
| **[I] Numerical validity** | Finite inputs; interval-certified denominators and nonzero vectors | Arithmetic error bounds, not fixed mm/px floors |
| **[I] Geometric attachment** | Same-snapshot analytic primitive + known parameter-motion law + supported reference location | Evidence predicate; no tolerance can replace missing evidence |
| **[I] Parameter construction** | Finite, attributable reference point and certified construction direction | Geometry-derived or mesh-derived; explicitly labelled construction |
| **[I] Stable UI proxy** | No supported world construction, or world mapping refused | Up increases q; gain `s mm/CSS px` from an identified valid depth |
| **[I] No usable depth** | No finite positive scale from the selected body/reference | Fixed UI numeric control; disable drag instead of inventing origin/depth |
| **[S/I] Offset normal divergence** | Each `atan2(|ni×m|, ni·m) ≤ π/4`; certify nonzero mean | Angular, dimensionless; invariant |
| **[S/I] Mesh planarity hint** | Maximum normal deviation ≤ `1e-3 rad` | Angular only; never proves an analytic plane |
| **[S/I] Mesh adjacency hint** | Point-to-triangle distance ≤ `2δ + arithmetic error` | Positional display uncertainty; δ follows actual mesh deflection policy |
| **[I] Analytic dimension agreement** | Same-snapshot authoritative certificate; otherwise certified arithmetic consistency | Do not invent a kernel-length epsilon when its tolerance is absent |

**[P] Arithmetic epsilon derivation:** for IEEE double unit roundoff u = 2^-53, a four-term dot product has the conservative bound:

```text
E_dot = γ7 Σ|xi yi|
γ7 = 7u/(1−7u) = 7.771561172376102e−16
```

Propagate such bounds through subtraction, division and normalization. Unresolved signs produce `numeric-uncertain`; an adaptive exact sign evaluation may resolve them. Exact arithmetic over supplied numbers cannot recover uncertain mesh geometry.

If derivative uncertainty is bounded by Ej < g:

```text
direction error ≤ asin(Ej/g)
relative reciprocal-gain error ≤ (Ej/g)/(1−Ej/g)
```

**[P]** These bounds scale with their operands and respect unit conversion. Fixed `1e-6` clip-w and `1e-9 px/mm` floors do not.

**[S/P]** Existing mesh deflection is `clamp(relative·bboxDiagonal, minMm, maxMm)`. Its positional tolerance must convert with units; it is similarity-homogeneous only between its clamps. Angular planarity is separate: over arc length ℓ on radius R, normal rotation is ℓ/R. Neither tolerance establishes kernel topology. [faceEdges.ts](/Users/andrejvysny/workspace/OneCAD/src/viewport/mesh/faceEdges.ts:40), [alignSolve.ts](/Users/andrejvysny/workspace/OneCAD/src/tools/preview/alignSolve.ts:57)

## 4. Degeneracy classes

| Class | Detection | Required behaviour |
|---|---|---|
| **[I] End-on manipulation** | `Cq < 0.08`, or projected derivative uncertifiable | Labelled vertical proxy |
| **[I] Near-end-on tangent** | Tangent conditioning < 0.08 | No mapping effect; tangent decoration may disappear |
| **[P/I] Screen-parallel tangent/outward** | Projected cross product zero/uncertain | No rejection; retain usable raw motion |
| **[I] Behind/on camera plane** | Perspective clip-w nonpositive/uncertain; orthographic visibility requires depth/frustum test | No world glyph; stable UI control |
| **[I] Offscreen/occluded attachment** | Outside viewport, or visibility unsupported | UI locator/proxy; never imply a visible surface contact |
| **[I] Concave edge** | Convexity not certified by available evidence | Parameter witness; do not infer blend displacement from “outward” |
| **[I] Tangent-continuous chain** | More than one prepared edge | One deterministic representative; label shared parameter and edge count |
| **[I] Curved edge** | Local tangent varies along polyline/analytic curve | Attach at one local edge point; never average tangents into a global direction |
| **[I] Missing/free/seam/non-manifold edge support** | Adjacent-face count other than two, or missing evidence | No two-support geometric fillet/chamfer claim |
| **[S/I] Divergent multi-face Offset** | Invalid normals, cancelling mean, or deviation > π/4 | Proxy; preserve existing refusal |
| **[I] Inner/outer cylinder** | Classification `sidedness === hole/pin` | Radius/Diameter always grow radially outward; signed Offset/Shell use sidedness |
| **[P/I] Cylinder axis nearly view-parallel** | Axis projection small | Not itself a degeneracy; test the radial manipulation derivative |
| **[I] Radial anchor unresolved** | `|(I−zzᵀ)(P−O)|` cannot be certified nonzero | Try another actual face sample; otherwise proxy |
| **[I] Zero/near-zero scalar** | Existing domain invalid, or witness below screen resolution | Preserve domain policy; never normalize by q or inflate a witness’s world length |
| **[I] Shell retained wall ambiguous** | No retained rim neighbour, multiple indistinguishable matches, missing orientation | Construction downgrade or proxy |
| **[I] Re-edit lacks predecessor evidence** | Current geometry cannot establish original reference | UI parameter control; no origin fallback |
| **[I] Stale/inconsistent evidence** | Snapshot/generation mismatch or conflicting primitives | Discard attachment; refresh outside frame loop |
| **[I] Perspective drag limit** | Frozen valid interval exhausted | Hold/rebase; keep strategy, explain view limit |

**[S]** Cylinder sidedness is already available and explicitly distinguishes `pin`, `hole`, and unmeasured. [types.ts](/Users/andrejvysny/workspace/OneCAD/src/ipc/types.ts:699)

## 5. Algorithm sketch

All proposed attachment choices below are **[I]**; derivatives and dimension identities are **[P]**.

### Fillet / Chamfer

Use a **parameter construction** for the general case.

1. Choose the first user-picked prepared edge in deterministic prepared order; otherwise the first prepared edge.
2. Let E be the closest point on its display polyline to its pick anchor; without an anchor, use the polyline’s arc-length midpoint.
3. Use that edge’s local resolved outward direction b. Do not average a closed chain’s directions or attach to an off-contour mean.
4. Define:

```text
H(q) = E + q b
dH/dq = b
```

The handle moves as q changes. Its witness E→H is labelled **“Radius parameter”** or **“Chamfer distance parameter”**; for a chain, append **“shared by N edges.”**

This segment measures the construction’s length q. It does **not** measure the resulting fillet’s radius from a centre, the chamfer’s bevel width, or a displacement of the edge.

Fallback:

```text
local outward → bbox-derived construction direction → anchored screen proxy
→ body-associated UI control → numeric-only
```

A bbox direction always carries construction provenance.

**[P] Why a generic geometric fillet attachment is unavailable:** for a certified right-angle convex corner, the blend midpoint moves by

```text
ΔX = −(sqrt(2)−1) q b
```

For the corresponding concave corner, its sign reverses. Neither is `+q b`. Adjacent-face identities alone do not certify the chosen blend section, convexity or resulting trimmed surface.

**[S]** Prepare supplies contour and adjacency evidence, not a resulting blend witness. Preserve the existing range-confidence clamp and never use `provenUpperBound` as a ceiling. [types.ts](/Users/andrejvysny/workspace/OneCAD/src/ipc/types.ts:1877), [filletRadius.ts](/Users/andrejvysny/workspace/OneCAD/src/tools/preview/filletRadius.ts:129)

### Shell

Attach a **thickness construction to a retained rim wall**.

For a fresh arm:

1. Find display boundary edges of removed faces.
2. Find retained faces sharing those edges; exclude every removed face.
3. Discard ambiguous adjacency matches.
4. Choose the candidate rim point nearest the picked removed-face anchor; tie-break by snapshot face/edge ordinal.
5. Prefer a supported planar wall. Let E be its rim point and n its outward normal:

```text
H(t) = E − t n
dH/dt = −n
```

For the open box, this selects one of the four side walls. The floor is not a rim neighbour of the removed top.

Witness: **“Thickness target t — construction.”** It spans the reference wall and its inward offset plane. It does not certify the actual shelled wall or its trimmed extent.

For a classified cylindrical retained wall:

```text
z = unit(axis)
C = O + z[z·(E−O)]
r̂ = unit(E−C)
σ = +1 for pin; −1 for hole

H(t) = C + (R − σt) r̂
dH/dt = −σ r̂
```

Require measured sidedness. A nonpositive predicted radius cannot be depicted as an inner cylindrical wall; retain only an explicitly labelled parameter control and let existing preview/domain validation decide the operation.

Fallback: supported retained wall → attributable mesh construction → picked-anchor proxy → body-associated UI control. Re-edits without predecessor wall evidence take the latter paths.

**[S]** Shell offsets inward; its frontend minimum and worker minimum are distinct existing policies. No supplied Shell result establishes a measured wall pair. [SCHEMA.md](/Users/andrejvysny/workspace/OneCAD/protocol/SCHEMA.md:2337), [shellThickness.ts](/Users/andrejvysny/workspace/OneCAD/src/tools/preview/shellThickness.ts:16)

### Offset: signed `Offset`

For one supported planar face, with reference point P and material-outward n:

```text
H(q) = P + q n
dH/dq = n
```

For one cylinder with known sidedness:

```text
H(q) = C + (R0 + σq) r̂
dH/dq = σ r̂
```

Witness: signed offset from the frozen reference surface, labelled as a **target** until validated.

For multiple faces, keep the existing divergence predicate. Use the mean-axis handle as a **shared parameter construction**:

```text
A = mean(reference points)
a = offsetAxisFor(normals)
H(q) = A + q a
```

Do not call this the moving centroid.

**[S/I] Additional restriction:** V3 closures can contain rebuilt blends and fixed supports. The frontend lacks their motion-role partition, so generic “every closure face moves by q” witnesses are unjustified. [SCHEMA.md](/Users/andrejvysny/workspace/OneCAD/protocol/SCHEMA.md:2413)

### Offset: `Total`

Require accepted Total preparation, its persisted opposite, t0, and same-snapshot planar classification.

Orient n from the opposite plane toward the selected plane; certify the sign. For supported selected reference point P:

```text
B = P − t0 n
H(T) = B + T n
dH/dT = n
```

No opposite-plane normal field is required: classify the opposite face through the existing call.

Witness:

- reference P↔B: prepared **reference thickness t0**;
- B↔H(T): **target Total T**.

Use plane-extension marks when finite-face attachment is unestablished. Never silently portray an arbitrary plane origin as a point inside the trimmed face.

**[S]** Accepted Total preparation establishes unique opposite coverage and material-column validation. [SCHEMA.md](/Users/andrejvysny/workspace/OneCAD/protocol/SCHEMA.md:4444)

### Offset: `Radius` / `Diameter`

From a classified cylinder axis `(O,z)` and supported sample P:

```text
C = O + z[z·(P−O)]
r̂ = unit(P−C)

Radius:   H(R) = C + R r̂       dH/dR = r̂
Diameter: H(D) = C + D/2 r̂     dH/dD = r̂/2
```

Therefore:

```text
Radius gain   = 1/|Jr̂|
Diameter gain = 2/|Jr̂|
```

These signs are identical for inner and outer cylindrical walls. Sidedness is needed to explain the material-normal offset, not to define increasing radius.

Witness:

- Radius: centreline C to radial target H(R);
- Diameter: supporting-cylinder diameter construction, labelled ØD. Unless both finite surface endpoints are established, do not depict it as a measured span between actual walls.

**[S]** The worker already defines `d = σ(R−R0)` or `σ(D/2−R0)`. Absolute dimensions therefore have an explicit drag reference. [SCHEMA.md](/Users/andrejvysny/workspace/OneCAD/protocol/SCHEMA.md:2462)

### Work placement and complexity

**[I]** At arm: snapshot-fenced classification, mesh attachment selection and provenance. Per frame/grab: O(1) projection and witness evaluation for the chosen handle. No per-frame classification or mesh scan.

**[S/P]** The current mesh boundary helper tests polyline points against face triangles. Across queried faces its worst-case work is O(QT), where Q is edge-polyline point count and T is triangle count; cache results per mesh generation. [faceEdges.ts](/Users/andrejvysny/workspace/OneCAD/src/viewport/mesh/faceEdges.ts:85)

## 6. Test vectors

**[N] Provenance:** all new expected numbers below follow from the displayed formulas; these are computed vectors, not executed repository tests.

Cameras, all with **1000×1000 CSS px**:

- **O:** orthographic, eye `(0,0,100) mm`, looking toward origin, screen-right +X, screen-up +Y, view height `100 mm`.  
  `screen(P)=(500+10Px,500−10Py)`; `s=0.1 mm/px`.
- **B:** orthographic, eye `100(1,1,1)/√3 mm`, looking toward origin; right `(-1,1,0)/√2`, up `(-1,-1,2)/√6`; view height `100 mm`.  
  `j(a)=10(right·a,−up·a)`; `s=0.1 mm/px`.
- **P60/P76:** perspective, eye `(0,0,100) mm`, looking down −Z, up +Y; FOV 60°/76°, near `0.1 mm`, far `10000 mm`.

| Case | Inputs and computation | Expected |
|---|---|---|
| Axis threshold | O; `a=(c,0,√(1−c²))`, c=`0.0799 / 0.0801`; `g=10c` | Proxy / axis. Axis gain `1/0.801=1.24844 mm/px`. Mathematical boundary is c=0.08; unresolved arithmetic ties refuse |
| End-on axis | O; a=+Z; j=0 | Proxy, +10 px upward ⇒ +1 mm |
| Near-end-on tangent | O; a=+X; tangent tilt `0.01°, 0.5°, 2°`, azimuth 45° | Tangent conditioning `0.000174533, 0.00872654, 0.0348995`; all retain j=(10,0), +10 px ⇒ +1 mm |
| Tangent threshold | Same; tangent conditioning `0.0799 / 0.0801` | Identical handle direction/gain on both sides |
| Projected parallel, world perpendicular | O; `a=(1,0,1)/√2`, `t=(1,0,−1)/√2` | Both project along +X; `g=7.07107`; +10 px ⇒ +1.41421 mm |
| Behind camera | P60; H0=(0,0,101) mm, depth −1 mm | No world glyph; UI fallback |
| Convex box edge | B; E=(40,40,20) mm, b=(1,1,0)/√2 | `g=10/√3=5.77350 px/mm`; +10 px ⇒ +1.73205 mm |
| Radius range | Same; q=`0.1,2,20 mm` | Construction travel from E=`0.577350,11.5470,115.470 px`; q=2 gives H=(41.4142,41.4142,20) mm |
| Concave corner | B; E=(0,0,20) mm, same b into free space | Same parameter gain; construction label retained; no convex midpoint claim |
| Four-edge tangent chain | O; four quarter-circle edges, radius 20 mm, z=20 mm; first representative at 45° | E=(14.1421,14.1421,20); q=2 gives H=(15.5563,15.5563,20) mm. Motion 20 px; no cancelling mean |
| Curved edge | O; circle radius 20 mm; representative P=(20,0,20), b=+X; local tangent +Y | q=2 moves 20 px along +X; tangent variation elsewhere has no effect |
| Open-box Shell | B; chosen rim E=(40,20,40) mm, n=+X, t=2 mm | H=(38,20,40); displacement `(14.1421,−8.16497) px`, length 16.3299 px |
| Curved Shell wall | O; cylinder R=10 mm, axis Z, E=(10,0,40), σ=+1, t=2 | H=(8,0,40) mm; 20 px inward; witness is thickness construction |
| Signed cylinder Offset | O; inner tube R=6 mm, σ=−1, q=+2 mm | H radial coordinate 4 mm; 20 px toward axis |
| Total plate | O; selected plane x=4 mm, opposite x=0; P=(4,0,20), T=6 | H=(6,0,20); +20 px. Reference thickness 4 mm; target 6 mm |
| Outer Radius | O; R0=10 mm, r̂=+X; +20 px | R=12 mm; kernel signed offset +2 mm |
| Inner Radius/Diameter | O; R0=6 mm, σ=−1; +20 px | Radius 6→8 mm; Diameter 12→16 mm; signed offset −2 mm |
| Cylinder axis end-on | O; cylinder axis Z, r̂=+X | Axis conditioning 0 is irrelevant; Radius Cq=1, Diameter Cq=0.5 |
| Radial direction end-on | O; cylinder axis X, r̂=+Z | Radius/Diameter proxy despite visible cylinder axis |
| Diameter threshold | O; radial conditioning `0.1598 / 0.1602` | Scalar conditioning `0.0799 / 0.0801`: proxy / axis |
| Divergent normals | O; normals symmetric about +X at `±44.99° / ±45.01°` | Accepted / refused by π/4 policy |
| Cancelling normals | O; normals +X and −X | Proxy; normalization forbidden |
| Zero/unresolved radius | O; radial sample P=C | No radial normalization; another real sample or proxy |
| Near-zero value | B; fillet q=0.1 mm | Valid floor; witness projects to 0.577350 px. Keep true length; enlarge glyph only |
| Planarity threshold | Two facet normals at `±0.000999 / ±0.001001 rad` from mean | Mesh hint accepted / refused |
| Curved-but-nearly-planar patch | Cylinder R=1000 mm, arc ±0.5 mm | Maximum normal deviation `0.5/1000=0.0005 rad`; mesh hint can pass, classification must still say cylinder |
| Missing/stale reference | Re-edit with no predecessor anchor, or mismatched snapshot | UI parameter control; no world-origin anchor |
| Exact perspective drag | P60; H0=0, v=(0.6,0,0.8); `g=5.196152`, `k=−0.008/mm`, p=50 px | `Δq=50/(5.196152+0.4)=8.934710 mm`; constant gain wrongly gives 9.622504 mm |
| Perspective sensitivity limit | Same; Cq0=0.6 | Depth factor ≤`0.6/0.08=7.5`; Δq≥−812.5 mm; stop at p=−562.916512 px |
| Unit conversion | O; +10 px ⇒ +1 mm | `1/25.4=0.0393701 in`; strategy and screen geometry unchanged |

**[N] Off-centre perspective check:** P76, depth 100 mm, NDC `(0.75,0.65)` gives:

```text
f = 500/tan(38°) = 639.970816 px
P = (58.596422, 50.783566, 0) mm
cond(+Z) = sqrt(0.585964² + 0.507836²) = 0.775404
sin(angle to view ray) = 0.612771
maximum conditioning = 1.265406
```

**[S/I]** The packet’s measured `cond(+X)=0.077` therefore selects proxy. Reproducing that particular measurement requires its camera orientation and anchor depth, which the packet does not specify.

## 7. Counterexample search

### False fillet measurement

**[N/P]** For the convex 90° box edge at E=(40,40,20), r=2 mm:

```text
true local circular centre = (38,38,20)
true blend midpoint = (39.414214,39.414214,20)
midpoint displacement = −0.828427 b mm

parameter handle = (41.414214,41.414214,20)
parameter displacement = +2b mm
```

Under camera B, those motions are respectively **−4.78293 px** and **+11.54701 px**.

**[I] Result:** claiming the parameter handle is a fillet contact point visibly lies. The proposed construction label prevents that claim. A true blend attachment needs additional certified witness evidence.

### A continuous rejection blend with the old gain

**[N/P]** Let j=(10,0), tangent direction `(1,1)/√2`, β=0.5:

```text
b = (7.5,−2.5)
|b| = 7.905694
direction angle = −18.434949°
j·unit(b) = 9.486833
```

Using `1/|b|` instead of `1/(j·unit(b))` overstates scalar response by **1.2×**. Even the corrected scalar response does not make the actual point move along b.

**[N/P]** Full rejection at a 1° screen angle rotates the direction by 89° and multiplies gain by `1/sin(1°)=57.2987`. A well-conditioned tangent does not prevent this.

**[I] Result:** β=0 eliminates both failures.

### Mesh adjacency falsely identifies a wall

**[N/S]** A 40 mm cube has diagonal `40√3=69.282032 mm`. At fine LOD:

```text
δ = 0.0005·69.282032 = 0.034641016 mm
adjacency tolerance = 2δ = 0.069282032 mm
```

An unrelated overlapping face **0.05 mm** away can pass the same distance test.

**[I] Result:** reject detectable ambiguity. A unique mesh match still is not a BRep proof; retain construction provenance. A claim of measured Shell thickness would remain unsupported.

### Inner cylinder sign inversion

**[N/P]** Inner radius 6→8 mm moves +2r̂, while material outward is −r̂:

```text
d = σ(R−R0) = −1·2 = −2 mm
```

**[I] Result:** using material outward as increasing Radius would invert the drag. Using r̂ for Radius and r̂/2 for Diameter prevents it.

## 8. Implementation handoff

**[I] Suggested pure types and signatures:**

```ts
type Vec2 = readonly [number, number];
type Vec3 = readonly [number, number, number];

type WitnessMeaning =
  | "measuredReference"
  | "targetConstruction"
  | "parameterConstruction";

interface LinearHandlePath {
  readonly q0Mm: number;
  readonly point0Mm: Vec3;
  readonly dPointDValue: Vec3;
}

interface HandleEvidence {
  readonly snapshotId: number;
  readonly attachment: "geometric" | "construction" | "ui";
  readonly witness: WitnessMeaning;
  readonly source: "classification" | "prepare" | "mesh" | "selection";
}

type FrozenMapping =
  | {
      readonly kind: "world";
      readonly direction: Vec2;
      readonly q0Mm: number;
      readonly g0PxPerMm: number;
      readonly kPerMm: number;
      readonly validDeltaMm: readonly [number, number];
    }
  | {
      readonly kind: "proxy";
      readonly direction: readonly [0, -1];
      readonly mmPerPx: number;
    }
  | {
      readonly kind: "disabled";
      readonly reason: string;
    };

function edgeParameterPath(anchor: Vec3, outward: Vec3): LinearHandlePath;
function shellPath(reference: Vec3, outward: Vec3): LinearHandlePath;
function totalPath(reference: Vec3, normal: Vec3, thicknessMm: number): LinearHandlePath;
function radiusPath(axisPoint: Vec3, radial: Vec3, diameter: boolean): LinearHandlePath;

function classifyMapping(
  path: LinearHandlePath,
  camera: ProjectionContext,
  scale: AnchorScale,
): FrozenMapping;

function sampleMapping(
  mapping: FrozenMapping,
  displacementPx: Vec2,
): DragSample;
```

`ProjectionContext`, `AnchorScale` and `DragSample` should carry certified validity/refusal information; no numeric sentinel for an unavailable scale.

| Location | Responsibility |
|---|---|
| **[I] `handleProjection.ts`** | Arithmetic filters, exact derivative, scalar conditioning, perspective inverse and frozen valid interval; remove tangent rejection from motion |
| **[I] `filletRadius.ts`** | Parameter construction; retain existing domain/range policy |
| **[I] `shellThickness.ts`** | Retained-wall construction from an already resolved display frame |
| **[I] `faceOffset.ts`** | Signed, Total, Radius and Diameter paths; preserve divergence refusal |
| **[I] Controller arm** | Classification calls, snapshot checks, representative/rim selection, evidence provenance |
| **[I] Controller grab** | Freeze complete displayed mapping and current q; no second ray-based mapping |
| **[I] Controller sample** | Invert frozen mapping; apply existing value policy; rebase stops |
| **[I] Renderer** | Draw/pick from the same packet; witness semantics and proxy styling; no geometry inference |

**[S]** Current `DragHandle.freezeStrategy` freezes only the enum and continues recomputing projections; Offset drag also has a separate ray/axis path. Both seams need alignment with the shared mapping. [DragHandle.ts](/Users/andrejvysny/workspace/OneCAD/src/viewport/engine/DragHandle.ts:251), [ModelToolController.ts](/Users/andrejvysny/workspace/OneCAD/src/tools/modelTools/ModelToolController.ts:8779)

**[I] Log once per arm/decision change**, not every frame:

```text
operation, distanceType, snapshot/generation, attachment source,
representative display entity, witness meaning,
Cq, g, s, cylinder sidedness, strategy, refusal reason
```

Diagnostics and UI:

- `poor-screen-sensitivity`: **“Radius: drag vertically — axis nearly end-on.”**
- `construction-only`: **“Thickness target — construction.”**
- `mesh-ambiguous` / `missing-reference`: **“Thickness: drag vertically or type — wall attachment unavailable.”**
- `projection-view-limit`: **“View limits this drag; release and reposition the view.”**
- `numeric-uncertain` / `stale-evidence`: disable that world attachment; keep valid numeric editing available.

## 9. Confidence

**High:** projection inversion, dimensional gains, inner-cylinder sign, tangent-rejection counterexamples, and the distinction between measured reference and target construction.

**Medium:** mesh-based Shell rim selection and local edge attachment. They improve placement without certifying topology or final wall/blend geometry.

**Missing evidence that would change the answer:**

- **Fillet/Chamfer measured witness:** a snapshot-bound result witness identifying its quantity and supporting geometry—e.g. certified blend centre/section point for radius, or reference-face setback endpoints for chamfer. Without it: parameter construction.
- **Shell measured witness:** certified retained reference wall plus corresponding inner-wall sample, oriented normal, measured separation and snapshot. Without it: retained-wall thickness construction.
- **V3 multi-face geometric motion:** per-face role and motion evidence for moving design faces, fixed supports and rebuilt blends. Without it: shared parameter control.
- **Geometric tolerance certificates:** bounds accompanying independently obtained analytic geometry if approximate agreement must be accepted. Without them: no invented kernel epsilon.
- **Exact reproduction of the supplied 0.077 result:** camera pose and anchor coordinates/depth.

**[I]** None of those additions is required for the proposed construction/proxy policy. The unresolved product choice is whether tangent-perpendicular *appearance* is important enough to retain as a separately labelled screen construction; it cannot simultaneously represent the moving point’s true projected direction.