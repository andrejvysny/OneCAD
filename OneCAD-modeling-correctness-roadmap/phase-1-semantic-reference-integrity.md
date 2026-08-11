# Phase 1 Specification — Semantic-Reference Integrity

Duration: 4–6 weeks; core ownership and stale-response work should gate after 2–3 weeks  
Prerequisite: Phase 0  
Priority: P1 identity safety  
Gate name: `REF-OWNERSHIP-AND-SNAPSHOT`

## Rationale

OneCAD's identity policy is conservative, but some authoring and operation paths lose the body or snapshot provenance that makes conservative resolution meaningful. The fix is not looser scoring. It is stronger ownership and snapshot contracts before the ladder runs.

## Goals

1. Enforce operated-body ownership for every semantic ref that must belong to a target body.
2. Refuse stale authoring evidence rather than degrade to anchor-only records.
3. Fence read-only preparation results at response adoption time.
4. Make repair candidates snapshot-scoped in both data and cache identity.
5. Clarify the dormant Revolve body-edge axis contract.
6. Preserve all current score, margin and tie-veto policy.

## Non-goals

- Closing the accepted ordinary-edit teleport residual.
- Persisting a complete OCCT history graph across from-zero replay.
- Changing resolver thresholds or weights.
- Adding new repair UI concepts beyond provenance and invalidation.

## Invariants

- A face/edge TopoKey is body-scoped and snapshot-scoped.
- A semantic ref intended for an operation target cannot change body through descriptor fallback.
- A stale promotion cannot be converted into an anchor-only authored ref.
- A repair candidate cannot be promoted outside the snapshot that produced it.
- `NeedsRepair` remains state, never a fatal Rust error.

## Work package 1.1 — Common body ownership

### Affected operations

- Fillet
- Chamfer
- Shell
- Hole
- OffsetFace

Extrude ToFace is different: its target face may intentionally belong to another body than the Boolean target. It still needs a real body id, but not equality with the operated body.

### Required behavior

Fillet/Chamfer:

- every typed edge primary must have `kind=edge`,
- every edge must have the same `primary.body`,
- the common body becomes the target,
- mixed-body refs fail before any identity or descriptor scoring.

Shell/Hole/OffsetFace:

- every operative typed primary body equals targetBodyId,
- the worker independently rechecks this for hand-authored plans,
- a foreign entry does not fall through and rebind onto congruent target geometry.

### Implementation direction

- Extend Rust validators.
- Retain primary body in the worker-side parsed ref or validate directly from input JSON before constructing `LadderRef`.
- Do not make generic `resolve_descriptor_stage` reject all body mismatches: some use cases resolve refs on their own named body. Ownership belongs in the caller's contract.

### Red-first tests

- Two congruent boxes; select one edge from each; Fillet/Chamfer must fail atomically.
- Foreign Hole/Shell/OffsetFace ref with the same ordinal and descriptor on the target.
- Intent-only legacy ref behavior remains classified and does not invent a body.
- All failures leave body volume/signature and partition unchanged.

Relevant source:

- `worker/src/ops/FilletChamferOp.cpp:42-52,102-174`
- `worker/src/elementmap/Ladder.cpp:333-367`
- `src-tauri/crates/onecad-core/src/edit/session.rs:1655-1703`
- `src-tauri/crates/onecad-core/src/document/record.rs:1145-1184,1750+`

## Work package 1.2 — Fail closed on stale promotion

### Defect

Extrude ToFace and Hole continue after `promoteOne` returns null, storing body + anchor without a persistent ElementId or descriptor.

### Required behavior

When promotion fails:

- remain in face-pick mode,
- preserve the last valid preview,
- display the stale-selection hint,
- do not author or preview a new ref,
- require a fresh pick on the current head.

### Tests

- Hold promotion, advance snapshot, release old response.
- Assert no operation params are sent.
- Re-pick on new snapshot and succeed.
- Repeat for ToFace direction 1, ToFace direction 2 and Hole.

Relevant source:

- `src/tools/modelTools/ModelToolController.ts:1681-1709,4291-4323`
- `src/ipc/promote.ts`

## Work package 1.3 — PrepareOffsetFace response fence

### Required behavior

Mirror `PrepareEdgeOp`:

1. capture admitted snapshot id,
2. send it,
3. require response snapshot equals admitted snapshot,
4. require current frontend head still equals admitted snapshot,
5. otherwise reject and re-pick.

No face promotion or stored closure may occur after a failed fence.

### Tests

- Undo while request is pending.
- Upstream edit while request is pending.
- New document while request is pending.
- Response snapshot mismatch.
- Same-head normal path unchanged.

Relevant source: `src/ipc/tauriClient.ts:1525-1570`.

## Follow-up milestone 1B — Repair provenance and dormant-axis contract

Work packages 1.1–1.3 form the first 2–3 week gate. The following two packages are independent cross-layer follow-ups and should not hold ownership/fence fixes hostage.

## Work package 1.4 — Snapshot-scoped repair candidates

### Required data model

Each dry-run resolution result and candidate set must carry:

- snapshotId,
- document revision or repair-event revision,
- refId,
- bodyId used for candidate enumeration.

The candidate cache key is `{revision, snapshotId, refId}`.

### Required behavior

- Any newer repair event invalidates older candidate loads for the same ref.
- Older out-of-order events do not replace newer store state.
- Candidate promotion passes the candidate snapshot id, not the transport's current snapshot by default.
- Snapshot mismatch forces reload; it never reinterprets the same ordinal on the new head.

### Protocol impact

`ResolveRefs` already accepts snapshotId. Additive result echo is sufficient. Update schema and cross-track fixtures if the response shape is normative.

### Tests

- Load candidates at N; publish N+1; old rows disappear.
- Click an old candidate after N+1; no promotion occurs.
- Same refId recurring across revisions does not reuse the old cache.
- Out-of-order needs-repair event is dropped.

Relevant source:

- `src/features/repair/RepairPanel.tsx:71-101`
- `src/features/inspector/historyActions.ts:180-223`
- `src/ipc/types.ts:500-535`
- `worker/src/session/ElementIdentity.cpp:379-387`

## Work package 1.5 — Revolve body-edge axis contract

### Current mismatch

Rust models `AxisRef::Element.edge` as `ElementId`; protocol says Revolve inputs are empty and axis rides in params; worker resolves `edgeId` using `shape_for_topokey`, which accepts `e:N`, not `el_*`.

Evidence:

- `src-tauri/crates/onecad-core/src/document/refs.rs:162-195`
- `protocol/SCHEMA.md:1061-1074`
- `worker/src/ops/RevolveOp.cpp:112-132`
- `src-tauri/src/worker/wire.rs:464-546`

### Decision

Choose one:

A. Support body-edge axes properly with an **additive, versioned companion ref**: retain legacy `edgeId`, add typed `edgeRef`, include the companion in inputs, and resolve it through partition/history/ladder. Absence preserves legacy/refusal behavior and does not move existing serialized bytes or history hashes.  
B. Declare body-edge axis unsupported for new v1 authoring and refuse it consistently while preserving old records losslessly.

Recommendation: A, because body-edge axes are standard CAD behavior. The implementation must update the complete identity-slot set together: `derive_inputs`, `KnownOperation::element_refs_mut`, `wire_op_inputs`, repair `InputPath`, descriptor stamping, SCHEMA/§14, and both-track fixtures. Do not expose it until persistence, reopen and upstream-edit tests pass.

### Tests

- Promote straight edge → revolve → save/reopen → upstream edit.
- Curved edge refusal.
- Missing/deleted edge produces NeedsRepair, not OP_FAILED or ordinal fallback.
- Two-body ambiguity retains the named body.

## Diagnostics

Every ownership or stale-provenance refusal must include:

- stable code,
- expected body/snapshot,
- actual body/snapshot,
- refId and operation,
- re-pick or repair action.

## Acceptance gates

- Existing ladder calibration tests byte-equivalent.
- Existing H5/H6 and checkpoint-fallback characterization unchanged.
- New cross-body tests green in C++ and real-worker Rust.
- Frontend stale-response tests green.
- Protocol fixtures green on both tracks.
- Full suites and T0 unchanged.

## Rollback

Ownership validation is additive and can be reverted independently. Snapshot result fields must be additive and older readers must tolerate them.
