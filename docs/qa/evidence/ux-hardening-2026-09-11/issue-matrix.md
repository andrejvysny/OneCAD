# UX hardening issue matrix — 2026-09-11 baseline

Status means **as of this baseline**, not a claim about the current dirty
worktree: **implemented earlier** is the engineering claim in Addendum D;
**unverified** needs an independent native pass; **open** is observed, deferred,
or a newly agreed hardening gap. IDs map to the original review and Addenda C,
D and E without changing those records.

| ID / finding | Baseline status | Required hardening outcome |
|---|---|---|
| H1/C1 Undo and redo recovery | Implemented earlier; native keyboard/menu behavior unverified; E3 open | Atomic feature-plus-auto-visibility undo, semantic labels, exact palette ranking, no-op clean state |
| H2 Shell/native recovery | Open as E1 | Reproduce stalled publication, find lock owner, retain responsive last-valid model and evidence |
| H3 stale/displaced selection | Implemented earlier; redo visual and recovery cleanup unverified; E6 open | Authoritative reconciliation, clear transient hover, no stale actionable inspector |
| M1 numeric first input | Implemented earlier; focus/rapid-entry unverified | Separate draft/parsed/preview/commit values; preserve typing, paste, Tab and Enter |
| M2 oversized fillet | Implemented earlier; persistent native validation unverified | Block invalid confirmation; explain limit and offer non-committing Use maximum only when known |
| M3 floating/weak controls | Partly implemented earlier; E2/E5/E7 open | Shared compact movable chip plus dockable inspector, semantic labels/units and no target occlusion |
| M4 sketches intercept solids | Implemented earlier; candidate UI open | Ordered overlap candidates, filters/cycling and explicit consumed-sketch visibility |
| M5 history incompleteness | Implemented earlier; E3/E5 open | Full semantic history with stable, responsive editing and distinguishable names |
| L1 constraints messaging | Partly implemented earlier; E6 open | Solver-currentness; unknown/restored is Not evaluated, never fabricated DOF 0 |
| L2/C7 snap rule opacity | Partly implemented earlier; isolated correctness unverified | Show winning snap/rounding, snap before solve, Alt bypasses snap only |
| C2 sketch Cancel | Implemented earlier; existing-sketch/refused rollback unverified | Correct entry-state rollback, atomic new-sketch discard and explained refusal |
| C3 feature patterns unavailable | Open | Versioned FeaturePattern for explicit same-lineage mixed chains; preserve Body Pattern V2 |
| C4 vertex drag/DOF affordance | Open | Constraint-authoritative drag, local blocking explanation and physical-input validation |
| C5 boolean intent/direction | Implemented earlier; breadth unverified | Explicit Add/Cut/New, affected-body/non-intersection feedback |
| C6 construction persistence | Implemented earlier; presentation regression unverified | Per-entry off default and stable active indicator |
| C8 measurement congestion | Partly implemented earlier; E5 open | Fixed single/pair panel, collision avoidance and independent pin/hide |
| E1 published-operation native stalls F/H/O | Open, release-blocking | Correlate commit through rendered completion; bounded/coalesced mesh work and evidence-on-timeout |
| E2 pointer versus Enter region result | Open, release-blocking | Shared single-flight confirm and interactive-UI boundary; identical selection and operation count |
| E3 undo granularity, generic labels, dirty no-op | Open | Atomic backend transaction and semantic command/history presentation |
| E4 invalid Offset/Pattern tool state | Open | Authoritative applicability, retained valid state, visible Cancel and target-correction recovery |
| E5 occlusion, offscreen entry, inspector overflow | Open | Stable 280–420 px inspector, relocatable/dockable chip, fit selection and responsive history |
| E6 stale visual/metadata after regen/recovery | Open | Reconcile selection, clear hover, current solver state and no removed-object actions |
| E7 phase wording/repeated Hole setup | Open | Normalized stage prompts/units and session-only successful Hole defaults |
| Audit: toolbar/tool grammar | Open | One `ActiveToolPresentation` source for chip/inspector order, validation and accessibility |
| Audit: fillet/chamfer reference clarity | Open | Named A/B references, persistent highlight and consistent secondary unit labels |
| Audit: shell/hole/offset placement | Open | Small near-geometry primary chip; secondary settings/errors in inspector; geometry-aware placement |
| Audit: pattern framing/invalid start | Open | Preview fit and valid-selection recovery without switching tools |
| Audit: navigation/section | Open | Preserve camera context; expose section plane/offset/flip without model history mutation |
| Audit: completion/autosave recovery | Open | Semantic completion, visible recoverability and retain/retry/cancel failure state |
| Plan: ambiguous reference recovery | Open | `NeedsRepair` on uncertain/congruent binding; no silent nearest-geometry rewrite |
| Plan: C++ producer-reference guard | Open | Remove unresolved producer fallback; reject an instance/reference that cannot be proven to originate from the selected pattern chain |
| Plan: explicit TopoKey promotion guard | Open | `AcquireElementIds` must fail closed when an explicitly supplied TopoKey is invalid; never fall back to nearest anchor |
| Plan: event/mesh lifecycle observability | Open | Debug-only stage telemetry and timeout samples; do not infer safe completion from worker publication |
| Plan: mesh request coalescing | Open; first review rejected | Add runtime/epoch ownership and stale-generation/ABA regressions before coalescing requests |
| Plan: active-tool presentation / UI boundary | Partly tested; native-unverified | One source for chip/inspector state, pointer-safe confirmation and no active-drag release regression |
| Plan: tool lifecycle feedback ordering | Open | Preserve published completion feedback across Cancel/tool switch and prevent an old non-preview commit from resetting a newer active tool |
| Plan: inspector migration | Open/WIP | 320 px default, 280–420 px bounds, dock/relocate behavior and complete legacy-overflow replacement across tools |
| Plan: persistence | Unverified | Isolated Save/Open/autosave QA comparison of geometry, parameters, dependencies, solver and dirty state |
| Plan: feature-pattern atomicity/identity | Open; first implementation under adversarial review | Complete pre-publication validation, transforms, per-instance provenance and deterministic repair/undo/persistence; negative-only tests do not establish mixed-chain geometry |
| Plan: delayed mesh transport | Scoped fake-provider coverage; native-pending | Delayed-mesh 4/4 validates a fake delayed provider only; validate coalescing/retirement/render completion against real worker and native UI |
| Plan: FeaturePattern coverage-manifest integration | Open integration defect | Add the required FeaturePattern manifest row(s) for both `KnownOperation` and worker dispatch, then rerun coverage and the chained tracing guard |
| Plan: precise Sketch Cancel rollback | Focused tests passed; native-unverified | Full `NetStep` rollback and mutation notification are reviewed, including readonly foreign gestures; preserve explicit existing-sketch/refused-discard boundaries and validate natively |
| Plan: accessibility/window matrix | Unverified | 1024×768, 1156×768, 1440×900, Retina/enlarged UI; no inaccessible confirmation or overflow |
| Plan: physical controls | Unverified/blocked by automation capability | Human or supported-device coverage for hover, modifier drag, key-repeat and trackpad navigation |

## Acceptance evidence rule

No row becomes closed from code review alone. A row requires focused automated
coverage where feasible, the required integration gates, and independent native
evidence when it changes Tauri, worker, persistence, interaction or visible
modeling behavior. Preserve blocked coverage as blocked rather than passing it.

## Dated checkpoint events

These events update evidence without rewriting the baseline rows above.

| Date | Baseline row(s) affected | Event | Current implication |
|---|---|---|---|
| 2026-09-11 | explicit TopoKey promotion guard | Focused tests now cover invalid explicit-key fail-closed behavior, finite-anchor uniqueness, and shared-topology dedupe. | Automated evidence only; real OCCT/native acceptance remains pending. |
| 2026-09-11 | tool lifecycle feedback ordering | Detached submitted-promise feedback is in progress. | The baseline risk remains open until sequencing/epoch regressions pass. |
| 2026-09-11 | inspector migration | All tool families render through the frame; drawer-quality review, placement, dock/relocate and legacy-overflow work remain. | WIP only; not a UX acceptance result. |
| 2026-09-11 | feature-pattern atomicity/identity | Fresh real-OCCT CTest 5/5 is bounded; the worker currently recognizes same-host Hole→Chamfer and independently-child Sketch→Blind/NewBody/one-direction Extrude→straight Fillet. | Deferred/hidden manifest rows track only those adapters; no general mixed-chain support claim. |
| 2026-09-11 | FeaturePattern coverage-manifest integration | Contracts verifier passed 39 rows/18 operations/15 tier-checked; coverage initially failed for missing FeaturePattern rows. The tracing guard was run separately afterward and passed with runtime-wry tracing disabled. | Manifest integration follows in this checkpoint; the earlier chained guard non-execution remains historically true. |
| 2026-09-11 | FeaturePattern coverage-manifest integration | Two deferred/hidden rows now name only the actual bounded worker adapters. Coverage passed 34 rows/9 corpus cases/20 registry operations; contracts passed 41 rows/19 operations/15 tier-checked. | Registry tracking is restored, not general FeaturePattern acceptance; typed Rust integration is still in progress and uncited. |
| 2026-09-11 | navigation/section | LayersMenu/NavPill focused suites passed 15/15; numeric precision, invalid alert, shared state and navigation mapping had bounded review. | Shared-popover focus/clamp and native UI validation remain open. |
| 2026-09-11 | feature-pattern atomicity/identity | Independent `cargo feature_pattern_integration` `independent_pattern_survives_edit_history_and_reopen` failed 0/1 during source-edit regeneration: source Extrude 10→16 leaves only the source and reports `FEATURE_PATTERN_PRODUCER_BIND` at instance 1 Fillet input 0. First three bodies/identity metrology pass; reopen assertions were not reached. | Native_plan owns the correction and tools_plan owns the test; no ignored failure or general reopen claim. |
| 2026-09-11 | failed-model rollback / transaction safety | A generic `client.undo` rollback can execute after the native API locks a different runtime, potentially undoing a replacement document or interleaved top transaction despite frontend result guards. | Native_fix is replacing blind undo with a backend-generated receipt bound to runtime instance, exact undo entry and revision plus conditional rollback. Null/no-op without a receipt must never undo prior work; frontend integration and independent runs remain pending. |
| 2026-09-11 | feature-pattern atomicity/identity | Real-worker `feature_pattern_integration` passed 2/2 for same-host Hole→asymmetric-Chamfer and independent Sketch/Extrude/Fillet source edits, history, raw persistence and fresh-worker reopen. Selected CTest passed 11/11 including canonical malformed/repair. | Bounded-adapter evidence only; general pattern adapters and native UI acceptance remain open. |
| 2026-09-11 | failed-model rollback / transaction safety | Rollback IPC/tauriClient and Picker/ViewportEngine focused tests passed 211. | Receipt must remain unarmed for no-op transactions or a previous undo top; active-edit signature mismatch prevented a stable-tree main receipt cargo gate. |
| 2026-09-11 | retained-failure UX | Review found numeric validation can reset the controller retained-failure block. | Require an independent typed validation block; do not accept failure preservation from current focused tests. |
| 2026-09-11 | failed-model rollback / transaction safety | Main independently ran receipt tests 5/5, same-document replacement 1/1, TypeScript, and frontend 145. Native_fix subsequently found and corrected a depth-only undo-cap issue. | Final independent rerun is still owed; this does not close receipt correctness or native acceptance. |
| 2026-09-11 | inspector migration / toolbar grammar | Latest history/Inspector/chips/presentation/edgeShell suites passed 217/217. An earlier model-tools/chips run had 46 files/889 passed, but an Inspector suite failed to load during Terra's in-progress HistoryList parse edit. | No stable integrated UI-green conclusion. Native UI validation and the responsive matrix remain open. |
| 2026-09-11 | accessibility/window matrix | The initial 48-test run was blocked at launch (Chromium Mach bootstrap permission and WebKit abort; provenance `2026-09-11T19-47-10-301Z`). A max-fail-1 retry had 1 pass, 1 failure, 46 not run; it isolated history buttons named only `Extrude`, colliding with toolbar labels. The exact regression later passed 2/2 on Chromium and WebKit with retries 0. | Preserve both failed/blocked runs. The exact defect is covered; the broad browser workflow is not passed. |
| 2026-09-11 | event/mesh lifecycle observability / stale visual metadata | SnapshotPublisher resets counters on reopen, so the same `(docId, snapshot, generation)` can repeat across sessions. | Native_fix is adding authoritative `runtimeSession` fences to snapshot/projection/change publication, bootstrap replay, and backend mesh/promote expected tokens; no native evidence yet. |
| 2026-09-11 | sketches intercept solids / candidate selection | Candidate enumeration is implemented and separately reviewed. | Chooser UI and full normal-selection behavior remain open; do not infer an interaction acceptance. |
| 2026-09-11 | inspector migration / E5 occlusion | Screenshot reproduced the old hard-coded 264 px CornerCluster/GridScaleChip inset overlaying the new 320 px Inspector. | Shared-inset correction is underway; responsive and native UI acceptance remain pending. |
| 2026-09-11 | feature-pattern atomicity/identity | C3 is extending the bounded chain from 0 through N instances. | Work is not accepted; support remains restricted pending adversarial review and broader adapters. |
| 2026-09-11 | inspector migration / E5 occlusion | Evidence correction: the preceding CornerCluster/GridScaleChip-over-Inspector screenshot was captured in the Playwright mock-browser lane, not native Computer Use. | Keep the browser-lane regression open and do not credit it as native evidence. The available last-run metadata has no reliable timestamp, so no `19-54-44` run ID is claimed. |
| 2026-09-11 | feature-pattern atomicity/identity | Main independently passed FeaturePattern core 9/9, selected CTest 3/3, and real-worker integration 2/2 covering four sources. | Bounded adapters only; the C3 0..N extension is still unaccepted. |
| 2026-09-11 | inspector migration / E5 occlusion | Store/component tests passed 44/44; Chromium measured nine layouts at three sizes × 280/320/420, plus collapsed 32, with 12 px cube/grid clearance. Mock screenshot `/tmp/onecad-layout-1024-420.png` still shows toolbar/header/cube crossing and model clipping. | ui_plan's measured-work-area fix is in progress. This is browser/mock evidence, not native validation. |
| 2026-09-11 | toolbar/tool grammar / accessibility window matrix | Approved golden-shell contract: `display: contents` measurement wrappers preserve SlotHost layout/order while measuring actual panel, toolbar and corner occupied rectangles; toolbar wraps rather than horizontally scrolling. | User-visible decision recorded; implementation and native acceptance remain pending. |
| 2026-09-11 | solver currentness / accessibility wording | Agent reports a fix for legacy `undefined === undefined` currentness with 38 tests. | Preliminary agent evidence only; additional accessibility/error-wording correction and independent main evidence remain pending. |
| 2026-09-11 | inspector migration / accessibility/window matrix | Test infrastructure now supplies an inert jsdom `ResizeObserver` only when absent; explicit lifecycle observer tests still install their own mock. Narrow StartScreen passed 20/20. | No production measurement or timeout change. Main four-target suite was previously 61 pass/19 fail: three StartScreen failures plus 12 missing-observer uncaught errors, with 16 remaining promotion failures assigned to native audit. Full rerun remains pending. |
| 2026-09-13 | feature-pattern atomicity/identity | Both `feature_pattern` CTest reds reproduced on a clean rebuild and root-caused with stderr probes: the i-2 Chamfer closure contains a fillet end-arc that OCCT history omits (only the blend FACE is `Generated`), so the ledger marks it Unknown and the straight-edge capability check refuses fail-closed. Astra `derive` (call 1) produced the ownership-completion rule with a certificate; probes PROBE5/6/7 measured fillet, chamfer and sequential-blend history exhaustively. | The refusal was correct against the ledger; the defect is blend-adapter ledger completeness. Fix in flight under `docs/design/astra/feature-pattern-producer-ownership.md`; no predicate loosened. Not a general pattern acceptance. |

## Append-only allocation and evidence update — 2026-09-11

Luna: simple fixes, test updates and docs. Terra: moderate UI. Sol: complex concurrency/geometry. Main Astra: review and acceptance only. Maximum three coding streams.

Main independent evidence: promoter + StartScreen six suites **240/240**; native receipt **6/6** including cap; currentness three suites **55/55**; chooser + ViewportRoot + Popover three suites **32/32** before browser work; bounded Revolve core **9/9** and real-worker **4/4**. Mounted layout covered 1024/1156/1440, Inspector 420, toolbar wrapping and corner clearance; `/tmp/onecad-mounted-layout-1024.png` is MOCK, not native.

The interrupted full-unit run remains **61 pass / 19 fail** and is not a stable full-gate or native claim. A26 tests and shared Add5 integration are agent-preliminary only. Chooser browser did not open at 430×400; diagnostic is pending, with no browser/native pass claimed.

## Continuation evidence — 2026-09-11 (append-only)

Main independently passed **7 files / 182 tests** (mock publication/import/promote, chips, HtmlOverlayDriver, ViewportRoot and ViewportEngine) before latest drag changes. Worker Release build/stage passed with OCCT deprecation warnings; `ctest -R feature_pattern` passed **3/3**; real-worker `feature_pattern_integration` passed **5/5**, including shared Add5 on the corrected enum/host ledger.

Chooser remains blocked/defective: publication current, installed mesh provenance undefined, exact candidate entry present but candidates filtered. Revision-fixture mismatch fixed, not proven root cause. Native app not launched; full gates not claimed. Latest drag **83 passed** and dev-demo **2 passed** are agent-preliminary only. Native event/restart review remains rejected pending ABA, queued restart, `getProjection` lifecycle/order, and coalescer corrections.

## Final hardening checkpoint — 2026-09-11 (append-only)

| Evidence | Result | Boundary |
|---|---:|---|
| Promoter + StartScreen | 240/240 (6 suites) | Main-independent frontend evidence |
| Native receipt | 6/6, cap included | Receipt only; app not relaunched |
| Currentness | 55/55 (3 suites) | Main-independent frontend evidence |
| Chooser browser | Chromium + WebKit 2/2, retries 0 | Fresh server; prior stale-server failure remains history |
| Frontend/UI bounded runs | 230 tests / 6 files; 129 tests / 3 UI suites | Main-independent; no full-gate claim |
| Worker/runtime checks | feature_pattern 3/3; real-worker 6/6; core 10/10; scheduler 1/1; queued-runtime 1/1; restart-owner 1/1 | Wrong-filter queued-runtime 0 tests excluded |

Mounted layout covers 1024/1156/1440, Inspector 420, wrapped toolbars and corner clearance; `/tmp/onecad-mounted-layout-1024.png` is MOCK, not native. Preserve RED **61 pass / 19 fail**, A26/shared Add5 preliminary labels, initial mock missing provenance, separate Cut/Rust evidence, and open full-suite/clippy, stress, persistence GUI, remaining adapters/CTest targets and D-camera gates.

## Final main gate evidence — 2026-09-11 (append-only)

| Gate | Result | Boundary |
|---|---:|---|
| Full CTest | 196/196 | [durable log](full-ctest.log); source `/tmp/onecad-ux-full-ctest.log` |
| Frozen-source full Vitest | 332 files; 5880 passed, 78 skipped; 5958 total; 31.36s | [durable log](full-vitest-final.log); source `/tmp/onecad-ux-full-vitest-final.log`; nine prior stale-contract/async failures fixed tests-only |
| Main TypeScript | passed | `npx tsc` |
| Final clippy | running at checkpoint | Do not claim |

Native GUI was not relaunched; full browser and full Rust workspace remain owed. Latest subscriber-failure isolation is included in full unit. General pattern breadth/UI, typed-target and D-camera remain open. Allocation: Luna simple tasks/docs, Sol complex work, Astra final review. No percentage increase claimed.

Main passed cargo fmt check and workspace clippy with `-D warnings` (49.21s; [durable log](final-clippy.log); source `/tmp/onecad-ux-final-clippy.log`). Full worker-required cargo workspace tests are still running; result pending.

Worker-required full workspace recorded **475 pass / 1 fail** in the first `lib` target; see [durable RED log](final-cargo-workspace-red.log). Focused DTO unit suite passed **22/22** after moving the runtime-session assertion to the projection DTO test.

Final worker-required Rust rerun exited 0: **1646 passed / 0 failed / 0 ignored / 0 filtered** across exactly 100 result lines, with no missing-worker skips ([durable log](final-cargo-workspace-rerun.log); source `/tmp/onecad-ux-final-cargo-workspace-rerun.log`). Earlier RED is preserved. Post-DTO fmt/clippy rerun is not newly claimed; full browser/native and remaining implementation gates remain open.

Post-DTO fmt check and warning-free workspace clippy passed in **44.48s** ([durable log](final-clippy-after-dto.log); source `/tmp/onecad-ux-final-clippy-after-dto.log`). Final build remains in progress; result pending.

Final `bun run build` passed (tsc + Vite, 2.03s) with the existing >500kB chunk warning ([durable log](final-build.log); source `/tmp/onecad-ux-final-build.log`). Native remained closed.

## CURRENT RESUMPTION CHECKPOINT — 2026-09-12 (append-only)

Package mapping is authoritative for resumed work: 2C shared presentation/controller contract; 3B candidate UI, annotations, framing/section/measurement/history; 3C FeaturePattern contracts/core/worker. Main independently verified measurement UI **2 files / 23 tests passed** and worker FeaturePattern **1/1**. A later required-worker Cargo run is RED: `shared_host::extrude_add_fillet_pattern_keeps_one_host_through_edit_and_reopen` **0 passed / 1 failed / 6 filtered**, source Fillet `NeedsRepair`, ambiguous scoring 4, candidate count 5; Sol/native_fix owns diagnosis. Typed-context **198** is preliminary only. Native app and all remaining full acceptance gates stay open; no acceptance claim is added here.

## MAIN ENGINEERING REVIEW CHECKPOINT — 2026-09-12 (append-only)

Main independently recorded UI + camera **11 files / 203 tests passed** with `npx tsc`, Stage B camera-engine **4 files / 101 tests passed** with `npx tsc` (nonexistent `PreviewMesh.test` excluded), required-worker `feature_pattern_integration` **7 passed / 0 failed / 0 ignored / 0 filtered**, selected CTest **13/13**, and full topology rebind **16 passed / 0 failed / 0 ignored / 0 filtered**. Automated engineering evidence only; native/manual acceptance remains open.

Critical open finding: `vfm5_teleport_on_the_ordinary_edit_lane_is_the_accepted_residual` passes with an explicit `boundDecoy=true` assertion. This is characterization, not safety acceptance. General patterns remain bounded; authoritative producer ledger/adapters are in progress, with no producer-to-host fallback. Tree hover/Reveal and typed-target checks are automated only.

Measurement currentness is newly open: mass cache is keyed by `bodyId`, and main code inspection identifies same-body regeneration/new-document async stale-result risk; no native reproduction. Current app is stale/closed, native was not launched, and full integrated gates were not rerun. No percentage or all-fixed claim.

## SUBSEQUENT MAIN CHECKPOINT — 2026-09-12 (append-only)

- Reference ambiguity: main topology **16/16** after resolver v6; stale-anchor teleport now refuses (`boundDecoy=false`, zero removed volume). Native recovery remains owed.
- Pattern provenance: main FeaturePattern **7/7**, selected CTest **5/5**. Mixed-chain expansion remains in progress; no general-pattern completion claim.
- Camera/entry: main **13 files / 184 tests**; corrected test-only typing then main TypeScript plus plane-pick **9/9**. Native physical/window matrix remains open.
- Fit Preview/shared UI: main **3 files / 25 tests**, TypeScript passed. Native interaction remains open.
- Runtime readiness: main **7/7**, then actual same-file reopen **1/1** after fixture correction. Atomic session check/enqueue prevents old readiness requests superseding new work; no-op callback tests were rejected as evidence.
- Open engineering: scoped measurements, annotation layout/controls, general patterns, planar-face/datum framing, independent responsiveness monitoring. Full integrated/browser/native gates still required.
