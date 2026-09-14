# OneCAD viewport-hardening — progress review and validation

**Date:** 13 September 2026  
**Branch:** `viewport-hardening`  
**Reviewed HEAD:** `61dab320878afe7c5aa42d0ab90d5404b39ef2f1`  
**Implementation checkpoint:** `6733174092be636acbbeb38901bd757fdeedcbf6`  
**Specification baseline:** `65b4c60eb226a201f2fa3eb565d7283b1c63b5ac`  
**Contract:** VP-HARDENING 1.0, WP00–WP16, 127 acceptance cases.

## 1. Verdict

**Substantial, directionally correct foundation work exists. The branch is not ready to be accepted as completed viewport hardening, and several implemented foundations need a consolidation pass before additional dependent packages.**

The best changes are the CSS-line/viewport-metrics cleanup, extraction of the on-demand scheduler, owned face-highlight buffers, initial lease accounting, semantic mesh validation, the move to rational-span edge sampling, and surface-derived normals. These follow the intended design rather than replacing the rendering stack.

The most important remaining problems are at module boundaries. A guarded promotion helper still has an unguarded normal-click caller. An installation transaction invokes arbitrary retirement callbacks in the middle of publication. Memory reservations describe installed bodies rather than the complete lifetime and actual layout of live resources. Preview and ghost consumers have not all adopted the new ownership contract.

**Recommendation:** pause new package breadth, repair the boundary defects below, publish reproducible evidence, and complete the outstanding WP03/WP08/WP09 reviews. Then resume the planned dependency order. Do not restart the architecture or rewrite the specification.

This is not a claim that every issue is a newly introduced regression. Findings below distinguish additional review findings, acknowledged follow-ups, and unfinished planned integration.

## 2. What was actually validated

### Review work performed

- Re-read the supplied specification, implementation/acceptance documents and the original rendering review.
- Resolve and pin the branch head, compare it to the design baseline, and recheck that the head remained unchanged at the end of source inspection.
- Inspect the branch's execution ledger, acceptance matrix, verifier, key test sources, and implementation across rendering, resource ownership, ingestion, promotion and worker tessellation.
- Inspect GitHub Actions/check-run metadata and the existence of cited evidence paths in the committed tree.
- Execute isolated Node.js control-flow and numerical probes. These reproduce specific decisions from the inspected source; results and code are included in this package.

### Explicit limits

**The repository's full tests, native application, OCCT worker and GPU pipeline were not executed in this review.** Container-side checkout was blocked by network/DNS access, and the local environment did not provide the project's Bun/Cargo/native macOS stack. GitHub connector reads provided the pinned source.

The reproduction harness is **manually extracted/transcribed logic with explicit doubles**, not a checkout of OneCAD. It establishes counterexamples for particular control-flow and arithmetic decisions. It does not establish native timing, measured GPU memory, rendered pixels, or final kernel-operation outcomes.

The implementation ledger reports successful runs, but the referenced raw logs are not present in the reviewed commit. Accordingly, this report labels those figures **repository-reported**, not independently verified.

Source: [docs/viewport-hardening/execution/STATUS.md](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/docs/viewport-hardening/execution/STATUS.md); [docs/qa/viewport-hardening/acceptance-matrix.json](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/docs/qa/viewport-hardening/acceptance-matrix.json); [scripts/verify-viewport-acceptance.mjs](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/scripts/verify-viewport-acceptance.mjs).

## 3. Progress against the implementation plan

The ledger describes seven work packages as touched: WP00–WP04 and WP08–WP09. Ten others remain not started. **This is not a weighted completion percentage.** Packages differ substantially in scope, and most touched packages still lack integrated/native acceptance.

| Package | Repository checkpoint | Review assessment |
|---|---|---|
| WP00 — baseline and inventory | Focused pass | Useful fixtures, ledger and probes exist. Evidence portability is broken: referenced log files are missing from the commit. |
| WP01 — CSS units and metrics | Focused unit/graphics results reported | Correct direction. Complete DPR 1/1.5/2, themes, projections, before-first-frame and physical display-switch matrix remains open. |
| WP02 — scheduler and recovery | Focused unit results reported | Original reentrant-dirty loss is addressed. Error-loop bound and submission acknowledgement still have holes. Native recovery remains unaccepted. |
| WP03 — ownership and highlights | Focused unit/graphics results reported; second review owed | Owned highlights are an improvement. Preview/ghost ownership, capacity admission, and cross-module publication remain incomplete. |
| WP04 — validation and admission | Focused unit results reported | Real semantic validation exists. Normal promotion can bypass stale-display guards; accounting does not match layout/lifetime. |
| WP05 — appearance and colors | Not started | Assembly dim/restore, appearance precedence and R18 face-color indexing are not fixed by this checkpoint. |
| WP06 — camera and input | Not started | Canonical camera, resize/zoom corrections, exact sketch-plane alignment and input ownership remain planned. |
| WP07 — visibility-aware picking | Not started | Screen acquisition versus occlusion separation and hover currentness remain planned. |
| WP08 — curve sampler | Worker-focused results reported; third review owed | Much stronger structure than midpoint-only sampling, but encoded-angle evidence, caps, closure and numerical certification need correction. |
| WP09 — normals/completeness | Worker tests reported; implementation not finally reviewed | Surface-based normals and internal edge-classification scaffolding are present. Completeness is not enforced end-to-end; degeneracy classification needs correction. |
| WP10 — versioned local origins | Not started | Precision/quality metadata are not yet on the wire. Existing v1 world-float limitations still apply. |
| WP11 — stable sketch resources | Not started | Dense-sketch hover/draft rebuilding remains outstanding. |
| WP12 — shared adaptive quality | Not started | Unified sketch quality, controlled body refinement and isolated display jobs remain outstanding. |
| WP13 — edges/sections/previews | Not started | Effective displayed-solid set, feature-edge policy and complete section-preview behavior remain outstanding. |
| WP14 — acceleration/preparation | Not started | BVH/spatial acceleration and bounded asynchronous preparation remain outstanding. |
| WP15 — grid/input/capture finish | Not started | Grid/touch/pen finishing and capture qualification remain outstanding. |
| WP16 — native qualification | Not started | No native acceptance or professional-readiness claim established. |

The ledger reports **26 focused-pass cases, one implementing, and 100 not-started**. These are recorded case states, not 26 fully accepted requirements: individual cases require multiple lanes, and their notes still identify missing graphics/native/device work.

### Reported test results and their evidentiary value

| Reported result | Correct interpretation |
|---|---|
| TypeScript check exit 0 | Reported checkpoint result; not rerun here. |
| 969 passing viewport/promotion tests and six expected-failure cases, as reported | Useful scoped result, not a clean whole-application suite. The six cases intentionally preserve unresolved R07/R09/R10/R18 failures. |
| Worker focused tests 45/45 | Reported focused success, not independent validation of numerical certificates. |
| Full CTest 197/199 at the WP09 checkpoint | Implementer-reported; two failures described as baseline failures. Exact baseline signatures/logs need publication before accepting that classification. |
| Rust edge-consumer tests 45 passing/3 failing | Scoped reported run; not full workspace acceptance. |
| Line-width graphics probe ~1.04 CSS px at DPR1 and ~1.07 at DPR2 for 1.25 authored | Promising repository-reported change, not measured by this review. Missing full matrix. |
| Hover resource series 11 → 13, then flat | Promising repository-reported result for a limited two-face probe. Not a preview/ghost/multi-document resource soak. |

The committed hover test executes **300 cycles of two pointer moves**, with render synchronization. It does not implement the full 1,000-alternating-hover plus preview/document-cycle qualification described by the acceptance plan. Old notes saying the test still has an expected-failure annotation are stale: the inspected test is now a plain test. Keep the historical record, but add a clear current summary rather than leaving contradictory sentences.

Sources: [docs/viewport-hardening/execution/STATUS.md](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/docs/viewport-hardening/execution/STATUS.md); [docs/qa/viewport-hardening/acceptance-matrix.json](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/docs/qa/viewport-hardening/acceptance-matrix.json); [e2e/vph-hover-lifecycle.spec.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/e2e/vph-hover-lifecycle.spec.ts).

## 4. Findings overview

**P1:** fix before accepting the affected hardening package or claiming its safety contract.  
**P2:** an important validation/instrumentation gap; not evidence of document corruption.  
No P0 document-corruption outcome was established by this review.

| ID | Priority | Finding | Evidence |
|---|---|---|---|
| PR-01 | P1 | Normal viewport picks bypass stale-mesh proof checks | Source call chain + extracted branch reproduction |
| PR-02 | P1 | Mesh publication is not atomic when retirement listeners throw | Source call chain + extracted transaction reproduction |
| PR-03 | P1 | Admission underprices metadata-colored layouts and releases retired resources too early | Source + arithmetic/lifetime reproductions |
| PR-04 | P1 | Highlight capacity growth can exceed the 64 MiB cache ceiling | Source + numerical counterexample; acknowledged follow-up |
| PR-05 | P1 | Reentrant invalidation bypasses the three-error scheduler stop | Extracted scheduler execution |
| PR-06 | P1 | Preview/ghost consumers still violate the new lease/ownership contract | Source; substantially acknowledged in ledger |
| PR-07 | P1 | Exhausted body-edge budget still emits more segments | Source + counting reproduction; acknowledged follow-up |
| PR-08 | P1 | Completeness is internal-only; small positive-area faces can be excused as degenerate | Source + predicate arithmetic; planned integration incomplete |
| PR-09 | P1 | Curve angular certification does not survive encoded geometry consistently | Float32 counterexample + source; pending numerical review |
| PR-10 | P1 | Clean checkout lacks evidence required by its own acceptance verifier | Matrix + committed directory inventory + verifier |
| PR-11 | P2 | Mesh-render acknowledgement ignores the supplied frame contents | Source call chain |
| PR-12 | P2 | Legacy edge-quality test reads positions as edge ranges | Source/schema mismatch |

## 5. PR-01 — stale display guard is bypassed by ordinary selection

**Package:** WP04, with WP07 caller integration. Additional integration finding.

### Source chain

`ViewportRoot.runPick()` builds an `EntityRef`, updates selection, and calls `promotePick(client, ref)`. That helper calls:

```ts
promoteOne(client, ref.bodyId, pick)
```

It supplies neither a captured installed-mesh proof nor an explicit snapshot. `promoteOne()` enforces `installedProofIsCurrent()` and `isEntryPromotable()` only inside its `if (proof && ...)` branch. Without proof, a TopoKey is sent to the client's default promotion path. An `el_` label is returned immediately without any currentness test.

The new stale-inspection-only rule is therefore **not enforced by the ordinary click/promotion path**. Protecting overlap-candidate promotion does not protect all consumers.

### Reproduction and limits

The isolated harness marks proof currentness false. Supplying that proof rejects promotion. Omitting the proof invokes the stubbed backend successfully; a persistent label also passes immediately. This confirms the bypass. It does **not** prove that a real kernel accepts every subsequent stale operation.

A meaningful native regression is: retain body A's old visible mesh after a failed replacement, ordinary-click one of its faces, then initiate a face-based operation. The old displayed topology must not be silently promoted against the new head.

### Required repair

Capture the installed entry and publication identity **when the hit is obtained**. Carry that immutable proof through selection and operation acquisition. Do not manufacture a proof by looking up the current registry entry later; that can pair old hit ordinals with new geometry.

Require proof for viewport-derived face/edge promotion, including the already-persistent-ID fast path. Non-viewport authoritative references need a separately typed, explicit entry point rather than an optional-argument bypass. Audit sketch-on-face, measure, repair, normal selection and tool-specific acquisition paths.

**Acceptance:** test the ordinary click path, overlap chooser, existing `el_` labels, pending/failed replacements, and a response arriving after another publication. Historical inspection remains possible, but may not create a current operation target. Maps to TEST-MESH-05 / TEST-PUB-03.

Sources: [src/ipc/promote.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/ipc/promote.ts); [src/viewport/ViewportRoot.tsx](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/ViewportRoot.tsx); [src/viewport/mesh/meshRegistry.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/meshRegistry.ts).

## 6. PR-02 — the supposedly atomic installation invokes throwing callbacks

**Packages:** WP03 + WP04. Additional cross-package finding.

`MeshIngest.loadBody()` prepares a new entry and body handle, then declares the publish block non-throwing. Its first publication operation is `swap(bodyId, entry)`.

However, `swap()` updates the registry and synchronously calls `retire(previous)`. Retirement invokes arbitrary subscribers without isolation. The real `HighlightLayer` subscriber retires cache entries and may rebuild highlights, including allocations.

A subscriber exception occurs **after registry mutation but before the body scene object and reservation have moved**. The outer catch sees `installed === false`, assumes nothing was published, and demotes the registry's new entry. The finally block releases the incoming reservation. The scene can still display the old entry while picking resolves the new registry entry.

### Isolated result

```text
Registry: new entry
Scene:    old entry
Publish completion flag: false
Incoming reservation: released
Old entry: retired
```

This follows the extracted source order under an injected retirement-subscriber exception. It is not a native out-of-memory reproduction, and does not establish a persistent-document mutation.

### Required repair

Make retirement marking a non-observing operation inside the transaction. Update registry, effective display/scene, reservation ownership and installed identity as one coherent commit. Notify retirement/highlight/submission subscribers afterward, with explicit exception isolation and diagnostics. A post-publication callback failure must not roll the geometry back conceptually or mislabel it stale.

Every preparation exit must release the uninstalled handle lease, geometry and reservation. Do not fix this by merely catching the exception around `swap()` while leaving registry/scene mismatch intact.

**Acceptance:** inject throwing retirement, highlight, display-state and body-loaded subscribers at each boundary. After each failure the scene, registry, admission and pick identity must all describe one consistent display snapshot. Also test failure after `buildBodyObject` but before final installation.

Sources: [src/viewport/mesh/meshSync.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/meshSync.ts); [src/viewport/mesh/meshRegistry.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/meshRegistry.ts); [src/viewport/engine/HighlightLayer.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/HighlightLayer.ts).

## 7. PR-03 — admission does not price the actual live resource set

**Packages:** WP03 + WP04. Additional findings. These are distinct mechanisms, not one cosmetic accounting correction.

### A. Metadata colors change the layout after it was priced

`validateMeshView()` charges the de-indexed color layout only when `view.faceColors` exists. It receives no body or authored-face appearance metadata.

Later, `buildFaceGeometry()` uses `needsVertexColors(view, bodyColor, authoredFaceColors)`. That predicate also returns true for a body color or a nonempty authored-face-color map. A native mesh without a FACE_COLORS payload section can therefore be admitted as indexed geometry and built as de-indexed position/normal/color streams.

For a concrete **assumed** mesh of 1,000,000 triangles and 500,000 vertices with normals:

```text
Priced indexed geometry:          24,000,000 bytes
Prepared de-indexed color data:  108,000,000 bytes
Difference:                      4.5×
```

This is arithmetic from the inspected layouts, not measured VRAM. Edge data and object overhead are excluded. CPU expansion is also missing from the price in this metadata-only color case.

### B. Old resources stop being charged while still alive

The outgoing reservation is released at swap/drop. But the resource remains alive until lease release and retirement flush. Multiple replacements before the next flush, or a real borrower retaining the outgoing entry, leave live resources uncharged. The initial old+new check is not a lifetime peak bound.

The harness models repeated 40-unit resources under a 100-unit budget: the reservation counter returns to 40 after each swap while held resources can total 200. This is a lifetime-policy counterexample, not an observed 200-unit allocation in OneCAD.

### C. Other lanes and costs must not remain implicit

The ledger acknowledges that preview/ghost raw views are validated without document admission. The price also needs to account for retained backing buffers, not merely convenient logical section totals, and for capacity rather than just used elements. A zero-copy view can retain more source storage than its own slice describes.

### Required repair

Create one immutable preparation plan that includes source buffer retention, appearance-dependent geometry layout, expanded edge data and actual allocation capacities. Reserve against that plan **before** derived allocation, and execute exactly that plan. Avoid separate predicates in accounting and construction that can drift.

Tie reservations to resource identities/final disposal, not only `bodyId` replacement. Leased retired resources remain charged. Share the admission authority with preview and other live preparation lanes; make final release idempotent.

**Acceptance:** metadata-only body/face color; imported FACE_COLORS; uncolored controls; multiple same-body replacements before a frame; a held old lease; preview cancellation; document close; every failure after reservation. Assert counters equal the resources the application still owns, not just the installed body map.

Sources: [src/viewport/mesh/validateMesh.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/validateMesh.ts); [src/viewport/mesh/meshAdmission.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/meshAdmission.ts); [src/viewport/mesh/meshSync.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/meshSync.ts); [src/viewport/mesh/meshRegistry.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/meshRegistry.ts); [src/viewport/mesh/faceColors.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/faceColors.ts).

## 8. PR-04 — the highlight cache can exceed its advertised capacity

**Package:** WP03. Acknowledged capacity/admission follow-up, confirmed here.

`HighlightLayer.buildFaceSet()` reserves `estimateFaceSetBytes(newSelection)`. It then reuses `OwnedFaceGeometry`, whose arrays grow by 1.5× and retain capacity after shrink. The cache `put()` charges the resulting actual capacity but does not recheck its ceiling.

A valid counterexample using the current formulas:

```text
Cache limit:                       67,108,864 bytes (64 MiB)
Old selection, 1,000,000 triangles:48,000,000 bytes
New selection estimate, 1,100,000: 52,800,000 bytes → reservation succeeds
Reused buffer after 1.5× growth:  72,000,000 bytes → exceeds the ceiling
```

This does not require an already-over-budget starting state or other pinned entries.

**Repair:** compute the exact planned position/index capacities before reservation. Include the reusable buffer's retained allocation and any simultaneous old/new allocation required by growth. Reserve those bytes, then allocate. Use a transactional resize/reservation or equivalent single authority; do not allocate first and discover the excess afterward.

**Acceptance:** custom small-budget unit test (four triangles to five demonstrates the same 192→240 estimated→288 actual pattern), near-64-MiB graphics test, pinned selection pressure, shrinking then regrowing, and stable degraded visualization with unchanged semantic selection. Maps to TEST-RES-03.

Sources: [src/viewport/mesh/faceSliceGeometry.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/faceSliceGeometry.ts); [src/viewport/engine/HighlightLayer.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/HighlightLayer.ts); [src/viewport/mesh/highlightCache.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/highlightCache.ts).

## 9. PR-05 — a failing callback can create a permanent frame loop

**Package:** WP02. Additional finding, independently exercised as extracted scheduler logic.

The original dirty-mask ordering problem is fixed: the mask is consumed before callbacks. The new `WORK_ERROR_LIMIT = 3` protection has a separate hole.

A work callback calls `invalidate()` and then throws. `invalidate()` immediately queues another frame. After three errors, `finally` stops calling `ensureFrame()`, but does not cancel the frame already queued inside the failing callback.

The exact extracted state-machine probe produced:

| Scenario | Executed attempts | Frames still queued |
|---|---:|---:|
| Throw without reinvalidation | 3 | 0 |
| Re-invalidate, then throw | 12, deliberately capped by harness | 1 |
| Healthy one-time reinvalidation | 2 | 0 |

A production-relevant entry is an after-render listener: `notifySubmitted()` invokes listeners without exception isolation, and listeners may invalidate. The renderer-call error catch does not cover every layer/listener failure.

**Repair:** prevent scheduling from inside a tick until its outcome is known, or cancel outstanding internal requests when parking on failure. Preserve dirty state for explicit external recovery, but distinguish internal failure churn from a genuine user/retry wake. Handle extension/listener exceptions independently where possible. Keep one-rAF coalescing and zero idle frames.

**Acceptance:** the three scenarios above, contribution/listener variants, context suspend/resume, explicit external retry, and disposal during a callback. The stop must be observable without hiding the original error.

Sources: [src/viewport/engine/FrameScheduler.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/FrameScheduler.ts); [src/viewport/engine/ViewportEngine.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/ViewportEngine.ts).

## 10. PR-06 — borrowed-resource migration is incomplete

**Package:** WP03. Substantially acknowledged follow-ups, confirmed in source.

### Exact preview handles

`buildBodyObject()` now acquires a lease and stores each handle in module-level `liveHandles`. The preview paths in `ViewportEngine.setPreviewBody()` and `clearPreviewBody()` still remove groups without calling the returned handle's `dispose()`.

A global retirement-time detached-handle sweep attempts to compensate. It is not equivalent to ownership: it only runs when a retirement event occurs, and detached preview resources can remain in `installed` state outside the committed registry or be disposed directly without triggering that event. The strong set can retain handles/entries after the preview is gone.

**Required fix:** explicitly dispose every preview handle on replacement, selected-ID clear, full clear, engine teardown and failed preparation. Remove the sweep once the actual owners are correct. Do not retain a global cleanup heuristic as the normal contract.

### Ghost resources

`GhostLayer` still creates ranged `shareIndexed()` wrappers and deliberately does not dispose them. It also borrows whole registry geometry without a lease. The old ownership defect therefore remains in a different editing path even though ordinary hover now uses owned buffers.

**Required fix:** whole-body ghosts borrow the exact geometry with leases. Ranged ghosts use owned compact geometry or another explicitly owned resource strategy. Clear/hide/source replacement/document close release their own resources. The semantic preview remains unchanged.

### Section leases must name the resource actually drawn

`SectionLayer.createPair(mesh)` looks up an entry by body ID, takes its lease, but draws `mesh.geometry`. It does not verify that this is the same geometry. Under registry/scene divergence, it can lease one resource and draw another. Carry exact entry identity from the displayed object, or verify identity and refuse a mismatched borrow. This issue is particularly exposed by PR-02.

**Acceptance:** TEST-RES-05's document/preview cycles, ranged OffsetFace ghosts, pattern/mirror ghosts, retirement while previews remain displayed, and source-entry identity mismatches. Record both application lease counts and renderer geometry counts.

Sources: [src/viewport/engine/BodyObject.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/BodyObject.ts); [src/viewport/engine/ViewportEngine.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/ViewportEngine.ts); [src/viewport/engine/GhostLayer.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/GhostLayer.ts); [src/viewport/engine/SectionLayer.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/SectionLayer.ts).

## 11. PR-07 — exhaustion of the edge budget still adds geometry

**Package:** WP08. Acknowledged numerical follow-up, confirmed in the caller.

The sampler itself refuses work when `remainingBodySegments` is zero. Its caller does something different: once the 2,000,000-segment body budget is exhausted, every remaining nondegenerate edge gets a two-point endpoint fallback. Each contributes another segment, and the counter remains clamped at zero.

Five additional edges can therefore produce 2,000,005 segments. More importantly, the declared body cap is not a cap.

Retries also relax requested tolerances up to four doublings. Returning the retry's certificate without preserving the original requested-versus-achieved status can mislead later metadata consumers. A warning log is useful but is not a machine-readable display-quality contract.

**Repair:** a hard exhausted cap must not allocate more segments. Keep edge ID/range tables aligned, but return explicit incomplete/quality-limited status for omitted drawable data; never describe the body as complete. Where a relaxed retry is allowed, preserve requested tolerance, achieved tolerance and degraded status separately. Preserve cancellation and deterministic edge order.

**Acceptance:** very small injected cap, many straight edges, many closed edges, the final edge exactly filling capacity, an edge too complex for its remaining share, and cancellation during retries. Assert total emitted segments and quality metadata, not only a warning or sampler-local limit.

Sources: [worker/src/tess/Tessellate.cpp](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/worker/src/tess/Tessellate.cpp); [worker/src/tess/CurveSampler.cpp](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/worker/src/tess/CurveSampler.cpp).

## 12. PR-08 — complete tessellation has not been established end-to-end

**Package:** WP09 / WP10 / WP13 boundary. Planned integration gap plus an additional degeneracy-policy problem.

### Internal diagnostics do not protect publication

The worker now records `allNondegenerateFacesCovered`, missing faces and normal provenance counters. This is useful. At the end of the same function it still encodes the v1 blob and sets `out.ok = true`.

The mesh payload consumed by the current frontend does not carry this completeness result. A zero-triangle face range can be structurally and semantically valid, so the frontend cannot distinguish a known degenerate face from missing required geometry using those ranges alone.

This report does not claim that metadata with an explicit `complete=true` bit was transmitted; no such completed v2 path exists here. The problem is that **successful transport/install state can present an incomplete body as ordinary current geometry**. TEST-MESH-06 is not satisfied by local C++ counters alone.

### Small area is not proof of degeneracy

`face_is_degenerate()` treats positive area as degenerate whenever:

```text
area <= 1e-12 × bodyDiagonal²
```

For a 10,000 mm diagonal, the threshold is 0.0001 mm². A positive-area patch below that threshold is not mathematically degenerate merely because its host is large. This predicate runs after a face fails to produce drawable triangles; it can excuse that failure rather than diagnose it.

The included arithmetic probe does **not** demonstrate that OCCT normally drops such a face. It demonstrates that the fallback classification can misreport a missing small face.

**Repair:** distinguish true degeneracy, display-detail omission, numerical failure and incomplete tessellation. If the complete published contract is not yet available, reject the incomplete candidate or explicitly mark it non-complete and non-actionable at the outer publication boundary. Preserve last valid display rather than silently pretending missing geometry is acceptable.

Do not decide topology degeneracy from a global body-relative area threshold. A legitimate display omission belongs to quality/completeness status and cannot satisfy a proof of complete topology coverage.

**Acceptance:** inject a missing nondegenerate face and missing edge; include legitimate cone/sphere degenerate topology; test a small valid face on a large body; follow the result through native worker, IPC, installation, picking and section caps. Keep fallback normals and singular-node provenance visible in diagnostics.

Sources: [worker/src/tess/Tessellate.cpp](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/worker/src/tess/Tessellate.cpp); [worker/src/tess/SurfaceNormals.cpp](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/worker/src/tess/SurfaceNormals.cpp); [worker/src/tess/SurfaceNormals.h](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/worker/src/tess/SurfaceNormals.h); [src/viewport/mesh/validateMesh.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/validateMesh.ts).

## 13. PR-09 — encoded geometry must agree with angular claims

**Package:** WP08. Pending numerical review; an independent numerical counterexample is supplied here.

### The double-precision cone is not enough

The span acceptance test evaluates the tangent cone about the **double-precision chord**. After float32 conversion, `apply_measured_quantization()` only withdraws the angular claim when chord length is less than 16 times endpoint quantization error.

That heuristic does not enforce a 5° angular target, nor the tighter half-cone used by the test.

Use this exact straight-edge example, in mm:

```text
A = (1000000.03, 0,    0)
B = ( 999999.97, 0.55, 0)
```

Float32 stores both X coordinates as 1,000,000. The encoded line loses its X slope. Independent arithmetic gives:

```text
Original chord length:          0.553263047751 mm
Maximum endpoint quantization: 0.030000000028 mm
16 × quantization:             0.480000000447 mm
Angular guard withdraws claim: false
Encoded chord direction error: 6.225829070179°
```

At chord tolerance 0.05 mm and angular tolerance 5°, the positional tolerance remains satisfied, but the angular claim is not. **This is not a counterexample to the positional/chord bound.** It is a counterexample to leaving the leaf's angular state `Satisfied` after encoding.

### Closure is still inferred from a parameter epsilon

`edge_is_semantically_closed()` falls back to `span >= period - 1e-9`. An open trim of `period - 5e-10` therefore passes that fallback even when the topology closure check was false. Subsequent endpoint snapping can close the displayed polyline. Report actual topological closure separately from approximation closeness.

### Other proof obligations remain

The code introduces an angular coefficient floor, weight-conditioning floor, and a resolution-exhaustion escape. Their roles must be explicitly separated into certified, unestablished and quality-limited outcomes. A constant named “noise” is not by itself a proven error enclosure for every supported rational span. Source-span generation, endpoint movement, subdivision roundoff, and encoded output all belong to the same declared result contract.

**Repair:** evaluate or conservatively bound direction error about the actual encoded chord and account for endpoint changes. Carry positional and angular status separately. Re-establish the cone after endpoint snapping, or downgrade the angular status with explicit evidence. Use topological closure or a proven complete periodic traversal, never a generic near-period shortcut to alter an open trim.

**Acceptance:** the straight-edge counterexample above through actual OCCT and mesh encoding; conics and rational splines; source endpoint adjustments; regular near-zero derivatives versus genuine cusps; nearly periodic but open trims; supported weight/degree extremes; cap exhaustion and explicit approximate status. Do not convert dense sampling into a proof certificate.

Sources: [worker/src/tess/CurveSampler.cpp](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/worker/src/tess/CurveSampler.cpp).

## 14. PR-10 — acceptance evidence is not portable with the checkpoint

**Packages:** WP00 and all passing states. Additional validation finding.

The committed matrix contains focused-pass rows whose evidence is a path such as:

```text
docs/qa/viewport-hardening/runs/wp01-focused/vitest.log
```

The committed `docs/qa/viewport-hardening/` directory contains the matrix and fixtures, but no `runs/` directory. A direct fetch of the WP03 graphics log returned not found. `.gitignore` excludes `*.log`.

The verifier explicitly rejects any passing row whose evidence file does not exist. Therefore **a clean checkout cannot pass the current acceptance verifier as committed**. This conclusion follows from the checked-in matrix, directory inventory and `existsSync` condition; the full verifier was not executed against a local checkout here.

GitHub API queries also returned no Actions runs for this branch and no check runs attached to the reviewed head. That is not a claim that the repository lacks CI generally or that local tests were never run. It means there is no retrieved CI artifact filling the evidence gap.

### Required repair

Choose a repository-compatible evidence policy. Commit sanitized compact logs or machine-readable run summaries with hashes; keep bulky screenshots/profiles in durable CI artifacts addressed by a checked-in manifest. Add narrowly scoped ignore exceptions if raw logs are intentionally versioned.

Each result needs source/worktree identity, command, environment, exit status, case/failure list, timestamp and artifact checksum. Until evidence is accessible, keep claims explicitly “local reported; artifact unavailable” rather than manufacturing a new passing state. Baseline failures require matching signatures at both baseline and checkpoint.

Run the verifier on a **clean checkout**, not only an implementation worktree that happens to contain ignored files. Do not remove the existence check just to make the gate green.

Sources: [docs/qa/viewport-hardening/acceptance-matrix.json](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/docs/qa/viewport-hardening/acceptance-matrix.json); [scripts/verify-viewport-acceptance.mjs](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/scripts/verify-viewport-acceptance.mjs); [.gitignore](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/.gitignore). Tree/CI observations were obtained from GitHub API for the pinned commit/branch.

## 15. PR-11 — render acknowledgement ignores the frame's displayed set

**Package:** WP02/WP04. P2 additional integration gap.

The engine now supplies a `FrameSubmission` with body IDs and per-body publication provenance. That is the correct direction and explicitly describes a renderer submission, not compositor presentation.

`MeshIngest.traceRendered()` still subscribes with a callback that ignores that argument. It checks that the body remains in `bodyObjects`, not that it was included in the frame. If isolation, a layers toggle or a replacement preview hides the body before the next frame, it can acknowledge the original expectation on an unrelated frame.

**Repair:** match the expected body and resource/publication identity to the actual submission record. A hidden/removed target should receive an explicit no-longer-required outcome where the protocol supports it, not a false rendered acknowledgement or endless timeout. A successful renderer call is not proof that a particular body was submitted.

**Acceptance:** hide/isolate between installation and the frame; mixed publications; context loss; frame submission failure; replacement preview; a frame that contains other bodies but not the expected one.

Sources: [src/viewport/engine/ViewportEngine.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/ViewportEngine.ts); [src/viewport/mesh/meshSync.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/meshSync.ts).

## 16. PR-12 — an existing edge-density test reads the wrong section

**Package:** WP08 validation. P2, existing test defect that should be corrected before relying on the suite.

`worker/tests/test_tessellation_quality.cpp` defines `kEdgeRanges = 8`. In the normative format, type **7** is EDGE_RANGES and type **8** is EDGE_POSITIONS.

The test reads float position bytes as unsigned point counts. Its edge-density assertions are therefore not evidence of actual edge sampling density. The separate triangle-density assertion is not invalidated by this observation.

**Repair:** use the correct named section identifier, validate the expected range-array length and point bounds, and include an independently authored tiny binary fixture whose ranges and position bit patterns are deliberately different. Then retain independent geometric error checks; density alone does not prove curve quality.

Sources: [worker/tests/test_tessellation_quality.cpp](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/worker/tests/test_tessellation_quality.cpp); [protocol/mesh_format.md](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/protocol/mesh_format.md).

## 17. What should be preserved

The review is not a recommendation to discard this work.

**CSS metrics:** one logical-pixel contract and event-driven DPR updates address an actual prior defect. Keep width and pick acquisition in CSS pixels and framebuffer allocation in device pixels. Finish the device matrix instead of changing the contract again.

**Highlight architecture:** owned compact face buffers and leased exact whole-body geometry are preferable to undisposable shared-attribute wrappers. Complete every consumer and fix budget growth rather than adding another caching/ownership layer.

**Semantic validation:** finite values, normal lengths, indices, contiguous topology ranges and UTF-8 checks are meaningful improvements. Keep them, while coupling preparation/admission and respecting stale display semantics across all acquisition paths.

**Worker geometry:** rational control-hull subdivision and surface-derived normals address root causes. The former needs honest numerical evidence at the encoded output; the latter needs completeness propagation and scale-correct degeneracy handling.

**Scope discipline:** leaving WebGPU, AO and cosmetic rendering out of the first correctness pass is appropriate. The branch cannot yet be judged as if camera, sketch batching, local origins, new picking and section integration had landed; the ledger explicitly says they have not.

Sources: [docs/viewport-hardening/execution/STATUS.md](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/docs/viewport-hardening/execution/STATUS.md); [src/viewport/engine/FrameScheduler.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/FrameScheduler.ts); [src/viewport/engine/HighlightLayer.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/engine/HighlightLayer.ts); [src/viewport/mesh/validateMesh.ts](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/src/viewport/mesh/validateMesh.ts); [worker/src/tess/CurveSampler.cpp](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/worker/src/tess/CurveSampler.cpp); [worker/src/tess/SurfaceNormals.cpp](https://github.com/andrejvysny/OneCAD/blob/61dab320878afe7c5aa42d0ab90d5404b39ef2f1/worker/src/tess/SurfaceNormals.cpp).

## 18. Recommended next execution sequence

### Checkpoint A — portable evidence and failure reproduction

Repair PR-10; publish the existing local evidence honestly or rerun the necessary focused gates. Add failing repository tests for PR-01 through PR-05 before fixes. Correct PR-12 so worker green results have a trustworthy edge oracle. Preserve baseline and red outputs.

### Checkpoint B — one coherent displayed-resource contract

Fix PR-01/02/03/04/06 together as a coordinated WP03/WP04 consolidation. Ownership, publication, preparation plans and promotion identity are inseparable here. Give each shared file one owner; do not run independent uncoordinated rewrites of the registry and ingestion controller.

Close PR-05 and PR-11 in the scheduler/submission stream. A focused test should exercise real engine listeners as well as the pure scheduler.

### Checkpoint C — honest worker output

Complete WP08's outstanding adversarial review using PR-07/09. Review WP09 using PR-08. Keep unit-normal, triangle-winding and seam checks; add completeness and encoded-geometry assertions. Do not mark complete merely because the first gallery passes.

WP10 may then carry the validated origin/quality/completeness design across protocol boundaries. Until that metadata is available, incomplete geometry needs an explicit non-success/degraded publication policy.

### Checkpoint D — resume existing work packages

Resume WP05–WP07 and the WP10–WP15 dependency graph. Canonical camera/input, depth-aware acquisition, stable sketch buffers, shared adaptive quality and effective-solid section policy are still necessary to achieve the professional viewport goal. Do not spend this phase on a new backend or decorative effects.

### Checkpoint E — native qualification

Run integrated frontend, worker, cross-track protocol and native lanes on the same pinned candidate. Native acceptance must include actual Tauri IPC/OCCT, DPR and physical input behavior, context loss/recovery, large-model resource limits and long editing sessions.

## 19. Acceptance gates for the consolidation pass

| Gate | Required result |
|---|---|
| Clean checkout | Acceptance verifier succeeds with accessible artifacts, without ignored local files. |
| Promotion | Ordinary clicks cannot promote stale/pending historical geometry; every viewport acquisition carries captured identity. |
| Atomicity | Every injected callback/build failure leaves registry, scene, budget and selection identity consistent. |
| Resource cost | Metadata-colored layouts are priced correctly; retired leases remain charged; preview lanes use admission. |
| Cache bound | Actual capacities never exceed the configured overlay ceiling; pressure uses documented degradation. |
| Lifetime | 1,000 alternating hover events, 100 preview apply/cancel cycles and 50 document cycles produce bounded resources. |
| Scheduling | Healthy reentrant invalidation redraws; repeated reentrant failure parks; explicit recovery works. |
| Worker bounds | Hard segment caps remain hard; requested and achieved tolerance/status stay distinct. |
| Worker completeness | Missing required geometry is never indistinguishable from a complete current display. |
| Encoded quality | Angular/position claims refer to the emitted representation, including quantization and endpoint changes. |
| Regression | Full relevant suites have no new failures; any baseline allowlist is exact and independently reproducible. |
| Native | Tests exercising the native path are run and attached, not substituted by mock-browser or isolated arithmetic evidence. |

## 20. Reproduction package

Run:

```bash
node reproductions/review-probes.mjs
```

`reproductions/results.json` records this review's execution under Node.js v22.16.0. Passing harness assertions mean the isolated counterexamples behaved as documented; **they do not mean OneCAD passed its acceptance tests**.

See `reproductions/README.md` for extraction scope. The repository was not modified. `02-CODING-AGENT-CORRECTION-BRIEF.md` turns this review into a bounded implementation handoff without replacing VP-HARDENING 1.0.
