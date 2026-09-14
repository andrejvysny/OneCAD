# wp09-surface-normals-break
date: 2026-09-14
mode: break
model: gpt-6-astra
effort: xhigh
access: grounded: worker/src/tess/SurfaceNormals.h, worker/src/tess/SurfaceNormals.cpp, worker/src/tess/Tessellate.cpp, worker/src/tess/Tessellate.h, worker/src/tess/EdgeClassification.cpp, worker/tests/test_surface_normals.cpp, docs/viewport-hardening/03-NUMERICS-AND-PROTOCOL.md, docs/viewport-hardening/01-SPECIFICATION.md, docs/qa/viewport-hardening/baseline/ctest-surface-normals-red.log, docs/qa/viewport-hardening/runs/wp09-focused/ctest.log
packet: sha256 18da078b1dbd
calls: 1 of ~3 for WP09; session 01a09f36-a120-71f3-affc-53242255ec0f recorded for followup
verdict: defective — completeness failures publish successfully (F1), the body-relative area escape excuses valid missing faces (F2), and the D1-noise / bbox-scale / triangle floors, the 0.2° tangent screen, the facet-chosen sphere-pole sign, the cone-apex averaging, and the one-triangle completeness rule lack the guarantees they claim (F3–F11)
verified by: orchestrator recomputation (python, 2026-09-14) — F2 threshold 1e-12·(1e4)² = 1e-4 mm² vs 0.005² = 2.5e-5 mm²; F2 bulge centre Jacobian |Su×Sv| = 4 mm² with all-collapsed boundary; F3 dyadic fixture in binary64: Q0 = −M+a exact, Q1 = M+a rounds to M, Dx(1/2) = a/2 vs exact a, normal error atan(2)−45° = 18.4349°; F5 triangle (0,0,0),(1,0,0),(2,1.5e-12,0) sines by cyclic permutation 7.5e-13 / 1.5e-12 / 7.5e-13 (reject/accept/reject at the 1e-12 floor); F6 two r=0.1 mm spheres with centres 0.0001 mm apart: dihedral 2·asin(0.0005) = 0.0573° < 0.2° band; F8 cone R=10/H=20 apex fan at −2°/0°/2°: facet spread 1.7889°, mean 0.8944° from each facet; shallow cone H = 10·tan 2° = 0.34921 mm, 72 sectors: spread 4.0038° (< 5° → averaged); F11 valence 10 000 → 49 995 000 pairwise comparisons; §4 ulp32(8192 mm) = 2^-10 mm and float32(8192.0001) == float32(8192); §3 square+strip area 1.02 mm² ≤ 0.001 × 2001 mm; transcript audit: 10 of 10 paths read are in the allowlist, nothing outside
rejected: none of the mathematics. F3's fixture is accepted as a conditioning counterexample for the CLAIMED universal 1e-12 relative bound, not as an observed OCCT D1 failure (Astra says so itself); the implementation closes it with an evaluator-independent absolute derivative-error budget rather than by inspecting OCCT's evaluation order. F6's tolerance-close analytic-equivalence attack is accepted; the existing 1°-hard / 0.05°-unknown behaviour is kept. §3's "display-detail omission" class is recorded as not authorised in this pass (D11), as Astra states.

## 1. Verdict

**Defective:** completeness failures publish successfully, body-relative area can excuse valid missing faces, and several normal/continuity predicates lack the claimed error guarantees.

## 2. Findings, ranked by severity

**F1 — Blocker · proved. Missing nondegenerate faces still publish.**  
`allNondegenerateFacesCovered=false` never affects `out.ok`; the test explicitly requires publication. This violates this pass’s D11 policy. **Smallest close:** assign `out.ok = out.completeness.allNondegenerateFacesCovered`; reverse that test assertion and update the contradictory header comment. Keep diagnostics and ordinal tables. [Assignment](worker/src/tess/Tessellate.cpp:443), [test](worker/tests/test_surface_normals.cpp:862).

**F2 — Blocker · numeric/proved. Neither area ratio nor boundary flags establish face degeneracy.**

- Body diagonal 10,000 mm: threshold `10^-12 × 10,000² = 10^-4 mm²`. A `0.005 × 0.005 mm` face has `2.5×10^-5 mm²`, so failure is excused.
- Merely substituting face diagonal fails differently: every square has `area/diagonal² = 1/2`, independent of whether its width lies below BRep tolerance.
- All-degenerate boundary edges do not prove zero interior area. Let `f=16u(1−u)v(1−v) mm`, `S=((2u−1)f,(2v−1)f,f)`, on `[0,1]²`. Every boundary maps to one point; at the centre, `Su=(2,0,0) mm`, `Sv=(0,2,0) mm`, hence `|Su×Sv|=4 mm²`: positive-area interior. OCCT acceptance of this constructed face remains untested.

**Smallest close:** delete the body-area escape; require a whole-face certificate with an explicit reason (§3). Boundary flags become evidence to inspect, not sufficient proof. [Predicate](worker/src/tess/SurfaceNormals.cpp:408).

**F3 — High · numeric/inference. `kSurfaceD1RelativeNoise=10^-12` is not a demonstrated relative forward-error bound.**  
A degree-dependent stability estimate needs a conditioning factor. Derivative control vectors can nearly cancel; rational weights, knots and transformation arithmetic also need bounds.

Conditional on the claimed noise, the sine arithmetic is reasonable:

`2×10^-12 / 10^-8 = 2×10^-4 rad = 0.0114592° < 0.1°`.

The premise fails as a general mathematical argument. The binary64 derivative-control calculation in §5 loses **50%** of one derivative component, yielding **18.43495°** normal error while both current acceptance floors pass. This is a computed evaluator counterexample, **not an observed OCCT D1 failure**.

**Smallest close:** replace the asserted universal bound with evaluator-specific absolute derivative-error evidence; use independent analytic formulas where available. Comparing against another call to the same OCCT D1 cannot validate its error. [Claim](worker/src/tess/SurfaceNormals.h:76), [spline oracle](worker/tests/test_surface_normals.cpp:751).

**F4 — High · proved/numeric. Face bbox is a geometric extent, not an evaluation-error scale.**

- A `0.01 × 0.01 mm` trim has `D=0.0141421 mm`. Supporting control coordinates of magnitude `10^10 mm` give `uC≈1.11×10^-6 mm`, versus `uD≈1.57×10^-18 mm`, using `u=2^-53`. These are conditioning scales, not measured OCCT errors.
- World AABB diagonal is not rotation invariant. A unit square’s diagonal changes from `√2 mm` to `2 mm` after a 45° in-plane rotation. A derivative-span product `1.5×10^-9 mm` therefore passes before rotation and fails afterwards.
- Invalid/zero diagonals and UV spans silently become `1.0`, introducing dimensionful/parameter-dependent defaults.

**Smallest close:** reject invalid scale inputs explicitly; use actual evaluator error bounds for acceptance. A rotation-invariant face extent may remain a heuristic, but cannot certify accuracy. [Scale construction](worker/src/tess/SurfaceNormals.cpp:39).

**F5 — High · numeric/proved. Triangle floors neither compose into the claimed angular bound nor give a symmetric predicate.**  
Source checks **both edges from the first vertex**, not the packet’s longest edge.

For `A=(0,0,0)`, `B=(1,0,0)`, `C=(2,1.5×10^-12,0) mm`, cyclic permutations produce sine values `7.5×10^-13`, `1.5×10^-12`, `7.5×10^-13`: **reject/accept/reject for the same triangle**.

With coordinate uncertainty `εD`, edge length `L`, and included angle θ, direction uncertainty scales as `εD/(L sinθ)`. Even granting `ε=10^-16`, simultaneous `L/D=2×10^-12`, `sinθ=2×10^-12` gives `2.5×10^7`, not `10^-4 rad`.

**Smallest close:** separate exact nonzero-area testing from trustworthy-normal testing; use a symmetric, bounded cross-product calculation. Uncertain positive-area triangles require failure/refinement, not “degenerate” deletion. [Implementation](worker/src/tess/SurfaceNormals.cpp:90).

**F6 — High · numeric/proved, with a metadata-trust condition. The edge screen can certify false tangency.**

- Two spheres, radius `0.1 mm`, centres separated by `0.0001 mm`, both face tolerances explicitly `0.0001 mm`: supporting-surface “equivalence” passes. Their intersection has real dihedral  
  `2 asin(0.0001/(2×0.1)) = 0.0572958°`.  
  All three samples pass `0.2°`, so distinct intersecting spheres become tangent.
- With untrusted/stale G1 metadata, the polynomial fixture in §5 has `0°` at all three samples and `1°` between them.
- One successful sample suffices; failed samples are skipped.
- Shared surface-pointer identity does not establish one-sided continuity across a nonsmooth spline knot.

The existing **1° hard / 0.05° without metadata unknown** behaviour is conservative and sound for those fixtures. The `0.2°` rendering band is not proof that a real crease is absent.

**Smallest close:** separate trusted continuity evidence from sampling consistency; require all required samples to be evaluable with bounded normal error. Tolerance-close analytic parameters cannot establish whole-edge G1 equivalence. Unproved joins remain unknown. [Screen](worker/src/tess/EdgeClassification.cpp:79), [equivalence/classification](worker/src/tess/EdgeClassification.cpp:108).

**F7 — High · proved. Sphere-pole sign is mesh-dependent.**  
Reversing usable incident facets reverses the supposedly analytic pole normal by **180°**, while its provenance remains `AnalyticSingular`. A zero/cancelled witness leaves a different sign path.

**Smallest close:** determine sphere parameterization chirality and face/placement orientation analytically, or cache the signed radial relation at one verified regular surface point. Facets may diagnose disagreement; they must not choose the analytic sign. Simply deleting the flip without handling indirect frames is insufficient. [Sign override](worker/src/tess/SurfaceNormals.cpp:310).

**F8 — High · numeric/proved. Known cone apex incorrectly enters the general 5° averaging rule.**  
Cone `R=10 mm`, `H=20 mm`, apex fan with base azimuths `−2°,0°,2°`: facet spread is **1.788891°**. Current code averages; the mean differs from each facet by **0.894445°**. NUM instead requires singular splitting at the known apex.

A full shallow cone, `R=10 mm`, `H=10 tan2°=0.349207695 mm`, with 72 equal sectors has maximum facet spread about **4.003808°**; averaging invents an axial apex normal.

**Smallest close:** dispatch known cone apices/nonsmooth singularities to splitting before the general fallback. [Fallback](worker/src/tess/SurfaceNormals.cpp:200), [binding rule](docs/viewport-hardening/03-NUMERICS-AND-PROTOCOL.md:395).

**F9 — High · proved. One surviving triangle establishes “complete”, despite arbitrary filtered losses.**  
`complete = !triangleVertexIndices.empty()` cannot distinguish removal of certified zero-area triangles from deletion of numerically rejected, positive-area coverage. Thus F5 can create a hole while completeness remains true.

**Smallest close:** distinguish `exactZero`, `usable`, and `unresolvedNonzero` triangles. Any unresolved loss invalidates successful publication; “at least one triangle” remains only a face-presence test. [Completion](worker/src/tess/SurfaceNormals.cpp:389).

**F10 — Medium · proved. Missing nodes still emit arbitrary +Z; finite-normal tests can miss NaN.**  
`NodeDecision.normal` defaults to +Z and is serialized for `Missing`, even though unreferenced. The unit-length test uses `std::max(worst, NaN)`, which can leave `worst` unchanged.

**Smallest close:** omit unreferenced missing slots with explicit remapping, or refuse emission; assert component finiteness before angular/length reductions. [Emission](worker/src/tess/SurfaceNormals.cpp:345), [test](worker/tests/test_surface_normals.cpp:673).

**F11 — Medium · proved. Singular-fan work has no demonstrated constant bound.**  
Pairwise spread is `k(k−1)/2`: valence 10,000 means **49,995,000** comparisons. The torus timing does not establish singular-fan cost.

**Smallest close:** enforce a documented valence/work bound, or use a deterministic conservative split when spread cannot be established within that bound. Emitting k split vertices itself requires Ω(k); the achievable overall target is linear in nodes plus triangles. [Pairwise loop](worker/src/tess/SurfaceNormals.cpp:152).

## 3. Face-degeneracy predicate

**A safe replacement is certificate-based and conservative. No area-only threshold separates the requested classes.**

For example, a planar `1×1 mm` square attached to a `2000×10^-5 mm` strip has area `1.02 mm²`. With tolerance `0.001 mm` and face diagonal about `2001 mm`, even **face-local** `area ≤ tolerance × diagonal` passes: `1.02 ≤ 2.001`. The substantial square survives geometrically. **Numeric counterexample.**

Use this replacement contract; unsupported cases fail closed:

```cpp
FaceDegeneracy classify_face_degeneracy(const TopoDS_Face& face);
// CertifiedZero(reason), CertifiedKernelCollapse(reason), Unproved(reason)
```

| Class | Required detection predicate | Consequence |
|---|---|---|
| **Exact zero-area topology** | Validated trim description plus certificate that its retained surface image has dimension ≤1, or that its retained 2D domain is empty. All-degenerate edges alone are insufficient. | Zero range; specific certificate reason. |
| **Kernel-collapsed sliver** | Whole trimmed component certified collapsed under the authoritative BRep tolerance semantics; no remaining resolved patch/island. | Zero range; distinguish tolerance collapse from mathematical zero. |
| **Missing valid/unproved face** | No output triangles and neither certificate succeeds. A positive-area interior witness proves the valid-face failure case; otherwise report uncertainty accurately. | `missingFaces`, warning, `out.ok=false`. |
| **Display-detail omission** | Requires explicit omission intent, an approximation budget and published downgrade—not geometry alone. None is authorized under this pass’s D11 rule. | Cannot use the degeneracy escape. |

**Implementable sufficient certificates — proved mathematically:**

- Recognized polynomial/rational spans with identically zero derivative-numerator cross product, with denominator validity established throughout the retained domain.
- Certified point/curve image bounds covering the **whole face**, not just edges or triangulation vertices.
- Robust trim-domain analysis establishing zero retained measure. Missing/malformed trimming is uncertainty, not an empty-domain certificate.

**Sub-tolerance width requires an additional semantic premise.** For a certified strip with paired non-seam rails, let `w(s)` be the full transverse surface-path length, and `τ0(s), τ1(s)` the applicable authoritative positional tolerances. A sufficient *width-unresolved* test is:

`w_upper(s) ≤ τ0(s) + τ1(s)` for every section s.

The sum follows from two positional uncertainty radii; all quantities are lengths. A certified upper bound over complete sections prevents coincident endpoints from hiding an interior bulge. Use local applicable tolerances, not the largest tolerance elsewhere on the face.

**Proof gap:** overlapping tolerance tubes establish unresolved width, not permission to identify topology. If no authoritative kernel-collapse rule grants zero-range status, return `Unproved`. That rule and its tolerance-combination semantics are missing from the allowed material.

Floating-point uncertainty belongs in outward-rounded measurement bounds, **not in an enlarged degeneracy allowance**. An interval crossing the decision boundary remains unproved. `BRepGProp::Mass()` is supplied without an error certificate; numerical zero alone cannot close this proof.

Under `p′=λQp+t`, `λ>0`, exact rank/domain certificates are unchanged; widths and authoritative tolerances must both scale by λ, areas by λ². Unit conversion likewise converts all length-valued inputs; angular constants remain unchanged. Fixed physical BRep tolerances during actual model resizing describe a different policy and may legitimately change classification. Rounded boundary cases require filtered/exact evaluation or an uncertainty result.

**Minimal integration change:**

```cpp
const auto evidence = classify_face_degeneracy(face);
// Only certified results enter degenerateFaces; log evidence.reason.
// Everything else enters missingFaces and clears coverage.

out.ok = out.completeness.allNondegenerateFacesCovered;
```

Remove the body-diagonal argument, area-ratio constant and obsolete publication expectation. No MESH1 field, ID ordering or normal-split ordering change is needed.

## 4. Missing degeneracy classes and detection predicates

| Class | Detection predicate |
|---|---|
| **Zero-width trim with nondegenerate longitudinal edges** | Certified coincident paired rails/full strip collapse; edge flags alone miss it. |
| **Thin annulus or curved ribbon** | Whole-component transverse-width certificate using surface paths and periodic-aware trimming; raw UV width is insufficient. |
| **Rank-one/point supporting image** | Identically zero Jacobian numerator over every retained span; certify rational denominator validity. |
| **Boundary collapse with positive interior** | A regular interior sample with certified nonzero Jacobian disproves whole-face degeneracy; F2’s polynomial supplies one. |
| **Malformed/self-overlapping trims, invalid rational denominator, failed evaluation** | Robust trim/representation validation; classify invalid/unproved and fail. Do not interpret cancelled/negative/nonfinite mass as known degeneracy. |
| **Local pole/apex/knot singularity on a valid face** | Known analytic singular locus or one-sided derivative analysis; apply node policy, never whole-face omission. |
| **Representation collapse after float32 conversion** | Re-evaluate emitted triangle nonzero area from the actual stored float32 positions; compare against pre-conversion evidence. This is a representation failure, not BRep degeneracy. |

For the last class, `ulp32(8192 mm)=2^-10 mm=0.0009765625 mm`; both `8192` and `8192.0001 mm` cast to `8192`. **Numeric.** Float representation must not grant a topology-degeneracy exemption.

## 5. Tests required and triggering inputs

Fixtures below are proposed OCCT surface/face or `Poly_Triangulation` constructions; **none was executed against OCCT here**.

| Finding | Input and necessary assertion |
|---|---|
| **F1–F2** | Valid planar `0.005×0.005 mm` face; test degeneracy independently of surrounding geometry at body diagonals `1`, `1000`, `10000 mm`. Inject missing triangulation after the meshing boundary—merely clearing it beforehand permits remeshing. Assert missing face, zero diagnostic range, `ok=false`; unchanged IDs/order. |
| **F2** | Degree-(3,3) polynomial bulge defined above, with collapsed boundary edges. Assert not certified degenerate because centre Jacobian is `4 mm²`. Also test the square-plus-strip counterexample. |
| **F3** | Tensor-product quadratic/linear surface defined below; oracle uses exact arithmetic on stored binary control values, not another OCCT D1. Assert ≤0.1° or explicit unresolved status. |
| **F4** | `S(u,v)=((a u+(1−a)u²) mm, v mm,0)`, `[0,1]²`, `a=1.5×10^-9`; triangulation includes `u=0`. Rotate 45° about Z. Assert equivalent regular-normal decisions. Repeat physical scales and mm↔m conversion with all dimensional inputs converted. |
| **F5/F9** | Synthetic triangle `(0,0,0),(1,0,0),(2,1.5×10^-12,0) mm`, all cyclic permutations, plus one ordinary surviving triangle. Assert invariant classification; positive-area rejection cannot leave successful completeness. |
| **F6** | Intersecting spheres above: centres `(±0.00005,0,0) mm`, radius `0.1 mm`; shared circle in `x=0`, radius `0.0999999874999992 mm`; explicit tolerances `0.0001 mm`. Assert never tangent. |
| **F6** | Polynomial join below, with and without untrusted G1 metadata; additionally make two of three samples unavailable. Assert unknown unless whole-edge continuity is established; partial sampling cannot certify consistency. |
| **F7** | Oblique sphere radius `20 mm`, axis `(1,2,3)`, both orientations and both mirror representations. Reverse/cancel/remove pole facet witnesses. Analytic signed radial must remain unchanged or emission must fail explicitly. |
| **F8** | Cone `10/0/20 mm`, synthetic two-triangle apex sector `−2°,0°,2°`; shallow cone `10/0/0.349207695 mm`, 72 sectors. Every apex output must have split provenance and match its assigned incident normal. |
| **F10** | Missing-source unreferenced node alongside usable geometry; assert no arbitrary +Z serialization. Inject NaN into the test normal array; finite-normal validation must fail. |
| **F11** | Collapsed-row spline fan with increasing valence, including 10,000. Assert the documented work bound or deterministic split path, plus repeatable indices/provenance. |
| **§4** | Planar strips `1×0.00005 mm` and `1×0.01 mm`, explicitly assigned rail tolerances `0.0001 mm`; verify tolerance semantics before granting the former an exemption. Test annulus radii `1` and `1.00005 mm`. Test a `0.0001×0.01 mm` rectangle at X=`8192 mm` for float32 collapse refusal. |

**F3 exact arithmetic fixture.** Set, in mm:

`M=2^42`, `a=2^-11`, `L=2^-6`.

`S(u,v)=(M(u−1/2)²+a(u−1/2), Lv, a(u−1/2))`.

Trim `u∈[1/2,1/2+2^-24]`, `v∈[0,1]`: approximately `0.015625×0.015625 mm`.

Quadratic X poles are `P0=M/4−a/2`, `P1=−M/4`, `P2=M/4+a/2`, all exactly representable. Standard binary64 derivative-control evaluation gives:

`Q0=2(P1−P0)=−M+a`,  
`Q1=2(P2−P1)=M` after rounding,  
`Dx(1/2)=(Q0+Q1)/2=a/2`, whereas exact `Dx=a`.

With exact `Dz=a`, normal error is `atan(2)−45°=18.4349488°`. Computed derivative-span travel is `3.2539072×10^-11 mm`, exceeding the current floor `2.2097087×10^-11 mm`; derivative sine is 1. **OCCT’s actual evaluation order remains unverified.**

**F6 three-sample escape fixture.** Share edge `(10t,0,0) mm`, `t∈[0,1]`, between:

`S1(t,w)=(10t,w,0)`,  
`S2(t,w)=(10t,−w,w f(t))`, with `w∈[0,10] mm`,

`f(t)=(512/15)tan(1°)(t−1/4)(t−1/2)(t−3/4)`.

These are tensor-product polynomial surfaces. Dihedral is zero at `t=1/4,1/2,3/4`, but **1° at t=1/8**; displacement there at `w=10 mm` is `0.174550649 mm`.

## 6. Verified versus inferred

**Verified — source inspection or shown computation**

- Publication behaviour, predicate branches, facet sign override, averaging, triangle deletion and completion logic.
- All numerical counterexamples and conditional error calculations above.
- Historical logs record 19 baseline failures, focused **45/45**, full **197/199**. They do not establish the new adversarial cases. [Baseline](docs/qa/viewport-hardening/baseline/ctest-surface-normals-red.log), [focused/full record](docs/qa/viewport-hardening/runs/wp09-focused/ctest.log).
- The inspected density test uses **coarse and fine**, not three deflections. The spline oracle shares OCCT’s evaluator; existing completeness tests endorse both attacked behaviours.
- Replacement specified; files unchanged because access is read-only.

**Inferred, conditional or missing**

- Actual OCCT D1 failure on the dyadic fixture; derivative implementation/order was not inspected.
- OCCT construction/meshing outcomes for newly proposed pathological faces.
- Authoritative sub-tolerance face-collapse semantics and applicable tolerance composition.
- A certified general D1 error contract, arbitrary trimmed-face collapse certifier, or singular-valence bound.
- Reported timing and display/export triangle-count measurements were supplied in the packet, not reproduced.

## 7. Where no defect was established

**Remaining-transform mirror algebra: sound under its stated placement contract.**  
For nonsingular A,

`(Au)×(Av)=det(A) A^-T(u×v)`.

Thus applying `sign(det A)` to adaptor derivative crosses, and reversed-XOR-negative-determinant to original triangulation winding, is correct. A mirror baked by `BRepBuilderAPI_Transform` must not trigger another operation-name-based flip. Existing tests cover retained and baked mirrors. [Tests](worker/tests/test_surface_normals.cpp:583).

**Remaining proof gap:** establish that the placement parity used for normals matches the adaptor’s actual remaining placement, while winding uses the triangulation’s placement; assert those representation postconditions explicitly.

**Uniform scaling/unit conversion alone does not invalidate dimensionless floors.** Positive finite derivative-span and triangle-ratio inequalities are homogeneous when dimensional inputs transform consistently. Their failures are conditioning, rotation-dependent bbox scale, invalid defaults and unsupported bounds—not dimensional mismatch alone. The old body-area ratio is also homogeneous; its defect is dependence on unrelated body geometry.

**No legitimate display-detail omission qualifies in this pass.** Checked NUM’s zero-range exception, spec §14 and fixed D11: mesher failure plus small size grants no omission permission.