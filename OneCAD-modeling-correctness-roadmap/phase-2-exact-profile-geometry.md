# Phase 2 Specification — Exact Profile Geometry

Duration: 3–5 weeks  
Prerequisites: Phase 0 and the Phase 1A ownership/stale-response gate (work packages 1.1–1.3)  
Priority: P1 geometry correctness  
Gate name: `EXACT-REGION-BREP`

## Rationale

Loop detection tessellates arcs, circles and ellipses to discover intersections. Fragmented graph edges receive synthetic ids, so `FaceBuilder` cannot reconstruct the original curve and emits straight polygon segments. The UI and region detector may look smooth enough, but Extrude/Revolve receive a faceted BRep. This changes area, volume, curvature, downstream fillets and topological identity.

## Goals

1. Preserve analytic curve geometry through region splitting.
2. Keep region identity deterministic and independent of OCCT ordinals.
3. Introduce scale-aware intersection and gap policies.
4. Bound and cancel dense intersection work.
5. Preserve simple-region serialized behavior and existing ids.

## Non-goals

- General spline support.
- A full 2D computational-geometry rewrite.
- Sketch UX redesign.
- Changing named-plane bases.
- New sketch entities.

## Current mechanism

- Lines become exact segments.
- Arcs/Circles/Ellipses are tessellated into line segments for graph construction.
- Intersection splitting creates ids such as `<base>#segN_pM`.
- When all loop ids resolve to live sketch entities, `FaceBuilder` constructs analytic OCCT edges.
- When fragments do not resolve, it builds a polygon wire.

Evidence:

- `worker/src/loop/LoopDetector.cpp:811-883,891-995`
- `worker/src/loop/FaceBuilder.cpp:390-475`
- `worker/tests/test_region_table.cpp:251-315`

## Target representation

Introduce a curve-fragment descriptor independent of sketch entity ids and OCCT handles:

```text
CurveFragment {
  baseEntityId,
  curveKind,
  parameterStart,
  parameterEnd,
  orientation,
  startPoint,
  endPoint,
  generationVersion
}
```

Parameter domains:

- Line: normalized `[0,1]` or exact endpoint slots.
- Circle: unwrapped angle interval.
- Arc: stored parameter interval preserving authored orientation.
- Ellipse: parametric angle interval.

The region graph may still use tessellation for candidate discovery, but accepted intersections must be refined against the analytic pair before fragments are authored.

## Work package 2.1 — Exact intersection refinement

### Required pairs

- line-line,
- line-circle/arc,
- circle-circle/arc,
- line-ellipse,
- circle/arc-ellipse where OCCT or the existing math can return stable parameters.

Unsupported pair handling must be explicit. Do not silently fall back to a polygon for a supposedly supported analytic pair.

### Requirements

- Deduplicate tangency as one parameter per curve.
- Reject or characterize near-coincident infinite intersections.
- Preserve full-circle seam handling.
- Sort split parameters deterministically.
- Quantize only for identity/signatures, not for geometry construction.

## Work package 2.2 — Analytic FaceBuilder fragments

### Required behavior

`FaceBuilder` constructs each fragment from the base entity and parameter interval:

- trimmed line,
- trimmed circle/arc,
- trimmed ellipse.

Polygon fallback is permitted only for an explicitly unsupported future curve kind and must produce a structured diagnostic that can be surfaced. It must not be the normal path for Arc/Circle/Ellipse.

### OCCT direction

Use exact `Geom_TrimmedCurve` or matching OCCT edge constructors on the sketch plane. Preserve wire orientation explicitly.

## Work package 2.3 — Region identity compatibility

### Constraints

- Simple unfragmented regions retain current ids byte-for-byte.
- Fragmented-region identity must derive from base entity id plus normalized parameter intervals and orientation, not tessellation segment indices.
- Entity-order permutations produce the same ids.
- Generation version is explicit so a future algorithm can coexist with legacy documents.

### Migration decision

Existing documents persist `regionId`. If the current fragmented-region ids cannot remain stable, provide one of:

A. Legacy id alias table generated from the old algorithm during transition.  
B. Versioned region identity stored on newly authored profiles while old ids continue to resolve through the legacy detector.

Recommendation: versioned identity plus legacy aliases. Never silently bind a stale region to the first available region.

## Work package 2.4 — Tolerance policy

### Current risks

- Loop detection has fixed tolerances.
- FaceBuilder may repair gaps up to 0.1 mm.
- Tiny and very large models can receive inconsistent semantics.

### Required policy

Separate:

- geometric intersection tolerance,
- node merge tolerance,
- wire connection tolerance,
- maximum repairable gap,
- identity quantization.

Each tolerance has a named source and scale rule. Maximum gap repair should be conservative and diagnostics should record any actual repair magnitude.

The phase must not silently derive all thresholds from `Precision::Confusion()`; OCCT precision is one input, not product policy.

## Work package 2.5 — Resource and cancellation bounds

Current pairwise segment intersection is quadratic and tessellated curves can generate many segments.

Required controls:

- preflight entity/segment count ceilings,
- cancellation polling in pairwise loops,
- bounded diagnostic evidence,
- timing metrics by sketch complexity,
- no unbounded UI freeze on one pathological sketch.

## Red-first fixtures

1. Two overlapping circles: three regions, exact analytic areas.
2. Circle cut by a line: two semicircular regions with arc edges.
3. Two arcs crossing twice.
4. Ellipse cut by line at rotated orientation.
5. Tangent circle-line: one split point, no micro-edge.
6. Near-tangent pair across a tolerance sweep.
7. Entity-order permutation.
8. Translation/rotation/uniform-scale metamorphs.
9. Very small and very large scale bands.
10. Dense 100-curve cancellation and ceiling behavior.

## Assertions

For each relevant region:

- BRep valid,
- exact curve-type census,
- area within analytic tolerance,
- closed wire,
- no micro-edges below policy unless input contains them,
- deterministic region id,
- Extrude volume equals area × depth,
- Revolve volume agrees with Pappus where applicable.

## Performance budget

Set after measurement, but target:

- ordinary sketches under 5 ms for region detection,
- 95th percentile under 25 ms for interactive authoring,
- explicit refusal before 200 ms for over-ceiling cases,
- no regression greater than 20% on existing simple sketches without a documented reason.

## Acceptance gates

- All existing region and profile tests green.
- Simple-region ids unchanged.
- New analytic fixtures green at C++ and real-worker Rust layers.
- Extrude/Revolve previews use the exact BRep.
- No polygon fallback warning for supported analytic entities.
- Full suites and T0 unchanged.

## Risks

- Region ids are user-visible persistence dependencies.
- OCCT intersection routines can return unstable near-tangent parameter sets.
- Exact arcs can expose downstream topology differences previously hidden by polygons.
- A compatibility alias path can become permanent debt; version it and set a removal criterion.

## Rollback

Keep the old detector as `regionIdentityVersion:1` during rollout. New authoring can opt into v2 behind a development flag until corpus and persistence gates pass.
