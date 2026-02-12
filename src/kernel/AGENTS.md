# Kernel / ElementMap Guidelines

## Scope & Build
- Kernel currently contains only the header-only ElementMap at src/kernel/elementmap/ElementMap.h; CMake target onecad_kernel is INTERFACE and not linked into OneCAD yet.
- OCCT is required for compilation; linking is intentionally deferred (keep CMake linkage commented until kernel gains compiled sources).

## ElementMap Overview
- Purpose: persistent topology IDs across OCCT ops via descriptor hashing + OCCT history (Modified/Generated/Deleted).
- Reverse binding map shapeToIds_ uses TopoDS normalized to TopAbs_FORWARD; orientation-sensitive shapes must be normalized before lookup.
- Not thread-safe; callers serialize access.

## Descriptor & Hashing (regression sensitive)
- Descriptor fields (14): shapeType, center, size (bbox diagonal), magnitude (area/length/volume), surfaceType, curveType, normal (optional), tangent (optional), adjacencyHash.
- Quantization in makeKey(): positions/size/magnitude/dir at 1e-6; stableHash uses FNV-1a seeds (14695981039346656037, multiplier 1099511628211). Do not change quantization or seeds without updating regressions.
- Face adjacencyHash: FNV over quantized edge lengths (1e-6). Edge adjacencyHash: FNV over length, vertex offsets from center, vertex count. Keep ordering/sorting behavior.
- computeDescriptor uses BRepBndLib, BRepGProp, BRepAdaptor_*; ensure shapes are valid and inexpensive enough before calling.

## ID Scheme
- ElementId is a string; hierarchical IDs use bodyId as prefix (bodyId/face-*, edge-*, vertex-*).
- Child IDs come from makeChildId(): suffix face|edge|vertex|body + reason (split|gen) + optional opId + stable hash + ordinal. Hash comes from descriptor key; altering key or quantization will remap all IDs.
- Sources vector tracks parentage; update() and rebindBody() append sources only once per ID.

## Update Flows
- registerElement(): compute descriptor from shape and bind immediately; registerEntry(): descriptor-only (shape may be attached later with attachShape()).
- rebindBody(bodyId, shape, opId): matches existing IDs to faces/edges/vertices of new shape via score() (center/size/magnitude + penalties for type/normal/tangent/hash). Unmatched entries lose shapes; new generated children are appended with opId and body as source.
- update(BRepAlgoAPI_BooleanOperation& algo, opId): consumes OCCT history (Modified/Generated/Deleted). Splits produce new child IDs; generated shapes infer kind from OCCT shape type. Returns deleted IDs. Provide algorithms that expose history; otherwise mapping will be stale.
- bindShape/unbindShape normalize orientation; ensure shapes are non-null before binding.

## Serialization
- Text format: header "ElementMap v1" then sorted entries; uses std::quoted, precision=17. Maintain ordering to keep diff-friendly output and deterministic hashes.

## Invariants & Gotchas
- Never change quantization constants, stableHash seeds, or descriptor composition without rerunning rigorous tests and updating expectations.
- Always null-check OCCT Handle<> before dereference; do not suppress OCCT exceptions (bubble or log).
- score() penalizes type mismatch heavily; if changing scoring weights, re-verify split/merge matching stability.
- normalizeShape() orients everything to FORWARD; passing reversed shapes will be treated as the same key after normalization.

## Testing
- For any ElementMap change: run proto_elementmap_rigorous.
- For descriptor or ID logic changes: also run proto_custom_map and proto_tnaming.

## Logging & Debugging
- Kernel is header-only and not wired into app runtime yet, so most diagnostics come from prototype binaries.
- For future kernel logs, use categories under `onecad.kernel.*` (recommended: `onecad.kernel.elementmap`).
- Log only key transitions to keep traces actionable:
- descriptor creation failures or exceptional OCCT states
- ID remap decisions in `rebindBody()` and `update()`
- deleted/generated element summaries (counts + representative IDs)
- Keep deterministic behavior: logging must not alter ordering, hashing inputs, or quantization paths.
- Prototype triage pattern:
- run relevant prototype
- capture stdout/stderr plus current `logs/<latest>.log`
- correlate failures with recent ID-remap/descriptor logs before changing scoring/quantization constants
