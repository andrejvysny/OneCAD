---
name: repo-topology-ownership
description: How the worker's transient topology-owner ledger attributes blend-born boundary topology — the certificate, where the contour set really comes from, and what stays fail-closed
metadata:
  type: project
---

Facts about `worker/src/ops/TopologyHistory.*` + `worker/src/session/TopologyOrigins.h`
and the FeaturePattern producer ledger that sits on top of them.

**Why:** OCCT's blend history is incomplete by design, and every "why is this reference
Unknown" bug in FeaturePattern traces back to it.

**How to apply:**

- `BRepFilletAPI_MakeFillet`/`MakeChamfer` report ONLY the blend FACE as `Generated(seed edge)`.
  The blend face's own boundary edges and ALL its vertices appear in no `Modified` and no
  `Generated` list — `Generated(vertex)` is empty for every vertex. Those orphans are why
  `modified_body_history` takes a `BoundaryCompletionCertificate`.
- The certificate's `contour_edges` come from the builder's OWN `NbContours()/NbEdges(I)/Edge(I,J)`
  (`BRepFilletAPI_LocalOperation`), NOT from the `resolved.edges` vector the op fed it:
  `trim_to_seeds` cuts that vector to ONE seed per tangent contour and OCCT then propagates the
  chain internally. The field is now PROVENANCE only — the adapter's assertion that this op may
  complete at all — because witnesses are no longer contour-scoped.
- Witness faces are every live after-face `Generated` from ANY before root (face, edge OR vertex)
  that is not IsSame a before face and is not a `Modified` output of one — the derivation's
  F = {D(f) and P(f)=empty}. A contour-edge-only rule is WRONG: measured on a 10 mm cube blended
  r=1 on the three edges of one corner (OCCT 8.0.1), the corner patch is reported under
  `Generated(vertex)` only, and its exclusive seam edge stays Unknown. Numbers: 6->10 faces,
  12->22 edges, 3 edge-generated witness faces + exactly 1 vertex-only one, omitted edges
  13->0 and vertices 9->0 with the widened rule (13->1 with the contour-only rule).
  `Generated`/`Modified` may return references into reused storage — never hold two at once.
- Completion is fail-closed and runs ONLY as the final `else` after survivor/successor/birth/
  conflict, so it can never overwrite exact lineage. `accounted_vanishing` disqualifies the WHOLE
  op on one violation. NON-EMPTY HISTORY IS NOT ACCOUNTING (both reviewers caught this): every
  before FACE/EDGE gone from the result needs some x in `Modified(b) | Generated(b)` with
  `!x.IsSame(b) && after.Contains(x)`. A list holding only `b`, or only a shape absent from the
  result, is the unreported-replacement adversary wearing a history list. Boolean adapters
  (`HoleOp`, `ExtrudeOp`) pass no certificate and so never complete.
- `kReferenceKinds` order {FACE, EDGE, VERTEX} is load-bearing: vertex completion reads the edges
  the EDGE pass completed. Reordering it silently narrows completion (fail-closed, not wrong).
- `TopTools_IndexedMapOfShape` hashes by `TopTools_ShapeMapHasher`, whose `IsEqual` is `IsSame` —
  so `Contains`/`FindIndex` are exactly the IsSame predicate this code needs, no epsilon anywhere.
  `TopExp::MapShapes(S, T, M)` does NOT clear M, so it accumulates across several witness faces.
- Producer sets for a FeaturePattern instance are built by walking the LIVE body and taking the
  EFFECTIVE claim per IsSame class, never raw `entries()` rows — a shape with two disagreeing rows
  must resolve Ambiguous and never be exported as a producer output.
- `TopologyOwnerLedger::lookup` is an O(rows) LINEAR SCAN, so classifying every sub-shape of a body
  with it is quadratic. Use `effective_claims(body_id)` (one pass, hashed
  `NCollection_IndexedDataMap<TopoDS_Shape, OriginClaim, TopTools_ShapeMapHasher>`) whenever more
  than a couple of shapes are classified. Measured 2041 rows x 2042 shapes x 384 passes:
  3190 ms -> 13 ms, identical results. Absent key == Unknown, matching `lookup`.
- Retained-host support in `feature_pattern_bind_instance_output_refs` reads FROZEN evidence for
  every clause; the LIVE `topology_owners.lookup(body, existing->shape)` must also be Known with
  the SAME producer, or the binding is stale and must be `ownership-mismatch`.
- A Fillet-produced CIRCULAR edge is patternable only under a `Linear` layout. Descriptor centres
  are BOUNDING-BOX centres and `transform_descriptor` rotates them; a bbox centre is rotation-
  covariant only when the bbox is, and a quarter-circle arc's is not (r=0.5 at 45 deg moves the
  centre 0.073 mm vs a 1e-6 match tolerance), so a rotated instance rejects the intended arc and a
  perpendicular twin can match uniquely. Hole-produced circles are full circles whose bbox centre
  IS the centre, so they keep the rotational branch under any layout.
- In `execute_feature_pattern` the ORIGIN gate runs before the straight-edge capability refusal:
  an Unknown/Ambiguous input is `NeedsRepair` (`unknown-origin`/`ambiguous-origin`, code
  `FEATURE_PATTERN_PRODUCER_BIND`), and `UNSUPPORTED_OP` is reserved for a genuinely unsupported
  curve type whose producer IS known. The gate needs `nested` for its `refId`, so it must sit
  after `instantiate` — which is side-effect free on `job`.
- Seed geometry for the shared-host chain in `test_feature_pattern.cpp`: 200x200x5 host box plus a
  10x8 rect extruded 10mm lands the block at world x∈[-8,0], y∈[0,10] (sketch u→+Y, v→-X). A
  r=0.5 fillet on its vertical corner edge yields exactly 4 orphan edges and 4 orphan vertices;
  the pinned vector is docs/design/astra/feature-pattern-producer-ownership.md §6.
- Locate a blend arc by DESCRIPTOR centre (bbox centre), not by `BRepAdaptor_Curve::Circle()
  .Location()` — the two differ by the fillet radius (arc at corner (-8,10) r=0.5: descriptor
  centre (-7.75, 9.75), circle location (-7.5, 9.5)).
- The repair reason tokens `unknown-origin` / `ambiguous-origin` / `ownership-mismatch` are NOT in
  SCHEMA §9's closed `reason` set (pre-existing drift; Rust degrades them to Unknown). Keep new
  emitters consistent with the existing bind emitter rather than inventing a fourth spelling.
