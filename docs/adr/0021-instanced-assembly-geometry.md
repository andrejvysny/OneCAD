# 0021 — Assembly geometry is instanced in the scene, local in MESH1

- Status: Proposed (2026-11-04). Delivery and qualification status belongs in
  Plane (ONECAD-342); this record does not establish implementation or gate
  completion.
- Date: 2026-11-04
- Related: ADR-0020 (occurrences), ADR-0022 (joint solver hosting).

## Context

MESH1 (`protocol/mesh_format.md`) carries geometry Z-up, right-handed, with no
transforms; "validation introduces no transforms or higher-precision
coordinates", and Rust must forward the worker's buffer byte-for-byte
(Invariant 5). ADR-0020 makes the same definition appear in many occurrences.
Copying a mesh per occurrence would multiply memory and bytes the worker
produced once. The 500k-triangle admission limit would otherwise be hit by
repetition, not by model complexity.

## Decision

1. **MESH1 is unchanged and stays in the definition's local frame.** No
   transform field, no per-occurrence mesh.
2. **`onecad-scene` gains an instance table:** `(mesh key, transform,
   occurrence path)` per instance. Meshes are keyed by definition regen, not by
   occurrence.
3. **Renderer** draws instances with a per-instance model matrix supplied
   through an instance vertex buffer. Normals are transformed with the
   rotation only; transforms are rigid (ADR-0020), so no inverse-transpose is
   needed.
4. **Picking** uses a transformed-AABB broadphase, then transforms the ray into
   the instance's local frame for the narrowphase and returns
   `OccurrencePath` + local `ElementRef`.
5. **Admission.** The 500k-triangle limit applies to **unique** triangles
   (one count per mesh key). Separate caps: 2000 instances, depth 8. These
   numbers are provisional until measured.
6. **Adversarial fixtures are required** before the caps are accepted:
   deeply nested, widely repeated, and mixed nested-and-repeated assemblies.

### Rejected alternatives

- **Pre-transformed mesh per occurrence from the worker.** Rejected: N times
  the bytes and tessellation, breaks "one regen per definition", and ties mesh
  keys to placement so a drag invalidates caches.
- **Adding a transform section to MESH1.** Rejected: the format is a
  wire contract for Rust and C++; placement is host state the worker does not
  own.
- **Counting admission over instanced triangles.** Rejected: rejects cheap
  repeated fasteners while admitting an unrepeated heavy part.

## Consequences

- A joint drag updates a transform and instance buffer, not geometry: no
  worker round trip, no mesh bytes change.
- Per-instance selection, highlight and hidden state need an instance-aware
  path in `onecad-scene` and the renderer; the existing body-keyed paths are
  not enough.
- Unique-triangle admission lets 2000 instances of a heavy mesh overrun
  draw cost. Draw-call and vertex-shader cost need their own measurement.
- Nothing here is implemented; no renderer or picking evidence exists.

## Open questions

- Are 2000 instances and depth 8 right once measured on target GPUs
  (including iPad, ADR-0019)?
- Is culling per instance enough, or are indirect draws needed?
- Do section views and hidden-line modes need per-instance state in
  `onecad-render`?
- Mirrored occurrences stay refused (ADR-0020); confirm that holds for
  fixtures from imported assemblies.
- Workbench and tree visual designs are supplied by the user; none are
  assumed here.
