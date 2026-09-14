---
name: repo-mesh-preparation-plan
description: PR-03A facts — the one layout/cost authority, buildBodyObjects' plan parameter and why it is last, the reserve-after-colour-resolve trade-off, and the pre-existing red modelTools lane
metadata:
  type: project
---

`src/viewport/mesh/meshPreparationPlan.ts` is the SINGLE authority for a mesh's GPU/CPU layout and
its byte cost. `needsVertexColors` lives there (moved out of `faceColors.ts`); `validateMeshView`'s
`MeshAccounting` is payload-only — it no longer carries `colorBytes` or `estimatedGpuBytes` —
and `MeshAdmission.reserve` takes `MeshResourceCost {cpuBytes, gpuBytes}`, which only a
`MeshPreparationPlan` satisfies. `preparedCpuBytesOf` is gone.

- `buildBodyObjects`'s `plan` is the LAST optional parameter, not the second the design named:
  ~30 call sites across 15 files (many owned by concurrent tasks) make a positional insert
  impossible. Omitting it derives the plan from the same function, so there is still one
  predicate. DEV cross-checks a supplied plan against the appearance and the built attribute
  bytes against the priced ones (`logError("vp", "mesh built off-plan" | "mesh plan does not
  match the appearance …")`). `import.meta.env.DEV` IS true under vitest — the positive-control
  tests in `meshRegistry.test.ts` prove the assertions fire.
- `sourceRetainedBytes` is the WHOLE ArrayBuffer (distinct buffers summed once), because a
  zero-copy `Float32Array` pins the blob, not its slice. `parseMeshPayloadBundle` shares one
  buffer across bodies, so each of them is charged all of it — deliberate over-count.
- `meshSync.loadBody` reserves AFTER `await resolveAuthoredFaceColors`, since the colours decide
  the layout. Consequence: a job parked on that round-trip holds its payload ArrayBuffer
  UNCHARGED. Two tests encode this (TEST-PUB-01, "pending-replacement"); do not "fix" them back.
- The mock box is a trap fixture for layout arithmetic: V = 4·T makes indexed and de-indexed GPU
  bytes coincidentally equal on the FACE attributes. Assert against `view.buffer.byteLength` and
  the edge expansion too, or use the synthetic 1,000,000-triangle view in
  `meshPreparationPlan.test.ts`.

`src/tools/modelTools` is PRE-EXISTING RED at HEAD `61dab320`: 437 failures across 32 files, 316
of them `TypeError: this.client.onDocumentChanged is not a function` — the controller subscribes in
its constructor and those harnesses' client mocks predate it (plus `clearPreviewBody`,
`setOrbitSuppressed`, `dispose` gaps). Do not host a new test in one of those files; give it its own
harness (see `ModelToolController.previewAdmission.test.ts`) and do not "repair" the old mocks
unless that is the task.
