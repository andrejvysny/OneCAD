# Phase 0 Specification — Safety and Truth

Duration: 2–3 weeks  
Prerequisites: none  
Priority: P0  
Gate name: `MODEL-CORRECTNESS-P0`

## Rationale

The current product can replace a dirty document through Open, publish an empty Boolean result as a modified body, display a partial circular-pattern preview that differs from committed geometry, and report success after a backend-resolved regen failure. These invalidate the meaning of every later correctness test.

## Goals

1. Prevent unsaved-work loss on all document replacement paths.
2. Make frontend success states reflect the backend's actual regen outcome.
3. Give zero-solid Boolean results an explicit lifecycle.
4. Make circular-pattern preview and commit use one normative placement rule.
5. Reproduce the curved-wall Draft risk and require applied-or-refused behavior if confirmed.
6. Retain useful diagnostics and authored values after recoverable failure.
7. Preserve all exact preview bodies across concurrent region sessions.
8. Restore actionable Boolean controls after a failed commit or preview barrier.

## Non-goals

- General topological-naming changes.
- Pattern multi-solid redesign.
- Universal deep shape validation.
- New modeling features.
- CI platform expansion.

## Work package 0.1 — Unsaved replacement guard

### Required behavior

Every action that can replace the open document must use one guard:

- Open Project.
- New Project from the editor.
- Import Project when used as replacement rather than append.
- Start-screen transitions invoked while an editor document still exists.
- Recovery open, if it replaces a live dirty document.

The guard offers Save, Discard and Cancel. Cancel preserves:

- document id and revision,
- dirty state,
- current selection,
- active tool and preview session,
- camera and open sketch,
- pending worker job state.

### Implementation direction

Move replacement intent handling behind `appStore.requestClose` or introduce a neutral `requestReplacement(intent, continuation)` that reuses the same dialog state. Do not duplicate dirty checks in `fileActions`.

Relevant source:

- `src/features/shell/fileActions.ts:187-204`
- `src/stores/appStore.ts:189-210,296-329`
- `e2e/unsaved-guard.spec.ts`

### Tests

- Dirty Cmd/Ctrl+O opens the guard before the native file dialog.
- Cancel invokes no open dialog and preserves the editor.
- Save failure leaves the editor intact.
- Discard opens the chosen document exactly once.
- Clean Open does not prompt.

## Work package 0.2 — Authoritative save state

### Required behavior

After Save:

- backend path is authoritative,
- `dirty=false` only if no edit landed during the write,
- frontend reflects the same result without waiting for an unrelated projection event,
- success text uses authoritative display/path naming rules.

### Implementation direction

Change the save command result from `void` to a small save outcome or projection containing:

- document revision saved,
- current revision,
- clean boolean,
- adopted path,
- authoritative title.

Wire change requires linked Rust/TypeScript update and protocol changelog entry if it crosses the documented API boundary.

Relevant source:

- `src/ipc/tauriClient.ts:1668-1682`
- `src/features/shell/fileActions.ts:77-107`
- `src-tauri/src/api/mod.rs:457-498`
- `src-tauri/src/document_runtime.rs:1980+`

### Tests

- Save clears the dirty indicator when no edit races.
- An edit during save keeps dirty true.
- Save As adopts the new path/title.
- Close-after-save proceeds only when the save outcome is clean or the user explicitly discards newer edits.

## Work package 0.3 — Shared operation-result classifier

### Required behavior

Define an explicit discriminated transport result with at least:

- `published`,
- `noop`,
- `needsRepair`,
- `failed`,
- `timeout`.

Superseded/cancelled regen terminals may be covered by a later published result, but that relationship must be explicit rather than collapsed into an empty result. `NeedsRepair` remains first-class document state and must never be converted into a fatal error.

A resolved `errorMessage` is never a success. A failure must preserve or restore the user's authored parameters and expose the diagnostic.

### Implementation direction

Populate the discriminant in `tauriClient` before migrating callers; a caller-side helper cannot reconstruct timeout, no-op and state from body counts. Then introduce one shared helper consumed by:

- fresh direct operations,
- operation re-edit,
- suppress/delete/rollback,
- repair rebind,
- Pattern/Mirror/Transform,
- Hole/Shell/OffsetFace/Fillet/Chamfer re-edit.

Do not infer success solely from changed body counts without accounting for valid delete-only and metadata-only outcomes.

Relevant source:

- `src/ipc/tauriClient.ts:790-826`
- `src/tools/modelTools/ModelToolController.ts:4784-4806`
- `src/features/inspector/historyActions.ts`

### Tests

Create a table-driven test that injects each outcome into every consumer family. Assert:

- no success hint on error,
- one sticky diagnostic,
- no duplicate record on retry,
- input values remain available,
- selection is not moved to a body that failed to publish.

## Work package 0.4 — Empty Boolean lifecycle

### Defect

`checked_boolean` can return a non-null, BRep-valid empty compound. `publish_boolean_result` treats `solids.size() <= 1` as modified, including zero solids.

### Phase decision

Phase 0 uses the conservative temporary policy: **zero-solid Cut/Intersect is a recoverable refusal**. No body is deleted, the tool is not consumed, and the pre-operation snapshot remains intact. This closes the invalid empty-body publication without inventing deletion semantics.

A later decision may make complete consumption a valid deletion, but that would require an explicit SCHEMA §§7.2/7.3 and §14 update, fixture-impact assessment, and cross-track sign-off.

### Implementation direction

Make `publish_boolean_result` return a classified outcome rather than mutating unconditionally. Require callers to handle zero, one and many solids. Ensure tool deletion happens only after result classification succeeds. Document zero-result refusal normatively; it emits no body lifecycle event or mesh.

Relevant source:

- `worker/src/ops/OpCommon.cpp:156-246,283-316`
- `worker/src/ops/BooleanOp.cpp:24-62`
- `worker/src/ops/RevolveOp.cpp:214-224`
- `worker/src/ops/ExtrudeOp.cpp:478-489`

### Red-first tests

- Standalone Intersect with disjoint boxes.
- Standalone Cut with tool fully containing target.
- Revolve Intersect with no overlap.
- Tool body lifecycle under each chosen policy.
- Preview and commit produce identical lifecycle events.
- No mesh handle is emitted for a deleted body.

## Work package 0.5 — Circular-pattern parity

### Defect

Frontend partial sweep uses `angle/(count-1)`; worker and protocol use `angle/count`.

### Required behavior

One normative formula and a cross-track fixture define placement semantics. Independent TypeScript and C++ implementations must produce the same transforms in:

- frontend ghost,
- mock client,
- worker,
- Rust tests,
- protocol examples.

Recommendation: retain the currently normative worker/protocol rule in this phase to avoid a protocol behavior change. If UX research later prefers span-inclusive partial sweeps, amend all layers as a separate change.

### Tests

- `angle=180,count=3` exact transforms.
- negative angle.
- full 360° no duplicate terminal instance.
- preview versus real-worker committed centroids.
- re-edit preserves the same distribution.

## Work package 0.6 — Draft applied-or-refused

### High-confidence candidate defect

Static inspection proves the worker iterates planar faces only and existing tests cover a square profile. It does not prove how OCCT reports `Build()` when zero faces were added. The first task is a circular-profile red probe: if OCCT returns success with unchanged geometry, promote this to a confirmed P0 defect; if it refuses, preserve and name that refusal.

### Required behavior

When `abs(draftAngleDeg) > epsilon`:

- at least one eligible side face must be added,
- the builder must complete,
- output must differ semantically from the undrafted prism,
- an analytic or geometric taper invariant must hold,
- otherwise return a named refusal.

### Tests

- Square prism ±10° with closed-form frustum volume and bbox direction.
- Circular profile: either proven conical taper or explicit unsupported/refusal.
- Mixed planar/curved profile.
- Near-limit ±89° safe refusal.
- Preview/commit equality.

## Work package 0.7 — Multi-session exact preview ownership

`applyPreviewBodies` currently calls the global `clearPreviewBody()` for each session response, so the last region response erases every other region's exact candidate. Replace global clearing with per-session ownership: remove only that session's prior bodies, retain other sessions, and compute replaced-body visibility as the union of live session claims.

Tests must deliver two region responses in both orders, then fail and recover one secondary session. All surviving candidates remain visible and failure state is tracked per session/epoch rather than globally.

Relevant source: `src/tools/modelTools/ModelToolController.ts:6407-6432`.

## Work package 0.8 — Actionable Boolean failure re-arm

A failed Boolean commit returns state to `armed` and reopens the preview session, but the operation segments and Apply/Cancel controls were cleared before commit and are not rebuilt. Re-arm must republish the full chip from retained state, keep target/tool/op values, and provide both retry and cancel paths.

Tests inject exact-preview failure, rejected commit and resolved regen failure. Each path restores controls once, keeps values, avoids duplicate rows, and permits a successful second Apply.

Relevant source: `src/tools/modelTools/ModelToolController.ts:7383-7470`.

## Diagnostics

All new failures should include:

- stable code,
- operation and stage,
- concise user message,
- bounded evidence with body ids, result solid count and relevant params.

Avoid message-text routing in the frontend.

## Performance budget

- Result classification: negligible.
- Boolean solid enumeration: reuse the existing `ranked_solids` pass.
- Draft semantic check: target under 2 ms for normal prisms; record actual measurements.

## Acceptance gates

Use the established lane matrix; do not claim these passed from the analysis sandbox.

Developer Mac:

- full worker CTest against pinned OCCT 8.0.1,
- full Rust workspace with the HEAD-built worker and `ONECAD_REQUIRE_WORKER=1`,
- TypeScript build and full Vitest,
- Chromium and WebKit Playwright with retries 0,
- T0 kernelbench unchanged,
- manual Tauri smoke: open → extrude → fillet → undo → save → reopen.

Trusted Linux CI:

- worker CTest and determinism,
- T0 same-host digest and portable semantics,
- frontend/Chromium only after documented runner prerequisites and the Linux pick investigation are complete.

Record exact commit, OCCT fingerprint, commands, counts and artifacts in the gate entry.

## Rollback

Each work package should be independently revertible. Do not combine protocol save-result changes with Boolean geometry changes in one commit.
