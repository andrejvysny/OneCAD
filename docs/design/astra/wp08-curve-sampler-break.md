# wp08-curve-sampler-break
date: 2026-09-13
mode: break
model: gpt-6-astra
effort: xhigh
access: grounded: worker/src/tess/CurveSampler.h, worker/src/tess/CurveSampler.cpp, worker/src/tess/Tessellate.cpp, worker/tests/test_curve_sampler.cpp, docs/viewport-hardening/03-NUMERICS-AND-PROTOCOL.md, docs/viewport-hardening/01-SPECIFICATION.md, docs/viewport-hardening/reference/design-math-checks.md, docs/qa/viewport-hardening/runs/wp08-focused/ctest.log
packet: sha256 96ad6116d09d
calls: 1 of ~3 for WP08; session 01a09bb1-4f33-7512-b342-3e15bae38282 recorded for followup
verdict: defective — reachable positive-weight curves receive false angular certification, valid stationary curves cannot terminate, and the float32 policy can erase accurately representable edges
verified by: orchestrator recomputation in Python (F1 tangent at t=1e-6 = 89.873°, hull bound 0.018 mm; F2 Bernstein coefficients of Q for ((2t−1)³,0) = [6, 1.2, −1.2, −1.2, 1.2, 6] with a root at t=0.5; F4 allowance at 1e6 mm = 0.4768 mm vs float32 spacing 0.0625 mm and three-axis half-ulp 0.0541 mm; F5 encoded chord rotation 71.565°; F7 closure threshold 1.42e-5 mm > represented gap 1.00e-5 mm); the local adversarial reviewer independently reproduced the interior-cusp collapse (2 points, 0.750 mm at t=0.5), the shifted full-circle failure (19.18 mm endpoint disagreement → 0 points) and the seam-crossing periodic B-spline failure (10.02 mm → 0 points) by compiling probes against the built worker library; `ctest -R curve_sampler` 1/1 and the 43-target pattern 43/43 confirmed before the fix round
transcript audit: 28 read-only commands; every path touched is in the allowlist (8 of 8), nothing outside it
rejected: F8's implication that an OCCT `GeomConvert_ApproxCurve` result could hide the described bump — accepted only as a limitation of the independent sample check (the derivative/curvature-driven refinement of NUM §2.7 step 5 is still owed), not as a demonstrated approximation escape; F10 is accepted as a contract statement (canonical increasing-parameter polyline order, orientation-blind, unchanged from the retired sampler), not as a defect

## 1. Verdict

Defective. Reachable positive-weight curves receive false angular certification, valid stationary curves cannot terminate, and the float32 policy can erase accurately representable edges.

## 2. Findings, ranked by severity

F1 — Blocker; proved + numeric. The 1e-9 relative significance floor hides substantial angular excursions. Quadratic P=[(0,0),(0,0.018),(20,0)] mm, w=[1e-6,1,1], tol 0.05 mm / 5°: q0 ratio 8.999996e-10 is skipped, every other coefficient passes, the span is accepted with an allegedly undefined start tangent; at t=1e-6 the tangent-to-chord angle is 89.87°. Scaling law: with P1.y=δ, chord L, weights [κ,1,1], the skipped ratio is ≈κδ/L, so the hidden excursion scales as ηL/κ, not ηL. Closure: exclude only coefficients proved zero; a positional insignificance concession must explicitly downgrade angular evidence (never `Satisfied`).

F2 — Blocker; proved + numeric. An interior stationary point becomes an unavoidable depth failure because `Undefined` is accepted only at exact source-span endpoints. Straight-locus cubic poles [(−1,0),(1,0),(−1,0),(1,0)] = ((2t−1)³,0): Q coefficients [6,1.2,−1.2,−1.2,1.2,6]·(1,0), root at t=0.5; every descendant touching the split boundary is rejected; at depth 32 the first pole is −2^-93 mm (not underflow). Genuine cusp [(3,−1),(−1,1),(−1,−1),(3,1)] = (3(2t−1)², (2t−1)³): endpoints-only output misses the cusp by 3 mm. Closure: recognise stationary roots at subdivision boundaries, preserve them as explicit singular vertices, certify each incident leaf separately.

F3 — Blocker; proved control-flow defect (native reproduction by the local reviewer). Full conic: conversion uses the whole basis independent of u1 (r=15 circle over [π/2,5π/2] disagrees by 15√2 = 21.21 mm → Failed). Periodic spline: after `SetNotPeriodic` the clamp to one period silently discards the wrapped part; a C1 three-arc periodic spline (period [0,3]) over [2,4] keeps only A2 while omitted A0 contains (40/9,20/9) mm, ≥4.44 mm from the retained polyline, with matching endpoints so the endpoint check cannot see it. Closure: split/unroll periodic intervals explicitly, preserve the requested starting phase, never repair a domain mismatch by clamping.

F4 — Blocker; proved + numeric. The representation allowance maxAbs·4·2^-23 rejects curves float32 represents accurately: it is 8/√3 = 4.62× above the bound ‖round32(p)−p‖ ≤ √3·2^-24·M; C(u)=(1e9, 15cos u, 15sin u) has an exactly representable X and Y/Z quantization below 1e-6 mm, yet the ladder emits two nearly coincident endpoints (30 mm loss). At 0.05 mm starvation starts at M=104 857.6 mm; all retries starve above 1 677 721.6 mm. Closure: measure rounding of candidate vertices before rejecting representability; never replace a qualified polyline with endpoints that carry no error bound.

F5 — Blocker for emitted angular quality; proved + numeric. Positional allowance does not bound float32 direction error: A=(100000,0,0), B=(100000.003,0.001,0) passes (allowance 0.0477 < 0.05) but encodes to a vertical segment, rotating 71.57° against a 5° request; angular perturbation depends on ρ/chord length. Closure: validate angular quality against the encoded chord or mark it unavailable.

F6 — Blocker for the mathematical input (native weight construction unverified). The 1e-6 weight-condition floor can refuse an exactly drawable line: collinear quadratic with normalised weights [r,1,1] needs depth 41 at r=1e-18 (raw weights [1,1e18,1e18]); r=1e-7 passes after 5 splits; the near-semicircle [1,r,1] pattern improves to ≈1/2 after one split. Closure: certify collinear monotone rational spans directly; otherwise use an arithmetic error estimate rather than a blanket ratio veto.

F7 — High; numeric. Endpoint snapping after certification: a line from 1e9 to 1e9+1e-5 mm (represented gap 1.0014e-5) is below the closure threshold 1.4211e-5 and collapses to one point; any snap displaces an endpoint after its leaf was certified (up to 1.25τ in the fallback budget). Closure: establish closure from curve/topology semantics, then recompute affected bounds against the final emitted endpoints.

F8 — Medium; proved limitation of the check. A smooth bump supported on (0.029,0.031) of height 0.1 mm produces zero sampled deviation at the initial samples 0.0215/0.0590; no refinement triggers at 0.05 mm. Own-segment distance ≥ nearest-segment distance, so the check cannot falsely pass a sample; the 1/4–3/4 split is sound under a uniform approximation bound. Closure: implement the specified derivative/curvature-driven refinement and never describe the logged tolerance as a proved achieved error.

F9 — Medium; source-proved. `max(body_segments_left,1)` grants every post-budget edge a segment and five sampling attempts; the fallback evaluation cap stops refinement rounds, not the leaf loop (7 000 leaves ≥ 231 000 evaluations). Closure: honour zero remaining budget while retaining zero-count identities; make evaluation exhaustion a typed incomplete check.

F10 — Medium; contract mismatch, accepted as a contract statement (see header).

## 3. Missing degeneracy classes

| Class | Detection predicate |
|---|---|
| Interior stationary point, cusp, turnaround | Q(t*)=0, 0<t*<1; classify one-sided directions separately |
| Small nonzero coefficient carrying direction | 0<‖q_i‖≤η·qmax; uncertainty must not imply zero |
| Weight boundary layer | predicted conditioning depth exceeds remaining depth; detect normalisation underflow separately |
| Periodic wrapping / displaced seam | requested interval differs from the converted domain or starting phase |
| Clustered / non-representable parameter interval | mid==t0 or mid==t1, non-finite midpoint, or an interval filtered by PConfusion despite geometric extent |
| Quantization collapse / rotation | encoded endpoints coincide, or the encoded chord violates the tangent cone |
| Endpoint / junction movement | final segment endpoints differ from those used to compute its bound |
| Open near-closure | small endpoint distance with distinct semantic endpoints; distance alone cannot establish closure |
| Cancellation-conditioned Q | coefficient arithmetic uncertainty reaches the cone/sign margin; recentring poles reduces translation-induced cancellation (translating a quadratic by (1e5,1e5,0) mm moved its Q coefficients by up to 1.65e-11) |

## 4. Tests required

F1: the weighted quadratic via `Geom_BezierCurve`; independent quotient derivative at t=1e-6; accepted angular evidence must cover it; δ on both sides of the floor. F2: both cubics; completion, an explicit t=0.5 singular boundary, independent chord bounds. F3: shifted full-circle trim; the A0–A2 periodic spline trimmed [2,4]; assert adaptor/conversion domains first, then full coverage, seam preservation, error at A0(1/3). F4: r=15 YZ circle at X=1e9 through MESH1; resolved circle, measured quantization, no endpoints-only replacement. F5: encode the stated line; compare encoded chord direction with the analytic tangent. F6: weighted collinear quadratic; OCCT accepts the weights, rational path retained, exact line without a conditioning-cap refusal. F7: open near-closure and endpoint-displacement fixtures. F8: compact bump against the check; estimated provenance and incomplete-check handling. F9: exhaust the body budget before another edge; count fallback evaluations. F10: reverse an edge and assert the chosen traversal convention.

## 5. Verified versus inferred (Astra's lists, unchanged)

Verified: source predicates and integration branches; numerical examples computed independently; hull coverage sound in exact arithmetic (segment-to-curve coverage by continuous projection); exactly-zero interior Q coefficients harmless, negative-parallel ones harmless only when excluded by a valid bound; the conic π/2 cone pre-test is not a parameter-angle measurement (ellipse (100,1) over 30°–150° passes with 1.98° tangent spread — harmless downstream); one transform on exact poles and none additionally on fallback poles, reflections preserve the predicates; no id cardinality/order corruption or same-build nondeterminism.

Inferred / unverified: native OCCT acceptance and precise domains for the F3/F6 constructions (the local reviewer subsequently reproduced F3 natively); any real OCCT fallback whose `MaxError()` misses the F8 bump; cross-platform byte determinism; a floating-point hull certificate needs arithmetic and endpoint-edit error accounting beyond the exact convexity proof.

## Disposition

All findings F1–F7 and F9 became red-first fixes in the WP08 fix round (ledger `docs/viewport-hardening/execution/STATUS.md` §6 DEV-WP08-1/2 and §7). F8 became the owed NUM §2.7 step-5 refinement. F10 became a written contract in `CurveSampler.h`.
