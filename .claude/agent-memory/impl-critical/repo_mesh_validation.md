---
name: repo-mesh-validation
description: WP04 mesh validation/admission facts — buildBodyObjects' caller fan-out, the parser/validator boundary, and what the real worker's MESH1 bytes actually guarantee
metadata:
  type: project
---

`buildBodyObjects` (`src/viewport/mesh/meshRegistry.ts`) has ~30 call sites across 15 files —
`src/viewport/engine/*.test.ts`, `src/tools/modelTools/**`, `src/ipc/promote.test.ts`, and the
production preview lanes `ModelToolController.applyPreviewBodies` and
`modules/library/placementController.onPreviewResult`. A signature change there is never a
one-package edit.

**Why:** WP04's brief scoped ownership to `src/viewport/mesh/**` while requiring
`buildBodyObjects` to take a branded `ValidatedMesh`. Both cannot hold, so the parameter is
`ValidatedMesh | BodyMeshView` and the raw-view arm is validated in place
(`requireValidatedMesh`, throws `MeshValidationError`).

**How to apply:** before widening or narrowing that signature, grep the whole of `src/` — and
prefer validating inside over changing every caller when another work package owns the files.

Facts the validator depends on, verified against `worker/src/tess/Tessellate.cpp`:

- Worker normals are explicitly `Normalize()`d with a `(0,0,1)` fallback below 1e-12, so a
  `[0.999, 1.001]` unit-length window is safe on real bytes.
- The MESH1 header bbox comes from `BRepBndLib::Add` (B-Rep bounds) computed BEFORE
  `BRepMesh_IncrementalMesh` runs, so on a RE-tessellation OCCT can derive it from a coarser
  triangulation still attached to the shape and the fine nodes then sit OUTSIDE it by up to the
  coarse deflection. Strict enclosure would false-reject real bodies: MESH1 v1 measures the
  excursion and prefers measured bounds; v2 restores strict enclosure with a declared
  quantisation allowance (NUM §9.3, WP10). On a void box it stays `{0,0,0}`.
- `parseMeshPayload` already cross-checks `FACE_ID_CHARS.byteLen == faceIdOffsets[F]`, so a moved
  TERMINAL id offset surfaces as the parser's `bad-length`, never the validator's `id-offsets`.
- `FACE_COLORS` (type 12) is the one section a wrong length only downgrades (warn, colours
  dropped) — exclude it from any "corrupt a section length" negative test.

`viewportStore.setStatusHint` severities are `"info" | "warn" | "error"` — there is no
`"warning"`.

Preview lanes are errors-as-values, never throws: `ipc/localSolver.ts` `emitPreviewResult` fires
its listeners from a `setTimeout`, so anything thrown inside `ModelToolController.applyPreviewBodies`
or `library/placementController.onPreviewResult` becomes an unhandled exception rather than a
caught failure. Both go through `viewport/mesh/previewMesh.ts` `validatePreviewMesh`, which returns
null and logs once per (bodyId, code).

`prismPreview.addWall` emits a ZERO-length normal for any zero-area ring — coincident points or
merely collinear ones — because it divides by `Math.hypot(nu, nv) || 1`. That is the real payload
that makes semantic validation reachable from a drag.
