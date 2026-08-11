# Phase 5 Specification — Robustness Breadth

Duration: 4–6 weeks  
Prerequisites: Phase 3 publication policy and Phase 4 coverage manifest  
Priority: P1/P2 robustness  
Gate name: `KERNELBENCH-BREADTH`

## Rationale

Kernelbench is sophisticated but protects Fillet only. Boolean, Chamfer, Shell, OffsetFace and Hole use OCCT algorithms with equally meaningful failure surfaces. The current Fillet M3/M4 groundwork should be completed, then reused rather than allowing a parallel benchmark framework to emerge.

## Goals

1. Complete the existing M3 metamorph execution set.
2. Complete M4 recipe-agnostic validators.
3. Add Boolean as the first non-Fillet operation family.
4. Add Chamfer second.
5. Add small characterization slices for Shell, OffsetFace and Hole after publication policy is stable.
6. Gate m1 semantics and record stable-host Linux digest rows.

## Non-goals

- Production Fillet rescue strategies.
- Variable-radius, G2, chord-width or corner Fillet.
- Massive random fuzzing without minimization.
- Cross-host digest equality.
- Windows resource limits beyond Phase 6 smoke.

## Work package 5.1 — M3 metamorph execution

Implement the already schema-expressible variants:

- mirror,
- uniformScale,
- farOriginTranslation,
- parameterEpsilon,
- edgeOrderPermutation,
- contourSeed.

### Requirements

- Geometry transforms carry an inverse for signature comparison.
- `parameterEpsilon` changes the requested parameter, not geometry.
- selector-order variants alter selection order only.
- evidence remains genuine shape signatures, never proxy booleans.
- T0 existing records remain unchanged where the preset does not request new variants.

Evidence: `TODO.md:674-679`.

## Work package 5.2 — M4 generic validators

Implement:

- supportTangency,
- crossSectionProfile,
- noSelfIntersection,
- manifold,
- toleranceGrowth,
- microTopology.

### Requirements

- Support tangency handles plane, cylinder and cone without assuming box geometry.
- Cross-section profile samples a defined local frame and compares the requested law.
- Tolerance growth is evidence first; required ceilings are per case.
- Micro-topology reports slivers and short edges with normalized scale.
- `notApplicable` fails a required check.

Retire `cylindricalRadius` and `g1BoundaryTangency` from cases where their assumptions do not hold; do not delete them from frozen v1.

## Work package 5.3 — Boolean foundation campaign

### Operation cases

For Union/Cut/Intersect:

- overlapping boxes,
- disjoint boxes,
- containment,
- face/edge/vertex touching,
- near-coincident faces,
- thin sliver overlap,
- cylindrical cut and common,
- large-coordinate far-origin,
- scale bands,
- expected-limit cases with measured refusal boundaries.

### Backends

- raw OCCT BOP,
- OneCAD `checked_boolean` plus publication policy.

### Validators

- status and failure taxonomy,
- Tier B publication evidence,
- exact or analytic volume where available,
- material change,
- remote supports unchanged,
- deterministic body lifecycle and split ranking,
- metamorphic equivalence,
- timeout/RSS limits.

### Verdict asymmetry

- raw pass → OneCAD fail on a supported case is a regression unless OneCAD's stricter policy identifies a proven invalid result.
- raw invalid pass → OneCAD safe refusal is a success.
- OneCAD invalid publication is always a gate failure.

## Work package 5.4 — Chamfer foundation campaign

Cases:

- equal-leg and two-distance,
- convex/concave edges,
- short edges,
- high valence,
- tangent contours,
- plane/cylinder support combinations,
- radii/distances near limits,
- edge-order permutations.

Backends:

- raw `BRepFilletAPI_MakeChamfer`,
- OneCAD Chamfer executor or extracted production builder without copied logic.

## Work package 5.5 — Small additional slices

Only after operation publication semantics are stable:

- Shell: thin walls, adjacent removed faces, concavity, near-thickness limit.
- OffsetFace: planar/cylindrical, tangent closure, collapse, self-intersection, total/radius/diameter.
- Hole: blind/through, bridge disconnection, countersink/counterbore, near-boundary seating.

These begin as characterization, not required supported cases, until case-specific acceptance semantics are measured.

## Work package 5.6 — CI and baselines

- Add `fillet/matrix:m1` semantic comparison to a suitable gate or scheduled campaign.
- Record `linux-x64` m1 digest rows on the stable self-hosted host.
- Add Boolean T0 to trusted Linux and macOS semantics.
- Add Chamfer T0 after its first baseline review.
- Keep raw result artifacts for a bounded retention period.

## Case promotion discipline

A failure becomes a regression case only after:

1. reproducible on the stable host,
2. minimized,
3. classified as supported, expected-limit or exploratory,
4. semantic expectation reviewed,
5. no duplicate existing case,
6. source and OCCT build provenance recorded.

M6 minimizer/promotion infrastructure from the existing Fillet roadmap can follow after Boolean/Chamfer foundation is useful. Do not build the dashboard first.

## Metrics

- safe-refusal rate,
- raw-pass/OneCAD-fail regression count,
- invalid-publication count,
- replay instability,
- metamorph failures,
- P50/P95 and timeout count,
- tolerance-growth distribution,
- sliver/micro-edge distribution,
- minimized regression count.

## Acceptance gates

- M3 variants execute with genuine evidence.
- M4 validators are required on curved-support cases.
- Boolean and Chamfer have both raw and OneCAD backends.
- No T0 regression.
- m1 gating failures remain zero.
- Baseline changes are reviewed case by case.
- Same-host digest and cross-host semantic responsibilities remain separate.

## Risks

- Benchmark extraction can accidentally copy production logic and make differential tests tautological.
- Broad campaigns can freeze accidental behavior before product semantics are decided.
- Performance timing on the 4-core LXC cannot be compared directly with the Mac.
- Unbounded case growth can become a maintenance burden; require minimization and classification.

## Rollback

New operation families are additive. Existing frozen Fillet v1 cases and baselines must remain untouched except for reviewed manifest format migrations.
