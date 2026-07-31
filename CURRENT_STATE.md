# OneCAD-Tauri — Current State (2026-07-31, TRUST + PREVIEW waves + multi-object sketch fix shipped)

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
