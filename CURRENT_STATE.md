## KERNELBENCH M4 — RECIPE-AGNOSTIC VALIDATORS (2026-08-13) — GATE PASSED, UNCOMMITTED

Roadmap Phase 5 WP5.2. `supportTangency`, `crossSectionProfile`, `manifold`,
`noSelfIntersection`, `microTopology` and `toleranceGrowth` are implemented and
REQUIRED on every m1 case, curved support pairs included. They previously
returned `notApplicable`, which a required check must fail.

The enabling change is that the blend is now taken from the fillet builder's own
history — `AdapterResult.blend_faces` (`Generated`) and `.support_faces`
(`Modified` of the selected edge's supports) — instead of being recognised by
surface type. "The cylinder in the output is the fillet" is false the moment a
SUPPORT is a cylinder, which is the assumption that keeps `cylindricalRadius` and
`g1BoundaryTangency` confined to all-planar pairs.

`crossSectionProfile` compares `1/|k|` for the larger principal curvature against
the requested radius, exact on plane, cylinder and cone because a constant-radius
blend is a canal surface whose circular sections are lines of curvature — worst
deviation across the matrix 2.7e-14 mm. Allowances carry a conditioning term
proportional to coordinate magnitude, because `farOriginTranslation` rebuilds the
model 1.7e6 mm out where double precision costs six orders of magnitude; without
it the probe reads arithmetic as a defect.

- **Changed:** `worker/src/benchmark/{BlendEvidence.h,BlendEvidence.cpp}` (new) ·
  `worker/src/benchmark/{Types.h,FilletRun.cpp,SemanticValidation.cpp,SemanticValidation.h,Execution.cpp}` ·
  `worker/tools/kernelbench-runner/{CMakeLists.txt,validator_fixtures.cpp}` (new) ·
  `src-tauri/crates/onecad-kernelbench/src/suite_v2.rs` ·
  `bench/robustness/{README.md,baselines/digests.json}`.
- **Gates:** ctest **116/116** · fmt · clippy · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1076 / 0** ·
  T0 **136/136** with digests AND semantics byte-unchanged · m1 **336 records**, 330 pass + 6
  characterization, metamorph 288/0, `gatingFailures: 0`. m1 digests re-recorded (case documents
  gained the six validators and the quality ceilings); m1 semantics unchanged, so no verdict moved.
- **Next:** Phase 5 WP5.3, the Boolean foundation campaign.

## KERNELBENCH M3.5 — NON-ISOMETRIC METAMORPH POLICY (2026-08-13) — COMMITTED `1d0fbb7`

Roadmap Phase 5 WP5.1 residual, closed. The metamorph comparison now carries a
RELATION per variant instead of demanding equality from all of them: rigid
variants keep equivalence, `scaled` is compared as a SIMILARITY (`k³·V`, `k²·A`),
and `parameterEpsilon` is compared for CONTINUITY (bounded, correctly signed
response; shape tolerance widened to `8·radius·|δ|`). The relation is a pure
function of the variant name, so the frozen result-v1 `metamorphEvidence` block
is untouched. Every coefficient is measured over m1 rather than guessed — the
numbers and their margins are in TODO.md § M3.5.

The two re-enabled variants immediately found a real defect: `validate_output`
measured the blend against the case's DECLARED radius while `Execution.cpp`
filleted with the scaled/nudged EFFECTIVE one, so `cylindricalRadius` and
`g1BoundaryTangency` reported a false red on correct results. The effective
radius is now threaded through; red-first proof is the new ctest
`kernelbench_metamorph` (`worker/tools/kernelbench-runner/metamorph_fixtures.cpp`),
verified to fail against the old call site and pass against the new one.

- **Changed:** `worker/src/benchmark/{Execution.cpp,SemanticValidation.cpp,SemanticValidation.h}` ·
  `worker/tools/kernelbench-runner/{CMakeLists.txt,metamorph_fixtures.cpp}` (new) ·
  `src-tauri/crates/onecad-kernelbench/src/{campaign.rs,prepared.rs,suite_v2.rs}` ·
  `bench/robustness/{README.md,baselines/digests.json,baselines/semantics.json}`.
- **Gates:** ctest **115/115** · `cargo fmt --all --check` · workspace `clippy -D warnings` ·
  `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1076 / 0** · kernelbench lib **61** ·
  T0 **136/136** unchanged and `compare` OK · m1 **336 records**, 330 pass + 6 characterization,
  metamorph 288/0, replay 336 stable, `gatingFailures: 0`, p50 11.8 ms / p95 68.9 ms.
- **Baselines:** m1 digests + semantics re-recorded (`darwin-arm64`, 336 rows). Every m1
  `inputDigest` moved because `input_digest` hashes the canonical case document, which now
  declares eight metamorphs instead of six. T0 rows are byte-identical.
- **Not implicated:** no frontend file changed, so vitest and Playwright were not re-run.
- **Still open:** `m1` has `darwin-arm64` rows only — record `linux-x64` on the self-hosted
  host (Phase 5 WP5.6). Next in roadmap order is Phase 5 WP5.2, the M4 recipe-agnostic
  validators.

## MODELING CORRECTNESS P2-P4 — LIVE DELTA (2026-08-12)

Reviewed current uncommitted worktree, OCCT 8.0.1 fingerprint
`0a6a1dce34181289`. Implemented: P2 exact analytic CurveFragments/BReps, v2
region identity, bounded refinement, and cancellation; P3 Tier A/B publication
evidence, strict worker readers, Pattern/Transform limits, Pattern V2 body
lineage, machine-readable operation contracts, Transform Tier A validator, and
explicit ImportStep/Transform/UI-mode disposition policy. V2 non-fused Pattern
preserves source instance zero and mints only `body_<opId>:<k>` transformed
children; V2 fused Pattern modifies source in place. Absent version remains V1.
P4 coverage manifest/classifier is implemented and CI-checked. P4 Boolean Intersect
vertical is complete: C++ fixtures, Rust real-worker tests (preview/commit parity,
disjoint refusal, save/reopen, undo restoration), frontend/Playwright Intersect
coverage with chip testids, and contract updated to supported/exposed.
Pattern/Mirror Playwright flows landed (`e2e/mirror-body.spec.ts`,
`e2e/linear-pattern.spec.ts`, `e2e/circular-pattern.spec.ts`). Critical mode
closure tests landed (`src/features/toolbar/ModelToolChips.test.tsx`) and the
real-worker corpus executor landed (`src-tauri/tests/corpus_executor.rs`),
running `a_sketch_extrude_blind` end-to-end and recording explicit unsupported
reasons for the remaining corpus cases. P4 is now complete.
Current gates: CTest 114/114; worker Release build; real-worker Rust workspace;
fmt/clippy; TypeScript; Bun build; Vitest 241 files / 4116 tests; targeted
Playwright boolean/pattern/mirror specs 12/12 (Chromium + WebKit, retries 0).
Full Playwright run was aborted after unrelated pre-existing flakes in
`unsaved-guard` and `view-ux`; not caused by this wave.
fmt/clippy; TypeScript; Bun build; Vitest 241 files / 4116 tests; Playwright
Chromium 196/196 and WebKit 196/196 with retries 0; kernelbench T0 both backends
unchanged (136 records, 0 gating failures, replay stable); contracts/coverage
verifiers pass. Manual Tauri smoke remains open. See
`OneCAD-modeling-correctness-roadmap/04-live-implementation-delta.md`.

## MODULAR-PLATFORM UI (2026-08-09, design turn 2 via `claude_design` MCP) — FE GATE PASSED

The first DELIBERATE user-visible change since the platform refactor. Implements the design project's turn 2 ("Modular platform — workspaces, extensions, palette, missing add-ons", options 2a-2d) on the existing Platform, plus two follow-ups the user asked for mid-wave: one title-bar button geometry, and document rename moved out of the title bar into File.

**THE RULE THAT SHAPED EVERY SCREEN: bind it, or say it is not there.** Where a real seam existed the UI reads it — the workspace menu is a projection of `platform.workspaces`, the palette is a projection of `commands` + `tools` + `workspaces` with no opt-in and no palette registry of its own, and the missing-extension banner / details dialog / "Unavailable data" explorer section are all the real `listDocumentModules()` diff against `platform.moduleIds()` (ADR-0005 made visible at last). Where no seam exists the screen says so: Extensions ▸ Browse and ▸ Updates render "no registry configured" rather than a mock catalog, because dead Install buttons teach users the feature is broken and fake listings outlive the mock; enable/disable and uninstall are ABSENT rather than inert; Simulation/Drawing/Visualization are real registered arrangements that switch modeling's tools OFF and name what is missing, instead of offering Extrude under a "Drawing" label.

WORKSPACES BECAME RUNTIME (the piece P2.5 deferred). `SlotHost` gained a `filter` predicate and `modules/shell/workspaceLayout.ts` resolves it — the platform still never learns what a workspace is, because `src/platform/**` may not import an application store. The rule is CONSERVATIVE: a panel is hidden only on an explicit `visible:false` placement or a user override. "Unlisted ⇒ hidden" reads tidier and would silently swallow every tool overlay the moment a tool grew a chip.

ALSO REAL: `ViewportEngine.setLayerVisible()` toggles `bodiesRoot`/`sketchRoot`/`contributionsRoot` for the new bottom-left Layers menu (grid reuses the existing `viewportStore` flag — one piece of state, two entry points); collapsible explorer sections with `TreeNode.meta`/`problem` and `TreeSection.defaultCollapsed`/`emptyNote` (additive platform contract); a Customize-workspace sheet whose panels, tool groups and layers are all DERIVED from the registries; `TitleBarButton`, one 28px control replacing three hand-rolled geometries sitting in a row.

STILL UI-ONLY, AND FLAGGED: the background-tasks chip has a real `begin/setProgress/end` API and draws an indeterminate bar when `progress` is undefined — nothing calls it, because regen is request/response over OCW1 with no progress frames. Rename (File ▸ Rename…) writes `DocumentState.displayTitle`, a session override kept OUT of `title` because `title` is backend-authoritative and every projection replaces it; there is no `RenameDocument` in the protocol, so the dialog says the file on disk is unchanged instead of implying a save. Both are next-tranche backend work.

SEAM WORTH WATCHING: `CommandPalette` imports `useModelingToolContext` — shell UI reaching one specific module, because evaluating `canExecute` needs a real selection context and no neutral selection SERVICE exists. Precedent is `ViewportRoot` → `datumViewport`; the fix is a module-published context service.

`src/test/contracts/shellContract.ts` was AMENDED, not worked around: six new shell contributions, nothing existing moved, and the decision is recorded in TODO.md as the contracts README requires.

GATE: `bunx tsc --noEmit` clean · `bun run build` green · vitest **236 files / 3986 tests** (from 227/3919) · hex-token grep 0 · Playwright **387 passed / 3 failed**. Rust/ctest untouched by this wave.
The three e2e failures are NOT this work, and that was checked rather than assumed: `sketch-reattach` (chromium) passes 3/3 on an isolated re-run — a load flake; both webkit `boolean-preview` failures reproduce DETERMINISTICALLY in a clean worktree at HEAD `352ddd1`, i.e. before any of this. That is the same pre-existing failure CURRENT_STATE already bisected to before the Platform refactor.
STILL OWED: manual Mac smoke — the workspace filter and the new `shell.overlay` region are layout changes no jsdom test can prove.

## P2.5 — GENERIC EXTENSION SEMANTICS + SDK BOUNDARY (2026-08-09, plan `act-as-senior-software-encapsulated-balloon.md`) — GATE PASSED

Finished the seams that were generic at the CONTRACT and modeling-specific at the RUNTIME, then froze a public SDK over the result. Committed as `c9779a3` (WP0) · `4b8ec0d` (WP1) · `eb59b6d` (WP2) · `7dee177` (WP3) · `bbe1ceb` (WP4) · `2006f6d` (WP5-7). NOT pushed.

WHY BEFORE P3, in one sentence each: a registered tool was silently dropped by the toolbar unless modeling's reverse map knew its id; `defaultShortcut` was write-only, so no contributed chord could ever fire; `TreeNode.id` was resolved globally across providers, so two providers minting the same string cross-wired the context menu; and `ModuleDefinition.deactivate` was declared and called from nowhere. Freezing an SDK over any of those would have frozen the wrong shape.

**VF-M5 CLOSED (WP0).** The gate's discriminator (`job.partition.size() == 0`) cannot express the hazard it guards: the RegenPlanner emits full-replay-from-0 plans for EVERY regen and `Session::fence_and_clone` clones an empty base for those (D5), so it is always true and the gate degenerated to `editedFrom.is_some()` — i.e. to any user edit, which is why the flagship fillet lane came back `NeedsRepair`. **SUPERSEDED 2026-08-09 — the sentence that stood here, "the real hazard needs a RESTORED basis, which V1 cannot produce", was FALSE.** Checkpoints are plumbed end to end and the executor's F12 fallback produces exactly that basis. Resolution (a) (`from_zero_replay = false`) was the right immediate call and is what shipped, but it left a real residual, now CLOSED by the SCHEMA §7.2 `checkpointFallbackReplay` field — see TODO.md § VF-M5 RESIDUAL for the red-first proof and the gate.

The four Playwright failures were STALE SPECS, not an open UX question — both removals were already pinned elsewhere (`TitleBar.test.tsx` asserts the Appearance toggle is absent; the unified start-screen `Import…` is recorded in TODO § Wave 4). Specs moved to the shipped entry points; `mockClient.importFileDialog()` gained `?mockimport=step` so the extension router's STEP half stays reachable from a browser lane.

ARCHITECTURE, in the order it landed: a platform `ToolHost` owns the active tool id and the activate/deactivate handshake while modules stay authoritative and REPORT (one truth, mirrored — ADR-0010) · shortcuts resolve modifier chords → the frozen module ruleset → the registry, so a contribution reaches the keyboard and cannot shadow a built-in, with conflicts resolved by scope→priority→built-in and an unbreakable tie firing NOTHING (ADR-0011) · tree rows are addressed by (providerId, nodeId), every provider announces its own changes, and row actions are command-backed with a declarative `confirm` (ADR-0012) · `deactivate` runs, in reverse order, never for a failed activation or a child scope, and a throw does not abort disposal · `@onecad/sdk` hands addons an `ExtensionContext` narrowed BY CONSTRUCTION — no `platform`, no `createScope`, no `CadClient`, `document.state` pre-bound (ADR-0013).

THE PROOF IS A FIXTURE, NOT A CLAIM: `src/addons/reference/` imports `@onecad/sdk` and `react` and nothing else, contributes one of every open kind over its OWN domain type, and is asserted through the SHIPPED toolbar, tree, shortcut lane and viewport host. It is deliberately NOT registered in `bootstrap.ts` — shipping a fake "Widgets" panel into the product UI to prove an architectural point is a worse trade than proving it in tests; it becomes the addon loader's first package when the loader lands.

REGRESSION CAUGHT BY THE E2E LANE (WP1): `mirror` is a member of BOTH tool unions and means different things, so routing every tool through the model applicability matrix disabled the SKETCH Mirror tool whenever no body was selected. Scope-gated, with a vitest case pinning it. This is the reason WP0 insisted on a green baseline first.

GATE: `bunx tsc --noEmit` clean · vitest **227 files / 3897 tests** (from 224/3837) · `git diff src/test/contracts/` byte-identical · hex gate 0 · production `bun run build` green · `cargo fmt --all --check` + workspace `clippy -D warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1045 / 0** across 72 targets against a worker built from HEAD · ctest **107/107** · Playwright chromium **195/195**, webkit 191/195 under load → **15/16 on an unloaded re-run**, the one survivor being the pre-existing `boolean-preview` flake.
**`boolean-preview` is PRE-EXISTING and not this work** — bisected in a throwaway worktree: the same test fails at `4145f3f` (before P2) and at `cf75bda` (before the Platform refactor entirely). Symptom, from the failing run's `fe-logs`: after the sketch is re-shown between the two extrudes, the click on the circle region does not select it and Extrude arms in multi-select instead. `sketchHitTestReady` already passed, so the sketch WAS hit-testable; the suspect is the click racing the visibility commit's projection push.
STILL OWED: the manual `tauri dev` smoke (open project → extrude → fillet → undo → save → reopen). Unchanged from the previous handoff — the one Definition-of-Done item with no evidence.
DEFERRED BY DECISION: the workspace runtime (`WorkspaceService`, workspace-filtered `SlotHost`, `WorkspaceExtension`) — it is not in the SDK's tool/tree/shortcut currency and can land without breaking an addon. Then: addon manifest + local loader, backend host + `platform_invoke`, GitHub installer, resource store, and the Rust facades before any crate extraction.

## PLATFORM REFACTOR M1+M2 (2026-08-08, plan `velvety-leaping-adleman.md`) — GATE PASSED
Modeling stops being synonymous with OneCAD: it is now the first built-in module (`onecad.modeling`) on a Platform, and `.onecad` carries namespaced module state that survives a round trip without its owner installed. Architecture only — no user-visible change, no modeling behavior change, no file moves.
**The shell no longer knows Extrude, Fillet, ModelTreePanel or the sketch toolbar.** `EditorScreen.tsx` went from 19 concrete imports to a one-line bridge; `src/app/shell/EditorShell.tsx` renders permanent structure plus one `SlotHost` per region, and the modules register into those slots.
WHAT MADE IT SAFE: four frozen behavior contracts written FIRST (`src/test/contracts/` — toolbar arrangement, the three keymap tables + cross-mode opt-out, editor mount order, inspector section order), with a README forbidding their edit to make a refactor pass. The toolbar assertion runs against `toolbarFromRegistry`, i.e. the arrangement rebuilt FROM the platform registry, not from the descriptor table that fed it — otherwise it would only prove the table equals itself. The keymap probe compares `resolveBinding` against an independently written oracle over every (key, shift, mode) triple. The mount-order probe was REPLACED (JSX scan → registry scan) against the same unmoved contract.
DESIGN CALLS WORTH CARRYING: contributions register on EDITOR MOUNT, not at bootstrap — the editor tree is a deliberate code-split chunk and hoisting its imports into the startup bundle to satisfy an architectural preference would make the start screen pay for the editor; that needed `platform.createScope(owner)` and a teardown fix (a scope disposes what IT registered, never sweeping the owner, or the editor's scope would kill the module's bootstrap registrations). Tool ids are SCOPE-QUALIFIED because `select`/`mirror` exist in both unions with different meanings. Separators are derived from group boundaries — `group` is consumer metadata, `priority` is the sort key, so order can never follow load timing. Two slots were added deliberately (`viewport.chrome`, `shell.notification`) because the frozen mount order cannot be reproduced with contiguous regions otherwise. `ToolDefinition.shortcutLabel` exists because Measure binds ⇧/ and is written "?".
PERSISTENCE (M2): `Document.modules` + a manifest descriptor table DERIVED at save, both `skip_serializing_if` empty so a document without module state writes no `"modules"` key anywhere — no container bump, no user migration. `EditCommand::SetModuleState` with a memento inverse puts programmatic writes on the SAME transaction path as user edits. `ModuleId` validates on AUTHORING only: deserialization is permissive because refusing an id a stricter build dislikes would destroy exactly the data preservation exists to protect. Proven: unknown-module state byte-equal across open → real modeling edit → save → reopen.
Three typed Tauri commands (`get_module_state` / `set_module_state` / `list_document_modules`), both clients implement them, and `src/platform/documentState.ts` binds the module id at construction so a module cannot address another's slice. **No dynamic router** — spec §97's `platform_invoke` is deferred to the addon-host effort where it will have a consumer.
`src/platform/architecture.test.ts` scans the real import graph for forbidden edges and carries a POSITIVE CONTROL, because every other assertion there expects an empty list — which is also what a broken scanner returns.
GATE: `bunx tsc --noEmit` clean · vitest **221 files / 3796 tests** (from 213/3720 at the W0 baseline) · `cargo fmt --all --check` + workspace `clippy -D warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1045 / 0** across 73 targets (from 1033/0) · ctest **107/107** · Playwright **386 passed / 4 failed** — the SAME four pre-existing `theme.spec` failures already root-caused to the concurrent session's TitleBar work · hex gate 0.
COMMITTED as `4145f3f` (77 files, +5252/−239), NOT pushed. `HANDOFF.md` was deliberately left unstaged — it carries 42 insertions from the concurrent session.
NOT DONE (recorded in TODO.md § Flagged seams + § NEXT SESSION): inspector sections / tree nodes / viewport layers are contracts with no producers yet; the toolbar component still renders the derived arrays rather than reading the registry live (`toolbarFromRegistry` exists and is proven, so the swap is mechanical); `zoomFit`/`home` are registered as modeling commands though they are really view navigation. **Manual `tauri dev` smoke was never run** — the only Definition-of-Done item without evidence.
**CORRECTION, made at handoff time:** this session first reported "OCCT artifact metadata is absent" as a pre-existing blocker on rebuilding the sidecar. That was a WRONG INVOCATION, not a repo problem — the build was pointed at `/opt/homebrew/opt/occt-8.0.1` (a plain Homebrew install with no metadata) instead of the pinned prefix `~/.onecad-occt/8.0.1`, which does carry `share/onecad/occt-build.json` and configures fine (`HANDOFF.md` § How to resume has the exact command). CONSEQUENCE for the numbers above: the worker-backed `cargo test --workspace` ran against `worker/build/onecad-worker` (19:37), which `HANDOFF.md` identifies as a **stale pre-VF-M5-gate build**, so it did not exercise `069bb48`'s worker changes. A HEAD-built worker is EXPECTED to fail `topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean` — that is the other session's open VF-M5 gate, not a platform-refactor regression. Every non-worker suite (vitest, tsc, fmt, clippy, ctest, Playwright) is unaffected by this and stands as reported.

## ADVANCED-FILLET M2 part 2 (2026-08-08) — GATE PASSED
The kernelbench v2 execution lane, cross-language. The worker executes a `schemaVersion: 2` case and the supervisor drives one end to end; no production fillet algorithm was touched.
C++: `CaseSpec` is NORMALIZED rather than forked — `Types.h` carries the v2 shape and the v1 parse fills the subset, so one execution path serves both formats (duplicating the executor per version is where a v2-only bug would hide). `parse_case` dispatches on `schemaVersion`; `CaseParserV2.cpp` mirrors `case_v2.rs::validate` field for field; shared primitives live in `CaseParserShared.h`. A non-constant radius law is REFUSED at parse — variable radius is M12, with a validator that can prove the law.
`GeometrySupportPair.cpp` is the point of the milestone. PRISM family (plane|cylinder × plane|cylinder) = one 2D profile extruded along Z, so the shared edge is a straight vertical line through the origin and the dihedral is FREE over (0,180); CONE family (plane × cone) = a frustum's base circle, where the dihedral is `90 - halfAngle` BY CONSTRUCTION, so the declared angle is rebuilt from the half angle and a disagreement is refused rather than silently generating different geometry than the file describes. Chose the prism form over the committed example's base-circle form because that locked plane↔cylinder to 90°, and the dihedral is the main conditioning axis for fillet failure; the prism form also keeps the blend a cylinder, which is what lets `constantRadius`/`cylindricalRadius` gate it. Example updated to match (not contractual; `regressions/` untouched).
Rust: new `prepared.rs` — `PreparedCase` is the version-agnostic view the supervisor executes (identity, ceilings, metamorph tolerance, canonical document), so `campaign`/`runner`/`child`/`result`/`result_validation` never grew a `match` on the schema version. New `suite_v2.rs` = `fillet/matrix` preset `m1` (24 cases, 60 records). `cli` gained the suite and a version-dispatched `run-case`.
TWO REAL DEFECTS FOUND BY RUNNING IT: (1) the result envelope echoed the case's `generator` block, so v2's extra `family` field failed strict result validation — 60/60 red; `Execution.cpp` now BUILDS the frozen v1 `{name,version,seed}` subset. (2) `parse_case_v2` did not re-check `schemaVersion` itself, so calling it directly on a v1 document read it as v2 — caught by the new fixture, now fail-closed on both sides.
FINDING (recorded, not worked around): `cylindricalRadius` and `g1BoundaryTangency` are BOX-SHAPED validators — the first counts every cylindrical face in the output against the requested radius, so a cylindrical SUPPORT reads as a blend of the wrong radius (17.0 mm error measured on a 20 mm support), and the second only recognises plane↔cylinder tangency pairs. Both are emitted only for all-planar pairs, pinned by test; their recipe-agnostic replacements land with M4.
Sizing is MEASURED: the prismatic throat is `R·(1 - cos θ)`; every pair blends at 0.20 of it and the cylinder↔cylinder lens refuses at 0.40, so supported cases sit at 0.04–0.16. Near-tangent dihedrals are `exploratory`, not `expectedLimit` — an expectedLimit case needs a radius the kernel is KNOWN to refuse, and guessing one produces a case that passes whatever happens.
GATE: ctest **107/107** pinned worker (106 → 107) · `cargo fmt --all --check` · workspace `clippy -D warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1032 / 0** across 73 targets (1000 → 1032) · kernelbench 56 lib + 5 integration (50 → 56) · **T0 both-backend UNCHANGED 136/136, replay 136 stable, metamorph 48/0/16, differential 136 same-status, gatingFailures 0** · **M1 both-backend 120 records, 114 pass + 6 characterization, replay 120 stable, metamorph 72/0, differential 120 same-status, gatingFailures 0, p50 10.3 ms / p95 62.3 ms** · committed v2 example runs through `run-case` on both backends, pass + replay-stable, ajv-valid.

## VF-M3+VF-M6 DEFECT FIXES + PROJECT IMPORT (2026-08-08) — GATE PASSED
Mailboxes: the two defect packages (VF-M5 from-0-replay anchor fidelity; VF-M6 import-blob lifecycle) and the user-priority feature "Import Project" landed together. **Import Project** appends another `.onecad` document's timeline to the open document: `onecad-core::io::project_import` reads the source container (records + sketches + referenced import blobs), `DocumentRuntime::import` stages blobs into the import workspace and merges every source record/sketch into one transaction, and the `api::import_project` command owns the `.onecad` picker (creates a blank runtime when none is open, appends when one is). Collision policy: a source `RecordId`/`SketchId` already present in the target refuses the whole import and leaves the target untouched — deterministic body minting needs unique ids, so the merge is fail-closed. Frontend entry points: File ▸ "Import Project…" (in-editor) and a start-screen sidebar button; `mockClient.importProject()` reuses the STEP fabrication so the whole mock lane is exercisable; `e2e/project-import.spec.ts` pins both.
VF-M5: from-0 replay ("ALLBACK" buckets) resolved stale WORLD anchors through the edited scene and a checkpoint rebuild is scored against the *edited* coordinates; the far-edge blend case repros as the edit confirms in the exact CHECKPOINT-replay context. The carve-out fix makes from-zero-replay behave differently from a live edit: the checkpoint-resume accepts AutoBind (its own rank never moved), the stale scene-participant falls out as `NeedsRepair`+ambiguous, and git-search/repair proceeds without silently misbinding. Kernel Half verified: worker is anchor-exact ONLY off from-zero-replay; the op-level bias feeds `OpContext` so every edit op sees the same gate; ctest 106/106 incl. the former failing checkpoint-replay kernel test, now asserting the correct `NeedsRepair` path.
VF-M6: `materialize` no longer trusts a directory where a file belongs (a collided digest dir armed a future import against a dead blob); stale-workspace sweeping is now PID-aware (`is_process_alive` via `libc::kill`, non-unix lanes degrade to no-sweep) so a live import cannot have its staging orphaned; `prepare_import` enforces the blob-byte cap against converted geometry too (belt-and-suspenders behind `add_import_record`'s raw cap).
GATE: `bun run build` clean · vitest 209 files / 3697 · Playwright project-import/step-import 12/12 · full Playwright 386/390 (4 pre-existing `theme.spec` failures root-caused to the concurrent title-bar work, not this package) · worker ctest 106/106 · cargo fmt/clippy clean · `cargo test -p onecad --lib` 253/0. `cargo test --workspace` (real-worker lane) + kernelbench T0 deferred to the next gate start.

**CORRECTION (2026-08-08, after-hours real-worker lane) — the VF-M5 `from_zero_replay` gate is WRONG and RE-OPENS this gate.** Running the full real-worker suite (`ONECAD_REQUIRE_WORKER=1 cargo test --workspace`, worker built from HEAD) fails one flagship test: `topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean` expects `needsRepair = 0` but the model comes back `needsRepair = 1`. The identical suite against a baseline worker (same tree, gate change stashed out and rebuilt) passes → the regression is the gate itself, committed in `069bb48`.
ROOT CAUSE: `PlanExecutor` (worker) computes the gate as `job.from_zero_replay = job.edited_from.has_value() && job.partition.size() == 0`. In V1 there is NO partition migration — every plan is "from-zero" over an empty base (no checkpoint plumbing; `Session.cpp` derives `from_zero` from `expected_base_hash == kEmptyPrefixHash`, trivially true for every plan). So `partition.size() == 0` is ALWAYS true, the gate degenerates to `edited_from.is_some()`, and an honest edit lane (`RegenRequest::ToEnd { from: 1 }` ⇒ carries an `edited_from`) is flagged as a from-zero replay → the anchor-exact carve-out is disabled → the fillet lane mis-scores and marks the model `NeedsRepair`. ~~The VF-M5 scenario proper ("replay a stale anchor after a REAL checkpoint restore") CANNOT ARISE in V1 because restores do not exist.~~ **WRONG — corrected 2026-08-09.** Restores exist and the executor's F12 fallback reaches that lane; it is now reproduced as a silent wrong bind and closed (TODO.md § VF-M5 RESIDUAL). Fixed with (pick one next session): (a) keep the track `anchorExact` carve-out ON until a real checkpoint-restore signal is plumbed; (b) plumb a `baseCheckpoint` discriminator into the plan so only true restores get the gate. The worker kernel ctest 106/106 does NOT cover this (it drives the ladder with synthetic flags, not the full RegenRequest lanE) — the regression is visible only in the real-worker cargo lane.
- **Rebuild required before any further real-worker run:** the sidecar binary in `worker/build/onecad-worker` was last built from a pre-gate tree (19:37), while source at HEAD carries the gate — results against that binary are stale either way.

## SESSION 3 SNAPSHOT (2026-08-08 19:43) — read HANDOFF.md first
- **Branch:** `master`, **4 commits ahead of `origin/master`, nothing pushed**, working tree otherwise CLEAN. The long-running uncommitted backlog from sessions 2 and 3 was split and committed at this session's close:
  - `796633c` `feat(kernelbench): case-v2 format + supportPair geometry generators` — M2 (this session, fully gated)
  - `cf75bda` `feat(bench): reproducible OCCT fingerprint + manual-QA triage (M0/M1)` — session 2
  - `685efc2` `wip: import project, viewcube drag, title bar appearance (concurrent work)` — **a concurrent session's in-flight work, NOT authored or gated here**
  - `069bb48` `wip: from-zero-replay anchor gate in the worker ladder (concurrent work)` — same, VF-M5
- **Still untracked by choice:** `.opencode/` (61 MB of agent tooling), `.agents/`, `STEP/` (sample CAD exports), `skills-lock.json`. Not project source; they want a `.gitignore` entry rather than a commit.
- **A SECOND SESSION MAY STILL BE EDITING THIS TREE.** Its work is now committed as the two `wip:` commits above, so a fresh edit from it will appear as new dirt. It added a **webkit** Playwright project (suite 190 → 386).
- **Session 3 files (M2 part 2, all in `796633c`):** `worker/src/benchmark/` — `Types.h`, `CaseParser.cpp`/`.h`, `CaseParserShared.h` (new), `CaseParserV2.cpp` (new), `SelectorParser.cpp`/`.h`, `Geometry.cpp`, `GeometrySupportPair.cpp`/`.h` (new), `Execution.cpp` · `worker/tools/kernelbench-runner/` — `CMakeLists.txt`, `case_v2_fixtures.cpp` (new) · `src-tauri/crates/onecad-kernelbench/src/` — `prepared.rs` (new), `suite_v2.rs` (new), `lib.rs`, `cli.rs`, `campaign.rs`, `runner.rs`, `child.rs`, `result.rs`, `result_validation.rs`, `search.rs`, `suite.rs`, `case.rs`, `metamorph.rs` · `tests/supervisor.rs` · `bench/robustness/README.md` + `examples/fillet-matrix-plane-cylinder-v2.json`.
- **Build/test (verified this session):** `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1032 / 0** across 73 targets against the pinned worker · ctest **107/107** · kernelbench 56 lib + 5 integration · `cargo fmt --all --check` + workspace `clippy -D warnings` clean · kernelbench T0 both-backend **136/136, unchanged** · new `fillet/matrix` `m1` **120 records, 0 gating failures** · `git diff --check` clean · hex gate 0. Vitest 209 files / 3698 is session 2's number — the frontend was not touched by session 3.
- **BLOCKER (not from this work, now committed inside `685efc2`):** `bun run build` is RED — `src/ipc/mockClient.import.test.ts:106` `'snap' is possibly 'null'`. Same class as the M0.2 bug: vitest is green because it does not typecheck. Does not affect M2, which is Rust + C++ only.
- **Blocked:** a full-suite e2e number is not attributable while a concurrent session edits this tree.
- **Next:** M3 metamorph execution, then M4's recipe-agnostic validators (which is what unblocks required validators on curved-support pairs). See HANDOFF.md § How to resume.

## ADVANCED-FILLET M0 + M1 (2026-08-08) — GATE PASSED (with named P0 residue)
First two milestones of the Advanced-Fillet roadmap. No production fillet algorithm touched — by design: the benchmark discovers the problem before the kernel guesses at a solution.
M0.1 OCCT fingerprint: root-caused against pinned source AND a real configure. OCCT's `OCCT_CHECK_AND_UNSET` does `unset(VAR CACHE)`, `CAN_USE_*` comes from scanning `BUILD_TOOLKITS`, and with `BUILD_MODULE_Draw=OFF` nothing needs `CSF_TclTkLibs` — so `-DUSE_TK=OFF` is DELETED from the cache and the old literal-presence loop aborted a correct configuration. Nine more keys can vanish the same way. New `scripts/occt-fingerprint.sh` normalizes EFFECTIVE configuration: an absent dependent key is OFF only when the policy asked for OFF; requested-ON-but-dropped, a missing required key, a disagreeing present key, and a non-CMake-boolean value all stay fatal. `scripts/tests/occt-fingerprint.test.sh` 14/14 with fixtures captured verbatim from a real cache. Verified end to end by building pinned OCCT 8.0.1 from scratch.
M0.2 frontend clean build: `bun run build` was red while vitest was green (vitest does not typecheck). `Rgba` is now readonly and the ~20 duplicated raw rgba tuples collapse onto it; read-only consumers take `ReadonlyMap`; new `src/test/fixtures/bodyMeshView.ts` replaces a hand-written 24-field structural mock. Also fixed a real hex-gate violation (`InspectorPanel` hardcoded `#a9aeb6`, which IS `--color-body-fill`) — now read through `palette.bodyNeutral()`.
M0.4 viewport auto-fit: bisected a REAL regression from `1fe0cef`. Its debounced re-fit lived in the React bridge, so a `fitView()` tween could start after everything else believed the camera had settled. Debounce moved onto `ViewportEngine` (`requestAutoFit`/`autoFitPending`/`cancelAutoFit`); e2e went 12+ failures → 7.
M1: all ~112 manual gate boxes triaged — 78 retired against existing automated tests (each cited `path:line`), 22 named gaps, 12 genuinely manual. `docs/qa/MANUAL_RELEASE_GATES.md` + `docs/qa/MANUAL_GATES_TRIAGE.md`; old list archived.
GATE: vitest 208/3687 · ctest 106/106 (pinned worker) · cargo fmt/clippy clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` 1000/0 (pinned worker) · kernelbench T0 136/136, 0 regressions · fingerprint 14/14 · hex gate 0 · Playwright 183/190.
M0.5/M0.6: the 7 remaining e2e failures (all pre-existing on a pristine `1fe0cef`) root-caused and fixed — fat-line edge layer polluting a spec bbox helper; the new selection-gated toolbar plus a locator matching two different inputs; and camera-distance inheritance making the sketch reject radius + dimension quantum zoom-dependent. 52/52 across the touched specs. OPEN: `playwright.config.ts` retries 1 in CI / 0 locally — a retry is what let the auto-fit regression look green. CAVEAT: a concurrent session is editing this tree (TitleBar/ViewCube/CadOrbitControls/Picker/ModelToolController) and added a webkit Playwright project, so a full-suite number is not attributable right now.

## BODY/FACE COLOR (2026-08-07) — GATE PASSED
Implemented user-authored body color and per-face color. Rust persists both in `BodyMeta` (`color` and `face_colors: BTreeMap<ElementId, [u8; 4]>`), exposes `SetBodyColor` and `SetFaceColor` commands, and projects them through `BodyDto`. Frontend stores and actions carry the new fields with optimistic writes; mock client implements both and resolves persisted elementIds back to current topoKeys. Mesh pipeline bakes body and face colors into vertex colors via `faceColors.ts`, looking up authored face colors by the mesh's actual id (ElementId when `idsHaveElementIds`, else TopoKey via `elementInfo` fallback). `meshSync` rebuilds on color changes. InspectorPanel has an Appearance section for body and face selections with color picker + opacity slider + reset (face section re-added after it was dropped from the file; face status/name/DOF handling corrected).
FINAL GATE: `cargo fmt --all --check`, workspace `clippy -D warnings`, and `cargo test --workspace --all-targets` all green; `bun run build` and `bun run test` green (206 files / 3636 tests). Added focused regression tests: Rust face-color serde round-trip; Vitest face-color wire payload + clear.
Deferred follow-up: real-worker interactive smoke (command is wired; rendering verified on mock lane), and imported STEP per-face colors are not yet keyed by persistent id.

## KERNELBENCH KBR-0 + FILLET SLICE (2026-08-07) — GATE PASSED
First end-to-end OCCT robustness benchmark (`onecad-kernelbench` Rust
supervisor + `onecad-kernelbench-runner` C++ child), differential raw-OCCT vs
production `FilletBuilder`, landed and gated. Resumed a session left with C++
non-buildable (interrupted semantic-selector refactor: `Geometry.cpp`
referenced removed `SelectorSpec` fields). Fixed by inspecting live files, not
trusting the handoff doc's claims of what was already done — several were
stale or wrong. Root-caused two bugs invisible to unit tests, only found by
actually executing T0: `suite.rs` emitted exactly one `surfaceDescriptor`
regardless of anchor count, silently crashing every disconnected/multiple-edge
box case (8/12 supported cases) at runtime; overflow-wedge cases used the
wrong selector mode entirely (anchor coordinates re-derived from a standalone
OCCT probe program, not from docs, since `BRepPrimAPI_MakeWedge`'s taper axis
isn't what the header comment implies). Also closed: ordinal-free semantic
selector now resolves by nearest-anchor matching (tolerance + uniqueness, no
OCCT ordinals ever persisted); `BRepCheck_Analyzer` silently reports an
empty solid as valid, so `exactValid` now also requires `faces > 0`; several
Rust supervisor gaps (search-result validation was missing `probeRadius`
entirely — every real search record failed strict validation; supervisor
artifact writes weren't cumulative-capped; macOS `RLIMIT_AS` is unreliable,
gated to RSS-polling only). Biggest lift: metamorph (translated/rotated
variant) evidence was fabricated (`surfaceSamplesMatch` aliased to an
unrelated bool). Replaced with genuine evidence — worker emits deterministic
face-centroid + probe-point shape signatures, new `metamorph.rs` inverse-
transforms and compares them within tolerance; verified via real T0 execution,
not just unit tests. FINAL GATE: worker CTest 106/106 · Rust workspace
fmt/clippy/test all green (worker-backed) · kernelbench unit+integration 35/35
· T0 both-backend 136/136 pass, replay 136/136 stable, differential 136/136
same-status, metamorph 48 pass/0 fail/16 correctly-notRun · search-critical
transition found near 16 mm · 332 generated cases/presets/results schema-valid
under Draft 2020-12 · `git diff --check` clean. Deferred: dedicated per-
scenario CTest fixtures (proven via real T0 run instead), boolean/later ops,
history adapter, Windows resource limits — all out of original scope. Details:
TODO.md § KERNELBENCH KBR-0.

## HISTORY-HARDEN (2026-08-04, plan `act-as-senior-software-mutable-turing.md`) — COMPLETE
Parametric history hardened end-to-end: change past decisions like professional
CAD. 10 waves, 6 gate commits (df15632→3ab2a22), internal adversarial review
REVISE→4 BLOCKER+9 MAJOR folded pre-approval, 2 design amendments during
implementation (recorded in TODO.md). FOUR blocker-class root causes found
that predate the plan: (1) ROLLBACK WAS VISUALLY INERT (count/index domain bug
— any roll-to published nothing); (2) `IntentQuery` NEVER CONSTRUCTED —
production topological rebinding was pure nearest-centroid at weight 0.25, the
entire descriptor system dead on the resolve path; (3) rolled-back append trap
(new features became permanently inert drafts); (4) unrepairable refs with
empty anchors FATALLY tore down regen. All fixed + red-first pinned.
Shipped: inline dimension editing on history rows (+ sketch-dim quick path);
rollback cursor viz + context menu + roll-to-end banner; insert-at-cursor;
snapshot-scoped promotion (VF-M3/M4); frozen descriptors (VF-M5a) with
measured (score,margin); edit-scoped anchor-tie veto w/ anchor-exact carve-out
(drift caught, teleport = documented residual, flagship gesture preserved;
resolverVersion 2, additive `editedFrom`); failure visibility (halt banner,
suppress-to-continue, regen busy); undo/redo checkpoint acceleration
(`RevertToEnd`, dirty floors, B1 clamp) + checkpoint-swallows-sketch guard;
reattach V1 world⇄datum (was inert — record plane never restamped); repair
generalization (slot table pinned both sides, Hole rebind volume-proven,
bodyId on items, viewport marker); dependency view. Also closed: VF-M1
(poison heals on param edit), VF-M7 (cross-body drill/shell guard).
FINAL GATE: ctest 89/89 · cargo 834/0 vs real worker · FE 3090/3090 (194) ·
e2e 179/179 zero-fail · tsc/fmt/clippy/hex clean. USER: MANUAL_GATES_RUN.md §0.
Deferred (TODO.md): from-0 history rung · worker sketch-state across
checkpoint restore · DAG error isolation · Shell faces §7.3 shape · reorder.

# OneCAD-Tauri — Current State (2026-08-03, WP-FIX COMPLETE — all Phase-V blockers closed)

## WP-FIX COMPLETE (2026-08-03, 5 waves, commits 4b9d9b8→(W5))
All 7 Phase-V BLOCKERs + adjacent M2/M8 fixed, every wave red-first-tested +
orchestrator-gated: W1 colored-mesh mask/instance fencing/epoch seeding/
checkpoint-mint guard · W2 checkpoints in-session-only + bounded ladder +
per-request D1 known-ops (reopen→append wedge dead; container growth
quadratic→linear, save 154→30 ms) · W3 save/autosave off the single-writer
lock (edit stall during autosave 2998→0.02 ms; persistence lane kills the
recovery-marker race) · W4 transform gate sees host-face sketches (hash-frozen
legacy bridge), delete seeds the gate, seeded repair indices track the
timeline, single-snapshot repair inverses · W5 split-ordinal tripwire
(worker rankKey evidence → OrdinalPermutation NeedsRepair, self-healing
anchors — silent identity swap dead). PLUS one pre-existing product bug found
by the gate rerun: chip clusters rendered UNDER the z-20 side panels near the
right edge (unclickable — e2e hole spec caught it); fixed via a dedicated
z-30 chip layer in ViewportEngine. Suite state at WP gate: ctest 87/87 ·
cargo 761/0 vs real worker · FE 2693/180 · e2e (rerun) · hex clean.
Remaining VF majors (M1 poison-key, M3/M4 identity-cache, M5 stale-anchor
ladder, M6 imports edge-cases, M7 cross-body topoKey) → WP-FIX2 queue.
Next per approved plan: user manual gates (Step 0) → WP-SHIP (unsigned .dmg
now UNBLOCKED — F1a/F1b landed; F2 release logging still open in SHIP scope)
→ feature packs D (print) → E (inspect) → F (machined) → G (polish).

# Previous header: (2026-08-03, FULL ROADMAP IMPLEMENTED + VALIDATION PHASE V)

## VALIDATION PHASE V (2026-08-03, plan `act-as-senior-software-shimmying-blanket.md`)
Independent re-validation of the shipped state: all 4 suites REPRODUCE EXACTLY
(ctest 87/87 · cargo 704/0 vs real worker · FE 2688/180 · e2e 157/157 zero-flake
· hygiene + hex fully clean). Probes: save-growth defect CONFIRMED empirically
(checkpoints 1/save never evicted, quadratic container growth, save 19→154 ms in
12 toy saves, all under the runtime lock); soak clean (no RSS growth over 120
regens). Adversarial review (3 agents, orchestrator-verified): **7 BLOCKER-class
defects** — colored-STEP meshes rejected by the Rust MESH1 validator (mask omits
FACE_COLORS — colored imports render NOTHING on the real lane); cross-document
regen commit (new_document during in-flight regen publishes doc A into doc B);
reopen→append regen wedge (self-satisfying fingerprint gate + in-session-only
restore + RetryFromZero reusing narrow known_ops = D1 hard-fail);
epoch desync when the worker restarts with no doc open; transform edit-safety
gate holes (host-face sketches invisible to it, delete bypasses it, positional
step_index drifts); split-child ordinals are geometric ranks (param edit swaps
identities silently); save/autosave serialize everything under the single-writer
lock. Full ranked list + 8 majors: TODO.md § VF FINDINGS. Next: WP-FIX (blocks
the unsigned tester .dmg), while user runs `docs/MANUAL_GATES_RUN.md` Step 0.

## ROADMAP COMPLETE (2026-08-02→03, ~29 commits d875ef9→d3c9289)
Every code item of the approved roadmap (plan
`act-as-senior-software-reflective-swing.md`) is implemented and gated:
WP-0 identity prerequisite · WP-A STEP-IMPORT (8 waves incl. XCAF colors) ·
WP-B BODY-TRANSFORM (gizmo, fold, align, edit-safety gate) · WP-C pro-ops
(mass props, units, draft UI, revolve binding, 2-dist chamfer, sketch
fillet/offset, hole tool w/ ISO/DIN standards). Final suite state:
**ctest 87/87 · cargo 704/0 vs real worker · FE vitest 2688/180 ·
e2e 157/157 · tsc/clippy/fmt/hex clean.** Suite growth over the run:
ctest 79→87 · cargo 600→704 · vitest 2172→2688 · e2e 111→157.
REMAINING (user-run): `docs/MANUAL_GATES_RUN.md` — §0 NEW-FEATURES batch +
the pre-existing backlog of open manual gates. Deferred by design: WP-A W6
(preflight dialog + progress frames), WP-B W3 pickFrame (safe via seeding
gate), riders (pattern-axis Tier B, DeleteBody op, scale), DIN 974-1/DIN 74
transcription spot-check, Mac packaging (needs credentials).
THREE production stdout-corruption defects found+killed during WP-A (all
would land bytes mid-OCW1-frame); pre-existing stale-anchor latent fixed in
WP-B W0; ModelTreePanel vitest race root-fixed in W4.

## WP-B BODY-TRANSFORM COMPLETE (2026-08-03, commits a540e74→7898e7d)
Full placement suite on the W0 backend: W1 FSM/chips/`t`/fold-flow (fold =
backend lineage query AND stored-targets==selection; full-vector FSM state so
axis switches can't clobber components; exact mock mesh transform incl. header
bbox + FACE_BBOXES re-derivation); W2 gizmo (9 handles, extrude-precedent
orbit arbitration, ring seam unwrap 360-not-0, degenerate-view guards
negative-checked, Copy sticky — one record has ONE flag); W2.5 align
face-to-face (planarity honesty gate, deterministic 180° branch, round-trip
1e-12, tangential nudge live after the solve). NO PreviewOp lane — rigid ghost
is kernel-exact (documented deviation). W3 pickFrame stays DEFERRED, safe via
the W0 edit-seeding gate. Final: FE 2429/171 · e2e 132/132 clean-machine.

## WP-C tranche 1 (2026-08-03, commits cf5a236/425c175/e2af9a0)
Mass properties (worker GProp verb → MeasurePanel; two-plane angle 60/120;
mock exact via divergence theorem — cylinder asserted as its 24-gon prism);
display units mm/cm/m/in (wire NEVER leaves mm — marshalling independence
pinned; valueText re-parse lanes deliberately mm-fixed); draft-angle UI
(±89° oracle clamp — backend was wired end-to-end with zero UI dispatching
it); revolve typed-region selection binding (the TODO:123 defect was mostly
already fixed by TRUST T3 — residual was selection binding, now closed).
Combined gate: ctest 86/86 · cargo 681/0 · FE 2534/173 · e2e 144/144.
Tranche 2 queued: 2-distance chamfer, sketch offset + fillet, hole tool.

## ROADMAP (approved 2026-08-02, plan `~/.claude/plans/act-as-senior-software-reflective-swing.md`)
User priorities: 3D-print + machined parts + daily driver; light multi-part (no
mates); STEP import first. Queue: Step-0 USER manual gates
(`docs/MANUAL_GATES_RUN.md`) → WP-0 ✓ → WP-A ✓ → WP-B (in flight) → WP-C
pro-ops sweep (mass props, units, hole tool, draft UI, 2-dist chamfer, sketch
offset/fillet, revolve region-parity). NOT next: assemblies/mates, Loft/Sweep,
drawings. Internal adversarial review pre-approval: REVISE → 3 BLOCKER + 7
MAJOR + 8 MINOR all folded (incl. B1 transform wrong-bind, B2 256-solid cap,
B3 checkpoint-replay fallacy → brep/xbf-primary codec).

## WP-B BODY-TRANSFORM W0 (2026-08-03, commit 0dc8c91) — parametric move/rotate/copy
Core `TransformBody{targets[], translate 3×Scalar, rotate{frozen center, axis,
angleDeg}, copy}` + `can_fold_transform` lineage query (fold rule: one
cumulative record per placement intent). Worker TransformOp: normative T∘R,
copy:false = modify-in-place id-preserved via apply_history level-1 rebind +
NEW `apply_placement` (anchors move WITH the body — fixed the pre-existing
stale-anchor latent); copy:true mints §2 N-body ordinals. EDIT-SAFETY GATE
(refined from the plan's blanket ban): healthy transform-then-model flows
resolve clean; editing/suppressing a TransformBody seeds NeedsRepair on
downstream lineage refs (descriptor scoring against moved geometry admits
congruent-decoy WRONG binds — H5-B class; gate holds until pickFrame W3).
ctest 85/85 · cargo 680/0 · transform_body 8/8. W1 FE (FSM/chips/`t`/fold/
exact mock) in flight; then W2 gizmo, W2.5 align, W3 pickFrame deferred-safe.

## WP-A STEP-IMPORT (2026-08-02, commits 0598369→885d92e) — COMPLETE
Real import: Start Screen + in-editor File menu → XCAF product names + per-face
colors → bodies are FIRST-CLASS (fillet/boolean/sketch-on-face/patterns) →
identity survives process death. Architecture: §7.8 import VERB was
structurally wrong (session mutation outside ExecutePlan = deleted by first
regen) → §7.3 `ImportStep` RECORD, content-addressed source in NEW
authoritative `imports/<sha256>` container section (256 MiB cap, refcount at
save pins suppressed records + provenance, doc with missing blob still OPENS).
Replay = **xbf-primary** (BinXCAF storage v12; plain brep DROPS XCAF attrs;
STEP co-stored as provenance via `provenanceSha256`; W0 proved cross-process
determinism + BinTools ~100× faster than STEP re-parse — and `from:0` regen
NEVER consults checkpoints, so replay speed is open-time UX). `InspectStep`
probe (includeGeometry conversion lane) runs BEFORE record authoring — bad
file leaves the start screen intact. MESH1 gains FACE_COLORS (type 12, flags
bit 4, unset=alpha-0 → body token, theme-rebake in place); FE de-indexes
colored bodies (triangle ordinals preserved — picking pinned by real-raycast
test), shadedVertex MaterialKind white-base.
THREE stdout-corruption defects found+fixed en route (all would land bytes
mid-OCW1-frame): BinTools_ShapeSet::Read version banner, TDocStd_Application's
PRIVATE messenger (58B ANSI), plus ExportStep's own knob leak (observed).
XCAF does NOT inherit colors downward — solid-label fill pass (whole-part-blue
was 0/6 faces). W5 acceptance gate: ordinal-agnostic, survived the brep→xbf
flip unchanged; delete-import ⇒ NeedsRepair "no-candidates", undo exact.
Suites at close: ctest 84/84 · cargo 654/0 · FE 2223/162 · e2e 114/114 · tsc/
clippy/fmt/hex clean. Deferred: W6 preflight dialog + progress frames.
SEAM FLAGGED (pre-existing): promoted-but-unconsumed ElementId doesn't survive
process death → face-sketch dblclick re-entry match can fail after reopen
(TODO.md, route to face-sketch owner).

## WP-0 (2026-08-02, commit d875ef9) — split-child identity prerequisite
`split_origin` 256-probe DELETED → `split_child_uuid` memoizes derived→(op,k)
at its sole mint path; exact + unbounded (>256-solid import used to silently
lose `split_of` ⇒ cross-process REF_UNRESOLVED). SCHEMA §2/§7.2 `:<k>` widened
to "ordered children of any N-body op" + single-vs-multi minting rule.
Locked at k=300; M5a cold-interner proof green.

# Previous state (2026-08-02, FILLET-CHAMFER-UNIFY shipped)

## FILLET-CHAMFER-UNIFY (2026-08-02, commit (pending)) — ONE direction-driven edge tool
Plan `~/.claude/plans/act-as-senior-software-peppy-quilt.md` (internal adversarial
review REVISE → 1 BLOCKER + 6 MAJOR folded pre-approval). Shapr3D parity: F arms
one "Fillet / Chamfer" tool; drag AWAY from the body rounds, INTO it bevels, the
chip flips live; `[Fillet|Chamfer]` segments are the explicit override that ends
auto for the session (host-boolean precedent). `chamfer` tool id + `H` are dead
(`h` = Home again); the chamfer ICON lives on in history rows.
THE HONESTY RULE: the drag direction may pick the op ONLY where the sign is
provable — the bisector tier (sum of adjacent flat face normals reconstructed
from MESH1; FE has no edge→face topology). The bbox fallback points INTO
material on a concave edge, so off-bisector arms are `auto:false` and the chips
own the type. A cylinder cap edge degrades tiers HONESTLY (smooth normals fail
the tangent-plane test) and is pinned by test.
FLIP MECHANICS: `beginPreview` freezes opType, so a type flip is a session
close+reopen fenced by `++armGen` (two hysteresis crossings inside one in-flight
begin = stale-kind session + leak — race-tested); hysteresis 0.25 > value clamp
0.1 so type never strobes at the floor; mid-drag flip restores the 80ms trailing.
RE-EDIT TYPE FLIP: core `op_type_edit_allowed` sanctions exactly the
Fillet⇄Chamfer pair (enum-variant matched; params field-identical; undo exact via
whole-record inverse; legacy `mode` string normalized in core). Re-edit chip
became the full armed cluster — necessary: `DimensionInput.commit` fires only on
a CHANGED number, so a pure type flip could never commit from the bare chip.
PRE-EXISTING LEAK FIXED (W2): re-edit while the same tool was armed never swept
the old arm's in-flight preview session (`setTool` no-op → no `onToolChange`);
`editEdgeOpFeature` now fences AFTER `setTool`.
PROOF vs real worker: swap fillet(r=2)→chamfer(d=2) on a 20×... box — removed
volume 23.463 → 50.000, ratio 2.131 (analytic 2.33; asserted on the RATIO, never
the record's own opType (tautology) or label (fixture rename)); undo restores;
zero needsRepair.
Suites at final gate (orchestrator-verified): ctest 79/79 · cargo 600/0 vs real
worker · tsc 0 · FE 2135/156 · e2e 111/111 · hex 1 pre-existing (inputProbe).
NOTE: workspace `fmt`/`clippy` at gate time were red from the CONCURRENT
DEV-OBSERVABILITY session's in-flight edits (its files only); this WP's crates
tested clean before those landed.
REMAINING: USER manual Tauri gate (TODO.md FILLET-CHAMFER-UNIFY checklist).
Backlog: 2-distance chamfer / chordal / G2, edge-midpoint drag gizmo.

## DARK-MODE (2026-08-02, commit (pending)) — Light/Dark/System, chrome + live viewport
Plan `~/.claude/plans/act-as-senior-software-transient-puddle.md`.
The app had no theming layer at all: one flat `@theme` block of light hexes, and
a viewport that memoizes `THREE.Color` and builds ~30 materials once. So a token
swap would have repainted the chrome instantly and left the entire 3D scene
light. The only pre-existing seam, `resetPaletteCache()`, was exported with zero
callers; this wave wires it.

`src/theme/` owns resolution — a registry (`themes.ts`, mirroring the
render-mode table) plus a controller that is the sole writer of `data-theme` on
`<html>`. Only the PREFERENCE is persisted; "system" resolves against the OS at
runtime, following the precedent `navigation.inputDevice` set. Chrome re-themes
through CSS alone: Tailwind v4 compiles color utilities to `var(--color-*)`, so
a `:root[data-theme="dark"]` block re-themes everything with no `dark:` variants
anywhere.

The viewport cannot do that, so it is told: `ViewportEngine.applyTheme()` drops
through clear color → light levels → environment → eleven layer refreshes. The
order is load-bearing — PMREM fills the directions the studio room does not
cover with the renderer's clear color, so the environment must rebuild AFTER the
new background lands.

Two findings worth carrying forward. Tailwind INLINES `--shadow-*` at build
time, so overriding those tokens per theme is a silent no-op — dark shadows go
through `--tw-shadow-color` instead. And `--color-body-edge` has to INVERT:
wireframe mode draws edges with no faces behind them, so near-black edges on a
dark canvas would be invisible.
Appearance is reachable two ways: the full three-way radio in the display
popover, and a one-click cycling shortcut in the title bar (naming the current
value, so its changing icon is never ambiguous).
Suites at gate: tsc 0 · FE 2085/153 · e2e 11/11 new (94 prior untouched) · hex
clean. cargo/ctest not re-run — only `capabilities/default.json` changed on the
Rust side.
OPEN: a user reported dark chrome with a still-LIGHT viewport under `tauri dev`.
Not reproducible in the mock lane on either path (toggle-after-boot and
cold-boot-already-dark are both green vs real WebGL) and the dark canvas token
is confirmed in the built CSS. The palette cache was re-keyed BY THEME in
response, which removes the ordering-bug class that would explain it: a read
taken while the document says dark can no longer return a light color, so
correctness no longer depends on `resetPaletteCache()` having fired first. The
leading remaining theory is a Vite HMR artifact, not a product bug — a CSS-only
hot reload re-themes chrome instantly while `data-theme` never CHANGES, so no
event fires and the engine's already-built materials stay stale. Needs a cold
restart plus devtools output from the real webview to close.
BACKLOGGED THIS WAVE: the first per-layer test draft was VACUOUS (most layers
keep shared materials off the scene graph until something draws with them, so an
idle traversal proved nothing) — caught only by neutering an implementation and
watching the test stay green; the helper now refuses an empty traversal and every
case attaches real content. `--tw-shadow-color` is a Tailwind internal and no
test can catch a regression in it, because jsdom never runs the Tailwind
pipeline. Dark collapses per-shadow alphas to a single tint. `--color-on-accent`
goes near-black on the accent for AA contrast, which reads unfamiliar.
`palette.ts` now mirrors ~40 token values by hand across two themes with nothing
enforcing the match.
REMAINING: USER manual Tauri gate (TODO.md DARK-MODE checklist).



## RENDER-MODE (2026-08-02, commit 55454d0) — display-mode registry + studio shading
No plan doc; direct WP continuing MODELING-REACH W3, which shipped the
display-mode button as a 3-state cycler over session-only `viewportStore` state.
Now the mode is DATA: `renderModes.ts` holds the descriptor table
(`faceVisible` / `edgeVisible` / `materialKind`), so adding a mode is a table
entry, not a branch in `BodyObject`, the ingest controller, or the store.
`bodyMaterials.ts` becomes the sole owner of body materials — shared per-kind
sets, dim save/restore that replays the observed prior state, tints that copy
rather than retain the shared palette color. `displayMode` moved to persisted
`settingsStore` (v4) and is coerced on EVERY hydration, not just on a version
bump, because a same-version blob can still carry an unknown id. The cycling
button became a radio-row popover sharing one open-slot with the snap popover.
Shading is the other half: Neutral tone mapping, a `RoomEnvironment` PMREM IBL
built once at init (never per frame — the idle-zero-rAF contract holds), and a
camera-relative key/fill rig. WebGL-only by CONSTRUCTION — `createEnvironment`
is simply absent on the WebGPU and mocked handles, so no `isWebGPU` branch
exists in the engine. Every overlay layer now sets `toneMapped: false`: tone
mapping is for lit body faces, everything else renders its design token exactly.
Suites at gate: tsc 0 · FE 2012/149 · e2e 94/94 · ctest 79/79 + cargo 594/0
untouched (no Rust/C++ files in the diff) · hex clean.
BACKLOGGED THIS WAVE: the IBL path has NO automated coverage — the unit-test
renderer mock deliberately omits `createEnvironment` and e2e asserts scene-graph
flags, not pixels, so the studio look rests on the manual gate alone; the WebGPU
lane (lights-only, higher intensities) is untested; PMREM fills uncovered
directions with the renderer clear color, so `--color-canvas` now silently
drives body shading; `palette.ts` still caches colors forever and
`resetPaletteCache()` is still unwired, leaving every construction-time material
frozen for the life of the engine; two `BodyMaterialLibrary` instances exist
with no single owner able to reach both.
REMAINING: USER manual Tauri gate (TODO.md RENDER-MODE checklist).
NOTE: the frozen-material and unwired-`resetPaletteCache()` flags above were
closed by the DARK-MODE wave that followed.

## SKETCH-ON-FACE (2026-08-01, commits 2ac7aba→(final)) — sketch on model geometry, with the host outline as locked reference
Plan `~/.claude/plans/act-as-senior-software-twinkly-crown.md` (internal
adversarial review REVISE → 2 BLOCKER + 6 MAJOR + 6 MINOR folded pre-approval).
Four gates + HOST-BOOLEAN follow-up, all 4-suite green vs the real worker.
**HOST-BOOLEAN (user-reported same day)**: every op off a face sketch spawned a
NEW body — Shapr3D push/pull expects the HOST modified. Fresh arm on a hostFace
sketch now seeds Add + host target (`BooleanSeed` through the pure arm event;
re-edit never clobbered; hidden host falls back); extrude drag is
DIRECTION-AWARE while auto (away=Add, into=Cut, live chip+tint; symmetric never
flips; explicit chip click ends auto for the session); revolve seeds Add. Bound
target survives Add↔Cut (was: re-prompted pick on multi-body docs). Proof vs
real worker: Add → bodyEvents MODIFIED / 1 body / 153600 exact; Cut inward →
86400 exact. Typed-negative-depth chip does not flip (drag only — open).
THE POINT: a part is built by sketching on what you already made. Before this
wave a face-hosted sketch opened EMPTY — no outline, nothing to snap or
dimension against — so "sketch on a face" was decorative. Now the host face's
own boundary is projected in as `referenceLocked` geometry that bounds regions
but cannot be moved or deleted.
**W0 (2ac7aba)**: W0a `face_sketch_plane` topoKey rung — the SAME latent
MODELING-REACH W2 fixed for `element_info` and flagged unfixed here: element-map
entries mint only when an OP consumes an id, so a just-promoted face is
genuinely absent and the feature's single most common path failed loudly.
Red-first vs the real worker. W0b `referenceLocked` END-TO-END with zero
producers (flag on all 5 entity variants, L1 guards, worker + solver, FE
hydration + marshal latch). **DEFECT CLASS FOUND**: the C++ oracle's
`addConstraint` veto SILENTLY DROPPED every non-Fixed constraint naming locked
geometry — removed; immobility moved to solver tag-0 pins (endpoint-position
pinning is singular on 180° arcs, so arcs pin by ANGLE).
**W1 (d145ae6)**: worker `FaceBoundaryProjector` rewritten to oracle parity
(exact Line/Circle/Arc, CCW-via-D1-tangent, normative ordering, byte-identical
determinism) + `ProjectFaceBoundary` verb (SCHEMA §7.6, no fixture bump) +
Rust `add_sketch_on_face` (frameOnly → Rust-owned basis → real projection in
that basis, 1e-9 exact tripwire, failure leaves the document untouched).
**PRE-EXISTING BUG FOUND+FIXED**: `CoplanarFacePatch`'s predicate took
`gp_Dir::Dot` through an implicit `gp_Vec→gp_Dir` conversion, which made the
1e-3mm DISTANCE test angular (a 0.05mm-off face at 100mm lateral passed) and
threw `Standard_ConstructionError` on coincident origins. Coplanar collection
also became an all-faces scan, not BFS — disconnected coplanar prongs were
being missed. Proof: a tilted 10°-draft face projects onto its own plane with
origin-on-plane ≤1e-4 and extrudes to kernel-area×height to 8 sig figs.
**W2 (b01b71b)**: the real lane (`tauriClient` face branch → the new command;
the FE can no longer author a host-face `AddSketch` at all), mock parity from
the shared mock-mesh corners, L3 guards (drag/Trim/Delete all refuse, loudly),
machine pins hidden from the inspector without touching the session. The
**bbox-centre anchor** was the blocker here — it is not a point ON the face, so
the anchor became the kernel-exact plane origin. Hint-clobber fixed early:
`tryEnterOnSelectedFace` returns `{entered, refusal}` so a non-planar reason
survives the picker's own prompt (a hint published before `beginPlanePick`
writes is silently overwritten — the last-word-wins trap MODELING-REACH W0a
already catalogued).
**W3**: the two remaining entry triggers. The plane picker now accepts a body
FACE after the datum and the three world quads (order pinned with all three
stacked under one pointer), with the standard face tint driven through
`selectionStore.setHover` — one writer — and a prompt that names the body.
Deliberately NO hover-time planarity check: that is one backend round-trip per
pointer move. Double-clicking a face in model mode re-enters the sketch already
hosted there, else creates one, and VALIDATES BEFORE flipping the mode so a
non-planar face hints without moving the user anywhere. Re-entry needed the
frontend to know which sketch lives on which face, which nothing projected —
hence the additive internal `SketchDto.hostFace {bodyId, elementId}` (from the
attachment's `primary`; `protocol/SCHEMA.md` untouched, zero worker frames
moved, world/datum rows byte-identical).
Suites at final gate: tsc 0 · FE 1962/143 · cargo 592/0 vs real worker · ctest
79/79 · e2e 94/94 · clippy/fmt clean.
BACKLOGGED THIS WAVE: "newest sketch on this face" is last-in-projection-order,
which is uuid-lexicographic in the real lane (harmless — both candidates are
valid hosts — but not literally newest); `FaceExtrudeProfileBuilder` keeps its
own coplanar scan (correct math, repointing changes shipped-op behavior);
locked bare Point / Ellipse carry no solver pins; `beginGroupDrag` lacks a
locked pre-filter (fails loudly via pins); synthesized mock bodies keep the +Z
fallback frame; a body face is not in the orbit gate during plane picking (same
deliberate choice datum planes made).
REMAINING: USER manual Tauri gates (TODO.md SKETCH-ON-FACE checklist + the older
MODELING-REACH/SKETCH-POWER/TRUST/PREVIEW ones); Codex post-hoc reviews after
2026-08-05. NOTE: `src/viewport/debug/inputProbe.ts:87` holds one raw hex in the
file already marked TEMPORARY — the hex gate is otherwise clean.


## MODELING-REACH (2026-08-01, commits c960183→be4beb4) — datum planes, measure, view UX, arc-endpoint welds
Plan `~/.claude/plans/do-thorough-exploration-and-rosy-lollipop.md` (adversarial
review REVISE → 2 MAJOR + 3 MINOR folded pre-approval). Five gates, all 4-suite
green vs real worker.
**W0a (c960183)**: hint-clobber 8-site class → `resetToSelect` (20 sites, hints
finally visible); stale static layer after sketch-switch → local geometryToken
bump; Equal/Midpoint real-worker DOF proofs (all 7 kinds now proven).
**W0b (7bc73a5)**: arc endpoints are REAL solver points — worker mints
`.start`/`.end` handles coupled by PlaneGCS ArcRules (tag-0,
redundancy-invisible), Rust Coincident gains optional positions (frozen serde
byte-stable), slot caps welded to walls. SCHEMA §7.3 positions + §7.4 e3.start
examples became TRUE. Findings: entity-Tangent degenerate once welded (dropped
from slot, DOF 9 honest; endpoint-tangency kind backlogged); latent
constraint-free drag TELEPORT fixed; dup wire center points confirmed (+4 DOF,
pre-existing, backlogged).
**W1 (3a210cf)**: datum offset planes — D tool (PlanePicker + offset ghost),
core resolves frames AT CREATION (frozen, Rust = basis authority, stamps
datum-attached sketch planes), DatumLayer + tree section + sketch-on-datum both
clients, DeleteDatum referenced-guard. Worker + protocol byte-identical. Proof:
extrude off an XY+10-hosted sketch → bbox z=[10,15] exact; legacy-swapped bases
pinned (XZ offset moves world +X).
**W2 (d1b151b)**: measure tool (?) — exact kernel face area / edge length +
center↔center distance (bbox centers, labeled honestly); LATENT FOUND+ROUTED:
element-map entries mint on demand so promoted-but-unused ids are absent → new
`query_element_by_topo_key` ladder rung; **`face_sketch_plane` has the SAME
latent (flagged follow-up)**. Units: `src/units/format.ts` single seam,
mm/cm/m/in input parsing, valueText round-trips guarded both directions; `?`
inert while sketching (NO_CROSS_MODE keymap set).
**W3 (be4beb4)**: display-mode button REAL (was dead — renderer never read it;
default shadedEdges), ⇧F fits selection (explicit visible-filter —
Box3.expandByObject recurses invisible children), ⇧I transient isolate (never
persisted; preview-stomp guarded both directions).
Suites at final gate: tsc 0 · FE 1893/138 · cargo 570/0 vs real worker · ctest
77/77 · e2e 90/90 · clippy/fmt/hex clean. 8 implementation agents, every diff
reviewed + every gate re-verified by orchestrator.
BACKLOGGED THIS WAVE: endpoint-tangency constraint kind; wire center-point
dedupe (worker honoring centerRef); face_sketch_plane topoKey rung;
solved-arc-endpoint/radius FE write-back; datum rename/visibility +
OffsetFromFace/AngledFromEdge; chained-datum delete guard if resolution ever
re-derives.
REMAINING: USER manual Tauri gates (TODO.md MODELING-REACH checklist + older
SKETCH-POWER/TRUST/PREVIEW ones); Codex post-hoc reviews after 2026-08-05.


## SKETCH-POWER (2026-07-31→08-01, commits 130854b→b191ed2) — sketch expressiveness wave
Plan `~/.claude/plans/do-thorough-exploration-and-rosy-lollipop.md` (internal
adversarial review REVISE → 2 MAJOR + 2 MINOR folded pre-approval; Codex gate
dead until Aug 5). Four waves, 5 gate commits, every gate 4-suite green vs real
worker.
**W0 latents (130854b, all red-first)**: tree sketch-switch while IN sketch mode
retargets the controller (was silent wrong-sketch writes; self-switch guard vs
open session + openSession-echo bracket — naive subscription self-switched every
entry); `prepare_sketch_regions` refuses during same-sketch drag +
`finish_sketch` clears the dangling gesture (Enter-mid-drag race); Alt
pick-through (face under coplanar sketch fill selectable, default path
byte-identical); `getSketch` backend-id resolve; history icons opType-first.
**W1 construction geometry (c2566e8 + 27db309)**: worker `WireSketch` now READS
the flag Rust always emitted (hardcoded false before — LoopDetector's filters
were dead code; an all-construction rect published a region+extruded, ctest bug
pin); SCHEMA §7.3/§7.4 documented, no fixture bump (remaining region ids
byte-stable, runtime-baseline-pinned); core `SetEntityConstruction` (memento
inverse free, squash-safe); FE: X = selection flip (mixed rule
`!every(construction)`) / sticky draw mode + chrome button; `marshalUpsert` flip
branch over a last-SENT cache (flip emitted ZERO ops before); construction
centerline as revolve axis pinned both lanes.
**W2 tools batch (2626e46, FE-only verified)**: Tangent/Equal/Midpoint
user-apply (solver-bounded matrix; Midpoint gated on `marshalsAsPoint` — arc
endpoints would marshal null + silently drop; dups → existing
reject-on-conflict); point (P) / centerRect (⇧R) / slot (S) / polygon (G,
digits 3-9, construction circumcircle, DOF 4 any n) via new `ToolConstraintSpec`
(tool-authored constraints, intra-batch inference suppressed); slot ships
Tangent×4+Equal ONLY (arc-endpoint Coincidents unmappable — real-lane DOF ≈13
documented, mock e2e labeled); marquee box select (rightward=window true
containment / leftward=crossing touch — the semantics legacy UI promised but its
findInRect never had; 8 teardown paths restore LMB orbit; plane-AABB =
conservative superset off-normal).
**W3 ellipse (b191ed2, protocol change)**: SCHEMA un-UNSUPPORTs Ellipse w/
normative normalization-echo; ONE WireSketch branch unlocks both lanes (5
red-first fixtures failed "unsupported entity type" incl. the plan-profile lane);
solver-free legacy parity (naive DOF, PlaneGCS registration deliberately
skipped — vendored GCS has full ellipse support, backlogged); true Geom_Ellipse
extrude (vol 0.19% off π·a·b·h); FE 3-click tool (key O — e is the extrude
handoff) w/ live swap-normalization, applicability BAILS on ellipse targets
(oracle-parity), trim=whole-delete, mirror=copy-only.
Suites at final gate: tsc 0 · FE 1660/126 · ctest 76/76 · cargo 547/0 vs real
worker · e2e 76/76 · clippy/fmt/hex clean. Delegation: 9 implementation agents
(3 parallel max), every diff orchestrator-reviewed, every gate
orchestrator-re-verified.
LATENTS FLAGGED (accepted): `sketchStaticSync` A→B switch relies on real-lane
geometryToken bump (mock may show stale until exit); `enterRegionPick`
hint-clobber (pre-existing — "No closed region" never visible, setTool clears
it); FE full-suite exit-1-with-all-passing observed once (teardown flake class);
`bbox_dims` exactness tests inflate ~0.35mm (BRepBndLib gap).
REMAINING: USER manual Tauri gate (TODO.md SKETCH-POWER checklist) + the older
TRUST/PREVIEW/AUTO-MODE manual gates still open.


## SKETCH-MULTI-OBJECT (2026-07-31, commit 267af13) — re-entry deleted prior objects
USER-REPORTED: rect + disconnected line in one sketch → only latest survived
Finish. Root cause: `seedIdMapFromWire` MERGED the enter_sketch wire into the
per-sketch id-map instead of REBASING — stale frontend ids from the previous
session made `marshalUpsert`'s removals-first diff emit `removeEntity` for every
previously drawn object's backend uuid on the FIRST edit after re-entry (12 ops
for a rect); viewport rendered from the untouched local session so loss showed
only after Finish. Fix: clear entity/point/constraint/constraintValue before
seeding (backendSketchId/planeKind survive). Red-first proven; pins in 4 lanes
(sketchWireMap unit, sketchMultiObject full-stack vitest, e2e, real-worker
sketch_multi_object.rs asserting document sketch + timeline record + re-entry
union). Suites: FE 1407/119 · e2e 61/61 · cargo workspace green · clean.
ADJACENT LATENT (flagged, unfixed): tauriClient.getSketch passes raw frontend id
(siblings resolve backendSketchId); tree sketch-switch while already in sketch
mode is a controller no-op (chrome points at new sketch, controller edits old).

## TRUST + PREVIEW (2026-07-31) — silent-wrong-behavior class killed + all-op kernel preview
Two waves in one day (plan `mossy-foraging-muffin.md`, internal adversarial
review substituting the Codex gate; dual review REVISE → all 5 blockers +
16 majors across both cycles fixed red-first).
**TRUST (commit 49089bc)**: Suppression was geometrically INERT (record flag
never reached StepState; deeper: `OperationRecord::outputs` never populated in
production → dependency graph had ZERO body edges → cascade AND anti-time-travel
validation silently dead) — now real: one predicate (records), hash filter
(repairs latent checkpoint mismatch), cascade-on-suppress-only, all-suppressed
publishes a Clear, executed-scoped outputs sync (checkpoint regen must not wipe
the prefix). Body name/visible durable (adopt+overlay+save-merge; DeleteBody on
timeline bodies rejected; suppressed body leaves the tree). Revolve got extrude's
parity: stored-profile re-edit (exact regionId, volume-proven), pure-read arm,
commit-boundary record guarantee (NEGATIVE tripwire vs real worker), stored-axis
restore, geometryToken cancel. Pattern/Mirror/Shell re-edit was DEAD on the real
lane (kind-guards vs folded FeatureKind; mock emitted nonexistent kinds — the
postmortem false-green class) — opType-gated now, sourced from stored params,
loud refusals. Boolean op-swap re-edit. Tree: context menu, F2 inline rename,
backend-backed visibility over a no-awaiter metadata transport (sketch delete
excluded — ToEnd). Optimistic suppress overlay DELETED (un-suppress after reopen
was impossible: `!undefined→true` lock). Unsaved-changes guard on every close
path incl. start screen + ⌘Q (ExitGuard self-healing; Don't-Save-quit no longer
resurrects discarded work as crash recovery). statusMessage no longer dropped by
history actions; doc-lifecycle store resets; trace.ts dev/?trace-gated.
**PREVIEW (this tree)**: PreviewOp exercised by ALL op types — one shared
builder table (previewOps.ts), each builder fixture-pinned byte-equal to its
commit call-site; controller session infra generalized behavior-preserving
(extrude parity pins untouched, e2e 50/50 before wiring). Revolve previews the
real kernel result during drag (Cut actually subtracts); Fillet/Chamfer/Shell
moved to armed-commit gesture (Enter/✓/Esc; NO click-away — armed tools claim
every press as a value drag) with kernel preview and OCCT-refusal BLOCKING ✓
with the named reason; Boolean previews the fused/cut candidate with both
sources hidden. preview==commit proven per op vs real worker (revolve 2352.411 /
cut 9502.555 / fillet 19981.229 / chamfer 19960 / shell 2224 / union 30000 /
cut 10000 — all exact), head fingerprint + revision untouched by every preview.
Latent bugs killed en route: trailing-throttle >90ms froze coalesced previews;
progress hint buried error hints; test-fixture record-plane≠document-plane
diverged preview vs commit. SCHEMA §14 doc-only entry (no wire change).
Suites at Wave-2 gate: FE 1404+ · cargo workspace green vs real worker (+~35
tests: suppression 8, body_metadata 7, preview_revolve 2, preview_edge_shell 4,
preview_boolean 2, revolve_ops 8, dto/wire pins) · ctest test_preview_op 14
cases · e2e 60/60 · clippy/fmt/tsc/hex clean.
REMAINING: USER manual Tauri gates (TODO.md — Wave 1 + Wave 2 checklists, plus
the older AUTO-MODE/EXTRUDE-COMMIT-FIX ones); preview p95 latency budget
unpinned; worker-side preview coalescing deferred (cancel groundwork in ctest).

## EXTRUDE-COMMIT-FIX (2026-07-30) — select+drag worked, apply silently failed
User-reported: region select + drag preview OK, committing the extrusion did
NOTHING. Root cause (two halves): (1) the interactive flow never authored a
sketch's `Sketch` TIMELINE record — only tests did — so the regen planner could
not resolve any modeling op's profile and every extrude commit failed "profile
sketch not found in plan" (autosave forensics: 20 Extrude records, 0 Sketch
records; each failed ✓ stacked a duplicate errored record). (2) documents saved
BEFORE the half-1 fix carry no records either, so reopening stayed broken.
Shipped (all in the working tree's AUTO-MODE batch + this session):
`finish_sketch` mints/refreshes the record (+ outcome → scheduler, feature row),
failed commits roll back their errored record (no more retry stacking), and NEW
this session — `from_document` BACKFILLS missing Sketch records at the timeline
front on open/recover (cursor shifted, in-memory until next save, fixed-point
proven). Real-worker repro: legacy container (sketch, zero records) → open →
pure region read → exact-region `at_cursor:false` commit → publishes 1 body;
red-first proven (kill-switch run fails). Suites: cargo workspace green vs real
worker (scheduler_commit 6/6 incl. both repros, m2_gate, wire_contract,
topology_rebind, sketch_squash) · clippy/fmt clean · tsc/build clean · FE
1232/1232 (stale StartScreen marker updated for the deleted mode toggle) ·
e2e 38/38. REMAINING: user manual Tauri gate — reopen the broken document,
✓-commit an extrude, then delete the legacy stacked errored rows.

# OneCAD-Tauri — Current State (2026-07-29, AUTO-MODE shipped)

## AUTO-MODE (2026-07-29) — tool+context-driven mode switching; titlebar toggle DELETED
Mode is now derived intent: `toolStore.mode` remains the single state (all
consumers untouched), but only tools/context set it — never the user. New
`src/tools/activateTool.ts` dispatcher (toolbar + shortcuts both route through
it): a sketch-only tool from model mode enters sketch mode WITH that tool
preserved (`setMode` gained `opts.tool`); a model-only tool from sketch mode
finishes through the SAME drained-squash path as Enter (one undo step) and arms
the tool — Extrude rides the existing pendingExtrude region-pick handoff
byte-identical. keymap `resolveBinding` gained a cross-mode fallback (tool
actions only — L in model mode starts a sketch with Line; shared letters
R/C/M/H stay context-local; Delete can't cross). Viewport double-click on a
static sketch (model mode + select tool) re-enters its edit session, mirroring
the tree. Design approved via 4 user decisions; an auto-arm-on-single-region
prototype was CUT after e2e exposed it breaking the pinned multi-region
rejection flow (picking is select-tool-gated; an early arm locks selection and
turns region clicks into click-away commits). Suites: tsc 0 · FE 1230/109 ·
build clean · e2e 38/38 (+3 auto-mode) · hex 1 pre-existing (inputProbe).
REMAINING: user manual Tauri gate (TODO.md AUTO-MODE checklist).

# OneCAD-Tauri — Current State (2026-07-29, EXTRUDE-REGION-PARITY shipped)

## EXTRUDE-REGION-PARITY (2026-07-29) — exact selected profile, preview == commit
User-reported: extrude previewed/committed the WRONG sketch region and was
preview-only. Root cause: the live Tauri preview bypassed Rust's typed operation
lowering — the frontend sent nested `params.profile.regionId`, C++ expected flat
`params.regionId`, received none, and extruded the FIRST region; the commit used
the correct mapper, so preview and commit diverged. Implemented across a Codex
CLI session (P1 worker + P2 Rust + most of P3 FE; died on its usage limit
mid-hardening) and a same-day Claude continuation (P3 gaps, adversarial-review
defects, proofs, gate).
**Architecture**: one worker-authoritative `RegionTable` per solved sketch feeds
BOTH `SketchRegions` publication and modeling profile lookup — nested cells
(annulus + inner disk) and intersection fragments are independently selectable,
holes participate in region identity (`cell-v2` canonical signature under the
stable `r_<16hex>` wire shape; legacy outer-only ids resolve only when unique,
else loud failure listing candidates). `PreviewOp` consumes the SAME canonical
worker operation as `ExecutePlan` via one shared Rust lowering; typed
`sketchRegion` selection + full-FSM param snapshots (stable opId = commit
recordId) close the frontend end. `geometryToken` invalidates stale profiles
across undo/reopen — including a NEW proactive cancel of an armed extrude when
its sketch is edited underneath.
**Continuation hardening**: commit BARRIER — confirm flushes the final params as
the newest preview epoch and holds `endPreview(true)` until that exact candidate
answers (failure → re-armed, work kept; 4s timeout proceeds, backend re-validates
authoritatively). Partial ear-clip now fails `SketchRegions` closed (exact
`loop−2` triangle-count law) instead of publishing incomplete material. `ToNext`
casts rays from profile vertices + centroid against BOUNDED faces
(`IntCurvesFace_ShapeIntersector`) — the legacy nearest-ray-PLANE rule could bind
a face plane the profile never crosses. `STALE_PREVIEW` structured error code
replaces message-text sniffing (SCHEMA §8). `opType` reaches FeatureMeta on the
real lane (Chamfer re-edit opened the FILLET editor). Document-scoped UI resets
on new/open/import/recover when replacing an open document.
**Proofs** (real worker): inner disk by exact id — preview == commit volume ±1
(≈π·25·7); annulus binds independently; save → FRESH worker reopen → identical
hash chain + identical region-id set from the read-only query; ToNext
laterally-missed pillar FAILS loudly, nearer-missed-face is skipped (binds z=8
not z=5, vol 800 exact). e2e: commit-barrier epoch equality, multiregion,
hole-extrude, booleans.
Suites at gate (2026-07-29): ctest 70/70 · cargo 486/0 vs real worker ·
clippy/fmt clean · tsc 0 · FE 1205/107 · build · e2e 35/35 · hex 1 pre-existing
(inputProbe). REMAINING: user
manual Tauri gate (TODO.md); backlog — revolve region-parity (+ reopened-sketch
revolve is broken on the real lane), analytic fragment wires (chord V1
limitation), `PreparedSketchRegions` vs live-gesture race, coplanar-fill pick
precedence, `historyActions` statusMessage drop.

## MODEL-OPS (2026-07-26) — sketch-based modeling correctness + breadth, 4 waves
Goal: make the sketch-driven feature set correct and reachable. The backend was
far ahead of the frontend, and one shipped behaviour was silently wrong.
**W0 profile correctness**: `SolverLane` ear-clipped a region's OUTER loop only
while `FaceBuilder` builds the face WITH hole wires — a rect+inner-circle
previewed as a slab and committed as a tube, and a click INSIDE a hole selected
the region. New `loop/PolygonFill.{h,cpp}`: bridge-merged holes with SHARED
vertex indices so bridges stay interior — load-bearing, because the frontend
recovers extrusion rings from single-use-edge topology. FE `PrismProfile.holes` +
inner walls; mock `detectRegions` learned the worker's containment rule (so the
e2e lane can even see a tube). SCHEMA §7.3 profile-binding prose corrected — it
documented an `inputs[]` semantic ref **no layer has ever produced or consumed**.
**W1 extrude end conditions + Chamfer**: the worker has implemented
ThroughAll/ToNext/ToFace + two-direction + draft since W-WP6 and the wire carried
the fields; the tool authored only Blind/Symmetric. Chamfer was absent from BOTH
the `ModelTool` and authorable `WireOperation` unions despite a shipping
`execute_chamfer`. Fixed, plus two latent defects: `default_label` keyed off the
coarse `FeatureKind` bucket (a Chamfer read "Fillet", a pattern read "Boolean"),
and re-edits routed on `kind`, so Chamfer opened the fillet editor and
Shell/patterns/Mirror were unreachable on the real lane.
**W2 sketch-on-face**: `SketchAttachment::HostFace` was typed in Rust AND C++
since M1 with ZERO constructors — every sketch that ever reached the worker was a
world plane. No worker change was needed (`parse_plane` always accepted a custom
basis); it needed a producer. `QueryElement` got its first Rust caller ever
(`ElementQuery` seam → `face_sketch_plane`, which refuses a non-planar face), and
`plane_from_point_normal` is lock-tested because the frame is frozen with the
sketch. Datum resolution (`resolve_datum`) landed pure + tested; datum
CREATION UI did not.
**W3 backend preview verb**: the drag-time "exact" mesh was synthesized in
JavaScript by the same function the mock uses, so Cut never subtracted. New
kernel-lane `PreviewOp` runs the candidate op through the same executor a plan
step uses, over a throwaway head copy — invisible to fencing (no fence, no
prepare, no scratch). **Proven: preview Cut = 7500 while the real body stays
8000, and committing the same op lands on 7500.**
Suites: FE 1162/105 · cargo 461/0 vs real worker · ctest 69/69 · e2e 32/32 ·
clippy/fmt/hex clean. OUT OF SCOPE by user decision: Loft/Sweep.
REMAINING: preview latency gate, worker-side preview coalescing, Fillet/Shell/
Revolve preview sessions, datum creation UI, user manual Mac gate (TODO.md).

# OneCAD-Tauri — MODEL-HARDEN (2026-07-22)

## MODEL-HARDEN (2026-07-22) — Extrude/Revolve commit fix + professional UX, 6 gates
Root-caused USER-REPORTED "extrude preview vanishes on tool close": append-at-end
AddOperation returned RegenHint::None and the planner clamps to the applied cursor
prefix — **no fresh op commit EVER regened in the real app** (HISTORY row real,
geometry never computed, silent teardown; every UI suite runs the mock lane, every
integration test drove regen explicitly — new coverage class closes this forever).
Plan Codex-reviewed (terra/high, "revise" → all 10 findings folded). W0: frontier
append joins applied prefix (core cursor promotion; redo-draft regression pinned),
production-driver test seam (regen_driver_with_emitter) + scheduler_commit.rs vs
real worker. W0.5: squash all-or-nothing guard (stray finish_sketch can't pop a
model op's undo entry), model-mode arms → pure getSketch, regen completion carries
sourceRevision (exact per-commit correlation under rapid commits; superseded
ignored), FeatureDto.statusMessage → HistoryList tint+tooltip, legacy-draft open
hint. W1: armed-commit gesture — release keeps tool ARMED w/ live preview + chip
cluster [value|⇔|✓|✕]; Enter / ✓ / click-away commits, Esc cancels; failed commits
re-arm + name the reason. W2: boolean picker [New Body|Add|Cut] (auto-target or
click-to-pick, Cut tint), multi-select regions → N sequential ops (stop-on-first-
failure, previews kept), sketch auto-hide on success; revolve parity (N-op loop,
all-regions axis validity; quickCommit → click-away 360°). W3: worker RevolveOp
boolean split-children parity (publish_boolean_result; SCHEMA §7.2/§14 signed off,
no fixture bump) + revolve_ops.rs — FIRST real-worker Revolve wire coverage
(Pappus, 180°, Cut split adoption, stale-axis loud, region binding).
Suites: FE 1108/105 · cargo 423/0 vs real worker · ctest 66/66 · e2e 29/29 ·
clippy/fmt/hex clean. Commits d190d6a→(final). REMAINING: user manual Mac gate —
TODO.md MODEL-HARDEN checklist.


## SKETCH-HARDEN (2026-07-21/22) — sketch production hardening + UX, 6 waves, all gates passed
Plan Codex-reviewed (terra/high, "revise" → all findings folded). W0: Rust
squash-at-finish txn (whole sketch session = ONE model undo step), FE mutation
coordinator (generation fencing, transactional id-map, finish flush barrier),
sketch-scoped ⌘Z (snapshot stack, coalesced dim edits), dimension validation
(FE+Rust, H/V-Distance stays signed), zoom-normalized degeneracy guards,
dispose parity. W1 (protocol, signed off): per-constraint conflict ids on every
solve surface incl. session re-enter — rows/badges tint, reject hints name the
clashing constraint; fixed latent solveDrag raw-UUID bug. W2: Finish≠Cancel,
statusHint severity/sticky/4s auto-dismiss (all ~76 sites classified), line
chain Enter-end + click-first-close, formatDimensionValue 3dp, badge stagger,
glyph single-source, bodies dim 0.35 in sketch, crosshair cursors, mirror
hover. W3 (reviewed, APPROVE): REAL parametric trim (param-space crossings,
doomed-piece destructive hover ghost, whole-delete fallback). W4: snap
candidate cache (O(n²) intersections once per edit, equivalence-proven).
W5: draw-path integration tests, +5 e2e specs (23 total), angleUnits tests,
flake root-cause fix, stale-comment/debt cleanup.
Suites: FE 1065 vitest, Rust 406 vs real worker, ctest 65, e2e 23/23, clippy/
fmt/hex clean. Commits f8deef1→d619f53 (5 wave gates).
REMAINING: user manual Mac gate — TODO.md SKETCH-HARDEN checklist.

## AC-USABILITY (2026-07-20) — sketch/extrude usability gap-closure, 3 waves, all gates passed
Frontend interaction layer completed: sketch entity selection+hover, select tool
w/ point-handle drag via the real gesture lane, entity+constraint deletion
(Delete key + per-row ConstraintList), user-applicable constraints (toolbar pill
+ context chips, applicability matrix ported from C++), Trim + Mirror tools,
multi-region extrude/revolve pick. Five cross-layer bugs fixed: Tangent
wire-dropped, angle deg/rad (3 sites), OverConstrained never emitted,
orphan points on delete, re-entry representation split (centerRef ownership).
Plus: extrude auto-arm ordering, real-worker drag-preview reverse map, e2e port
collision. New proofs: sketch_reentry/sketch_edit/sketch_constraints real-worker
gates, hole-extrude volume ctest, Playwright e2e 6 specs (harness net-new).
Suites: FE 832 vitest, Rust workspace green vs real worker, ctest 65, e2e 6/6.
REMAINING: user manual Mac gate (`npm run tauri dev` — TODO.md checklist).

Non-destructive migration of OneCAD-CPP (~69k LOC C++20 Qt6+OCCT) into a 4-layer
Tauri app per NEW_SPEC.md. Tracker: `TODO.md` (per-WP gates + flags). OneCAD-CPP
stays untouched.

## Milestone status

| Milestone | Status |
|---|---|
| M0 foundations (protocol contract, scaffolds, corpus) | DONE |
| M1 cores (Rust document/history/regen/io · C++ worker OCCT+solver · frontend slice) | DONE |
| M2 first micro-slice integration gate | **PASS** (`m2_gate.rs` vs real worker) |
| M2-R implementation review | DONE — systemic BodyId wire-form defect found+fixed, `wire_contract.rs` regression gate, independent review APPROVE |
| M3 packaging gate | Linux portion DONE (externalBin deb-verified, path chain, bundle-dylibs.sh, PACKAGING.md); **Mac-side verification DEFERRED** (checklist §5) |
| FX file/app UX | DONE — save/open/recents/STEP-export UI, worker-status, live constraints |
| **M4 topology slice (backend + repair UI)** | **DONE, review-closed** (H5-B proven vs real worker) |
| **M5 lifecycle** (Revolve tool, STL/OBJ, checkpoints, splits, autosave+recovery, onecad-regen CLI, crash drills, drift gate) | **DONE** |
| M6 hardening + backlog | Shell+Patterns+Mirror + sketch parity (snaps, autoconstrain, Dimension) DONE; remaining: datum, Loft/Sweep, Playwright e2e, perf |

## What works end-to-end (real worker, automated gates)
Sketch (PlaneGCS, dof) → regions → extrude (multi-region by normative FNV id;
stale id fails loudly, never a silent wrong profile) → booleans/pocket/ToFace
(wire_contract volumes exact) → MESH1 → pick → ElementId promotion (stable,
Invariant 1) → fillet via scored ladder → **parametric edit → auto-rebind
(fillet survives) or deterministic NeedsRepair — the H5-B fix the corpus
documents as the legacy app's unfixed defect** (`topology_rebind.rs`) → repair
UI (banner → panel → score-ranked candidates → click-to-rebind via
promote + EditOperationInput) → save v2 container → reopen → deterministic
replay → STEP export → undo. Plus: Revolve tool (axis-pick + angle drag +
lathe preview), file menu (⌘O/⌘S/⇧⌘S/Export STEP), recents, worker-status
surfacing, history suppress/roll/delete affordances, solver-position
hydration on sketch re-entry.

## Suites (all green, orchestrator-verified)
- Worker: 61/61 ctests (OCCT 7.9.3; breadth ops m6a_ops incl.)
- Rust: 379 tests, clippy -D warnings + fmt clean (chaos 14, real-worker 5,
  m2_gate 2, wire_contract 8, topology_rebind 5, breadth_ops 6, checkpoints, regen CLI — vs real binary;
  ONECAD_REQUIRE_WORKER=1 guard in CI prevents vacuous greens)
- Frontend: 559 vitest, build green, hex-token gate 0

## Architecture decisions log (D-series + session additions)
- D1: NewBody BodyIds worker-minted `body_<opId>`, Rust adopts+fences
- D2: STEP export in worker · D3: `primary.topoKey` removed · D4: fencing =
  workerEpoch+expectedBaseHash only · D5: from-0 plans always base-valid
- Hash authority: Rust sole; planner hash decoupled from wire form (golden-pinned)
- Wire body form: ALL body-bearing params render `body_<uuid>` at the wire layer
  (core serde frozen); `intent` subtrees round-trip verbatim (never rewritten)
- Region binding: non-empty regionId MUST match (OP_FAILED naming available ids);
  empty = first-region V1 fallback; legacy ids sanitized on load (migrate diagnostic)
- AutoBind: elementId slot carries the Rust-minted id; topoKey is evidence (§9)
- Ladder policy: auto-bind ≥0.85 AND margin ≥0.10; symmetric tie ⇒ NeedsRepair;
  a fillet consumes its edge — re-resolving it NeedsRepairs (autoBind there = mis-bind)

## Key flags / known gaps
- Mac packaging verification (signing/notarization/bundle-dylibs first run) — needs a Mac
- L2 exact preview still local (no backend preview verb); revolve L1 only
- Checkpoints live (save-on-explicit-save policy, in-session restore V1; restore-fallback D1 edge flagged in TODO)
- Autosave+recovery live (30s debounce, startup-only recovery V1); onecad-regen CLI in CI
- Repair UI seams: resolveRefs sends refId-only; >1-body operated-body derivation;
  candidate viewport highlight = data seam; suppressed flag = optimistic overlay
- STEP import stub; Loft/Sweep UNSUPPORTED at worker (Shell/Patterns/Mirror LIVE end-to-end)
- Env note: Linux dev container uses conda-forge OCCT 7.9.3 at /opt/occt793
  (apt 7.6.3 too old); CI = macos-14 + Homebrew

## Conventions
- Orchestrator (Fable) designs/briefs/reviews; WPs → Opus subagents; RISKY WPs
  get independent adversarial review; commits at gate boundaries only
- protocol/SCHEMA changes need sign-off + §14 changelog (+ fixture bump if shapes move)
- Worker binary for tests: `ONECAD_WORKER_PATH=worker/build/onecad-worker`
