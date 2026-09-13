# Numerical algorithms and mesh protocol contracts

**Authority:** Normative companion to [the specification](01-SPECIFICATION.md).  
**Status:** These are implementation designs and derivations, not executed OneCAD code.  
**Abbreviation:** References to `NUM §n` mean a section of this document.

## 1. Shared numerical rules

### 1.1 Different tolerances must have different names

Never introduce a single global `EPSILON` for all geometry behavior.

| Quantity | Purpose | Must not be used for |
|---|---|---|
| Kernel tolerance | Existing BRep validity/operation semantics | Pixel picking, line width |
| Display chord tolerance | Approximation of a curve/surface | Constraint satisfaction |
| Normal angular tolerance | Shading-vector error or continuity classification | Deciding model topology |
| Roundoff bound | Floating-point calculation uncertainty | Hiding a real gap |
| Acquisition radius | Pointer convenience in CSS pixels | Depth visibility |
| Closure tolerance | Existing sketch topology contract | Forcing a visually nearby open curve closed |
| Approximation uncertainty | Whether rendered samples resolve an occlusion | Persistent reference rebinding |

New dimensionless constants and safety factors below are specified policy choices. Put each in one named policy module with units, rationale, and tests. When an existing kernel tolerance is needed, read the authoritative per-shape/per-entity tolerance; do not manufacture a new modeling tolerance.

All math helpers reject NaN, infinity, zero/negative sizes where invalid, and degenerate bases. Return a typed failure reason. Do not replace invalid vectors with an arbitrary +Z direction except where +Z is actually the defined plane normal.

### 1.2 Rounding and validation

Calculate bounds and origins in float64. Cast to float32 only at the representation boundary. Compute `quantizationErrorMm` by comparing each written float32 position with the original double position; report the maximum Euclidean discrepancy across surface and edge points. This is a measured representation error, not a guessed ULP multiple.

Normalize emitted normals after all transforms. Validation accepts finite normal lengths in `[0.999, 1.001]`; producer values outside this range are a production bug or a legacy input requiring a documented normalization repair. Zero-length normals are never silently normalized.

For arithmetic decision tests in double precision, use a scale-dependent roundoff estimate such as `64 * Number.EPSILON * max(1, localMagnitude, rayDistance)`. This is an engineering bound for these computations, not a formal interval-arithmetic proof for arbitrary OCCT operations. Tests must include sensitivity around the boundary.

### 1.3 Meaning of error evidence

Use separate fields for `requestedTolerance`, `kernelEstimatedError`, `sampledMaxError`, and `certifiedUpperBound`. Only populate the last when the construction actually supplies a bound. Dense testing can falsify a claimed bound but cannot prove an arbitrary curve stayed inside it between samples.

This distinction is essential for OCCT surface tessellation and approximate procedural curves. The UI may say "display quality limited"; it must not promise exact geometric accuracy from a mesher's input setting.

## 2. Curve sampling — selected algorithm

### 2.1 Curve-family dispatch

1. **Finite line:** emit exact endpoints, unless degenerate; preserve orientation.
2. **Circle/ellipse/conic:** convert a trimmed finite interval into exact rational quadratic Bézier spans of at most 90° where applicable. Keep periodic seam and endpoint identity. Exact OCCT conic-to-B-spline conversion may supply these spans.
3. **Polynomial/rational Bézier:** process trimmed homogeneous control points directly.
4. **B-spline/NURBS:** split at knots/continuity intervals and convert each trimmed span through `GeomConvert_BSplineCurveToBezierCurve`; keep original parameter intervals and orientation. Verify periodic-interval handling explicitly. [EXT-03.]
5. **Offset/procedural curve not exactly convertible:** use the approximate path in §2.7. Do not recursively sample only midpoint and endpoint tangents.

Apply the edge's location transform to double coordinates once. Either operate in local coordinates with a proven scale bound or in transformed double coordinates; the selected initial implementation operates in transformed double coordinates before subtracting the resource origin. Avoid cast-to-float during subdivision.

### 2.2 Positive-weight rational Bézier representation

For degree `n`, poles `P_i` and positive weights `w_i`:

```text
H_i = (w_i P_ix, w_i P_iy, w_i P_iz, w_i)
X(t) = Σ B_i^n(t) (w_i P_i)
W(t) = Σ B_i^n(t) w_i
C(t) = X(t) / W(t)
```

Validate finite weights and a positive denominator. Normalize all weights by their maximum when useful; this does not change the curve. If weight conditioning threatens safe evaluation, subdivide and/or return an explicitly uncertified result. Do not clamp individual weights, which would change the curve.

Perform de Casteljau subdivision in homogeneous coordinates at `t=0.5`. Preserve the shared split point exactly in the emitted sequence; calculate it once and pass it to both children. Use an iterative stack to avoid uncontrolled recursion.

### 2.3 Chord error bound

Let `A=P_0` and `B=P_n` for the current span. For every dehomogenized control point compute distance to the **finite segment** `[A,B]`, not merely its infinite line:

```text
v = B-A
u = clamp(dot(P-A, v)/dot(v,v), 0, 1)
d(P,[A,B]) = norm(P-(A+u*v))
bound = max_i d(P_i,[A,B])
```

When the chord is degenerate, use distance to `A`.

With positive rational weights the curve lies in the convex hull of the poles. Distance to a convex segment is a convex function, so every curve point is within `bound` of the segment. Because a continuous curve connects A to B, its scalar projection along the chord covers the interval; this also bounds segment-to-curve distance when the whole curve is contained in that tube. This is the reason to use a hull criterion instead of a sampled midpoint.

Accept the chord only if `bound + representationAllowance <= requestedChordTolerance`, and the angular rule passes or reports a separately permitted singular-endpoint status. Keep the analytic endpoints even for a closed span; a closed circle must first be split into nondegenerate spans.

### 2.4 Angular criterion, including rational curves

For a polynomial Bézier, derivative poles are `n*(P_(i+1)-P_i)`. A sufficient tangent cone can be checked against these poles.

For a rational span, use the derivative numerator:

```text
Q(t) = X'(t) W(t) - X(t) W'(t)
C'(t) = Q(t) / W(t)^2
```

Represent `Q` in Bernstein form of degree `2n-1`. Multiplying Bernstein polynomials of degrees `a` and `b` uses:

```text
c_k = Σ_(i+j=k) [choose(a,i)*choose(b,j)/choose(a+b,k)] * a_i * b_j
```

Here the symbols `a_i`, `b_j` denote scalar/vector coefficients, not degrees. Compute `X'W` and `XW'` separately at the same degree, then subtract. Unit tests must compare this representation with an independently evaluated quotient derivative at random parameters.

For nondegenerate chord direction `d`, require every significant derivative-numerator pole `q_i` to lie in the forward cone:

```text
parallel = dot(q_i,d)
perpendicular = norm(q_i - parallel*d)
parallel > roundoffBound
perpendicular <= tan(angularTolerance/2)*parallel
```

The cone is convex, so all nonzero derivatives lie within half the specified angle of the chord, bounding their total mutual angle by `angularTolerance`. Scale coefficients before testing to avoid overflow/underflow.

Zero derivative at a span endpoint can be a valid stationary endpoint. Never assert a defined tangent there. Subdivide and classify an unresolved singularity as `angular-undefined`, keeping a certified chord bound if available. Interior cusps/turnarounds must split and remain explicit; do not accept a long chord across one merely because endpoint tangents agree.

### 2.5 Subdivision algorithm and limits

```text
stack = exact Bézier spans in reverse traversal order
output = [first exact endpoint]
while stack not empty:
    check cancellation and global output budgets
    span = pop()
    compute chord hull bound and derivative cone
    if accepted:
        append span.last endpoint once
        record leaf parameter interval, error bound, angular status
    else if limits reached:
        return QUALITY_LIMITED with last valid resource retained by caller
    else:
        left,right = homogeneous midpoint split(span)
        push(right)
        push(left)
```

Initial limits: depth 32 per exact source span; 262,144 emitted segments per edge; 2,000,000 edge segments per body, subject to stricter byte admission. A limit is not an acceptance condition. If a replacement cannot meet the requested error, the caller can retain the previous valid display with a quality-limited status. A coarse new initial display may be shown only with explicit actual bound/status.

Do not pass a global changed-body budget to each edge as if each independently owns the full budget. Admission tracks running totals. Check cancellation after each span and after bounded batches of leaves.

### 2.6 Required counterexample

```text
C(t) = (t, 128*t²*(t-0.5)*(t-1)², 0), 0<=t<=1
Degree-five poles:
(0.0,  0.0, 0)
(0.2,  0.0, 0)
(0.4, -6.4, 0)
(0.6,  6.4, 0)
(0.8,  0.0, 0)
(1.0,  0.0, 0)
```

Endpoints/midpoint lie on the chord, and endpoint tangents agree. The old test accepts a straight line. At `t=0.25`, the deviation is 1.125 mm. New hull logic must reject the original span for a 0.05 mm budget. The test independently evaluates the polynomial, not the new sampler's own sample function.

### 2.7 Fixed approximate fallback

For an unsupported exact curve family:

1. Keep its finite trim and continuity intervals.
2. Ask `GeomConvert_ApproxCurve` for a B-spline approximation using a maximum positional allowance of one quarter of the requested display chord tolerance. Preserve the returned error estimate and completion status. [EXT-05.]
3. Require a successful result, finite bounded domain, and reported estimate no greater than the allowance.
4. Process the resulting Bézier spans using the hull algorithm with the remaining three-quarter budget.
5. Independently test the original curve at Chebyshev-distributed and uniform points per original interval; recursively add samples where derivative/curvature or deviation changes sharply, within evaluation limits.
6. Mark the combined result `kernel-estimated`, not `certified`, unless the approximation provider actually supplies a certified bound. Sample failures reject the requested result.

If approximation fails, emit a typed tessellation diagnostic. Do not fabricate a straight chord and label it valid. This preserves support for broad CAD imports without pretending every procedural curve has an exact NURBS representation.

## 3. Projected sketch curves, fills, and dashes

### 3.1 Largest singular value for an estimate

For a local plane-to-screen Jacobian `J=[[a,b],[c,d]]`, define:

```text
A = a*a+c*c
B = a*b+c*d
C = b*b+d*d
lambdaMax = 0.5*(A+C + sqrt((A-C)^2+4*B^2))
sigmaMax = sqrt(max(0,lambdaMax))
```

Use scaled arithmetic for large values. The largest column length is not generally `sigmaMax`. This estimate is valid at the evaluated point only; do not claim it bounds perspective scaling across an entire large curve.

### 3.2 Exact projective hull path

Transform each rational homogeneous control point through plane placement, view, and projection matrices. The resulting clip coordinate has a denominator `w_clip`. For a span whose control hull lies inside the near half-space and whose denominators are positive, division produces a rational screen curve with positive normalized weights. Projected poles bound the projected curve.

Convert projected poles to CSS pixels, then apply the finite-segment hull test from §2.3 directly using the 0.25 CSS-pixel construction target. Preserve enough parametric/world data to render the original curve samples and to share them with fills.

If the clip-space near-plane hull straddles the plane, subdivide; clip accepted leaves against the near plane for drawing. Do not project a leaf with mixed-sign/near-zero denominators. Entirely behind-camera spans do not create enormous bounds or snap markers.

Use camera matrices to obtain the active near clipping equation for the selected depth convention. Do not assume OpenGL `z+w>=0` after enabling a renderer mode that changes clip conventions. The `ProjectionAdapter` returns the valid homogeneous half-spaces.

The full curve's analytic identity remains available even when only visible leaves are rendered. Hull bounds for offscreen spans can cull by the view rectangle plus line/pick margin. Culling must not connect visible leaves across a clipped gap.

### 3.3 Hysteresis

For each entity maintain current samples, requested-quality revision, estimated/projected maximum error, and a coarsen deadline. Refine above 0.35 px; build toward 0.25 px. Coarsen only when below 0.12 px for 250 ms. A curve already at its segment cap records `limited`; it does not freeze the quality state of another curve.

The interaction tolerance of 0.75 px is a transient policy. End-of-interaction schedules refinement even if no new pointer event occurs. Budget state and pending state are different.

### 3.4 Fills

Curve and fill boundary caches share sample identity. A fill builder receives rings with `(entityId, orientation, leaf intervals, sample revision)`. It must reject mismatched revisions instead of drawing a fill that covers an old polygon while the new curve has refined.

Use authoritative static-region outer/hole classification. For live provisional closure, preserve the existing semantic distinction from authoritative regions. Triangulate holes explicitly; do not independently guess nesting from triangle winding alone. Degenerate/self-intersecting provisional loops can be shown as unfilled, with no backend mutation.

### 3.5 CSS-stable dashes

Stock line distances are not automatically CSS dash lengths. For planar conics/lines, accumulate projected CSS arc length along visible sampled leaves and provide the dashed line shader with those distances. In the custom WebGL2 screen-space dashed variant, use clip-w compensated interpolation rather than assuming a nonstandard `noperspective` qualifier. At each endpoint emit the varying pair `(cssDistance * clipW, clipW)`. After normal perspective-correct interpolation, divide the interpolated first component by the second. This recovers a screen-linear distance along the projected segment. Require positive, near-clipped `clipW` and apply the same value to both side vertices of a fat-line endpoint. This makes dash phase follow screen distance rather than distorted world distance.

Reset phase per semantic entity, not per batch chunk. Preserve phase at chunk splits by carrying the original accumulated distance. Recalculate projected dash-distance buffers on relevant camera/quality changes; never rebuild unchanged committed position buffers merely for a dash update. A clipped gap starts a new visible interval but must use a deterministic phase derived from the entity start where that can be safely projected; otherwise start at the first visible leaf and record that presentation convention.

## 4. Local origins and floating-point precision

### 4.1 Coordinate equations

Initial version-2 resources bake all current body placement/orientation into double positions; the local resource transform is translation only:

```text
O_body = double-precision world bounding-box center
p_local_f32 = float32(p_world_f64 - O_body)
p_world_reconstructed = O_body + double(p_local_f32)
p_render = p_local_f32 + (O_body - O_render)
camera_render = camera_world - O_render
```

Compute `(O_body-O_render)` in double precision before constructing the GPU matrix. Do not construct a huge float32 world matrix and multiply by an inverse huge camera matrix on the GPU. This initial contract avoids ambiguity about double-applying instance rotations. Future instancing can add a separately versioned rigid transform; do not silently reinterpret this version.

Normals are direction vectors and do not subtract an origin. Bounds in the legacy 64-byte header are **local** in version 2. Global double bounds are reconstructed from local-double bounds plus origin metadata.

### 4.2 Render-origin rebasing

Choose a rebase cell size `cell=max(1 mm, 2^floor(log2(max(visibleDiagonal,1 mm)/8)))`. Rebase when the target is farther than one cell from the current render origin, or a precision estimate requires it. Snap each new origin coordinate to that cell lattice near the target.

Rebase all render consumers in one frame snapshot. Old screen metrics and new transforms must never be mixed. Keep user document coordinates and stored camera target in doubles. Selection anchors convert back with the **resource/frame origin revision captured with the pick**, not whatever origin exists when an asynchronous response returns.

### 4.3 Plane transformation

For a world plane `n·p_world+c_world=0` and `p_world=p_render+O_render`:

```text
n_render = n_world
c_render = c_world + dot(n_world,O_render)
```

For a resource-local query using `O_body`, use `c_local=c_world+dot(n_world,O_body)`. Keep normals normalized so signed distance is in millimeters. Section sliders continue to store document-space offsets.

### 4.4 What this does and does not guarantee

Subtracting a local origin removes translation-induced float32 loss. It does not eliminate loss from a huge single-body extent. Report actual quantization error, and qualify the specified extent/feature combinations. Do not promise submicron detail on a kilometer-long single float32 mesh.

Numerical baseline examples: float32 spacing around 1,000,000 mm is 0.0625 mm; around 10,000,000 mm it is 1 mm. Native tests must translate the same small part and compare reconstruction/picking before and after local-origin conversion.

## 5. Camera, fitting, clipping, and input math

### 5.1 Canonical scale and projections

Let `h` be half the visible height at the target plane, `f` vertical FOV in radians, `a` aspect ratio:

```text
perspective distance D = h/tan(f/2)
orthographic left/right = -a*h, +a*h
orthographic bottom/top = -h, +h
```

Changing projection preserves h and target. Changing FOV preserves h and orientation, moving the perspective camera along its back vector. Orthographic mode may retain a virtual distance for depth placement, but that distance is not its zoom scale.

Normalize orientations and require a right-handed orthonormal basis. For a sketch plane with unit u, v, n and `u×v=n`, camera basis columns are `[u,v,n]`; the camera looks along local `-Z`. Position it on the +n side. A malformed basis is a typed error, not a scene-root rotation.

### 5.2 Anchored zoom

Given scale factor f, clamp the new h first:

```text
h_new = clamp(h_old*f, h_min, h_max)
f_effective = h_new/h_old
```

For perspective homothetic zoom around visible anchor A:

```text
target_new = A + f_effective*(target_old-A)
camera_new = A + f_effective*(camera_old-A)
```

This preserves orientation and screen position of A while scaling target distance. If the chosen anchor has a different depth than the target, this still preserves its projection because both camera and target undergo the same similarity transform around A.

For orthographic zoom, keep the orientation and scale h, then solve the target translation in camera right/up directions so the anchor's projected CSS point remains fixed. Moving the target's back component is unnecessary; do not introduce hidden dolly drift. An equivalent method is unprojecting the cursor ray onto the anchor-depth plane before and after changing h and translating by the difference.

For both modes, if f_effective=1, target remains unchanged. Validate finite nonzero h before division.

### 5.3 Fit in a measured safe rectangle

Use the camera basis and all 8 corners of the target bounds. For perspective, solve the minimum distance satisfying horizontal and vertical projection inequalities for every corner, including safe-rectangle offsets and padding. Reuse the existing `computeCameraFit` implementation when it satisfies those tests; do not replace it with a bounding sphere scaled only by vertical FOV.

For orthographic fit, compute min/max coordinates in right/up, determine h from safe-rectangle scale, and move target to the correct projected center. Include positive near-depth margin and refuse impossible limits. Padding is 24 CSS pixels around the requested geometry inside the measured safe rectangle.

Fit requests carry the source publication and chosen target class (visible, selection, preview, plane). A response from a removed selection cannot move the current camera to a stale object.

### 5.4 Adaptive near/far

Transform the 8 corners of effective displayed bounds to camera space. Let positive depth be distance along camera forward. Aggregate minimum and maximum visible positive depths; use conservative bounds, not only body centers.

For a scene entirely in front of the eye:

```text
n_candidate = max(1e-4 mm, 0.5*d_min)
f_candidate = max(n_candidate+1e-3 mm, 1.1*d_max)
```

If bounds straddle the eye, retain the positive near floor and set far from positive extent. This is an expected precision-risk case; do not force the camera away without user input. For no visible geometry use a default inspection range derived from camera scale, not a giant fixed range.

Expand the frustum immediately if required to avoid clipping. Contract near/far only after 150 ms stable input and when change exceeds 20%; this avoids depth flicker during small camera motions. A Fit action may apply the final range immediately with its camera transaction. Near must remain positive and far>near.

For a conventional b-bit depth buffer, the local depth spacing estimate is approximately:

```text
Δz(z) ≈ z²*(far-near)/(far*near*(2^b-1))
```

Use the actual depth bits and a separate reverse-depth model when enabled. This estimate guides diagnostics/tests; it is not a license to move geometry by Δz until conflicts disappear.

### 5.5 Two-pointer gesture update

Store both pointer positions and calculate centroid C and distance S after each event. Treat each event's new pair as the next gesture state:

```text
centroidDelta = C_new-C_old
scaleFactor = S_old/S_new
```

Apply pan and anchored scale in **one** camera transaction, using the centroid before/after to preserve the content under the moving fingers. Update the baseline after every accepted event. Adding/removing a pointer rebases the baseline without moving the camera. This avoids double-counting common translation as both pointers emit events.

## 6. Visibility-aware edge picking

### 6.1 Candidate acquisition

Query body bounds, then segment bounds expanded conservatively for a 6 CSS-pixel disk at relevant depth. Project candidate segments, clip to the near plane, and calculate the closest projected point to the pointer. Candidates outside the CSS disk are rejected before semantic lookup.

For projected endpoints s0,s1, get screen interpolation λ. Recover the original segment parameter with homogeneous interpolation:

```text
t = (λ/w1) / ((1-λ)/w0 + λ/w1)
P = (1-t)*P0 + t*P1
```

Here `w0,w1` are endpoint clip-space denominators after near clipping, and P0/P1 are the corresponding clipped world points. Denominator positivity is required. Do not linearly interpolate world position using λ in perspective.

### 6.2 Matched visibility ray

Cast a new ray through the candidate's closest screen point, not necessarily through the original pointer. Use double render/resource transforms and clipping policy. Query the nearest visible surface including exact-preview and section-cap occluders according to display role.

Let t_e and t_s be distances on this same ray. For a well-conditioned intersection use `epsilonNumeric = qEdge + qSurface + 64*Number.EPSILON*max(1, abs(t_e), abs(t_s), localMagnitude)`, where q values are the producer-measured quantization errors in millimeters, and localMagnitude is the largest absolute resource-local coordinate used by the query. This is a stated engineering roundoff policy, not a formal bound for ill-conditioned ray/triangle arithmetic. Near-tangent or otherwise ill-conditioned hits must return ambiguous instead of receiving an arbitrarily enlarged epsilon. Do not include the six-pixel radius. If `t_e > t_s + epsilonNumeric`, the edge is occluded **unless** the only discrepancy comes from an explicitly quantified surface/edge approximation ambiguity.

For approximation ambiguity, require candidate adjacency and local error metadata. Do not use a global body tolerance to let an unrelated rear edge through. If the uncertainty interval crosses the visibility decision, retain the face candidate and request finer geometry. The user can explicitly invoke pick-through to choose the hidden/ambiguous edge without altering ordinary visibility rules.

No finite tolerance can make an inaccurate display mesh a perfect BRep visibility oracle. This contract makes uncertainty explicit instead of silently choosing a likely-wrong edge.

### 6.3 Selection and hover rules

In ordinary mode, sort visible acquired edges by screen distance first, then matched depth and ID. Edge preference applies only over a locally coincident/touching face result after visibility passes. Explicit pick-through includes occluded classes after visible ones and never silently switches itself on.

Before promotion to an operation target, compare document, runtime session, snapshot, generation, installed resource identity, and topology revision. LOD-only resource changes can be resolved against the same authoritative topology, but an old snapshot TopoKey cannot be promoted merely because its string still exists.

### 6.4 Section cap occlusion

A cap is not a BRep face. For a matched ray crossing a section plane, compute the plane hit P. Determine whether P lies inside a verified closed displayed solid through a robust ray-parity query on that solid's original unclipped triangle mesh. Count intersections with duplicate shared-edge hits consolidated using scale-aware numerical tolerance. Use at least one deterministic alternate direction when the ray meets a triangle boundary or tangent ambiguity; unresolved results become ambiguous inspection, not an editable face.

A nearer cap intersection can block a candidate behind it. Return `sectionSurface` for cap clicks. It must not mint a persistent face ID or silently forward a click to the far interior face behind the displayed cap.

## 7. Surface normals and transformed winding

### 7.1 Primary evaluation

For each face node with UV `(u,v)`, evaluate the supporting surface derivatives `S_u` and `S_v`. Where regular:

```text
n_local = normalize(S_u × S_v)
n_oriented = orientationSign * n_local
n_world = normalize(inverseTranspose(A) * n_oriented)
```

Use the surface location transform A exactly once. Avoid mixing `BRepAdaptor_Surface` output already in world placement with a second BRep location transform.

An implementation may use `BRepLib_ToolTriangulatedShape::ComputeNormals` on isolated triangulation to obtain surface-aware normals, but the helper is documented as a no-op when normals already exist. Old cached triangle-averaged normals must not bypass the new policy. Tests must establish the helper's orientation and placement convention before using it as an adapter. The selected contract remains the equations above. [EXT-04.]

### 7.2 Reflection and winding

When emitting triangles from original local orientation, apply a winding reversal when `faceIsReversed XOR (det(A)<0)`. A negative determinant reverses cross-product orientation even though inverse-transpose maps the physical outward normal. Confirm this with a mirrored closed-solid fixture, not only a reflected open triangle.

OCCT may already bake certain transforms/orientation changes into the returned shape. Determine which representation is being read and apply the rule to the actual remaining A. Never invert twice based on an external operation name such as "Mirror".

### 7.3 Singular and degenerate nodes

- Plane: use oriented analytic plane normal.
- Sphere pole: use the normalized vector from analytic sphere center to point, with orientation/transform.
- Cylinder/torus regular nodes: use their analytic parameterization/derivative normal.
- Cone apex or genuine nonsmooth singularity: there is no unique smooth normal. Split the singular node per incident triangle/fan and use a valid oriented incident normal; record `singular-split` provenance.
- General singular UV: collect nondegenerate incident triangle normals in the same face. If their maximum angular spread is ≤5°, normalize their area-weighted sum and record `triangulation-fallback`. Otherwise split the node per smooth incident fan/triangle and record `singular-split`.
- No nondegenerate incident triangle: do not emit an arbitrary normal. Diagnose/remove degenerate triangles while preserving face ordinal and completeness state.

Never smooth across a true face crease. Duplication for normals must not change triangle ordinal ordering or face ranges; only vertex indices may change.

### 7.4 Acceptance of shading normals

Normals must be finite/unit length; analytic fixtures should agree within 0.1° away from singularities. Tangent seam samples that are semantically smooth should agree within 0.2° on the fixture corpus after tolerance-aware evaluation. These are rendering acceptance thresholds, not changes to the BRep continuity definition. A conical apex is not required to have a unique smooth normal.

## 8. Edge classification

### 8.1 Metadata

Each topological edge has a u32 bitset:

| Bit | Meaning |
|---:|---|
| 0 | Open boundary: one incident face in the displayed sheet/solid partition |
| 1 | Hard/crease edge: verified discontinuity or non-tangent join |
| 2 | Tangent boundary: continuity established |
| 3 | Periodic seam: same supporting face seam representation |
| 4 | Degenerate edge: semantically present, no reliable drawable segment |
| 5 | Nonmanifold adjacency |
| 6 | Classification unknown |
| 7–31 | Reserved, must be zero for policy version 1 |

Flags may combine where meaningful, but validation rejects `tangent` plus `hard`, and rejects `unknown` plus a definitive continuity classification. Open or nonmanifold edges remain visible in feature mode. Do not rely solely on flag precedence to hide them.

### 8.2 Classification algorithm

Build edge-to-face incidence from the authoritative shape without renumbering topology. Inspect OCCT seam and continuity metadata first. A verified seam of one regular face may be hidden in feature mode. Explicit C1/G1-or-better continuity metadata is a starting proof for tangent classification, with orientation/location consistency checked.

For imported shapes lacking trustworthy continuity, keep the edge `unknown`. Multiple original-parameter samples are a diagnostic screen for likely tangency, NOT permission to hide an edge: samples can miss a localized crease just as they can miss a spline excursion. Upgrade to tangent only from trustworthy continuity metadata with orientation/location consistency or an analytic supporting-surface equivalence check that establishes continuity along the complete edge interval. Sample-only results remain `unknown`, visible, and labeled `sample-consistent` in diagnostics. Do not change or heal the authoritative BRep just to obtain a display classification.

Never classify a shallow but real 1° crease as tangent merely because the surface mesher uses a 5° angular deflection. Tangency tests and mesh quality are different concepts.

## 9. MESH1 binary version 2

### 9.1 Version strategy

Retain the `MESH1` family magic and existing 64-byte header/table structure. Set header `version=2`. Version-1 parsers must reject version 2; unknown-section skipping is not enough because coordinate semantics change. Preserve version-1 decoding for old cache/persistence fixtures, with explicit legacy status.

Existing type codes 1–12 retain their data layouts. In version 2, position, edge-point, face-bounds, and header bounding-box values are **local to the required origin**. This semantic change is exactly why the version changes.

Types 13–21 below are allocated by this specification for the pinned baseline, where they are unused. If the repository has allocated them after this baseline, record the collision before coding and assign a non-colliding contiguous range through one canonical ADR. Do not silently collide or renumber existing sections.

### 9.2 Additional sections

All f64 sections must start at an 8-byte boundary; all others remain at least 4-byte aligned. Table entries remain 16 bytes and byte offsets remain u32. Padding bytes must be zero.

| Type | Name | Required | Exact size / meaning |
|---:|---|---|---|
| 13 | `LOCAL_ORIGIN` | v2 always | 24 B: 3 little-endian f64 world-mm coordinates |
| 14 | `LOCAL_BOUNDS64` | v2 always | 48 B: min xyz and max xyz, f64 local-mm bounds enclosing surface and edge positions before quantization |
| 15 | `QUALITY_INFO` | v2 always | 48 B record, layout below |
| 16 | `EDGE_FLAGS` | if edges | 4*E B u32 classifications |
| 17 | `FACE_ERROR_BOUNDS` | optional | 8*F B f64 maximum certified bound in mm, or -1 for not certified |
| 18 | `EDGE_ERROR_BOUNDS` | if edges | 8*E B f64 certified chord bound, or -1 for not certified |
| 19 | `FACE_SOLID_IDS` | v2 always | 4*F B u32 local solid-partition ordinal; `0xFFFFFFFF` for open/unknown face |
| 20 | `EDGE_FACE_OFFSETS` | if edges | 4*(E+1) B u32 prefix offsets into type 21 |
| 21 | `EDGE_FACE_ORDINALS` | if edges | 4*A B u32 face ordinals, where A is the final type-20 offset |

Type 19 enables per-solid caps for compounds. Solid ordinals are local display metadata, not persistent topology IDs. Partitions must be independently verified closed/oriented before they are cap-eligible.

Types 20–21 carry edge adjacency required for NUM §6.2. Each edge owns a sorted unique list of incident face ordinals; every ordinal must be <F. A seam can name its single supporting face once, with the seam bit expressing its role. A zero-adjacency wire edge has an empty list; it is not a closed-solid boundary. Offsets begin at zero, are monotonic, and end at A. These are mesh-local ordinals, never persistent IDs. A consumer without verified adjacency must not grant the adjacency-based approximation exception in picking.

`QUALITY_INFO` layout:

| Byte offset | Type | Field |
|---:|---|---|
| 0 | u32 | `displayPolicyVersion`, initially 2 |
| 4 | u32 | `qualityFlags` |
| 8 | f64 | `requestedChordToleranceMm`, finite >0 |
| 16 | f64 | `requestedAngularToleranceRad`, finite >0 |
| 24 | f64 | `maxCertifiedEdgeErrorMm`, -1 if any drawable edge lacks a certified bound |
| 32 | f64 | `quantizationErrorMm`, measured finite >=0 |
| 40 | f64 | `maxSampledSurfaceErrorMm`, -1 when no independent surface sampling was performed |

Quality flags: bit0 complete nondegenerate face coverage; bit1 all drawable edges chord-certified; bit2 all face error bounds certified; bit3 some normal fallback; bit4 quality-limited; bit5 some curve angular status undefined/uncertified; bits6–31 reserved zero. Do not set all-face-certified solely because OCCT meshing completed.

`FACE_ERROR_BOUNDS` absent means unknown, not zero. A known zero-triangle degenerate face can have bound 0 but must have explicit producer diagnostics; a missing nondegenerate face cannot use that escape.

Header flags 0–4 retain meanings. No new header flag is necessary: v2 and section presence determine required data. Bits5–15 remain reserved zero. This avoids independent contradictory flags for mandatory v2 fields.

### 9.3 Bounds rule

Header local f32 min/max must conservatively enclose quantized output, using outward rounding or recomputation from the written float32 values. `LOCAL_BOUNDS64` encloses pre-quantization output. The validator compares these with measured positions within the declared quantization allowance. Never use stale kernel world bounds as local bounds.

Face bboxes are local in v2 and must enclose that face's triangles. No code may derive document fit directly from header f32 values without applying `LOCAL_ORIGIN`.

### 9.4 Negotiation and requests

Add capability strings to the existing worker hello capability array:

```text
mesh.format.v2
mesh.display-quality.v2
mesh.edge-classification.v1
```

The new frontend communicates only through CadClient/Tauri DTOs. Rust checks worker capabilities and validates the returned mesh version before forwarding bytes. Add `meshFormatVersion` and a structured `displayQuality` to the existing tessellation request, preserving existing required body/session fields. Old callers omitting these fields get version 1 and the existing tier contract; new callers explicitly request version 2.

```typescript
interface DisplayQualityRequestV2 {
  policyVersion: 2;
  linearLevel: number;   // integer -4..16
  angularLevel: number;  // integer 0..8
}
```

Normative formulas:

```text
linearToleranceMm = 0.05 * 2^(-linearLevel)
angularToleranceRad = (5*pi/180) * 2^(-angularLevel)
```

`linearLevel=0, angularLevel=0` is the initial committed display tier. Negative linear levels are permitted for explicitly coarse interaction/budget states, not silent degradation of a settled qualified mesh. Request rounding chooses the finest level necessary; if requested accuracy exceeds max level, mark limited.

Expose new DTO metadata with camelCase and keep existing Tauri command naming conventions. C++/Rust models, request parsers, mock client, recorded fixtures, and serializer tests must agree. Do not paste only this TS interface into one layer.

### 9.5 Cache and identity

Do not overload the existing `<bodyId>:<lod>:<generation>` string by appending undocumented fields. Define a separate versioned cache-key structure internally:

```text
(documentId, runtimeSession, snapshotId, generation,
 bodyId, geometry/content revision, meshFormatVersion,
 displayPolicyVersion, linearLevel, angularLevel,
 kernelBuildFingerprint)
```

The string representation, if required, is canonically serialized from typed fields, not parsed with the legacy regex. A legacy key never aliases a v2 cache entry. Appearance-only changes should update color attributes/descriptors without native retessellation when topology and geometry are unchanged.

A display refinement is read-only and must not increment document revision, mint new ElementIds, or create an undo record. Use a separate display-resource revision. Original snapshot identities remain attached to every result.

### 9.6 Migration sequence

1. Add dual-version parsing and byte fixtures to Rust/TS with v1 unchanged.
2. Add v2 encoder and producer fixtures in C++; keep default v1 for old requests.
3. Add capability negotiation, request fields, display-quality cache keys, and cross-track fixtures.
4. Add local-origin transforms in every frontend consumer.
5. Switch new application requests to v2 only after integration gates pass.
6. Keep v1 decoder for old saved display caches; asynchronously regenerate to v2 before large-offset qualification.
7. Validate packaging fingerprints so a new app cannot silently launch an incompatible old worker.

User documents and BRep data remain untouched. Cached rendering can be invalidated; user-authored geometry cannot be discarded because a display cache version changed.

## 10. Semantic mesh validation details

Validate multiplication and addition using safe integer/checked arithmetic before typed-array construction. A byteLen calculation wrapping a u32 is never acceptable.

For face ranges require contiguous triangle coverage in ordinal order, allowing zero-count faces at a shared boundary. A triangle must belong to exactly one range. For edge point ranges require contiguous point coverage, allowing known zero-point degenerate edges. Sum `max(0,count-1)` with checked arithmetic before allocating segment endpoints.

ID offset arrays begin at 0, are monotonic, end exactly at character-section length, and remain in bounds. Decode with fatal UTF-8 validation. IDs are nonempty and capped at 256 UTF-8 bytes per ID; existing producer tests must establish this covers actual IDs. Enforce uniqueness within face IDs and within edge IDs. Do not reject the same textual TopoKey prefix across face/edge namespaces by accident. Use the existing ElementId grammar; do not create a guessed regex that rejects valid persisted IDs.

For indexed colors, build a temporary vertex owner/color map only after admission. A vertex shared across differently colored faces is split in the prepared rendering representation, with triangle order unchanged. The canonical wire payload remains unchanged.

Unknown section types are skipped only when offset/length/alignment remain structurally valid. Unknown binary versions fail. Required v2 sections cannot be skipped. Validate types 20–21 as contiguous edge adjacency lists with sorted unique in-range face ordinals. Reserved quality/edge bits fail until a new policy version defines them. Contradictory certification flags and absent/unknown error-bound arrays fail validation; absence is never a zero error.

Recommended result type:

```typescript
type ValidationResult =
  | { ok: true; mesh: ValidatedMesh; accounting: MeshAccounting }
  | { ok: false; code: MeshValidationCode; detail: string; bodyId: string };
```

Do not throw across UI store actions. Low-level parsing may throw typed exceptions internally, but the ingestion boundary converts them into explicit failure/stale-inspection state.

## 11. Resource and frame transaction pseudocode

### 11.1 Safe scheduler

```text
invalidate(reason):
    if disposed: return
    requestedRevision += 1
    dirtyReasons |= reason
    ensureOneFrameUnlessLost()

tick(time):
    scheduled = false
    if disposed or contextLost: return
    advanceActiveTransitions(time)  // may invalidate
    consumedReasons = dirtyReasons
    consumedRevision = requestedRevision
    dirtyReasons = NONE            // consume BEFORE callbacks
    if consumedReasons != NONE or transitionsChanged:
        frame = buildConsistentFrameSnapshot()
        submit(frame)
        notifySubmitted(frame, consumedRevision)
        retireUnreferencedResourcesAtFrameEnd()
    if dirtyReasons != NONE or transitionsStillActive:
        ensureOneFrameUnlessLost()
```

Avoid an extra idle frame when a transition ends: return distinct `changedThisTick` and `stillActive` flags. Wrap submission failure to enter a bounded error/recovery state; do not spin forever resubmitting a failing shader.

### 11.2 Lease transaction

```text
install(newResource, expectedPublication):
    validate identity and admission before GPU allocation
    if expectedPublication is no longer current: discard prepared output
    acquire registry ownership of newResource
    prepare new display/section/highlight leases
    swap effective display snapshot atomically
    release old display/section/highlight leases
    mark old registry entry retired
    invalidate GEOMETRY
```

If preparing a consumer fails, release newly acquired leases and leave the old display snapshot intact. Do not publish half the scene and then try to repair it from a later event.

## 12. Algorithms intentionally not selected

Do not substitute midpoint-only subdivision, endpoint-tangent-only tests, arbitrary normal averaging across faces, hardcoded global depth bias, globally de-indexed meshes, GPU synchronous hover readbacks, undocumented buffer-detach disposal tricks, continuous idle polling, or a renderer rewrite.

These exclusions are not style preferences. Each would reintroduce an identified correctness, ownership, precision, or latency risk that the program is intended to remove.
