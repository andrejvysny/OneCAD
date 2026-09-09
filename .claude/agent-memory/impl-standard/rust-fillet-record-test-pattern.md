---
name: rust-fillet-record-test-pattern
description: How to build a Rust real-worker integration test that drives Sketch/Extrude/Fillet OperationRecords directly (no live solver) and reads step diagnostics
metadata:
  type: project
---

Pattern for a self-contained `src-tauri/tests/*.rs` file proving a worker diagnostic
(e.g. `FILLET_BLEND_APPROXIMATED`, SCHEMA §7.2/§7.3) survives into the document's
step diagnostics via `DocumentRuntime` + real worker.

- Each `tests/*.rs` file is a separate compilation unit — no shared `tests/common`
  module in this repo. Copy the harness (`real_worker()`, `spawn_worker`,
  `runtime_over`, `add_op`, `regen_all`, `published`) from `tests/topology_rebind.rs`
  or `tests/chamfer_reference.rs` rather than trying to import it.
- `SketchOpParams.entities`/`.constraints` are `Vec<serde_json::Value>` — you don't
  need to drive the live sketch solver (`AddSketch`/`enter_sketch`/`sketch_upsert`/
  `finish_sketch`) for a fixed, already-fully-defined sketch (e.g. a circle with a
  literal radius and center point). Build a typed `onecad_core::sketch::Sketch` with
  `SketchEntity::point`/`::circle`/`::line`, call
  `onecad_lib::worker::wire::sketch_wire(&sk)` to get `(plane, entities,
  constraints)` JSON, and drop the plane (build your own `SketchPlaneRef` from
  `sk.plane` + the matching `PlaneKind`). No solve needed since the entities already
  carry literal coordinates.
- `ExtrudeParams.profile` should be `Some(SketchRegionRef { region: RegionId::new(""),
  region_identity_version: None, region_anchor: None, .. })` for first-region
  fallback (V1) when you don't need a real region id — matches
  `tests/topology_rebind.rs::extrude_op`. Leaving `profile: None` compiles but skips
  the dependency-graph link to the sketch; safer to always set it.
- `SketchRegionRef` lives in `onecad_core::document::refs`, NOT
  `onecad_core::document::record` (only re-exported there internally).
- To fillet/chamfer a specific topological edge without scanning MESH1 buffers by
  hand: if you know the OCCT-assigned `TopoKey` (e.g. `"e:1"` — deterministic for a
  given op sequence + worker build, and pinned in NDJSON fixtures under
  `protocol/fixtures/`), call
  `ElementQuery::query_element_by_topo_key(&wm, snapshot, body, "e:1")` to get its
  `ElementInfoDto` (with `.center`), then `rt.promote_selection(snapshot, body,
  vec![(TopoKey::new("e:1"), Some(anchor))])` to mint the `ElementId`. This avoided
  needing a geometric mesh-scan edge-picker for a cylinder/cylinder tee seam — the
  topo keys from a fixture's NDJSON plan reproduced exactly under an equivalent
  Rust-built plan.
- `FilletParams` for a chain-tangent multi-edge fillet: `edge_ids: Vec<ElementId>` +
  parallel `edges: Vec<ElementRef>` (each with `primary`, `kind: Edge`, and an
  `anchor`), `chain_tangent_edges: true`, `tangent_closure_version: Some(1)`.
- Mesh fetch through `DocumentRuntime` is `rt.get_mesh(body, Lod::Coarse, None)`, not
  `MeshProvider::mesh(&wm, ..)` (that trait fn needs `<dyn MeshProvider>::` and a
  different `Lod` shape — `Lod` is a plain `{Coarse, Medium, Fine}` enum, no
  `default()`).
- `ModelSnapshot` has no `identity_version` field; its snapshot id is `.id.0` (`pub
  struct SnapshotId(pub u64)`).
- Step diagnostics after a regen: `rt.projection().features.iter().flat_map(|f|
  f.diagnostics.iter())` — same read path `tests/sketch_regions.rs` uses for
  `REGION_REBOUND_BY_ANCHOR`. `Diagnostic.evidence` is `Option<serde_json::Value>`.
