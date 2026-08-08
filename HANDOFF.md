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

## Open questions

- **Commit boundary.** Nothing is committed. ~50 of my files are interleaved with
  a second session's in-flight work in the same tree, and this is now two
  sessions' worth. Splitting them cleanly gets harder the longer both stack up.
  The M2-part-2 files are cleanly separable — everything under
  `worker/src/benchmark/`, `worker/tools/kernelbench-runner/`,
  `src-tauri/crates/onecad-kernelbench/`, and `bench/robustness/` is mine and
  touches nothing the other session has open.
- **A second session is actively editing this tree** — `TitleBar.tsx`,
  `ViewCube.tsx`, `CadOrbitControls.ts`, `Picker.ts` (removed `hasHitAt`),
  `ModelToolController.ts`, `FileMenu.tsx`, `StartScreen.tsx`, `appStore.ts`,
  `document_runtime/tests.rs`, and it added a **webkit** Playwright project
  (suite 190 → 386). Two failures are theirs: `theme.spec:121,145`
  (`getByRole('button', {name: /^Appearance:/})` not found, from their
  `TitleBar.tsx`) and webkit-only `boolean-preview`.
- **`src/ipc/mockClient.import.test.ts:106` currently fails `tsc`**
  (`'snap' is possibly 'null'`) — their uncommitted file, and the *same* class of
  bug M0.2 fixed: vitest is green (209 files / 3698 tests) because it does not
  typecheck. `bun run build` is red until they fix it.

## Pointers

- Tasks → `TODO.md` · Snapshot → `CURRENT_STATE.md`
- Roadmap + 15 agent constraints → `TODO.md` § ADVANCED-FILLET ROADMAP
- QA triage → `docs/qa/MANUAL_GATES_TRIAGE.md` · release gates →
  `docs/qa/MANUAL_RELEASE_GATES.md`
- Benchmark contract → `bench/robustness/README.md`
