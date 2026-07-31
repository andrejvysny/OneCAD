# Handoff — TRUST + PREVIEW waves (make OneCAD dependable)

Session 1 · 2026-07-31

## Goal

Make the app **usable**: kill the silent-wrong-behavior defect class (things that
look like they worked but didn't, or bound the wrong target) and the
commit-blind ops (no preview before commit). Chosen from a full-repo state
analysis as the highest-value path; sketch-workflow breadth (P3 of the roadmap)
deliberately deferred.

## Original plan

`~/.claude/plans/mossy-foraging-muffin.md` (approved 2026-07-31). Two waves +
extras; internal adversarial review substituted for the Codex gate (Codex CLI
limit until Aug 5 — see project memory). Plan file records all user decisions
(all-5-op preview; fillet/chamfer/shell armed-commit WITHOUT click-away;
suppression hash-filter + cascade-on-suppress-only; no body Delete in tree;
T0/T1 Rust prerequisites in scope).

## Done so far (and why)

All committed, tree clean, `master` (no remote push — never pushed by convention):

- `335ddb3` — baseline commit of the previously-uncommitted AUTO-MODE +
  EXTRUDE-COMMIT-FIX part 2 batch (user chose commit-now, manual-gate-later).
- `49089bc` — **Wave 1 TRUST**: suppression made geometrically real (was fully
  inert; deeper find — `OperationRecord::outputs` never populated in production,
  so the dependency graph had ZERO body edges: cascade and anti-time-travel
  validation were both dead); body name/visible durable across regen+save;
  revolve given extrude's region parity incl. the commit-boundary record
  guarantee (negative tripwire vs real worker — the postmortem class);
  pattern/mirror/shell re-edit was DEAD on the real lane (kind-guards vs folded
  FeatureKind, mock emitted nonexistent kinds) — opType-gated now; boolean
  op-swap re-edit; tree context menu/F2 rename/backend visibility over a
  no-awaiter metadata transport; optimistic suppress overlay deleted
  (un-suppress after reopen was impossible); unsaved-changes guard on every
  close path incl. start screen + ⌘Q.
- `c2c30a5` — **Wave 2 PREVIEW**: one shared builder table (`previewOps.ts`)
  fixture-pinned byte-equal to each commit call-site (SCHEMA §7.6
  no-second-mapper); kernel preview for Revolve (Cut subtracts during drag),
  Fillet/Chamfer/Shell (moved to armed-commit gesture; OCCT refusal BLOCKS ✓
  with the named reason), Boolean (candidate shown, both sources hidden).
  preview==commit proven per op vs real worker, volumes exact, head+revision
  untouched. Latent fixes en route: trailing throttle >90ms froze coalesced
  previews; progress hints buried error hints.
- `267af13` — **SKETCH-MULTI-OBJECT fix** (user-reported after the waves):
  re-entering a sketch deleted previously drawn objects — `seedIdMapFromWire`
  merged instead of rebased, stale ids marshalled as removals. 4-lane pins.

Process facts worth keeping: dual internal adversarial reviews found 5 BLOCKERs
+ 16 MAJORs across both cycles — all fixed red-first; every wave gate ran the
full 4-suite matrix vs the real worker. Parallel subagents MUST be forbidden
from `git stash` (one swept the shared tree — see project memory
`parallel-subagents-no-git-stash`).

Dead-ends ruled out (don't revisit): click-away commit for fillet/chamfer/shell
(armed tool claims every left press as a value drag — needs a real handle
hit-test first); worker-side ExecutePlan of an empty all-suppressed plan (worker
short-circuits — the Rust-side Clear publish through the normal accept path is
the implemented design); mock revolve preview synthesis (axis unavailable in the
lane session); e2e asserts of preview geometry for fillet/shell/boolean (mock
has no CSG — real-worker tests are the guards).

## How to resume

1. Run the `handoff` skill with "resume".
2. Read `CURRENT_STATE.md` top two sections + `TODO.md` "TRUST + PREVIEW waves"
   section — both current and authoritative.
3. Verify: `bun run test` · `bun run e2e` · from `src-tauri/`:
   `ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 cargo test --workspace`
   (stage worker first if missing: `scripts/build-worker.sh Release`).
4. The immediate blocker is USER-side: manual Tauri gates (`bun run tauri dev`)
   — Wave 1 + Wave 2 checklists in TODO.md, plus the older
   AUTO-MODE/EXTRUDE-COMMIT-FIX ones and the multi-object re-entry check.

## Open questions

- Next implementation wave: roadmap P3 (sketch workflow: construction toggle,
  Tangent/Equal/Midpoint user-apply, marquee select, point tool, shape variants,
  offset) vs backlog items (units layer, measure, datum UI, STEP import) — user
  call after manual gates pass.
- Post-hoc Codex review of both waves once the limit resets (after 2026-08-05).
- Flagged latents to schedule: `tauriClient.getSketch` raw-id seam; tree
  sketch-switch no-op while in sketch mode; preview p95 latency budget in
  solverbench; worker-side preview coalescing (cancel groundwork = ctest case 14).

## Pointers

- Tasks → TODO.md ("TRUST + PREVIEW waves" + "SKETCH-MULTI-OBJECT" sections)
- Snapshot → CURRENT_STATE.md (top 3 sections cover this session)
- Plan → `~/.claude/plans/mossy-foraging-muffin.md`
- Postmortem discipline → docs/POSTMORTEM_EXTRUDE_COMMIT.md
