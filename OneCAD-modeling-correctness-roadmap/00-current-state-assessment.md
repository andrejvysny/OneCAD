# OneCAD Modeling Correctness Assessment

Repository: `andrejvysny/OneCAD`  
Reviewed commit: `1c11d4958aeadea14dd8431ba78c41f14be12142`  
Review date: 2026-08-10  
Method: connected GitHub inspection plus a read-only shallow clone; static source and test analysis only.

## Executive conclusion

OneCAD has a stronger correctness spine than most early CAD systems: deterministic worker replay, a typed Rust document model, snapshot-fenced promotion, atomic prepare/accept publication, a conservative topological-naming ladder, explicit `NeedsRepair`, exact operation-specific tests for several features, and a real OCCT robustness harness.

The central weakness is not generally poor engineering. It is uneven vertical proof and several places where older parity behavior, newer UX, and the worker's publication rules no longer agree.

The highest-priority findings are:

1. Standalone Boolean and Revolve Boolean can publish an empty but non-null BRep as a modified body. `checked_boolean` accepts any non-null BRep-valid result, while `publish_boolean_result` treats zero solids as the single-body branch. Extrude and Hole add their own empty-result guards; standalone Boolean and Revolve do not. Evidence: `worker/src/ops/OpCommon.cpp:216-230,283-300`; `worker/src/ops/BooleanOp.cpp:43-60`; `worker/src/ops/RevolveOp.cpp:214-224`; compare the explicit guards in `worker/src/ops/ExtrudeOp.cpp:478-485` and `worker/src/ops/HoleOp.cpp:287-290`.
2. Partial circular-pattern preview and committed geometry disagree. The frontend distributes a partial sweep across `count - 1` gaps, while the worker and protocol always divide by `count`. A 180° three-instance ghost shows 0°, 90°, 180°; the worker commits 0°, 60°, 120°. Evidence: `src/tools/preview/patternPreview.ts:122-134`; `src/tools/preview/patternPreview.test.ts:74-81`; `worker/src/ops/PatternOp.cpp:166-173`; `protocol/SCHEMA.md:1291-1305`.
3. Open Project bypasses the dirty-document guard. Close and Quit use `requestClose`; Open calls `openDialogAndOpen`, resets document-scoped UI, and replaces the document directly. Evidence: `src/features/shell/fileActions.ts:187-204`; `src/stores/appStore.ts:204-210,296-329`.
4. Multiple modeling and history workflows treat a resolved regen failure as success. `tauriClient` returns `ApplyOperationResult.errorMessage`; several direct and re-edit paths only catch rejected promises. Pattern/Mirror is one confirmed example: it applies the projection, selects the source, tears down, and shows a success hint without inspecting `errorMessage`. Evidence: `src/ipc/tauriClient.ts:790-826`; `src/tools/modelTools/ModelToolController.ts:4784-4806`; representative history/repair paths in `src/features/inspector/historyActions.ts:81-118,180-219`.
5. A refused promotion for Extrude ToFace or Hole is deliberately degraded to a body-qualified anchor-only ref rather than refused. If the refusal was caused by a stale snapshot, the subsequent preview resolves the old world anchor against the new topology. Evidence: `src/tools/modelTools/ModelToolController.ts:1681-1709,4291-4323`; `worker/src/ops/ExtrudeOp.cpp:185-237`; `worker/src/ops/HoleOp.cpp:100-125`.
6. `PrepareOffsetFace` lacks the post-response snapshot/head fence implemented by `PrepareEdgeOp`. An undo or edit during the round trip can allow an old closure to be adopted against a newer frontend head. Evidence: `src/ipc/tauriClient.ts:1525-1570`.
7. Fillet and Chamfer do not validate that every typed edge ref belongs to the target body. The first edge chooses the target; a foreign-body edge misses the identity rung, then falls through to descriptor matching against the target body because `LadderRef` discards `primary.bodyId`. A congruent edge on the target can therefore be selected. Evidence: `worker/src/ops/FilletChamferOp.cpp:42-52,102-174,359-373`; `worker/src/elementmap/Ladder.cpp:162-166,333-367`; Rust lockstep validation checks element ids but not common body ownership in `src-tauri/crates/onecad-core/src/edit/session.rs:1655-1703`.
8. Intersected analytic sketch curves are converted to polygon edges in the final BRep. Loop detection tessellates arcs, circles, and ellipses; split fragments receive synthetic ids; `FaceBuilder` cannot resolve those ids to original analytic entities and uses straight segments. Existing overlap tests only assert broad area ranges. Evidence: `worker/src/loop/LoopDetector.cpp:827-883,913-995`; `worker/src/loop/FaceBuilder.cpp:390-475`; `worker/tests/test_region_table.cpp:251-315`.
9. Curved-wall Draft is a high-confidence candidate defect requiring a red probe. `apply_draft` considers planar faces only and existing draft coverage uses a square prism. A circular profile can therefore add no eligible wall; static inspection does not prove whether OCCT then reports success with unchanged geometry or refuses. Evidence: `worker/src/ops/ExtrudeOp.cpp:240-275`; `worker/tests/test_wp6_extrude.cpp:204-220`.
10. Pattern and Mirror publication semantics are under-specified relative to downstream operations. Linear/Circular Pattern can publish a compound or disjoint fuse under one BodyId with no shape audit; Mirror can publish a disjoint fuse with no validation. Single-solid modifiers then refuse or behave inconsistently. Evidence: `worker/src/ops/PatternOp.cpp:39-112`; `worker/src/ops/MirrorOp.cpp:36-100`; `protocol/SCHEMA.md:1283-1305`.

## Corrections to the earlier project snapshot

The live repository is materially ahead of the earlier `05c9712` analysis.

- Playwright is now gated in CI on macOS Chromium and WebKit. It is not globally omitted. See `.github/workflows/ci.yml:391-457`.
- Real CTest and kernelbench lanes now run on trusted self-hosted Linux, with macOS retained as the shipping gate. See `.github/workflows/ci.yml:45-174,176-389`.
- Current static inventories are 53 C++ `test_*.cpp` files, 46 Rust integration-test files, 241 Vitest files, and 70 Playwright specs.
- TRACK A is complete. Boolean picking was root-caused and fixed. See `TODO.md:3-39,568-583`.
- Fresh element promotion is atomically bound into the worker partition before publication. See `Session::bind_element_ids`, `worker/src/session/Session.cpp:326-360`.
- The VF-M5 checkpoint-fallback path is closed with `checkpointFallbackReplay`; the ordinary-edit teleport residual remains accepted and must not be accidentally changed. See `TODO.md:585-599`; `protocol/SCHEMA.md:2475-2531`.
- Kernelbench is still fillet-only and CI still runs T0 only. M3 and M4 remain unstarted. See `TODO.md:674-684`; `bench/robustness/README.md:89-116`.

## Supported operation surface

Worker-dispatched operations at this commit:

- Sketch
- Extrude
- Revolve
- Fillet
- Chamfer
- Shell
- Boolean
- LinearPattern
- CircularPattern
- MirrorBody
- ImportStep
- TransformBody
- Hole
- OffsetFace

`Loft` and `Sweep` remain typed but unsupported and are explicitly excluded from this roadmap. Dispatch: `worker/src/session/PlanExecutor.cpp:256-291`. Typed source: `src-tauri/crates/onecad-core/src/document/record.rs:292-314`.

Datum geometry is document-side, not a C++ modeling operation. It is correctly assessed through Rust, frontend, and worker-backed use by datum-hosted sketches rather than through a nonexistent DatumOp.

## Operation-by-operation assessment

| Operation | Current strength | Main correctness risk | Evidence level |
|---|---|---|---|
| Sketch/profile | Broad solver, region, construction, ellipse, drag, and e2e coverage | Intersected curves become faceted BRep boundaries; gap repair can heal up to 0.1 mm without a product-level diagnostic | Strong tests, important semantic defect |
| Extrude | All end conditions, two directions, draft, Boolean modes, typed ToFace, real-worker tests | Anchor-only stale ToFace fallback; curved-wall Draft outcome is unproven when no planar side is eligible; `ToNext` samples only profile vertices plus centroid | Strong but incomplete mode and geometry proofs |
| Revolve | Exact NewBody and Boolean-mode C++ coverage; Rust preview/commit tests | NewBody result has no BRep/solid/volume audit; body-edge axis is persisted as ElementId but worker resolves it as TopoKey; feature Intersect absent from UI | Good primary path, dormant contract gap |
| Fillet | Deepest production validation, semantic checks, diagnostics, kernelbench, identity tests | Mixed-body edge refs can fall through onto target-body descriptors; production cancellation is pre-build only | Strongest operation with one ownership hole |
| Chamfer | Exact equal/two-distance geometry and deterministic reference-face rule | Shallower validation than Fillet; same mixed-body ref path; no kernelbench | Good nominal coverage, weaker robustness |
| Shell | Typed face lockstep and same-body validation in Rust; real-worker preview and repair tests | Positive thickness only; BRepCheck-only publication; no self-interference/single-solid/volume gate; cross-body selection reaches UI before rejection | Good identity discipline, validation gap |
| Boolean | Central OCCT BooleanOperation, deterministic options, cancellation, split ranking | No direct Intersect tests; empty result publication hole; tool always consumed; no explicit no-overlap policy | Largest exposed operation gap |
| LinearPattern | Kernel and Rust exact-volume tests for fuse/compound | No e2e; one-body multi-solid result; sequential unbounded fuse; no history; UI always fuses | Thin vertical proof |
| CircularPattern | Kernel/Rust full-circle test; L1 ghost tests | Partial-sweep ghost/commit mismatch; no e2e; UI always fuses; world-origin axes only | Confirmed product defect |
| MirrorBody | C++ fuse/no-fuse tests | No e2e; UI always no-fuse and world planes; disjoint fused result not audited; resolved failure reported as success | Thin vertical proof |
| TransformBody | Strong atomic multi-target and identity coverage; exact T∘R | Malformed worker payloads can default components; no post-transform shape audit; direct UI result classifier can report false success | Strong core, UX truth gap |
| ImportStep | Rich STEP/BREP/XBF, color, persistence and compatibility coverage | Invalid solids are advisory by policy; post-scale audit absent; content hash trusted at worker boundary; exact-tie order remains traversal-dependent | Strong but policy-heavy |
| Hole | Exact analytic volumes, typed host face, point fence, preview/commit and persistence | Stale promotion degrades to anchor-only; split host remains one multi-solid body despite single-body mental model; BRepCheck-only publication | Strong nominal path, boundary gaps |
| OffsetFace | Best direct-edit postconditions: ownership, closure, one solid, volume, self-interference, semantic movement checks, custom history adapter | Frontend prepare response can be adopted after head movement; Rust validator does not require each primary body to equal target; no kernelbench | Strong kernel, snapshot adoption gap |

## Validation policy assessment

### What exists

`audit_shape()` checks:

- null shape,
- `BRepCheck_Analyzer`,
- exact single-solid count,
- finite positive volume,
- self-interference via `BRepAlgoAPI_Check`,
- per-kind maximum tolerances as evidence.

Evidence: `worker/src/kernel/validation/ShapeAudit.cpp:23-78`.

### Important correction

Maximum tolerances are recorded but not rejected. `ShapeAuditResult::publishable()` does not inspect the tolerance values. This matches the protocol's statement that tolerance growth is measured but not yet a production rejection policy. See `ShapeAudit.cpp:23-26`; `protocol/SCHEMA.md:1127-1131`.

### Current asymmetry

- Fillet uses full `audit_shape` for input and output plus semantic validation.
- OffsetFace independently implements BRep validity, exact solid count, volume, self-interference, and operation-specific movement semantics.
- Boolean/Extrude/Hole/Shell/Chamfer use partial inline checks.
- Pattern/Mirror/Transform/Revolve NewBody have little or no common post-publication validation.
- Import intentionally treats invalid solids as warnings.

The correct next step is not to call the heaviest audit blindly after every operation. It is to define one policy-driven publication validator with per-operation shape-class and risk tiers.

## Identity and regeneration assessment

### Sound foundations

- Rust remains the hash and persistent-id authority.
- Worker NewBody ids are deterministic and adopted under D1.
- `workerEpoch + expectedBaseHash` is the correct worker fence; document revision remains advisory.
- Candidate execution snapshots and rolls back all body/partition changes on failure or `NeedsRepair`. See `worker/src/session/PlanExecutor.cpp:294-392`.
- Fresh promoted ids are snapshot-fenced and atomically bound. See `worker/src/session/Session.cpp:326-360`.
- The descriptor ladder uses min-cost assignment, score ≥0.85, margin ≥0.10, and the resolver-v2 edit-scoped tie veto. See `worker/src/elementmap/Ladder.cpp:162-330`; `protocol/SCHEMA.md:2421-2531`.

### Gaps

- `ElementMapPartition::apply_history()` documents all builder history, but implementation consults `IsDeleted()` and `Modified()` only; it never calls `Generated()`. See `worker/src/elementmap/ElementMapPartition.cpp:293-423`.
- The body ownership carried by a semantic ref is not retained in `LadderRef`. Operation-specific validators must therefore enforce target-body consistency before fallback scoring.
- The accepted ordinary-edit teleport residual is real and explicitly outside this roadmap unless reopened by decision.
- Repair candidates are cached only by `refId`, while the candidate TopoKey is snapshot-scoped and carries no snapshot id. See `src/features/repair/RepairPanel.tsx:71-101`; `src/ipc/types.ts:500-535`.

## Coverage assessment

### Strong areas

- Fillet CTest, Rust and kernelbench.
- TransformBody real-worker tests.
- Hole analytic and persistence tests.
- OffsetFace kernel semantic checks.
- Sketch solver and common authoring workflows.
- macOS full Rust workspace with a staged real worker.
- Linux worker CTest and T0 kernelbench on trusted events.

### Highest-value missing tests

1. Standalone Boolean Intersect at C++, Rust, frontend and Playwright layers.
2. Empty-result Cut/Intersect policy and lifecycle events.
3. Partial circular-pattern preview equals committed transforms.
4. Pattern and body-mirror Playwright workflows.
5. Cross-body Fillet/Chamfer refs at full executor and Rust authoring layers.
6. Curved-overlap profile BRep exactness, not broad polygon-area bounds.
7. Circular-profile draft refusal or real taper proof.
8. Stale ToFace/Hole promotion refusal.
9. Stale OffsetFace prepare response.
10. Resolved `errorMessage` handling across every commit, re-edit, history and repair action.
11. Repair candidate invalidation across revisions.
12. Pattern/Mirror/Hole multi-solid publication semantics.

The corpus is a valuable frozen oracle but is not currently discovered as a complete executable suite. `corpus/README.md:92-108` describes the intended consumption path; CI exercises selected recordings rather than compiling every case.

## CI assessment

Current CI is materially improved and should be preserved:

- Ubuntu frontend build and Vitest.
- Trusted self-hosted Linux worker CTest and determinism.
- Trusted self-hosted Linux T0 kernelbench digest and semantic gates.
- macOS worker, full Rust workspace, T0 semantics, both browser projects, packaging linkage, and 7.9.3→8.0.1 persistence.

Open CI decisions:

- Retries remain zero; decide explicitly before making checks required.
- `TODO.md:39` records that no checks were required at this commit; verify the live GitHub ruleset/settings state before changing branch protection.
- Linux Chromium has a body-pick difference and remains ungated.
- No Windows lane exists.
- T0 is the only kernelbench suite in CI; m1 is not gated.
- No automated operation/mode coverage manifest exists.

Evidence: `.github/workflows/ci.yml`; `TODO.md:31-39`.

## Severity model used by the roadmap

- P0: can lose user work, publish empty/invalid geometry as success, or display/commit materially different geometry.
- P1: can silently bind the wrong topological element, silently ignore an authored parameter, or publish topology incompatible with downstream operations.
- P2: insufficient evidence, inconsistent diagnostics, performance/cancellation exposure, or dormant contract mismatch.
- P3: documentation or maintainability debt with no direct current correctness impact.

## Recommended strategic direction

For the next 3–6 months:

1. Convert the newly identified P0/P1 findings into red-first tests.
2. Fix publication and UI truth before expanding operation breadth.
3. Enforce body ownership and snapshot consistency across every semantic-ref path.
4. Replace faceted intersection-region BReps with exact analytic curve fragments.
5. Introduce a policy-driven publication validator and explicit multi-solid semantics.
6. Expand kernelbench to Boolean and Chamfer while completing the generic M3/M4 metamorph and validator groundwork.
7. Turn CI green into a required, cross-platform release claim and make the corpus executable.

Do not start Loft, Sweep, assemblies, FEM, TechDraw, CAM, addon kernel extensions, or advanced production Fillet rescue strategies during these phases.
