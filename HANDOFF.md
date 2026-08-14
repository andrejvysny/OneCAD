# Handoff — Modeling UX unification (the `UX/` audit + hardening spec)

Session 9 · 2026-08-14

> **SEVEN THREADS IN THIS FILE.** Session 9 (this one) closed the program: it ran
> the two browser lanes session 8 owed and committed the tranche. Session 8, which
> implemented the whole delta program, follows immediately below — read it for the
> reasoning, and read `TODO.md` § MODELING UX UNIFICATION beside it, that section
> is the per-package evidence ledger and the authority on what was measured.
> Session 7 and earlier are history.

## Session 9 — the two lanes, and the gate commit

### What it did

Session 8 ended with exactly one gate owed: U5's browser lanes. Both were run,
one at a time, retries 0:

| Lane | Result |
| --- | --- |
| `E2E_PORT=4191 bun run e2e -- --project=chromium` | **200/200**, 14.2 min |
| `E2E_PORT=4193 bun run e2e -- --project=webkit` | **200/200**, 8.6 min |

No failure in either, so no triage was needed, and neither MC-R8 nor MC-R9
reappeared. The U0–U7 tranche was then committed (**commit only, no push** — the
user's explicit call).

### Three corrections to session 8's state — the docs had drifted

1. **`master` was NOT "2 commits ahead, nothing pushed".** It is in sync with
   `origin/master` at `4ac8565`; `git reflog show origin/master` records pushes at
   2026-08-13 22:50 and 2026-08-14 08:20 / 09:22. Four commits landed after
   `dc4bd5e` (`9559b8f`, `19088d0`, `f9df6b7`, `4ac8565`), presumably from the
   session sharing this tree. **Check the reflog before quoting an ahead-count
   here** — a concurrent session makes that number stale within hours.
2. **MC-R8 is CLOSED, not carried.** `19088d0` root-caused it: the DEBOUNCED
   auto-fit a new body schedules moves the camera under `boolean-preview.spec.ts`'s
   unsettled screen-point probe, the click's ray then misses the body and the
   selection clears. Fix is one `waitForCameraSettled` in `findBodyScreenPoint`.
   It is NOT the projection-push race everyone (including session 7's note) had
   assumed.
3. **MC-R9 is therefore not "the same signature class".** `revolve-commit.spec.ts`
   already settles the camera at both probe sites (`:124`, `:130`), so MC-R8's
   mechanism cannot explain it. Still un-root-caused, still recorded, still not
   retried away.

## Session 9b — the data-integrity audit

Asked "what next", the direction chosen was data-integrity defects, scope **investigate first,
then decide**. That scope was the right call, because the two recorded defects were not open:
VF-M6 had been fixed on 2026-08-08 with the box never ticked, and the W5 promoted-id seam lost its
consumer in `1fe0cef`. **The ledger was wrong in both directions** — this repo has repeatedly
recorded closed-but-open (MC-R8, D1, D9, D14) and now open-but-closed. Treat an open box citing
`file:line` as a hypothesis until re-verified; line numbers drift and a fix landing in one package
never ticks another package's box.

So the audit went and looked instead. Eight probes, full table in `TODO.md` § DATA-INTEGRITY AUDIT.

**Safe, with the evidence that would have caught the bug:** crash recovery exists and is well built
· the container write is atomic and has a crash-simulation test · the import-blob carrier is
insert-only and the undo stack never persists, so the remove→save→undo sequence has no loss window
· a save snapshots under the runtime lock · unknown module state round-trips verbatim · every
`EditCommand` has a real inverse.

**Five findings, none fixed** — DI-1 (HIGH) a recovered document is unprotected against the next
crash, because recovery consumes the marker and discovery is marker-keyed, contradicting the
comment that says otherwise; DI-2 `recover_document` never ticks the autosave loop; DI-3
`promote_selection` / `prepare_edge_op` persist state with no tick and no dirty flag, so the close
prompt is skipped; DI-4 an authored face colour reopens as data but neither paint path can find its
face; DI-5 STEP export drops XCAF names and colours.

Two probes are committed as executable evidence rather than prose:
`src-tauri/tests/face_color_reopen.rs` (real worker, save → fresh worker → reopen, in-session
controls beside both measurements, assertion mutation-proved) and
`io::recovery::tests::an_autosave_whose_marker_was_consumed_is_not_offered`.

DI-4 and the W5 seam share one root — nothing re-binds a persisted `ElementId` at open — so one fix
closes both.

### How to resume

The UX program is complete and committed. The audit is complete and NOT committed. Next is a
product call, not a task:

0. **Decide which of DI-1…DI-5 to build.** DI-1 + DI-2 are the same half-day and are the only ones
   that lose real work. DI-4 is the interesting one architecturally (persisted-id re-binding).


1. **U8 (typed face/datum/axis references)** — queued since planning, never
   scheduled against the other roadmap tracks.
2. The manual Tauri smoke (spec §14) for this program still has no evidence and
   needs the native stack.
3. Nothing is pushed for the tranche commit. Say so before pushing; the tree is
   shared.

---

## Session 8 — the modeling UX unification program

### Goal

The user supplied two documents in `UX/` and asked for a thorough analysis, a plan,
and then "continue autonomously and fully implement plan and UX hardening":

- `onecad-modeling-ux-audit-interaction-system-roadmap.html` — a senior UX audit
  (2026-08-11). Verdict: the direct-modeling primitives are good, the interaction
  RULES are fragmented per tool. Nine ranked findings.
- `onecad-modeling-ux-implementation-hardening-specification-b022edf7.md` — a
  coding-agent brief pinned to commit `b022edf7`: 19 defects (D1–D19), nine work
  packages (WP0–WP8).

### The finding that shaped everything: the spec was 21 commits stale

HEAD is `4ac8565`; the spec is pinned at `b022edf7`. Executing it as written would
have redone closed work — and, worse, trusted three "closed" claims that
re-verification showed were only PARTLY closed:

- **D9/WP1** — `classifyRegen` existed and worked, but **11 call sites bypassed it**.
- **D1** — the ✕ was added; ✓ and Enter parity were not. Four tools had no Enter path
  at all while the frozen contract claimed `enterSupport: true` for all twelve.
- **D14** — the pure function was fixed; `InspectorPanel` still hardcoded
  `Under-constrained · DOF {dof}`, and its own test fixtures were DOF-3 only, which
  locked the bug in.

Plus six defects in neither document (boolean selection, position-derived body names,
the origin not being a reference at all, repair count vs rows, `OffsetFaceOp`'s
missing publication gate, two divergent screen-scale implementations).

So the approved plan (`~/.claude/plans/bright-munching-oasis.md`) is the re-verified
DELTA, not the spec. **If you are tempted to go back to the spec text, read the drift
ledger at the top of that plan first.**

### Approved decisions (taken with the user before any code)

1. Full U0–U7 scope; U8 (typed refs) queued.
2. The frozen contract gains `primaryEntry` + `livePreviewOnEdit` columns — a
   TIGHTENING, which the frozen-contract rule permits.
3. Pattern count: 2–12 stepper, typed entry to the worker's 128.
4. ✓/✕ everywhere; the accent `Apply` button is deleted.

### Done, and why (one line each — full evidence in TODO.md)

- **U0** — 32 reds, each paired with a green control so no red could be blamed on its
  fixture. Two new contract columns. Tests only.
- **U1** — all 11 bypassing sites classify; the re-edit correlation root cause turned
  out to need **no wire change** (`updateOperationParams` already carries the record
  id, and `failed_steps_of` keys on it). Found a real defect while doing it: the
  body-count fallback called every metadata-only command a failure, so
  `classifyRegen` gained an explicit `noTerminal` option.
- **U2** — `onApply` deleted, one table-driven Enter router, one refusal wording. Also
  fixed the sketch→Extrude handoff, which armed the tool and then reset it to Select;
  **the browser lane caught that, not the unit tests.**
- **U3** — `onPreview` + type-to-enter. Two real defects surfaced: the angle parser
  accepted `12abc`, and our own preview echoed back and ate the keystroke. The first
  echo guard was too broad and broke the unit re-label — **again caught by the lane.**
- **U4** — one OperationHUD frame + result summaries (`Cut · Body 1 survives · Body 2
  is consumed`). D6's union deferred a second time, deliberately.
- **U5** — three full 82px tori → compact ±26° arcs; new `getInteractionOverlayBounds`;
  the chip finally offsets clear of the widget; one screen-scale implementation.
- **U6** — unique body labels (Rust), one pattern-count range, `Total` label, boolean
  roles + Swap, ghost-fidelity disclosure.
- **U7** — one constraint-status authority, candidate-count truth, the sketch ORIGIN
  becomes a real snap tier that persists a `Fixed` when accepted, `OffsetFaceOp` runs
  the shared publication gate.

### Dead ends and corrections — do not redo these

- **Do not rank the origin snap above everything.** Tried it; it steals snaps from
  nearer endpoints and reds four `snapEngine` specs. It sits with QUADRANT.
- **Do not anchor the origin purely on coordinates.** Tried it; a face-hosted sketch
  then double-anchors (`sketchOnFace.test.ts` catches it) and geometry merely drawn at
  screen centre gets fixed. It is gated on `InferOptions.originAccepted`.
- **Do not put the rotation arcs in the negative quadrant.** Tried it; they end up
  behind the widget from the default iso camera and the arrows win the raycast.
- **Do not claim the edge-on ring ambiguity is fixed.** It is REDUCED (52° of arc
  instead of a full circle). Any coplanar overlay handle can still be crossed; the
  characterisation spec says so.
- **Do not run two e2e lanes concurrently.** Two vite servers fight over port 4177
  (strictPort) and the second run dies on `page.goto`. One lane at a time.

### How to resume

1. Run the `handoff` skill with "resume".
2. **First task: run U5's browser lanes.** Nothing else is owed.
   ```bash
   pkill -f playwright; pkill -f "vite --port"
   bun run e2e -- --project=chromium
   bun run e2e -- --project=webkit
   ```
   U5 changed gizmo geometry, screen scale and chip placement — the lane is the only
   real check for all three. If a gizmo spec fails, `e2e/modelToolHelpers.ts`
   `findGizmoHandle` brute-force-scans the canvas at 3px steps, so a changed handle
   footprint shows up there first.
3. If both lanes pass, the program is complete and ready for a gate commit. **The user
   has not authorized a commit** — ask.
4. The manual Tauri smoke (spec §14) has never been run for this program and needs a
   machine with the native stack.

### Open questions

- Is U8 (typed face/datum/axis references) queued straight after this, or behind other
  roadmap tracks?
- MC-R8 and the new MC-R9 are both un-root-caused full-suite-only nondeterminism. They
  are recorded, not retried away. *(Session 9: MC-R8 is in fact closed by `19088d0`;
  MC-R9 stands, and is not the same mechanism. See the session 9 block above.)*

### Pointers

- Plan → `~/.claude/plans/bright-munching-oasis.md` (drift ledger + per-package scope)
- Evidence ledger → `TODO.md` § MODELING UX UNIFICATION, one block per package
- Snapshot → `CURRENT_STATE.md` · Source documents → `UX/`

---

Session 7 · 2026-08-13

> **FIVE THREADS IN THIS FILE.** Session 6 is immediately below — it ran the
> mandated gate ladder over Session 5's uncommitted work and committed it — and
> Session 7 (the live one) sits inside it, at § "Session 7 — the two lanes": it ran
> both full browser lanes, closed MC-R7 and opened MC-R8. Read those two first.
> Session 5 follows with the roadmap plan and its decisions — read it
> next, it still governs what "done" means. Sessions 4 and 3 are history for the
> Platform refactor and the Advanced-Fillet program; read them only if you touch
> those areas.

## Session 6 — ran the ladder, committed the tranche

### What this session was

A Codex run (`019ffbc3-dde5-7aa1-b19f-02b6ce8987de`) implemented Session 5's
completion plan, then died on usage limits **mid `cargo test --workspace`** with
102 dirty paths and nothing committed. This session analyzed that rollout, ran the
full gate ladder, fixed what it turned red, and committed.

Scope was set with the user up front: **stabilize + verify only** (no new plan
features), **commit at a green gate** (no push).

### Landed (two commits on `master`, unpushed)

- `cf6273d feat(modeling): harden plan-stream, worker lockstep, identity V3, publication evidence`
- `dc4bd5e docs: reconcile state, TODO, roadmap delta, risk register, residual register`

The code is deliberately ONE commit: the Rust/C++/protocol changes are mutually
dependent and a finer split would have put non-building intermediates on master.
That reasoning is in the commit body.

### Gate ladder — measured, not claimed (2026-08-13, local mac, unsandboxed)

| Gate | Result |
| --- | --- |
| worker Release build + restaged sidecar/manifest, `ctest` | 119/119 |
| `cargo fmt --all --check`, `clippy --workspace --all-targets -D warnings` | clean |
| `ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` | **767/767 over 60 targets** |
| corpus | 9 of 9 executed, zero skips |
| `cargo check --features tauri-e2e` | clean (first compile that code ever got) |
| `npx tsc --noEmit` · `bun run build` · `bun run test` | 0 · clean · 250 files / 4182 tests |
| coverage + contract verifiers, negative controls, hex gate | pass · 15/15 · empty |
| kernelbench `fillet/foundation:t0` both backends + `semantic-compare` | 136/136, 0 regressions, baseline unmoved |
| Playwright retries 0 | chromium 199 passed / 1 failed · webkit 199 passed / 1 failed |

### Three defects the ladder caught (fixed, in `cf6273d`)

1. **Worker stub violated SCHEMA §7.5.** It emitted an `autoBind` ResolveRefs
   resolution with no `bodyId`. Omitting `bodyId` is legal only on a
   non-promotable missing-body `needsRepair` with no candidates — which is exactly
   what the real worker returns for a bodyless ref (`ElementIdentity.cpp`
   `missing_body_resolution`). The stub now mirrors that branch; `solver_stub` pins
   both branches instead of the one contract-violating shape.
2. **`topology_rebind` fed a real V3 region id into a version-less profile.** The
   worker correctly refused (`regionId 'r_…' matched no selectable region`). Same
   fixture class Codex was mid-fixing when it died; now carries
   `region_identity_version: (!region.is_empty()).then_some(3)`, matching
   `revolve_ops` / `m2_gate` / `wire_contract` / `step_import_gate`.
3. **`src-tauri/src/tauri_e2e.rs` had never been compiled and did not** — E0597
   borrow error in `composition_status`. Codex's sandbox could not download the
   WDIO crates, so the whole `tauri-e2e` feature was unverified source.

### The one browser failure, and what it actually was

`e2e/extrude-commit-gesture.spec.ts:135` "click-away commits" failed
deterministically (3/3) on **both** browsers — and identically on a clean worktree
at baseline `9933689`, so it predates the modeling work. Root cause found by
reading history, not by guessing:

**commit `c7df7c8` — "D2: click-away commit removed entirely (spec choice)"**. The
frozen contract `src/test/contracts/modelingInteractionContract.ts` pins
`clickAwayPolicy: "cancel"` for every tool, and
`ModelToolController.commit.test.ts` has a vitest test asserting click-away must
NOT commit. `c7df7c8` shipped on vitest/tsc/build gates without the e2e lane, so
two stale artifacts survived it: this e2e spec, and the arm hint text still
promising "click away to confirm".

So this is **not** a product bug — production is right, the spec and the hint were
lying. Enter and the chip ✓ remain the only commit gestures.

### MC-R7 — closed (see the Session 7 block below)

Two files, both the MC-R7 correction:

- `e2e/extrude-commit-gesture.spec.ts` — the test now asserts click-away does NOT
  commit and leaves the tool armed, matching the frozen contract.
- `src/tools/modelTools/ModelToolController.ts` — `armHintFor` no longer promises
  "click away to confirm" / "click away to revolve".

Verified: that spec file on chromium **5/5**, `npx tsc --noEmit` 0, `bun run test`
**4182/4182**, then both full lanes with retries 0 — **webkit 200/200**,
**chromium 200/200**.

## Session 7 — the two lanes, and the one thing they turned up

Session 7 · 2026-08-13

The lanes ran on `E2E_PORT=4191` and `4193`. Check the port before quoting any lane
result: 4177 is held by the concurrent `OneCAD-Component-Library` session, and 4187
turned out to be held by a stray node process. A collision surfaces as
`ERR_CONNECTION_REFUSED`, which reads exactly like a product failure.

`e2e/extrude-commit-gesture.spec.ts` passes on both lanes, so **MC-R7 is closed as
stale evidence, not a product defect** — the spec and the arm hint were wrong, the
production behavior was right all along.

### MC-R8 — new, recorded, deliberately not fixed

The FIRST chromium run came back 199/1, on `e2e/boolean-preview.spec.ts:356`
(Intersect chip). The 20 s poll on `previewOwner === "boolean"` timed out at `null`,
so the boolean lane never opened. That spec is **9/9 in isolation** with
`--repeat-each=3`, and the immediate full rerun was **200/200**. The signature
matches the boolean-preview projection-push race already bisected to before the
Platform refactor: the region click lands ahead of the sketch-visibility commit's
projection push.

So the browser gate is green **as measured** but not yet **reproducibly** green.
The one move that must not happen here is adding a Playwright retry — a retry is
what let the auto-fit regression look green once already (Session 3, M0.4).

### How to resume

1. **Nothing is committed.** The MC-R7 fix (two files) plus the doc updates
   (`CURRENT_STATE.md`, `TODO.md`, `HANDOFF.md`,
   `docs/qa/modeling-residuals-v1.json`) are all in the working tree. Suggested
   subject: `fix(e2e): the click-away spec asserted a gesture D2 removed`. No push
   was requested; `master` is 2 commits ahead of `origin/master` already.
2. Then either root-cause MC-R8, or move on to the still-unrun gates below — MC-R8
   blocks a "reproducibly green" claim, not the work.

### Still unrun on any machine (unchanged from Session 5, do not claim these)

Real-Tauri WDIO composition (compiles now, never executed), kernelbench m1,
Linux/Windows release matrix, 20-run stability sample, P2 measured
ceilings/performance, P3 semantic + overhead closure and Pattern budget, Chamfer
and Boolean campaign breadth.

### Environment notes worth carrying

- `ONECAD_OCCT_ROOT=/Users/andrejvysny/.onecad-occt/8.0.1` (read from
  `worker/build/CMakeCache.txt`).
- `scripts/build-worker.sh` regenerates BOTH the staged sidecar and
  `src-tauri/binaries/onecad-worker-manifest.json`. Skipping the restage makes
  manager tests fail on a hash mismatch that looks like a real defect.
- Kernelbench needs absolute paths: a relative `--out-dir` resolves against the
  process cwd and silently writes outside the repo.

---

## Session 5 — the roadmap plan (still governs "done")

Session 5 · 2026-08-13

## Goal

Finish the seven-phase program in `OneCAD-modeling-correctness-roadmap/` — every
supported modeling operation with an explicit publication contract, proportionate
validation, fail-closed identity, preview that matches commit, evidence at kernel /
real-worker / user-flow layers, and CI that blocks rather than informs.

## Original plan

`~/.claude/plans/now-lets-plan-next-sunny-lighthouse.md` — approved 2026-08-13.
Seven tracks A–G. **Read it before doing anything**; it carries the per-item
evidence (file:line) that this summary compresses away.

Three decisions were taken with the user while planning, and they shape everything
downstream:

1. **Finish Phase 5, then Phase 6 as a separate second tranche** (the master
   roadmap's own recommendation for one founder).
2. **Replace the owed manual Tauri smoke with a real automated Tauri lane** — it
   closes risk R-20 permanently instead of discharging a manual gate once.
3. **Generalize kernelbench's case-v2 additively** rather than cutting a v3. Every
   m1 `inputDigest` will move again and must be re-recorded with a written
   explanation, exactly as in `1d0fbb7`.

### What the planning phase found, and why it reordered the work

Three exploration agents verified every phase exit gate against the live tree. The
finding that set the plan's shape: **"recorded complete" and "gate satisfied" have
come apart in all five finished phases, and the pattern is consistent — the
implementations landed, the evidence did not.** Repeatedly a work package's code is
correct and complete while the tests the spec mandates do not exist, and in a few
places an artifact asserts something the code does not do. Hence Track A (the
unbilled remainder of Phases 0–4) comes before any new Phase 5 breadth.

## Done so far (and why)

Five waves committed this session, all on `master`, **none pushed** (10 commits
ahead of `origin/master`).

- **`1d0fbb7` — M3.5, non-isometric metamorph policy** (Phase 5 WP5.1 residual).
  The metamorph comparison now carries a RELATION per variant instead of demanding
  equality from all of them: rigid → equivalence, `scaled` → similarity (`k³·V`,
  `k²·A`), `parameterEpsilon` → continuity. The relation is a pure function of the
  variant name, so the frozen result-v1 block is untouched. Coefficients measured,
  not guessed. **It found a real defect**: `validate_output` measured the blend
  against the case's DECLARED radius while `Execution.cpp` filleted with the
  scaled/nudged EFFECTIVE one.
- **`8bd0fdb` — M4 recipe-agnostic validators** (Phase 5 WP5.2). The enabling move
  was taking the blend from the fillet builder's own history (`Generated` /
  `Modified`) instead of recognising it by surface type — "the cylinder in the
  output is the fillet" is false the moment a *support* is a cylinder.
  `crossSectionProfile` uses `1/|k|` of the larger principal curvature, exact on
  plane/cylinder/cone because a constant-radius blend is a canal surface.
- **`772b3d2` — A1, the shared operation-result classifier** (WP0.3). The one item
  in Track A that was a live correctness defect (R-04) rather than missing evidence:
  `needsRepair` was declared and never produced, no shared helper existed, and every
  consumer inferred success from body counts.
- **`6b08e27` — A2, draft applied-or-refused** (WP0.6). **The mandated
  circular-profile red probe finally ran; the answer is REFUSAL**, so R-10 is closed
  as not-present with evidence rather than by inspection.
- **`4b62965` — A3, stable diagnostic codes** for the Draft and empty-Boolean
  refusals, both of which returned a bare `OP_FAILED` with the reason only in the
  message.

### Decisions and dead-ends worth not rediscovering

- **`repairSteps` rides `regen-finished`, not the `needs-repair` event.** The
  sibling event carries the same facts but is emitted AFTER `regen-finished`
  (`api/mod.rs` `emit_regen_events`), so an awaiter settling there would always miss
  it. This was the whole reason `needsRepair` had never been produced.
- **A NeedsRepair record is never rolled back.** `keepsRecord()` covers
  published/noop/needsRepair. Rolling back the record repair operates on would
  delete the thing the user is about to fix.
- **`classifyRegen` falls back to the old body-count inference when `terminal` is
  absent**, deliberately, so legacy fixtures behave exactly as before.
- **Blend/support faces must come from builder history.** Sampling the blend's
  boundary with the END CAPS fails every non-cone case at exactly π/2 — that
  boundary is a right angle by design, not a defect.
- **Tolerances need a conditioning term.** `farOriginTranslation` rebuilds 1.7e6 mm
  out where double precision costs six orders of magnitude; without the term the
  probe reads arithmetic as a defect.
- **Two findings recorded, not fixed** (both outside their item's scope): `Arc`
  entities still reach the BRep as polylines while `Circle` stays analytic — the
  slot probe measures 28 faces and a 24-gon cap, direct evidence for the Phase 2
  residual the plan tracks as B3. And the draft refusal already carried a structured
  `Diagnostic{stage:"build"}`, which narrowed A3 to the code rather than the
  envelope.
- **Every new test table was proved non-vacuous by mutation**, and this is the house
  style now: bypassing the classifier reds 9 rows; reverting `commitPattern` reds 4
  and reproduces R-04 verbatim; a wrong code in `boolean_empty_refusal.ndjson` reds
  the fixture. Do the same for anything new.

## How to resume

1. Run the `handoff` skill with "resume".
2. Read `~/.claude/plans/now-lets-plan-next-sunny-lighthouse.md` — the plan is the
   authority, this file is the summary.
3. **Answer the open question below**, because A4 cannot start without it.
4. Environment (nothing here is optional — `bundle.externalBin` makes any cargo
   command that compiles the app crate require the staged sidecar):
   ```bash
   ONECAD_OCCT_ROOT=$HOME/.onecad-occt/8.0.1 scripts/build-worker.sh Release
   ctest --test-dir worker/build --output-on-failure          # expect 117/117
   cd src-tauri && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
   ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 cargo test --workspace
   ```
5. A second interactive session (`onecad-library-content-impl`) shares this working
   tree. Coordinate before touching Rust or worker sources; **never branch or stash
   here** — a stash once swept a concurrent session's work.

## Open questions

- **A4 needs a product call, and it blocks the next item.** The Revolve body-edge
  axis is implemented and deliberately unexposed (`ModelToolController.ts:8302`),
  its four required tests are missing, and the P3 contract row asserts
  `uiExposure:"exposed"` while `src/ipc/types.ts:843` says the tool only authors the
  `sketchLine` variant. **Either expose it in the UI, or mark the row
  deferred/hidden with a contract test.** The four tests get written either way; only
  the contract row and the UI differ.
- **Nothing is pushed** (10 commits). No push was requested; say so before doing it.
- **Phase 6 needs Proxmox host access** (F4/F5) that refused connections last time,
  and **no Windows machine is identified anywhere** for F6.

## Pointers

- Plan → `~/.claude/plans/now-lets-plan-next-sunny-lighthouse.md`
- Tasks → `TODO.md` § NOW · Snapshot → `CURRENT_STATE.md`
- Roadmap bundle → `OneCAD-modeling-correctness-roadmap/` (read
  `04-live-implementation-delta.md` first; it supersedes the baseline files)

---

# Handoff — Platform refactor (Milestones 1 + 2), and what comes next

Session 4 · 2026-08-08

> Session 4 is the Platform/module refactor, COMMITTED as `4145f3f`. Session 3
> (further down, unchanged) is the Advanced-Fillet roadmap — its § "VF-M5 gate
> regression" is the diagnosis behind P1 here.

## Goal

Reorganize OneCAD so future capabilities (FEM, TechDraw, CAM, third-party
addons) can be added as clean modules, without redesigning the modeling engine
or changing any user-visible behavior.

The principle: **modeling stops being synonymous with OneCAD and becomes the
first privileged built-in module running on the OneCAD Platform.**

## Original plan

`~/.claude/plans/velvety-leaping-adleman.md` — seven waves, W0…W7. Scope was
fixed with the user up front: Milestone 1 (frontend platform, spec §195) **plus**
Milestone 2 (module-namespaced persistence, spec §196); **no file moves**;
branded namespaced ids with the `ModelTool`/`SketchTool` unions retained.

Out of scope by decision: SDK package, test addon, addon manifest/loader/host,
GitHub install, resource-store generalization, dynamic Tauri router, crate
extraction, and everything in spec §193.

## Done so far (and why)

All seven waves landed and are committed as `4145f3f`. Full wave-by-wave detail
is in `TODO.md` § PLATFORM REFACTOR; the reasoning worth carrying:

- **Contracts first, refactor second.** Four frozen behavior contracts went into
  `src/test/contracts/` BEFORE anything moved: toolbar arrangement, the three
  keymap tables + cross-mode opt-out, editor mount order, inspector section
  order. `src/test/contracts/README.md` states the rule that makes them worth
  anything: a probe may change when the mechanism changes, a contract may not be
  edited to make a refactor pass.
- **The probes are non-tautological by construction.** The toolbar assertion runs
  against `toolbarFromRegistry` — the arrangement rebuilt FROM the platform
  registry — not against the descriptor table that fed the registry, or it would
  only prove the table equals itself. The keymap probe compares `resolveBinding`
  to an independently written oracle over every (key, shift, mode) triple.
  `architecture.test.ts` carries a POSITIVE CONTROL, because every other
  assertion in it expects an empty list, which is also what a broken scanner
  returns.
- **Registration happens on EDITOR MOUNT, not at bootstrap.** `App.tsx`
  code-splits the editor deliberately and `StartScreen` idle-prefetches that
  exact specifier; hoisting nineteen feature imports into the startup bundle to
  satisfy an architectural preference would make the start screen pay for the
  editor. This forced two real design changes: `platform.createScope(owner)` for
  independent child scopes, and a teardown fix — a scope disposes what IT
  registered and never sweeps the owner, or the editor's short-lived scope would
  tear down the module's bootstrap registrations.
- **Tool ids are scope-qualified** (`…tool.model.mirror` vs `…tool.sketch.mirror`)
  because `select` and `mirror` exist in both unions meaning different things; a
  flat map would have let one silently shadow the other.
- **Separators are derived from group boundaries.** `group` is consumer metadata,
  `priority` is the sort key — so palette order can never start following module
  load timing.
- **Two slots were added deliberately** (`viewport.chrome`, `shell.notification`).
  Without them the frozen mount order cannot be reproduced with contiguous slot
  regions, and mount order is load-bearing (a past defect had tool chips render
  under the side panels and become unclickable).
- **Module state lives in `document.json`** (ADR-0004), skip-if-empty in both the
  document and the manifest, so existing files serialize byte-identically: no
  container bump, no user migration. `ModuleId` validates on AUTHORING only —
  deserialization is permissive because refusing an id a stricter build dislikes
  would destroy exactly the data preservation exists to protect.
- **Programmatic writes use the user's path.** `EditCommand::SetModuleState` has
  a memento inverse; there is no separate mutation lane for automation.

Deviations from the spec, each with the reason recorded in `TODO.md`:
`onecad.shell.workspace.design` rather than `onecad.workspace.design` (a
contribution id must sit under its owner's namespace, and the workspace composes
several modules); no `platform_invoke` router (three typed commands until the
addon host needs one).

### Dead ends / things already ruled out

- **Do not point `build-worker.sh` at `/opt/homebrew/opt/occt-8.0.1`.** It is a
  plain Homebrew install with no `share/onecad/occt-build.json`, so CMake aborts
  with "OCCT artifact metadata is absent". This session mis-reported that as a
  repo blocker; it is not. Use `~/.onecad-occt/8.0.1` and
  `worker/build-pinned` — see § How to resume in the Session 3 thread below.
- **A single full-suite e2e failure is not evidence.** `filletChamfer.spec:169`
  failed once in a full run and passed 13/13 in isolation. The four `theme.spec`
  failures are the real, reproducible, pre-existing ones.
- **Do not split the commit.** M1 (frontend) and M2 (Rust) were gated as one
  tree; splitting produces at least one commit that does not build, because
  `platform/index.ts` exports `documentState`, which needs the M2 client methods.
- **Do not `git checkout -b` in this tree.** A concurrent session shares the
  working directory; branching silently redirects THEIR commits too.

## How to resume

1. Run the `handoff` skill with "resume".
2. Re-read `CLAUDE.md` — it gained a § "Architecture laws" section that binds new
   code — then `docs/ARCHITECTURE.md` (normative) and `docs/adr/0001`–`0008`
   (why). Those three are the standing rules for this program.
3. Work `TODO.md` § NEXT SESSION, which has the full checklist. The shape:
   - **P0 (do first, ~30 min).** Manual `tauri dev` smoke — the only
     Definition-of-Done item with no evidence. Then rebuild the sidecar against
     the pinned prefix and re-run the worker lane so the numbers describe HEAD.
     `topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean`
     is EXPECTED to fail there; that is P1, not a refactor regression.
   - **P1.** Close the VF-M5 gate regression. Belongs to the other session —
     check `git log` for new `wip:` commits and coordinate before starting. The
     diagnosis is already written in the Session 3 thread below; do not re-derive
     it.
   - **P2.** Finish Milestone 1: toolbar reads the registry live; inspector
     sections, tree nodes and viewport layers become real contributions (the
     contracts exist with zero producers); move `zoomFit`/`home` off modeling.
     Mechanical — the golden tests already pin every answer.
   - **P3.** Milestone 3: `@onecad/sdk` + a bundled test addon that imports
     nothing else. **Do P2 first** — an SDK frozen over a half-converted surface
     freezes the wrong shape.
4. Suites, all expected green except where noted:
   ```bash
   bunx tsc --noEmit && bun run test        # 221 files / 3796
   bunx playwright test                     # 386 pass / 4 fail (theme.spec, pre-existing)
   cd src-tauri && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
   ```

## Open questions

- **Push?** `master` is 5 commits ahead of `origin/master`, nothing pushed. Two
  of those commits (`685efc2`, `069bb48`) are the concurrent session's in-flight
  `wip:` work. Coordinate before `git push`.
- **P1 fix direction** — keep the anchor-exact carve-out enabled until a genuine
  restore signal is plumbed, or plumb one now? Recorded as a choice, not decided.
- **Four untracked paths** (`.opencode/`, `.agents/`, `STEP/`, `skills-lock.json`)
  still want a `.gitignore` entry rather than a commit. Nobody has decided.
- **Should `TitleBar`/`StatusBar`/`NavPill`/`CornerCluster` stay an
  `onecad.shell` module**, or go back to being permanent hard-coded structure?
  Implemented as a module for consistency; the simpler alternative is defensible.

## Pointers

- Tasks → `TODO.md` § NEXT SESSION · Snapshot → `CURRENT_STATE.md`
- Architecture laws → `docs/ARCHITECTURE.md` · decisions → `docs/adr/`
- Frozen behavior contracts → `src/test/contracts/README.md`
- Plan → `~/.claude/plans/velvety-leaping-adleman.md`

---

# Handoff — Advanced-Fillet roadmap (M0 · M1 · M2)

Session 3 · 2026-08-08

Supersedes the Session 1 handoff (TRUST + PREVIEW), which is closed. Sessions 2
and 3 both worked this roadmap; M2 is now complete.

## Goal

Make Advanced Fillet OneCAD's first genuinely differentiated geometric
operation, and build the infrastructure that can **prove, case by case, that
OneCAD is safer and increasingly more capable than raw OCCT**.

Effort split: kernel robustness 70% · direct modeling 20% · UI 10%. Loft/sweep,
assemblies, and release work are all deferred.

The headline KPI is the **raw-OCCT rescue rate** — same input, same intent, raw
OCCT fails, OneCAD succeeds, and both a deep audit and a semantic test pass —
under two hard invariants:

- `raw PASS → OneCAD FAIL` on a supported case is a serious regression.
- Returning invalid or semantically wrong geometry is **always worse** than
  safely refusing.

## Original plan

M0 reproducible-green → M1 QA automation → M2 case-v2 + fillet geometry
generators → M3 metamorphs → M4 failure taxonomy + semantic validators → M5
`--jobs/--shard/--resume` → M6 minimizer + regression promotion → M7 SQLite
ingest + HTML dashboard → M8 large characterization campaign (production fillet
algorithms UNTOUCHED) → M9 baseline KPIs → M10 `FilletBuilder(FilletDefinition)`
→ M11 OCCT capability spike → M12 variable radius → M13 critical-radius
diagnostics → M14 second campaign → M15 first evidence-driven rescue strategy →
M16 measure rescue improvement → M17 chord width / verified G2 / overflow /
corners → M18 Shapr3D goldens → M19 DirectEditPlanner + reblend.

Full roadmap with the 15 agent constraints: `TODO.md` § ADVANCED-FILLET ROADMAP.
**M2–M19 is a multi-month program.** This session delivered M0, M1, and the
first half of M2.

## Done so far (and why)

### M0.1 — OCCT build fingerprint (`scripts/occt-fingerprint.sh`, new)

`build-pinned-occt.sh` required every `-D` to appear literally in
`CMakeCache.txt`. Root-caused against the pinned source **and** a real configure:
OCCT's `OCCT_CHECK_AND_UNSET` does `unset(VAR CACHE)`, and each `CAN_USE_*` comes
from scanning `BUILD_TOOLKITS`. With `BUILD_MODULE_Draw=OFF` nothing requires
`CSF_TclTkLibs`, so `CAN_USE_TK` is OFF and CMakeLists (8.0.1 L558-565; 7.9.3
byte-identical over every `USE_*`/`CAN_USE_*`/`OCCT_CHECK_AND_UNSET` line)
DELETES our `-DUSE_TK=OFF`. Nine further keys can vanish the same way.

The normalizer represents **effective configuration**: an absent DEPENDENT key
is `OFF` *only when the policy requested OFF* (OCCT deleted it precisely because
the feature is not in the build). Requested-ON-but-dropped, a missing REQUIRED
key, a present key disagreeing with policy, and a value outside CMake's truth
constants are all still fatal. Normalization, not relaxation.

Verified end to end by building pinned OCCT 8.0.1 from scratch into a clean
prefix — `USE_TK` really is absent from the produced cache, and the old script
would have aborted there.

### M0.2 — frontend clean build

`bun run build` was red while `bun run test` was green, because **vitest does not
typecheck**. `Rgba` is now `readonly` (a color is a value, never mutated) and the
~20 duplicated raw `[number,number,number,number]` literals collapse onto it;
read-only consumers take `ReadonlyMap`. New `src/test/fixtures/bodyMeshView.ts`
replaces a hand-written 24-field structural mock.

Also fixed a real hex-gate violation found while gating: `InspectorPanel`
hardcoded `#a9aeb6`, which **is** `--color-body-fill`. Now read through
`palette.bodyNeutral()`, so the no-color swatch follows the theme.

### M0.4 — viewport auto-fit regression (a real product bug the gate caught)

`1fe0cef` replaced "auto-fit once on the first body" with a 250 ms debounced
re-fit — right intent (a multi-body assembly must not be framed on whichever body
streamed in first), wrong place: the timer lived in the React bridge, so a
`fitView()` tween could START after everything else already believed the camera
had settled.

Bisected, not guessed: `measure.spec.ts` is 5/5 at `d1c5339`, **1/5 at
`1fe0cef`**, and 5/5 at `1fe0cef` with only that hunk reverted.

Fix keeps the intent and makes the scheduled state real: `ViewportEngine`
`requestAutoFit()` / `autoFitPending` / `cancelAutoFit()`, cancelled by explicit
`fitView`, `fitToBodies`, `enterSketch`, and `dispose`. User-visible too — a
queued fit could snap the camera mid-interaction, and one landing during sketch
entry was saved as the restore pose.

### M0.5 / M0.6 — the remaining 7 e2e failures, root-caused not retried

All seven reproduced on a pristine `1fe0cef`; none came from this work package.

- **`transform-body` ×4** — the spec's `bodyBounds` folded the fat-line EDGE
  layer into the bbox. `Line2` reports `isMesh === true`, and its `position` is
  the *instanced unit-quad template* `(-1,-1,0)…(1,2,0)` (real endpoints live in
  `instanceStart`/`instanceEnd`). Every body's min was clamped to ≤ -1 while
  `max` stayed correct — exactly the observed `[-37, -1, -15]`. Traversal now
  takes `userData.kind === "face"` only.
- **`history-inline-dimension:213`** — `toolApplicability.ts` is NEW in
  `1fe0cef`; the toolbar is selection-gated now, and committing an extrude hides
  its sketch, so Extrude sat `aria-disabled` and the click burned the 45 s
  timeout. Second cause: `getByLabel("Dimension value")` matched both the row
  editor and the tool chip.
- **`hole:226`** — `findFaceOnBody` probed pixels before the camera settled.
- **`sketch-degenerate:35` + `live-dim-mouse-rounding:62`** — `enterSketch` aims
  along the plane normal at `controls.getDistance()`, so the sketch view
  **inherits the model camera distance**, which sets `planePixelWorld`, hence the
  draw tools' screen-constant reject radius (`minSize = 4 × planePixelWorld`)
  AND the zoom-adaptive dimension quantum. Entering mid-fit made both
  non-deterministic; every caller settled AFTER entering, too late.

Same class as the auto-fit bug: **camera state read before it settled.**

`playwright.config.ts` now has `retries: 0` everywhere (user decision). A CI
retry is what let the auto-fit regression read as green.

### M1 — manual-QA debt triaged

All ~112 boxes of the historical checklist classified: **78 retired** against
existing automated tests (each cited `path:line`), **22 named gaps** (GAP-K/H/F,
each with its target lane), **12 genuinely manual**.

- `docs/qa/MANUAL_RELEASE_GATES.md` — the 12 survivors, grouped by *why* a
  machine cannot judge them: visual 4 / native 4 / hardware 4. Carries an
  explicit "do not grow this file" rule.
- `docs/qa/MANUAL_GATES_TRIAGE.md` — item-by-item classification + the gap backlog.
- `docs/qa/archive/MANUAL_GATES_RUN-2026-08-04.md` — historical list, verbatim.

### M2 part 1 — kernelbench case-v2

- `bench/robustness/schemas/case-v2.schema.json` — strict throughout.
- `src-tauri/crates/onecad-kernelbench/src/case_v2.rs` — a separate type set.
- `bench/robustness/examples/fillet-matrix-plane-cylinder-v2.json` — ajv-validated
  example. Deliberately NOT in `regressions/`: regressions are contractual.

Shape decisions:

- **`selector` is top level** (v1 nested it under the operation) — the same
  geometry+selection is reused across radius laws and later across operations, so
  a resolving selector is a *precondition*, not part of the definition.
- **`operation.definition` mirrors the kernel-level `FilletDefinition`** — radius
  law + continuity — so `constant`/`linear`/`controlPoints` are all expressible
  and adding one later is additive on both sides.
- **`geometry.parameters` is a typed per-recipe union**, not the free-form object
  the plan sketched. A free-form bag lets a typo silently generate different
  geometry — the one failure mode a fuzzing corpus cannot survive.
- **`continuity` admits only `g1`; `sizeType` only `radius`.** Per agent rules 11
  and 15: an option is not a capability. G2 and chord width become expressible
  only once a validator can prove the result.

Guards: v1/v2 are provably disjoint (tested both directions in Rust AND ajv); a
test asserts v1 regressions still validate under `Case`; the round-trip
reproduces the committed file, which already caught three missing
`#[serde(rename)]` (`startRadius`, `relativeDelta`, `anchorIndex`).

### M2 part 2 — geometry generators + the v2 execution lane (session 3)

The worker executes a `schemaVersion: 2` case and the supervisor drives one end
to end. No production fillet algorithm touched.

**The C++ `CaseSpec` is normalized, not forked.** `Types.h` carries the v2 shape
and the v1 parse fills the subset, so ONE execution path serves both formats.
Forking the executor per version is exactly where a v2-only bug would hide.
`parse_case` dispatches on `schemaVersion`; `CaseParserV2.cpp` mirrors
`case_v2.rs::validate` field for field (Rust is authoritative for what a case may
CONTAIN, C++ for what the runner will EXECUTE, and a disagreement makes a valid
case fail as an invalid request); shared primitives moved to
`CaseParserShared.h`. A non-constant radius law is REFUSED at parse rather than
flattened to its peak — variable radius is M12, with a validator that can prove
the law.

**`GeometrySupportPair.cpp` is the milestone.** Two constructions:

- **Prism family** (plane|cylinder × plane|cylinder) — one 2D profile extruded
  along Z. Every support is prismatic, so the shared edge is the straight
  vertical line through the ORIGIN and the dihedral is free over `(0,180)`. A
  planar support is a rotated half-space box; a cylindrical one is a cylinder
  tangent to the support line AT the origin with material inside, so near the
  edge it is locally the same half-plane and the dihedral is exactly the profile
  angle. The solid is their `BRepAlgoAPI_Common`.
- **Cone family** (plane × cone) — a frustum's base circle. Here the dihedral is
  NOT free: it is `90 - halfAngle` by construction. The generator rebuilds it
  from the half angle and refuses a case that declares anything else, rather
  than silently generating different geometry than the file describes.

Why the prism form and not the committed example's base-circle form: that locked
plane↔cylinder to a 90° dihedral, and the dihedral is the main conditioning axis
for fillet failure. The prism form also keeps the blend a CYLINDER, which is what
lets `constantRadius`/`cylindricalRadius` gate it at all. The example was updated
to match — examples are explicitly not contractual, `regressions/` is untouched.

**Rust `PreparedCase`** (`prepared.rs`) is the version-agnostic view the
supervisor executes: identity, resource ceilings, metamorph tolerance, canonical
document. `campaign`/`runner`/`child`/`result`/`result_validation` route through
it, so none of them grew a `match` on the schema version. `suite_v2.rs` is the
`fillet/matrix` preset `m1` (24 cases → 60 records); `cli` gained the suite and a
version-dispatched `run-case`.

**Two real defects, both found by RUNNING it rather than by unit tests:**

1. The result envelope echoed the case's own `generator` block, so case-v2's
   extra `family` field failed the supervisor's strict result validation —
   60/60 red with an empty stderr. The RESULT schema is frozen at v1, so
   `Execution.cpp` now BUILDS the `{name, version, seed}` subset.
2. `parse_case_v2` never re-checked `schemaVersion` itself — only the dispatch
   did — so calling it directly on a v1 document read it as v2. Fail-closed on
   both sides now, caught by the new ctest fixture.

**Sizing is measured, not assumed.** A blend needs room; for a prismatic pair the
throat is `R·(1 - cos θ)` where `R` is the tightest curved support (half a planar
support's width when neither is curved). Swept against OCCT 8.0.1: every pair
blends at 0.20 of that throat, the cylinder↔cylinder lens refuses at 0.40.
Supported cases sit at 0.04–0.16, so a red there is a kernel regression and not a
greedy case.

**Finding — `cylindricalRadius` and `g1BoundaryTangency` are box-shaped.** The
first counts EVERY cylindrical face in the output against the requested radius,
so a cylindrical SUPPORT reads as a blend of the wrong radius (17.0 mm error
measured on a 20 mm support); the second only recognises plane↔cylinder tangency
pairs and cannot see a blend meeting a curved support. Both are emitted only for
all-planar pairs, pinned by test. Their recipe-agnostic replacements
(`supportTangency`, `crossSectionProfile`) are expressible in case-v2 but
unimplemented, and an unimplemented validator reports `notApplicable` — which
FAILS a required check rather than passing it — so emitting them now would red
the whole suite. They land with M4.

### Dead ends / things already ruled out

- The e2e failures are **not** flakiness to be retried away — every one had a
  mechanical cause. Don't reintroduce `retries`.
- `sketch-degenerate` and `live-dim-mouse-rounding` pass in isolation and fail
  in-suite. Isolation runs prove nothing here; use a full run or a pristine
  worktree.
- A full-suite e2e number is **not attributable** while a second session edits
  the tree — one run hit a live `[vite] Transform failed` from a mid-edit save.

## How to resume

0. `master` is **4 commits ahead of `origin/master` and unpushed**; the working
   tree is clean apart from four deliberately-untracked paths. Two of those
   commits are a concurrent session's `wip:` work — do not push without
   coordinating (see § Open questions).
1. Run the `handoff` skill with "resume".
2. Re-read `CLAUDE.md` (kernelbench + protocol conventions) and `TODO.md`
   § ADVANCED-FILLET ROADMAP, especially the **15 agent constraints** — they are
   the standing rules for this whole program.
3. Build/verify:
   ```bash
   ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" \
   ONECAD_WORKER_BUILD_DIR="$PWD/worker/build-pinned" scripts/build-worker.sh Release
   ctest --test-dir worker/build-pinned --output-on-failure
   cd src-tauri && ONECAD_WORKER_PATH=$PWD/../worker/build-pinned/onecad-worker \
     ONECAD_REQUIRE_WORKER=1 cargo test --workspace
   ```
   Pinned OCCT 8.0.1 is already installed at `~/.onecad-occt/8.0.1`; the build
   script short-circuits on its fingerprint.
4. Run both suites to confirm the baseline before changing anything:
   ```bash
   export ONECAD_KERNELBENCH_RUNNER=$PWD/worker/build-pinned/onecad-kernelbench-runner
   cd src-tauri
   cargo run --release -p onecad-kernelbench -- run \
     --suite fillet/foundation --preset t0 --backend both --out-dir /tmp/t0
   cargo run --release -p onecad-kernelbench -- run \
     --suite fillet/matrix --preset m1 --backend both --out-dir /tmp/m1
   ```
   T0 must stay 136/136 with `gatingFailures: 0`; M1 is 120 records, 114 pass +
   6 characterization.
5. **Next task is M3** — metamorph EXECUTION. The v2 metamorph set (mirror,
   uniformScale, farOriginTranslation, parameterEpsilon, edgeOrderPermutation,
   contourSeed) is expressible in the schema and validated on both sides, but
   only `translation` and `rotation` are actually executed: `VariantSpec` in
   `worker/src/benchmark/Types.h` still carries only translation + rotation, and
   `suite::Variant` mirrors it. Widening that pair is the work.
   Then M4, whose recipe-agnostic validators are what unblock required
   validators on curved-support pairs.
6. **KBR case-v1 stays frozen.** Never edit `case.rs` or `case-v1.schema.json`.
   `examples/` is NOT contractual; `regressions/` and `presets/` are.

## VF-M5 gate regression (RESOLVED 2026-08-09 — kept for the record)

**Both halves are now closed, and one claim below is FALSE.** The regression was
real and was fixed by `from_zero_replay = false`. But "V1 has no checkpoint
plumbing / there are no restores" is wrong: checkpoints are plumbed end to end and
the regen executor's F12 fallback reaches exactly the hazardous lane. That residual
is now closed by the SCHEMA §7.2 `checkpointFallbackReplay` field, which is the
"plumb one" option this note offered — proven red-first by
`topology_rebind::vfm5_lane_d_checkpoint_fallback_replay_must_not_bind_the_decoy`.
See `TODO.md` § VF-M5 RESIDUAL. The text below is the original note, unedited.

### Original note

**The `from_zero_replay` gate in `069bb48` is WRONG and re-opens the defect-fix
gate.** Real-worker lane (`ONECAD_REQUIRE_WORKER=1 cargo test --workspace`,
worker built from HEAD) fails
`topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean`
(`needsRepair` expected 0, observed 1). Baseline worker passes the same —
regression is the gate. Root cause: V1 has no checkpoint plumbing, so the
setup gate `partition.size() == 0` (worker `PlanExecutor`) is ALWAYS true and
the gate degenerates to `edited_from.is_present()`; the flagship edit lane
(`ToEnd { from: 1 }`, carries `edited_from`) is falsely treated as a
from-zero-replay → anchor-exact carve-out off → model flagged `NeedsRepair`.
The VF-M5 scenario (stale world anchors after a real checkpoint RESTORE) cannot
occur in V1 — there are no restores. Fix: keep the carve-out enabled until a
genuine `baseCheckpoint`/restore signal is plumbed into the plan, or plumb one.
See `CURRENT_STATE.md` for the full writeup. Rebuild the worker before
re-running the real-worker lane — the on-disk sidecar `worker/build/` is
currently a stale pre-gate build (19:37).

## Open questions

- **Commit boundary — RESOLVED, but read this before pushing.** The backlog was
  split into four commits on `master` at this session's close (see
  `CURRENT_STATE.md` for hashes). Two of them, `685efc2` and `069bb48`, are a
  CONCURRENT SESSION'S in-flight work, committed as `wip:` so it was not left
  loose; their message bodies say plainly that this session neither authored nor
  gated them. **Nothing is pushed.** If that session is still running it will
  keep editing these files, so coordinate before `git push` — a force-push or a
  rebase over their in-flight tree is the thing to avoid.
- **The `wip:` commits carry a known-red build.** `bun run build` fails on
  `src/ipc/mockClient.import.test.ts:106` (`'snap' is possibly 'null'`), which is
  now committed inside `685efc2`. Same class as the M0.2 bug: vitest is green
  (209 files / 3698 tests) because it does not typecheck. Not a M2 problem —
  M2 is Rust + C++ only — but `master` does not currently build the frontend.
- **Two e2e failures belong to that session, not to this work:**
  `theme.spec:121,145` (`getByRole('button', {name: /^Appearance:/})` not found,
  from their `TitleBar.tsx`) and webkit-only `boolean-preview`.
- **Four paths are deliberately untracked:** `.opencode/` (61 MB), `.agents/`,
  `STEP/`, `skills-lock.json`. They want a `.gitignore` entry, not a commit;
  nobody has decided which.

## Pointers

- Tasks → `TODO.md` · Snapshot → `CURRENT_STATE.md`
- Roadmap + 15 agent constraints → `TODO.md` § ADVANCED-FILLET ROADMAP
- QA triage → `docs/qa/MANUAL_GATES_TRIAGE.md` · release gates →
  `docs/qa/MANUAL_RELEASE_GATES.md`
- Benchmark contract → `bench/robustness/README.md`
