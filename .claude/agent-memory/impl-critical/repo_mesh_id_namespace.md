---
name: repo-mesh-id-namespace
description: MESH1 face/edge id tables mix TopoKeys with minted ElementIds, and only TopoKeys can be promoted — the measured cause of "Selection is out of date"
metadata:
  type: project
---

The MESH1 id table is a TWO-NAMESPACE table, and the promote lane accepts only one of them.

**Why:** `Tessellate.cpp`'s `minted_ids(partition, body_id)` builds a `topoKey → elementId`
map from every partition entry for the body, and `label()` substitutes the ElementId whenever
one exists (setting `IDS_HAVE_ELEMENTIDS`). `AcquireElementIds` → `resolve_pick`
(`session/ElementIdentity.cpp`) only parses `f:N`/`e:N` via `shape_for_topokey`, so an
ElementId-shaped `topoKey` falls straight through to the anchor rung and, with no anchor,
is DROPPED — which Rust's `promote_selection` then reports as a mismatched batch. The two
ordinal spaces themselves agree (both are `TopExp::MapShapes` 1-based indices); the
divergence is the NAMESPACE, not the ordinal.

**How to apply:**

- A face/edge is ElementId-labelled in the mesh from the moment ANY op references it: the
  op's own input ref is resolved and bound during the regen, so the publish-time
  tessellation already carries the id. Measured 2026-09-11 on revolve + 4 holes: the mesh
  named exactly ONE minted id at every snapshot — the Hole's host face — even though ~20
  unrelated ids had been promoted at the previous snapshot. A bulk promote does NOT survive
  a from-0 regen; an op reference does.
- The viewport hands the mesh string back verbatim (`Picker.ts` sets `topoKey: id` and only
  ALSO sets `elementId` when it starts with `el_`; `ViewportRoot.promotePick` → `promoteOne`
  sends it as the pick's `topoKey`). Re-picking any face an op has used USED to be refused —
  the user-visible "Selection is out of date — pick again". CLOSED 2026-09-11: `promoteOne`
  short-circuits an `el_`-prefixed pick (returns it as its own elementId, no wire call) and
  `DocumentRuntime::promote_selection` refuses a non-`f:`/`e:`/`v:` pick by name before the
  wire call.
- An `el_` pick ALWAYS failed promotion, anchor or not: even when `resolve_pick`'s anchor rung
  resolved it, the worker returns the REAL TopoKey, and Rust's `returned_keys != requested_keys`
  batch check then reports "an incomplete or mismatched batch". So refusing early is a strict
  improvement, never a regression, for every caller of `promote_selection`.
- CLOSED 2026-09-11 (WP-U4 G1): `ProjectionSource` (TS + `ProjectionSourceInput` in
  `api/mod.rs`, NOT OCW1) carries an optional `elementId`; `projectRequests` fills it from the
  pick and `project_to_sketch` addresses such a source by it and SKIPS promotion. Sources are
  now de-duplicated on (body, ADDRESS), and the un-promoted subset is what reaches
  `promote_selection`. `placeComponent` was already safe — `library.rs::resolve_mate_input`
  short-circuits on `mate.targetElementId`, which `Picker.ts` populates for an `el_` label.
- The pick-shape guard in `DocumentRuntime::promote_selection` is `TopoKey::is_valid()`
  (`<f|e|v|b>:<u64>`), not a prefix list — `f:` and `face:1` pass a prefix test and then cost a
  worker round-trip to resolve to nothing.
- The mock lane mirrors `resolve_edge_pick` through `mockClient.mockPickKey`: an
  elementId-addressed pick is resolved back to its TopoKey via `mockElementIdToTopoKey` (which
  `mockClient.promoteSelection` populates), because echoing the `el_` label back as the prepared
  edge's `topoKey` silently degrades the arm to its no-geometry tier. An id the mock never
  minted falls back to the raw label — so an e2e fixture must NOT seed an invented `el-…` id on
  a ref that an edge-op lane will address by it.
- The mesh a snapshot's viewport holds is the one tessellated AT PUBLISH TIME (drained from
  the regen's inline artifacts into `MeshCache`, keyed `(BodyId, Lod, generation)`), so a
  promotion made after a publish never changes that snapshot's labels — only the next
  publish's.
- Consequence for tests: `rt.get_mesh(body, lod, None)` re-fetches per generation, so
  "enumerate the head mesh's ids and promote each" is a faithful model of the viewport.
  `src-tauri/tests/selection_promote.rs` is that gate.
- Locating a face for a scenario must NOT read the mesh id table (that is the thing under
  test): scan `f:1..=face_count` with `ElementQuery::query_element_by_topo_key` and match on
  the descriptor (`normal` is UN-oriented; `center` is the bbox centre, which for a planar
  face lies on the plane).
