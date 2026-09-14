---
name: repo-promotion-proof
description: How a viewport pick proves currentness before it becomes an ElementId — the two typed promote entry points, where the proof is captured, and what clears it
metadata:
  type: project
---

`src/ipc/promote.ts` has TWO entry points and no proof-free viewport form. `promoteOne` is deleted.

**Why:** the review finding PR-01 — the stale check only ran `if (proof && …)`, most callers passed no
proof, and the `el_` fast path returned before any check. So a click on a body whose replacement
failed (still drawn, `displayState = "stale-inspection-only"`) minted an id from an ordinal that
addresses a publication the document has moved past.

**How to apply:**

- `promoteViewportPick(client, proof, pick)` — anything the user CLICKED. Proof is mandatory and is
  checked (a) before the wire call, (b) BEFORE the `el_` short-circuit, (c) again after the reply.
  `bodyId` is read off `proof.entry.bodyId`, never passed in.
- `promoteAuthoritativeRef(client, bodyId, pick, snapshotId, origin)` — labels the BACKEND named.
  `origin` is a closed union (`history-repair` | `edge-op-closure` | `offset-closure`) that exists
  only so a Picker hit cannot be passed here; the explicit `snapshotId` is the whole fence.
- `promoteRef(client, ref)` looks up `pickProofFor(ref)` and FAILS CLOSED when there is none.
- `installedEntryIsCurrent(bodyId, entry)` is the BODY half (live doc, this publication, visible,
  promotable, provenance match); `installedProofIsCurrent` adds the ordinal-resolves half. Use the
  entry form for anything that is not an ordinal claim — a body candidate, a currency gate on a ref
  that already carries an `elementId`.

`src/viewport/mesh/pickProof.ts` owns `InstalledPickProof` (re-exported by `promote.ts`) and a
`WeakMap<EntityRef, InstalledPickProof>`. `EntityRef` must stay a plain serialisable record, and
selection already compares by object identity, so the proof rides the ref OBJECT, not a field.
`Picker.resolvePick` stamps `PickHit.entry` from the same lookup that decoded the ordinal —
`getEntry(bodyId)` at promotion time is the mis-bind, never do it.

Attach sites: `ViewportRoot.refFromHit` (every click/hover ref), `OverlapCandidateChooser.candidateRef`,
`promotePick`'s elementId write-back (carries the proof to the replacement object),
`SketchController.facePickFrom` / `tryEnterOnSelectedFace` (via `FacePickTarget.proof`).
`rebindPick.reconcileSelectionForBody` CLEARS the proof of every ref of the regenerated body — a
survivor carries an `elementId` and needs no promotion.

Read-only lanes: `viewportHitIsCurrent(hit)` (also in `promote.ts`) is the gate for code that QUERIES
geometry from a hit instead of minting an id — `placementController.onPointerMove` and
`attachmentPicker.onPointerDown`, both of which call `classifyElement`. `CadClient.classifyElement`
takes a `GeometryReadFence`, but the library module may only reach it through
`GeometryQueryService` (`modules/modeling/manifest.ts`), which deliberately does not expose one
(ADR-0002 narrow surface) — so those lanes REFUSE rather than fence.

Traps:
- ANY test that stubs `engine.probePick` must put a real registry `entry` on the hit, or every
  currency gate refuses. `placementController.test.ts`, `attachmentPicker.test.ts`,
  `SaveAsComponentDialog.test.tsx` and `SketchController.face.test.ts` all install a body for this.
- `meshSync.adoptPublication` must KEEP the previous publication when `change.changedBodies` is
  empty. Most `DocumentChange`s carry no geometry (`upsertVariable`, `renameVariable`,
  `deleteRecord`, a no-op `editOperationInput`), and clearing it makes `installedEntryIsCurrent`
  false for every body while those meshes are still drawn — every pick refuses and the sticky stale
  hint never clears. It still clears for a foreign/superseded change, and for one whose retained
  publication is about another document.
- `meshSync.loadBody`'s NOTIFY block runs every call-out through `announce()` (try/catch → logError),
  and the "Body failed to load" hint lives ONLY in the not-installed branch of the outer catch.
  `this.engine` must be captured into a local first — a closure cannot carry the COMMIT block's
  non-null narrowing.
- A COSMETIC republish (colour, visibility, `reconcile()` self-heal) swaps the entry WITHOUT
  reconciling the selection, so every selected ref's proof then names a retired entry. Gate
  promotion on the proof, but gate anything an `elementId` can answer on
  `installedEntryIsCurrent` instead, or a colour change breaks measure.
- `PromotePick.kind` is stripped by `tauriClient.promoteSelection`; adding it is not a wire change.
- `ModelToolController:4101` (chamfer reference face) and `:5165` (offset evidence) look like
  viewport sites but promote labels from `prepareEdgeOp` / `prepareOffsetFace` closures — including
  tangent-CHAINED faces nobody clicked. They are authoritative, not viewport.
- Fillet arming never promotes (the backend promotes the edges inside `prepareEdgeOp`); the
  controller's only ref-driven promotion site is `measurePick`.
