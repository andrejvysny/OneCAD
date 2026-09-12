# OneCAD correctness and UX hardening — Claude Code continuation

Prepared: **2026-09-12**. Purpose: continue implementation in a new Claude Code session without losing the design, evidence, failures, or remaining acceptance work.

**The full plan is NOT finished. Do not interpret a commit, an older `status: done`, or a focused green suite as full acceptance.**

## 1. Start here

The user requested all findings in `OneCAD-UX-Review-2026-09-11.md` be fixed, including the larger correctness/UX hardening plan subsequently approved in chat. Coding was delegated; the main session designed, reviewed, and independently verified. The user now requests this consolidated continuation document because usage limits are low.

Your first task is to close the current measurement/annotation/pattern checkpoint, not restart the project or repeat the earlier camera and typed-target implementation. Then rebuild the native app and obtain an interim native smoke before broadening feature-pattern adapters or adding more infrastructure.

Read this document, root `AGENTS.md` and `CLAUDE.md`. Read current heads of `CURRENT_STATE.md`, `TODO.md`, `HANDOFF.md`, and `PLAN.md`, but apply the corrections below: some heads still describe work that was subsequently implemented, and some linked evidence files are absent. Read the original review and append-only issue matrix when mapping individual findings.

### Current repository identity

- Workspace: `/Users/andrejvysny/workspace/CAD/OneCAD-Tauri`.
- Branch: `master`.
- At the beginning of this handoff inspection: HEAD `80d4e74c8d32b4d847261d7d502dd397286f0788`, **312 porcelain entries**. This included pre-existing user work, not just this program.
- **During inspection, another actor changed HEAD to `ee5b449c0826538c7d1cbc83a810a8e6cebc9176`**, subject `Harden semantic publication contracts`. Git then reported a clean tree and `master...origin/master` without ahead/behind counts. That commit contains 325 changed files. The handoff-writing agent did not commit, push, pull, stage, or request this change. Authorship and how the remote-tracking reference moved were not investigated.
- This document and its small TODO pointer are subsequent documentation changes. Recheck Git before continuing; do not reset to the old HEAD or replay already committed work.
- Latest agent inventory showed only the main agent. Earlier child agents are no longer available; their names below identify responsibility/history, not resumable processes.
- No source implementation or build/test execution was performed while writing this handoff. Existing source, Git state, and logs were inspected; `git diff --check` was run.

### Completion estimate — not acceptance

The last engineering estimate was about **60–65% implemented**; an earlier weighted estimate was 61.5% and 50–55% full delivery. These are rough historical judgments, not audited coverage or a new measurement. More code landed afterward, but red gates and extensive native acceptance remain. Do not increase the percentage merely because the diff grew.

## 2. Rules that must survive the handoff

1. Preserve all user and agent changes, including unrelated files, legacy data, original report text, and prior failure records. No commits, pushes, pulls, resets, or cleanup unless explicitly requested.
2. Use the cheapest capable coding agents. Earlier allocation: Luna for simple changes, Terra for bounded UI/state/tests, Sol for concurrency, geometry, identity, transactions and cross-layer contracts; Astra main reviewed every implementation. In Claude Code, verify actual available models/tools and use equivalent tiers. Do not assume GPT tools exist or invoke a separately billed model service without authority.
3. Maximum **three simultaneous coding streams**. A shared controller, runtime, store, or protocol section has one owner at a time. Communicate exact ownership and require agents to preserve others' changes.
4. Main session reviews every diff and independently runs final gates. Agent test claims are preliminary. Keep heavy builds/tests serial: one explicit owner of the heavy lane, no competing Cargo, CMake, browser or full-unit jobs.
5. Reproduce failures before fixes. Preserve the red result and distinguish fixture defects from production defects. Do not weaken tests to make the current behavior green.
6. Use Bun. `bun.lock` is authoritative; do not change stale `package-lock.json`. Use `npx tsc --noEmit`, not a global `tsc`.
7. Build/stage the worker before compiling the app crate with Cargo. Use `ONECAD_REQUIRE_WORKER=1`; a silently skipped worker test is not a pass.
8. Native acceptance requires real Tauri IPC and OCCT. Playwright's normal lane uses a mock backend. Browser screenshots and jsdom layout assertions are not native or GPU evidence.
9. Maintain `TODO.md` iteratively. Append report/matrix evidence rather than deleting historical findings. Track implementation and native validation separately.
10. Do not re-enable Tauri's `tracing` feature, edit a Cargo registry checkout, upgrade frameworks speculatively, add retries to hide browser failures, or invent numerical tolerances/reference fallbacks.

### Architecture invariants

- React/Three frontend receives backend-authoritative projection DTOs through `CadClient`; it does not run the native kernel.
- Tauri commands use camelCase DTOs and snake_case command names. Keep client, mock, API registration, DTOs and events in lockstep.
- `DocumentRuntime` is single-writer: locked prepare/begin, unlocked worker drive, locked fenced commit. Worker epoch and expected base hash govern mutation; document revision is advisory, not an ownership token.
- `protocol/SCHEMA.md` and `protocol/mesh_format.md` are normative. Wire changes require Rust/C++ implementations plus executable cross-track fixtures.
- OCW1 frames alone go to worker stdout. Logging goes to stderr.
- Z-up, right-handed coordinates throughout. Never rotate the scene or swap axes to compensate for a camera issue.
- Rendering is on demand. Avoid idle loops, per-frame React updates, focus-destroying remounts, and redundant preview/read requests.
- Typed references fail closed. Ambiguity means `NeedsRepair`, not nearest geometry or silent reference rewriting.

## 3. Locked product decisions

- Compact movable tool chip plus a stable parameter inspector. Chip order: primary value/unit, operation badge, Confirm, Cancel. Secondary settings, standards, detailed validation and references belong in the inspector.
- Invalid typed dimensions remain visible and prevent confirmation. Separate draft text, parsed intent, preview, and committed values. Preserve fast typing, paste, first digits, Tab and focus.
- `Use maximum` applies a verified current limit without committing. Unknown range analysis does not manufacture a limit.
- Sketch dragging snaps the requested target before solving; constraints remain authoritative. Alt bypasses snapping, not constraints. Explain blocked or unsatisfied snaps.
- Operation plus automatic consumed-sketch visibility is one backend transaction and one Undo/Redo step. Explicit visibility edits stay separate.
- Keep last valid model inspectable while operations are pending or failed. Distinguish previewing, applying, completed, failed and genuine cancellation; never claim cancellation after publication.
- General feature patterns persist selected source record IDs, layout, count, and semantics version; they do not persist copied source payloads. Sources are in canonical timeline order. Count includes the source. Shared parameters only; no per-instance overrides or suppression.
- General patterns re-evaluate mixed chains within one host/body lineage. Any invalid instance rejects the complete pattern atomically. Existing Body Pattern V2 records and behavior remain intact.
- Isolated QA projects may be saved for persistence validation. Never overwrite existing user files. Unsupported physical input coverage remains explicitly blocked.
- No unresolved product decision. Remaining unknowns are engineering and evidence gaps.

## 4. What was implemented, and what that means

“Present” means code exists. “Focused verified” means the main session ran named bounded checks on a prior checkpoint. Neither means current integrated or native acceptance.

| Package | Present work | Remaining boundary |
|---|---|---|
| 0 — inventory/evidence | Original review, issue matrix, three preserved stall logs and process samples | Reconcile every original/addendum finding against current source and acceptance; preserve untracked/previous work history |
| 1A — responsiveness/publication | Narrow Tauri feature correction; unlocked runtime work; runtime/manager/epoch fences; event/mesh coalescing; render-completion correlation; readiness scheduling guard | Independent main-thread responsiveness probe and automatic bounded timeout capture; native reproducer/stress proof |
| 1B — confirmation | Interactive UI boundary, single-flight confirmation, region snapshot and focused regressions | Native pointer/accessibility/Enter equivalence, annulus geometry, rapid/repeated activation |
| 1C — solver/references | Solver currentness metadata and unevaluated state; persistent-reference guards; resolver v6 ambiguity correction | Full native restored-sketch truth, recovery and congruent-twin matrix; final integrated identity baselines |
| 1D — selection | Authoritative removed-selection cleanup, hover cleanup, inspector guards | Native removal/undo/recovery, no stale actionable inspector, hover recomputation |
| 2A — transactions | Operation/consumed-sketch visibility atomicity; runtime/revision/exact-undo-entry rollback receipts and capped history handling | Native Undo/Redo, failed rollback, persistence; separate sketch-visit lifecycle races remain |
| 2B — tool semantics | Palette exact-match ranking, no-op dirty-state checks, field-local undo boundaries, numeric drafts/limits, Hole defaults and semantic messages | All report variants, explicit Add/Cut/New target/direction/non-intersection messages, delayed/invalid-mode/tool-first native matrix |
| 2C — shared contract | Discriminated active-tool presentation, typed targets/references, shared chip/inspector authority | Cross-family semantic audit and native parameter consistency |
| 2D — layout | Movable/dockable chips, safe work area, stable input focus, 320px inspector with 280–420 bounds and drawer behavior, history layout | Native sizes/scaling, actual target/manipulator obstruction and keyboard reachability |
| 2E — sketch behavior | Solver-backed drag/snap/rollback work; guarded sketch entry and cleanup | Physical constraints/grid/Alt/trim/Cancel tests; backend sketch-visit ownership for normal asynchronous exits |
| 2F — guidance | Construction reset/stable indicator, sketch wording/constraint labels and feedback | Every slot/trim/construction/constraint variant in native GUI |
| 3A — picking | Ordered overlap candidates, filters/cycling/preview, current installed-mesh proof and reference-safe promotion | Native overlap/twin/removed-topology cases and all selection kinds |
| 3B — inspection | Tree hover/Reveal, panel-aware camera fitting, explicit Fit Preview, section controls, measurement readouts and new annotation controls | Current measurement/annotation gate; planar-face/datum framing outside sketch entry; native inspection matrix |
| 3C — general patterns | Separate versioned records, dependencies/planning, bounded nested execution, factual topology provenance and several adapters | Current mixed-chain RED, remaining operation adapters, generality, source edits/suppression/repair/persistence/identity proof |
| 3D — pattern UI | Bounded existing presentation/tests | Broad feature-pattern UI stays hidden until backend contract and execution support are proven |
| 4 — documentation | Append-only review and checkpoints | Reconcile all findings and final evidence after implementation; no “all fixed” claim now |

### 4.1 Native stalls and readiness scheduling

Primary files: `src-tauri/Cargo.toml`, `src-tauri/src/api/mod.rs`, `document_runtime.rs`, `events.rs`, `state.rs`, `worker/manager.rs`; `src/ipc/tauriClient.ts`, `src/viewport/mesh/meshSync.ts`; `scripts/tests/tauri-event-dispatch-feature.test.sh`.

The three original F/H/O samples show a shared webview-registry mutex wait. Later investigation tied a two-thread stall to Tauri's tracing-enabled synchronous background JavaScript dispatch while holding the webview registry lock. `Cargo.toml` now keeps `tauri = { version = "2", features = [] }`; application tracing remains separate. Native stress has not established final closure. Preserve locked dependencies and test the narrow correction rather than editing installed framework code.

Readiness fix: `enqueue_initial_regen_if_current` checks the exact runtime session and enqueues a tagged request while holding the same runtime lock. Merely checking a document UUID is insufficient: reopening the same file preserves UUID. Merely tagging a request for driver-time rejection is insufficient: a stale enqueue can cancel valid current scheduler work before the driver rejects it.

Tests must call the real scheduling helper with deterministic barriers. Earlier no-op callback tests were rejected. The corrected same-file regression saves and reopens the same file, asserts equal document UUID and unequal runtime sessions. Initial new/open/import/restart replay uses `RevertToEnd { from: 0 }`; actual edits keep `ToEnd` and their real dirty floor.

### 4.2 Reference ambiguity and resolver v6

Primary files: `worker/src/elementmap/{Ladder.cpp,Ladder.h,Scoring.h,ElementMapPartition.cpp}`, relevant schema/fixtures, `src-tauri/tests/topology_rebind.rs`.

An older passing test deliberately accepted `boundDecoy=true` for a stale-anchor teleport. It was characterization of unsafe behavior, not acceptance. The v6 correction removes the post-upstream-edit exact-anchor carveout that could select a congruent twin. The corrected regression requires `NeedsRepair`, zero removed volume, and `boundDecoy=false`.

Do not weaken the clean-replay rules to accommodate wrongly classified edit requests. Existing clean-replay guards use the established 0.01mm bound, rival separation and descriptor margin; do not introduce fresh epsilons. Resolver scoring version and `corePolicyVersions.resolver` are different version domains; do not mechanically change both.

### 4.3 Typed presentation, camera and Fit Preview

Primary files:

- `src/tools/modelTools/activeToolContext.ts`, `activeToolPresentation.ts`, `ModelToolController.ts`.
- `src/features/inspector/{ActiveToolInspector.tsx,ActiveToolTargets.tsx,FitPreviewButton.tsx}`.
- `src/stores/{toolChipStore.ts,toolChipPlacementStore.ts,inspectorLayoutStore.ts,viewportWorkAreaStore.ts}`.
- `src/viewport/engine/{cameraFit.ts,CameraRig.ts,CadOrbitControls.ts,ViewportEngine.ts,PreviewMesh.ts,faceFrameBounds.ts,sketchFrameBounds.ts}`.
- `src/tools/sketch/SketchController.ts` and its entry/exit/face/plane-pick tests.

Typed presentation uses authored target IDs/roles and live names. Missing required references block confirmation; Chamfer A/B must be honest, distinguishable references. No unsafe context casts or second parameter authority.

Camera fitting uses all eight bounding-box corners in the destination camera basis, handles perspective/orthographic projection and panel-safe rectangles, preserves existing margin and distance limits, and refuses invalid bounds. Fit model includes visible geometry only. A missing explicit selection must not silently fit the whole model. Fit Preview uses preview geometry, not manipulator handles; no automatic camera action or model fallback.

Sketch entry captures exact indexed face geometry before awaits, rejects replaced mesh entries/body transforms, uses finite committed non-construction plus draft sketch bounds, and restores prior camera context once on exit. Non-pole yaw derives from the face normal; plane X axis determines pole orientation. Entry tickets span promotion and document/runtime/mode/disposal changes. Same-document stale cleanup waits for Cancel; refused discard retains changes, warns, and does not delete or automatically re-enter.

Fit Preview has accessible disabled reasons, clears stale refusal feedback, and is omitted for terminal/unsupported states. These changes have focused automated evidence, not native acceptance.

### 4.4 Scoped measurement reads — latest code, not independently accepted

Primary files:

- `src/ipc/{client.ts,types.ts,tauriClient.ts,mockClient.ts,promote.ts}`.
- `src/stores/measureStore.ts`.
- `src/features/measure/MeasurePanel.tsx`.
- Measure paths in `src/tools/modelTools/ModelToolController.ts`.
- `src-tauri/src/{api/mod.rs,dto.rs,document_runtime.rs,state.rs}`.

Approved additive contract:

```ts
interface GeometryReadFence {
  documentId: string;
  runtimeSession: string;
  snapshotId: number;
}
// Optional for compatibility; all Measure callers must supply it.
elementInfo(bodyId, elementId, topoKey?, fence?)
massProperties(bodyId, fence?)
classifyElement(bodyId, elementId, topoKey?, fence?)
```

Rules implemented in the latest source:

- Capture the fence before the first await from `getCurrentMeshPublication()`, matched to the current projection. Snapshot must be a positive safe integer; never manufacture zero. `documentStore` does not itself carry the authoritative published snapshot.
- Under the runtime lock, prepare a `GeometryReadTicket` containing runtime instance, head snapshot, head epoch and a cloned runtime-owned query provider. Check live fencing epoch as well; worker restart can advance it before another head is published.
- Release the runtime lock for the worker read. Revalidate exact runtime/head/epochs afterward. Stale results are rejected.
- `AppState.make_backend_with_query()` returns engine, mesh, solver and query from the **same factory tuple**. Bind the returned provider once to the new runtime. Do not construct a runtime and later read the globally replaceable query slot; overlapping replacement can select another document's worker.
- Preserve backend staging/rollback behavior. Older `make_backend()` callers can use its wrapper; scoped runtime construction must bind the matching query.
- Frontend cache/render guards include document, runtime and snapshot, not just body ID. Delayed results are checked after each await.
- Measurement picking additionally captures the installed `MeshEntry` proof, validates live geometry, body visibility/isolation, mesh identity, snapshot/generation and ordinal, passes proof/snapshot to promotion, and rechecks afterward. A pre-existing `elementId` does not bypass stale installed geometry.
- Controller clears picks on a new publication even without a projection write, and on document/runtime replacement at the same advisory revision.
- Panel subscribes to publication notifications and hides mismatched mass results immediately.
- Legacy history/placement/rebind read callers remain explicitly unscoped; do not claim they gained this protection.

Latest handoff inspection confirmed a new `ModelToolController.measureCurrentness.test.ts` with **three test definitions**: publication-only clearing, same-revision runtime replacement, and delayed element read after installed mesh replacement. These were not run by main. The delayed promotion and delayed classification controller cases requested during review are not present in this file; add/verify them. Do not treat mock-client latency tests alone as proof of controller behavior. Review incomplete test doubles and ensure guards are genuinely reached.

Other test pointers:

- `src/features/measure/MeasurePanel.test.tsx` — delayed mass after same-body snapshot advance.
- `src/ipc/mockClient.publication.test.ts` — scoped stale head and delayed element/classification refusal.
- `src/ipc/tauriClient.test.ts` — scoped geometry-read argument marshalling.
- `src-tauri/src/document_runtime/tests.rs` — `geometry_read_ticket_requires_and_revalidates_exact_head`.
- `src-tauri/src/state.rs` — `runtime_query_from_factory_is_not_replaced_with_global_slot`.

The first main Cargo compile failed with five structural errors: `read_query` had landed in the wrong struct and in a function argument list instead of the runtime initializer. Corrections exist, but **no subsequent successful independent compile is claimed**. Run the compiler first.

### 4.5 Annotation layout and actual measurement inspector

Primary files: `src/features/measure/{MeasureOverlay.tsx,MeasurePanel.tsx,AnnotationControls.tsx}`, `src/stores/measurementAnnotationStore.ts`, `src/viewport/engine/{annotationPlacement.ts,HtmlOverlayDriver.ts,ViewportEngine.ts}`, `src/modules/modeling/{inspectorSectionIds.ts,inspectorSections.ts,ui.ts}`.

New transient annotation state has separate A/B/pair identity, manual hide, screen-space pin, availability and placement status. Pair identity includes both picks. Pins stay fixed in screen coordinates while readings update. Identity guards reject old callbacks; clearing or overlay remount resets visit-local pins. Do not persist stale geometry references through annotation state.

Placement is explicit opt-in, after normal tool controls settle. Priority is pair, then B, then A, with deterministic tie ordering. Protect fixed chips/manipulators; use measured sizes and existing 6px spacing. Candidate centers derive from desired position, safe edges, and occupied edges; choose the nearest valid placement. Unknown size stays hidden, not guessed. Pinned annotations never move to make space; hide them when blocked and restore when possible.

Auto-hidden annotation hosts use `visibility: hidden` while staying measurable, not `display: none`; ResizeObserver can discover sizes and invalidate the on-demand renderer. Avoid per-frame React state or layout reads. Manual hide remains independent of automatic no-space hiding.

Important correction: the old “docked” MeasurePanel was actually an absolute 228px viewport overlay. Latest source moves it to a **Measurement inspector contribution at priority 90**, gated by model scope and active Measure tool. Only the geometry labels stay in ViewportOverlay. Existing overlay priorities remain explicit 100–160. The root card is full-width/min-width-zero rather than absolute. Annotation controls wrap within the inspector and expose independent Hide/Pin actions and status.

Latest review fixed JSX accidentally inserted into `inspectorSections.ts`: it now uses `createElement(MeasurePanel)` without renaming the file. Annotation nullability and ResizeObserver test typing were also corrected after a TypeScript failure. These fixes and the new `src/modules/modeling/measurementInspectorSection.test.tsx` require a fresh independent gate. Its 280px test is a jsdom contract assertion, not physical window evidence.

## 5. Feature patterns — do not mistake the subset for general support

### 5.1 Main source map

- Domain records/dependencies: `src-tauri/crates/onecad-core/src/document/record.rs`, `history/graph.rs`, `history/timeline.rs`.
- Planning: `src-tauri/crates/onecad-core/src/regen/feature_pattern.rs`, `feature_pattern_tests.rs`.
- Worker: `worker/src/session/FeaturePattern.cpp`, `FeaturePatternIdentity.cpp`, `FeaturePatternValidation.cpp` and headers.
- Provenance: `worker/src/session/TopologyOrigins.h`, `ScratchJob.h`, `Session.*`, `PlanExecutor.cpp`, `PreviewOp.cpp`, `worker/src/ops/TopologyHistory.*`, `OpTypes.h`.
- Existing history adapters: `ExtrudeOp.cpp`, `HoleOp.cpp`, `FilletChamferOp.cpp`, `RevolveOp.cpp`.
- Tests: `worker/tests/test_feature_pattern.cpp`, `test_topology_origins.cpp`, `src-tauri/tests/feature_pattern_integration.rs` and its `shared_host`, `host_chain`, `revolve`, `persistence`, `independent` modules.
- Contract: `protocol/SCHEMA.md`, `protocol/fixtures/feature_pattern_{malformed,repair}.ndjson`, modeling coverage/contracts manifests.

### 5.2 Required invariant and present ledger

Topology origin is factual and operation-independent: `Known(producerRecordId)`, `Ambiguous`, or `Unknown`. It is not inferred later relative to whichever pattern is executing.

OCCT Modified/surviving topology inherits prior ownership. Adapter-certified Generated topology or owned temporary-tool output may be born at the current operation. Conflicting claims become Ambiguous; missing claims stay Unknown. A before/after shape difference alone cannot establish birth: a Boolean can rebuild a host boundary without creating source-owned topology.

Resolved source-input evidence includes the effective source hash and pre-source input ownership. Propagate ledgers through scratch execution, checkpoints, accepted sessions, rollback and preview. Nested FeaturePattern adoption preserves the nested ledger instead of overwriting it with a single outer-pattern owner.

Refresh output references for **all previous selected producers after every step**, not just the immediately preceding feature. This is required for i−2 references after an intervening modifier. A virtual reference must resolve to topology owned by the expected selected producer.

External retained support is allowed only through explicit slots (currently Hole support face or Chamfer reference faces), with known unselected ownership, the designated host and unchanged original identity. Missing selected-producer output must never fall through to “host support.” Unknown/ambiguous ownership means repair.

Some history membership indexing was improved. Global origin lookup/insert still has potential quadratic behavior. Measure representative costs before optimizing; exact shape identity and ambiguity behavior must survive any index.

### 5.3 Current intended subset, not current green coverage

The current code attempts these bounded chains, with additional per-variant restrictions:

- Hole followed by Chamfer on the same host.
- Selected Sketch, one-direction Blind NewBody Extrude, then Hole/straight-edge Fillet/Chamfer.
- One or two directly consumed Sketches, NewBody Revolve with sketch-line axis, then bounded edge modifiers.
- Selected Sketch, Blind Add/Cut Extrude on an explicit host, then bounded Hole/Fillet/Chamfer.
- References back to an earlier producer across an intervening modifier; retained host support slots.

The mixed Add/Fillet/i−2 Chamfer and late-failure cases are currently red in the newest on-disk worker log. Do not advertise those as working. Some circular-edge cases have special producer-backed handling; inspect actual validation rather than generalizing “all Fillet/Chamfer.”

The approved direction is a typed step/slot fold, not an ever-growing whitelist:

```text
StepRole: SketchMaterializer | LocalCreator | HostModifier
RefSlot: input_index, kind, role
Step: source_index, source_record_id, op_type, role,
      consumed_sketches, bodies, ref_slots
AdapterPlan: execution_class, creator_index?, designated_host, steps
```

Keep explicit selected sources in canonical timeline order. Record dependencies must invalidate pattern/downstream operations for source edits, suppression or deletion. Reuse normal operation validation/execution with transformed reference/coordinate adapters. Namespace provenance by pattern, instance and source. Reject cycles, multiple hosts, unselected consumed operands and nonunique external references with precise explanations.

### 5.4 Canonical fixture correction and newest RED

The last main selected CTest run was **4/5**, failing `canonical_feature_pattern_repair`. The old fixture claimed source Hole/Chamfer records that it had never actually executed. With strict source evidence, failure occurred at the root Hole rather than the intended later Chamfer.

Latest source corrects the fixture: execute and accept the actual Hole with the matching effective payload/hash, then pattern that Hole plus an intentionally missing Chamfer. A separate case exercises missing source-zero evidence, and another exercises nested invalid geometry. Root source evidence is now checked too. Do not simply replace expected diagnostics with any earlier failure.

Nested repair items now receive instance/source/input context. Exact input parsing accepts only the complete `<nestedOpId>.inputN` format with an in-range integer. If the slot cannot be identified, return contextual `OP_FAILED` with original diagnostics; do not invent index zero.

**Additional evidence discovered while writing this handoff:** `worker/build/Testing/Temporary/LastTest.log`, timestamp **2026-09-12 13:46:57 CEST**, records a later four-target run:

| Target | Existing log result |
|---|---|
| `topology_origins` | Passed |
| `feature_pattern` | Failed |
| `canonical_feature_pattern_malformed` | Passed, 7 expectations |
| `canonical_feature_pattern_repair` | Passed, 19 expectations |

Failure text, preserved here because CTest overwrites that log:

```text
FAIL: shared-host Add edges compose through Fillet then i-2 Chamfer
FAIL: shared-host Add modifier chain rolls back after a later instance fails
```

This is **3/4 from an inspected existing log**, not a fresh independent main run. It supersedes the assumption that only the fixture remains red. Rebuild and reproduce both failures before changes. Inspect source materialization, effective hashes, origin binding and nested diagnostics to distinguish valid safety refusal from an adapter defect. Do not loosen ownership checks or volume expectations merely to pass.

The current test file is over 1,100 lines. Split coherent fixtures/tests when useful, but do not let cleanup obscure correctness work. At least one late-failure assertion still compares geometry signature and partition size; expand it to exact partition bindings, shape ownership/claims and resolved-evidence hashes. Counts alone do not prove atomicity.

### 5.5 Remaining adapter breadth

Still requires deliberate implementation and per-operation proof:

- Shell and OffsetFace ownership/history and reference-slot mapping.
- TransformBody composition/conjugation (`T * S * T^-1`) and ownership propagation.
- Shared-host Revolve Add/Cut and remaining supported variants.
- Boolean chains with explicitly selected/owned consumed tools.
- Mirror and existing Body Pattern operations where the final chain respects one-host lineage.
- Gear and other currently implemented independent creators.
- Assess nested FeaturePattern and resource Import/Place/Detach against explicit contracts; do not silently declare whole product families out of scope. Unsupported adapters must say so until implemented.
- No claim of Loft/Sweep or opaque-extension support.

For every adapter, test source edits, whole-pattern suppression, repair, Undo/Redo, Save/Open, deterministic identity, failed-instance atomicity and unchanged Body Pattern V2 behavior. Older seven-test integration coverage does not prove these properties for each newly added chain.

## 6. Other unfinished engineering packages

### Sketch visit ownership receipts

`SketchController` guarded entry is not enough. Normal asynchronous keep-exit, switch and discard can await Cancel and later call Finish against a replaced runtime or a newer visit to the same sketch. Stale hints can also land in the new visit. Backend Finish paths can mutate without a sufficiently specific session receipt.

Design a runtime- and visit-bound receipt for begin/finish/cancel/rollback, then implement it across frontend/backend as one Sol-equivalent package. Preserve existing Finish/Escape semantics and refused-discard behavior. Test same-file reopen, same-sketch second visit, delayed cancel/finish, disposal and stale feedback. Existing undo rollback receipts do not by themselves close this lifecycle race.

### Independent responsiveness monitoring

Render watchdogs correlate commit/publication/mesh install/after-render. They do not independently prove that the native main thread is responsive.

Proposed direction, not approved implemented behavior: debug-only main-thread acknowledgement using a narrow dispatch that does not acquire the webview registry mutex; at most one outstanding probe; one bounded evidence capture per stall; app-owned evidence path; safely quoted/no-shell command arguments; no release monitor. Review the contract before coding. Capture timeout evidence without causing another lock inversion or unbounded logging. Native reproducer and stress remain mandatory.

### Framing, inspection and UI review

Explicit planar-face/datum normal framing outside sketch entry remains open. Reuse section state/rendering; current controls expose principal planes, numeric offset and flip. Navigation and section changes must not alter model history.

Re-test tree/canvas hover distinction, Reveal, fixed measurements, annotation pins, inspector collapse/resize, chip relocation/Dock/Return, offscreen fallback and selected-target obstruction with actual native windows. Preserve complete history, distinguish long names, and keep inline editors below rows rather than widening them.

## 7. Evidence ledger and known limitations

### 7.1 Historical broad gates — September 11 checkpoint only

These were recorded before the subsequent camera, provenance, measurement and annotation work. They are not current integrated evidence.

- Vitest: **332 files; 5,880 passed, 78 skipped, 5,958 total**.
- Worker-required Rust workspace rerun: **1,646 passed, 0 failed, 0 ignored, 0 filtered**, across **100 result lines**.
- CTest: **196/196**. Current CTest registration has since grown; do not hard-code 196 as the expected current count.
- Clippy after DTO correction: warning-free `-D warnings`, 44.48 seconds; fmt recorded passed.
- Bun build/TypeScript: passed, existing >500kB chunk warning.
- Coverage/contracts: recorded 34 coverage rows, 9 corpus entries, 16 CI jobs, 20 registry entries; 41 contract rows, 19 operations, 15 tier checks.
- Browser: bounded chooser Chromium/WebKit **2/2**, retries zero. Not a full browser run for the latest implementation.

Evidence location correction: older documents link `docs/qa/evidence/ux-hardening-2026-09-11/full-*.log` and `final-*.log` as durable files. Those gate logs were **not present at those linked paths during handoff inspection**. The directory does contain the three native incident logs/samples, README and issue matrix. Verified temporary sources exist:

```text
/tmp/onecad-ux-full-vitest-final.log
/tmp/onecad-ux-full-ctest.log
/tmp/onecad-ux-final-clippy.log
/tmp/onecad-ux-final-clippy-after-dto.log
/tmp/onecad-ux-final-build.log
/tmp/onecad-ux-final-cargo-workspace.log
/tmp/onecad-ux-final-cargo-workspace-rerun.log
```

The file without `-rerun` is the **RED Rust run: 475 passed, 1 failed**. The `-rerun` file is the 1,646-pass run; its totals were re-counted during handoff inspection, not re-executed. Preserve these logs into an explicitly named evidence directory before temporary files disappear, and fix links additively. Do not confuse a surviving file name with a successful result.

### 7.2 Subsequent focused main checks

- Typed inspector/chips/tree/Reveal/camera/navigation: **11 files / 203 tests**, TypeScript passed at that checkpoint.
- Resolver v6 + corrected clean replay + Revolve origin hook: worker-required FeaturePattern **7/7**, topology recovery **16/16**; later selected CTest **5/5** at that checkpoint.
- Readiness helper: **7/7**; subsequently corrected real same-file reopen case **1/1**.
- Camera/sketch entry: **13 files / 184 tests**. A test mock return-type error was corrected; main then passed TypeScript and plane-pick **9/9**.
- Fit Preview/targets/presentation: **3 files / 25 tests**, TypeScript passed.
- Annotation/engine strengthened suite: **6 files / 98 tests**. jsdom canvas warnings do not imply native rendering evidence.
- Latest main worker Release build/stage: passed with stdout hygiene; C++ deprecation/initializer warnings existed. No warning-free C++ claim.

### 7.3 Latest unresolved gates

- Main selected pattern CTest **4/5 RED**, then the later inspected existing log **3/4 RED** described above. No fully green current pattern gate.
- Main TypeScript failed on three annotation contract/ResizeObserver fixture errors. Corrections exist; then the inspector `.ts` JSX error was found and corrected. No successful main TypeScript rerun after all final edits.
- Main measurement Cargo compile failed with five structural errors. Corrections exist; no successful main compile after final measurement changes.
- New measurement-currentness and measurement-inspector tests have no independent results yet.
- No full unchanged-tree unit/build/browser/Cargo/CTest/verifier suite after the final changes.
- No native acceptance of the latest implementation. No claim of completed housing/flange, 100-commit stress, persistence or window-size matrix.

## 8. Exact continuation order

### Step 1 — reconcile and preserve

1. Check HEAD, status, active agents/processes and ports. The externally created `ee5b449` commit is the new baseline unless it changed again. Do not infer the old dirty state still exists.
2. Preserve existing failure/evidence logs before rerunning commands that overwrite them.
3. Read the latest source of the three unfinished checkpoint areas, not just their prior agent summaries.
4. Add a dated TODO checkpoint with assigned owners and test status. Do not rewrite historical results.

### Step 2 — close compilation and focused regressions

Give a bounded agent the measurement/annotation compile and test repair. Reserve controller/runtime/API ownership for the complex owner; a UI owner may change only annotation/inspector rendering files. Run `npx tsc --noEmit` first. Correct actual contracts, not type assertions that hide errors.

Run the focused frontend tests below, then the scoped Rust tests. Add missing controller delay cases and exact provider/currentness coverage. Review the inspector registration and focus behavior before accepting.

In a disjoint worker/core stream, reproduce both newest feature-pattern failures, fix their cause, and rerun worker/canonical/Rust pattern gates. No further adapter breadth until this checkpoint is green. Main reruns final checks serially after sources are frozen.

### Step 3 — fresh native interim check

Build a new bundled debug `.app`; the existing app is stale. Test native project creation, sketch/extrude, measurement inspector, camera transitions/Fit Preview, and a representative modifier operation. Confirm actual commit/render completion and inspect logs. Feed concrete native failures back to their owners.

### Step 4 — remaining implementation

Prioritize sketch-visit receipts and independent responsiveness monitoring, then general-pattern adapters in contract/core/worker slices, with UI only after support stabilizes. A separate bounded UI stream can address planar-face/datum framing and native-discovered layout defects without sharing controller ownership.

Do not continually add infrastructure while deferring native evidence. Alternate a reviewed bounded implementation checkpoint with a meaningful native workflow.

### Step 5 — full acceptance and documentation

Freeze the integrated tree. Run all gates serially, then complete the native matrix below. Append each finding's implementation and acceptance evidence. Report remaining blocked coverage accurately. Only declare the plan complete when no required issue or gate remains open.

## 9. Verification commands

Commands below are instructions for the next session, not gates executed during this handoff. Run from the stated directory. Capture complete stdout/stderr, exit code, source identity, worker provenance and test counts. Use shell `pipefail` when piping to `tee`; never let logging conceal a failed command. Preserve red logs under distinct names.

### Frontend checkpoint — repository root

```bash
npx tsc --noEmit
bun run test src/features/measure src/stores/measurementAnnotationStore.test.ts src/modules/modeling/measurementInspectorSection.test.tsx src/tools/modelTools/ModelToolController.measureCurrentness.test.ts src/ipc/mockClient.publication.test.ts src/ipc/tauriClient.test.ts src/viewport/engine/annotationPlacement.test.ts src/viewport/engine/HtmlOverlayDriver.test.ts src/viewport/engine/ViewportEngine.test.ts
```

Check the runner's selected files and counts. `MeasureOverlay.test.ts` is **not** `.test.tsx`; an earlier wrong filter omitted it. Do not accept zero tests or a subset selected accidentally.

### Worker build and focused pattern checks — repository root

```bash
scripts/build-worker.sh Release
ctest --test-dir worker/build -R '^(wp6_ladder|topology_origins|feature_pattern|canonical_feature_pattern_malformed|canonical_feature_pattern_repair)$' --output-on-failure
```

The staging script builds the pinned OCCT worker and stages the sidecar/manifest. Do not use a stale staged worker with fresh Rust. If CMake configuration fails, follow root AGENTS and `scripts/build-pinned-occt.sh`; do not switch OCCT versions casually.

### Scoped Rust checks — from `src-tauri`

```bash
ONECAD_WORKER_PATH="$PWD/../worker/build/onecad-worker" ONECAD_REQUIRE_WORKER=1 cargo test -p onecad --lib geometry_read_ticket_requires_and_revalidates_exact_head -- --nocapture
ONECAD_WORKER_PATH="$PWD/../worker/build/onecad-worker" ONECAD_REQUIRE_WORKER=1 cargo test -p onecad --lib runtime_query_from_factory_is_not_replaced_with_global_slot -- --nocapture
ONECAD_WORKER_PATH="$PWD/../worker/build/onecad-worker" ONECAD_REQUIRE_WORKER=1 cargo test -p onecad --lib readiness_gate -- --nocapture
ONECAD_WORKER_PATH="$PWD/../worker/build/onecad-worker" ONECAD_REQUIRE_WORKER=1 cargo test --test feature_pattern_integration -- --nocapture
ONECAD_WORKER_PATH="$PWD/../worker/build/onecad-worker" ONECAD_REQUIRE_WORKER=1 cargo test --test topology_rebind -- --nocapture
```

Confirm filter names still exist and each runs nonzero tests. These do not replace API delayed-read tests, canonical cross-track replay or the full workspace gate.

### Full integrated gates — serial, unchanged source

From repository root:

```bash
npx tsc --noEmit
bun run build
bun run test
bun run e2e --project=chromium --retries=0
bun run e2e --project=webkit --retries=0
scripts/build-worker.sh Release
ctest --test-dir worker/build --output-on-failure
node scripts/verify-modeling-coverage.mjs
node scripts/verify-modeling-contracts.mjs
scripts/tests/verify-modeling-coverage.test.sh
scripts/check-worker-stdout-hygiene.sh
scripts/tests/tauri-event-dispatch-feature.test.sh
```

Then from `src-tauri`:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ONECAD_WORKER_PATH="$PWD/../worker/build/onecad-worker" ONECAD_REQUIRE_WORKER=1 cargo test --workspace
```

Also run affected protocol fixtures, deterministic geometry/identity baselines and existing Body Pattern V2 tests. Review skip counts individually. Existing C++ warnings and Vite chunk warnings are distinct from the required warning-free Rust clippy gate.

Playwright uses port 4177; Vite/Tauri dev uses strict port 1420. A stale running server can serve old modules and create misleading results. Identify ownership before stopping a server; restart deliberately and confirm the current app is being served. Do not enable retries.

## 10. Native Computer Use verification

Read `/Users/andrejvysny/.codex/skills/onecad-computer-use/SKILL.md` before native work. Claude Code must verify it has actual native Computer Use access; a skill file does not grant a tool. If only browser control exists, record native coverage blocked and request the appropriate tool/human pass rather than claiming equivalence.

Build from repository root:

```bash
scripts/build-worker.sh Release
bun run tauri build --debug --bundles app
```

**Do not add the extra `--` shown in the older skill example.** Root AGENTS correctly states Tauri build flags must not be forwarded to Cargo.

Native bundle:

```text
/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src-tauri/target/debug/bundle/macos/onecad.app
```

The existing executable's inspected modification time was **2026-09-11 18:43:03 local time**. It does not contain the latest source. Previous native discovery found no running OneCAD; current process state was not rechecked during this handoff. `tauri dev` may show an unbundled window that native discovery cannot attach to; prefer the bundled debug app. It need not be copied over an existing installed application for this test.

When native tools are available: attach by the exact app path, read the returned API documentation, refresh accessibility/UI state before each action, use accessible controls first, and inspect screenshots before coordinate clicks. Never invent unsupported tool calls. Log app/build identity and use `ONECAD_LOG_DIR` or `logs/dev.jsonl` according to `docs/DEBUGGING.md`.

### Required acceptance matrix

1. **Annulus confirmation:** pointer, accessibility activation and Enter commit identical selected regions, exactly one operation, and a genuinely hollow body. Repeated activation while pending never duplicates it.
2. **Responsiveness stress:** at least 100 commits per affected Fillet, Hole and Offset family across three fresh launches. Record commit, publication, mesh request/install and rendered completion; remain interactive. Count actual successful commits, not attempts or previews.
3. **Tool failures:** invalid modes/values/targets, delayed responses, fast typing/paste/Tab, repeated confirmation, cancellation before/after publication, tool-first recovery. Invalid values stay visible; controls and Cancel remain reachable.
4. **Sketch truth:** dimensioned drag; constrained/locked geometry; isolated grid snapping/rounding; Alt bypass; trim; new- and existing-sketch Cancel boundaries; refused rollback; Finish/Escape conventions; solver currentness after restore.
5. **Selection/recovery:** deleted objects, topology replacement, recovery and Undo/Redo leave no stale actions, nameless inspector, stale hover or fabricated DOF. Congruent twins fail closed.
6. **Complex parts:** a fully dimensioned housing and a revolved flange exercising holes, cuts, shell, edge treatments, patterns and downstream edits. Verify dimensions/topology and resulting history, not only visual resemblance.
7. **General patterns:** mixed source chains, source edits/deletion/suppression, internal/external reference remapping, twin ambiguity, late-instance atomic failure, deterministic instance identities and unchanged Body Pattern V2.
8. **Persistence:** isolated native Save/Open and autosave recovery. Compare geometry, parameters, dependency graph, solver state and dirty state. No overwrite of user documents. Record any crash/recovery action and its isolated target.
9. **UI:** 1024×768, 1156×768 and 1440×900, normal/Retina scaling and enlarged UI. Test chip relocation, Dock/Return, inspector 280–420px/collapse, focus order, hover, measurements, annotation pins and long history names. No inaccessible confirmation, horizontal overflow or obstructed modeling targets.
10. **Complete tool inventory:** every existing tool variant named in the original report receives an explicit pass, defect or blocked result, including navigation/input-device guidance and all applicable operation modes.

Physical hover, dragging, modifier navigation and native key-repeat require supported input tooling or a human-assisted pass. Do not simulate them through direct application state and call that physical coverage. If local repo guidance requires a human-run persistence/autosave checklist, retain that acceptance boundary.

## 11. Documentation and evidence discipline

The authoritative finding list is the original review plus all addenda, not this summary alone:

- `OneCAD-UX-Review-2026-09-11.md`.
- `docs/qa/evidence/ux-hardening-2026-09-11/issue-matrix.md`.
- `PLAN.md` — larger current plan at the top; smaller WP-U1–U12 `status: done` below is historical.
- `TODO.md` — append-only/reverse-chronological program and gate ledger.

For each finding, record: identifier, reproduction, owning layer, owner, implementation files, automated command/result, native evidence, remaining limitation. Distinguish **not implemented**, **source present**, **focused automated verified**, **integrated verified**, **native verified**, and **blocked**. These are different states, not synonyms for “done.”

Do not delete earlier red results. Add the later correction/result next to an explicit date. When changing golden contracts, record the user-visible decision in TODO. The Measurement inspector move at priority 90 is already a recorded design decision; verify any golden changes follow it rather than blindly updating snapshots.

This document supersedes stale status/resume suggestions in older handoff heads, not normative schema or architecture. Keep source and actual test outputs authoritative when they disagree with prose.

## 12. Suggested prompt for the new Claude Code session

> Continue OneCAD correctness and UX hardening from `CLAUDE-CODE-HANDOFF-2026-09-12.md`. Read AGENTS/CLAUDE and reconcile Git first; another actor committed the implementation as `ee5b449` during handoff preparation. Preserve everything and do not commit, push or pull. Delegate coding to the cheapest capable agents, maximum three streams with exclusive shared-file ownership; main reviews and independently runs serial gates. First close current measurement/annotation compilation and current mixed-feature-pattern failures, then build a fresh native app and validate an interim workflow. Continue remaining sketch-session ownership, responsiveness, general-pattern adapters/UI and full native acceptance. Do not treat historical green tests, existing source, or older `status: done` as completion. Track evidence and remaining work append-only in TODO and the original review. No speculative framework upgrade, registry edits, weakened reference safety or mocked native claims.

## Unresolved questions

- Product decisions: none.
- Engineering: exact cause of current mixed-pattern failures; remaining delayed-measurement coverage; sketch-visit receipt design; responsiveness monitor design and native proof; complete adapter scope within the agreed lineage.
- Environment: actual Claude Code model/native-input availability and current app/process/port state must be checked in the new session.
