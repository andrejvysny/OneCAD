# OneCAD-Tauri — Current State (2026-07-22, MODEL-HARDEN shipped)

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
