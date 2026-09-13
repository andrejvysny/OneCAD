---
name: repo-map-mesh-viewport
description: File locations and structure facts for the tessellation/MESH1/viewport pick-camera pipeline (not stated in CLAUDE.md)
metadata:
  type: project
---

Durable structural facts found while auditing the mesh/tessellation/viewport pipeline (2026-09-13), not already in CLAUDE.md:

- `worker/src/tess/` has 6 files: `Tessellate.{cpp,h}` (BRepMesh_IncrementalMesh drive + face/edge walk + Mesh1Input build), `Mesh1.{cpp,h}` (pure binary encoder, no OCCT deps), `MeshHandle.{cpp,h}` (not yet audited — likely worker-side mesh cache/handle wrapper).
- `Tessellate.cpp` has two near-duplicate code paths: `tessellate_body()` (per-body, with face/edge ids + partition-based ElementId labeling) and `tessellate_raw()` (STL-only export path, no ids) — normal/winding/location-transform logic is copy-pasted verbatim between them, not shared.
- Edge/face enumeration order comes from `TopExp::MapShapes(shape, TopAbs_FACE/EDGE, map)` into a `TopTools_IndexedMapOfShape` — 1-based OCCT canonical explorer order.
- `sample_edge()` in Tessellate.cpp has a hardcoded `kMaxDepth = 16` recursion cap; hitting it silently accepts the current chord/angle deviation (no error, no flag).
- MESH1 section type codes 1-12 are the only ones defined in `protocol/mesh_format.md` §4; codes 13-21 are undefined (not reserved explicitly, just absent — unknown types must be skipped per spec).
- `src/viewport/mesh/parseMeshPayload.ts` validates header/section-table structure exhaustively (magic, version, reserved bits, alignment, overlap, bounds, duplicate type, exact byteLen) but does NOT validate: finite floats, index-bounds (indices < vertexCount), range contiguity, or offset-prefix-sum monotonicity, or UTF-8. `onecad-protocol/src/mesh.rs::validate_mesh_blob()` mirrors the same structural checks Rust-side and also never parses section bodies (forwards verbatim).
- Mesh publication fencing uses `MeshProvenance {documentId, runtimeSession, snapshotId, generation}` (`src/viewport/mesh/meshRegistry.ts` + consumed in `meshSync.ts`) — no separate "epoch" field on this path.
- `MeshCache` (Rust, `src-tauri/src/mesh_cache.rs`) is keyed by `(BodyId, Lod, generation)`, LRU with dual bounds: `DEFAULT_CAPACITY=512` entries AND `DEFAULT_BYTE_CAPACITY=256MiB`.
- Camera constants: `CAMERA_NEAR=0.1`, `CAMERA_FAR=100_000` in `src/viewport/engine/CameraRig.ts`; `CAMERA_MIN_DISTANCE=0.5`, `CAMERA_MAX_DISTANCE=50_000` in `src/viewport/engine/cameraFit.ts`, consumed by `CadOrbitControls.ts::clampDistance()`.
- `meshSync.ts` (the `MeshIngest` class) hardcodes `DEFAULT_LOD: Lod = "fine"` for every committed-body fetch — no distance/zoom-based dynamic LOD selection in that file.
- No BVH/octree/spatial-index code exists anywhere under `src/viewport/` — `Picker.ts` raycasts brute-force over `traverseVisible()`-gathered face/edge object lists. `FACE_BBOXES` (MESH1 type 11, "pick accel" per spec comment) is parsed into `BodyMeshView.faceBboxes` but not consumed by any acceleration path found.
- Worker ctest naming: tessellation-related targets are `wp5_mesh1` (in-process MESH1 encode test, from `worker/tests/test_wp5_mesh1.cpp`) and `tessellation_quality` (`worker/tests/test_tessellation_quality.cpp`, coarse-vs-fine smoothness check). Both registered in `worker/tests/CMakeLists.txt` via the `foreach(_t ...)` pattern that auto-generates `add_executable`/`add_test` from a name list.
- Wire LOD is a string enum `"coarse"|"medium"|"fine"` in `Tessellate` request args (`protocol/SCHEMA.md` §7.6); the binary MESH1 header encodes the same tiers as u16 `lod` (0/1/2). "Tier" elsewhere in SCHEMA.md refers to validation Tier A/B, unrelated to mesh LOD.
