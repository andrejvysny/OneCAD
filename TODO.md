# OneCAD-Tauri Migration TODO

## NEXT — post-merge plan (2026-08-14)

One trunk again. Everything below is ordered by what a **daily-driver** needs, in
the product's own priority order (functional 3D-print + machined parts, light
multi-part, STEP as the machined deliverable): first do not lose the user's work,
then tell the user the truth about what happened, then make the exported artifact
faithful, then pay the measurement debt.

Sequencing preference stands: clear USER manual gates first, then one deep work
package at a time.

### T0 — merge aftercare (do before anything else; hours, not days)

- [ ] **Rebuild the worker in this worktree.** `worker/build/onecad-worker` and
      `src-tauri/binaries/onecad-worker-{aarch64-apple-darwin,manifest.json}` here
      date from 12:01, BEFORE the merge — no `ClassifyElement`/`ExportGeometry`,
      no component ops, an uncapped `PreviewOp`, and a manifest bound to that old
      binary's hash. `bundle.externalBin` makes every cargo command need the
      staged file, and `manager.rs` refuses a worker whose SHA-256 disagrees with
      the manifest, so this BLOCKS cargo and the app here:
      `ONECAD_OCCT_ROOT=~/.onecad-occt/8.0.1 scripts/build-worker.sh Release`.
- [ ] **Manual Tauri smoke of the merged stack.** Never run for either program,
      and this is the first time they meet in the real app. Minimum path: place a
      component from the library onto a hole rim (mate seats), edit a free
      parameter (re-bake), bind a past extrude to `=name`, confirm through the
      new ✓/✕ vocabulary with live numeric entry, switch to assembly colours,
      save, reopen. Watch `logs/dev.jsonl` for `"level":"ERROR"` and `regen:`.
- [ ] **Run the real-Tauri WDIO composition lane (MC-R4).** The `tauri-composition`
      CI job exists and its invocation was fixed, but the lane has still never
      executed on any machine. It is the only gate that proves the PACKAGED app
      and its bundled worker compose — precisely the axis the merge disturbed
      (new verbs, new binary hash, a re-manifested sidecar).

### T1 — data integrity (HIGHEST; "can lose the user's work" class)

The audit that landed in `8133bcd` found these and deliberately implemented no
fix. For a daily driver they outrank every feature below.

- [ ] **DI-1 + DI-2 together (~½ day).** They share a lane and a test: recovery
      consumes the crash marker while keeping an autosave that discovery can no
      longer reach, AND never ticks the autosave loop, so a recovered document is
      unprotected against the next crash from two directions at once. Pick one of
      DI-1's three options (keep the marker until the next autosave supersedes it
      · write a fresh marker at recovery · scan autosave files as a fallback) —
      they differ only in stale-offer profile. DI-2 is one `note_mutation()` call;
      the value is the test.
- [ ] **DI-3 (~2 h).** `promote_selection` and `prepare_edge_op` write
      `regen.elements`, which `build_save_payload` persists, with no tick and no
      dirty flag — so close takes the clean fast path and skips the prompt. The
      audit classified all 55 commands; these are the only mutating rows without
      a tick, so this closes the class, not just two cases.

### T2 — finish the result-truth doctrine

- [ ] **`upsertVariable` / `removeVariable` / `replaceComponent`.** The last three
      kernel-touching commands exempt from `regenOutcome` (see § MERGE for the
      two implementation options). **A product decision is owed first**, and it is
      small but real: a variable edit that SAVES while its downstream regen FAILS
      has two truths, and the UI currently states one. Decide what the user is
      told, then the code is mechanical. Do not "fix" it by throwing — the
      variable really was saved.

### T3 — make the machined-parts deliverable faithful

- [ ] **DI-5 — STEP export is lossy.** Import replays XCAF names and colours;
      export is a bare `STEPControl_Writer` with no XCAF document, so a file that
      came in coloured leaves grey. For the machined-parts priority the exported
      STEP *is* the deliverable, which makes this the highest-value non-integrity
      item on the list.
- [ ] **DI-4 — an authored face colour stops being paintable after a reopen.**
      Nothing re-binds a persisted `ElementId` at open, so both frontend paint
      paths come up empty. Same root as the retired W5 seam: **one fix closes
      both**, and it also makes DI-5's export worth doing on a reopened document
      rather than only on a freshly authored one. Sequence DI-4 before DI-5 if
      only one lands.

### T4 — hardening and measurement debt (the open residuals)

- [ ] **MC-R5 — release enforcement.** No current 20-run stability sample and no
      mandatory Windows worker-backed release matrix. Blocks calling any build a
      release rather than a build.
- [ ] **MC-R2 — publication overhead is unmeasured.** Structured
      `PublicationDecision` evidence and timings exist; the Tier A/B budgets
      (5%/15% P95) have never been closed with numbers.
- [ ] **MC-R1 — profile/region latency ceilings unmeasured** under
      `regionIdentityVersion: 3` (P95 25 ms, refusal 200 ms, ≤20% simple-sketch
      regression).
- [ ] **MC-R9 — browser-lane nondeterminism.** `revolve-commit.spec.ts:111` failed
      once in a loaded 19-minute chromium run and passes in isolation. It closes
      only on a measured root cause — never on a clean re-run, which this merge's
      426/426 explicitly is NOT evidence against.
- [ ] **MC-R3 stays guard-only:** the corpus executes 9/9 and must not regain a
      silent skip or an untyped expectation.

### T5 — carried forward from the UX program, and one debt this merge created

- [ ] **`DetachComponent` has no UI entry point.** Its backend and service seam
      are complete and proven, the manifest row says `uiExposure: "hidden"`
      because of it, and the mock lane still throws. Flipping the row to
      `exposed` requires a menu item AND an e2e spec **together** — the coverage
      verifier will reject one without the other, which is the intended pressure.
- [ ] D6's discriminated `ToolEditorDescriptor` union.
- [ ] Shell's thickness handle (needs a face normal on the prepare response).
- [ ] U8 typed references.

### Not owed

The worker's `capabilities[]` and the stub's verb parity were completed during
the merge; `scoringVersion 3`, region identity V3 and the mesh-limit contract are
consistent across both tracks. No fixture bump is outstanding.

## GEAR GENERATOR — G1 framework + involute spur (2026-08-14) — IN PROGRESS

Plan: `~/.claude/plans/enumerated-nibbling-crescent.md`. The framework phase —
schema, Rust, worker op and frontend for a committable involute spur gear.

### USER-VISIBLE CHANGE DECISION (required by `src/test/contracts/README.md`)

**A new `gear` tool is added to the model toolbar**, so two frozen contracts
change deliberately — this is a product addition, NOT a refactor being made to
pass:

- `toolbarContract.ts` gains `{ id: "gear", icon: "circularPattern",
  label: "Gear", shortcut: "⇧G" }` in the **model.solid** group after `Combine`.
  A gear MINTS a body, so it belongs with the solid creators rather than the
  modifiers.
- `keymapContract.ts` gains `⇧G → tool("gear")` (model scope). `G` unshifted is
  the sketch-mode polygon tool and is untouched.

**ICON DEBT:** `circularPattern` is borrowed for its radial-repetition reading.
There is no gear glyph in the registry and authoring one is a design pass, not a
side effect of this work — the same "no new icon was minted" call already made
for OffsetFace (`pushpull`). A proper gear glyph is owed.

### Landed so far

- **SCHEMA §7.3 `Gear`** + `opType` enum entry + §14 changelog. Recipe-conditional
  payload with null-spelling (Hole's contract), placement that is exactly one of
  face/frame, TierB publication, and the referenceability rule stated normatively.
- **Rust** `KnownOperation::Gear` with `GearParams`/`GearPlacement`/`GearFrame`/
  `InvoluteExternalParams`, `validate()` checked both ways, `derive_inputs`
  (placement only — a gear has no host), `element_refs_mut`, `scalars_mut`
  (dimensions only; coefficients like `clearance` are deliberately NOT
  variable-drivable), `KNOWN_OP_TYPES`, and `validate_gear` at both authoring
  entry points. 15 unit tests.
- **Worker** `kernel/geometry/BSpline` (the first member of the new geometry
  capability layer), `ops/gear/GearTool` (profile -> wire -> face -> prism ->
  bores), `ops/GearOp` (placement resolution, D1 mint, TierB publication) and the
  `PlanExecutor` dispatch arm. ctest **131/131**.
- **Frontend** types, the `gearParams` wire mapper, the `gearOp` PREVIEW builder
  (same rules, per SCHEMA §7.6's one-mapper rule), `gearMachine.ts` FSM,
  tool/keybinding/id registration, and the mock-lane op. vitest **4422/4422**.

### Gate evidence (measured 2026-08-14)

- `cargo fmt --all --check` clean · `cargo clippy --workspace --all-targets -D warnings` clean
- `ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` **1259 passed / 0 failed**
- `ctest --test-dir worker/build` **131/131** (was 119 before the gear work)
- `bunx tsc --noEmit` clean · `bun run build` clean · `bun run test` **4455/4455** (271 files)
- hex gate empty

The compiler did real work here: adding the variant broke FIVE exhaustive matches
(`scalars_mut`, `derive_inputs`, `dto.rs` × 2, and `wire.rs`'s deliberate
`_covered` guard whose comment says "a new variant fails to compile below until
it is given a decision"). Each needed a real decision, not a wildcard — notably
`wire_op_inputs`, where a Gear contributes ONLY its placement face because it has
no host body to echo.

### STILL OWED for the G1 gate ("involute spur committable from UI")

The FSM, wire mapper, preview builder, tool registration and mock lane are done
and tested (33 gear-specific frontend tests, including a byte-identical
preview==commit assertion). **The controller + chip UI is not built**, so the
tool can be activated but has no parameter surface yet. This is wiring, not
design — the Hole surface is the template throughout.

**G1-h.1–h.4 DONE (2026-08-14), redesigned mid-flight — see below.**

- [x] **G1-h.1 `ModelToolController` methods.** `startGear` · `tryPickGearFace`
      (promotes TopoKey → ElementId first, mirroring `tryPickHoleFace`) ·
      `onGearEvent` · `openGearPreview` · `commitGear` · `cancelGear` ·
      `editGearFeature` seeded through `gearFsmFromParams`. `gearInputs()` is the
      one structural divergence from Hole's mirror: a Gear has no HOST body to
      echo (SCHEMA §7.3 / `wire_op_inputs`'s Gear arm), so it contributes only
      its placement face.
- [x] **G1-h.2 `toolChipStore.showGear` + `GearChipHandlers`/`GearChipOpts`**,
      mirroring `showHole` — `value`/`count` carry module/teeth (reused, not
      duplicated); 14 gear-prefixed fields carry the rest.
- [x] **G1-h.3/h.4, REDESIGNED.** The first pass put all 16 involute fields in
      the floating chip (Base/Tooth/Bores/Accuracy segments, mirroring Hole) —
      the user saw it live and correctly rejected it: a single floating strip
      that wide is unreadable. Landed instead, FreeCAD-precedented:
      - `features/toolbar/GearChipCluster.tsx` is now MINIMAL — `Gear · {teeth}T
        m{module}` + the shared ✓/✕, nothing else.
      - `features/inspector/GearPropertiesPanel.tsx` (new) hosts every field, in
        the right-sidebar properties panel (`InspectorPanel`), reading/writing
        the SAME `toolChipStore` gear state + `onXxx` handlers the chip would
        have — one state, two views. Renders whenever `toolChipStore.kind ===
        "gear"` (a fresh placement OR an `editGearFeature` re-edit), taking
        priority over every other panel state.
      - `features/inspector/GearSelectedSummary.tsx` (same file): selecting an
        EXISTING Gear's HISTORY ROW (`sel.kind === "feature"`, `opType ===
        "Gear"`) shows a read-only params card + an "Edit parameters" button
        that calls `editGearFeature` — the FreeCAD "select an object, see its
        properties" moment the user asked for, second-hand off the request.
      - **NOT wired: selecting the body in the TREE** (as opposed to its history
        row). There is no bodyId→authoring-featureId correlation ANYWHERE in the
        projection to resolve it from — confirmed by reading
        `HistorySelectionSection` (`sections.tsx`), whose own doc comment
        already admits its body branch is `features.slice(0, 3)`, a stub, not a
        real per-body lookup. `body_<opId>`'s `opId` is the WORKER's plan-step
        id, not the record's own — checked live on the mock lane (a committed
        Gear's `bodyId` was `body_<preview-session-uuid>`, not `body_mf100`),
        so the tempting "strip the `body_` prefix" shortcut is wrong, not just
        untested. A real fix needs a projection field (`BodyDto` or a sibling
        map) carrying the minting feature's id — cross-cutting (Rust dto.rs +
        mock + `types.ts`), owed as its own WP, not bolted on here.
      - Manually verified end to end on the mock lane (`playwright-cli`): arm →
        sidebar edits teeth/module/bores live → ✓ commits (rev advances, no
        error) → history row reads `20T m2` → row click shows the read-only
        card → "Edit parameters" swaps to the live panel → ✓ re-applies.
      - **Real bug caught in passing**: `dto.rs feature_value` had no `Gear` arm
        at all (fell into the wildcard `text_only("")`) — a REAL-backend Gear
        row would have shown BLANK, while the mock lane already formats
        `"{teeth}T m{module}"` independently. Added the arm — `text_only`, NOT
        `dimensioned`: `HistoryList.tsx displayValue` prefers `primary`+
        `primary_kind` over `text` outright once `primary` is set, so a
        `Some(module)` primary would have silently DISCARDED the "20T m2" text
        and shown a bare "2 mm" instead. Caught before shipping, pinned in
        `dto.rs`'s `primary_value_is_the_row_editable_dimension_per_op_type`
        test. Gear is deliberately NOT in `featureValueEdit.ts WIRE_FIELD`
        either (no inline one-number edit) — same reasoning `canBindFeatureValue`
        already documents for Hole: `commitGear` wholesale-replaces the op from
        the FSM, so a merge-patch inline edit would silently discard any other
        field a real per-field editor might set. `cargo test --workspace` full
        run green (0 failed) after the change.
- [x] **Free-space placement, added post-h.4 (user caught it live).** The FSM's
      `pickFrame` path (mutually exclusive with `face`, SCHEMA §7.3
      `GearPlacement`) existed since G1's framework phase but had no UI trigger
      — a click that missed every face just errored "click a flat FACE". Fixed:
      `tryPickGearFace` now falls back to `tryPickGearGround` on a miss (no hit,
      not a face, or a preview body) — `engine.screenRay` → `groundPlanePoint`
      (`@/modules/library/placementSolver`, world Z=0 ground plane), the EXACT
      chain the Component Library's own free-space drop
      (`placementController.ts`) already uses; not new math. Frame =
      `{origin: hitPoint, axis:[0,0,1], xDir:[1,0,0]}` — world Z-up identity,
      same convention a component with no attachment uses. Re-clicking ground
      while already frame-armed moves it (no new preview session), mirroring
      the face branch's "same face" fast path. Verified live: chip arms at the
      clicked empty-space point, sidebar edits, ✓ commits clean (`rev` advances,
      `error=none`).
- [x] **Moved into a new title-bar "Generators" menu, added post-h.4 (user
      called out the toolbar as the wrong home for it).** Plan:
      `~/.claude/plans/fuzzy-leaping-rocket.md`. Gear was a body-mint tool
      mixed into the general `model.solid` toolbar group alongside
      Extrude/Fillet/Combine; the user wanted it in a dedicated top-bar
      dropdown that is explicitly meant to be **the future home for more
      generator types** (FreeCAD keeps its gear family in its own toolbar, not
      the general one). Investigated splitting into per-recipe buttons
      (FreeCAD has ~13) first — scoped down to Spur-only for now: only
      `involuteExternal` has any backend (SCHEMA/Rust/worker); Timing GT/HTD,
      Timing-T, Lantern, Worm are math-only (`worker/src/kernel/gear/
      *Math.cpp`, G0) with `GearOp.cpp` actively rejecting any other recipe
      string. Each becomes its own future WP (tool id + backend) that lands in
      the SAME menu automatically once built — no more shell edits needed.
      - **No new registry** — `platform.tools` already carries every
        registered tool. Two new optional `ToolDefinition`/`ToolPresentation`
        fields (`toolbarHidden`, `generatorsMenu`), threaded the same way
        `flyout` already is. Precedent for "registered but toolbar-invisible"
        already existed (`FloatingToolbar.tsx`'s `hiddenToolGroups` workspace
        filter) — this generalizes that shape into a real field instead of a
        runtime-only override.
      - `GeneratorsMenu.tsx` (new, `src/features/shell/`) mirrors
        `WorkspaceSwitcher.tsx`'s registry-projection pattern exactly (not
        `FileMenu.tsx`'s hardcoded rows) — reads `platform.tools` filtered to
        `generatorsMenu === true`, activates via `platform.toolHost.activate`
        (identical path a toolbar click takes, never a hand-rolled
        `setTool`), hides itself entirely when nothing has opted in. Zero
        modeling-specific imports — a future non-gear generator from a
        different module appears with no edit to this file.
      - `⇧G` and ⌘K discoverability are UNCHANGED (per explicit product
        decision) — `toolbarHidden` only removes the `FloatingToolbar` row;
        the tool stays fully registered.
      - `MODEL_TOOLS_CONTRACT` (`toolbarContract.ts`) lost the Gear row;
        `registryToolbar.ts`'s golden PROBE gained a `toolbarHidden` skip so
        it keeps reflecting production, and `ids.test.ts`'s "toolbar contract
        == id map" assertion was corrected to "toolbar contract == id map
        MINUS toolbarHidden entries" — a real invariant fix, not a probe
        edit, since the id map (shortcut/palette reachability) and the
        toolbar's visible set are now deliberately different sets.
      - New `GeneratorsMenu.test.tsx`, boots the REAL modeling module
        (`renderWithPlatform`) so Gear's own flag is what proves the row
        exists, not a test fixture.
      - Verified live via `playwright-cli`: Gear icon gone from the model
        toolbar; "Generators ▾" between File and Design; opens to a "Gear ⇧G"
        row; click arms the SAME facePick/ground-plane gesture as before;
        `⇧G` still arms it directly.
      - Gate: `tsc` clean, `bun run test` 272 files / 4459 (up from 271/4456),
        hex gate empty.
- [x] **Real gear glyph, added post-h.4 (user flagged the borrowed icon).**
      `icons/authored.ts gear` — an 8-tooth rim (primary) + accent axle bore,
      same "one verb gets the accent" grammar as `hole`'s drilling arrow.
      Replaces the `circularPattern` borrow in `tools.ts`, `toolbarContract.ts`.
- [ ] **G1-g.1 `src-tauri/tests/gear_ops.rs`** (real sidecar): preview == commit,
      save/reopen identity, param re-edit incl. a tooth-count topology change,
      placement rebind + the 1e-3 mm fence, cross-body ref isolation.
- [ ] **G1-g.2 `protocol/fixtures/gear_*.ndjson`** — `execute_plan` + `preview_op`
      + error paths, against BOTH the worker and the stub.
- [ ] **G1-g.3 `e2e/gear.spec.ts`** (mock lane): entry → pick → chip → commit →
      history row → re-edit seed. Geometry assertions stay in the layers below.
- [ ] **G1-h.5** re-run the full four-suite gate + the cross-check harness.

Owed but NOT blocking G1: a real gear icon (see the icon-debt note above), and
`Fillet2d` with geometric site identification.

### Deliberate scope limits, each refused BY NAME rather than silently ignored

- `helixAngleDeg` ≠ 0 and `doubleHelix` are **UNSUPPORTED** in this version
  (they need the Frenet sweep infrastructure). The fields exist and round-trip so
  the payload will not change shape later; the FSM does not offer them as
  controls at all, because a control that always fails is worse than none.
- Fillets (`headFillet`/`rootFillet`) are **not implemented**. `Fillet2d` with
  geometric site identification is still owed; the plan scheduled it here, and it
  is deferred rather than done badly — upstream's index-based insertion is
  exactly the fragility this port refuses to carry.

## GEAR GENERATOR — G0 gear math core (2026-08-14) — GATE PASSED

Plan: `~/.claude/plans/enumerated-nibbling-crescent.md`. Port of
`freecad.gears` v1.3.0 (GPL-3.0) as a native `Gear` operation. **A separate
feature track, deliberately NOT jumping the T0–T2 queue above** — G0 is
worker-only, adds no Rust/frontend/schema surface, and nothing in it blocks or
is blocked by the data-integrity work.

Approved scope (user decisions taken before G0): op named **`Gear`**, not
`Generator` — `ComponentSourceRef::Generator` already exists in `record.rs` and
the collision would be permanent. Recipes in scope: involute external
(spur/helical), timing GT/HTD, timing-T, lantern, worm ZA. Out: racks, internal,
cycloid, bevel, crown, hypocycloid cam. Bore-face referenceability is IN (G1).

**Three corrections to the source specification**, verified against live code —
each would have been a real defect if built as written:
- **TierA/TierB were inverted.** The spec has gears publishing at `TierA` for
  "deep validation from day one". `TierA` is the LIGHT tier; `TierB` is the one
  running `BRepAlgoAPI_Check` self-interference (`ShapeAudit.cpp:220-274`), and
  Fillet/Chamfer already use `TierB`. G1 must use `TierB`.
- **`crates/onecad-core/…` is not a path in this repo**; Rust lives under
  `src-tauri/crates/…`.
- **`§7.3` is not a reservable slot** — it is one section holding every op's
  payload as a `####` subsection. A new op appends, it does not claim a number.

Also: ordinal child-body minting (`body_<opId>:<k>`) already exists and is
general (`OpCommon.h` `ranked_solids`/`publish_boolean_result`), so a future
multi-body recipe reuses it rather than building it; and the element map is
mint-on-demand (`ElementMapPartition.h:11-13` — "entries exist ONLY for elements
referenced by an op input or minted on demand"), so "teeth are not referenceable"
costs one guard at the `AcquireElementIds` promotion site, not new infrastructure.

### Landed

Five pure-C++ samplers under `worker/src/kernel/gear/` (namespace
`onecad::kernel::gear`), **no OCCT by construction** — the `PolygonFill`
precedent. `GearTypes.h` carries the shared 2D vocabulary and the
line/arc/interpolated segment kinds the G2 wire builder will consume.

- [x] `InvoluteMath` — factors, involute + trochoid-undercut sampling, tooth assembly
- [x] `TimingMath` — GT/HTD, the 7-row normative section table, 4/6-arc branches
- [x] `TimingTMath` — trapezoidal T profile
- [x] `LanternMath` — offset-involute (parallel curve) flank
- [x] `WormMath` — ZA trapezoidal thread wrapped to the cylinder
- [x] `scripts/gear-crosscheck/` — the §10.1 leg-2 harness (see its README)

### Two improvements on upstream, both deliberate and documented

- **Timing-T flank endpoints are closed form.** Upstream runs
  `scipy.optimize.minimize` on a squared distance whose true answer is a
  line∩circle intersection. The objective is flat at its optimum so it stops
  early — **up to 5.1e-6 mm of measured error**. Solved exactly here; root
  selection is justified geometrically, not fitted.
- **The lantern root solve is bracketed.** Upstream's objective has a SPURIOUS
  ROOT AT φ=0 for every parameter set, and upstream calls `scipy.optimize.root`
  from a heuristic guess with nothing preventing it landing there. Differentiating
  in closed form gives `g'(φ) = 2(1−cos φ)(r₀φ − r_r)`, which proves the
  meaningful root is unique on `(r_r/r₀, ∞)`; the solve brackets there, so the
  degenerate root is excluded by construction rather than by luck.

### The finding that justifies the harness existing

**The reference's undercut trim branch is unreachable.** `InvoluteTooth.points`
computes `s = trimfunc(...)` then guards on `isinstance(s, ndarray)` — but
`trimfunc` returns a Python *list*, so the guard is never true and the trimmed
result is discarded every time. Every undercut gear freecad.gears has ever
produced silently took the `nearestpts` fallback. This port first implemented
the *intended* behaviour and disagreed with the reference on **224 of 2673**
swept cases. Now matches reachable behaviour, documented in `InvoluteMath.h`
with the reasoning for choosing parity over the "better" geometry.
**Reading the source did not catch this. The sweep did.**

### Gate evidence (measured 2026-08-14)

- **Cross-check: `cases=2697 compared=2673 refused=24 mismatched=0`** — full
  agreement with the reference across the sweep.
- All 24 refusals reviewed via `--show-refusals`: one condition
  (`module=2, diameter=5, clearance=0.25` → worm root radius exactly 0), correctly
  fail-closed where upstream emits a degenerate shape.
- **`ctest --test-dir worker/build` 129/129**, up from 119; five new gear tests.
- Every sampler compiles standalone under `g++ -Wall -Wextra`, zero warnings.
- **Mutation-proved**, not just green: ~30 mutants across the five files, all
  killed. Three real gaps were found and closed this way — `head` was inert in
  every involute oracle case, worm arc interior points were checked for radius
  but not angular position, and one surviving mutant was measured (≤3.6e-12 mm)
  and recorded in-code as genuinely equivalent rather than papered over.

### Defect found in the repo's existing test convention

`test_polygon_fill.cpp`'s "exit code == failure count" idiom is **unsound for
large failure counts**: POSIX truncates exit status to 8 bits, so a run failing
exactly 256 (or 512, …) assertions exits `0` and reports PASS. Hit for real —
a mutant printing hundreds of FAIL lines exited clean. All five gear tests clamp
their status. **`test_polygon_fill.cpp` still has the raw idiom**; it is
currently safe only because its assertion count is small. Worth fixing when that
file is next touched.

### Owed before G1

Nothing blocking. Not yet done, and out of G0's scope by plan: no schema, Rust,
worker-op or frontend surface exists yet — G0 is math only. `docs/` is untouched.

## MERGE — `OneCAD-Component-Library` × `master` (2026-08-14) — GATE PASSED

Two programs ran in parallel from merge-base `39f5839` (2026-08-13): master's
modeling-UX unification U0–U7 plus the publication/identity-V3 hardening and the
data-integrity audit (10 commits), and this branch's whole Component Library
program plus document variables (28 commits). 159 vs 185 changed files, 27
overlapping. Resolved INBOUND (`git merge master` on the branch) so the shared
`master` worktree at `../OneCAD-Tauri` was never left broken; master lands by
fast-forward.

`git merge-tree` predicted the conflict set exactly: **8 files**, seven of them
"both sides appended at the same anchor".

- [x] **Trait/command unions** — `worker/mod.rs` (×2), `worker/manager.rs`,
  `api/mod.rs`, `lib.rs`. Master's `query_body_topology` and the branch's
  `classify_element`/`classify_element_by_topo_key` now sit side by side in
  `ElementQuery`, in `PendingBackend` and in `WorkerManager`; `generate_handler!`
  carries `api::query_body_topology`, `api::classify_element` and all 13
  `library::*` commands. **Losing a trait method is a compile error; losing a
  command is SILENT** — the frontend's `invoke` just 404s at runtime.
- [x] **`protocol/SCHEMA.md` §14** — both sides prepended entries dated
  2026-08-13; concatenated, master's block first.
- [x] **`TODO.md` / `HANDOFF.md`** — both rewrote their heads wholesale. Rebuilt
  as sibling threads over the shared tail: master's UX/data-integrity block, then
  the Component Library block. `HANDOFF.md` also drops a pre-existing branch
  artifact (a duplicated Platform-refactor H1 with no body) and adopts master's
  `## Session 5` seam, so the file has ONE H1 per thread again.
- [x] **`DimensionInput.tsx`** — the only real logic conflict. Master's live
  numeric entry (`onPreview`/`initialText`, the strict `parse()`, the
  seed/echo/unit guards) and the branch's `=name` binding (`onCommitExpr`/`expr`,
  `parseExprInput`) are ORTHOGONAL — no caller passes both — so all four props
  are unioned rather than chosen between. Three decisions worth naming:
  a seed outranks a binding in the initial text (typing a digit IS the unbind
  gesture); a binding change joins a unit switch as a re-label EXEMPT from the
  echo guard (new `lastExpr` ref — without it the merge is only accidentally
  correct); and a half-typed `=` never previews. `commit()` keeps the expr branch
  FIRST and master's strict `parse()` second, so `"12abc"` still refuses.

### What auto-merged and still broke

- [x] `dto.rs` — the branch's `feature_dto_omits_primary_expr_when_unbound` built
  a full `FeatureDto` literal predating master's `diagnostics` field (E0063). The
  **only** hard compile error in the merge.
- [x] Verified rather than trusted, because each fails SILENTLY:
  `regionIdentityVersion: 3` survived in `localSolver.ts` and `mockClient.ts`
  (master's V3 bump landed in a region the branch moved by +723 lines);
  `projectionHydration.ts` carries BOTH `primaryExpr` and `diagnostics`;
  `support/mod.rs` has master's `with_event_mutation` beside the branch's
  `mate_placement`; all three new verbs are registered in `main.cpp` and all six
  branch sources are in `worker/CMakeLists.txt`.

### The QA evidence gate (what would actually have broken CI)

Not a conflict — a cross-check. `verify-modeling-coverage.mjs` fails on any op a
registry knows and the manifest does not, and THREE of its scanners see the new
ops (`KnownOperation`, the `PlanExecutor.cpp` dispatch regex, the SCHEMA §7.3
catalogue). The branch never touched `docs/qa/`.

- [x] `PlaceComponent` — coverage + contracts rows, `uiExposure: "exposed"`,
  citing `test_component_ops.cpp`, `component_ops.rs`,
  `placementController.test.ts` and both library e2e specs.
- [x] `DetachComponent` — rows with `uiExposure: **"hidden"**, and no browser
  lane, because that is the truth: the backend and the `CadClient`/service seam
  are complete and proven (`detach_component_preserves_body_and_volume_across_the_swap`),
  but **nothing in the UI calls it** — no menu item, no command, and the mock lane
  still throws. Claiming `exposed` would have forced a fabricated Playwright
  citation. Ticking it later needs a UI entry point AND a spec, together.

### Policy gaps closed in the same landing (approved scope)

These are defects the merge EXPOSED — each is one program's invariant meeting the
other program's code for the first time.

- [x] **SCHEMA §7.2 mate carve-out.** Master wrote "a `completed` stream contains
  only `ok` rows"; `PlaceComponent` has always published an `ok` row carrying
  `planStep.needsRepair[]` when its mate could not re-seat. Both deliberate,
  never reconciled. §7.2 now says a published step MAY carry `needsRepair[]`
  **only** for a mate, never for a topological input it built from — because a
  component's geometry does not depend on its mate, so failing the step would
  destroy a valid body to report a placement problem. No code changed on either
  track; Rust's `validate_prepared_stream` already accepted this shape.
- [x] **`PreviewOp.cpp` obeyed no transport limit.** Master capped
  `attach_tessellate` at `kChunkSize`/`kInitialBulkCredit` and wrote that promise
  into §14 — but `PreviewOp` inlined its blob uncapped, and that is exactly the
  lane the library preview drives (`library.rs`, `Lod::Medium`, over generator
  output that can carry modeled helical threads). Now DEGRADES the LOD until the
  mesh fits and reports the tier it actually used, failing by name only when even
  `coarse` is over budget. Degrade, not refuse: a preview is an illustration, and
  an error would blank a catalog card for a component that places fine.
- [x] **`validate_modeling_input` on both component ops.** Master added the
  Tier-A preflight to all ten other mutating ops; `PlaceComponent` — the one op
  that consumes an *untrusted* BRep blob out of a package — had none.
- [x] **Result truth for the component lanes.** `placeComponent`,
  `setComponentParams` and `detachComponent` used a bare `call()`, so they never
  learned their own regen terminal: a placement whose regen failed reported the
  same silent success as one that seated. All three now go through `applyEdit`
  and return `ApplyOperationResult`; `placementController` and
  `ComponentParametersSection` read the verdict via `classifyRegen`.
- [x] **`featureValueEdit.ts` reported false success.** It checked only
  `errorMessage`, which master's U1 correlation made insufficient — a regen that
  publishes while THIS record fails carries `terminal: "failed"` and no message,
  so a `=name` binding that never took rendered as bound. Verdict first, hydrate
  second, mirroring `treeActions.ts`.
- [x] **Worker announcements.** `make_hello_result`'s `capabilities[]` had
  drifted — six ops shipped unannounced — so it was completed rather than
  extended by three and left half-true. `onecad-worker-stub` learned
  `ClassifyElement` and `ExportGeometry` (the bake writes a real file and reports
  its real byte count; a stub that faked `bytes` would make the verb untestable).

### Open, deliberately not in this merge

- [ ] **Result truth for `upsertVariable` / `removeVariable` / `replaceComponent`.**
  The last three kernel-touching commands still exempt from the `regenOutcome`
  doctrine. They differ from the component lanes in KIND, not degree: they return
  `Vec<VariableDto>` / `ReplaceComponentReportDto` rather than a projection, and
  fire their regen asynchronously through `sched.handle`, so there is no terminal
  to correlate without a wire change. Two options, both needing their own gate:
  (a) add `variables` to `DocumentProjection` and route them through `applyEdit`
  like everything else — cleanest, but it moves a DTO the golden tests pin;
  (b) return the terminal alongside the existing payload. Either way a product
  call is owed first: a variable edit that SAVES while its downstream regen FAILS
  has two truths, and the UI currently states only the first. Do not "fix" this by
  throwing on a failed regen — the variable really was saved.

### Gates

- [x] `ctest` — **124/124**, worker rebuilt against the merged sources.
- [x] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -D warnings` — clean.
- [x] Worker-backed `cargo test --workspace` — **1244 passed / 0 failed**, 82 binaries,
  `ONECAD_REQUIRE_WORKER=1`.
- [x] `bun run build` + `bun run test` — **4422 passed / 0 failed**, 269 files.
- [x] `verify-modeling-coverage.mjs`, its negative-control selftest, and
  `verify-modeling-contracts.mjs` — all three clean.
- [x] Hex gate (`grep -rn '#[0-9a-fA-F]\{6\}' src`) — empty.
- [x] `bun run e2e` (mock lane, **both** projects, `retries: 0`) — **426 passed /
  0 failed**, 31.7 min, no failure artifacts written.
- [ ] Real-Tauri WDIO composition and a manual Tauri smoke are unrun for this
  merge, as they were for each side separately.

Two test fixtures were updated, both for the same reason and neither to make a
failure go away: `featureValueEdit.test.ts` and `InspectorPanel.test.tsx` mocked
edit results with no `terminal`, which `types.ts` already declares no production
result may omit. The verdict now reads that field, so a fixture without one
described a result the backend cannot produce.

## DATA-INTEGRITY AUDIT (2026-08-14) — INVESTIGATION ONLY, NO FIX LANDED

Question asked: **can OneCAD lose or corrupt a user's document, and how?** Eight probes over the
persistence lane. Two probes are committed as executable evidence; everything else is a read with
`file:line`. **No fix is implemented — which of DI-1…DI-5 gets built, and in what order, is a
product call.**

### Why this ran at all — the ledger was wrong in BOTH directions

The two entries this repo carried as open data-integrity defects turned out to be closed:

- **VF-M6** (`imports.rs` blob lifecycle, 3 defects) was fixed 2026-08-08 and the box was never
  ticked. Six days as a phantom open bug.
- **The W5 promoted-id seam** is real as a mechanism but its consumer was deleted in `1fe0cef`;
  it is latent, not user-reachable.

Both are corrected in place. The rule this suggests: an open box that cites `file:line` must be
re-verified against the live tree before it is believed — line numbers drift, and a fix landing in
another package does not tick anyone else's box.

### Verified SAFE (each with the evidence that would have caught the bug)

- [x] **Crash recovery exists and is well built.** 30 s *debounced* autosave (not a fixed cadence),
      pid-stamped session marker, and both writers serialized on one persistence lane with the
      documented lock order runtime → release → lane (`src-tauri/src/autosave.rs:1-40,62,124`).
      Zero autosave activity with no document open.
- [x] **The container write is atomic.** Sibling temp → `fsync` → `rename` → parent-dir `fsync`
      (`onecad-core/src/io/container.rs:37-44,383`), with a crash-between-temp-and-rename test at
      `:1367` and a dedicated `save_to_temp_for_test` seam at `:339`. A crash or a full disk cannot
      leave a short, valid-looking `.onecad`.
- [x] **Import blob refcount vs undo is not a loss window.** A save drops blobs no live record
      names, but the in-session carrier is INSERT-ONLY (`document_runtime.rs:2114,2184,2256` — no
      remove/retain/clear anywhere) and the undo stack is memory-only (`edit/undo.rs:317`, nothing
      in `container.rs` persists it). So the sequence that would strand a record — remove import →
      save → undo → regen — still has the bytes. Suppressed and rolled-back records pin their blobs
      deliberately (`io/imports.rs:179-183`).
- [x] **A save cannot capture a half-committed regen.** `build_save_payload` runs under the runtime
      lock; only the container write is off-lock, on a blocking thread, under the lane
      (`autosave.rs:27-40`). Regen commits under the same lock in `finish_regen`.
- [x] **Unknown module state round-trips verbatim** per ADR-0004, pinned by
      `container.rs:1296 unknown_module_state_survives_open_modify_save` plus an odd-payload case
      at `:1345`.
- [x] **Every `EditCommand` has a real inverse.** The `Inverse` family covers records, cursor,
      sketches, bodies, datums, variables, module state and repair, with `Composite` for
      multi-subsystem commands (`edit/undo.rs:48-127`). The one `Inverse::Noop` producer is a
      genuine no-op (`edit/session.rs:900` — rolling to the row the bar already sits on).

### FINDINGS — recorded, not fixed

- [ ] **DI-1 (HIGH) — a recovered document is unprotected against the NEXT crash.**
      `api::recover_document` consumes the crash marker (`api/mod.rs:796`) while deliberately
      keeping the autosave file, on the stated reasoning that "the autosave file itself is kept so a
      re-crash before the next tick still recovers". That reasoning is false: discovery iterates
      `*.session.json` MARKERS (`io/recovery.rs:140-181`), so a kept autosave with no marker is
      unreachable. **Probe committed:**
      `io::recovery::tests::an_autosave_whose_marker_was_consumed_is_not_offered`, with the
      marker-present control right above it. Fix options differ in their stale-offer profile: keep
      the marker until the next autosave supersedes it · write a fresh marker at recovery · scan
      autosave files as a fallback. ~½ day including the lane test.
- [ ] **DI-2 (MEDIUM-HIGH) — `recover_document` never ticks the autosave loop.** It sets
      `dirty = true` (`document_runtime.rs:2073-2076`) but does not call `state.note_mutation()`,
      and the autosave loop is driven ONLY by that tick — it never reads `is_dirty`
      (`autosave.rs:255-282`). Protection is incidental, from the tick a *published* regen bumps
      (`lib.rs:174-176`). If the recovery replay fails, no-ops or is cancelled, nothing is
      autosaved — and by DI-1 there is no marker either. One line to fix; the value is in the test.
- [ ] **DI-3 (MEDIUM) — two commands mutate persisted state with no tick and no dirty flag.**
      `promote_selection` (`api/mod.rs:1378`) and `prepare_edge_op` (`api/mod.rs:2207`) both write
      `regen.elements` (`document_runtime.rs:3260`), which `build_save_payload` persists as
      `doc.elements` (`:1914`). Neither calls `note_mutation`, and Rust reports `dirty:false`, so
      `appStore.requestClose` takes the clean fast path and closes without a prompt. The rows are
      identity plumbing rather than user intent, which is why this is not HIGH — but "persisted by
      save, invisible to both the autosave tick and the unsaved guard" is a state no other mutation
      is in. Audit rows: all 55 Tauri commands were classified; these two plus DI-2 are the only
      mutating rows without a tick, and no non-mutating row has a spurious one.
- [ ] **DI-4 (MEDIUM) — an authored FACE COLOUR survives as data but stops being paintable after a
      reopen.** `SetFaceColor` keys on the Rust-minted `ElementId`; the frontend paints it through
      exactly two paths (`meshSync.resolveAuthoredFaceColors`) and both bottom out in the worker's
      element-map partition, which is minted on demand and dies with the process. `BindElementIds`
      has ONE production call site — inside `promote_selection` — and nothing re-binds a persisted
      id at open. **Probe committed:** `src-tauri/tests/face_color_reopen.rs`, real worker, save →
      fresh worker → reopen. Measured: the colour is still in the reopened projection under the same
      id, the mesh id table carries TopoKeys (`idsHaveElementIds:false`) and `elementInfo` answers
      `None`. In-session controls sit beside both measurements, and the assertion was
      **mutation-proved** (inverting it reds the test). Same root as the W5 seam, so one fix —
      re-binding persisted ElementIds at open — closes both.
- [ ] **DI-5 (LOW-MEDIUM) — the STEP round-trip is lossy on the way out.** Import keeps XCAF names
      and colours (BinXCAF replay, `SCHEMA.md:2373`), but export is a bare `STEPControl_Writer`
      (`worker/src/io/ExportStep.cpp:12,52,59`) with no XCAF document, so names and colours are
      dropped. A file that came in coloured leaves grey. Relevant to the machined-parts priority,
      where the exported STEP is the deliverable.

Gates for the audit itself (measured 2026-08-14): `cargo fmt --all --check` clean · `cargo clippy
--workspace --all-targets -D warnings` clean · worker-backed
`ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` **1101 passed / 0 failed across 76
result lines**, including the two new probes. (That count is not comparable to the 810/69 figure in
the session-8 ledger — this run tallies every result line, doc-tests included. The number that
matters is zero failures.) The `face_color_reopen` assertion was mutation-proved: inverting it reds
the test, reverting it greens it again. No frontend or browser gate is owed — no `src/` file
changed.

## NOW — MODELING UX UNIFICATION, U0–U7 (2026-08-14, plan `~/.claude/plans/bright-munching-oasis.md`)

Source: the two documents in `UX/` — the senior UX audit (2026-08-11) and the b022edf7-pinned
hardening specification. **The spec is 21 commits stale**; HEAD is `4ac8565`. The plan is the
re-verified DELTA, not the spec as written. Re-verification found three defects the spec and
`TODO.md` both record as CLOSED that are only PARTLY closed, plus six that are in neither document.
Full drift ledger + evidence in the plan file. Approved scope: U0–U7 (U8 typed references queued).

### U5's browser lanes — RUN, both green (2026-08-14, session 9)

- [x] **Both full lanes, retries 0, one at a time: `chromium 200/200` (14.2 min, `E2E_PORT=4191`)
      and `webkit 200/200` (8.6 min, `E2E_PORT=4193`).** This is the gate U5 owed — it changed the
      transform gizmo's geometry (82px tori → ±26° arcs), its screen scale and the placement chip's
      anchor, and the browser is the only real check for all three. No failure in either lane, so
      no triage was needed; neither MC-R8 nor MC-R9 reappeared. Ports were verified free before
      each launch (a concurrent session has held 4177/4187 before, and a collision reads as
      `ERR_CONNECTION_REFUSED`, i.e. exactly like a product failure).
- [x] Gate commit of the U0–U7 tranche — authorized this session, **commit only, no push**.
- [ ] Manual Tauri smoke (spec §14) has never been run for this program; needs the native stack.

Carried forward, each with its reason in the package block below: D6's discriminated
`ToolEditorDescriptor` union · Shell's thickness handle (needs a face normal on the prepare
response) · U8 typed references · MC-R9 browser nondeterminism (MC-R8 is CLOSED — root-caused and
fixed in `19088d0`).

Approved product decisions, taken before U0:
- the frozen interaction contract gains `primaryEntry` + `livePreviewOnEdit` columns (TIGHTENING —
  more rows red, never edited afterwards to make an implementation pass);
- pattern count keeps a 2–12 stepper with direct numeric entry up to the worker's 128;
- ✓/✕ becomes the single confirm vocabulary for all twelve tools; `ApplyButton` goes away, and the
  body-lifecycle meaning it carried moves into U4's result-summary slot.

### U0 — red evidence + contract teeth (2026-08-14) — GATE PASSED

Tests only; **no production file changed** other than the frozen contract's two new columns (the
user-visible-change decision recorded above, per `src/test/contracts/README.md`). 32 new reds, each
paired with a green control so no red can be blamed on its fixture.

- [x] **Contract columns + self-check.** `modelingInteractionContract.ts` gains `primaryEntry`
      (`typeToEnter|clickFirst|none`) and `livePreviewOnEdit`. `modelingInteraction.golden.test.tsx`
      asserts every row declares Enter support and that both new columns track `primaryParameter`.
- [x] **D1 is only half-closed — 4 reds.** `modelingInteraction.golden.test.tsx`: boolean,
      linearPattern, circularPattern and mirror still publish `onApply` instead of the shared
      `onConfirm`/`onCancel` pair. Extrude + fillet are the green controls.
- [x] **The Enter column was never enforced — 4 reds.** New
      `modelingInteraction.keyboard.probe.test.ts` arms each of those four through its re-edit entry
      point and dispatches a real capture-phase Enter on `window`.
      `ModelToolController.onKeyDown:8022-8064` routes Enter for the other tools only, so nothing
      commits. Each has a control that fires the chip's own callback and reaches the client once.
- [x] **D3 + D4 — 4 reds.** New `modelingInteraction.numeric.probe.test.ts`: typing while armed does
      not reach the primary value (no router listens), and no keystroke rebuilds the ghost (blur/Enter
      only). Controls assert the chip published the seeded value, so the arm itself is proven.
- [x] **D9/WP1 is only partly adopted — 12 reds.** New
      `ModelToolController.terminalFamilies.test.ts`: transform, boolean, shell, offsetFace, fillet and
      hole re-edit each announce a RESOLVED `failed`+message and `needsRepair` as a NON-STICKY
      success. Six `published` controls green. Root cause recorded for U1: `applyOperation` mints a
      `recordId` only for `addOperation` (`tauriClient.ts:890`), so an `updateOperationParams` re-edit
      has no per-record correlation and a published-overall regen whose failing step is this record
      never settles as failed (`tauriClient.ts:505-547`).
- [x] **Two more families, same defect — 2 reds.** `treeActions.test.ts` (`dispatch` catches a
      REJECTION only) and `reattachActions.test.ts` (own `revision > before` heuristic).
- [x] **Boolean success selection contradicts its own contract row — 2 reds.** New
      `modelingInteraction.selection.probe.test.ts`: a split Cut selects the stored target instead of
      the child outputs (`ModelToolController.ts:7454`, `:7536`), and does so even when the commit
      FAILED. The contract row says `allChildOutputs`.
- [x] **D14 is only half-closed — 1 red.** `InspectorPanel.tsx:119-123` hardcodes
      `Under-constrained · DOF {dof}` with no status check and a `?? 0` default, so a
      fully-constrained sketch still renders the impossible string. `constraintStatus.ts` was fixed;
      this second authority was not. The existing `InspectorPanel.test.tsx` fixtures are DOF-3 only,
      which is why it went unnoticed.
- [x] **Candidate copy can contradict the candidate list — 1 red.** `RepairPanel.test.tsx`: the
      header count comes from the `needs-repair` event, the rows from a separate `resolveRefs`
      response, so "2 candidates" can sit directly above "No candidates to choose from" (audit
      finding 06).
- [x] **Body display labels are position-derived — 1 red (Rust).**
      `onecad-core/src/document/body.rs::fresh_body_names_never_collide_with_a_live_body`. Measured:
      delete `Body 1` of three, create one, get `["Body 2", "Body 3", "Body 3"]`. `default_name()` is
      `format!("Body {}", self.bodies.len() + 1)` — a position, not an allocation.
- [x] **The sketch origin is not a reference at all — 2 reds.** New
      `src/tools/sketch/originSnap.probe.test.ts`: `computeSnap` has no origin tier (endpoint /
      midpoint / center / quadrant / intersection / onCurve / grid / align / polar only) and
      `inferConstraints` never anchors an endpoint on (0,0). So the audit's "rectangle on the origin
      still reports DOF 2" is STRUCTURAL — D14 fixed only the label. `"Fixed"` already exists as a
      constraint type, so the fix needs no protocol change. A third spec is the green control: an
      oblique line away from the origin must still infer nothing.

Gates (measured, 2026-08-14): `bunx tsc --noEmit` clean · `bun run build` clean · `bun run test`
**255 files / 4231 tests — 32 failed, 4199 passed**, exactly the 32 intended reds and **zero
pre-existing tests broken** (baseline was 250 files / 4182 tests, all passing; +5 files, +49 tests =
32 reds + 17 green controls) · `cargo fmt --all --check` clean · `cargo clippy -p onecad-core
--all-targets -D warnings` clean · `cargo test -p onecad-core --lib` **263 passed / 1 failed** (the
intended red) · `verify-modeling-coverage.mjs` and `verify-modeling-contracts.mjs` pass · hex gate
empty.

NOT run at this gate, and not claimed: worker CTest, the worker-backed `cargo test --workspace`
lane, and both Playwright lanes. U0 changed no production behaviour, so none of them can move; they
are owed at the first package that does (U1).

Deliberately NOT covered by U0's terminal matrix: `confirmExtrude` (`:6784`) and the two revolve
paths (`:2639`, `:2724`), which use the legacy body-count inference rather than the classifier.
Their fixtures need a profile/region arm that the six re-edit families do not; U1 must add them
rather than treat the matrix as complete.

### U1 — result truth, completed (2026-08-14) — GATE PASSED

**Unresolved question 1 is ANSWERED: no wire change.** `updateOperationParams` already carries the
target record id (`WireEditCommand.record`), and `failed_steps_of` keys on `rec.record_id` for ANY
errored timeline record (`document_runtime.rs:3759`) with no add/update distinction. The re-edit
correlation was available all along and simply unused. `tauriClient.applyOperation` and
`applyEditCommand` now pass it, so a published-overall regen whose FAILING step is this record
settles as `failed` for its own awaiter. `removeOperation`, `setOperationSuppression` and
`setRollback` are deliberately NOT correlated — the first names a record that no longer exists, and
the other two act on DOWNSTREAM steps, so scoping the change to the named record's own bodies would
drop exactly the bodies that moved.

- [x] **All 11 bypassing sites now classify.** `commitTransform`, boolean re-edit + fresh, hole/shell/
      offsetFace/edge-op re-edit, `treeActions.dispatch`, `reattachSketch`, and both legacy
      body-count sites (`confirmExtrude`, `confirmRevolve` × 2). One new shared
      `settleScalarEdit()` covers the four `updateOperationParams` re-edits so they cannot drift.
- [x] **`classifyRegen` gained an explicit `noTerminal` option**, and this was a REAL defect found by
      the change: metadata-only commands (`RegenHint::None`) publish a projection, run no regen, and
      return no terminal — so the body-count fallback called every eye-click and every rename a
      FAILURE and reverted it. `treeActions` and `reattachSketch` pass `noTerminal: "published"`;
      every other caller keeps the historical inference verbatim.
- [x] **A `needsRepair` record is never rolled back and never reads as success**, now in extrude and
      revolve too (both previously rolled it back because it changes no bodies). Each family reports
      it as the ask it is: sticky, `info`, "… needs repair".
- [x] **Boolean success selection is `allChildOutputs`** per its contract row — new
      `booleanResultSelection()` selects everything the regen published for the record, falling back
      to the target only when it published nothing. Both boolean paths select ONLY on `published`, so
      a failed commit no longer moves the selection.
- [x] **The terminal matrix is complete.** `ModelToolController.terminalFamilies.test.ts` now covers
      extrude and revolve re-edit as well (the honest gap U0 recorded), 8 families × 3 terminals.
      Their fixtures needed a sketch/region read and a delivered mesh ingest, which is why U0 left
      them out. **Proved non-vacuous by mutation**: restoring `confirmExtrude`'s body-count check
      reds `extrude re-edit · failed with a reason` while its `published` control stays green.

Gates (measured): `bunx tsc --noEmit` clean · `bun run build` clean · `bun run test` **255 files /
4237 tests — 16 failed, 4221 passed**. The 16 are exactly U0's remaining reds (8 → U2, 4 → U3,
4 → U7); U1's own 16 went green and **no pre-existing test regressed**.

NOT run: worker CTest and the worker-backed `cargo test --workspace` (no Rust/C++ file changed) and
both Playwright lanes — owed at the first package that changes rendered behaviour (U2).

### U2 — one confirmation grammar (2026-08-14) — GATE PASSED

- [x] **`onApply` is deleted.** Boolean, both patterns and mirror publish the same `onConfirm` /
      `onCancel` pair as every other tool; `showBoolean` also stopped being the one `show*` that took
      positional handlers. `ApplyButton` is gone — `ConfirmButtons` (✓/✕) is the single confirm
      vocabulary for all twelve tools, per the approved decision.
- [x] **One table-driven Enter router.** `armedConfirm()` replaces the hand-enumerated `if` chain
      that simply omitted boolean/linear/circular/mirror. A table cannot silently omit a tool: a new
      reducer without a row is visible. The frozen contract's `enterSupport` column is green for all
      twelve rows for the first time.
- [x] **No armed model tool renders without a cancel.** The fillet/shell bare-numeric fallback was
      already unreachable (`armShell` and both `showFillet` sites all wire ✓/✕) and is deleted.
- [x] **Reducer event names unified on `confirm`**, with `apply` kept as an accepted alias so a stale
      caller cannot silently become a no-op. Refusal wording unified on "Cannot confirm invalid
      preview: …" (it forked "confirm"/"apply" across six sites).
- [x] **The sketch→Extrude handoff no longer throws the intent away.** Pressing `E` while sketching
      used to finish the sketch and then `resetToSelect("Select one closed sketch region, then choose
      Extrude")` — it armed and immediately reset to Select, which is the "arms then resets" defect
      the spec names. It now keeps the tool and enters the region pick; with a SOLE extrudable region
      `enterRegionPick` arms outright, so `E` in a one-region sketch lands straight on "Drag the arrow
      to set depth". Caught by the browser lane, not by unit tests.

**User-visible changes** (deliberate, recorded per `src/test/contracts/README.md`): the accent `Apply`
button is replaced by ✓ everywhere; `E` from sketch mode arms Extrude directly on a single region.
Two e2e specs encoded the old behaviour and were rewritten (`auto-mode`, `ellipse`) — the rewrite
keeps the region-identity assertion so "armed directly" cannot hide a wrong-region bind. `chip-apply`
is now `chip-confirm` in four specs.

Gates (measured): `bunx tsc --noEmit` clean · `bun run test` **255 files / 4237 tests — 8 failed,
4229 passed** (the 8 are U3's 4 and U7's 4; no pre-existing test regressed) · full Playwright lanes,
retries 0: **chromium 200/200**, **webkit 200/200**. MC-R8's boolean-preview lane passed in this
chromium run; that is one clean sample, NOT a root cause, so MC-R8 stays open.

### U3 — live numeric contract (2026-08-14) — GATE PASSED

Closes D3 (blur-gated values) and D4 (no type-to-enter), the two columns U0 added to the frozen
contract.

- [x] **`DimensionInput` gained `onPreview` + `initialText`.** `onPreview` fires on every PARSEABLE
      change with the document-domain value; partial text emits nothing, stays editable and never
      clamps. `initialText` is the type-to-enter seed — the character typed on the canvas REPLACES
      the formatted value, and it previews too (it is a parseable change like any other).
- [x] **One numeric field for every armed model chip.** `clusterInput` and `numericChip` both route
      through a single `primaryField()`: live `onPreview`, `commitOnBlur={false}` (an armed model
      tool commits on Enter or ✓ only — nothing is lost, the value already went out through the
      preview), and the type-to-enter seed/focus.
- [x] **The type-to-enter router is controller-owned**, not chip-owned — a chip growing its own
      keyboard behaviour is the fragmentation this program removes. A printable `0-9 . -` on the
      canvas calls `toolChipStore.beginPrimaryEntry(char)`, guarded on: nothing editable focused
      (`isEditableTarget` — which is what the command palette and inspector editors are), no
      modifier, and `PRIMARY_VALUE_CHIPS.has(kind)`. That set is keyed on the CHIP, not on the
      reducers: the chip IS the editor, so it cannot drift from what is on screen the way a parallel
      FSM table would, and boolean/mirror/axis-pick/region-select correctly have nothing a digit
      could mean.
- [x] **Two real defects the live path exposed, both fixed:**
      - **the angle parse accepted trailing junk.** `Number.parseFloat("12abc")` is 12, so an angle
        typo silently committed 12 — and under live preview it ALSO rewrote the field mid-keystroke.
        Angles are now as strict as lengths: only a complete number parses.
      - **our own preview echoed back and ate the keystroke.** Live preview means the edit returns
        as a new `value` prop, and re-formatting on that echo rewrote the text under the cursor:
        typing `25.` collapsed to `25` the instant the 25 previewed. The field now ignores the echo
        of the value it last previewed. **The first version of that guard was too broad and the
        browser lane caught it**: it also suppressed the re-label on a UNIT switch, so 50.8 mm no
        longer re-read as 2 in (`e2e/units.spec.ts:155`). A unit change always re-displays; only a
        value echo is suppressed.
- [x] **The probe drives both halves of the real path.** `modelingInteraction.numeric.probe.test.tsx`
      renders the chip AND the controller: the canvas keystroke goes to `window` (where the
      controller listens), everything after it goes to the focused field (where a browser sends it).
      Simulating the second character on `window` would be testing something no browser does. Four
      specs per tool incl. a seeded-value control and a partial-text/no-clamp spec.

**Deliberately NOT migrated:** `HoleChipCluster`'s two raw `<input>`s (hole depth, counterbore /
countersink). They are SECONDARY parameters whose domain includes `null` ("Thru"), which is not a
`DimensionInput` domain, and a second `aria-label="Dimension value"` would make the primary-value
locator every spec uses ambiguous. The existing rationale in that file still holds; the "one numeric
field" rule is about the PRIMARY value, which already routes through `DimensionInput`.

Gates (measured): `bunx tsc --noEmit` clean · `bun run test` **255 files / 4239 tests — 4 failed,
4235 passed**; the 4 are U7's, and **no pre-existing test regressed** · full Playwright lanes,
retries 0: **chromium 200/200**, **webkit 200/200** (re-run clean after the units fix — the first
run was 199/1 and is not claimed).

### U7 — repair and sketch truth (2026-08-14) — GATE PASSED

Taken before U4–U6 because it closed the last four U0 reds and the native toolchain was warm; the
packages are independent, so the order costs nothing.

- [x] **One constraint-status authority.** `InspectorPanel`'s SelectionState branch reads
      `sketchStatusText` instead of hardcoding `Under-constrained · DOF {dof}`. This was the second,
      silent authority D14 left behind — the pure function was fixed, this branch was not, so a
      fully-constrained sketch selected in the TREE still rendered the impossible string.
- [x] **Candidate copy cannot contradict the candidate list.** The count now comes from the same
      `ResolveRefs` response that renders the rows; a collapsed row states the REASON only until
      that response lands. The `needs-repair` event's `candidateCount` is a scoring hint, not a
      promise that any of them is an eligible rebind target (audit finding 06).
- [x] **The sketch origin is a real reference.** It is now a snap tier — ranked WITH quadrant:
      below the three snaps that name a relationship to drawn geometry, above the two derived ones,
      because ranking it top would steal a snap from a nearer endpoint and ranking it bottom would
      lose it to any stray curve passing the origin. `autoConstrain` emits ONE `Fixed` per sketch for
      a point accepted on it, which is what removes the two translation DOF every other dimension
      leaves behind (the audit's 60×40 rectangle reading DOF 2). `Fixed` was already wired end to
      end, so no protocol change. Skipped when the sketch is already anchored — existing geometry on
      the origin, or REFERENCE-LOCKED geometry (a sketch on a model face is positioned by its host
      and its projected boundary carries its own `Fixed` constraints; `sketchOnFace.test.ts` caught
      that case).
- [x] **`OffsetFaceOp` runs the shared publication gate.** It was the one mutating operation that
      never did.

**Honest limit on the OffsetFace change:** its own postconditions (null, `BRepCheck`, exactly one
solid, positive volume above `kMinVolume`, self-interference, semantic delta) are equal or stricter
than every Tier A check except the audit-error path, and no real offset input was found that
produces a non-solid-like result — so **there is no end-to-end negative control, and none is
claimed**. The change buys the structured `PublicationDecision` evidence the P3 contract rows promise
for every other operation, and stops future drift. `ImportOp` remains uncovered by design (its
advisory/healing policy is versioned separately).

Gates: `bunx tsc --noEmit` clean · `bun run test` **255 files / 4240 tests, ALL GREEN — every U0 red
is now closed** · worker Release build + `ctest --test-dir worker/build` **119/119** ·
`cargo fmt --all --check` clean.

### U6 — tool entry, roles, and result truth (2026-08-14) — GATE PASSED

- [x] **Unique body display labels.** `default_name` scans for the lowest FREE `Body N` instead of
      `bodies.len() + 1`, which was a position, not an allocation: deleting `Body 1` of three made the
      next fresh body `Body 3` as well. Measured before the fix: `["Body 2", "Body 3", "Body 3"]`.
      Deterministic, stable across save/reopen, and needs no counter to serialize.
- [x] **One pattern-count range across TS/Rust/worker.** `PATTERN_COUNT_MIN/STEPPER_MAX/COUNT_MAX`
      (2 / 12 / 128) replace the silent 2–12 clamp. The +/− buttons still step 2–12 — one click per
      instance for the common case — while TYPING reaches the worker's `kMaxPatternCount`. Out of
      range is REFUSED and marked, never clamped: a clamp would commit a count the user never saw
      previewed.
- [x] **`Total`, not "count".** The label states that the number includes the source — the two
      readings differ by exactly the body the user is looking at.
- [x] **Boolean states its roles.** Two preselected bodies now ARM outright with roles assigned in
      selection order, instead of discarding half the selection and asking for a pick already made;
      one body still enters `AwaitingSelection: Pick the tool body`. The chip names both operands and
      offers **Swap** — a Cut is not symmetric, and before this "did I pick the right target?" could
      only be answered by cancelling. Swap RE-OPENS the preview lane rather than re-sending params:
      a session's operand refs are fixed at `beginPreview`.
- [x] **Ghost fidelity is disclosed.** Pattern and Mirror arm hints read "… · placement preview,
      validated on Apply · Enter or ✓ to confirm". They have no kernel candidate — the viewport shows
      the source mesh transformed locally, which proves placement and nothing else — and a translucent
      shape looks identical to an exact candidate.
- [x] **Mirror's fuse default matched to the record.** Re-edit fell back to `true` where
      `MirrorBodyParams.fuse_with_original` is `#[serde(default)] bool` — i.e. `false`. A legacy record
      with no `fuseWithOriginal` loaded non-fused in Rust but would have re-edited as fused, and
      committing that silently flips the result mode.

**Dropped after checking the source, not implemented blind:** the plan listed "mirror gains the
`resultPolicyVersion` symmetry the patterns have". `MirrorBodyParams` has no such field, so there is
nothing to preserve — the asymmetry is correct, and adding one would be a record/wire change with its
own gate. The pattern `fuseResult ?? true` fallback is likewise left alone: absent means V1 legacy
aggregate semantics, which `TODO.md`'s Pattern compatibility baseline pins.

Gates (measured): `bunx tsc --noEmit` clean · `bun run test` **255 files / 4244 tests, all green** ·
`cargo clippy --workspace --all-targets -D warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test
--workspace --no-fail-fast` **810 passed / 0 failed across 69 targets** · worker `ctest` 119/119 ·
full Playwright lanes, retries 0: **chromium 200/200**, **webkit 200/200**.

**The origin anchor is a user-visible change and the browser lane priced it.** Geometry whose point
lands on the origin now loses two translation DOF, so five specs that draw from screen centre (which
IS the origin) reported new numbers: circle 3→1, arc 5→3, ellipse 5→3, and a solitary circle/ellipse
is no longer "No constraints yet". All five were updated with the reason, not the number alone.
Tightened while doing so: the anchor is gated on `InferOptions.originAccepted`, which the controller
sets from the same point-snap preference that puts the origin in the snap ladder — so with point
snapping OFF a coordinate that happens to be (0,0) is a coincidence, not an accepted relation, and
"do not auto-fix geometry merely because it was drawn near the origin" holds.

**New residual — MC-R9 (full-suite-only, observed once).**
`e2e/revolve-commit.spec.ts:111` ("Revolve guidance …") failed in ONE full chromium run that took
19 min instead of the usual 14 (the box was loaded), and passes in isolation and in the clean
re-run. Superficially the same signature class as MC-R8, but **not the same root cause**: MC-R8 was
the debounced auto-fit moving the camera under an unsettled probe (`19088d0`), and this spec already
calls `waitForCameraSettled` at both of its probe sites (`:124`, `:130`). Recorded, not retried
away: two clean full lanes are the claim, one loaded run is the caveat.

### U4 — OperationHUD + result summaries (2026-08-14) — GATE PASSED

- [x] **One HUD frame for every armed model tool.** `panel()` is now the shared OperationHUD: common
      tone (the WARN border on `valueError`, previously hand-rolled in the offsetFace branch and
      absent everywhere else), a common result-summary slot above the controls, and a common
      `role="status" aria-live="polite"` region. The offsetFace branch's private copy of the frame is
      deleted. A tool can no longer quietly acquire its own tone, its own validity treatment, or no
      accessible status at all.
- [x] **D18 — every body-lifecycle operation states its result BEFORE Apply.** The audit's complaint
      was that the user cannot tell whether instances are linked, merged, copied or editable later
      until after committing, and a count alone cannot say it: `3` is the same number whether the
      source survives or is folded away. So the summary names the LIFECYCLE:
      - `Linear pattern · 3 total · 2 new bodies · source retained` (V2 keeps the source as instance
        zero, hence N−1 children), or `· fused into the source`;
      - `Circular pattern · …` the same way;
      - `Mirror · 1 new body · source retained`;
      - `Cut · Body 1 survives · Body 2 is consumed` — a Boolean CONSUMES its tool body, the single
        most surprising thing about the operation and the one the audit found unstated;
      - `Move · 2 bodies in place` / `Copy · 2 bodies · sources retained`.
      It is published from the same place that rebuilds each ghost, so it cannot drift from what the
      viewport is showing.

**`ToolChipState` remains a flat object — deliberately, and this is the second time it has been
deferred.** The spec's D6 asks for a discriminated `ToolEditorDescriptor` union. No red test forces
it; it is pure internal type-safety; and the store gained four fields in this program (`primaryEntry`,
`onSwap`/`targetName`/`toolName`, `resultSummary`), so a union rewrite now would land on a moving
target. The behavioural half of D6 — one confirm protocol, one numeric field, one HUD frame — is
what U2/U3/U4 actually delivered, and that is what a user or a future tool can observe. The type
refactor stays a separate pass with its own gate.

Gates (measured): `bunx tsc --noEmit` clean · `bun run build` clean · `bun run test` **255 files /
4247 tests, all green** · coverage + contract verifiers pass · hex gate empty · full Playwright
lanes, retries 0: **chromium 200/200**, **webkit 200/200**.

### U5 — gizmo overlay + collision-safe HUD (2026-08-14) — GATE PASSED

Closes D5, the audit's P0 #2. **No transform semantics changed** — world-axis-only, frozen pivot,
fold, copy and align are untouched. This is grab geometry and placement.

- [x] **Three full 82px tori → three compact double-headed ARCS.** `TubeGeometry` over a ±26° sweep
      at r=70, centred on each plane's 45° bisector — where no arrow lives (they are on the axes) and
      no quad reaches (its far corner is r≈44) — with a cone at each end so the handle reads as
      grabbable rather than decorative. Classification stays `{kind:"ring", axis}`, so the gesture and
      `e2e/modelToolHelpers.findGizmoHandle` are unchanged.
- [x] **The pick corridor is now 6px (12px across) against a 2.2px stroke** — a trackpad user no
      longer has to land on a hairline.
- [x] **The rotation handles no longer overlap EACH OTHER.** Three full circles of one radius meet at
      six points, two rings deep, exactly where the translation arrows live; which ring you got was a
      coin flip. The arcs share no point and none sits on an axis, so the arrow underneath wins
      cleanly — pinned by a new spec.
- [x] **`ViewportEngine.getInteractionOverlayBounds("transformGizmo")`** — the projected screen box.
      No such API existed (`projectPoint` returns one point; the bounds helpers are world-space
      `Box3`s), which is why nothing could ask where the gizmo was on screen.
- [x] **The chip sits clear of the widget.** `TransformChipOpts` was the one `*ChipOpts` that did not
      extend `ChipAnchorOpts`, so `resolveChipAnchor` never ran for it and the chip anchored dead on
      the pivot — across the stems, the handle intersections and the pivot itself. It now offsets
      along the active axis by the gizmo's own projected reach plus the shared gap, so the two cannot
      drift apart when the geometry changes.
- [x] **One screen-scale implementation.** Both overlays were fed `planePixelWorld()`, which measures
      at the ORBIT TARGET and ignores orthographic zoom — so a handle away from the pivot was sized
      for the wrong depth and every ortho zoom level scaled it identically. They now use
      `screenScale.worldPerPixel` at each overlay's OWN anchor, which is what every other
      constant-size layer (OriginTriad, PlanePicker, contributions) already used.

**The edge-on case is characterised, not claimed fixed.** Any overlay handle coplanar with the view
direction can be crossed; that is inherent to an unprojected, non-depth-tested widget, and
nearest-hit remains the honest rule. What changed is the SIZE of the region — a 52° arc in one
quadrant instead of a full circle. The old characterisation spec is kept, with its reasoning updated.

**Shell's thickness handle is NOT implemented, deliberately.** A drag handle needs a direction, and
the shell arm has none: `EntityRef.anchor` carries `worldPoint`/`surfaceUv` and no normal, and
nothing in the shell path probes one (fillet and offsetFace get theirs from their own prepare
responses). The spec's own rule — "the handle must not imply a valid direction where the operation
cannot define one" — makes guessing worse than omitting. It needs a face normal on the prepare
response, which is a wire change with its own gate.

Gates (measured): `bunx tsc --noEmit` clean · `bun run build` clean · `bun run test` **255 files /
4248 tests, all green** · full Playwright lanes at the gate, retries 0.

Unresolved questions:
1. ~~Wire-level record correlation for `updateOperationParams`~~ — answered in U1: not needed.
2. Is U8 (typed face/datum/axis references) queued straight after U7, or behind other roadmap tracks?
3. MC-R8 is still un-root-caused. U2 and U6 touch that lane; the browser gate is not claimable for
   those two packages until it is.

## NOW — modeling-correctness hardening completion (2026-08-13)

Source: user-supplied completion plan. Baseline `9933689`; clean `master`, one commit ahead of
`origin/master`. No commit/push/pull authorized.

- [x] P0 strict plan-stream state machine; malformed fixtures refuse before `AcceptPrepared`.
      Wire 34/34, executor 18/18, onecad-core full clean; malformed order/dup/range/enums/terminal
      controls assert zero accepts + discard.
- [x] P0 release worker lockstep manifest/hash/fingerprint; prepared-mesh transport limits.
      Real worker manifest hash/hello matched; manager 20/20, release check + Clippy clean. Inline
      meshes share hello constants; oversized bodies use chunked Tessellate.
- [x] P1 ResolveRefs provenance/history/import-order hardening. Wrong/missing body/revision evidence
      refuses; Modified+Generated focused 2/2; exact STEP order ties refuse.
- [ ] P2 V3 core/new authoring done: `cell-v3`, seam-safe physical distance, frozen V1/V2, exact
      analytic fail-closed. Worker full 119/119 + tsc. Still open: separated public tolerance knobs,
      production cancellation breadth, source/pair/fragment ceiling matrix, measured perf targets.
- [ ] P3 structured `PublicationDecision` evidence/timings + Tier-A modeling-input preflight done.
      Still open: all semantic evidence routing, measured Tier A/B overhead, Pattern fuse/topology budget.
- [ ] P4 mode-aware coverage + corpus **9/9** done. Real-worker corpus: all frozen cases,
      typed assertions, zero skips; verifier controls 15/15. Still open: full zero-retry browsers,
      complete C4 mode matrix, Intersect promotion, real diagnostic browser vertical.
- [ ] P5 real-Tauri lane implemented: feature-gated official WDIO plugins, relocated bundle,
      lockstep worker hash/fingerprint, Extrude → Fillet → Undo → Save/reopen, retained logs and
      cleanup. `cargo check --features tauri-e2e` now compiles (it never had: the first compile of
      `tauri_e2e.rs` exposed an E0597 borrow error in `composition_status`, fixed). The lane itself
      is still UNRUN — no relocated-bundle WDIO execution has happened on any machine.
- [ ] P6 kernelbench Boolean foundation done additively: strict case-v2 two-body recipe/roles,
      Fuse/Cut/Common, raw OCCT + OneCAD publication paths, stable replay/differential evidence.
      Focused Rust 62+5 and kernelbench CTest 5/5 pass. Chamfer/campaign breadth, T0/m1 campaigns,
      and cross-platform release enforcement remain open.
- [x] Reconciled current state, live delta, risk register, packaging, protocol, contracts, coverage;
      added versioned residual register. Manual triage remains historical evidence, not a completion claim.
- [x] **Final gate ladder RUN end to end (2026-08-13, unsandboxed local mac)** — worker → Rust →
      frontend → verifiers → benchmarks → browsers. Measured, not claimed:
      - worker Release build + stage (sidecar **and** `onecad-worker-manifest.json` regenerated) ·
        `ctest --test-dir worker/build` → **119/119**
      - `cargo fmt --all --check` clean · `clippy --workspace --all-targets -D warnings` clean ·
        `ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` → **767/767 across 60
        targets, 0 failed, 0 ignored** · corpus **9 executed / 9, zero skips** ·
        `cargo check --features tauri-e2e` clean
      - `npx tsc --noEmit` 0 · `bun run build` clean · `bun run test` → **250 files / 4182 tests** ·
        coverage + contract verifiers pass · verifier negative controls **15/15** · hex gate empty
      - kernelbench `fillet/foundation:t0` both backends → **136 records, 136 pass, 0 fail,
        rescued=0 regressions=0 replay-unstable=0**; `semantic-compare` vs
        `bench/robustness/baselines/semantics.json` → OK on darwin-arm64 (frozen T0 unmoved)
      - Playwright retries 0: **chromium 199 passed / 1 failed**, **webkit 199 passed / 1 failed**
      - Three real defects the ladder caught and this session fixed: the worker STUB emitted an
        `autoBind` ResolveRefs resolution with no `bodyId` (SCHEMA §7.5 allows omission only on a
        non-promotable missing-body `needsRepair`) — stub now mirrors the real worker's
        missing-body branch and `solver_stub` pins both branches; `topology_rebind` extrudes fed a
        real V3 region id into a version-less profile, which correctly refused
        (`regionId … matched no selectable region`) — fixture now carries version 3, matching the
        `revolve_ops`/`m2_gate`/`wire_contract`/`step_import_gate` pattern; and `tauri_e2e.rs`
        did not compile.
- [x] **MC-R7 correction — DONE, both lanes measured.** Root cause is
      NOT a product bug: commit `c7df7c8` removed the click-away commit deliberately
      ("D2: click-away commit removed entirely (spec choice)"), the frozen contract
      `src/test/contracts/modelingInteractionContract.ts` pins
      `clickAwayPolicy: "cancel"`, and `ModelToolController.commit.test.ts` already
      asserts it must not commit. `c7df7c8` shipped without the e2e lane, so the spec
      and the arm hint text kept promising the removed gesture. Both are now fixed in
      the working tree (spec asserts no-commit + still-armed; hint reads
      "Enter or ✓ to confirm"). Verified: that spec chromium 5/5, tsc 0,
      Vitest 4182/4182, then BOTH full lanes with `E2E_PORT` and retries 0 —
      **webkit 200/200**, **chromium 200/200** on the rerun. MC-R7 is closed in
      `docs/qa/modeling-residuals-v1.json` as stale evidence rather than a product
      defect.
- [x] **MC-R8 — CLOSED with a root cause and a fix, commit `19088d0`.** The symptom was
      `e2e/boolean-preview.spec.ts:356` (Intersect chip) timing out at `previewOwner === null`
      in a FULL chromium run only, 9/9 green in isolation. The race is NOT the projection push
      first guessed at: an instrumented full lane measured `autoFitPending: true` at probe entry
      in all three tests, every run. Each extrude in `twoBodies` commits a body, a new body
      schedules the DEBOUNCED auto-fit, and whether that 250 ms timer fires before or after the
      probe's click is pure timing — when it fires first the tween moves the camera and the ray
      misses the body, so `runPick` returns no ref (a genuine miss, not the deferred
      `"unsettled"` case) and the selection clears. Fix is one `waitForCameraSettled` at the top
      of the local `findBodyScreenPoint`, matching `helpers.ts findFaceOnBody`. No retry added.
      Two latent same-pattern scanners were audited and deliberately left alone
      (`modelToolHelpers.ts findFacePoint`, `hole.spec.ts farthestPixelOnFace`) — a timing edit
      to a shared helper needs its own two-lane evidence.
- [x] ~~**Pre-existing browser defect**~~ — superseded by the MC-R7 entry above; kept for the
      measurement that found it: `e2e/extrude-commit-gesture.spec.ts:135`
      "clicking empty canvas away from the handle commits (click-away)" fails deterministically
      (3/3 with `--repeat-each=3`) on BOTH chromium and webkit — and fails identically on a clean
      worktree at baseline `9933689`, so it predates the modeling-correctness work. The click leaves
      the tool `armed` and mints no body. Tracked as MC-R7, now closed: the spec was wrong, not the
      product. Both lanes re-measured at 200/200 — but see MC-R8 above before calling the browser
      gate reproducibly green.
- [ ] Still unrun on any machine: real-Tauri WDIO composition, kernelbench m1 campaign,
      Linux/Windows release matrix, 20-run stability sample.

Unresolved questions: none.

## CL-TRIM (2026-08-13) — GATE PASSED · **seed catalog cut to one family**

Scope reduction, requested: the shipped component catalog is now the ISO 4762
socket head cap screw and nothing else. The nine other families — ISO 7380,
4014, 4017, 4032, 7089, 7093 and the three machine elements (ISO 15 bearing,
NEMA 17/23 steppers) — are gone from the seed catalog **and** from the code
that generated them. This is a delete, not a feature flag: re-adding a family
means a manifest, a worker generator and a dimension table, not a revert.

Why the screw: it is spec §12's flagship flow ("search M8 socket head, drag it
onto a hole") and the only family carrying pinned exact-volume ctests. The rest
was breadth with no consumer.

- [x] **Seed catalog** — nine `component.toml` directories deleted;
  `SEED_PACKAGES` is one entry; `SEED_VERSION` 3 → 4 → **5** (the ledger, below).
- [x] **Worker** — `ComponentGenerators.cpp` keeps `build_socket_cap` and the
  three thread-detail cutters; `build_button_head` / `hex_screw` / `hex_nut` /
  `washer` / `bearing` / `stepper_motor` and their helpers (`build_hex_prism`,
  `build_dome_head`, `lookup_key`, `text_param`) are gone. `known_generator_ids`
  is `"iso4762"`. `MachineElementTables.{h,cpp}` deleted; `FastenerTables` is
  one struct and one table. **The DISPATCH stays** — an unknown id still fails
  loudly naming what exists, which is the whole point of WP-A1 and is not the
  same thing as having only one family.
- [x] **Rust mirror tables** — `onecad-library/src/tables/fasteners.rs` deleted
  (it had no consumer outside its own tests once the generators went);
  `iso4762.rs` and its worker cross-pinning stay.
- [x] **Template** — `onecad.std.template.nema17-mount` removed with its NEMA 17
  package. A starter that places a component the library does not ship opens
  straight into `NeedsRepair`. Two honest starters (`blank`, `printed-part`)
  beat three with a broken reference; spec §8's "3–5 starters" is now under-met
  and recorded as such rather than faked.
- [x] **Mock lane** — `MOCK_SEED_FIXTURES` gone, catalog is the one SHCS
  fixture, template mirror is two.
- [x] **Provenance** — `THIRD_PARTY_NOTICES` drops the four BOLTS classes whose
  data no longer ships (hex.blt ×2, nut.blt, washer.blt) and the two
  hand-transcribed non-BOLTS standards; the ISO 4762 / hex_socket.blt entry and
  the ISO 261 pitch note stay.

**Gate:** vitest 4347/4347 · ctest 124/124 · `cargo test --workspace` 80 targets,
0 failures (worker-backed, `ONECAD_REQUIRE_WORKER=1`) · clippy `-D warnings`
clean · `cargo fmt --check` clean · Playwright **422/426**, every library spec
green (`library-browse-place-snap`, `library-preview`, `library-place-freespace`,
`library-author-component`, `library-template`).

The four e2e failures are NOT this work, and that was checked rather than
assumed: `filletChamfer:181` and `sketch-multi-object:44` pass in isolation
(the documented full-suite load-flake class); `extrude-commit-gesture:135`
fails DETERMINISTICALLY on both browsers in a clean worktree at HEAD `2b9b04b`,
i.e. before any of this. It expects 2 body options and gets 1 — pre-existing,
unrelated to the catalog, and left for whoever owns that lane.

### CL-TRIM.2 — the install ledger + safe prune (`SEED_VERSION` 5)

Found by using it: after the cut the panel still listed nine dead cards on an
already-seeded root, because seeding never removed anything. They can never
render — the worker has no generator for those ids — so leaving them was worse
than the risk the no-delete rule was protecting against.

Seeding now keeps a **`.seed-ledger.json`** mapping each installed package id to
the SHA-256 of the manifest it wrote. A package is pruned only when all four
hold: the id is ledgered, the id is no longer in `SEED_PACKAGES`, the directory
holds nothing but `component.toml`, and that file still hashes to the ledgered
value. Fail any check and the package is **adopted** — left on disk, dropped
from the ledger, never reconsidered, because at that point it is the user's.
Both outcomes are reported in `SeedOutcome` and logged by name: a pass that
deletes from the user's library says what it took.

A package that is already present and byte-identical to what this build ships
is CLAIMED into the ledger on the `kept` path. Without that, a root seeded
before the ledger existed would never ledger anything it already had, and the
gap would persist for the whole life of the install.

- The rule is intact: seeding still never deletes anything a user wrote. It
  takes back only what it can prove it wrote and no longer ships.
- **Not covered:** a root at marker ≤ 4 has no ledger, so its dead directories
  are unprovable and stay. One manual `rm` clears them; the dev machine's root
  was cleared this session.
- **Templates are not pruned**, and that is stated rather than skipped: their
  bytes are generated per install (a fresh `DocumentId` each time), so there is
  no "still unmodified" proof to check, and a prune without one is the guess
  this whole design exists to avoid. The dead `nema17-mount` starter is removed
  by hand, once.
- Six prune tests, one per safety check (pruned · edited⇒adopted ·
  added-file⇒adopted · unledgered⇒untouched · already-gone · still-shipped),
  plus two claim tests (identical⇒claimed · user-authored⇒never claimed).

**Seams flagged:**
- `GeneratorRequest::text_params` is populated but no generator reads anything
  but `thread` out of it now. Kept on purpose — it is the seam a non-thread-keyed
  family needs, and dropping it would have to be undone to add one back.
- The thread cutters keep `thread_length_mm` for the same reason, though the
  sole caller passes the full shank length.
- No seeded starter carries a `PlaceComponent` record any more, so nothing
  proves an authored record meets the generator it names at open time. Noted in
  `component_ops.rs` where that test used to live.

## WP-F1.3 (2026-08-13) — GATE PASSED · **Component Library program COMPLETE**

**A component authored from a document is PARAMETRIC.** Its free parameters are
the source document's VARIABLES, and `setComponentParams` on a placed instance
replays the frozen source with the new values, re-bakes, and swaps the
instance's solid. This was the last hole in spec §12's definition of done and
the last ratified deviation (§13.3 #3, now amended in the spec): before this a
`document`-source placement carried baked geometry with nothing editable.

The two blockers are gone — WP-VE.1 made `Scalar.expr` drive regen, WP-VE.2
exposed the variable table — so this WP is the join.

- [x] **Authoring declares free params** (`SaveAsComponentDialog` "Parameters"
  section + `NewComponentSpec.parameters`). `listVariables()` rows with
  checkboxes; a checked variable becomes `[parameters].<name> = { role = "free",
  key = "<variable>", value = <current> }`. Only VARIABLES are offered, because a
  re-bake sets variables — anything else would be an edit that cannot be
  honoured. `save_as_component_at` REFUSES a parameter naming a variable the
  document does not declare, at authoring time where the author can fix it (the
  same discipline as the single-solid rule). No variables / nothing checked ⇒ an
  empty table and byte-identical behaviour to every pre-WP-F1.3 package.
- [x] **`ParameterSpec::free_variable` lives in `onecad-library`**, not the app
  crate, so `toml::Value` stays an implementation detail of the package format
  (the app crate has no `toml` dependency and does not gain one).
- [x] **The re-bake lane** (`library.rs`, replacing the Document refusal arm).
  In order: `check_free_params` (role=free, existing) → `variable_overrides`
  (each param's `key` IS the variable name; a free param with NO `key` is refused
  BY NAME — guessing "the same-named variable" is the mis-bind spec §0 invariant
  4 forbids) → `frozen_source_document` (revision-verified through
  `resolve_source`, PLUS a direct check that the package's `source.onecad` still
  hashes to what the instance recorded) → replay → export → stage.
- [x] **The ephemeral worker is the design.** The worker is one session per
  process, so replaying a second document needs a second process; running it on
  the open document's worker would trample the session the user is editing. So:
  `WorkerManager::spawn(SupervisorConfig::production(resolve_worker_path()))`,
  wrapped in an `EphemeralWorker(WorkerManager)` guard whose `Drop` calls
  `retire()` — the reason it is a guard and not a call at the end of the happy
  path is that EVERY `?` in between must also tear it down. It is never installed
  in `AppState`'s `live`/`warm` slots, gets no restart hook and no status
  forwarder, and never bumps the open document's epoch: the open session, its
  fencing tokens and its epoch never see it. The scratch `DocumentRuntime` opens
  the frozen bytes from a per-call temp dir (swept on every path, error included),
  applies one `EditCommand::SetVariable` per override, regens `ToEnd { from: 0 }`,
  and is dropped with the worker. **No lock is held across any of it** — the
  runtime lock is taken only to read the record before, and to stage+apply after.
- [x] **Exactly one solid, never a silent fuse.** The replay must yield exactly
  one body AND `ExportGeometry` exactly one solid; more is a loud refusal naming
  the count. The author already answered the multi-solid question at save time
  (`unionSolids`); re-deciding it here would change what the component IS behind
  their back.
- [x] **One undoable edit at the SAME `RecordId`.** `stage_blob_and_apply` puts
  the re-baked blob in the carrier and applies `UpdateOperationParams` under ONE
  runtime lock (the `add_import_record` discipline: no regen may see a record
  naming a blob that is not yet staged). The instance keeps its recorded
  component revision — the package did not change. `source.sha256` moves, which
  is what makes the step dirty and the geometry actually rebuild.
- [x] **A missing/changed package refuses the EDIT, never the document.** The
  typed `LibraryError` (`NotFound` / `RevisionMismatch`) travels out unchanged;
  the instance's cached blob is authoritative, so it keeps rendering exactly what
  it had. Proven by test, including the volume after the refusal.
- [x] **Configurator needed no source-kind branch** — it already keys on
  `role === "free"` (WP-B4). One real bug fixed: `currentValue` read `spec.key`
  before `spec.value`, which is right for a generator (`thread = { key = "M6" }`)
  and WRONG for a document param, whose `key` is the variable NAME — the field
  would have shown the literal word `depth`. `value` now wins; no package
  declares both.
- [x] **Catalog preview stopped refusing.** `component_preview_mesh_at` dropped
  free params for a blob-backed source into `source_with_free_params`, which
  refuses them — fine while a `document` package declared none, an error on a
  thumbnail now that it can. Params are applied only for a `generator` (the only
  kind whose geometry they select); a blob-backed component previews the solid
  its package carries.
- [x] **`place_component_at` / `set_component_params_at` / `reindex_library_at` /
  `list_library_components_at` are now `pub`** — the `save_as_component_at`
  precedent, so the worker-backed test drives the REAL command cores through
  `tauri::test::mock_app` instead of re-deriving them.
- [x] **SCHEMA: no wire change, verified.** `ExportGeometry` (§7.8),
  `PlaceComponent.source` (§7.3) and the blob-staging path all pre-exist;
  `SetVariable` is an `EditCommand`, not a wire op. `source.params` on a
  `document` kind was already an optional field the worker ignores — it is now
  non-empty. §7.3 gains one CLARIFYING sentence saying so (documentation of
  existing behaviour, no shape change ⇒ no fixture bump, no §14 entry).
  `record.rs`'s "empty in this build" doc comments were stale and are corrected.

**Gate:** ctest 124/124 green · `cargo fmt --check` clean · `cargo clippy
--workspace --all-targets -D warnings` clean · `cargo test --workspace
--no-fail-fast` 81 targets green (`ONECAD_REQUIRE_WORKER=1`), incl. the new
`tests/component_rebake.rs` 3/3 · `tsc --noEmit` clean · vitest 264 files / 4348
tests green (+6) · hex gate empty.

**New worker-backed proof** (`src-tauri/tests/component_rebake.rs`): author
`depth = 10` driving a 20×20 extrude (4000 mm³) → save as a component with
`depth` free → place → `setComponentParams depth = 20` → the PLACED body is
8000 mm³ by `QueryMassProperties` → save, **delete the library**, reopen on a
FRESH worker → still 8000. Plus: a deleted package refuses the edit and leaves
4000 mm³ standing; a two-body source document refuses the re-bake.

### The Component Library program (spec §12) is COMPLETE

Everything in the definition of done ships: browse/search, drag-snap onto a hole
with auto-size, one undoable node, in-place parameter edit (now for authored
components too), save/close/reopen with the library deleted and geometry intact,
mate re-seating, authoring one's own snapping component, and project templates.

**Known deferrals — none of them block the definition of done:**
- **A re-bakeable source document must build exactly ONE body**, and a package
  baked with `unionSolids` cannot be re-baked at all (nothing records that
  choice). Both are loud refusals at the first edit, never a wrong solid. Fixing
  either means recording the baked body / the fuse choice in `component.toml`,
  i.e. a package-format change.
- **WP-VE.2b** — bindings through the model TOOLS (Hole first): its re-edit
  rebuilds every `HoleParams` scalar from the FSM, so binding one field today
  would silently discard a binding on another.
- **WP-VE.3** — sketch dimensions cannot be variable-bound: they are solver
  values, not `Scalar`s, and need a document-variable → constraint-value lane.
- **P4 registry** — the shared/remote component registry (spec §11) is not built;
  the library is a local folder.
- A `document` component's CATALOG preview shows the package as authored, not a
  placed instance's re-baked geometry (a preview re-bake would pay a worker per
  thumbnail). The viewport shows the instance, which is where it matters.

## WP-VE.2 (2026-08-13) — GATE PASSED

**Users can author variables and bind op params to them.** WP-VE.1 made
`Scalar.expr` drive regen but nothing exposed it: no command, no UI. Now there
is a `Variables` inspector section and `=name` binding on a history row's value.
No worker/C++ change, no SCHEMA change (verified below).

- [x] **Three thin Tauri commands** (`api/mod.rs`) — `list_variables`,
  `upsert_variable(name, value)`, `remove_variable(name)`, all delegating through
  `rt.apply(EditCommand::{Add,Set,Remove}Variable)` so a variable edit is as
  undoable as any other edit and schedules the same regen (ARCHITECTURE §9: no
  second write path). Keyed by NAME, not id — a name is what an `expr` binds to,
  so making the frontend carry an id would only give it a second way to be stale.
  Name validation reuses `regen::variables::is_bare_name` (newly `pub`) rather
  than a second copy of the grammar: the app must not be able to mint a variable
  no binding could ever name. Existing name ⇒ `SetVariable` (id + declaration
  position preserved); new ⇒ `AddVariable`; case-SENSITIVE, matching
  `VariableTable::get`; unknown remove REFUSED, never a silent no-op. The
  read+decide+apply happens under ONE runtime guard.
- [x] **`primaryExpr` on the projection** (`dto.rs` `FeatureValue`/`FeatureDto` →
  `documentStore.FeatureMeta`). Minted in the SAME `feature_value` match arm the
  number is (`.bound(&p.distance)` etc., 7 arms), extending that function's
  existing "one match decides both" rule to a third field. **This is the honesty
  spine**: it is the only thing the row renders `=name` from, so the UI can never
  display a binding the document does not hold.
- [x] **Binding lane assessment → SHIPPED.** The inspector's op-param edit already
  sends the SCHEMA §7.3 object form: `featureValueEdit.commitFeatureValue` builds
  `{[field]: {value}}` and `updateScalarParamsCommand` shallow-merges it over the
  stored params. Upgrading it to `{value, expr}` was a one-site change per layer
  (`WireScalar` gains `expr?`; `commitFeatureValue` gains an `expr` param;
  `sections.tsx makeValueEdit` passes `onCommitExpr`). Omitting `expr` CLEARS a
  binding — the backend replaces the whole op — which is exactly what "typed a
  plain number over a bound field" should mean, and what makes the clear path
  free.
- [x] **`DimensionInput` binding is OPT-IN** (`onCommitExpr` + `expr` props).
  Most consumers of that chip are sketch constraint badges, whose values are
  solver dimensions with no `Scalar` and nowhere to record an `expr`; accepting
  `=name` there would promise something the backend cannot keep. Without the
  prop, `=name` stays the unparseable text it has always been. A MALFORMED
  binding (`=`, `=2w`, `=w * 2`) flashes the error rather than falling through to
  the numeric parse and quietly re-committing the old number.
- [x] **Hole is deliberately NOT bindable** (`canBindFeatureValue`). Its re-edit
  does not use the merge-patch lane: `ModelToolController.commitHole` rebuilds
  every `HoleParams` scalar from its FSM and wholesale-replaces the op, so
  editing a hole's DEPTH would silently discard a binding on its DIAMETER — a
  field the user never touched. An affordance an ordinary follow-up gesture
  throws away is worse than none. **Follow-up: WP-VE.2b** — thread `expr` through
  the Hole tool (and the other `ModelToolController` re-edit sites, which today
  clear the binding on the one field they patch; that one is correct-by-intent,
  Hole's blast radius is not).
- [x] **Variables inspector section** — modeling module contribution, priority
  500 (last, after Dependencies). Rows (name · numeric value · delete), a draft
  row committing on Enter, inline validation, empty-state one-liner. The row's
  number field is KEYED ON ITS VALUE: it is uncontrolled, so a new `defaultValue`
  alone never reaches the DOM and the row would keep showing a stale number after
  a re-value or an undo (caught by the e2e). An `expr`-driven variable renders
  read-only — V1 refuses to resolve a chained expression, so a number field there
  would invite an edit that does nothing. Re-lists on `document-changed`, which
  is what makes it follow an UNDO rather than its own last write.
- [x] **FROZEN CONTRACT AMENDED — explicit user-visible change**
  (`src/test/contracts/inspectorContract.ts`, per `contracts/README.md`).
  "Variables" is appended to every MODEL-mode state (`empty`, `body`, `face`,
  `edge`, `sketch`, `sketchRegion`); `sketchMode` is unchanged. It is the first
  DOCUMENT-LEVEL section, so it renders regardless of selection.
  `InspectorPanel`'s EMPTY state now hosts sections too — it hosted none because
  every section then was about the selection, and gating a document's own
  parameters on picking some unrelated body would hide them exactly where a user
  looks for them. The golden PROBE also changed (it now flushes the async section
  before reading, and zeroes the mock lane's simulated latency): it was passing
  only because `Variables` had not resolved yet, i.e. asserting against a panel
  the user never sees. The contract values changed on purpose; the probe changed
  because the mechanism gained an async section.
- [x] **SCHEMA: no change, verified.** `Scalar` already serializes as
  `{value, expr?}` and §7.3 (amended 2026-07-16) already requires both readers to
  accept the object form — the only new wire content is an `expr` the core
  already round-trips and the worker already ignores (`read_scalar`). No fixture
  bump, no §14 entry. `primaryExpr` is a DTO/tauri-IPC field, not OCW1.
- [x] **Mock lane parity** — a real in-memory `VariableTable` (ordered, so a
  re-value keeps its position) with the same validation and the same
  `document-changed` emit, plus `primaryExpr` mirrored in
  `featureValueForParams` (whose `dimensioned` now takes the source scalar FIRST,
  so a row value cannot be minted without naming the scalar its binding comes
  from). Without that mirror the e2e binding spec would be vacuous.

**Gate:** `cargo fmt --check` clean · `cargo clippy --workspace --all-targets -D
warnings` clean · `cargo test --workspace` 79 targets green (`ONECAD_REQUIRE_WORKER=1`)
· `tsc --noEmit` clean · vitest 264 files / 4342 tests green (+40) · playwright
`e2e/variables.spec.ts` 4/4 (chromium + webkit) and the three neighbouring
history specs re-run green · hex gate empty.

**Seams flagged:**
- **WP-VE.2b** — bindings through the model TOOLS, Hole first (above).
- Sketch dimensions still cannot be bound: they are solver values, not `Scalar`s.
  A binding there needs a document-variable → constraint-value lane that does not
  exist yet.
- The V1 grammar (`[A-Za-z_][A-Za-z0-9_]*`, no arithmetic) is now duplicated as
  `VARIABLE_NAME_RE` in `src/ipc/types.ts` for the pre-flight check. It is pinned
  on both sides by tests; a real expression engine must retire both together.
- `HistoryList.test.tsx > clicking the value opens an editor and commits the
  typed number` flaked ONCE under full-suite load (a 220ms `VALUE_EDIT_OPEN_MS`
  timer) and passed on every re-run, including the full suite. PRE-EXISTING, not
  introduced here.

## WP-VE.1 (2026-08-13) — GATE PASSED

**Variables actually drive geometry** (core lane). A `Scalar`'s `expr` was
stored and never read anywhere: `regen/**` had zero references to the variable
table, the worker's `read_scalar` takes `value` only, and a probe (extrude with
an expr-bound distance, `SetVariable 10 → 20`, regen `Published`) produced
bit-identical volumes. That is fixed. No worker/C++ change, no frontend change
(VE.2), no SCHEMA change.

- [x] **The substitution pass** — `onecad-core/src/regen/variables.rs` (new):
  `substitute_variables` (in place over a caller-owned record slice),
  `substituted_timeline` (the `Timeline`-level wrapper), `resolve_expr`,
  `write_back_resolved_values`. V1 semantics per `document/variables.rs`:
  `expr` is a **bare variable name**; a trimmed non-identifier (arithmetic,
  `w * 2`) fails as an UNSUPPORTED EXPRESSION rather than being looked up as a
  literal name and reported missing. A variable whose own value is
  expression-driven is refused too (chained expressions are not V1).
- [x] **Substitution site: the regen mirror**
  (`document_runtime::sync_regen_timeline` + `from_document`). The mirror
  timeline now holds the EFFECTIVE records; `session` keeps the stored ones
  verbatim, and `build_save_payload` writes the stored ones, so nothing on disk
  moves until the write-back below. Placing it there rather than inside
  `begin_regen` is load-bearing: ONE substituted record set feeds the planner
  hash, the checkpoint-staleness guard (`history_prefix_hash` over the same
  effective records ⇒ a variable edit correctly invalidates checkpoints), the
  wire lowering (`wire::wire_op` serializes `PlannedOp.operation`), AND the
  executor's replay-from-0 fallback (`RegenPlanner::without_checkpoint` re-plans
  off the scratch timeline). Hash and geometry cannot disagree.
- [x] **Per-step failure channel** — an unresolvable binding is stamped
  `StepState::Error { reason }` on the mirror by `substituted_timeline` (the
  state is a pure function of records × variables, so it needs no worker
  round-trip), and `begin_regen` folds the lowest broken step into the SAME
  execution ceiling the SCHEMA §7.3 seeded-repair gate uses — the plan stops
  strictly below it, publish ≤ m−1, downstream steps stay Dirty. It reaches the
  UI through `feature_dto`/`failed_steps_of` unchanged. **Never** a fallback to
  the stale cached `value`; a WARN rides the regen lane.
- [x] **Derived write-back** — `sync_variable_values`
  (`document_runtime` → `DocumentSession::sync_variable_values` →
  `Timeline::sync_resolved_scalar_values`), called from `finish_regen`'s
  published branch right beside `sync_record_outputs`/`sync_mate_placements` and
  scoped to `executed`. No undo entry, no revision bump: `Scalar.value` under an
  `expr` IS documented as the last evaluated value. `expr` is never rewritten,
  and the number written is the one the plan was hashed with, so re-substituting
  it is a no-op — no hash moves, no checkpoint dies from the write-back.
- [x] **Scalar inventory** — `KnownOperation::scalars_mut()`
  (`document/record.rs`), mirroring the `element_refs_mut` hand-table
  discipline: Extrude `distance`/`draftAngleDeg`/`distance2`, Revolve
  `angleDeg`, Fillet `radius`, Chamfer `radius`/`distance2?`, Shell `thickness`,
  LinearPattern `spacing`, CircularPattern `angleDeg`, ImportStep `unitScale`,
  TransformBody `translate[0..3]`/`rotate.angleDeg`, Hole
  `diameter`/`depth?`/`cb*`/`cs*`, OffsetFace `distance`, PlaceComponent
  `placement.translate[0..3]`/`placement.rotate.angleDeg`. Field ORDER is
  normative (the write-back position-zips two records' lists).
- [x] **SCHEMA: no wire change, verified.** `Scalar` already serializes as
  `{value, expr?}` and §7.3 (amended 2026-07-16) already requires both readers
  to accept the object form; `worker/src/ops/OpCommon.cpp::read_scalar` takes
  `value` and ignores everything else, so `expr` rides the wire inert exactly as
  before. No fixture bump, no §14 entry.
- [x] Gate, all RUN: `ctest --test-dir worker/build` **124/124** ·
  `cargo fmt --all --check` clean · `cargo clippy --workspace --all-targets
  -D warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace
  --no-fail-fast` **exit 0**, every target green including the golden
  hash-pin/corpus targets (`regen_planner` 13, `corpus_executor`,
  `wire_contract`, `m2_gate`). New: 10 core unit tests in `regen/variables.rs`,
  1 planner golden (`a_document_without_expressions_plans_identically_with_a_
  populated_variable_table` — a populated variable table cannot move a single
  plan hash on a doc that binds nothing), 3 worker-backed tests in
  `src-tauri/tests/variable_driven_ops.rs` (exact `QueryMassProperties`:
  4000 → 8000 on `SetVariable 10 → 20`; a missing variable errors THAT step with
  a message naming `Extrude.distance` + `height` while the sketch step stays
  fine and no body is built off the stale value; stored record keeps
  `expr` + stale `value` pre-regen, resolved value written back post-regen, both
  round-tripped through a real container save/reopen).

### WP-VE.3 — sketch dimensional constraints (DEFERRED, not in VE.1)

The 6 `Scalar`-typed constraint values in `sketch/constraint.rs`
(`Distance`/`HorizontalDistance`/`VerticalDistance`/`Angle`/`Radius`/`Diameter`)
CANNOT ride the op-param pass, because the sketch lane is structurally
different, not merely another field list:

* `SketchOpParams.entities`/`constraints` are **opaque `serde_json::Value`** —
  an already-lowered, already-SOLVED wire snapshot, minted by
  `document_runtime::sketch_record_op` → `wire::sketch_wire` at finish/backfill
  time. The typed `Constraint` with its `Scalar` never reaches the timeline
  record, so substituting record params would rewrite solved coordinates, not a
  driving dimension.
* The authoritative typed sketch lives in `Document.sketches` and is solved on
  the SCHEMA §7.4 solver lane (`enter_sketch` → `wire::sketch_upsert_args`),
  which regen never re-runs.

**Where the hook goes:** resolve the constraint `Scalar`s (same
`regen::variables::resolve_expr`) when building the solver payload in
`wire::sketch_upsert_args`/`sketch_wire`, AND add a variable→sketch invalidation
that re-solves the affected sketches and refreshes their `Sketch` records via
`sketch_record_op` before the plan compiles — otherwise a variable edit would
change the solve but not the record the worker replays. That re-solve pass is
the actual work; the substitution itself is two lines.

## COMPONENT-LIBRARY WP-F3 (2026-08-13) — GATE PASSED

Placement-gesture polish + mock-lane parity (spec §5.4). Frontend only — no
Rust, no worker, no protocol change.

- [x] **Free-space follow + drop (spec §5.4 steps 1/6)** — closes the WP-1.5
  scope cut recorded in `placementController.ts`'s header (comment rewritten,
  not left stale). With no valid snap target the ghost follows the camera ray ∩
  world `z = 0` (`placementSolver.groundPlanePoint`, a new pure helper — the
  existing solve math is untouched) at identity rotation, and a click commits
  **with `mate: undefined`**. A ray that cannot reach the plane (parallel, or
  pointing away) leaves the ghost where it was and the commit follows the
  ghost — never a fabricated point. Snap behaviour is byte-identical: all three
  "no target" branches route to the new lane, the matched branch is unchanged,
  and the pre-existing auto-size/mate cases still pass verbatim.
- [x] **Mock lane classifies cylinders, MEASURED** (`mockMeshMetrics.
  cylinderMetricsFromMesh`): facet-normal covariance → least-significant
  eigenvector as the axis, a Kåsa circle fit in the perpendicular plane, and
  BOTH the perpendicularity and the fit residual checked before an answer is
  returned. A box face / a flat cap / an annulus still answer `other` with
  `frame: null`. The circle is FITTED rather than averaged because the seam
  vertex is duplicated — a plain centroid sits off-axis and would fail its own
  equidistance test.
- [x] **`?vpdemo=cyl` publishes a bushing with a real Ø8.5 bore**
  (`mockMeshes.makeBoredCylinderMesh`, `mockClient.seedMockDemoCylinder`) —
  opt-in for the same reason `?mocklibrary=1` is: an always-on second body
  changes every existing spec's body list and camera fit. Ø8.5 is the M8
  clearance hole, so auto-size resolves to a size the armed default is not.
- [x] **Mock catalog + templates mirror what the app ships**: `iso15` /
  `nema17` / `nema23` mirrored field-for-field from
  `src-tauri/resources/library-seed/*/component.toml` (identity, `code`/`length`
  free params + domains, table-locked dimensions, attachment `accepts`), and the
  three starters from `library_seed_templates.rs` — both behind `?mocklibrary=1`,
  which is the mock's stand-in for a SEEDED LIBRARY ROOT. This clears WP-F2b's
  flagged frontend seam ("`mockTemplates` starts EMPTY, so the mock lane shows
  no starters"). `setComponentParams` now resolves the component by identity
  instead of comparing against the one SHCS id (a motor's `length` edit failed
  on this lane only). Geometry parity is still NOT claimed — every mock
  placement is the same synthetic solid.
- [x] Gate, all RUN: `bunx tsc --noEmit` clean for every file in this change
  (the only errors in the tree are in `SaveAsComponentDialog.tsx`, another
  agent's in-flight WP — untouched here) · `bun run test` **4302/4302 in 262
  files** · `bunx vitest run src/modules/library src/ipc src/features/start`
  694/694 · Playwright chromium, FULL suite: **210 passed, 1 failed** — the 2
  new `library-place-freespace.spec.ts` cases and the new starter-grid case
  pass, and `library-browse-place-snap` / `library-template` / `library-preview`
  / `library-author-component` stay green (9/9). `retries: 0` throughout.
- **PRE-EXISTING FAILURE, not from this WP** —
  `extrude-commit-gesture.spec.ts` › "clicking empty canvas away from the handle
  commits (click-away)": the click-away commits no body (Bodies stays at 1).
  Reproduced DETERMINISTICALLY in a clean `git worktree` at HEAD (`3b43c21`)
  with none of this change present, so it is a real product defect that landed
  earlier, not a regression here and not a flake. Needs its own investigation.
- **FLAGGED SEAM:** "a free-space commit records NO mate" has no UI surface to
  read — nothing renders a placement's mate — so the e2e asserts the observable
  consequence (the body lands on the ground point with identity rotation) and
  the mate-absence itself is pinned in `placementController.test.ts`. Giving the
  inspector a mate row would make it e2e-visible; not in this WP's scope.
- **FLAGGED SEAM:** `library-browse-place-snap.spec.ts`'s card locators had to
  be scoped by name — the opt-in catalog is four cards now, and an unscoped
  `getByTestId("library-card")` is a strict-mode violation.

## COMPONENT-LIBRARY WP-F1.2 (2026-08-13) — GATE PASSED

"Save as Component" authoring completed (spec §7/§9): attachments are PICKED in
the viewport, and a multi-solid body is offered a union instead of only a
refusal. Closes the two "deliberately does not do" items the WP-B2 dialog
carried (bar parameter roles, which stays out — a `document` package has baked
geometry and no re-bake lane).

- [x] **Attachment picking** (`src/modules/library/attachmentPicker.ts`, new).
  Copies `placementController`'s gesture pattern rather than sharing it
  (capture-phase `pointerdown`/`keydown` on `window` + `setOrbitSuppressed`);
  `placementController.ts` untouched. A pick → `engine.probePick` →
  `geometryQuery.classifyElement` → `deriveAttachment`: planar face ⇒
  `accepts:["plane"]`, cylinder ⇒ `["cylinder","hole"]`, circular edge ⇒
  `["cylinder","hole","circularEdge"]` — checked against `placementSolver`'s own
  snap table in a test, so an authored attachment cannot fail to match the snap
  kind its geometry produces. Anything else stays in pick mode with a hint. Esc
  leaves PICK mode, not the dialog.
  - **Frame origin is the CLICKED point** (projected onto the plane, or onto the
    axis for a cylinder), not OCCT's surface origin — that one is a
    parametrization artifact and would seat the component where the author never
    pointed. A circular edge uses its own centre. `x` is world X projected ⊥ z,
    falling back to world Y — stable, so re-picking a face re-authors the same
    frame. Frames are WORLD coords, which is right because `ExportGeometry`
    bakes session coordinates verbatim (world == component-local after the bake).
  - Services reach the dialog through `configureAuthoringController` in
    `register.ts`, the same soft-lookup `configurePlacementController` uses
    (ADR-0002; a harness without modeling's bootstrap simply cannot arm).
  - Dialog rows: editable name (validated `[a-z0-9_-]`, non-empty, unique —
    a duplicate would silently drop an attachment from the manifest table),
    `on`/`accepts` summary, frame badge, Remove. `on` follows the name
    (`face:seat`), so the two cannot drift. No rows ⇒ the pre-F1.2 model-origin
    seat, unchanged.
  - While picking, the dialog's scrim goes `pointer-events-none` and stops being
    a click-to-close target; events inside the dialog are let through untouched.
- [x] **Union at bake** (SCHEMA §7.8 `union` + §14 additive entry). Worker fuses
  (`BRepAlgoAPI_Fuse` chain) BEFORE writing, and checks the RESULT's solid
  count, not just `IsDone()` — OCCT "fuses" disjoint solids into a compound of
  both, and writing that would move the failure to whoever places the component.
  Face colors are dropped for a fused result (the face set is rewritten).
  Rust threads `union_solids` through `GeometryExporter`/`WorkerManager`/
  `wire`/`save_as_component`; the multi-solid refusal now carries the marker
  `library::MULTI_SOLID_REFUSAL` (`"MULTI_SOLID_BODY"`) because `ApiError` has
  no code field and the dialog must not key its offer on prose. The offer is
  honest on screen: pick-primary and split are NOT offered in v1.
  `CadClient.saveAsComponent` gained an append-only `unionSolids?` (mock accepts
  and ignores it — nothing is baked on that lane).
- [x] Gate, all RUN: `scripts/build-worker.sh Release` + `ctest` **124/124** ·
  `cargo fmt --all --check` clean · `cargo clippy --workspace --all-targets -D
  warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace
  --no-fail-fast` **0 failed** (78 suites; `component_ops` 13/13 incl. the new
  multi-solid refusal + union opt-in against the real worker) · `tsc --noEmit`
  clean · vitest **4302/4302** (262 files; +14 new: dialog picking/union, 4
  `attachmentPicker` unit) · hex gate empty.
- Seam flagged: the union offer re-submits the WHOLE save, so a backend that
  fails after the bake (a taken id) is retried end to end. Harmless today
  (the bake is the expensive part and it is a temp file), but a future
  "bake once, retry the write" lane would want a prepared-bake handle.

## COMPONENT-LIBRARY WP-F1.1 (2026-08-13) — GATE PASSED

Attachment local frames travel end-to-end: optional
`[attachments].<key>.frame = { origin, z, x }` in `component.toml`
(normalized at parse; left-handed bases unrepresentable — y is derived),
frozen into `ComponentMate.self_frame` at placement by the package lookup,
carried as optional `mate.selfFrame` on the wire (SCHEMA §7.3 + §14 additive
entry), honored by BOTH solvers via `S ∘ F⁻¹` (rotation `R_s·R_fᵀ`,
translation `T_s − R·origin_f`) — worker `ComponentMateSolver.cpp` and FE
`placementSolver.ts` in verbatim parity with matching numeric test cases.
Absent frame ⇒ identity ⇒ byte-identical pre-F1.1 behavior (existing reseat
ctests unchanged). Components no longer have to seat at their model origin —
the enabler for authored attachment placement (F1.2).

- [x] Gate (run on the combined F1.1+F2b tree): worker rebuilt, ctest full
  pass · `cargo fmt/clippy` clean · `ONECAD_REQUIRE_WORKER=1 cargo test
  --workspace` 0 failed (component_ops 12/12) · `tsc` clean · vitest
  library+ipc 639/639.

## COMPONENT-LIBRARY WP-F2b (part 2: starter templates) — GATE PASSED

Spec §8's "ship 3–5 honest starters" — the half WP-F2 left queued. Three seed
beside the component packages on first run and show in the start screen's
template grid with **no frontend change** (`listTemplates` → `TemplateGrid`
already renders whatever the library root holds; a starter carries no preview
PNG, so the card falls back to the existing hatch + cube well).

- [x] **Documents are GENERATED at seed time, not checked in as bytes**
  (`src-tauri/src/library_seed_templates.rs`, new). Built with `onecad-core`
  types and frozen through the same `ContainerWriter` a real save uses, so a
  document-schema / container-layout / record-shape change either compiles or
  does not — a hand-frozen binary would rot silently and fail to open.
- [x] **What the three actually contain (honest inventory, not a wish):**
  - `onecad.std.template.blank` — "Blank". An empty document, mm. Nothing else.
  - `onecad.std.template.printed-part` — "3D-Printed Part". Empty document plus
    ONE resolved `OffsetFromPlane` datum on XY at 0 named "Build plate" (real,
    selectable content — it lists in the sketch-plane picker beside the world
    planes). Deliberately nothing more: **there is no print-settings machinery
    in the document model** (no nozzle, layer height or bed size to encode) and
    the variable table has no UI, so anything else would be decoration.
  - `onecad.std.template.nema17-mount` — "NEMA 17 Motor Mount". Empty document
    plus one `PlaceComponent` record for the seeded `onecad.std.nema17` at the
    origin, generator source, **no mate** (there is nothing in a starter to
    mate to, and an unresolvable recorded mate is exactly the `NeedsRepair`
    this project exists to avoid). Version + revision are read out of the seed
    manifest at build time, so the starter can never claim an identity the
    shipped package does not have.
- [x] **Seeded in the SAME pass, under the SAME marker** (`SEED_VERSION` 2 → 3,
  which is what installs them into an already-seeded library root). The
  component rule is reused verbatim: the check is on the DIRECTORY, the user's
  copy always wins, and a deleted starter stays deleted until the version moves.
  `SeedOutcome` gained `templates_installed`/`templates_kept` (reported apart
  from the package split because a template is not an `IndexEntry` and never
  enters the component index).
- [x] Gate, all RUN: `cargo fmt --all --check` clean · `cargo clippy
  --workspace --all-targets -D warnings` clean · `ONECAD_WORKER_PATH=…
  ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` **0 failed**
  (79 suites), incl. 6 new: 4 unit (`library_seed_templates`: every starter is
  a readable container / blank is empty / the datum resolves / the NEMA record
  matches the shipped package + kept-not-overwritten), 2 worker-backed in
  `tests/component_ops.rs` (`the_seeded_nema17_starter_regenerates_into_a_motor`
  hits the exact analytic volume **72636.60 mm³**, the same `motor_volume`
  formula the C++ suite pins; the two geometry-free starters open and
  regenerate to `NoOp`).
- **FLAGGED SEAM (frontend, deliberately untouched):** `mockClient` backs
  `listTemplates` with an in-session `mockTemplates` array that starts EMPTY, so
  the mock/Playwright lane still shows no starters (only ones saved during the
  session). The real (tauri) lane reads the seeded root and needs nothing.

## COMPONENT-LIBRARY WP-F2 (part 1: bearings + steppers) — GATE PASSED

Spec §6.2's two non-fastener families. The starter-template half of WP-F2 is
NOT in this pass and stays queued.

- [x] **`iso15` — ISO 15 deep-groove ball bearing**, keyed by BEARING CODE
  (`625 608 6000 6001 6200 6201 6202 6802`). ONE revolved solid, not two
  rings: a component resolves to exactly one solid (spec §9) and a real
  bearing's rings are connected only through its balls, so the r–z section is
  the boundary rectangle with a shallow annular relief in both faces — reads
  as a bearing, stays one connected ring ("stepped-ring geometry suffices").
  Race wall / relief depth are RENDERING fractions, deliberately not table
  data: ISO 15 tabulates the boundary dimensions and nothing inside them, so a
  tabulated race width would be an invented dimension.
- [x] **`nema17` / `nema23` — NEMA stepper motors**, one generator per frame
  (the house style is one id per family). Body block + pilot boss + shaft
  fused, four blind mounting holes cut in one boolean. Body length is the only
  free dimension; below the frame's minimum it is REFUSED, never clamped.
- [x] **Tables self-authored** (`ops/MachineElementTables.h/.cpp`, new file —
  neither family is a fastener, neither is keyed by a thread, neither comes
  from BOLTS). Spec §6.3 asks for exactly this; **no `THIRD_PARTY_NOTICES`
  entry is owed** because nothing was copied.
- [x] **Op layer stopped naming string params one by one.** `source.params`
  now forwards EVERY string to the generator verbatim (`thread` is read back
  out of that map, so the two cannot drift), which is what lets `iso15` be
  keyed by `code` with no wire change. `length` gained an absent-vs-present
  flag so a motor defaults to its own frame's body length instead of
  inheriting a screw's 20 mm.
- [x] **Seeds** `onecad.std.iso15` / `.nema17` / `.nema23` (`SEED_VERSION`
  1 → 2, which is what restores them into an already-seeded library root).
- [x] **SCHEMA §7.3 registered-id list + §14 entry.** Additive only: no field
  added/removed/retyped, `source.params` was already an open map, no fixture
  moves.
- [x] Gate, all RUN: ctest **124/124** · `cargo fmt --check` clean · `cargo
  clippy --workspace --all-targets -D warnings` clean · `ONECAD_REQUIRE_WORKER=1
  cargo test --workspace --no-fail-fast` **1183 passed / 0 failed** (79
  suites), incl. `every_seeded_component_places_through_the_real_worker` and
  `every_seeded_component_meshes_for_the_library_ui` over all **10** seeded
  packages.
- **FLAGGED SEAM (frontend, deliberately untouched here):** the mock lane's
  own catalog in `src/ipc/mockClient.ts` still lists the seven fastener
  packages only, so the mock/Playwright UI shows no bearing or motor until it
  is extended. The real (tauri) lane reads the seeded library and needs
  nothing. Also unshipped: `ComponentParametersSection`'s free-param control
  is fine for `code` (a domain dropdown) but nothing resolves a `role="table"`
  `from = "iso15.bore"` yet — those rows are declarative, as they were for
  every prior package.

## COMPONENT-LIBRARY HARDENING WP-H0…H4 (2026-08-13) — GATE PASSED

Full-branch review (3 investigators + 3 reviewers over the ~22.8k-line diff)
then hardening. Plan: `~/.claude/plans/iterative-splashing-raven.md`.

### WP-H0 — master merged, resolution audited, full gate re-run
- [x] `master` (39f5839) merged as `f242712`. The PRIOR in-tree resolution had
  dropped ALL FOUR master-side SCHEMA hunks (ResolveRefs snapshot echo, draft +
  boolean diagnostic codes, pattern-lineage fixture entry) — re-merged keeping
  both sides; tracker docs stack both session threads; `mockClient` keeps
  `RegenTerminal` + `ReindexReport`.
- [x] Post-merge gate, all RUN: ctest **124/124** · `cargo fmt/clippy` clean ·
  `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **0 failed** · `tsc` clean ·
  vitest **260 files / 4261** · Playwright **410/410** (23.4m, retries 0) ·
  hex gate empty. Branch verified NOT breaking master.

### WP-H1 — component id/version path safety (security)
- [x] `validate_identity` (onecad-library) + `PlaceComponentParams::validate`
  (onecad-core) now reject path-escaping ids/versions (`/`, `\`, `..`,
  charset-pinned) — both values name the package directory
  (`<root>/<id>@<version>`), so `onecad.std/../evil` could previously escape
  the library root. Tests at both entry points.

### WP-H2 — mate authoring from the placement gesture (spec §5.4 step 5)
- [x] The gesture commit now RECORDS its snap: `placementController` sends
  `PlaceComponentMate` (attachment key, target bodyId+topoKey+elementId,
  snap kind, flip, anchor world point) through
  `CommandApiService.placeComponent` → new `place_component` `mate` arg →
  `resolve_mate_input` promotes the topoKey to a Rust-minted ElementId at the
  head (fail CLOSED — an unpromotable mate refuses the whole placement) →
  `PlaceComponentParams.mate`. WP-3.1's regen re-seat lane finally has a UI
  producer; spec §12's "move the plate, screws re-seat" is now reachable.
- [x] EN ROUTE BUG (register.ts): the `CommandApiService.placeComponent`
  forwarding DROPPED `params` — auto-size ghosts previewed the sized screw,
  commits placed the default. Fixed; every arg forwards.
- [x] Tests: vitest pins gesture→mate arg (incl. flip); Rust pins
  record-carries-mate + fail-closed refusal; existing worker reseat ctests
  unchanged. e2e mate assertion deferred (no client seam from Playwright to
  read record params; chain is pinned unit+integration instead).

### WP-H3 — frontend robustness
- [x] `componentPreviewScene`: `webglcontextlost` now resets the shared
  offscreen renderer to "not tried yet" instead of latching the null terminal
  state (context loss used to kill every future thumbnail for the session).
- [x] `mockClient`: `resolveComponentSource`/`placeComponent` now accept this
  session's AUTHORED components (fixture-geometry reuse, own identity) —
  unblocks author→place on the mock lane.
- [x] Reviewer claims verified NO-CHANGE: ComponentPreview3D lifecycle (the
  `disposed` flag already guards every async path), placementController
  listener teardown (cancelPlacement runs in every failure branch),
  localSolver rotate fallback (controller always sends rotate).

### WP-H4 — spec ratification (docs only)
- [x] Spec gains §13 ratifying the four deviations: in-place
  `SetComponentParams`/`ReplaceComponent`, mate NOT in wire `inputs[]`,
  `[source]` codec/format + `document.geometry`, model-origin seating.

Remaining (approved scope, next): WP-F1 authoring completion (param-role UI +
attachment placement + single-solid choice), WP-F2 bearings/NEMA tables
(LANDED — see the WP-F2 section at the top) + starter templates (still
queued), WP-F3 gesture polish (free-space follow, auto-size e2e via
mock cylinder classify; Tab-cycle turned out ALREADY SHIPPED —
`placementController.ts:420`).

## COMPONENT-LIBRARY WP-B5 + WP-B6 (2026-08-13) — GATE PASSED, PHASE B CLOSED

Two things at once: P3's remaining e2e coverage (WP-B5), and — at the user's
request, with the screenshot that prompted it — real 3D previews for every
component (WP-B6).

### WP-B5 — e2e for authoring and templates

`library-author-component.spec.ts` drives the wiring four modules only meet at
runtime in: a body row's context menu (modeling's row, the library's item,
`platform.menus` in between) → the `Slots.ShellOverlay` dialog → the catalog
read the panel makes. `library-template.spec.ts` drives the whole template loop
— command palette → dialog → close project → the start screen's Templates row →
a NEW document from it.

**A real defect the first e2e caught**: `LibraryPanel` loaded its catalog once
at editor mount, so a component authored while the editor was open never
appeared — the user's own save looked like it had failed. It now reloads
whenever the library tab becomes visible, which also covers a reindex done from
the start screen.

Both specs state what they do NOT own: the geometry half (bake, single-solid
refusal, the package that places back as the same solid) is a real-worker
concern and is tested there. The mock lane has no kernel, and faking one would
prove nothing.

### WP-B6 — component previews

Every card showed the same generic cube icon; the catalog was unbrowsable by
eye. Components now render their real geometry.

**`component_preview_mesh`** runs a `PlaceComponent` candidate through
`PreviewOp` (SCHEMA §7.6) — a throwaway copy of the worker's session head — and
returns MESH1. It works with **no document open**, which is the case that
matters: the most useful place to browse a catalog is the start screen, before
any project exists. It picks the open document's worker when there is one and
the PRE-WARMED worker otherwise, rather than `AppState::preview()` (which is
`PendingBackend` until a document opens). Blob-backed components materialize
into a preview-lane `ImportWorkspace`, since there is no document carrier to
stage into.

**One WebGL context, not one per card.** A browser allows ~16 live contexts and
a catalog can hold hundreds, so cards get a data URL from a single shared
offscreen renderer (`componentPreviewScene`), cached per `id@version[#params]`,
with concurrent callers sharing one request — a grid mounts every card at once,
and 30 cards must not ask the kernel to build the same screw 30 times. The
DETAIL view (`ComponentPreview3D`) is the only live context: its own small
scene, drag-to-orbit, mounted only while a component is selected. It is used by
the start-screen browser and by the inspector, where it previews the placed
instance AS CONFIGURED (keyed on the live free params, so changing a size
re-renders it).

Colors come from `palette` — the viewport's own token-resolved colors — so a
preview reads as the same material as a real body and re-themes with it.

**Every failure degrades to the old behaviour**: no worker, no WebGL, an
unbuildable component, a malformed mesh — all end in `null` and the card shows
its icon. A component that cannot be pictured is still listed, searchable and
placeable.

**A defect found by RUNNING it**: the preview minted its body id from a
decorated op id (`preview_<uuid>`), and `body_<opId>` must parse as a UUID —
the wire refused every preview. Caught by the real-worker test, not by reading.

**Contention flakes, recorded not chased.** Both appeared only while another
full suite was saturating the machine, and both were re-proven green in
isolation immediately after — the same SwiftShader-load signature this repo's
own gate notes already describe:
- two vitest runs launched DURING the Playwright sweep each reported one
  unnamed failure; two consecutive runs on a quiet machine were 4206/4206;
- the final Playwright sweep (run while those vitest runs were going) failed
  `revolve-preview.spec.ts` — a foreign spec this WP touches nothing in — and
  it passed 2/2 on re-run. The sweep 20 minutes earlier, on the same code
  modulo two comment-level frontend edits, was **416/416**.

GATE: worker ctest **119/119** · `ONECAD_REQUIRE_WORKER=1 cargo test
--workspace` green (new: `every_seeded_component_meshes_for_the_library_ui` —
every seeded component meshes to a real MESH1 with vertices, through the same
command the cards call) · `cargo fmt`/`clippy -D warnings` clean · `bunx tsc
--noEmit` clean · vitest **251 files / 4206** · Playwright **416/416** (see the flake note above) · hex
gate empty.

## COMPONENT-LIBRARY WP-B3 (2026-08-13) — GATE PASSED

Project templates (spec §8) — the start screen's `templates` nav key has been
a stub since the start screen shipped; it is now real in both directions.

**Same storage discipline as a component, deliberately a DIFFERENT index.**
`templates/<id>/` holds `template.toml` + `template.onecad` (+ optional
preview), written with the same one-directory-per-entry, refuse-don't-overwrite
rules. What they do NOT get is an entry in `library.json`: a template is not
placeable, has no parameters, attachments or source kind, and giving it an
`IndexEntry` would force every consumer of the component index to learn to skip
a kind it can never use. Templates are listed by reading the directory — there
are a handful, and the read happens on the start screen, not in a regen loop.

**A template is immutable by never being the save target.** `new_from_template`
opens its container like any other document and then calls the new
`DocumentRuntime::detach_from_file`, so the copy is untitled and dirty and the
first Save prompts for a location. Nothing marks the file read-only, and
nothing should — a user editing their own template file is not doing something
wrong. It is also deliberately NOT recorded in recents: nothing has been saved,
and an entry pointing into the library would reopen the template itself.

**"Save as Template…" is a COMMAND, not a File-menu item.** The File menu lives
in `features/shell`, and the shell must not import the library to offer a
library action (the same wall WP-B1 exists to respect). A registered command is
the module-owned way in, and the palette already lists every one.

**Content scope, as decided with the user**: the MECHANISM ships; the
opinionated starters (3D-print part, NEMA mount plate, enclosure base) do not.
Authoring those means committing to datum placement and NEMA footprint accuracy
that should be reviewed, not guessed — carried Q4 stays open. The empty state
says how to make one rather than pretending the row is broken.

**Skipped-not-fatal, everywhere it matters**: a template directory whose
manifest is unparsable, or which has no document to instantiate, is skipped and
the rest still list. An unreadable library root reads as "no templates" on the
start screen rather than an error banner — the screen's job is to get the user
into a project.

GATE: vitest **249 files / 4195** (up from 248/4191) · `bunx tsc --noEmit`
clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1168 passed / 0
failed** (5 new `template.rs` tests, including the path-traversal refusal —
an id IS a directory name) · `cargo fmt`/`clippy -D warnings` clean · worker
ctest 119/119 unchanged (no C++ touched).

## COMPONENT-LIBRARY WP-B4 (2026-08-13) — GATE PASSED

`ReplaceComponent` + opt-in version upgrade (spec §3.3) — the last named P3
operation. The Rust half landed with WP-A3's commit; this closes the UI.

**In place, at the same `RecordId`.** SCHEMA §7.3 already fixed the design:
this is not a wire op but an `UpdateOperationParams` on the existing
`PlaceComponent` record, so identity, timeline position, every downstream
reference and the minted `body_<opId>` all survive. Deleting and re-adding
would break all four.

**A mate rides across by ATTACHMENT NAME, or not at all.** A recorded mate
names an attachment on the OLD component; the new one is a different package
with its own table. Same name ⇒ the mate is carried unchanged (target, kind and
flip are all still valid, and regen re-seats it). No such name ⇒ the mate is
dropped, the instance keeps its frozen placement, and the report NAMES what was
lost. Binding to the new component's first attachment instead would be the
mis-bind spec §0 invariant 4 forbids; dropping it silently would leave the user
to discover later that their part stopped following its target.

**The upgrade offer is a REPORT.** `component_upgrade_available` looks only,
and the inspector renders a row the user has to click. Nothing upgrades itself
— a document opening differently because a shared library moved underneath it
is the Toolbox failure this whole design exists to avoid. Pinned by a test that
renders the section with an offer present and asserts `replaceComponent` was
NOT called.

**`LibraryIndex::latest` orders versions numerically** rather than by the
`BTreeMap`'s lexicographic key order, which had `1.9.0` sorting after `1.10.0`
— the first bump past 9 would have offered a DOWNGRADE as an upgrade.
`newer_than` is strict, so an instance recorded at a version newer than
anything indexed (a partially-synced library root) is offered nothing.

**Inspector**: the section no longer returns null when a component declares no
free params — a `document`-source component has none by construction, and
replace/upgrade are exactly what its instances need.

GATE: vitest **248 files / 4191** (up from 248/4187; 4 new upgrade/replace
tests, and the existing configurator tests gained the two new client methods) ·
`bunx tsc --noEmit` clean · Rust side already green in WP-A3's gate
(`replace_swaps_identity_at_the_same_record`,
`replace_carries_a_mate_by_attachment_name_and_reports_a_dropped_one`,
`upgrade_is_offered_only_when_a_strictly_newer_version_is_indexed`).

## COMPONENT-LIBRARY WP-B2 (2026-08-13) — GATE PASSED

"Save as Component" (spec §7) — a body at head becomes a reusable
`document`-kind package, and places back as the same solid.

**The bake is the design.** The package stores the frozen authoring document
AND a solid exported out of the live session (`ExportGeometry`, §7.8), and a
placement copies the baked solid. Nothing ever replays a frozen document (spec
§4 wants the geometry in the placing document anyway, and the worker is one
session per process). The frozen document is provenance plus the input a future
re-bake lane would replay.

**Spec §9's single-solid rule is enforced at SAVE time**, where the author can
still union/split, rather than at placement on someone else's machine.
`ExportGeometry` reports the count it actually wrote.

**Two honest limits, stated in the dialog rather than faked:**
- **No face-clicking to place attachments.** The solver seats a component by its
  local origin and +Z, and nothing on the wire carries a per-attachment frame —
  an attachment placed on an arbitrary face could not be honoured at placement.
  So the authoring rule is the one every built-in generator already follows:
  **the component seats at its model origin**. The dialog says so. Per-attachment
  frames (`mate.selfFrame` + a canonicalizing bake) are the follow-up.
- **No parameter roles.** A baked source has no re-bake lane, and
  `set_component_params` already refuses to edit one; declaring free params here
  would offer an edit that cannot be honoured.

**Storage** (landed with WP-A3's commit, exercised here): `package::to_toml` +
`write_package` (write → recompute the revision over what is on disk → rewrite),
content-addressed blob put, and **version-qualified package directories** — the
last found by a test, because a bare `<id>` directory made a 1.1.0 save
overwrite 1.0.0's manifest and leave every instance recording 1.0.0 resolving
`NeedsRepair` with no file left to explain why. Re-saving the same id@version is
refused: that is a semver bump, not an overwrite.

**The test that proves the whole claim**:
`a_body_saved_as_a_component_places_back_as_the_same_solid` authors a published
body against the REAL worker, then resolves the package out of the library and
places it into a FRESH document, asserting the exact kernel volume matches.
Everything else (bake, blob, manifest, index, placement) is tested separately;
only this shows they compose.

**Frontend**: the menu item is a `Slots.TreeContext` contribution (WP-B1) gated
on `kind === "body"`, so the library reaches a row modeling provides without
either module importing the other. The dialog is a `Slots.ShellOverlay`
occupant; it stays OPEN on a backend refusal, because every way this can fail is
something the author fixes in place, and closing would discard their typing.

**`shellContract.ts` amended** (a user-visible change, per that file's own
rule): `SaveAsComponentHost` joins `Slots.ShellOverlay` at the end, after the
four shell modals. It renders `null` until a body is being authored, so nothing
moved and nothing new is visible until the user asks for it.

GATE: vitest **248 files / 4187** (up from 247/4178) · `bunx tsc --noEmit` clean
· `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1163 passed / 0 failed** ·
`cargo fmt`/`clippy -D warnings` clean · worker ctest 119/119 unchanged (no C++
touched) · hex gate empty.

## COMPONENT-LIBRARY WP-B1 (2026-08-13) — GATE PASSED

Cross-module context-menu contributions — the platform seam "Save as
Component" needs, and the one W10 flagged when it shipped `TreeNodeAction`
("P3's addon therefore gets rows but not menu items").

**The gap, precisely.** A tree provider's own rows can already declare
`node.actions`, so the module that OWNS a row can add items to it. Nothing let
a DIFFERENT module do so — and "Save as Component…" belongs on a body row that
`onecad.modeling` provides, while the item belongs to `onecad.library`. Neither
module may import the other (ADR-0002), so without this the only route was
another `node.kind` branch inside the shell, which is exactly what ADR-0003
forbids.

**`platform.menus`**, a registry like every other: `MenuContribution {id, slot,
title, danger?, confirm?, appliesTo?, run}` addressed by `Slots.TreeContext` /
`Slots.ViewportContext` (both were declared and had no consumer), ordered by
`(priority, registration index)`, owner-namespaced ids, duplicate = hard
failure, swept with its owner.

**Ordering rule, frozen**: capability items (Rename, Hide/Show) → the row's own
declared actions → contributed items, each block behind its own separator. A
foreign module can append, never interleave or displace — the first thing a
user notices otherwise is the item they aim for by muscle memory moving under
their cursor. `src/test/contracts/treeMenuContract.ts` pins the per-kind label
order and `ModelTreePanel.menu.test.tsx` probes it through a REAL registered
provider plus a real contribution from a second module.

A contribution whose `appliesTo` throws is treated as not applying: one bad
predicate must not take the whole menu down with it. Pinned by test.

GATE: vitest **247 files / 4178** (up from 246/4172; 6 new menu tests + the
teardown sweep now asserts `platform.menus`) · `bunx tsc --noEmit` clean · no
Rust/C++ touched.

## COMPONENT-LIBRARY WP-A3 (2026-08-13) — GATE PASSED, PHASE A CLOSED

Auto-size on hole rims (spec §5.3's hole row, §5.4 step 3) — the last named
content gap, and the step that makes spec §12's flagship sentence true end to
end: drag a screw onto a hole and it arrives at the hole's size.

**No wire change was needed, which was worth checking before designing one:**
`ClassifyElement` already returns `frame.radius` for cylinder and circle frames
(SCHEMA §7.5) and it already reaches the frontend as `ClassifyResult.frame
.radius`. The measurement was there; nothing consumed it.

**The picker is pure and refuses rather than substitutes.**
`nearestSmallerThread(holeDiameter, domain)` takes the largest declared size
whose NOMINAL diameter still fits — an M6 clearance hole (6.6) takes an M6, not
the M8 that cannot pass. Nothing fitting returns `null` and the armed size
stands: a 1 mm hole gets no auto-size rather than the smallest thread in the
domain, which would be the Toolbox failure mode in miniature. A non-metric
designation (an inch series, a bearing code) opts out instead of being guessed
at.

**The real work was making the ghost and the commit agree.** They run through
two different call paths — `updatePreview`'s `source.params` and
`placeComponent`'s own `params` — and the free params reached NEITHER before
this WP: `CadClient.placeComponent` had no params argument at all, and the
preview's generator source carried no `params` key, so every ghost previewed
the generator's DEFAULT size. This module has already shipped that exact class
of bug twice (a dropped `rotate`, then a hardcoded `source`), so the pairing
now has its own test that watches both calls from ONE gesture.

**Threaded through, end to end**: `source.params` on the preview op (validated,
never coerced — a malformed map throws rather than being dropped),
`placeComponent(…, params)` on `CadClient` / `CommandApiService` / the tauri
client / the mock, and `place_component(…, params)` in Rust, which validates
them against the component's own signature. That validation moved into a shared
`check_free_params` so the place site and the edit site cannot drift; the place
site needs it because a gesture that authored a non-free key would produce a
record the edit command then refuses to touch. A blob-backed source with params
is refused with a reason — baked geometry has nothing to re-derive.

**Coverage, and one honest gap.** Unit tests for the picker (including the
nothing-fits and non-metric cases), `previewOps` tests for the `source.params`
passthrough and its refusals, Rust tests for place-with-a-free-param and the
non-free refusal, and a new `placementController.test.ts` driving a real
pointermove→pointerdown gesture over stub services to pin ghost/commit
agreement. **The Playwright mock lane cannot cover auto-size**:
`mockClient.classifyElement` derives its answer from mesh metrics and only ever
reports `plane` or `other` — it has no cylinder/circle case and therefore no
radius. Recorded here rather than papered over with a fabricated mock frame;
teaching the mock to classify a cylinder is its own piece of work.

Also removed: `placementController`'s scope-cut comment block, which still
claimed "no auto-size" and "no mate persistence" — the second had been false
since WP-3.1.

GATE: vitest **246 files / 4172** (up from 245/4162) · `bunx tsc --noEmit`
clean · `cargo test --workspace` green (2 new `library.rs` tests) · `cargo
fmt`/`clippy -D warnings` clean · worker ctest **119/119** unchanged (no C++
touched) · hex gate empty · Playwright full sweep run at the phase boundary.

## COMPONENT-LIBRARY WP-A2 (2026-08-13) — GATE PASSED

First-run seeding of the built-in catalog — the other half of "the library is
empty". WP-A1 gave the worker seven generators; nothing on disk pointed at
them. The library root is `<app_data_dir>/library`, empty on a fresh install,
so a real user opened the panel and saw nothing, and spec §12's flagship flow
was unreachable in the shipped app regardless of what the kernel could build.

**Seven `component.toml` packages** (`src-tauri/resources/library-seed/`), one
per registered generator, declaring identity/metadata/`[parameters]`/
`[attachments]` and never a dimension — the generator owns those. Each
family's `thread` domain is its STANDARD's range, not a uniform M2–M12: ISO
7380-1 and ISO 7093-1 start at M3, and the manifests say so rather than
inventing rows. Nuts and washers declare neither `length` nor `thread_detail`,
because they have neither.

**Embedded, not bundled as resources** (`include_str!`): a generator package is
one small text file, and embedding makes seeding behave identically in `tauri
dev`, a packaged bundle, `cargo test`, and CI — none of which resolve a
resource directory the same way. It also removes a packaging step that could
silently ship an app with an empty library. `include_bytes!` extends this the
day a package needs a `preview.webp`.

**The rule the module exists to protect: the user's copy always wins.** Seeding
only ever CREATES a package directory that does not exist — never overwrites,
never merges, never deletes. A `.seed-version` marker records that the pass
ran, so a deleted built-in stays deleted; bumping `SEED_VERSION` re-runs the
pass and restores what is missing. Both halves have their own test.

**Re-indexing after seeding is not optional** and is why the pass lives in
`library.rs` rather than the pure module: `list_library_components` reads the
PERSISTED index and deliberately never rebuilds it (WP-1.6's explicit-action
rule), so a package that seeds without being indexed is still an empty panel.
Runs at app setup on a blocking task, off the window-creation critical path,
degrading to "library stays as found" on any I/O failure.

**The test that could not exist before:**
`every_seeded_component_places_through_the_real_worker` seeds a temp root,
reindexes, and places EVERY indexed component through the real OCCT worker,
asserting one publishable body each. Manifest `source.generator` ids and the
worker's registered ids live in different languages in different directories;
before WP-A1 a mismatch was invisible (every id built a socket cap screw), and
now it is an `OP_FAILED` — this turns that into a test failure instead of a
user-facing one. It also proves the shipped `thread` domains are sizes the
generators accept.

Also pinned: every seed manifest parses, validates its identity, names a
non-empty component, declares at least one attachment (a component with none
can never snap), and carries a `revision` equal to the recomputed content hash
of a manifest-only package — so adding a preview file without updating the
manifest fails here rather than making every placement resolve `NeedsRepair`.

GATE: `cargo test --workspace --no-fail-fast` **1148 passed / 0 failed** (7 new
`library_seed` unit tests + the real-worker catalog sweep) · `cargo fmt --all
--check` + workspace `clippy -D warnings` clean · worker ctest unchanged at
**119/119** (no C++ touched) · frontend untouched.

## COMPONENT-LIBRARY WP-A1 (2026-08-13) — GATE PASSED

Per-family generator dispatch + six more ISO fastener families (spec §6.2's
seed catalog). First WP of the "content gaps" phase the session-16 plan opened
after scoping found the shipped library is **empty**: no seeded packages exist
anywhere in the repo, and the worker's generator lane had no dispatch at all.

**The defect this closes, stated plainly:** `ComponentOp.cpp` read
`source.generatorId`, checked only that it was non-empty, and then built an ISO
4762 socket cap screw **for every value**. Harmless while exactly one family
existed; the instant a second one is seeded it is the silent-substitution
failure spec §0 invariant 4 exists to forbid — ask for a washer, get a screw.
An unregistered id now fails `OP_FAILED` naming the registered ids.

**Worker, restructured (a move, not a rewrite):** the ISO 4762 table, both
thread cutters, and the SHCS builder moved out of `ComponentOp.cpp` into
`ops/ComponentGenerators.{h,cpp}` (builders + registry) and
`ops/FastenerTables.{h,cpp}` (data only, so a table edit can never change HOW a
family is built). The only change to either cutter is a `thread_length_mm`
argument — the threaded run measured from the tip — which fully-threaded
families pass as the whole shank, reproducing the old behavior exactly. ISO
4762's pinned exact-volume ctests are the proof the move changed nothing;
they passed unmodified on the first build.

**Six new families**, each with its own analytic exact-volume ctest:
`iso7380` (button head, dome = sphere ∩ cylinder), `iso4014` / `iso4017` (hex
prism head; the two standards share every geometric column and differ ONLY in
the threaded run — 4014 threads `b` from the tip, 4017 threads all of it, and a
test pins both halves of that), `iso4032` (hex nut, cosmetic bore),
`iso7089` / `iso7093` (normal and large series washers).

**Data provenance, per family, and one real correction.** ISO 4014/4017/4032/
7089 come from BOLTS (`data/hex.blt` `hexbolt2`/`hexscrew2`, `data/nut.blt`
`hexagonnut1`, `data/washer.blt` `plainwasher1`, fetched via `gh api`
2026-08-13). **ISO 7380-1 and ISO 7093-1 are not in BOLTS at all** — checked,
not assumed: `hex_socket.blt` has no button-head class and `washer.blt` carries
7089/7090/7091/7092 only. Those two are hand-transcribed from the standards and
say so at their definition, in both copies, and in `THIRD_PARTY_NOTICES`.
**BOLTS lists ISO 7089 M10 with a 10.0 mm bore; ISO 7089 and DIN 125 A both
publish 10.5, and a 10.0 bore will not pass an M10 bolt** — corrected on both
sides and pinned by a test on each so a future re-import cannot silently undo
it. Every other value is verbatim.

**Rust mirror**: `tables.rs` became a module directory (`tables/iso4762.rs`
verbatim + `tables/fasteners.rs`), keeping WP-2.2's discipline — an
authoring/metadata copy that is never a geometry authority, with per-family
spot-checks against the SOURCE values and a cross-pinning test against the
worker's numbers, so drift in either copy fails loud.

**Table-extremes robustness** follows WP-2.6's precedent rather than
kernelbench (whose case-v2 schema is architecturally fillet-only, established
there): every seeded size of every family builds at cosmetic detail (63 cases),
plus smallest/mid/largest × 3 thread details for the three new threaded
families (27 cases). All pass; no new kernel limit surfaced beyond the M2
turn-density one WP-2.6 already characterized.

**Protocol**: §7.3's `generator` bullet documents the dispatch and the loud
refusal, with a §14 entry. Two STALE §7.3 paragraphs also corrected in the same
entry — `inputs[]` has carried no mate ref since WP-3.1, and the "not yet
re-seated by the worker" paragraph described a build that shipped two WPs ago.
Documentation-only; found while writing the dispatch note.

GATE: worker ctest **119/119** (118 baseline + new `component_generators`, 12
cases / ~200 checks) · `cargo test -p onecad-library` **37/37** (up from 28) ·
`ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` 100% green ·
`cargo fmt --all --check` + workspace `clippy -D warnings` clean · frontend
untouched (no `src/`/`e2e/` file in this WP's diff).

## COMPONENT-LIBRARY P3 WP-3.2 (2026-08-13) — GATE PASSED

Blob-backed component geometry: the `embedded` **and** `document` source kinds
(spec §2.1), plus the `ExportGeometry` verb that bakes what they carry.

**What the scoping pass found, and why this WP is bigger than "add the document
kind":** `ComponentSourceRef` had no `Document` variant at all, and `embedded`
had never shipped either — `resolve.rs` returned `MalformedPackage "lands in
WP-1.2"` and `ComponentOp.cpp` refused every kind but `generator`, despite spec
§10's P1 line ("`embedded` source kind only"). So the spec §12 differentiator —
"reopen with the library folder deleted and still see the part" — had **no
automated proof for anything but a generator**, which re-runs from params and
never needed cached geometry. Both kinds reduce to one mechanism (a baked solid,
content-addressed, cached in the user's own document), so they landed together on
one lane.

**The architecture fork, decided with the user before implementation:** a
`document` package carries `source.onecad` (identity/provenance/future re-edit)
**plus a baked geometry blob**; placement copies the blob and nothing ever
replays a frozen document. The spec-literal alternative (replay `source.onecad`
at place time) was rejected on two hard facts checked in the code, not assumed:
the worker is **one session per process** (`Session.h:3`, `WorkerManager` owns
one child), so a nested replay needs a SECOND worker process; and spec §4
requires a placed component to render with the library deleted, so the geometry
has to live in the document regardless. Baking at authoring puts it there once.
The record shape leaves room for a re-bake lane (`documentSha256` is recorded and
currently unread — deliberately, it is the key that lane needs).

**Reused, not rebuilt** — a component's cached solid is an import source in every
respect that matters, so it rides `io::imports` verbatim: same content-addressed
section, same `ImportWorkspace` materialization + process-global sha→path
registry, same wire-only non-hashed path injection, same worker-side
`read_brep_solids`/`read_xcaf_solids`/`read_step` readers and the same
`brepFormat` version-pin refusal.

**Worker:** new `io/ExportGeometry.{h,cpp}` (§7.8) — the inverse of
`InspectStep`'s conversion lane, baking a body already in the session into a
replay codec at a Rust temp path, echoing `codec`/`format` so no pin is
hardcoded Rust-side. `ComponentOp.cpp::read_source_blob` is the shared arm for
both blob kinds (they differ only in the record's provenance fields), enforcing
**exactly one solid** (spec §9) and falling through to the existing
placement + WP-3.1 mate-reseat tail, so a baked component gets persistent mates
for free.

**Two things found by running it, not by inspection:**
1. `ExportGeometry`'s first cut required a bare `TopAbs_SOLID` and refused the
   very first real body it was pointed at. A published body is **solid-LIKE**
   (`single_solid_policy` admits a compound wrapper, and a fused feature
   routinely produces one) — it now flattens to the contained solids and refuses
   only a body with none. Caught by the real-worker integration test; the
   worker-only ctest would not have, because it never bakes a body a real op
   published.
2. `xbf` face colors are indexed by the BODY's face map, which only coincides
   with the solid's when the body flattens to one. Multi-solid bodies drop
   colors rather than misapply them (the `ModifiedShape` remap
   `ImportOp::scale_solids` does would be needed to do it properly).

**Rust:** `ComponentSourceRef::Document` + `Embedded.brep_format`, with a shared
`blob_ref()` accessor so the two places that must never miss a kind (wire
lowering, save-time refcount) walk ONE thing. `referenced_import_shas` gained
`PlaceComponent`/`DetachComponent` arms — **load-bearing**: without it the baked
blob is dropped at the first save and the document reopens with a component whose
geometry no longer exists. `DocumentRuntime::stage_component_blob` mirrors
`add_import_record`'s "stage before authoring" discipline (and re-verifies the
digest, so a mis-keyed blob never enters the carrier). `library.rs` resolves all
three kinds, stages under the SAME lock as the edit, and refuses a `document`
package with no bake — loudly, naming it — rather than silently replaying it.
`set_component_params` on a baked source is refused with the reason.

**Package format (documented deviation from spec §2.1's comment lines):** both
blob kinds gained `codec` + `format` (a reader cannot know the byte form or the
version pin otherwise), and `document` gained the `geometry` pointer. Spec §2.1
names only `blob` / `file`.

**Frontend — a real defect, fixed:** `placementDraftParams` +
`placeComponentParams` hardcoded a generator source, so arming ANY non-generator
component previewed the generator stub's M6 screw and committed something else.
New `resolveComponentSource` command (library-owned, like list/reindex) resolves
the real source AND stages its bytes at ARM time — the ghost previews through the
same wire path a commit takes, so it must be materialized before the first
preview, not at commit.

GATE: worker ctest **118/118** (117 baseline + `component_blob_source`: export→
place round trip on both codecs, 2-solid refusal, wrong/absent format pin,
unmaterialized blob, unknown codec/kind; two WP-0.2 ctests updated from
"embedded is UNSUPPORTED" to the shipped behavior) · `cargo fmt`/`clippy -D
warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **100% green**
(new: `a_baked_component_survives_save_and_reopen_with_no_library` — the spec §12
claim, automated for the first time; blob-less component fails only its own step;
`referenced_import_shas` pinning; `blob_ref` coverage; validator + camelCase
pins; wire `source.path` injection incl. the empty-path rule; four `library.rs`
command tests) · `bunx tsc --noEmit` clean · vitest **245 files / 4162** ·
Playwright **404/404**.

## COMPONENT-LIBRARY P3 WP-3.1 (2026-08-13) — GATE PASSED

Persistent mate re-seating on regen (spec §5.5) — P3's first WP, chosen
first because it's the highest architectural risk piece in the whole
Component Library effort (de-risk-first, mirroring how this program's own
P0 was a foundation spike). Full research (three dispatched agents:
explore, architecture design, and extensive hand-verification against the
actual code) preceded implementation — the initial plan's "Rust pre-pass
resolves mate before hashing" framing was wrong; the correct design
resolves entirely **worker-side, mid-`ExecutePlan`**, so it sees SAME-TICK
geometry (a plate's hole moving in the SAME regen the mate re-seats
against) for free, with zero hash-fencing changes needed.

**Worker (new files + `ComponentOp.cpp` wiring):**
`ComponentMateSolver.h/.cpp` is a verbatim C++ port of `src/modules/
library/placementSolver.ts` (WP-1.5's interactive-gesture math) —
numerically pinned against that file's own test cases
(`worker/tests/test_component_mate_solver.cpp`, 9 assertions). `resolve_
mate_reseat` (new, in `ComponentOp.cpp`) does cross-body-safe ladder
resolution (VF-M7 discipline: target body ALWAYS read from `mate.target
.primary.bodyId`, never assumed to be the op's own body — mirrors `HoleOp
.cpp::resolve_host_face`'s two-rung order), classifies the resolved face/
edge via a new in-process `session::classify_shape` (`ClassifyElement.{h,
cpp}` refactor — no wire round trip, so it sees this-tick geometry),
solves the candidate seat, and epsilon-gates it (`kMateReseatTranslation
EpsilonMm = 1e-3`, reusing Hole's own `kPointPlaneFence` value/unit
verbatim per spec's "mirrors Hole's 1e-3 mm re-projection gate";
`kMateReseatRotationEpsilonDeg = 1e-2`, a new constant, no existing
precedent — flagged as a judgment call). An unresolvable target pushes a
`NeedsRepair` item and leaves the frozen `placement` untouched — the
component ALWAYS publishes, never drops, per spec's own words.
`worker/tests/test_component_mate_reseat.cpp` (5 assertions): reseat on
move, no-op within epsilon, NeedsRepair-on-vanished-target-still-publishes,
VF-M7 cross-body parity.

**Two real defects found by RUNNING it against the real worker, not by
inspection — the Rust-level integration tests exist specifically because
the worker ctest suite (constructing `OpContext`/`ctx.bodies`/`ctx
.partition` directly) cannot see either one:**

1. **`mate.target` used to ride in the wire `inputs[]`** (shipped in
   WP-1.5, `wire.rs::wire_op_inputs`'s `PlaceComponent` arm). The worker's
   generic `resolve_input_refs` pre-flight treats ANY unresolved `inputs[]`
   entry as blocking — correct for a face/edge an op structurally needs
   (Hole can't drill nowhere), wrong for a mate: an unresolvable target
   must still let the component publish at its frozen `placement`. Before
   the fix, a mate to a deleted body published **zero bodies**, not the
   component at its last-good spot. Fixed: `wire_op_inputs`'s
   `PlaceComponent` arm now returns no input, ever — `mate.target` travels
   only in `params`, resolved entirely by `resolve_mate_reseat`.
   `element_refs_mut`'s manual-repair-rebind surface is UNCHANGED (still
   exposes the mate target) — the two mechanisms are independent, and this
   is now a deliberate, permanent divergence between them (both existing
   wire.rs unit tests asserting the OLD 1:1 shape updated to assert the
   new one, with the reasoning recorded inline).
2. **`merge_outcome` (`PlanExecutor.cpp`) unconditionally downgraded any
   step with non-empty `needs_repair` to `NeedsRepair` status**, discarding
   `body_events` even when the op outcome carried them. Every OTHER op's
   needs_repair path returns BEFORE building geometry (resolve-first,
   build-second), so this was invisible until now — mate is the FIRST op
   that legitimately wants to publish geometry AND flag a repair
   simultaneously. Fixed: a step stays `Ok` when it produced body events,
   even alongside needs_repair evidence; verified safe for every existing
   op (117/117 ctest unchanged; no existing op populates both).

**Rust (regen pipeline):** `PlanStepEvent.mate_placement: Option<Box
<FrozenPlacement>>` (boxed — clippy `large_enum_variant`), parsed in `wire
.rs::parse_mate_placement` (deliberately infallible, same reasoning as
`parse_body_rank_keys`). `Scratch::mate_placement_by_step` buffers it per
step, gated by `last_valid_step` (same cutoff body/element buffering
uses), applied via new `Timeline::set_place_component_placement` (mirrors
`set_record_outputs`'s "derived data, no undo entry" shape) directly on
`session.timeline` inside `RegenExecutor::run` — lands in `self.regen
.timeline` via the existing `self.regen = scratch` swap, no new hook
needed. New `DocumentSession::sync_mate_placements` (mirrors `sync_record_
outputs`) propagates it into the document's OWN authoritative timeline,
matched by `RecordId` (never a raw index carried across the two
timelines — they can differ in length/order across a checkpoint-
accelerated regen). Wired into `document_runtime.rs::commit_snapshot`
right beside `sync_record_outputs`.

**Protocol:** SCHEMA §7.2 gains OPTIONAL `planStep.matePlacement` (purely
additive) AND a CORRECTIVE note on `PlaceComponent`'s `inputs[]` (mate
removed — see defect 1 above), both in one §14 changelog entry (cross-
track, single-repo).

GATE: worker ctest **117/117** (115 baseline + 2 new targets:
`test_component_mate_solver`, 9 numeric-parity assertions;
`test_component_mate_reseat`, 5 integration assertions) · `cargo fmt`/`clippy -D
warnings` clean workspace-wide · `ONECAD_REQUIRE_WORKER=1 cargo test
--workspace --no-fail-fast` **100% green** (two pre-existing wire.rs unit
tests updated to assert the new, correct `PlaceComponent` inputs[] shape;
new `component_ops.rs` integration tests: reseat-on-move through the REAL
worker + `DocumentRuntime` — including a genuine `promote_selection`/
`AcquireElementIds` round trip and a ladder-scoring anchor-placement
lesson recorded inline — and unresolvable-mate-still-publishes, which is
the test that caught defect 1) · frontend gates unaffected (zero frontend
files touched, confirmed via `git status`).

## COMPONENT-LIBRARY WP-2.6 (2026-08-13) — GATE PASSED, P2 CLOSED

Kernelbench cases from table extremes (spec §10) — the last named P2 WP.
**Real scope call made before writing code**: kernelbench's case-v2 schema
(Rust `case_v2.rs`, C++ `Types.h`/`CaseParser.cpp`, JSON Schema) is
architecturally fillet-only — `OperationFamily`/`OperationTypeV2` are
closed single-variant enums (`Fillet` only), `deny_unknown_fields`
throughout, and the selector/validator machinery is edge-blend-topology-
shaped (edge midpoints, adjacent-surface convexity/valence) — none of it
maps onto solid-body fastener placement. A real `OperationFamily::Component`
extension is a genuine multi-part architecture fork (new definition type,
new selector/validator semantics for solid-body-boolean-cut robustness),
not a config addition — confirmed by reading `case_v2.rs`, not guessed.
Checked master (this branch's sibling worktree) for prior art first: its
only new kernelbench commit since divergence (`b7b47e2`) is pure
Advanced-Fillet-roadmap metamorph-variant work, unrelated. User chose the
lighter mechanism: extend `worker/tests/test_component_ops.cpp` into a real
cross-product matrix, same mechanism WP-2.5 already used for its own
few-sizes coverage, now exhaustive.

**New `test_place_component_thread_detail_matrix_across_the_full_seed_range`**:
all 9 thread sizes (M2–M12) × all 3 `thread_detail` values = 27 cases, each
asserting `Ok` + a published body + a finite positive volume. Length is
`4×d1` per size — a proportional synthetic value, not claiming any external
standard, chosen so even the tightest pitch (M2, 0.4mm) clears several
grooves/turns. All 27 pass.

**A real kernel limit, found by the matrix doing its job, not assumed
away**: a dedicated stress case at M2×60mm (~150 turns at pitch 0.4mm) makes
`BRepOffsetAPI_MakePipeShell::Build()` fail — but SAFELY: `IsDone()` false,
no exception, no crash, no hang (the whole 30+-case binary runs in ~12s), no
partial/garbage shape ever reaches `checked_boolean`. `cut_modeled_thread`'s
existing `!pipe.IsDone()` guard (built in WP-2.5) already does exactly the
right thing here — this is the op's safe-refusal contract working, not a
bug. The test asserts that contract (`Status::Failed`/`OP_FAILED`, no body
published), not success. M2×16mm (WP-2.5's own gate case, 40 turns) and
M2×8mm (this matrix, 20 turns) both succeed; M12×100mm (opposite extreme —
coarsest pitch, similarly long shank, ~57 turns) also succeeds — isolating
this as a turn-DENSITY limit, not a pure-length one. The exact M2 boundary
between 16mm (fine) and 60mm (refused) is left uncharacterized — that
precision is exactly what a real kernelbench `OperationFamily::Component`
extension would binary-search and pin; not built this session, see above.

Also added: `simplified` below one pitch (the `n<=0` guard in
`cut_simplified_thread`) returns the cosmetic blank EXACTLY — pinned by
volume, not just a status check.

**Zero Rust/frontend changes** — confirmed via `git status`: only
`worker/tests/test_component_ops.cpp` touched. **P2 is now closed** —
WP-2.1 through WP-2.6 all landed.

**Also this session, before WP-2.6 itself**: merged `master` (sibling
worktree `OneCAD-Tauri`, 3 commits ahead — P3/P4 modeling-correctness work
+ the kernelbench metamorph widening above) into this branch. Two conflicts
(`TODO.md`, `CURRENT_STATE.md`), both simple append-log splices, resolved
by concatenating both sides' new sections — no content dropped. **The two
"pre-existing failures" every prior WP-2.x gate entry reconfirmed
(`sketch_on_face`, `wire_contract`) are GONE post-merge** — master's P2
region-identity fix (`region_identity_version: 2` in both test files' own
fixtures) landed cleanly with no conflict. `cargo test --workspace` is now
genuinely 100% green, not "green except two known failures." Future gate
entries should stop citing them.

GATE: worker `ctest` **115/115** (`component_ops` carries the matrix + 3
extreme-length cases, ~34 new assertions) · Rust/frontend gates unaffected
by WP-2.6 itself (no source touched; the merge's own full gate, run once
just before WP-2.6: `cargo fmt`/`clippy -D warnings` clean · `cargo test
--workspace` **100% green** · `tsc` clean · vitest **245/4159** · Playwright
**404/404**, up from 396 — master's new pattern/mirror specs).

## COMPONENT-LIBRARY WP-2.5 (2026-08-12) — GATE PASSED

Three-level thread detail (spec §6.4) — `thread_detail` free param
(`cosmetic`/`simplified`/`modeled`, default `cosmetic`) on the ISO 4762
generator. Worker+Rust-table WP, like WP-2.1/2.2/2.3: `ComponentParameters
Section`'s dropdown already renders any domain-enum `role: free` param
generically (confirmed by reading the component before writing any frontend
code — zero frontend logic changed, only the mock fixture).

**New `pitch_mm` table column, DIFFERENT provenance than the rest of the ISO
4762 table**: the ISO 261 coarse-pitch series, a public numeric standard —
not BOLTS data, not subject to `THIRD_PARTY_NOTICES`. Added to both
`worker/src/ops/ComponentOp.cpp::iso4762Table()` and the Rust mirror
(`onecad-library::tables::Iso4762Row`), with the existing cross-pinning test
extended to cover it (same drift-guard WP-2.2 established).

**`cosmetic`** is the unchanged WP-2.1 blank (byte-identical when the param
is absent, same rule `thread`/`length` already follow). **`simplified`**
cuts N discrete annular grooves (one per pitch) from the shank: each groove
is a shallow V revolved 360° into a ring solid, and ALL N rings are
accumulated into ONE `TopoDS_Compound` before a SINGLE `checked_boolean`
cut — not N sequential booleans, which would both accumulate tolerance
debris and turn each ring into its own independent failure point.
**`modeled`** cuts a true helical single-start V-thread via a
`BRepOffsetAPI_MakePipeShell` sweep along a helix built on a
`Geom_CylindricalSurface`, FLAT-TRUNCATED (no ISO 68-1 root/crest fillet) —
a deliberate fidelity cut, not a silent shortcut: a rounded root needs a
fillet-on-profile step BEFORE the sweep, stacking a third independently-
fragile OCCT operation for a difference invisible outside extreme
close-up/3D-print slicing.

**Real bug found and fixed while building the helical sweep, not assumed
away**: `BRepBuilderAPI_MakeEdge(Geom2d_Curve, Geom_Surface, first, last)`
builds a p-curve-only edge with no 3D approximation curve —
`BRepOffsetAPI_MakePipeShell::Build()` needs a real 3D curve on its spine, so
without `BRepLib::BuildCurves3d(edge)` the build silently raised
`Standard_NullObject` with an EMPTY message deep inside OCCT (traced by
bisecting the try block with step-by-step debug prints, not by reading docs
first — the failure mode gives no hint by itself). Fixed with one
`BuildCurves3d` call; flagged in a code comment so a future reader doesn't
rediscover this the hard way.

**Explicitly deferred, not silently expanded** (per this program's
narrow-scoping precedent — see WP-2.3/2.4's own entries): the StatusSection
modeled-thread progress producer (spec §7/§10) — no `begin/setProgress/end`
async-task API exists anywhere in this codebase yet, building one is a
platform-level addition orthogonal to thread geometry; and the kernelbench
table-extremes robustness suite (spec §10) — TODO.md already names **WP-2.6**
for exactly this, so this WP's own ctest coverage is correctness-at-a-few-
sizes (cosmetic-match, simplified M6, modeled M6 AND M2 — the seed range's
tightest pitch, 0.4mm — plus an unknown-value failure case), not an
extremes battery.

`component.toml` stays test-fixture-only (Q4, still open project-wide, not
resolved by this WP).

GATE: worker `ctest` **114/114** (component_ops carries 5 new assertions:
cosmetic-explicit-matches-default, simplified removes material, modeled
removes material at M6 and M2, unknown `thread_detail` fails loud) ·
`cargo fmt --all --check` + workspace `clippy -D warnings` clean ·
`cargo test -p onecad-library` **28/28** (existing cross-pin test widened,
not a new test count) · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace
--no-fail-fast` green except the same 2 pre-existing failures every prior
WP-2.x gate reconfirms (`sketch_on_face::a_line_across_the_projected_rect_
yields_two_extrudable_regions`, `wire_contract::nested_inner_disk_parity_
and_reopen_stability`) · `bunx tsc --noEmit` clean · vitest **245 files /
4154 tests**, unchanged from WP-2.4's baseline (fixture-shape-only ripple,
confirmed via `git status`: zero `src/` logic files touched beyond the mock
fixture) · `bunx playwright test` **396/396**, unchanged from WP-2.4's
baseline (0 regressions).

## COMPONENT-LIBRARY start-screen browser (2026-08-12) — GATE PASSED

User-requested follow-up to WP-2.4: the library must be explorable from the
START screen too, not only inside an opened project (the screenshot the
user attached is `StartScreen.tsx`'s sidebar — Recent/Starred/Templates —
with no library entry point at all).

**Backend-feasible with zero Rust changes, verified not assumed**:
`list_library_components`/`reindex_library` (`library.rs`) resolve
`library_root(app)` off `app_data_dir()` only — neither touches
`AppState.runtime`/`DocumentRuntime`, so both already work with no document
open. Confirmed by reading `library.rs` before writing any frontend code,
not by trying it and hoping.

New `StartNavKey` member `"library"` (`StartScreen.tsx`), a fourth
`StartSidebar` nav row (`cube` icon, no new asset), and two new components:
- `src/features/start/StartLibraryPanel.tsx` — list/search/reindex, own
  toolbar, full-width responsive card grid (unlike the editor's narrow
  `LibraryPanel`, this occupies the whole main content area).
- `src/features/start/ComponentDetails.tsx` — a details side-pane for a
  selected card: identity, category/tags, attachments, free params (with
  `domain` shown inline), and an honest "Open a project to place this
  component" line.

**Deliberately NOT the editor's `LibraryPanel` reused with a flag** — that
component's card click ARMS `placementController.ts`'s gesture, which
reaches a live `ViewportEngine`/`DocumentRuntime` that genuinely doesn't
exist on this screen. Faking an arm/drag affordance with nowhere for it to
land would be the same mistake the codebase already rejected once
(Extensions ▸ Browse's "no registry configured" empty state, over dead
Install buttons). `StartLibraryPanel` is READ-ONLY by construction: browse,
search, reindex, inspect — no placement path, no attempt at one. If the
user wants "browse → jump straight into a new project with it armed" next,
that is a real follow-up (needs `newProject()` to resolve before the
editor's module registration can arm anything — a genuine sequencing
problem, not sketched here), not something this change silently attempts.

GATE: `bunx tsc --noEmit` clean · vitest **245 files / 4154 tests** (up from
244/4148 — new `StartLibraryPanel.test.tsx` (5 cases) + one `StartScreen.test.tsx`
case) · `cargo fmt --all --check` + workspace `clippy -D warnings` clean,
unaffected (zero Rust files touched) · manual `playwright-cli` verification
against `?mocklibrary=1` (dev server on :1420): Library nav renders the
fixture card with NO project open, selecting it shows the details pane
including free params + the "open a project" line, matches the screenshots
above. **Full Playwright not re-run** — no e2e spec targets `StartScreen` at
all (start-screen flows are vitest-only in this repo; `library-browse-
place-snap.spec.ts` covers the in-editor placement flow, untouched by this
change), and the prior WP-2.4 full run (396/396) already covers every start-
screen-adjacent spec (`project-import`, `step-import`) with this addition
being purely a new inert-until-clicked nav branch.

## COMPONENT-LIBRARY WP-2.4 (2026-08-12) — GATE PASSED

Configurator UI (param edit → live designation, spec §294) — the first WP
with a real frontend surface since WP-1.7. Scoped to the POST-placement edit
surface (an inspector section for an already-placed instance); pre-placement
sizing (picking a size before dragging from the library card) and the live
3D preview spec §294 also describes are cut, recorded below, not silently
dropped.

**Backend surface widened first, since neither DTO carried what the UI
needs**: `onecad-library::index::IndexEntry` gained `parameters` (full
`ParameterSpec` map — the index previously carried `parameter_keys`, names
only, not `role`) and `designation` (`metadata.designation`, spec §2.1's BOM
template). `dto::LibraryComponentDto` mirrors both, populated in
`library.rs::index_entries_at`. `LibraryComponentDto` dropped its `Eq`
derive — `ParameterSpec` carries `toml::Value` (via `value`/`domain`), which
only implements `PartialEq` (it can hold an f64).

**Real bug found and fixed while reading the file this WP needed to touch
next to anyway**: `tauriClient.ts::placeComponent` never forwarded its
`rotate` argument to the `place_component` Tauri command — the REAL backend
placed every component unrotated regardless of the flip gesture (`A` key),
masked only because the mock lane's `commitPlaceComponent` DOES honor
`rotate`. WP-1.5's own handoff claimed rotate shipped end to end; it shipped
on the mock lane only. Fixed (one line + a param), covered by the existing
`library-browse-place-snap.spec.ts` (mock lane, so it can't itself prove the
real-backend half — no automated coverage of the real Tauri path existed
before or after this fix; flagged, not solved, since proving it needs a
real-worker Playwright lane this repo doesn't have).

**New `CadClient.setComponentParams`** (append-only interface addition),
wired in both clients:
- `tauriClient.ts` — a thin `call(CMD.setComponentParams, ...)`, same shape
  as `placeComponent`/`detachComponent`.
- `mockClient.ts` — a REAL implementation, not a "not yet" stub like
  `detachComponent`'s. `commitPlaceComponent` now writes `featureParams` for
  every placed instance (it bypasses `commitOp`'s generic `wireParamsOf` —
  `PlaceComponent` throws there, it never reaches the generic op-preview
  lane — so this WP is the first place anything stores it), and
  `setComponentParams` re-derives the real role=free check against
  `MOCK_LIBRARY_FIXTURE.parameters`, merges, and re-stores. Documented
  limitation: the mock's synthetic mesh is a fixed demo shape regardless of
  size, so a param edit changes the stored value + the live designation but
  not the rendered geometry — the real worker-backed lane
  (`component_ops.rs`) is where a size change actually resizes the body.

**New `ComponentParametersSection`** (`src/features/library/`), registered
as the library module's own inspector contribution
(`modules/library/register.ts`, new `inspectorSectionIds.ts` mirroring
modeling's), priority 250 (between modeling's History=200 and
Constraints=300). DATA-DRIVEN, same honesty rule `SketchDimensionsSection`/
`HistoryFeatureSection` already follow: `canRender` can only see "a feature
is selected in Model mode" (platform `SelectionRef` carries no `opType` —
ADR-0002's boundary), so the component itself reads `documentStore`'s
`FeatureMeta.opType` and renders nothing, label included, unless it's
`PlaceComponent`. Renders each `role: "free"` key as a domain `<select>` or
a plain numeric `<input>` (min-checked client-side, mirroring the backend's
own check), plus the live designation via `{key}` template substitution
(`formatDesignation`, exported + unit-tested). Commits go through
`client.setComponentParams` — deliberately NOT the generic
`applyEditCommand`/`updateScalarParamsCommand` path `featureValueEdit.ts`
uses for other ops' inline edits, since that generic path would bypass the
role=free enforcement WP-2.3 exists specifically to apply.

**A real spec-example inconsistency found and worked around, not
copied**: spec §2.1's own designation example is `"ISO 4762
M{thread}x{length}"` (literal `M` before the placeholder), but this
codebase's established thread convention (WP-2.1's worker table, WP-2.2's
Rust mirror, both keyed `"M6"`-style) stores the FULL designation as the
`thread` value — pairing the two doubles the `M` (`"MM6"`). Every
designation string this WP authored (`mockClient.ts`'s fixture, the
`library.rs` test fixture) drops the literal `M`: `"ISO 4762
{thread}x{length}"`. No real `component.toml` designation strings exist yet
to conflict with (P3 authoring is unbuilt) — flagged for whoever writes the
real ISO 4762 package content.

**Scope cuts, recorded not discovered late**: no pre-placement configurator
(spec §294's "selecting a card opens the configurator" before a drag) — this
WP covers editing an already-placed instance only, the well-defined slice
that builds directly on WP-2.3. No live 3D preview while editing — would
need a `PreviewOp` ghost session wired into the inspector section, real
extra surface. No `role: table`/`computed` resolution anywhere client-side
(consistent with WP-2.3's own scope note: the worker derives those from its
own table, nothing on the wire needs them) — a designation template
referencing one (`{head_d}`) renders literally, not blank.

GATE: `bunx tsc --noEmit` clean · vitest **244 files / 4148 tests** (up from
243/4142 — new `ComponentParametersSection.test.tsx`, 6 cases, plus the
existing `LibraryPanel.test.tsx` fixture updated for the new required
`parameters` field) · full Playwright **396 passed / 0 failed** (unchanged
from WP-1.7's own number — the rotate fix and fixture widening touch nothing
any other spec depends on) · `cargo fmt --all --check` + workspace `clippy
-D warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace
--no-fail-fast` green except the SAME two pre-existing failures
(`sketch_on_face`, `wire_contract`) · `cargo test -p onecad-library`
unaffected, 28/28 unchanged · `cargo test -p onecad --lib library::` **5/5**
(up from 4 — new `list_carries_parameters_and_designation_for_the_configurator`).

Next: WP-2.5 (three-level thread detail: cosmetic/simplified/modeled) or
WP-2.6 (kernelbench cases from table extremes) — either is independent of
this WP per the plan's dependency graph. Pre-placement configurator + live
3D preview remain open if the user wants spec §294 covered in full.

## COMPONENT-LIBRARY WP-2.3 (2026-08-12) — GATE PASSED

`SetComponentParams` command (`src-tauri/src/library.rs`) + role enforcement
— the first WP that makes any of P2's table-driven sizing reachable from a
live gesture (once WP-2.4 wires a caller). New `set_component_params_at`
(private, `*_at`-split per the module's own convention) + public
`#[tauri::command] set_component_params`, registered in `lib.rs`'s
`invoke_handler`.

**Enforcement split, as flagged by `validate_place_component`'s own doc
comment**: `onecad-core` checks structure only (`PlaceComponentParams::validate`)
because it cannot depend on `onecad-library` to resolve a component's actual
`[parameters]` signature. This WP is the app-crate half — a new
`component_package_at` helper loads the full `component.toml` (not just the
index's `parameter_keys` names) by walking `IndexEntry.path`, and every
requested key is checked against `ParameterRole::Free` before anything is
merged. An unknown key or a `role: table`/`computed` key is rejected loud
(`InvalidCommand`, naming the key and the component) — never silently
dropped or silently applied.

**`source.params` mirrors the merged free-param map only, not a fully
resolved signature — a deliberate scope cut, not an oversight.** The worker
(WP-2.1) reads `role: free` keys by name (`thread`/`length`) and derives
`role: table`/`computed` values itself from its own table; shipping those
into `source.params` here would duplicate data nothing on the wire reads.
`component.toml`'s `[parameters]` table stays the single resolution
authority, authoring-side only.

Read path reuses `DocumentRuntime::operation_params` + a direct
`serde_json::from_value::<PlaceComponentParams>` (not `detach_component_at`'s
field-by-field `.get("source")`/`.get("placement")` — this WP needs every
field to reconstruct a valid record). A record that isn't a placed component
(e.g. already `DetachComponent`) fails that deserialize the same way a
missing `componentId` would — no separate "is this a PlaceComponent" check
needed. `Embedded`-source placements are rejected outright (spec's embedded
source carries no `params` field at all — there is nothing to merge into).

GATE: 2 new tests in `library.rs`'s `#[cfg(test)] mod tests`
(`set_component_params_merges_a_free_override_and_reaches_source_params`,
`set_component_params_rejects_a_non_free_key`, the latter covering both the
role-mismatch and unknown-key branches) — both green. `cargo fmt --all
--check` + workspace `clippy -D warnings` clean. `ONECAD_REQUIRE_WORKER=1
cargo test --workspace --no-fail-fast` green except the SAME two
pre-existing failures (`sketch_on_face`, `wire_contract`) reconfirmed
failing identically this session. `cargo test -p onecad-library` unaffected
(28/28, unchanged — no `onecad-library` crate files touched, only the
app-crate bridge). Frontend/e2e untouched, confirmed via `git status` (only
`src-tauri/src/library.rs` + `src-tauri/src/lib.rs` in this WP's diff).

Next: WP-2.4 (configurator UI — param edit → live designation). This is the
first WP with a real frontend surface since WP-1.7; will need a
`CadClient.setComponentParams` method + both client impls (mock/tauri) per
the append-only interface rule.

## COMPONENT-LIBRARY WP-2.2 (2026-08-12) — GATE PASSED

`onecad-library::tables` real content — P2's Rust-side metadata mirror of
WP-2.1's worker table. New `Iso4762Table`/`Iso4762Row` (`tables.rs`, full
BOLTS column set `d1/d2/b/k/s/t_min/l`, M2–M12), a spot-check harness (M3/
M6/M12 against the BOLTS source, plus a test pinning every seed size's
geometry-relevant fields — `d1`/`d2`/`k` — against the worker's own table so
the two independently-typed copies can't silently diverge), and a new
`THIRD_PARTY_NOTICES` at repo root crediting BOLTS / Johannes Reinhardt,
LGPL 2.1+, with source URL and retrieval date (spec §6.3). Checked first:
no existing project-authored notices file anywhere in the repo (`worker/
third_party/{nlohmann,planegcs}` vendor code directly with none; only
`node_modules/**` has any) — root is the standard convention.

**Deliberate duplication, not a shared source**: this crate's table serves
`component.toml`'s `[parameters] role="table"` resolution and authoring/
metadata (designation strings), never geometry — the worker's own copy
(WP-2.1) is the sole geometry authority, per spec §6's "generators are
built-in and versioned [in the worker]" framing. A future change to either
copy without the other now fails the cross-pinning test loud.

GATE: `cargo test -p onecad-library` **28/28** (up from 22) · `cargo fmt
--all --check` + workspace `clippy -D warnings` clean · full workspace
`cargo test --workspace --no-fail-fast` green except the SAME two
pre-existing failures (`sketch_on_face`, `wire_contract`) · frontend
untouched, confirmed via `git status` (no `src/`/`e2e/` files in this WP's
diff).

Next: WP-2.3 (SetComponentParams command + role enforcement — the first WP
that makes any of P2's table-driven sizing reachable from a live gesture).

## COMPONENT-LIBRARY WP-2.1 (2026-08-12) — GATE PASSED

Table-driven ISO 4762 generator (worker C++) — P2's first slice, kicked off
per plan `~/.claude/plans/resume-implementation-of-component-twinkling-glade.md`.
`ComponentOp.cpp`'s hardcoded M6×20 constants (`kHeadDiameter` etc.) are
replaced by `iso4762Table()`, a BOLTS-seeded `std::map<std::string,
Iso4762Size>` keyed by thread designation (M2–M12, spec §6.2's seed range).
`resolve_source_and_publish` now reads `source.params.thread` (default
`"M6"`) and `source.params.length` (default `20.0`) — chosen so every
EXISTING caller, none of which send these fields yet
(`placementController.ts`'s `placementDraftParams` only sends `translate`/
`rotate` + `generatorId`/`generatorVersion`), stays byte-identical to P0/P1
behavior. An unknown thread designation fails loud (`OP_FAILED`, lists the
known sizes) — never a silent M6 substitution, the founding invariant (spec
§0#4) applied to the new lookup path. A non-positive `length` fails loud
too.

**Data provenance**: fetched the real BOLTS project
(`github.com/boltsparts/BOLTS_archive`, `data/hex_socket.blt`, class
`hexsocketheadcap` = ISO 4762/DIN 912) via `gh api`, not guessed or
hand-derived. Per-file header confirms **LGPL 2.1+**, author Johannes
Reinhardt — the repo's GitHub API `license` field reports GPL-3.0 (a
whole-repo default), verified to be a red herring at the file level, which
carries its own LGPL 2.1+ header (matches spec §6.3's "LGPL 2.1+, per-part
license tracking" claim once checked, not just assumed). Full M1.4–M64
table retrieved; scoped implementation to spec §6.2's stated M2–M12 —
M1.4/M1.8 carry `None` for `t_min`/`b` at the source (undersized for a
practical hex-socket detail), not worth engineering around outside the
spec's own range.

**Real deviation from the top-level plan doc's P2 sketch, found before
writing code, not guessed past**: the plan's item 2 ("SetComponentParams
C++ dispatch lands, plumbing already exists from WP-1.2") assumed
`SetComponentParams` is a distinct wire op. It is not — WP-1.2 already
recorded that `SetComponentParams`/`ReplaceComponent` are in-place edits of
the existing `PlaceComponentParams` record via the generic
`EditCommand::UpdateOperationParams`, same as Hole's profile-mode edits.
There is no new C++ dispatch arm to add for param edits — regen of an
edited `PlaceComponent` record already reaches this WP's table lookup
automatically. What's still missing (WP-2.3) is a Rust-side authoring
command enforcing role=free before constructing the edit.

GATE: `worker/tests/test_component_ops.cpp` extended with M6-explicit
(matches the default path exactly), M2, M12 exact-volume cases, an
unknown-thread failure case, and a non-positive-length failure case — **12
assertions, 0 failures** (`ctest -R component_ops`). Full worker ctest
**114/114**. `bunx tsc`/vitest/Playwright not re-run — zero frontend/e2e
files touched this WP (confirmed via `git status`, not assumed).
`ONECAD_REQUIRE_WORKER=1 cargo test --workspace` unaffected (WP-2.2's gate
entry below covers the combined Rust run).

Next: WP-2.2 (this session, immediately following).

## COMPONENT-LIBRARY WP-1.7 (2026-08-12) — GATE PASSED, P1 CLOSED

`e2e/library-browse-place-snap.spec.ts` — WP-1.5's manual `playwright-cli`
verification (armed card → hover → correctly-oriented ghost → click commits
→ tree updates → Escape cancels), converted into a repeatable Playwright
gate. Two specs: commit flow (browse → arm → hover-snap ghost → commit → one
new named body in the projection) and cancel flow (arm → hover-ghost →
Escape → armed state clears, ghost clears, body count unchanged).

**Scope finding, made while writing this (not assumed from the plan doc):**
the original plan's WP-1.7 wording ("save, close, delete library root,
reopen, assert body present") is **not provable in the Playwright MOCK
lane** — `mockClient.newDocument()`/`openDocument()` fabricate a fresh
synthetic document on every call; there is no in-memory or on-disk
persistence for a `page.reload()` to round-trip through. A "reopen" step
there would either hang (nothing to reopen) or silently prove nothing,
which this codebase's own philosophy treats as worse than an honest gap.
That invariant is a REAL-worker/Rust concern and **is** covered, for the
generator-source case P1 ships:
`src-tauri/tests/component_ops.rs::place_component_survives_save_and_a_fresh_worker_reopen`
(save → shut worker down → spawn a FRESH worker process → reopen → assert
identical geometry). What the new e2e spec owns instead is the frontend
wiring chain the kernel test never sees: card → armed state → hover →
classify → ghost → commit → tree → Escape — matching `hole.spec.ts`'s own
stated split of responsibility ("what THIS spec owns is the chain the
kernel never sees").

**Residual flagged, not fixed here (out of WP-1.6/1.7 scope):** the
EMBEDDED-source variant of "reopen without the library folder" — a cached
BLOB surviving a deleted library root, spec §12's actual differentiator
claim — has no automated test anywhere yet. WP-1.3's own gate entry above
shipped `place_component` as "generator source only (matches the worker's
WP-1.2 scope)"; the embedded-blob authoring-time copy-in path the original
plan's WP-1.3 section described was never built. Whoever picks up P2/P3
should either build that path or narrow spec §12's claim to match reality.

`findFaceOnBody` (existing helper) supplies the hover pixel — the mock
fixture's `headSeat` attachment accepts `["plane"]`, so any planar face on
the `vpdemo` box matches; ghost presence is polled via
`window.__vpEngine.previewBodies.size` (the same `?vpdebug` surface
`findFaceOnBody` itself already relies on for raycasting).

GATE: new spec **4/4** (chromium + webkit) · full Playwright suite
**396/396** (up from 392 pre-existing — the new spec's 2 tests × 2 browsers),
**zero failures**, `retries: 0`. `bunx tsc --noEmit` clean · vitest
**243/4142** (unchanged from WP-1.6, no new frontend test file) ·
`cargo fmt --all --check` + workspace `clippy -D warnings` clean ·
`ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` — every
target green except the SAME two pre-existing failures bisected to baseline
in earlier sessions (`sketch_on_face::a_line_across_the_projected_rect_...`,
`wire_contract::nested_inner_disk_parity_and_reopen_stability`, both `fetch
body mesh` panics, unrelated to Component Library) · `cargo test -p
onecad-library` **22/22**, unchanged · worker binary untouched, no rebuild
needed (WP-1.6/1.7 are frontend/e2e-only, zero C++/Rust changes).

Next: P1 is closed to spec §10's gate language (kernel CTest + Rust tests +
a Playwright spec for browse→place→snap→save→reopen, with the save/reopen
half's actual coverage boundary now documented above). P2 (parametric
fasteners) is next, with its own open question (BOLTS ingestion tooling
ownership) — needs a follow-up plan, not started here.

## COMPONENT-LIBRARY WP-1.6 (2026-08-12) — GATE PASSED

`Slots.StatusSection`'s first real producer — closes a platform debt named
at the MODULAR-PLATFORM wave (the tasks chip has had a `begin/setProgress/
end` API with zero producers since it landed).

- `src/features/shell/StatusBar.tsx`: added `<SlotHost slot={Slots.
  StatusSection}/>` beside the existing hardcoded `<TasksChip/>`.
- `src/modules/library/register.ts` (`contributeLibraryUi`): registers a
  new `LibraryStatusSection` panel at `Slots.StatusSection` — a static
  "N components" count, fetched once on mount via `listLibraryComponents()`.
  Deliberately minimal, per the plan's own instruction ("don't invent UI
  just to fill the slot") — no shared store with `LibraryPanel`, nothing
  live-tracks its search/filter state.
- `src/features/library/LibraryPanel.tsx` (`reindex`): wraps the existing
  `reindexLibrary()` call with `tasksStore.getState().begin("library.
  reindex", …)`/`.end(...)`. `ReindexReport` (`{total, indexed, skipped}`)
  is a single atomic response with no incremental count, so begin→end with
  no `setProgress` in between is the honest shape — the same "progress is
  optional" rule `tasksStore`'s own doc comment states. Local `reindexing`
  `useState` kept alongside (drives the button's own "…" label/disabled —
  a different UI signal than the app-wide chip, not a duplicate source of
  truth for the same thing).
- **`editorMountOrder.golden.test.ts` amended, recorded**: `Slots.
  StatusSection` is a genuine exception to "every rendered slot is an
  `EDITOR_REGIONS` entry" — it nests INSIDE `StatusBar`'s own `<SlotHost/>`,
  and `StatusBar` itself IS the `Slots.ShellBottom` panel. The mount-order
  CONTRACT (`EDITOR_MOUNT_ORDER_CONTRACT`) is untouched — the top-level scan
  never saw `StatusSection` before and still doesn't; only the probe's
  "every panel lands somewhere rendered" completeness check needed to learn
  about the new nested slot, per the contracts README's "probe may change,
  contract may not" rule.

GATE: `bunx tsc --noEmit` clean · vitest **243 files / 4142 tests** (up from
243/4141 at WP-1.5 — one new tasks-chip begin→end transition test in
`LibraryPanel.test.tsx`, no new test file) · `cargo fmt --all --check` +
workspace `clippy -D warnings` clean (Rust untouched by this WP — no Rust
files changed).

**Real bug caught mid-implementation:** `StatusBar.test.tsx` rendered
`<StatusBar/>` bare in all 9 of its `render()` calls; adding `<SlotHost/>`
made every one of them throw (`usePlatform()` outside a `<PlatformProvider>`)
since `SlotHost` needs a platform context ancestor. Fixed with a
`renderStatusBar()` helper wrapping in `<PlatformProvider platform=
{createPlatform()}>` — the same minimal-fixture shape `reference.test.tsx`
already uses — not a StatusBar defect, a test-fixture gap the new
contribution surfaced.

Next: WP-1.7 (e2e spec for the WP-1.5 flow).

## COMPONENT-LIBRARY WP-1.5 (2026-08-12) — GATE PASSED

The snap solver + interactive placement gesture (spec §5.1-§5.4): classify →
attach → concentric/flush + flip; drag ghost via the real `PreviewOp` lane.
The largest remaining P1 WP and the one WP-0.1's `ClassifyElement` latency
spike de-risked.

**Real architectural gap found and resolved before writing the gesture, not
guessed past**: `ViewportEngine.configurePicking` is a single hardwired pick
seat, owned by `ViewportRoot.tsx` and wired to modeling's selection store —
no existing seam let a second module take hover/click during an armed
placement. Presented the fork to the user (reuse the existing pick primitive
vs. a new engine-level exclusive-gesture capability vs. dropping live drag);
chose the former. Resolution: `src/modules/library/placementController.ts`
is a module-level singleton (like `ModelToolController`, but library-owned
and independent of it) that adds its own `window` `pointermove`/`pointerdown`/
`keydown` listeners in CAPTURE phase only while armed —
`stopPropagation`/`preventDefault` shuts out orbit/select without touching
`ViewportRoot.tsx` or any platform contract, `engine.setOrbitSuppressed(true)`
is the belt to that suspenders. Hover reuses `ViewportEngine.probePick` (the
SAME one-shot primitive the existing dblclick-to-sketch handler already
calls) rather than the continuous `configurePicking` feed. The ghost mesh
renders through `engine.setPreviewBody`/`clearPreviewBody` — already generic,
already how `ModelToolController` renders every other tool's L2 preview body,
so no platform surface had to change at all.

**Scope cut vs. the full spec, deliberate and recorded (not discovered
late)**:
- **No free-space ghost follow.** The ghost appears only once hovering a
  target whose classification matches one of the component's attachments.
  Spec step 6's "drop in free space, position later with Move" fallback path
  is not wired — there is no Move-tool integration point yet for library.
- **No auto-size.** P1 has exactly one seeded generator (the hardcoded M6
  SHCS) with no size table to pick a nearest-smaller size FROM — auto-size is
  P2 scope (spec §6) by construction, not a WP-1.5 omission.
- **No `mate` persistence (spec §5.5).** Checked the worker first:
  `ComponentOp.cpp`'s `resolve_source_and_publish` reads
  `placement.{translate,rotate}` only and has no `mate` handling at all.
  Recording a `mate` now — even a well-formed one — would be inert data
  masquerading as a real feature, worse than omitting it. The computed snap
  transform is written into `placement` directly; `CommandApiService`/
  `CadClient.placeComponent` widened to carry `rotate` (previously
  translate-only, WP-1.3's own doc comment anticipated this), still no
  `mate` parameter. Re-seat-on-regen is P3, unchanged from the plan.

**The ghost preview is the REAL PreviewOp lane, both clients, not a second
mapper** (spec §5.1 forbids one). `ipc/previewOps.ts` gained a
`placeComponentOp` builder and `OpType`/`OperationOp`/`PreviewParams` gained a
`PlaceComponent` arm — same mapper Extrude/Fillet/etc. already share. The
REAL/tauri lane needs zero C++ changes: `execute_place_component` already
handles it (WP-1.2). The MOCK lane's local-fallback branch in
`localSolver.ts::updatePreview` synthesizes the SAME fixed M6 SHCS mesh
(`mockMeshes.ts::placeComponentGhostMesh`, reusing the existing
`placementMatrix`/`transformMesh1`/`concatMesh1` trio TransformBody's mock
already established) — a rigid placement of a KNOWN shape needs no live
document data, unlike Extrude's profile-dependent prism.

**Commit stays on the dedicated `placeComponent` command, not
`endPreview(commit)`.** The generic preview-commit path would skip
`Library::resolve_source`'s revision re-verification (library.rs's own doc
comment: "never authoring a record with a lie in it") — the ghost session is
ALWAYS cancelled (`endPreview(sessionId, false)`), the real commit is a
separate `CommandApiService.placeComponent` call. `CommandApiService` also
gained the generic `beginPreview`/`updatePreview`/`endPreview`/
`onPreviewResult` pass-through (ADR-0002: the kernel touch routes through
modeling's published services, and a further kernel-touching component
operation arriving is exactly the widening `CommandApiService`'s own P1.3
doc comment anticipated).

**`onecad-library`'s `IndexEntry` widened**: `generator_id`/
`generator_version` (generator-source only) and the `[attachments]` table
verbatim, threaded through `LibraryComponentDto`/`LibraryComponent` — the
snap solver's accepts-matching input and the ghost draft's identity fields.
`AttachmentSpec` gained `Eq` (needed for `LibraryComponentDto`'s existing
derive).

**Candidate-transform math is a separate pure module**
(`src/modules/library/placementSolver.ts`, 16 unit tests, no viewport/DOM
dependency): component-local convention mirrors the worker's hardcoded stub
exactly (origin at the seating plane, head +Z, shank -Z) — `coincident`
(plane) aligns local +Z to the target's outward normal, seats at the pick
point; `concentric` (cylindrical face) aligns to the axis, seats by
projecting the pick onto the axis line (an honest "seated under the cursor"
approximation — `ClassifyFrame` carries no face bounds, so a true
nearest-END solve isn't possible); `concentricAndCoincident` (circular edge)
seats exactly at the frame origin. Flip negates the alignment direction.
Tab cycles among the component's attachments that still match the current
hover's classification, preserving the user's choice across hovers when it
stays valid.

**Real bug caught mid-implementation**: the golden mount-order test
(`editorMountOrder.golden.test.ts`) calls `contributeModelingUi`, not
`contributeModeling` (the bootstrap function that registers
`ModelingServices.GeometryQuery`/`CommandApi`) — so `services.require(...)`
inside `contributeLibraryUi` threw in that harness even though the real app
boot order (bootstrap before editor mount) never hits it. Fixed with a SOFT
lookup (`services.get`) — a missing service leaves placement quietly
unarmed (`configurePlacementController(null)`) instead of failing editor
mount, which is also the more defensive real-world choice regardless of this
test.

Manual `playwright-cli` verification against `/?vpdebug&vpdemo&mocklibrary=1`
(the mock lane's honest-empty-by-default library gains a `?mocklibrary=1`
opt-in fixture, same dev-only URL-flag pattern as `?mockimport=step` —
`listLibraryComponents()` stays `[]` by default, WP-1.4's "no fake catalog"
rule unchanged): card click arms placement (ring highlight, status hint) →
hovering the demo body's top face produces a correctly-oriented ghost
(screenshotted both flip states — head-up/shank-down default, head-buried/
shank-up flipped, both geometrically correct) → click commits → body appears
in the model tree named after the component, `bodiesChildren` 1→2 → Escape
cancels cleanly from an armed-but-uncommitted state. Zero console errors
beyond the pre-existing favicon 404.

GATE: `bunx tsc --noEmit` clean · vitest **243 files / 4141 tests** (up from
242/4138) · `cargo fmt --all --check` + workspace `clippy -D warnings` clean
· `ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` — every
target green except the SAME two pre-existing failures already bisected to
baseline in this session's CURRENT_STATE snapshot
(`sketch_on_face::a_line_across_the_projected_rect_yields_two_extrudable_regions`,
`wire_contract::nested_inner_disk_parity_and_reopen_stability`, both `fetch
body mesh` panics, unrelated to Component Library) · ctest **114/114**
(worker untouched this WP — zero C++ changes, `execute_place_component`
already handled the ghost) · manual `playwright-cli` pass as above.

Next: WP-1.6 (StatusSection + tasks-chip real producer — `reindexLibrary`
becomes the tasks-chip's first real producer), then WP-1.7 (e2e:
browse→place→snap→save→reopen-without-library).

## COMPONENT-LIBRARY WP-1.4 (2026-08-12) — GATE PASSED

`onecad.library` UI module scaffold + the library panel (spec §7 "The
Library panel"). Browse + reindex only — drag-to-place and the snap solver
are WP-1.5.

- [x] `src/modules/library/` mirrors `modules/modeling/`'s shape exactly:
  `manifest.ts` (`moduleId("onecad.library")`, schema v1), `panelIds.ts`
  (const-map, mirrors `modeling/panelIds.ts`'s split-for-code-splitting
  reasoning), `register.ts` (`contributeLibrary` bootstrap hook — empty in
  P1, no tools/commands yet — + `contributeLibraryUi` editor-mount panel
  registration). `EditorShell.tsx`'s `useEditorContributions` gained a third
  `createScope`/`contributeLibraryUi`/dispose call, alongside modeling and
  shell.
- [x] **Real layout conflict found and resolved, not assumed away**: the plan
  flagged "check how the model tree already occupies ShellLeft before
  assuming two panels coexist trivially" — confirmed by reading
  `ModelTreePanel.tsx` directly: it's `absolute left-0 top-0 w-[220px]`,
  fully occupying the slot. `SlotHost` mounts every contribution in a slot
  simultaneously (no exclusivity mechanism), so a second `Slots.ShellLeft`
  panel would overlap it pixel-for-pixel. **Resolved with a VS Code-style
  shared tab strip, not a platform/slot change**: a new tiny
  `sidebarTabStore` (`"model" | "library"`) + `SidebarTabHeader` component,
  rendered by BOTH panels at their own top; each panel reads the store and
  returns `null` when it isn't the active tab. Zero platform contract
  changes — `ModelTreePanel` gained exactly two lines (the store read + the
  early return) plus the shared header; `LibraryPanel` is a normal new
  contribution. Considered and rejected: a new shared tab-CONTAINER
  contribution (would require modeling's existing, pinned `ShellLeft`
  registration to change) and inventing per-workspace panel exclusivity in
  the platform (a bigger, unrelated architecture change).
- [x] `src/features/library/LibraryPanel.tsx`: search (client-side filter on
  name/id/category/tags), Reindex button, card grid backed by
  `CadClient.listLibraryComponents()`/`reindexLibrary()`. Honest states:
  loading, read-error, "no components indexed yet" (not a fake catalog), and
  "no matches for {query}". Selecting a card is inert — nothing to wire it to
  before WP-1.5's placement gesture exists.
- [x] `src/test/contracts/shellContract.ts` AMENDED (recorded here, per its
  own README rule): `LibraryPanel` joins the frozen mount-order list right
  after `ModelTreePanel`. `editorMountOrder.golden.test.ts` extended to
  register the library scope too.
- [x] **Visually verified in a real browser** (`bun run dev` + `playwright-cli`
  against `/?vpdebug&vpdemo`, the mock-lane editor boot with a seeded body):
  screenshotted both tab states — Model tree showing Body/Sketches/Datums
  with the Library tab inert, then Library showing the empty state with
  search+Reindex, then back to Model with full sketch/inspector state
  intact. Zero console errors beyond the pre-existing favicon 404. Round
  trip (Model→Library→Model) confirmed no state loss.
- [x] 6 new tests (`LibraryPanel.test.tsx`): tab-gated rendering, empty
  state, error state, card list, search filter, reindex-reloads.

Gate: `bunx tsc --noEmit` clean · vitest **242 files / 4124 tests** (up from
241/4118) · manual browser verification via `playwright-cli` (screenshots
taken, reviewed, no regressions) · `bun run build` not re-run this WP
(no bundling-relevant change beyond new modules already covered by tsc+vitest).

Next: WP-1.5 (snap solver: classify → attach → concentric/flush + flip +
auto-size; `PreviewOp` drag ghost) per the plan — the largest remaining P1
WP, and the one WP-0.1's `ClassifyElement` latency spike (p95=0.16ms, GO)
was de-risking for.

## COMPONENT-LIBRARY WP-1.3 (2026-08-12) — GATE PASSED

Bridges `onecad-library` into the app crate. New `src-tauri/src/library.rs`:
`list_library_components`/`reindex_library`/`place_component`/`detach_component`
Tauri commands, plus the codebase's first real `ModelingServices.CommandApi`
registration (closes the WP-0.1-flagged gap's second half — `GeometryQuery`
closed the first).

- [x] Library root: `ONECAD_LIBRARY_ROOT` env override, else
  `<app_data_dir>/library` — mirrors the established `ONECAD_WORKER_PATH`
  precedent exactly (a real dev/test seam, not test-only plumbing).
- [x] `place_component`: resolves `{id, version}` via `Library::resolve_source`
  (revision-verified), builds `PlaceComponentParams` (generator source only —
  matches the worker's WP-1.2 scope), applies via
  `EditCommand::AddOperation`. `detach_component`: reads the target record's
  CURRENT `source`/`placement` via `rt.operation_params` (the same read path
  `get_operation_params` uses), re-applies them on `DetachComponentParams` via
  `EditCommand::UpdateOperationParams` — the sanctioned in-place swap WP-1.2
  taught `edit::session::op_type_edit_allowed`.
- [x] **`*_at` split discovered mid-implementation, not planned upfront**:
  `place_component`/`detach_component` take `AppHandle` (for `app.emit(...)`
  and `app.path()`), but `AppHandle` is pinned to the concrete `Wry` runtime
  in a compiled `#[tauri::command]` fn — `tauri::test::mock_app`'s
  `MockRuntime` cannot satisfy it (confirmed by a real compile error, not
  assumed). `tauri::State<'r, T>` is NOT runtime-generic, so it works fine.
  Fix: split every command into an `AppHandle`-free `*_at(root, &State, ...)`
  core (testable) plus a thin public wrapper that resolves `root`, calls the
  core, then does the `AppHandle`-dependent `finish()` tail (emit + scheduler
  + mutation-tick) separately — mirrors `crate::recents`'s OWN `*_at` split,
  just for a different reason (that one is untestable-AppHandle-path-resolution;
  this one is untestable-AppHandle-type-mismatch AND wanting event-emission
  out of the tested critical path).
- [x] **Tests moved in-crate, not `tests/*.rs`**: the same `AppHandle`
  mismatch meant an external integration test file couldn't call the private
  `*_at` cores (external tests only see `pub` items). Tests live in
  `library.rs`'s own `#[cfg(test)] mod tests`, using `tauri::test::mock_app`
  for a real `State<AppState>` exactly like `sketch_on_face.rs` does.
- [x] **No real worker needed for these tests, deliberately**: `place_component_at`/
  `detach_component_at` only exercise `DocumentSession::apply` (pure timeline
  mutation) — regen never runs. `AppState::new` is built with an explicit
  ALL-`PendingBackend` factory rather than relying on "no worker binary
  happened to resolve", so the test is deterministic regardless of the
  environment's `ONECAD_WORKER_PATH`. Real-worker proof that the op
  ACTUALLY publishes correct geometry stays `component_ops.rs`'s job.
- [x] 2 new DTOs (`LibraryComponentDto`, `ReindexReportDto`) in `dto.rs`;
  `ApiError: From<onecad_library::LibraryError>` (uniformly `InvalidCommand`
  — every `LibraryError` variant is a typed, per-entity, recoverable failure).
- [x] Frontend: `CadClient.listLibraryComponents`/`reindexLibrary`/
  `placeComponent`/`detachComponent`, both `tauriClient`/`mockClient` impls.
  Mock lane returns an HONEST empty catalog + throws on place/detach (spec
  precedent: "no registry configured" beats a fake catalog that outlives the
  mock and teaches a UI bug to pass e2e) — real mock-lane behavior arrives
  with WP-1.7's e2e lane, not invented here.
- [x] `ModelingServices.CommandApi` registered in
  `modules/modeling/register.ts` with `{placeComponent, detachComponent}`
  ONLY — per Open Question 6's resolution: these two are the only
  KERNEL-TOUCHING library operations (ADR-0002), so only they route through
  modeling's service; `listLibraryComponents`/`reindexLibrary` are pure reads
  library UI will call directly via `CadClient`, no service indirection.

Gate: `cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D
warnings` clean · `cargo test -p onecad --lib library::` **2/2** (list/reindex
round trip; place→detach through the real command core, asserting the
record's `opType` actually flips) · full workspace `cargo test --workspace`
against the real worker still green (same two pre-existing failures,
untouched) · `bunx tsc --noEmit` clean · vitest **241 files / 4118 tests** ·
`bun run build` clean.

Next: WP-1.4 (`onecad.library` UI module scaffold + library panel on
`Slots.ShellLeft`) per the plan.

## COMPONENT-LIBRARY WP-1.2 (2026-08-12) — GATE PASSED

Matures the op family to its P1 shape: `ComponentSourceRef::Embedded` added,
`KnownOperation::DetachComponent` landed. SCHEMA §7.3 doc block + §14
changelog entry (owed from WP-0.2, now caught up).

**Design correction from the original plan, made and recorded here rather
than silently guessed through**: the plan (written before implementation
started) assumed `SetComponentParams` and `ReplaceComponent` would ALSO be
distinct `KnownOperation` variants ("C++ falls through to `UNSUPPORTED`" only
makes sense if they're real `opType` tags). Implementing `DetachComponent`
first exposed why that's wrong: `Hole`'s counterbore/countersink switch — the
actual precedent spec §3.3 cites — stays `KnownOperation::Hole` the whole
time; only the FIELD VALUES change via `update_operation_params`. Applying
that same logic: `SetComponentParams` (edit `.params`) and `ReplaceComponent`
(edit `.componentId`/`.version`/`.revision`/`.source`) are BOTH just
`update_operation_params` calls that keep `KnownOperation::PlaceComponent`
and overwrite fields — no new variant, no new wire opType, no C++ dispatch
line ever needed for either. Only `DetachComponent` earns a real variant,
because it's the one case where the record's shape GENUINELY changes (drops
`component_id`/`version`/`revision`/`mate` entirely — "no `component_*`
fields remain"). This cuts WP-1.2's actual scope roughly in half versus the
original plan and removes an entire (wrong) design branch before any C++ was
written against it. `SetComponentParams`/`ReplaceComponent` are now purely
app-crate/command-layer concerns (WP-1.3+), not core-type work.

- [x] `ComponentSourceRef::Embedded { sha256, codec: ImportSourceCodec }`
  added (reuses `ImportSourceCodec` verbatim). Widens the op's source enum to
  the full spec §2.1 3-kind shape minus `Document` (still P3).
- [x] `KnownOperation::DetachComponent(DetachComponentParams)` —
  `{ source: ComponentSourceRef, placement: FrozenPlacement }`, NO
  `component_id`/`version`/`revision`/`mate`. All 5 Rust mirror sites +
  `validate_detach_component` (wired into both `add_operation` and
  `update_operation_params`) landed in one pass, having already learned the
  full mirror-site list from WP-0.2.
- [x] **New sanctioned op-type swap** in `edit::session::op_type_edit_allowed`:
  `PlaceComponent → DetachComponent`, one-directional (mirrors the existing
  Fillet⇄Chamfer precedent; the reverse is deliberately NOT sanctioned — "the
  honest break link" is one-way). 3 new integration tests in
  `edit_session.rs` (forward swap accepted + `RecordId` preserved, reverse
  swap rejected, swap to an unrelated op type rejected).
- [x] Shared `validate_component_source` extracted (both `PlaceComponentParams`
  and `DetachComponentParams` validate the same `source` shape).
- [x] `dto.rs` — `DetachComponent` buckets into `FeatureKind::Boolean`
  (same interim-bucket precedent as `PlaceComponent`/`TransformBody`),
  labelled "Detach Component".
- [x] Wire slot-order pin + H5 op-set-agreement test both extended with
  `DetachComponent` rows (always empty — no mate, no identity, no
  topological dependency at all).
- [x] **Worker**: `ComponentOp.cpp` refactored — `execute_place_component`
  and `execute_detach_component` share one `resolve_source_and_publish`
  pipeline (the two build IDENTICAL geometry; only the RECORD's params
  differ). `PlanExecutor.cpp` dispatch line added.
- [x] **Gate — worker CTest**: `test_component_ops.cpp` grew from 4 to 7
  cases (3 new: exact-volume parity with `PlaceComponent`, placement-transform
  invariance, embedded-source refusal) — ctest still **114/114** (same target,
  more cases inside it).
- [x] **Gate — Rust integration**: `component_ops.rs` gained
  `detach_component_preserves_body_and_volume_across_the_swap` — the FULL
  swap through the REAL worker (not just the Rust-core unit test): place,
  regen, swap via `UpdateOperationParams`, regen again, assert same `BodyId`
  + same exact volume, and that `rt.projection()`'s `FeatureDto.op_type`
  actually reads `"DetachComponent"` after the swap.

Gate: worker CTest **114/114** · `cargo fmt --all --check` · `cargo clippy
--workspace --all-targets -- -D warnings` clean · `ONECAD_WORKER_PATH=...
ONECAD_REQUIRE_WORKER=1 cargo test --workspace` all green except the same two
pre-existing failures WP-0.1 bisected to baseline (untouched by this WP).

Next: WP-1.3 (bridge `onecad-library` into the app crate; build
`ModelingServices.CommandApi`; author-time blob copy-in for `Embedded`
sources) per the plan.

## COMPONENT-LIBRARY WP-1.1 (2026-08-12) — GATE PASSED

New `onecad-library` crate: package format, index, content-addressed blob
store, resolution, registry trait. Pure domain crate — deliberately does
**not** depend on `onecad-core` (Open Q1 resolved: op-params-facing types stay
in `onecad-core::document::record`; this crate owns the on-disk
`component.toml` package format independently, since its `[parameters]`
role/domain/snap shape has no op-params analogue — the app crate, WP-1.3, is
where the two get translated) and must not depend on `tauri` or perform
network I/O in v1 (same walls `onecad-core` keeps; a future `RemoteRegistry`,
P4, is the sole place network I/O may enter).

- [x] `package.rs` — `ComponentPackage`/`Identity`/`Metadata`/`SourceSpec`
  (`Embedded`/`Generator`/`Document` — the FULL 3-variant spec §2.1 shape,
  wider than `onecad-core`'s P0-reduced `ComponentSourceRef` since this crate
  isn't bound by the op's phased rollout)/`ParameterSpec`/`ParameterRole`/
  `AttachmentSpec`. `parse`/`validate_identity` (namespaced id, non-empty
  version, `sha256:`-prefixed revision) + `compute_revision` (SHA-256 over
  every file in the package dir EXCEPT `component.toml` itself, sorted by
  relative path — avoids the self-referential "the manifest that carries the
  hash can't hash itself" problem).
- [x] `blob.rs` — filesystem-backed `blobs/<sha256>` store, genuinely SEPARATE
  from `onecad-core::io::imports`'s zip-embedded store (different lifecycle:
  shared across documents, survives document close) but same design
  principles: content-address filename, `MAX_BLOB_BYTES` cap, re-hash-on-every-read
  integrity check. Regression pin `a_flipped_byte_on_disk_is_caught_on_read`
  mirrors `onecad-core`'s `verify_blob_catches_a_flipped_byte`.
- [x] `index.rs` — `library.json` (`indexVersion: 1`, `id -> version -> entry`),
  atomic write (sibling temp + rename, mirrors `onecad-core::io::container`'s
  save discipline), `reindex` walks package directories and SKIPS (not
  aborts on) a malformed one, `load` of a missing file is an empty index (a
  fresh library root is a legitimate state, not an error).
- [x] `resolve.rs` — `{id, version, revision} -> ResolvedSource` (Generator
  only — P1 scope, mirrors the op's reduction). Revision mismatch is a typed
  `LibraryError::RevisionMismatch`, never a panic or a load-breaking `Err` —
  converting it to `NeedsRepair` is the app-crate's job (WP-1.3), not this
  crate's (spec §4).
- [x] `registry.rs` — `RegistrySource` trait + `LocalRegistry` impl.
- [x] `tables.rs` — stub (P2 content).
- [x] `Library` struct (`lib.rs`) ties it together: `open`/`index`/`reindex`/
  `get`/`resolve_source` all real; `save_component`/`save_template` are
  typed-error stubs (P3 — authoring needs document/geometry access this crate
  deliberately doesn't have) with their real signatures settled now so P3
  doesn't redesign the surface.

Gate: `cargo test -p onecad-library` **22/22** (package.toml parse/validate
round-trip incl. a malformed-TOML fixture, revision-hash recompute-on-write
determinism + content-sensitivity + manifest-self-exclusion, `library.json`
atomic write + reindex-reconciles-disk + skips-bad-package, blob integrity
incl. the flipped-byte regression pin, end-to-end open→reindex→get→resolve) ·
`cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D
warnings` clean (new crate added to workspace members) · full workspace
`cargo test --workspace` against the real worker still green (same two
pre-existing failures, unaffected — this WP touches no worker-facing code).

Next: WP-1.2 (mature the op family to the full spec shape — add
`SetComponentParams`/`ReplaceComponent`/`DetachComponent`, wire `DetachComponent`
in C++, extend `ComponentSourceRef` with `Embedded`) per the plan.

## COMPONENT-LIBRARY WP-0.2 (2026-08-12) — GATE PASSED, P0 COMPLETE

Completes P0's combined go/no-go (WP-0.1's latency GO + this WP's op skeleton).
Lands the real `KnownOperation::PlaceComponent` (spec §3.1), generator-source-only,
with a hardcoded (non-table-driven) ISO 4762 M6×20 SHCS generator — table-driven
sizing is P2.

- [x] **All 5 Rust mirror sites updated in one commit** (record.rs `KnownOperation`
  enum + `KNOWN_OP_TYPES`, `element_refs_mut`, `op_type`, `derive_inputs`;
  `worker::wire::wire_op_inputs`; `document_runtime::element_ref_input`) — plus a
  6th the plan's survey undercounted: `edit::session`'s `validate_place_component`,
  wired into **both** `add_operation` and `update_operation_params` (this pair, not
  the struct's own `validate()`, is the actual authoring-time enforcement point).
- [x] New types (`record.rs`): `PlaceComponentParams`, `ComponentSourceRef`
  (`Generator` variant only — P0 scope), `ComponentParamValue`, `ComponentMate`,
  `MateKind`, `FrozenPlacement` (reuses `TransformRotation` verbatim).
- [x] **Real defect caught by the round-trip test, not by inspection**: `#[serde(tag
  = "kind", rename_all = "camelCase")]` on `ComponentSourceRef` renames the VARIANT
  name ("generator") but does **not** cascade `rename_all` into the struct-variant's
  own fields — `generator_id`/`generator_version` serialized snake_case, so the
  worker read an empty `generatorId` and every placement failed `OP_FAILED`. Fixed
  with explicit `#[serde(rename = "generatorId"/"generatorVersion")]`, matching the
  codebase's existing `AxisRef` precedent (which uses the same per-field rename for
  the identical reason, not because the names differ semantically). Pinned by
  `place_component_source_fields_are_camel_case` so this can't regress silently
  again — this class of bug (an internally-tagged enum's struct-variant fields
  silently keeping snake_case) is worth checking for on any FUTURE internally-tagged
  enum added to this file.
- [x] Wire slot-order pin (`wire_op_inputs_slot_order_is_the_repair_slot_table`,
  `wire.rs`) and the H5 op-set-agreement test (`element_refs_mut_covers_exactly_the_
  wire_typed_ref_slots`) both extended with `PlaceComponent` rows (mate-present and
  no-mate cases) — the `_covered` exhaustiveness guard in the op-set test forces
  this on every future variant.
- [x] `dto.rs`: `feature_kind`/`default_label` non-exhaustive-match compile errors
  (expected — new enum variant) resolved: `PlaceComponent` buckets into
  `FeatureKind::Boolean` (same "no dedicated icon yet" interim-bucket precedent as
  `TransformBody`), labelled "Place Component".
- [x] **Worker**: `worker/src/ops/ComponentOp.h/.cpp` (mirrors `HoleOp.h`/`ImportOp.cpp`'s
  shape) — validates `source.kind == "generator"` + non-empty `generatorId`
  (P0 scope refuses `embedded`/`document` with `OP_FAILED`/`UNSUPPORTED` code,
  `Status::Failed` — distinct from `PlanExecutor`'s dispatcher-level
  `Status::Unsupported`, reserved for an entirely unrecognized opType), builds the
  hardcoded M6 SHCS solid (head+shank cylinders, checked Union fuse), applies
  `placement` (same `T ∘ R` normative order as `TransformBody`, `TransformOp.cpp`'s
  `gp_Trsf` pattern reused), publishes through `single_solid_policy` as a NewBody.
  Dispatch line added to `PlanExecutor.cpp::run_single_op`.
- [x] **Gate — worker CTest**: `worker/tests/test_component_ops.cpp` (4 cases:
  exact M6 SHCS volume ~1036.73mm³, rigid-placement volume invariance, embedded-source
  refusal, empty-generatorId refusal) — ctest **114/114** (113→114).
- [x] **Gate — Rust integration**: `src-tauri/tests/component_ops.rs` (worker-backed,
  skip-if-missing, mirrors `hole_ops.rs`) — `PlaceComponent` through `DocumentRuntime`
  end to end with the EXACT `QueryMassProperties` volume (not mesh-chord), plus a
  save→fresh-worker-reopen round trip proving the op has zero library-root dependency
  (nothing in it reads a library folder at all — the deliberate P0 de-risking choice
  the plan called out for exactly this reason).
- [x] 4 new Rust unit tests in `record.rs` (`place_component_is_a_known_op_type`,
  `place_component_source_fields_are_camel_case`, `place_component_derives_
  conditional_mate_input`, `place_component_validation_matrix`).
- [x] SCHEMA §7.3 doc block not yet added (deferred — flagging as owed before P1
  ships; WP-0.1's §7.5 `ClassifyElement` entry was landed, this op's wasn't in this
  pass and should land alongside WP-1.2's op-family maturation, same commit as the
  full `SetComponentParams`/`ReplaceComponent`/`DetachComponent` doc pass).

**P0 combined gate: GO.** WP-0.1's p95=0.16ms latency + this WP's passing
`component_ops` tests satisfy the spec's "one M6 screw placed concentrically on a
hole in a dev build" framing (concentric placement itself is P1.5's live-drag job;
this WP's test computes a fixed `placement` directly, per the plan's explicit P0/P1
boundary).

Gate: worker CTest **114/114** · `cargo fmt --all --check` · `cargo clippy
--workspace --all-targets -- -D warnings` clean · `ONECAD_WORKER_PATH=...
ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **74 test binaries, all green**
except the same two pre-existing failures WP-0.1 already bisected to baseline
(`sketch_on_face.rs::a_line_across_...`, `wire_contract.rs::nested_inner_disk_
parity_and_reopen_stability`, both `fetch body mesh` panics, both reproduced at
baseline commit `5036597`, neither touched by this or the prior WP). Frontend
untouched this WP (Rust/C++ only) — tsc/vitest/build not re-run, no regression
surface.

Next: WP-1.1 (`onecad-library` crate scaffold: package/index/blob/resolve,
embedded-only) per the plan.

## COMPONENT-LIBRARY WP-0.1 (2026-08-12) — GATE PASSED, P0 GO

Plan: `TheComponentLibrary/onecad-component-library-spec.md` (normative spec) +
implementation plan at `/Users/andrejvysny/.claude/plans/do-thorough-analysis-of-abstract-thunder.md`.
First work package of the Component Library feature (placeable mechanical
parts, mate-like snapping, parametric generators, templates) — the P0
foundation spike's risk item: is interactive surface classification fast
enough for a live hover gesture.

- [x] New read-only kernel verb `ClassifyElement` (SCHEMA §7.5) — surface/curve
  classification + a seatable frame (plane origin+normal, cylinder/circle
  axis+radius) for a picked face/edge. Addressed like `QueryElement` but with
  no `snapshotId` (always current head — a continuously re-issued live hover
  query, not a pick tied to one snapshot, same reasoning as
  `QueryMassProperties`). `worker/src/session/ClassifyElement.h/.cpp`
  (modeled on `FaceProjection.cpp`'s `resolve_seed` shape), dispatch in
  `worker/src/main.cpp`.
- [x] Rust plumbing: `ClassifyElementDto`/`ClassifyElementFrameDto`
  (`dto.rs`), `classify_element_args`/`parse_classify_element` (`wire.rs`),
  `ElementQuery::classify_element(_by_topo_key)` trait methods + `PendingBackend`
  fallback (`worker/mod.rs`), `WorkerManager` impl (`manager.rs`),
  `#[tauri::command] classify_element` (`api/mod.rs`), registered in `lib.rs`.
- [x] Frontend: `ClassifyResult`/`ClassifyFrame` types (`ipc/types.ts`),
  `CadClient.classifyElement` + both `tauriClient`/`mockClient` impls (mock is
  honest about its gap — only the plane case gets a real frame, no cylinder
  axis synthesized from mesh data).
- [x] **Closed a real platform gap**: `ModelingServices.GeometryQuery` was
  declared in `manifest.ts` since the Platform refactor but never registered
  anywhere. `register.ts::contributeModeling` now calls the codebase's first
  `scope.registerService(ModelingServices.GeometryQuery, …)`, proven by a new
  vitest asserting `platform.services.require` resolves it (not just that it's
  declared).
- [x] **P0 go/no-go, measured**: `src-tauri/tests/classify_latency.rs`
  (worker-backed, skip-if-missing) builds a real 20×20×25 box via
  Sketch+Extrude, issues 500 `ClassifyElement` calls over the live stdio
  round-trip mixing all 6 faces + 12 edges. **p50=0.08ms p95=0.16ms
  p99=0.35ms** — gate was p95<16ms. **GO**: the live-hover gesture (WP-1.5)
  is not blocked; no click-to-classify fallback needed.
- [x] SCHEMA §7.5 doc block + §14 changelog entry for the new verb.

**Two pre-existing test failures found and bisected, NOT this work**: both
`sketch_on_face.rs::a_line_across_the_projected_rect_yields_two_extrudable_regions`
and `wire_contract.rs::nested_inner_disk_parity_and_reopen_stability` panic
identically (`fetch body mesh`, `get_mesh().expect(...)`), in isolation and in
the full suite. Neither test file, nor `get_mesh`/tessellation/mesh-cache code,
is touched by this WP. Confirmed pre-existing by stashing all WP-0.1 changes,
rebuilding the worker from the unmodified baseline commit (`5036597`), and
re-running `a_line_across_...` in isolation — **identical panic at baseline**.
Root cause not investigated further (out of scope for this WP); flagged for
whoever next touches mesh-fetch/tessellation.

Gate: worker CTest 113/113 · `cargo fmt --all --check` · `cargo clippy
--workspace --all-targets -- -D warnings` clean · `ONECAD_WORKER_PATH=... 
ONECAD_REQUIRE_WORKER=1 cargo test --workspace` all green except the two
pre-existing failures above · `bunx tsc --noEmit` clean · vitest **241 files /
4117 tests** all pass · `bun run build` clean. Playwright not run (no viewport/
UI surface changed this WP — WP-1.4/1.5 are where a Playwright spec is due).

Next: WP-0.2 (`KnownOperation::PlaceComponent` skeleton, generator-source-only,
hardcoded ISO 4762 M6) completes the P0 combined gate; then P1 (`onecad-library`
crate scaffold onward) per the plan.

## MODELING CORRECTNESS P3 — PUBLICATION POLICY (2026-08-12) — COMPLETE

- [x] Machine-readable per-operation contract rows: `docs/qa/modeling-operation-contracts.json` (35 rows / 16 operations) covering support status, validation tier, body lifecycle, empty/multi-solid semantics, and `uiExposure`.
- [x] Transform policy row; wired `TransformBody` through the common `publication_decision` Tier A validator (`worker/src/ops/TransformOp.cpp`).
- [x] ImportStep policy row and explicit invalid-solid warning exception documented in contracts.
- [x] UI mode disposition recorded: Revolve Intersect, Mirror fuse, Extrude Intersect hidden; Pattern fuse hidden; Transform move/copy exposed; OffsetFace Total/Diameter deferred.
- [x] `scripts/verify-modeling-contracts.mjs` validates schema, required fields, uniqueness, and coverage-manifest cross-reference.

Gates: worker Release build; CTest 113/113 passed; `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` passed; contract/coverage verifiers passed.

## MODELING CORRECTNESS P4 — BOOLEAN INTERSECT VERTICAL (2026-08-12) — COMPLETE

- [x] C++ standalone fixtures: overlap, containment, identity, face/edge/vertex touching refusal (`worker/tests/test_boolean_intersect.cpp`). CTest 114/114.
- [x] Rust real-worker tests: preview/commit parity, disjoint refusal, save/reopen, undo restoration (`src-tauri/tests/preview_boolean.rs`).
- [x] Frontend + Playwright: Intersect tool selection, target/tool pick, Apply, single body row; added `data-testid` to boolean op chip buttons.

## MODELING CORRECTNESS P4 — REMAINING COVERAGE (2026-08-12) — COMPLETE

- [x] MirrorBody Playwright flow (`e2e/mirror-body.spec.ts`).
- [x] Linear/Circular Pattern Playwright flows (`e2e/linear-pattern.spec.ts`, `e2e/circular-pattern.spec.ts`).
- [x] Critical mode closure tests: Extrude/Revolve overflow hide Intersect; Mirror/Pattern chips have no fuse/union toggle (`src/features/toolbar/ModelToolChips.test.tsx`).
- [x] Real-worker corpus executor (`src-tauri/tests/corpus_executor.rs`): enumerates all `corpus/cases/*.json`, executes `a_sketch_extrude_blind` end-to-end (volume within tolerance), and records explicit unsupported reasons for the rest. Zero unclassified files.

Gates: worker Release build; CTest 114/114; `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` passed; TypeScript / Vitest passed; contract/coverage verifiers passed; targeted Playwright boolean/pattern/mirror specs 12/12 (Chromium + WebKit, retries 0). Full Playwright suite not re-run yet; prior unrelated `unsaved-guard` / `view-ux` flakes remain the only known blockers.

## MODELING CORRECTNESS P2 GATE (2026-08-12) — AUTOMATED LANES PASSED; MANUAL TAURI SMOKE OPEN

- [x] Build worker Release against pinned OCCT 8.0.1 fingerprint `0a6a1dce34181289`.
- [x] CTest 113/113 passed.
- [x] Cargo fmt/clippy/workspace tests with `ONECAD_REQUIRE_WORKER=1` all passed.
- [x] Kernelbench T0 both backends: 136 records, 0 gating failures, replay 136 stable, metamorph 48 passed, differential same-status 136. Summary matches baseline except timing.
- [x] TypeScript check, production build, Vitest 241 files / 4116 tests passed.
- [x] Playwright zero retries: Chromium 196/196 passed, WebKit 196/196 passed.
- [ ] Manual Tauri smoke (open project → extrude → fillet → undo → save → reopen) still owed; no automated equivalent exists.

Fixed P2 region-identity regression found by real-worker gate:
- `src-tauri/tests/sketch_on_face.rs`: `region_extrude_record` now authors `region_identity_version: 2` so fragmented projected regions resolve under the V2 exact-profile detector.
- `src-tauri/tests/wire_contract.rs`: `nested_inner_disk_parity_and_reopen_stability` now sets `region_identity_version: 2` when binding the exact disk/annulus region ids.

## MODEL-CORRECTNESS-P0 + REF-OWNERSHIP-AND-SNAPSHOT P1 (2026-08-11) — COMPLETE

- [x] P0 — deferred replacement guard; authoritative `SaveOutcome`; classified terminals; zero-solid Boolean refusal; circular `angle / count`; semantic Draft refusal; per-session preview ownership; Boolean re-arm.
- [x] P1A — typed-ref local ownership before ladder scoring; stale ToFace/Hole promotion and OffsetFace adoption fences.
- [x] P1B — provenance-versioned repair candidates; additive typed Revolve body-edge axis with no ordinal fallback.
  - [x] `ResolveRefs` echoes `{revision, snapshotId, refId, bodyId}`; candidate loads key on that tuple, old events/loads/clicks cannot promote stale ordinals. Focused FE 32/32; Rust lib 256/256.
- [~] Gates — worker Release + CTest 112/112; Cargo fmt/clippy/workspace worker tests; `npx tsc --noEmit`; Bun build; Vitest 241 files / 4115 tests all pass. Full Playwright retries 0: Chromium 196/196, WebKit 196/196 after `commitExtrudeAtHandle` releases a failed retry pointerdown. Manual Tauri smoke and T0 digest/semantic campaign remain open.

Gate evidence: baseline `1c11d49`; `scripts/build-worker.sh Release`; `ctest --test-dir worker/build --output-on-failure`; `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 cargo test --workspace`; `npx tsc --noEmit`; `bun run build`; `bun run test`; `bunx playwright test e2e/repair-rebind-multibody.spec.ts e2e/sketch-multi-object.spec.ts --project=chromium --project=webkit --retries=0`. Untracked roadmap bundle intentionally untouched.

## TRACK A — a CI that means something (2026-08-10)

Goal: before any R1 work, make "green" a fact CI enforces rather than a claim verified by hand. Four work packages, all landed.

### A1 — the inline-edit test was a REAL race, not a slow deadline
- [x] **My first diagnosis was wrong and the second attempt found the mechanism.** I read `InspectorPanel.test.tsx`'s failure as an arbitrary wall-clock deadline and widened it; it went red again in CI **with a 4 s budget**, on an assertion whose work is pure microtasks — which a timeout cannot explain.
- [x] Actual cause: `DimensionInput.commit()` reads the `text` STATE and calls `onCommit` only when the formatted new value differs from the current one. Firing `change` and `keyDown` inside ONE `act` nests their flushes into the outer scope, so the `keyDown` handler can still close over the pre-change text — `formatValue(n) === formatValue(value)` holds, the commit is CORRECTLY skipped, and `applyEditCommand` never fires. Split into two `act` scopes so Enter runs against a component that has re-rendered.

### A2 — the 12 red e2e specs were THREE unrelated causes, not one
The plan (and `TODO.md`) attributed all 12 to the live-dim tree. Wrong on 10 of them.
- [x] **Toolbar flyout families (8 of 12).** `d81f758` made the toolbar one slot per family, so Center rectangle (behind Rectangle) and Ellipse (behind Circle) have no button until picked from a flyout. `selectSketchTool` now tries the direct button, then DISCOVERS the owning flyout by opening each chevron rather than hardcoding a family table. Menu rows concatenate title+shortcut with no separator (`"EllipseO"`), so the title span is matched exactly. Note for future helpers: **`count()` does not auto-wait** and must never be the first thing asked of a mounting toolbar.
- [x] **Extrude gesture + a wrong precondition (2).** Confirming from the armed state leaves depth at zero and nothing commits (UNIFY-UX), so a real handle grab is required — restored as a shared `dragExtrudeDepth`, filling an orphaned docstring that had lost its function. The body-COUNT precondition was wrong in EITHER drag direction: the rectangle overlaps the seeded body, so auto Add/Cut resolves to `Cut` and the extrude modifies `body1`. Correct behaviour, irrelevant to reattach — the precondition now asserts a moved revision.
- [x] **The angle change (2), and production is right.** The chip is withheld on a first segment because `angleLadder` authors nothing without a `prev`, so a drivable-looking chip would be a lie. The spec now types on a chained second leg. Two things the rewrite had to learn from the code: the field types the **visual corner angle**, not the raw turn (`cornerAngleOf = 180 - |turn|`, so a typed 30 is a 30° corner and a 150° turn), and geometry cannot be asserted with absolute headings — the plane's +U runs opposite to screen +x and leg 2 is stored end-first — so it measures the undirected corner at the shared vertex.
- [x] **Full Playwright suite 392/392** locally, both browsers, run alone.

### A3 — `worker-7.9.3` deleted
- [x] It never compiled (four benchmark sites call `failure.what()`, absent from 7.9.3's `Standard_Failure`) and `continue-on-error` masked that on EVERY run including green ones. The two persistence jobs stay: different question, and they build only `test_occt_persistence`, which never touches the benchmark sources.

### A4 — `ci.yml` split into two lanes, self-hosted + macOS shipping gate
- [x] **Self-hosted (Linux):** `linux-worker` (hygiene, build, selftest+fingerprint, ctest 110/110, edge-op determinism) and `linux-kernelbench` (`-p onecad-kernelbench`, T0 both backends, linux-x64 digest gate, cross-host semantics gate). Uses the PERSISTENT OCCT prefix, so the 40-90 min kernel build is paid once.
- [x] **macOS shipping gate:** packaging linkage smoke (no Linux equivalent), `cargo test --workspace`, both e2e projects, persistence pair.
- [x] **SECURITY:** every self-hosted job is gated to trusted code — a push to this repo, or a PR whose head branch lives here. Fork PRs get the full GitHub-hosted lane. User has since set fork-PR approval in Settings → Actions as the backstop.
- [x] **e2e is now gated in CI** — chromium and webkit, 392 executions.

### The finding that changed a design decision: digests are SAME-MACHINE
- [x] The macOS digest gate failed comparing **darwin-arm64 against a darwin-arm64 baseline**: same pinned OCCT source, same build id, same architecture, but GitHub's `macos-14` AppleClang is not the laptop's — and the trig-heavy `valence4-*` family moved. So platform-keying (yesterday's conclusion) is still too coarse.
- [x] The digest gate therefore runs **only on the self-hosted runner**, the one persistent machine. Both lanes gate the portable thing: **semantics**. `bench/robustness/baselines/README.md` records the measurement.

### Two CI-only flakes, both "a local-machine number standing in for a condition"
- [x] `ModelToolController.wave2` asserted straight after `flush()` — a single `setTimeout(0)`, ONE macrotask tick — while `editExtrudeFeature` awaits `endPreview` then `beginPreview`. Passed 18/18 in isolation, red in CI. Now waits for the condition.
- [x] `boolean-preview`'s lane polls carried 5 s; the lane opens behind a body pick, a Combine arm and a preview round-trip. It passed on both browsers in one CI run and failed on both in the next — a deadline flake. Raised to 20 s behind a named constant, assertions unchanged.

### Still open
- [ ] **Runner root installs** (Proxmox SSH was refusing connections, so these could not be done): `unzip` (blocks `setup-bun`, which is why `frontend` is GitHub-hosted) and Playwright's chromium libraries. The exact one-liner is in `ci.yml` above `e2e-chromium`. Both jobs move to the runner once they land.
- [ ] **`boolean-preview` on ubuntu chromium** fails a body pick that succeeds on macOS chromium — a plausible SwiftShader/GL difference and a real finding. Chromium runs on macOS meanwhile; investigating this is what would let the whole e2e lane move to the runner.
- [ ] **OPEN DECISION — `retries` in CI.** With e2e gated, the suite runs ~99.5% clean: 195/196 per job, with a DIFFERENT spec failing each run (`boolean-preview`, `tree-visibility`, live-dim…). Every one so far has been a local-machine deadline standing in for a condition, and each is individually fixable — but it is a long tail. `playwright.config.ts` sets `retries: 0` deliberately, with the rationale that retries once hid flakes that were invisible in CI and hard-red locally (`TODO.md` OPEN DECISION, and the config's own comment). That rationale predates e2e being a gate at all. Choose: keep 0 and grind the tail down, or `retries: 1` on CI only and keep grinding without a red gate.
- [ ] Nothing is a REQUIRED check yet — worth turning on once the retries decision lands, so the gate actually blocks.

## SELF-HOSTED RUNNER — Linux benchmark host, S1-S4 ladder (2026-08-10) — GATE PASSED

`.github/workflows/self-hosted.yml`, `workflow_dispatch` ONLY. Runner `prx-lxc` = Proxmox LXC 107 "GithubRunner", Debian 13 trixie, unprivileged, **4 cores / 8 GB / 70 GB**. Purpose: long campaigns move off the Mac (wall-clock is free there); the Mac keeps the fast iteration lane.

- [x] **SECURITY — the repo is PUBLIC and the runner is on the home LAN.** Self-hosted jobs are in their own workflow because `ci.yml` triggers on unfiltered `pull_request:`; a fork PR reaching a self-hosted job executes the fork's code on that hardware. Every job additionally re-checks `github.repository` + `github.event_name`. Never add `pull_request`/`pull_request_target` there. 0 forks today.
- [x] **S1 environment report.** Container shipped with only curl/tar/python3/perl and **no sudo**. One-time root install via `pct exec 107`: `git ca-certificates build-essential cmake ninja-build pkg-config libboost-dev libeigen3-dev nlohmann-json3-dev`, then `libx11-dev libxext-dev libxmu-dev libxi-dev libgl-dev libglu1-mesa-dev`. gcc 14.2.0 · cmake 3.31.6 · ninja 1.12.1. (`git`'s absence is not cosmetic: `actions/checkout` silently falls back to the REST tarball, leaving no `.git`.)
- [x] **S2 pinned OCCT 8.0.1 from source.** Artifact provenance byte-identical to macOS (same `sourceCommit`, `buildId`, normalized option list). 120 MB prefix / 49 `libTK*.so`. Persistent prefix outside the workspace + an assert that a second invocation prints "Reusing pinned OCCT", so the 40-90 min build is paid once.
  - OCCT's `TKService` compiles against Xlib on Linux where macOS uses Cocoa. Installing X11 headers is the correct fix, NOT `USE_XLIB=OFF`: `HAVE_XLIB` is auto-detected and absent from the pinned option policy, so headers cannot move the fingerprint, whereas suppressing the module either edits the policy or lets two materially different builds share one fingerprint.
- [x] **S3 worker + ctest.** **`ctest` 110/110** and **`fingerprint 0a6a1dce34181289` identical to macOS** — the seed is `occtVersion|sourceCommit|buildOptions|buildId|kernelPolicyVersion`, all platform-independent, and that design now has evidence. Edge-op determinism `cmp` byte-identical.
- [x] **S4 kernelbench T0, both backends.** 136 records · `gatingFailures` 0 · replay 136 stable / 0 unstable · metamorph 48 passed / 0 failed · differential 136 same-status — semantically identical to the macOS baseline. Timing **p50 29.7 ms / p95 993 ms** vs macOS 10.3 / 62.3 (4 shared LXC cores); M5 sizing must use these, not the Mac numbers.

### Three repo defects the port surfaced (all fixed)
- [x] **`json_fwd.hpp` was never vendored** next to `json.hpp`, and nothing declares it — there is no `find_package(nlohmann_json)` in the build. Six benchmark headers resolved it from the system. macOS was green only because Homebrew also ships 3.12.0; Debian's 3.11.3 puts a second inline ABI namespace into `nlohmann` and every `json_pointer` reference goes ambiguous. **This was a live latent failure on the shipping platform** — any Homebrew bump past 3.12.0 breaks macOS identically. `VENDOR.txt` already stated the invariant this restores.
- [x] **`DT_RUNPATH` is not transitive.** GNU ld defaults to `--enable-new-dtags`, so the OCCT path resolved only the worker's own `DT_NEEDED` entries; all 26 OCCT-internal edges (`libTKOffset -> libTKG2d`) fell through to the system path and the worker died at startup. OCCT's libraries carry no RPATH of their own. `-Wl,--disable-new-dtags` under `if(NOT APPLE)` restores transitive `DT_RPATH`; the macOS link line is unchanged.
- [x] **`std::reverse` without `<algorithm>`** in `test_polygon_fill.cpp` — libc++ transitive, libstdc++ not. FLAGGED: 34 more files use `std::u?int*_t` without `<cstdint>` and currently compile only by transitive luck; a separate hygiene sweep, not landed here.

### Digests are platform-dependent; semantics are not
- [x] Same pinned OCCT 8.0.1, identical build id AND identical 16-hex fingerprint, yet **182 of 272 digest values differ**: `translated` inputDigest **0/32** (translation is exact in FP) · `rotated` **32/32** (trig) · `base` **20/72** — precisely the trig-built shapes (all 8 `valence4`, `overflow-02/-03`); every box and `valence3` is bit-identical · `normalizedDigest` **130/136**.
- [x] The 1e-9 quantization CANNOT fix this: rounding to a grid narrows but never closes boundary straddles, and with thousands of quantized values per record a straddle is near-certain. A digest is a **same-host** regression tripwire only.
- [x] `digests.json` keyed `suite|case|backend|variant|platform` (256 macOS rows migrated to `darwin-arm64`, 136 `linux-x64` rows recorded). `record` only replaces the current platform's rows; `compare` on an unrecorded platform exits 3.
- [x] **`semantics.json` is new and NOT platform-keyed** — the portability claim. Verified by running T0 on macOS against the baseline recorded on Linux: both hosts satisfy the same row. Timing and tolerance distributions excluded (host properties, not kernel behaviour).
- [x] Consequence for M5: "byte-identical `results.jsonl` across `--jobs`/`--shard`/`--resume`" stays valid (same host, same binary). Any CROSS-host claim must be semantic.

### Full verification sweep at `cb88ba9` (2026-08-10)
All four suites run on macOS with the three worker fixes in place, plus the manifest tool exercised against both suites and both platforms.

- [x] **ctest 110/110** · `cargo fmt --all --check` · `clippy --workspace --all-targets -D warnings` · `cargo test --workspace` with `ONECAD_REQUIRE_WORKER=1` — all green, so the vendored `json_fwd.hpp` and the `NOT APPLE` link-option guard leave macOS untouched.
- [x] **vitest 241 files / 4102 tests, all pass** · `bun run build` green.
- [x] **Manifest tool, tested rather than assumed.** `fillet/matrix:m1` 120 rows unchanged (the migration path I had NOT previously exercised) · `t0` 136 rows unchanged · macOS satisfies the Linux-recorded semantics row · guards: unrecorded platform → 3, bad mode → 2, missing semantics suite → 3 · **negative controls**: a tampered digest and a tampered `gatingFailures` both correctly report a mismatch · census 392 rows (`darwin-arm64` 256, `linux-x64` 136), every key 5 fields.
- [x] **DT_RPATH verified at the ELF level on Linux**, not inferred from a green build: tag `0x0f RPATH` (not `0x1d RUNPATH`) and **0 unresolved libraries, down from 26**.
- [x] Runner after the full ladder: 4.8 GB used of 69 GB. Persistent state ~1 GB (OCCT prefix 120 MB, worker build 142 MB, cargo 97 MB, workspace 638 MB).

### Two stale claims corrected by that sweep
- **`InspectorPanel.test.tsx:341` is CI-FLAKY, not broken.** `ci.yml` went red at `cb88ba9` on `expected "applyEditCommand" to be called at least once`; the preceding commit `b344ab6` was green with byte-identical frontend code (the only delta was `TODO.md`), the file passes 5/5 locally in isolation and 4102/4102 in the full suite, and a rerun of the SAME sha went green. It is a `vi.waitFor` timing out under CI load (that run spent 102 s in environment setup alone). **A flaky test inside a gate is a WP0.2 blocker** — wiring Playwright/vitest as required checks makes this fail the branch at random.
- **`HANDOFF.md:126`'s "4 fail (theme.spec, pre-existing)" is stale.** `theme.spec.ts` passes clean. A full-suite run under CPU contention reported 19 failures; re-running the 7 non-sketch ones in isolation (`offset-face`, `theme` ×2, `transform-body`, `history-inline-dimension`, `multiregion`, `point`) gave **38/38 pass**. They were contention artifacts of running vitest and Playwright concurrently — `workers: 1` + `retries: 0` makes this suite timing-sensitive, so e2e must be run alone.

### Playwright: 380/392, and the 12 are exactly WP0.1's scope
Re-run in isolation, deterministic and symmetric (6 chromium + 6 webkit), all four spec files from the landed sketch-angle work: `center-rect` ×1, `ellipse` ×3, `live-dim-line` ×1, `sketch-reattach` ×1, per browser. This confirms the plan's WP0.1 estimate exactly — the specs encode the OLD absolute-heading semantics and must be reconciled against the signed-turn chip before Playwright can be a CI gate.

### Still open
- [ ] **USER:** Settings → Actions → General → "Fork pull request workflows from outside collaborators" must require approval. Not readable via REST for a public repo. Consider making the repo private.
- [ ] De-flake `InspectorPanel.test.tsx:341` before vitest/Playwright become required checks (WP0.2).
- [ ] `m1` has `darwin-arm64` rows only; record `linux-x64` when the suite next runs there.
- [ ] `CLAUDE.md` still tells contributors to `brew install nlohmann-json`; that package is now inert (no `find_package`). Fold into the doc pass.
- [ ] The 34 `<cstdint>` transitive-include cases above.

## REF-H0 — Fresh Subelement Identity Contract (2026-08-09) — COMPLETE

Goal: a face, edge, or vertex promoted from the current published snapshot resolves
directly and uniquely on that unchanged worker head. Shell, OffsetFace, Hole, and
Fillet must consume the same trusted identity path; ambiguity thresholds and
operation-specific ordinal fallbacks remain unchanged.

### Investigation
- [x] Reproduce root cause from live code/history: `AcquireElementIds` returns worker
  evidence, Rust mints and caches the `ElementId`, but the authoritative worker-head
  `ElementMapPartition` never receives that binding. `PrepareOffsetFace` then treats
  the fresh id as authoritative and refuses its inevitable partition miss.
- [x] Confirm Shell's separate persistence gap: legacy `ShellParams.openFaces`
  stores bare ids, so Rust discards the frontend's typed face evidence before
  `PreviewOp` and full replay. The contract now adds typed `ShellParams.faces` in
  strict lockstep while preserving absent/empty legacy compatibility.
- [x] Audit `body_<uuid>` normalization: frontend/Rust/worker conversions are
  consistent in the clean-box path; not the root cause.
- [x] Audit mesh face/edge ordinals: tessellation and resolver use the same
  `TopExp::MapShapes` domains; not the root cause.
- [x] Find secondary MESH1 defect: partially persistent id tables mix `el_*` and
  TopoKeys, while the picker treats every label as persistent when the body-global
  flag is set.

### Red-first contracts
- [x] Box face/edge/vertex `AcquireElementIds` round trip: same head resolves
  `unchanged`, same id and TopoKey; `QueryElement(elementId)` reports present.
- [x] `AcquireElementIds` then `PrepareOffsetFace` with the returned id succeeds.
- [x] Fresh promoted top-face Shell `PreviewOp` returns no `NeedsRepair`, without a
  prior operation pre-seeding the partition.
- [x] MESH1 mixed-id pick classifies `el_*` as persistent and `f:N`/`e:N` as
  snapshot-scoped evidence.

### Implementation
- [x] Implement internal `BindElementIds`: validate exact/idempotent evidence and
  atomically install the full Rust-minted batch into the snapshot-fenced worker
  head; promotion returns and caches only after bind succeeds.
- [x] Make identity reads use one atomic published-state snapshot.
- [x] Implement `ShellParams.faces` in strict order/id lockstep with `openFaces` so
  preview, full replay, save/reopen, and fresh-worker replay retain typed evidence;
  accept absent/empty `faces` only for legacy bare-id behavior.
- [x] Fix shared preview diagnostics to report actual tool/op, never `extrude` for
  Shell/OffsetFace/Hole.
- [x] Keep safe refusal: no threshold relaxation, first-candidate fallback, or
  operation-specific identity path.

### Verification
- [x] Run focused worker ctests and real-worker Rust identity/OffsetFace/Shell tests.
- [x] Re-run Fillet fresh selection lifecycle without manual repair.
- [x] Run worker build + ctest, Cargo fmt/clippy/workspace tests, focused frontend
  tests, TypeScript, and production build.
- [x] Review final diff; preserve unrelated sketch/live-dimension worktree changes.

Gate: worker CTest 110/110; real-worker Rust workspace green; fmt + clippy
`-D warnings` green; focused frontend 127/127; full Vitest 240 files / 4051 tests;
`npx tsc --noEmit` + production build green. Fresh Shell cold reopen publishes
`needsRepair=0` at volume 2224; fresh promoted Fillet commit publishes
`needsRepair=0`. Final Shell body/load hardening: onecad-core 259 unit + 176
integration tests green; persisted foreign-body typed refs fail closed.

### Unresolved questions
- None.

## EXTRUDE DIRECT MANIPULATION — moving two-way arrow + dimension-only chip (2026-08-09, plan `simplify-this-chip-in-optimized-unicorn.md`) — FE GATE PASSED

Applying an extrude was a form, not a gesture: a twelve-control chip demanded every
decision before the user saw a result, while the arrow sat frozen at the profile
centroid and the chip sat on top of the prism. The arrow is now the operation — it
travels with the depth, points where material is going, reads two-way until the
first grab, and turns destructive on a Cut; the chip carries a dimension, a `⋯` and
✓/✕, and rides beside the arrowhead. **Extrude only** by decision; revolve, fillet,
hole, offset-face and transform keep today's clusters, and the new seams
(`DragHandle.setAxis`, overlay `axisFrom`, `ChipPlacement`) are built to generalize.

**TWO LATENT DEFECTS FOUND WHILE VALIDATING THE DESIGN, FIXED FIRST.**
- *The grab was a ratchet waiting for a moving arrow.* The drag reported the
  ABSOLUTE axis projection (`axisDepthFromRay`), which only ever worked because the
  arrow never moved. Anchored at `centroid + normal·depth`, every re-grab would add
  an arrow-length to the depth — and `commitExtrudeAtHandle` re-grabs on each
  `toPass` retry, so it would have drifted in the gate lane immediately. Now
  grab-relative, exactly like the offset-face arrow. `forceExtrudeGrab` zeroes the
  grab basis, which collapses to the old absolute mapping and keeps the depth-exact
  unit tests meaningful.
- *A press on the chip already started a depth drag.* The chip layer is a sibling of
  the canvas overlay inside the container the controller listens on, and the extrude
  branch — unlike fillet/shell/offset — never excluded chip targets. Parking the chip
  at the arrowhead would have made ✓ and the value field eat the grab.

DECISIONS WORTH CARRYING:
- **Two-way means UNDECIDED, not symmetric.** One glyph, one meaning: the second head
  retires at the first grab, and a symmetric extrude keeps a single head at the
  `+|depth|` face. The pick envelope tracks the drawn heads — a permanently symmetric
  envelope would reach backwards through the prism and the body (`depthTest:false`,
  no occlusion test in `raycast`) and swallow selection clicks.
- **`inside`, not `gap ≈ 0`, decides "into material".** A sketch coplanar with a body
  face reads gap ≈ 0 on BOTH sides; only the side whose ray exits through a BACK face
  actually starts in the solid. This is what lets the rule generalize past
  `hostFace` — a sketch on a datum plane through a block now cuts — without turning
  every coplanar second extrude into a wrong silent Cut.
- **The chip's live position never goes through the store.** `worldPos` is the mount
  effect's key, so a per-frame write would unmount the chip: focus lost and
  `commitOnBlur` firing on half-typed text. The controller calls `engine.moveChip`.
- **`offsetPx` is clearance to the chip's near EDGE.** The driver adds half the
  element's own size as a CSS percentage, so no layout read per frame. A centre-based
  offset was tried first and the chip still covered the arrow — which the e2e lane
  caught as an unclickable handle, because the chip-exclusion guard was already in.
- **The `⋯` button is a READOUT.** It shows the resolved boolean mode and raises a dot
  for any other non-default, because a mode the drag changes on its own must never
  change out of sight. `e2e/sketch-on-face.spec.ts` asserts that readout rather than
  the segments: a dismiss-on-outside-press popover cannot stay open across a drag.
- **Esc still cancels the TOOL, not the popover.** The controller owns Escape from a
  window listener registered in capture at construction, so nothing mounted later can
  preempt it; racing listener order to fake "first Esc closes" would be worse than Esc
  meaning one thing everywhere. Dismissal is an outside press or a second `⋯` click.
- `maybeNegativeDragHint` RETIRED — the arrow's colour, the prism tint and the `⋯`
  readout say it, and with the auto lane generalized its precondition rarely held.

BUG THE E2E LANE CAUGHT (and no unit test could): `LineSegments2` **extends Mesh**, so
the probe's `isMesh` filter admitted the fat edge lines, whose `raycast` reads
`raycaster.params.Line2` and throws on a plain raycaster — every extrude arm died in
the browser while jsdom stayed green. The probe now takes `userData.kind === "face"`.

GATE: `bunx tsc --noEmit` clean · `bun run build` green · vitest **240 files / 4040
tests** (from 236/3986) · hex gate 0 · Playwright chromium: the 25-spec
extrude/boolean/tree/revolve set **25/25**, with `revolve-commit` green UNMODIFIED
(proof the shared `BooleanModeSegments` was not collapsed out from under it).
NOT RUN: Rust, ctest, webkit — untouched by this wave.
STILL OWED: the manual `tauri dev` smoke, which is the only thing that can prove the
UX itself (arrow follows, chip never covers the prism, push-in cuts / pull-out joins).

FLAGGED: in the mock e2e lane the material probe finds committed face meshes, but the
generalized rule's coverage lives in `materialProbe.test.ts` +
`ModelToolController.extrudeGesture.test.ts`; the datum-plane-through-a-body case has
no e2e of its own yet. NEXT: generalize the moving arrow to revolve/offset-face, and
decide whether `tree-visibility.spec.ts` should hide its seed body.

## UNIFY-UX — Fillet/Revolve/Extrude chip parity (2026-08-09, plan `act-as-senior-ui-ux-tranquil-sparrow.md`) — FE GATE PASSED

Goal, from a user UI/UX review against Shapr3D/Fusion 360: unify the three tools'
armed-chip UX. Extrude already had a moving two-way arrow + leader-lined chip (see
above); Fillet/Chamfer had NO 3D handle at all (a whole-viewport screen-space claim)
and its chip sat centered directly ON the picked edge; Revolve showed nothing in the
viewport until both a face AND an axis were picked, guided only by a StatusBar string.
Landed in 4 phases, reusing the shared chip/store/overlay infra the extrude wave built
to generalize rather than growing three divergent implementations.

DECISIONS WORTH CARRYING:
- **Phase 0 (shared infra).** `ChipAnchorOpts` (`anchorAxisFrom`/`anchorOffsetPx`)
  promoted from `ExtrudeChipOpts`-only onto `EdgeOpChipOpts`/`RevolveChipOpts` too, with
  a shared `DEFAULT_CHIP_OFFSET_PX` (`toolChipStore.ts`) applied whenever an axis is
  given but no explicit offset — default-ON, not a per-call-site magic number.
  `HtmlOverlayDriver` now draws an actual DASHED LEADER LINE from the raw anchor to the
  offset chip position (previously the offset only repositioned the chip; no line
  existed) — a plain absolutely-positioned bordered `<div>`, rotated/scaled per frame,
  matching the driver's existing zero-React-render discipline. It piggybacks on
  whatever already mutates `worldPos`/`axisFrom` each frame (`setWorldPos`/
  `setAxisFrom`, called by `moveChip`) — no second write path, so extrude's
  continuously-moving arrow never desyncs from its line (regression-pinned:
  `HtmlOverlayDriver.test.ts` "tracks a LIVE move"). `ChipOverflow` extracted from
  `ExtrudeChipControls`'s `ExtrudeOverflow` as the shared `⋯`-button-plus-popover shell;
  extrude's own `chip-mode-readout` testid preserved via an override prop so the
  existing e2e/vitest contract didn't need touching.
- **Phase 1 (Fillet/Chamfer).** Reused `showValueHandle`/`hideValueHandle` (the same
  shared `DragHandle` instance extrude/offset-face already use) whenever
  `filletAxisSource !== "screen"` (a resolvable bisector/bbox direction); the degraded
  ("screen") tier keeps the old whole-viewport claim unchanged. Per explicit user
  decision, grabbing the handle is now REQUIRED in the non-degraded case — mirrors
  offset-face's `offsetDegraded` press-gating exactly, narrows "claims every press" to
  "claims presses on the handle," and click-away stays excluded for the degraded tier
  only, unchanged. Chip anchor moved from the raw picked-edge point to the handle's own
  base/tip pair, so it's leader-lined off the arrow instead of sitting on the edge.
  [Fillet|Chamfer] + the chamfer second leg moved behind a new `EdgeOpOverflow` (own
  file `EdgeOpChipControls.tsx`) — per user decision, NOT left inline, even though it's
  flipped more often than extrude's collapsed settings.
- **Phase 2 (Revolve).** New `revolveAxisPick` chip kind — text + ✕ only, anchored at
  the profile centroid — fills the axisPick phase's total silence (previously: zero
  chip, faint unlabeled candidate lines, a StatusBar string). A NEW screen-fixed,
  `pointer-events-none` top-anchored banner (`ViewportRoot.tsx`, `revolve-empty-hint`)
  covers the truly-nothing-selected state — the ONLY existing precedent for a
  viewport-space (not StatusBar, not world-anchored) hint in this codebase is the
  `chip==="cached"` pill, reused rather than inventing new visual language. Derived
  from existing reactive state only (`toolStore.modelTool==="revolve" &&
  toolChipStore.kind==="none"`) — no new controller-to-store plumbing needed, since
  BOTH new/existing revolve phases now publish a chip kind, leaving `"none"` true only
  during the genuine gap. Armed chip leader-lined off one axis-line endpoint
  (`revolveChipAxisFrom`). Boolean segments moved behind a new `RevolveOverflow`
  (`RevolveChipControls.tsx`); the Axis-reset button stays inline (primary action, per
  plan). Per explicit user decision: NO rotate-handle gizmo this wave — angle doesn't
  map cleanly onto `DragHandle`'s linear forward/twoWay model; the leader-lined chip +
  already-existing live lathe preview covers the gap for v1.
- **Phase 3 (Extrude).** Pure dedupe/regression pass: local `CHIP_AXIS_OFFSET_PX`
  removed in favor of the Phase-0 shared constant directly; the new leader-line code
  path verified against the ONE thing that could desync it (extrude's per-frame
  `moveChip`-driven arrow) via a dedicated `HtmlOverlayDriver` test rather than
  eyeballing it — see above.

WEBKIT-ONLY E2E FLAKE, FOUND AND FIXED WHILE VALIDATING FILLET'S NEW HANDLE: the FIRST
drag right after arming (no prior chip interaction to let a natural render happen) could
land a `mouse.down()` on a handle a JS-side hit-test had JUST confirmed hot, and
silently do nothing — reproduced directly: `hitExtrudeHandle` at the identical point
flips true→false→true across consecutive calls in WebKit headless specifically
(Chromium never showed it). A 300ms settle wait in the new `dragEdgeOpHandle` e2e helper
(`modelToolHelpers.ts`) fixes it — every OTHER handle-drag call site in this codebase
already has a prior interaction that incidentally buys the same margin, which is why
this never surfaced before. Verified with 20/20 repeat-each runs across both browsers
post-fix (0/10–9/10 failing before it, depending on the mitigation attempt).

GATE: `bunx tsc --noEmit` clean · `bun run build` green · hex-gate 0 on touched files ·
vitest **240 files / 4060 tests** (from 240/4051 pre-wave — +9: 4 leader-line + 1
live-move regression in `HtmlOverlayDriver.test.ts`, 4 in `ModelToolChips.test.tsx` for
the two new overflows/axisPick chip) · Playwright chromium+webkit, FULL suite (69 spec
files): **380/392 passed**. The 12 failures (`center-rect`, `ellipse` ×3,
`live-dim-line`, `sketch-reattach` — ×2 browsers) are ALL in sketch-drawing /
live-dimension specs, whose source (`LiveDimChips.tsx`, `liveDimStore.ts`,
`SketchController.ts`, `liveDimFrames.ts`, `liveDimension.ts`, `liveToolMachines.ts`,
`CornerCluster.tsx`) was mid-edit in this same working tree from a CONCURRENT session
before and during this wave (uncommitted, not touched by this diff) — same pattern the
EXTRUDE DIRECT MANIPULATION gate above flagged and preserved. Zero overlap with any
file this wave touched; every spec that exercises a file this wave DID touch
(`filletChamfer.spec.ts` 26/26, `revolve-{commit,preview,region}.spec.ts` 8/8 incl. 2
new guidance-banner cases, `extrude-{commit-gesture,boolean,draft,end-conditions,
multiselect}.spec.ts`, `offset-face.spec.ts`, `shell-preview.spec.ts`,
`boolean-preview.spec.ts`, `sketch-{on-face,fillet,hole-extrude,offset}.spec.ts`,
`history-inline-dimension.spec.ts`, `tree-visibility.spec.ts`) is green.
NOT RUN from here: Rust, ctest (untouched by this wave — no worker/backend changes).
NOT INVESTIGATED further: the 12 pre-existing sketch/live-dim failures above — outside
this wave's scope and file set, belongs to the concurrent session's own gate.

### Unresolved questions
- Dashed leader-line visual spec (color/dash pattern/min-max length) has no design
  token yet — engineering default used (`--color-border-strong`, 1px dashed, hidden
  under 4px), needs a design pass.
- Revolve's pre-axisPick copy ("Pick an axis line", "Select a sketch region to
  revolve") is a placeholder pending copy review.
- Whether `hitExtrudeHandle`/`showValueHandle`/etc. get renamed now that fillet is a
  third caller (e.g. `hitValueHandle`) — naming-only, not blocking.
- Revolve rotate-handle gizmo and Fillet's whole-viewport-vs-handle-required tradeoff
  were explicit user calls for v1; both are flagged in the plan as revisitable.


## PLATFORM REFACTOR — Milestones 1 + 2 (2026-08-08, plan `velvety-leaping-adleman.md`) — IN FLIGHT

Architecture-only refactor: modeling stops being synonymous with OneCAD and becomes the first built-in module on a Platform, and `.onecad` gains namespaced module state that survives a round trip without its owner installed. **No user-visible change, no modeling behavior change.** Out of scope: SDK package, test addon, addon manifest/loader/host, GitHub install, resource-store generalization, dynamic Tauri router, crate extraction, any file moves.

Laws: `docs/ARCHITECTURE.md` (normative) + `docs/adr/0001`–`0008` + the new CLAUDE.md § Architecture laws.

### Recorded baseline (W0, measured — not inherited from a doc)
- `bunx tsc --noEmit` was **RED** on `src/ipc/mockClient.import.test.ts:106` (`'snap' is possibly 'null'`, from concurrent commit `685efc2`). Fixed by narrowing, no cast. Now green.
- `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` against the **staged** sidecar (`worker/build/onecad-worker`, built 19:37, pre-VF-M5-gate): **1033 passed / 0 failed**. The VF-M5 red recorded in this file does NOT reproduce on that binary — consistent with the note that it needs a worker built from HEAD.
- Worker could NOT be rebuilt here: `scripts/build-worker.sh` aborts with "OCCT artifact metadata is absent" for `/opt/homebrew/opt/occt-8.0.1` (no `occt-build.json`; that prefix is not a `build-pinned-occt.sh` product). **Unrelated to this refactor — flagged, not worked around.** The VF-M5 FOLLOW-UP gate below therefore stays open and unverified from here.
- Frontend baseline at W0 close: vitest **213 files / 3720 tests** green; `cargo fmt --all --check` + workspace `clippy -D warnings` clean; `cargo test -p onecad-core --lib` 242/0.

### W0 — invariants captured ✅
- [x] Frozen behavior contracts in `src/test/contracts/` (+ a README stating they may not be edited to make a refactor pass): toolbar arrangement, the three keymap tables + cross-mode opt-out, editor mount order, inspector section order per state.
- [x] Four golden probes: `toolbarConfig.golden.test.ts`, `keymap.golden.test.ts` (full (key, shift, mode) resolution matrix against an independently-written oracle, not a call into the code under test), `editorMountOrder.golden.test.ts` (JSX scan; becomes a registry scan at W3 against the SAME contract), `InspectorPanel.golden.test.tsx`.
- [x] CORRECTED ASSUMPTION while writing them: the inspector's Constraints section is UNCONDITIONAL in sketch mode (label + "No constraints yet.") — the contract records shipped behavior, not the guess.
- [x] Rust `unknown_document_state_survives_open_modify_save` in `io/container.rs` — the W5 guarantee rehearsed on today's `Document.extra` lane: open → real modeling edit → save → reopen, foreign key byte-equal.
- [x] `docs/ARCHITECTURE.md`, `docs/adr/README.md` + ADR-0001…0008, CLAUDE.md § Architecture laws.

### W1 — platform core ✅ (pure addition, nothing wired)
- [x] `src/platform/ids.ts` — branded ids + reverse-DNS owner validation + namespace enforcement (an addon cannot construct an `onecad.*` contribution id).
- [x] `src/platform/registry.ts` — owner-scoped generic registry. Duplicate id ⇒ throw naming the holder; foreign namespace ⇒ throw; order is `(priority, insertion index)` with `group` as consumer metadata, NOT a sort key (group names sort alphabetically, which is never the intended visual order); snapshot reference is cached because `useSyncExternalStore` loops on a fresh array per call.
- [x] `src/platform/contributions.ts` — Command / Tool / Panel / Inspector / Viewport / Workspace contracts, domain-neutral (scopes are opaque strings so the platform never learns "model"/"sketch"). `ViewportContext.invalidate()` is the on-demand-render seam.
- [x] `src/platform/{slots,events,services,platform}.ts` — closed slot list, owned event subscriptions, service registry with a naming `require()`, module lifecycle with dependency topo-sort, cycle + missing-dependency rejection, and a failed activation that leaves nothing registered.
- [x] `src/platform/react/` — `PlatformProvider` (context, never a global singleton), `SlotHost` (renders no wrapper DOM — these are absolutely-positioned overlays and a wrapper would change stacking), `ContributionBoundary` (per-contribution isolation; the app-level boundary's full-screen fallback is the wrong shape for a 260px panel).
- [x] Tests 46/46: ownership, stale-handle safety, duplicate + namespace rejection, order independent of registration order, tie-break, notification, dependency order, cycle, failed-activation cleanup, scope teardown completeness, slot render order, error isolation.

### W2 — modeling owns its tools, commands and bindings ✅
- [x] `src/modules/modeling/tools.ts` + `bindings.ts` are now the SINGLE source of truth for the palette and the three key tables. `features/toolbar/toolbarConfig.ts` and `shortcuts/keymap.ts` DERIVE from them and keep their exported shapes, so every call site and test is untouched. Resolution rules (mode precedence, exact chord, cross-mode fallback + opt-out) stay in `keymap.ts` — the module owns WHICH keys exist, the shortcut layer owns HOW one resolves.
- [x] Descriptors are discriminated on `scope`, so a sketch-only tool placed in the model table is a compile error, and consumers narrow without a cast.
- [x] Separators are DERIVED from group boundaries rather than authored — `group` is consumer metadata, `priority` is the sort key.
- [x] Ids are SCOPE-QUALIFIED (`…tool.model.mirror` vs `…tool.sketch.mirror`): `select` and `mirror` exist in both unions and mean different things, so a flat map would have let one shadow the other.
- [x] `registryToolbar.ts` rebuilds the arrangement FROM the registry — the golden assertion runs against that, not against the table the registry was built from, or it would only prove the table equals itself.
- [x] Tool activation and command execution DELEGATE to the existing `activateTool` / `runAction`, so a registry-driven invocation and a toolbar click cannot diverge.
- [x] `ToolDefinition.shortcutLabel` added: Measure binds ⇧/ but is written "?", so glyph and chord are not derivable from each other.

### W3 — EditorShell + slot hosting ✅
- [x] `src/app/shell/EditorShell.tsx` renders permanent structure + one `SlotHost` per region; the 19 concrete imports are gone. `EditorScreen.tsx` is now a one-line bridge so `App.tsx`'s code-split specifier and `StartScreen`'s idle prefetch keep working.
- [x] Contributions register on EDITOR MOUNT, not at bootstrap: the editor tree is a deliberate code-split chunk, and hoisting those imports into the startup bundle to satisfy an architectural preference would make the start screen pay for the editor. That needed `platform.createScope(owner)` (independent child scope) and a fix to scope teardown — `dispose()` no longer sweeps the whole owner, which would have let the editor's scope tear down the module's bootstrap registrations.
- [x] Panel ids live in `panelIds.ts`, split from the files that import components, so a workspace definition can name a panel without dragging the editor chunk in.
- [x] TWO NEW SLOTS, deliberately: `viewport.chrome` (controls anchored to the viewport frame — nav pill, corner cluster; they sit above the docked panels, unlike scene-tracking overlays) and `shell.notification` (banners). Without them the frozen mount order could not be reproduced with contiguous slot regions.
- [x] The mount-order probe was REPLACED (JSX scan → registry scan) against the SAME frozen contract, plus two new checks: every registered panel lands in a region the shell actually renders, and re-registration after teardown is collision-free (StrictMode double-invokes).

### W4 — default workspace + composition root ✅
- [x] `src/app/bootstrap.ts` — `bootstrapOneCAD()` creates the Platform, registers `onecad.shell` + `onecad.modeling`, initializes in dependency order. It lives in `app/` and NOT in `platform/`: the composition root is the only place allowed to know both sides.
- [x] `App.tsx` builds it in a state initializer and wraps the tree in `PlatformProvider` — no global singleton, and every existing test that renders `<App/>` keeps working with no setup.
- [x] `platform.initializeSync()` added because the React root must have a Platform on its FIRST render; it throws if a module's `activate` is async, so the restriction is visible at startup rather than as a half-built registry.
- [x] `onecad.shell.workspace.design` reproduces the current layout declaratively. NAMING DEVIATION recorded: the spec sketches `onecad.workspace.design`, but a contribution id must sit under its owner's namespace, and this workspace is owned by `onecad.shell` because it composes several modules. No workspace switcher in the UI.

### W5 — Rust module-owned document state ✅
- [x] `onecad_core::document::modules` — `ModuleId` (reverse-DNS, validated on AUTHORING only), `ModuleState { schemaVersion, payload }`, `ModuleStateTable`. Deserialization is deliberately permissive: refusing an id a stricter build dislikes would destroy exactly the data preservation exists to protect.
- [x] `Document.modules` + the `DocumentData` mirror, `skip_serializing_if` empty — pinned by test that a document without module state writes NO `"modules"` key, in the document and in the manifest. No container-version bump, no user-document migration.
- [x] `Manifest.modules` descriptor table, DERIVED from the document at save so the two can never disagree — this is what makes "this project uses an addon you do not have" answerable without decoding a payload.
- [x] `EditCommand::SetModuleState` + `Inverse::RestoreModuleState`, dirty floor `None` (no timeline step can consume state the platform cannot interpret). Programmatic writes therefore use the SAME transaction path as user edits.
- [x] Proofs: unknown-module state byte-equal across open → real modeling edit → save → reopen; a module id this build would refuse still round-trips; undo restores the PRIOR slice rather than merely deleting the new one; clear-then-undo restores.

### W6 — wire + missing-module reporting ✅
- [x] `ModuleStateDto` / `DocumentModuleDto`; three typed Tauri commands (`get_module_state`, `set_module_state`, `list_document_modules`). NO dynamic router — spec §97's `platform_invoke` is deferred to the addon-host effort, where it will have a consumer.
- [x] `CadClient` gains three append-only methods; `tauriClient` and `mockClient` both implement them, so the whole persistence lane is exercisable with no backend. `set_module_state` refuses a `schemaVersion` without a payload rather than silently treating it as a clear.
- [x] `src/platform/documentState.ts` — the service binds the module id at construction, so a module cannot address another module's slice by accident. `missingModules()` reports, never blocks.

### W7 — enforcement + gate ✅
- [x] `src/platform/architecture.test.ts` scans the real import graph: Platform must not import `@/features`, `@/tools`, `@/modules`, `@/stores`, `@/viewport`, `@/app`; modules must not import the shell or deep-path into the platform. Carries a POSITIVE CONTROL (an edge that really exists) because every other assertion expects an empty list — which is also what a broken scanner returns.

### Flagged seams (carried forward, not fixed here)
- `zoomFit` / `home` are registered as MODELING commands because that is where their bindings live today; they are really view-navigation and belong to the platform once a selection/viewport service exists. — **P2 W11.**
- ~~The toolbar component still renders from the derived `MODEL_TOOLS`/`SKETCH_TOOLS` arrays~~ — **CLOSED by P2 W8.**
- Inspector sections, tree nodes and viewport layers are NOT yet contributions — `InspectorContribution` / `ViewportContribution` / `TreeProvider` exist as contracts with no producers. — **P2 W9/W10/W12.**
- **DECISION OWED before P3 — is the toolbar extensible?** W8 made it read the registry live, but it stays a MODELING PROJECTION: `registryToolbar.ts:52` skips any tool id absent from modeling's reverse map, and `FloatingToolbar` activates through `activateTool` (the store `Tool` union) rather than `ToolDefinition.activate`. So an addon could register a valid tool and be silently absent from the toolbar. Opening it needs applicability to become a contribution concern (`toolApplicability.ts` is typed on the modeling `Tool` union) and `ToolEntry` to give way to `ToolDefinition` — which changes the currency the frozen toolbar contract is written in. P3 freezes the SDK surface over whichever answer we pick, so pick it first.
- Module state is stored in `document.json` (ADR-0004); moving to `modules/<id>/state.json` later is a container-format change.
- **CORRECTED (was mis-reported as a blocker):** `scripts/build-worker.sh` failed with "OCCT artifact metadata is absent" only because it was pointed at `/opt/homebrew/opt/occt-8.0.1`, a plain Homebrew install. The PINNED prefix `~/.onecad-occt/8.0.1` carries `share/onecad/occt-build.json` and configures fine. Nothing is blocked. CONSEQUENCE: this session's worker-backed 1045/0 ran against `worker/build/onecad-worker` (19:37), which HANDOFF.md § VF-M5 identifies as a **stale pre-gate build** — so it did not exercise `069bb48`'s worker changes. Re-run against a HEAD build before trusting that number for the worker lane (see NEXT SESSION P1).

## P2.5 — GENERIC EXTENSION SEMANTICS BEFORE THE SDK (2026-08-09, plan `act-as-senior-software-encapsulated-balloon.md`) — IN FLIGHT

P3 does not start over a surface where three registries are generic at the contract and modeling-specific at the runtime: an addon tool is silently dropped by the toolbar, `defaultShortcut` never reaches the keyboard, and `TreeNode.id` collides across providers. Sequence: WP0 baseline → WP1 tool runtime → WP2 shortcuts → WP3 tree + settings → WP4 module lifecycle → WP5 `@onecad/sdk` → WP6 reference addon → WP7 enforcement. Workspace runtime is deliberately DEFERRED past the reference addon (it is not in the SDK's tool/tree/shortcut currency).

### WP0 — green baseline ✅ (2026-08-09)
- [x] VF-M5 flagship regression closed — see § VF-M5 FOLLOW-UP below. **A RESIDUAL remains open** (§ VF-M5 RESIDUAL): the gate is off, and the F12 fallback lane it should protect is real. Re-arming it is a protocol change.
- [x] The four Playwright failures were stale specs, not an open UX question. Both removals were already decided AND pinned elsewhere (`TitleBar.test.tsx:67-69` asserts the Appearance toggle is absent; the unified start-screen `Import…` is recorded in § Wave 4), so the specs moved to the shipped entry points rather than the UI moving back.
      - `theme.spec` — the two title-bar-toggle tests became a Settings-modal pair: the modal drives the theme AND the engine (the half CSS cannot fake), and modal ↔ popover stay in step. Same intent, the control that exists.
      - `project-import` / `step-import` — one `Import…` button now routes on extension, so a `.onecad` pick is an OPEN (asserted through `appStore`, since the mock's `openDocument` pushes no projection) and the append lane stays covered by the in-editor File ▸ Import Project… test. `mockClient.importFileDialog()` gained `?mockimport=step` (same dev-only URL-flag pattern as `?vpdemo`) so the router's STEP half is still reachable from a browser lane.
- [x] Gate: `bunx tsc --noEmit` clean · vitest **224 files / 3837** · worker ctest **107/107** · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1045 passed / 0 failed** across 72 targets (worker built from HEAD against `~/.onecad-occt/8.0.1`, staged) · `cargo fmt --all --check` · workspace clippy `-D warnings` · full Playwright **387 passed / 3 failed**.
- [x] **The 3 remaining Playwright failures are PRE-EXISTING and are not this work.** All three are `boolean-preview.spec.ts` (chromium :276, webkit :227 + :276). Verified by bisect in a throwaway worktree: the same test fails on `4145f3f` (before P2) and on `cf75bda` (before the Platform refactor entirely). Symptom, from the failing run's `fe-logs`: after the sketch is re-shown between the two extrudes, the click on the circle region does not select it — Extrude then arms in multi-select ("Select regions to extrude") instead of the depth drag. `sketchHitTestReady` already passed, so the sketch WAS hit-testable; the suspect is the click racing the visibility commit's projection push. Machine-dependent: the 2026-08-08 full run had it green.
- [ ] **Manual `tauri dev` smoke still owed** (open project → extrude → fillet → undo → save → reopen). Unchanged from P0 below: the one Definition-of-Done item with no evidence.

### WP1 — generic tool runtime ✅ (`4b8ec0d`, ADR-0010)
- [x] `ToolHost` on the platform owns the active id + the activate/deactivate handshake; `FloatingToolbar` renders `ToolDefinition`s directly (title/icon/shortcutLabel, `group` boundary ⇒ separator, `priority` ⇒ order) and activates through the host. `toolFromId` no longer stands between a registration and the screen.
- [x] Modeling stays authoritative and REPORTS: `toolStore` changes are mirrored into the host, so AUTO-MODE, the Esc ladder and self-arming controllers cannot leave the highlight disagreeing with the store. `deactivate()` runs only on a CROSS-owner swap.
- [x] `canActivate` returns `ToolAvailability` (the toolbar has always explained WHY a tool is grayed) and gained `subscribe(onChange)` — the same staleness rule W14 gave `TreeProvider`. Modeling backs every tool with ONE shared emitter over two stores.
- [x] A tool with NO scopes now appears everywhere (`CommandDefinition.scopes`'s documented "empty ⇒ always"); the old projection dropped that case, which made a zero-knowledge contribution impossible. Icons resolve defensively — `def.icon` is an open string and the old cast crashed inside `Icon`.
- [x] **Real regression caught by e2e**: `mirror` is in BOTH tool unions, so routing every tool through the model applicability matrix disabled the SKETCH Mirror tool with nothing selected. Applicability is model-scope only, with a vitest case pinning it.
- [x] `toolbarFromRegistry` demoted to the golden probe's projection (the contract is written in `ToolEntry` and may not be edited); new case pins that a FOREIGN registration leaves modeling's arrangement byte-identical.

### WP2 — registered shortcuts reach the keyboard ✅ (`eb59b6d`, ADR-0011)
- [x] `defaultShortcut` was write-only — three producers, no reader. `platform.shortcuts` resolves chords over both registries; `useShortcuts` asks modifier chords → `resolveBinding` → the registry, in that order, so a contribution CANNOT shadow a built-in and the golden keymap oracle stays byte-identical.
- [x] Conflicts never resolve by load order: scope-specific → explicit priority → built-in over addon → otherwise the chord fires NOTHING and is reported. A keystroke does not bypass `canExecute`. The ⌘-chords stay hardcoded and are explicitly not addon-reachable in v1.

### WP3 — tree public surface + settings ✅ (`7dee177`, ADR-0012)
- [x] Rows are addressed by `(providerId, nodeId)`. Ids stay provider-local: demanding globally unique ids pushes a naming burden onto every contributor to fix a host bug.
- [x] Modeling's provider implements `subscribe`; the host dropped its five modeling store subscriptions and now watches only providers.
- [x] `TreeNodeAction` is command-backed with a declarative `confirm`, so a row action is reachable from a palette rather than living inside one popover. Delete-datum/delete-sketch became modeling commands; the panel lost its `kind` branches for them. **Reattach stays the one flagged modeling-specific branch** — it needs a fact the node does not carry AND a second anchored popover.
- [x] Settings moved from `ModelTreePanel` to the shell's `StatusBar`. `settingsStore` ownership is untouched and out of scope.

### WP4 — module lifecycle ✅ (`bbe1ceb`)
- [x] `ModuleDefinition.deactivate` was declared and called from NOWHERE. It now runs on `disposeOwner` and `platform.dispose()`, before the registrations go, in reverse initialization order. Not for a failed activation, not for a short-lived child scope, and a throw does not abort disposal. New `deactivating` state; `platform.dispose()` also sweeps the registries per owner (it only walked tracked scopes).

### WP5/WP6/WP7 — SDK boundary + reference addon + enforcement (ADR-0013)
- [x] `@onecad/sdk` → `src/sdk/` (tsconfig + vite alias; not published — a real package buys nothing until something outside this repo builds against it). Re-exports ids + checked constructors, `Disposable`, the contribution contracts, `Slots`, `SelectionRef`, document-state types and `ExtensionContext`.
- [x] `ExtensionContext` replaces `ModuleScope` for addons: same registrations, no `platform`, no `createScope`, and `document.state` pre-bound to the addon's namespace. Narrowed BY CONSTRUCTION, not by cast — a cast leaves `platform` present at runtime. `createExtensionContext`/`registerExtension` are host-side and absent from the SDK.
- [x] `src/addons/reference/` contributes command + tool + panel + inspector section + tree section with a command-backed action + viewport layer + workspace + its own document namespace, over its OWN domain type (`com.onecadtest.reference.item`), importing `@onecad/sdk` and `react` only. Tests drive the REAL toolbar, tree, shortcut lane and viewport host.
- [x] **Deliberately NOT registered in `bootstrap.ts`** — shipping a fake "Widgets" panel into the product UI to prove an architectural point is a worse trade than proving it in tests. It becomes the addon loader's first package when the loader lands.
- [x] `architecture.test.ts` enforces both directions (addon ⇏ application, SDK ⇏ application implementation), plus a runtime-surface snapshot of the SDK barrel and an explicit "the SDK does not export the host" check. Both new rules carry the positive-control pattern.

## MODULAR-PLATFORM UI (2026-08-09) — design turn 2 implemented, FE gate PASSED

Source: `claude.ai/design/p/f68f85fa` → `OneCAD UI Explorations.dc.html`, turn 2
("Modular platform — workspaces, extensions, palette, missing add-ons"), options
2a–2d. Read via the `claude_design` MCP; nothing from the design project is
vendored into the repo.

**This is the first DELIBERATE user-visible change since the platform refactor.**
`src/test/contracts/shellContract.ts` was amended for it (six new shell
contributions, nothing existing moved) — recorded here because the contracts
README requires an explicit decision, never a refactor-driven edit.

### What is REAL (bound to something that exists)
- [x] **Workspace selector** in the title bar, projected off `platform.workspaces`. Four shell workspaces registered (Design + Simulation/Drawing/Visualization); an add-on's workspace lands in its own ADD-ONS group automatically, and an empty group renders nothing.
- [x] **Workspace panel filtering**: `SlotHost` takes a `filter` predicate; `modules/shell/workspaceLayout.ts` resolves it. Rule is CONSERVATIVE — a panel is hidden only on an explicit `visible:false` placement or a user override. "Unlisted ⇒ hidden" would silently swallow every tool overlay.
- [x] **⌘K command palette**, projected off `commands` + `tools` + `workspaces` registries. No palette registry: anything registered is findable, nothing opts in. Disabled entries stay visible with their owner's `reason`.
- [x] **Missing-extension banner + details dialog + "Unavailable data" explorer section**, all from the REAL `listDocumentModules()` ⋂ `platform.moduleIds()` diff. This is ADR-0005 made visible.
- [x] **Extensions manager** — Installed tab lists real modules with their lifecycle state and a count of what each contributed.
- [x] **Viewport layers menu** on the NavPill's previously-dead "View presets" button. New `ViewportEngine.setLayerVisible()` toggles `bodiesRoot`/`sketchRoot`/`contributionsRoot`; grid reuses the existing `viewportStore` flag (one piece of state, two entry points).
- [x] **Collapsible explorer sections** + `TreeNode.meta` / `TreeNode.problem` / `TreeSection.defaultCollapsed` / `TreeSection.emptyNote` (additive platform contract).
- [x] **Customize workspace sheet** — panels/tool groups/layers all DERIVED from the registries, never enumerated. Tool-group hiding filters `FloatingToolbar` only; the tool stays registered and reachable by shortcut and palette.
- [x] **Start-screen Extensions entry** (2d), with Settings below the rule — configuration, not project content.
- [x] **Title-bar buttons unified** behind one `TitleBarButton` (28px · 6px radius · ghost). Home/File/workspace/⌘K were three different hand-rolled geometries in a row.
- [x] **File ▸ Rename…** (moved out of the title bar at user request — a title inside the drag region is a rename you trigger by accident).

### What is UI-ONLY, and why (no backend exists)
- **Extensions Browse / Updates** render an explicit "no registry configured" empty state. Deliberately NOT a mock catalog: dead Install buttons teach users the feature is broken, and fake listings outlive the mock. There is no addon loader (`platform/extension.ts` closing note).
- **Enable/disable + uninstall** are absent rather than inert — a half-unloaded built-in module is not a state the app is designed to run in.
- **Background-tasks chip** (`tasksStore`, status bar) has a real `begin/setProgress/end` API and renders an INDETERMINATE bar when `progress` is undefined. Nothing calls it yet: regen is request/response over OCW1 with no progress frames. **FLAGGED** — first producer should be regen once the protocol can report progress.
- **Simulation / Drawing / Visualization** are real registered arrangements with no module behind them: each switches modeling's tool surfaces OFF and shows a `WorkspacePlaceholder` naming what is missing. Leaving Extrude on screen under "Drawing" would have been worse.
- **Rename is display-only** (`DocumentState.displayTitle`). There is no `RenameDocument` in the protocol — the Rust runtime derives the title from the file it loaded, and `renameRecentProject` needs a path the editor does not hold. The dialog SAYS the file is untouched. **FLAGGED — next backend tranche.**

### Flagged seams
- `CommandPalette` imports `useModelingToolContext` from `@/modules/modeling`. Shell UI reaching a specific module: the palette needs a real selection context to evaluate `canExecute`, and no neutral selection SERVICE exists yet. Precedent: `ViewportRoot` → `datumViewport`. Fix is a module-published context service, not a cast.
- `paletteStore`/`workspaceStore`/`layersStore` overrides are session-only — nothing persists yet.
- `Popover` gained `top-start`, which measures the panel after mount (one frame at the seeded position, same as every other placement).

### Gate (2026-08-09)
- `bunx tsc --noEmit` clean · `bun run build` green · vitest **236 files / 3986 tests** green (was 227/3919; +9 files) · hex-token grep empty · Playwright **387 passed / 3 failed**.
- The 3 e2e failures are pre-existing, verified not assumed: `sketch-reattach` (chromium) passes 3/3 isolated (load flake); both webkit `boolean-preview` failures reproduce in a clean worktree at HEAD `352ddd1` — the same failure already bisected to before the Platform refactor.
- Rust/ctest untouched by this wave.
- REMAINING: manual Mac smoke (`bun run tauri dev`) — the workspace filter and the new `shell.overlay` region are layout changes no jsdom test can prove.

## NEXT SESSION — three work packages (2026-08-08, handoff)

Read `HANDOFF.md` § Session 4 first. P1 is another program's open gate; P2 finishes what the platform refactor left half-done; P3 is the next real tranche. **Do P2 before P3** — an SDK frozen over a half-converted surface freezes the wrong shape.

### P0 — verify before touching anything (30 min)
- [ ] Manual smoke, the only Definition-of-Done item with no evidence: `bun run tauri dev` → open an existing project, extrude, fillet, undo, save, reopen. Chrome and layout must look identical to before `4145f3f`. Any visual difference is a slot-order or z-index regression — `src/test/contracts/shellContract.ts` is the contract it violated.
- [ ] Rebuild the sidecar against the PINNED prefix and re-run the worker lane, so the numbers describe HEAD:
      ```bash
      ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" \
      ONECAD_WORKER_BUILD_DIR="$PWD/worker/build-pinned" scripts/build-worker.sh Release
      ctest --test-dir worker/build-pinned --output-on-failure          # expect 107/107
      cd src-tauri && ONECAD_WORKER_PATH=$PWD/../worker/build-pinned/onecad-worker \
        ONECAD_REQUIRE_WORKER=1 cargo test --workspace
      ```
      EXPECTED: `topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean` FAILS. That is P1, not a platform-refactor regression — it is the VF-M5 gate that `069bb48` opened.

### P1 — close the VF-M5 gate regression — CLOSED (2026-08-09, resolution (a))
Full diagnosis already existed in `HANDOFF.md` § "VF-M5 gate regression" and § VF-M5 FOLLOW-UP above.
- [x] The discriminator was wrong. Worker `PlanExecutor` used `job.partition.size() == 0` to mean "from-zero replay", but the RegenPlanner emits full-replay-from-0 plans for EVERY regen and `Session::fence_and_clone` clones an empty base for those (D5), so it is ALWAYS true — the gate degenerated to `edited_from.is_some()` and the flagship edit lane (`ToEnd { from: 1 }`) was falsely treated as a replay, disabling the anchor-exact carve-out and flagging `NeedsRepair`.
- [x] **Resolution (a) taken.** `job.from_zero_replay = false` in `PlanExecutor.cpp`, with the derivation and the re-enable condition (a genuine restored-basis signal, never partition emptiness) recorded at the assignment. The field is re-documented in `ScratchJob.h` and `Ladder.h` as "replay onto a RESTORED basis", which is what it always meant; V1 has no restore (SaveCheckpoint/RestoreCheckpoint UNSUPPORTED, `baseCheckpoint` never read), so the hazard cannot occur. The ladder behaviour stays wired and stays covered by `worker/tests/test_wp6_ladder.cpp`, which builds `LadderEditContext` directly.
- [x] Red-first, verified on a worker built from HEAD against the pinned prefix: `topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean` reproduced RED (`needsRepair` 1, expected 0) BEFORE the change and passes after. The gate was not weakened.
- [x] The 4 Playwright failures were STALE SPECS, not the platform refactor and not an open UX question — both removals are already pinned elsewhere: `TitleBar.test.tsx:67-69` asserts the Appearance toggle is absent (it lives in `DisplayModePopover` + `SettingsModal`), and `TODO.md` § Wave 4 records the unified start-screen `Import…` button with the sidebar duplicates removed. Specs rewritten to the shipped entry points; `mockClient.importFileDialog()` gained `?mockimport=step` so the extension router's STEP half stays reachable from the browser lane.

### P2 — finish Milestone 1 (plan `~/.claude/plans/resume-parallel-muffin.md`, Codex-reviewed terra/high → revise, 3 blockers + 4 high all folded)
- [x] **W8 — toolbar reads the registry live.** `FloatingToolbar` now takes `usePlatform()` + `useRegistryEntries(platform.tools)` and memoizes `toolbarFromRegistry` off that snapshot — the projection allocates a fresh array per call and would loop `useSyncExternalStore` as the snapshot itself. `toolbarConfig.ts` was NOT deleted as first planned: `src/test/contracts/toolbarContract.ts:6` imports `ToolEntry` from it, and rewriting that import edits a frozen contract file. It shrank to the entry shape (`ToolItem`/`ToolSeparator`/`ToolEntry`/`isSeparator`) and is now the contract's type anchor; the runtime derivation (`MODEL_TOOLS`/`SKETCH_TOOLS`/`toolsForMode`/`toolEntriesFor`) is gone. Golden probe moved to `modules/modeling/registryToolbar.golden.test.ts` (probe follows the mechanism; contract does not move) and gained a live-registry case: disposing modeling's scope empties the toolbar. New `src/test/renderWithPlatform.tsx` boots a REAL platform (not a stub registry, which would let a registration bug pass) for `FloatingToolbar.test.tsx` + `SketchEntry.test.tsx`, which rendered bare.
- [x] **W9 — inspector sections become contributions.** The eight labelled sections (Appearance body/face, Dimensions, History selection/feature, Constraints hint/list, Depends on) moved out of `InspectorPanel.tsx` into `src/features/inspector/sections.tsx` and register through `src/modules/modeling/inspectorSections.ts`. The panel keeps the frame, the branch tree, headings, the DOF card and the trailing hints; EMPTY and REPAIR host no sections at all, which is why the contract is untouched by leaving them chrome. **ONE platform change:** `InspectorContext` gained `scopes: readonly string[]` — opaque tokens, the same currency as `ToolDefinition.scopes` — because three sections gate on mode and an `InspectorContext.mode` would teach the platform a modeling concept. `SlotHost` could not be reused (it is bound to `platform.panels`), so `InspectorSectionHost` is modeling-owned and subscribes to `selectionStore` + `toolStore` as well as the registry: `canRender` is a plain predicate and a registry-only host would keep rendering the last selection's sections. Sections render their OWN `SectionLabel` — the label is what the order contract counts, so a section with nothing to show must render nothing at all. `subElement` carries the ElementId only, never the topoKey, or an unpromoted face would look promoted to every `canRender`. **Probe change, contract untouched:** `InspectorPanel.golden.test.tsx`, `InspectorPanel.test.tsx` and `RepairPanel.test.tsx` gained a `renderWithPlatform` harness (the panel needs a Platform now); every assertion, `sectionsOf` and the `tracking-[0.07em]` marker are byte-identical, and `git diff --exit-code src/test/contracts/` is clean. New `InspectorSectionHost.test.tsx` (7) covers the staleness cases a registry-only host would pass: selection change, mode change, register-after-mount, dispose, priority-over-registration order, throwing-section isolation, and the EntityRef→SelectionRef mapping including the topoKey guard.
- [x] **W10 — TreeProvider beneath the existing rendering.** `TreeProvider` did not exist anywhere (not in `contributions.ts`, not in the ADRs — only the two unused `Slots.Tree*` constants), so the shape was designed here: `TreeNode { id, label, icon, kind, selected, dimmed?, visible?, select(), activate?, toggleVisible?, rename? }` + `TreeSection` + `TreeProvider.sections()`, with a `platform.tree` registry mirroring `platform.inspector`. **`select()` is separate from `activate()`** because click and double-click mean different things on every shipped row (a sketch selects on click, re-opens on double-click); collapsing them would either change behavior or push modeling's selection semantics back into the generic panel. **An ABSENT capability is how a row says it lacks one** — a datum supplies no `toggleVisible` and no `rename`, so the eye and the Rename item disappear on their own; the menu now branches on the node's capabilities rather than on `menu.kind`. `sections()` READS the stores (`getState()`) and the panel keeps the subscriptions purely to re-render — that is what lets the contract stay a plain function instead of a hook, and it is commented as load-bearing in both files. The visible tree is unchanged. **Flagged seam:** the context menu still lives in the panel and its datum/sketch branches still switch on `node.kind`; Reattach needs its own anchored popover and two-click delete is panel state, so a provider-supplied `TreeNodeAction[]` is the next step. P3's addon therefore gets rows but not menu items.
- [x] **W13 — ADR-0007 corrected.** It documented iteration order as `(group, priority, insertion index)`; the runtime sorts `(priority, insertion)` (`registry.ts:73-78`) and `group` is consumer metadata. Sorting by group first would order the UI by how a group is SPELLED. Text fixed and pinned by a new `registry.test.ts` case whose groups are spelled so that alphabetical order is the reverse of the intended one.
- [x] **W11 — view navigation off modeling.** `zoomFit`/`home` are `onecad.shell.command.*` now, with the two chords in a new `modules/shell/bindings.ts` (`keymap.ts` concatenates modeling's global table then the shell's, which reproduces the frozen `GLOBAL_KEYS` order exactly). `Escape`/`Enter` stay modeling — global in reach, modeling in meaning — and `isolate` stays modeling because it masks BODIES. **The trap this wave was really about:** implementing the service over `engineBridge.fitView()` — the obvious reading of "zoom to fit" — silently turns ⇧F from *frame the selection* into *frame everything*, and neither the type system nor the keymap contract notices. `modules/shell/viewportNavigation.ts` therefore delegates to `viewportStore.zoomFit/homeView`, where the selected-body semantics live, and `runAction` calls THAT object rather than the store, so the command and the keystroke are one call. Registered in `shell/module.ts` (bootstrap), NOT `register.ts` — that file imports the chrome components and would drag them into the startup bundle. New `viewportNavigation.test.ts` (8) pins the selection matrix through both entry points, the service lookup, the no-op-before-engine case, and the chords.
- [ ] **Deferred, flagged:** a registry-driven keymap. `useShortcuts` installs a window keydown handler outside React, so routing resolution through the command registry is its own change; `keymap.ts` does a static two-owner merge for now.
- [x] **W12 — ViewportContribution + the full DatumLayer port.** `ViewportContext` carried only `invalidate()`, which was enough to declare the contract and nothing else; the first real producer showed what a layer actually needs. It now also carries `root`, `createLabel`, `onFrame`, `onThemeChange`, `raycastFromClient` and `registerSecondaryHover`, with type-only `three` imports recorded in **ADR-0009** (the forbidden edge is `@/viewport`, not the rendering library, and `slots.ts` already called in-scene contributions Three.js objects). `createLabel` is a façade rather than the `HtmlOverlayDriver` itself — handing out the driver would put a renderer internal in the public surface and let a contribution move labels it does not own.
  - **Six methods deleted from `ViewportEngine`** (`syncDatums`, `datumHitTest`, `setDatumHover`, `setDatumSelected`, `setDatumGhost`, `isDatumGhostVisible`) plus the layer field. A datum is a MODELING record; a viewport that knows what one is cannot host a module that has none. Five caller sites rerouted through `modules/modeling/datumViewport` (`datumSync`, `SketchController` ×3, `ModelToolController` ×3, `ViewportRoot`), and `Picker`'s hardcoded datum branch became `registerSecondaryHover`.
  - **The host is `ContributionHost.ts`, and reconciles — it does not attach once.** Contributions register on editor mount while `engine.init()` is still awaiting the renderer, so a one-shot attach drops whichever side loses that race; `ViewportRoot` hands the engine the REGISTRY and it stays subscribed. Teardown is reverse-order. Failure policy is explicit: a throwing `attach` is rolled back (labels included) and skipped, a throwing frame/theme/hover callback detaches that contribution — one bad layer may not kill a frame.
  - **Ordering is pinned by test**, not by luck: frame callbacks run at the ladder position the datum layer used to occupy, and secondary hover keeps built-ins first, then contributions in registry order, first non-null wins.
  - **`themeRefresh.test.ts` gained its first DatumLayer case, negative-checked** (neuter `refreshColors` ⇒ red, restore ⇒ green). It never had one, and the silent-failure invariant is now *harder* to satisfy for a contribution: it is not in `applyTheme()`'s list at all, so a forgotten `onThemeChange` is invisible to the engine. `viewport/engine/README.md` § Theming says so.
  - Probes followed the mechanism: the e2e datum surface moved from `window.__vpEngine.datumHitTest`/`.isDatumGhostVisible` to `window.__datumVisuals` (DEV-only), published by the contribution.
- [x] **W14 — hardening pass** (Codex implementation review, `gpt-5.6-sol`/high over the committed W8–W12 range; 1 high + 5 medium + 1 low, all verified in source and all fixed, each with a negative-checked regression test).
  - **HIGH — the late-attach hole.** `DatumSync.attach()` pushed the projection ONCE. The layer is a contribution now, so it attaches on the viewport host's schedule: whenever it landed second, that sweep went into a `null` and the datums stayed invisible until an unrelated store change re-pushed them. `datumViewport` gained `onDatumVisualsChanged` (fires immediately with the current value), and `DatumSync` replays the full sweep + selection through it.
  - **Removal did not schedule a frame.** `reconcile()` only invalidated on ADD, so a disposed contribution's objects left the scene graph while the on-demand renderer drew nothing — the last framebuffer kept showing them. Now any change invalidates, including `setRegistry(null)` and a guard-triggered detach.
  - **The label cleanup skipped every second label.** Each `dispose()` splices itself out of the array the host was iterating. Snapshot + `finally`, so a contribution's own throwing disposer can no longer strand host-owned overlay items either. The first version of this test did NOT catch it (the fake tidied up after itself) — rewritten so the host's loop is what runs.
  - **A crashed contribution came straight back.** `guard()` detached it but its registration remained, so the next unrelated register/dispose re-attached it and it threw again. Failed ids are held back until their registration goes away — a re-register is still a genuine second chance, pinned by test.
  - **`canRender` ran outside its own `ContributionBoundary`** (admission is decided before there are children), so a throwing predicate escaped the section and would have taken the whole inspector down. Guarded; a predicate that cannot decide does not admit.
  - **`TreeProvider` gained optional `subscribe`.** A provider backed by data the panel does not watch — i.e. any non-modeling one — could never update its rows after mount. Optional only because modeling's is backed by stores the host already watches for its own reasons; the contract says so.
  - `docs/DEBUGGING.md:243` still told developers to call the deleted `__vpEngine.datumHitTest`; now documents `window.__datumVisuals`.
  - Gate: tsc clean · vitest **224 files / 3837** · contracts byte-identical · hex gate empty · Playwright chromium datum/sketch-on-face/acceptance **14/14**.

### P3 — Milestone 3: SDK boundary + bundled test addon (the next real tranche)
Validates the boundary on OneCAD's own code before any third party sees it (spec §197).
- [ ] `@onecad/sdk` — a package that re-exports ONLY the public surface: ids, contribution contracts, `ModuleScope`, the document-state service, selection/event types. It must NOT re-export `createPlatform`, the registries' internals, or anything from `@/features`/`@/tools`.
- [ ] A bundled test addon (an architectural fixture, NOT a product feature) that registers a command, a panel, an inspector section, a viewport contribution and its own document namespace — importing `@onecad/sdk` and nothing else.
- [ ] Extend `src/platform/architecture.test.ts`: the addon's imports must resolve only to the SDK. That test already carries a positive control; keep that pattern.
- [ ] Prerequisite from P2: the addon cannot contribute an inspector section until inspector sections have a host.

## VF-M5+VF-M6 DEFECT FIXES + IMPORT PROJECT (2026-08-08) — GATE PASSED

Defect fixes from the review round (worker ladder + import blob lifecycle), then the "Import Project" feature: append another `.onecad` document's timeline to the open document. Assembly/joints explicitly out of scope; static import now, live/XREF later.

### VF-M5 — from-0 replay rebases onto stale WORLD anchors
- [x] Root cause: on a from-0 replay an inherited edit ("ALL" buckets, whose refs were animated to the source model) resolves WORLD anchors through the post-edit scene; the replayed model telerecorders scored anchor-exact, which vetoed their legit `NeedsRepair` — replay silently misassigned. The far-edge blend captured it and anchor-rebased nothing.
- [x] Fix (attempt, RE-OPENED — see VF-M5 FOLLOW-UP below): from-0 replay now runs with edit-context `from_zero_replay`, which disables the anchor-exact carve-out during that loop. The checkpoint path (from-zero carrying the user's original edit) still binds AutoBind; the from-ZERO (checkpoint-recreated) replay surfaces the same case as `NeedsRepair` + "ambiguous" so a follow-up repair/step runs as authored. **The gate as built regresses the flagship real-worker edit lane and must be reworked — V1 has no checkpoint restore, so the carve-out should stay ON until a genuine restore basis is plumbed.**
- [x] Plumbed `from_zero_replay` through `LadderEditContext` → `ScratchJob` → `PlanExecutor` → into the four edit ops (`Shell/Hole/Fillet-Chamfer/Extrude`); `job.partition.size()==0` is the from-zero gate — **WRONG as a discriminator, see FOLLOW-UP**
- [x] Docs: gap-closed note in `protocol/IRR/SCHEMA.md` + `docs/qa/…` (records the accepted residual + the gap this runs through).

### VF-M6 — import-blob lifecycle (3 defects)
- [x] `materialize()`: now checks the staged path is actually a FILE (a collided digest that resolved to a directory would previously arm the job against a wall, then corrupt the model). Sweeper now also kills stale per-workspace... clean.
- [x] `sweep_stale_workspaces()`: sweeping is now PID-aware — a workspace survives while its importing process is alive (unix: `libc::kill(pid, 0)`; non-unix lanes disable sweeping rather than guess). Parse + sweep covered by unit tests.
- [x] `prepare_import()`: converted-geometry byte cap enforced (defense in depth; `add_import_record` already caps raw blob bytes).
- [x] Tests: `imports.rs` unit tests for the materialize/parent-dir/absolute path, the PID-aware sweep, and the over-cap reject. Full ctest + cargo green.

### Wave 3 — Import Project (backend)
- [x] `onecad_core::io::project_import` — `read_project_for_import` (open container → records + sketches + import blobs), `find_import_collisions` (refuses a source `RecordId` or `SketchId` that already exists in the target), `ProjectImportError` taxonomy.
- [x] `DocumentRuntime::import_project(path)` — collision-refuse → stage every blob into the import workspace/carrier → one transaction appending each record (`AddOperation` at cursor) + each sketch, merging all outcomes. Refused imports leave the target untouched.
- [x] `api::import_project` — Rust-owned `.onecad` picker; with an open runtime it appends, with none it seeds a blank document (`make_backend` + `new_blank` + adopt), then `PROJECTION_UPDATED` + snapshot + scheduler. No runtime → `NoDocument` error surfaced to the UI hint.
- [x] Wire: `client.ts` interface, `tauriClient` mapping, `commandMap`.
- [x] Tests: `document_runtime/tests.rs` import-append / collision-refuse / blank-runtime import; `imports.rs` lifecycle tests; full cargo workspace green.

### Wave 4 — import through the UI (mock + e2e)
- [x] `mockClient.importProject()` — merges the STEP-import fabrication (one body + `Import` row) so the whole frontend lane is mockable. Mock-unit test covers the appended snapshot.
- [x] `FileMenu` "Import Project…" (Open/Save group, above the Export hairline) → `fileActions.importProject` (dialog-backed, cancel = no-op) — shines one hint "Project imported".
- [x] `StartScreen` — sidebar "Import Project…" below Import STEP→; both entry points pinned by `e2e/project-import.spec.ts` (in-editor + start-screen lanes).
- [x] Unified start-screen import — the header button now says `Import…`, and the start screen routes `.onecad` vs STEP/STP by extension through one dialog. Sidebar duplicate removed. Gate: focused vitest on `StartScreen`, `tauriClient`, and `mockClient.import` green; `cd src-tauri && ONECAD_REQUIRE_WORKER=1 cargo test --workspace --lib` green.

### Gate
- [x] `bun run build` (tsc+vite) green · vitest **209 files / 3697 tests** · Playwright `project-import`/`step-import` spec **12/12** · full Playwright **386/390** (4 pre-existing `theme.spec` failures root-caused to the WORK `TitleBar` changes, not this package) · worker ctest **106/106** · `cargo fmt --all --check` · workspace clippy `-D warnings` · `cargo test -p onecad --lib` **253 passed / 0 failed**. Full `cargo test --workspace` (real-worker lane) and kernelbench left for the follow-up gate below.

### GH-0 — GATE PASSED 2026-08-09 (WP0.0 → WP0.3)
- [x] **`boolean-preview.spec.ts` is GREEN** — 16/16 across chromium + webkit at `--repeat-each=4`. Closed by three fixes, and the root cause was NOT what the plan assumed (see WP0.2 below for the full correction).
- [x] **Kernelbench baseline is now a RETAINED MANIFEST, not a summary.** `bench/robustness/baselines/digests.json` freezes 256 rows of `(suite, caseId, backend, variant) → {inputDigest, normalizedDigest}` for `fillet/foundation:t0` (136) and `fillet/matrix:m1` (120), with `scripts/kernelbench-manifest.mjs record|compare`. The aggregate the gate used to quote cannot answer "did this change move a digest?" — a case can change shape and still pass — and replay only compares two runs of the SAME build. Comparator proven by mutating one saved digest and watching `compare` exit 1.
- [x] Suites re-measured against the HEAD-built pinned worker: T0 **136 records, 136 pass, 0 fail, gatingFailures 0**; m1 **120 records, 114 pass + 6 characterization, gatingFailures 0** — both identical to the previously recorded numbers.
- [x] Gate: ctest **109/109** · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1048 / 0** across 73 targets · `cargo fmt --all --check` + workspace `clippy -D warnings` clean · `bunx tsc --noEmit` clean · vitest **240 files / 4043 tests** (from 236/3986) · hex gate 0 · Playwright **380 passed / 10 failed**.
- [x] **The 10 Playwright failures are NOT this work, and that was verified rather than assumed.** All 10 are 5 specs × 2 browsers (`ellipse` ×3, `center-rect`, `sketch-reattach`) and ALL of them reproduce at `d81f758` in a throwaway worktree — i.e. before any commit of this tranche. They are the concurrent session's `feat(toolbar): tool flyout families` commit: `Ellipse` / `Center rectangle` moved behind a flyout and those specs still query them as top-level buttons (`getByRole("button", {name: "Ellipse"})` ⇒ element not found). `boolean-preview` PASSES at `d81f758` in that same worktree and passes here, so this tranche took the suite from 3 boolean-preview failures to 0 and introduced none. **Owner: the toolbar session** — the specs need to open the flyout, or the flyout needs to keep the family's active tool queryable by name.

### GH-0 WP0.2 — boolean-preview: THREE DEFECTS, root cause NOT what the plan predicted (2026-08-09)
- [x] **Defect 1 — a REJECTED region fetch wiped live selection.** `sketchStaticSync.loadSketch` fell through its `catch` into `reconcileRegionRefs(id, ∅)`, so a transient `getSketchRegions` failure dropped every `sketchRegion` ref for that sketch. An empty set from a FAILED fetch is not evidence a region disappeared; a DELETED sketch still reconciles (that set is authoritative).
- [x] **Defect 2 — a pick landing during a refill read as "empty space".** A token-driven reload calls `removeSketch` and only restores the fills two awaited round trips later; in that window `sketchStaticHitTest` returns null and `ViewportRoot` called `sel.clear()`. Now `ViewportEngine.sketchStaticPickState` returns an ATOMIC `hit | miss | unsettled` and an unsettled pick DEFERS (one bounded retry, cancelled by the next pick, released on detach) instead of clearing. Polling a readiness flag cannot fix this — the window can open between the poll and the click, which is why the old `sketchHitTestReady` passed immediately before the failure.
- [x] Both proven red-first: reverting either fix turns the three new `sketchStaticSync.test.ts` cases red (verified by actually reverting them, not by inspection).
- [x] **Defect 3, and the ACTUAL root cause — a stale click coordinate, not a race in the pick lane.** Instrumenting the pick with both hits on a failing webkit run showed `state:"miss"` with **no body hit AND no sketch hit**: the ray lands on empty space. `extrudeRegionAt` projected the sketch-plane point to client coordinates without waiting for the camera; re-showing a sketch schedules the DEBOUNCED auto-fit, which then moves the camera and invalidates the coordinate. `waitForCameraSettled` already existed for precisely this and its own comment describes the failure ("a fit can be scheduled-but-not-started … invalidating any client coordinate the caller computed from a probe") — `extrudeRegionAt` simply never called it.
- [x] **Two hypotheses were wrong and are recorded so they are not re-tried.** (a) The plan's assumption that the refill teardown window was the cause: `sketchHitTestReady` polls the tri-state and reports `hit` immediately before the click, and the deferral only engages on `unsettled`. (b) The follow-up hypothesis that HIT PRECEDENCE was returning the body ref for a point over the region: refuted by the probe — there was no body hit at all. Fixes 1 and 2 are still correct defects on their own merits; neither was this one.
- [x] **Defect 4 — `clickApplyButton` could silently not click.** Its `toPass` retry only asserted that a rect EXISTED, so a click that missed (the chip is overlay-positioned and can move between measuring and clicking) still returned successfully and left the caller to time out on a lane that was never applied. It now retries on the OUTCOME (lane closed), re-checking first so a successful apply is never clicked twice.
- [x] An earlier attempt to make the spec wait for the region to appear in `selectionStore` before clicking Extrude was REVERTED: it made things worse (4 failures) because it asserts a precondition the product does not guarantee at that point.
- [x] Result: **16/16** on `--repeat-each=4` across both browsers.

### VF-M5 RESIDUAL — CLOSED 2026-08-09 by `checkpointFallbackReplay` (GH-0 WP0.0+WP0.1)
- [x] **RED FIRST, and the entrance criterion was real.** `topology_rebind::vfm5_lane_d_checkpoint_fallback_replay_must_not_bind_the_decoy`: checkpoint selected → worker killed → restore reports `restored:false` → F12 replay → the fillet **silently consumed a congruent decoy parked on the stale anchor** (`needsRepair=0`, `removed=10.3009` = the analytic wedge, `centroidY=69.00107` matching the decoy prediction exactly against the authored rib's `68.99535`). Green after the field lands.
- [x] **Three constraints found by measurement, all of which had made the earlier fixtures vacuous.** (a) A sketch at step 0 can only regen with `ToEnd { from: 0 }`, which claims no `editedFrom` — so `post_upstream_edit` is false and the veto never arms. That is why every H5 test uses `regen_all` and why NONE of them reach this carve-out; the fixture puts the comb sketch at step 2 behind a throwaway body. (b) Editing a `TransformBody` **seeds** NeedsRepair downstream (`transform_body.rs:906`), so it never reaches the ladder and cannot be the driver. (c) At 20 mm of rib separation the ordinary auto-bind margin gate refuses first (measured margin 0.0745 < 0.10) and the carve-out never decides — the scene needs 48 mm.
- [x] Independently confirmed the veto is reachable end to end by forcing `from_zero_replay = true` and running the suite: the ONLY test that flips is `h6a` (the false-positive direction), and neither H5 decoy test changes.
- [x] **The fix is the protocol change this entry demanded.** SCHEMA §7.2 `checkpointFallbackReplay` (OPTIONAL, omitted when false), set by exactly one place — `regen/executor.rs`'s F12 branch — and consumed by `PlanExecutor.cpp` as the sole source of `job.from_zero_replay`. §14 entry + §13 lockstep note (the worker is a bundled `externalBin` sidecar, so no capability negotiation is needed — but that property is now written down as load-bearing). Two canonical fixtures added, `protocol/fixtures/execute_plan_{ordinary,checkpoint_fallback}.ndjson`, run by BOTH tracks (`messages.rs` list, `worker/tests/CMakeLists.txt`, `check_interop.sh`).
- [x] **The ordinary edit lane is deliberately UNCHANGED** and now pinned by its own characterization test (`vfm5_teleport_on_the_ordinary_edit_lane_is_the_accepted_residual`): the teleport still binds the decoy there. That is the documented H6a residual, not a bug this tranche fixes — a change to it must be a decision, not a side effect.
- [x] Accepted trade: a legitimate anchor-exact rebind occurring inside an F12 fallback now returns `NeedsRepair` (a conservative false positive on a rare lane). Deterministic `NeedsRepair` beats a silent wrong bind.
- [x] Gate: ctest **109/109** (107 → 109, the two new canonical fixtures) · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1048 / 0** across 73 targets (from 1045/0 across 72) · `cargo fmt --all --check` + workspace `clippy -D warnings` clean · worker built from HEAD against `~/.onecad-occt/8.0.1`.
- [x] Stale "V1 has no restore / checkpoints UNSUPPORTED" prose corrected everywhere it was asserted: `worker/src/session/Session.cpp`, `ScratchJob.h`, `Ladder.h`, `CURRENT_STATE.md` (both sites), `HANDOFF.md`.

### VF-M5 FOLLOW-UP — real-worker lane RED (the flagship regression) — CLOSED 2026-08-09
- [x] `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` against a worker built from HEAD **FAILED** `topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean` (`needsRepair` expected 0, got 1). Reproduced here on a fresh `worker/build-pinned` build before changing anything.
- [x] Root cause confirmed by reading, not inferred: `Session::fence_and_clone` clones an EMPTY base for any plan whose expectedBaseHash is the empty anchor (D5, `Session.cpp:153-175`), which is every ordinary regen — so `job.partition.size() == 0` in `PlanExecutor` is always true and `from_zero_replay` degenerated to `edited_from.is_some()`.
- [x] Fix: resolution (a). `job.from_zero_replay = false`, restoring the shipped edit lane. **The SECOND half of that entry — "the hazard cannot occur in V1" — was wrong and is corrected in § VF-M5 RESIDUAL above.** Ladder behaviour untouched and still covered by `worker/tests/test_wp6_ladder.cpp`.
- [x] Re-verified on the real-worker lane: `topology_rebind` 13/13, `cargo test --workspace` **1045 passed / 0 failed** across 72 targets, ctest **107/107**.

## ADVANCED-FILLET ROADMAP — M0 REPRODUCIBLE-GREEN + M1 QA-AUTOMATION (2026-08-08) — GATE PASSED

First two milestones of the Advanced-Fillet program (kernel robustness 70% / direct modeling 20% / UI 10%; KPI = raw-OCCT rescue rate, with `raw PASS → OneCAD FAIL` treated as a serious regression and invalid geometry always worse than a safe refusal). No production fillet algorithm was touched — by design, the benchmark discovers the problem before the kernel guesses at a solution.

### M0.1 — OCCT build fingerprint: effective configuration, not literal presence
- [x] Root-caused against the pinned source AND a real configure, not assumed. OCCT's `OCCT_CHECK_AND_UNSET` does `unset(VAR CACHE)`, and `OCCT_IS_PRODUCT_REQUIRED` derives each `CAN_USE_*` by scanning `BUILD_TOOLKITS`. With `BUILD_MODULE_Draw=OFF` nothing requires `CSF_TclTkLibs`, so `CAN_USE_TK` is OFF and CMakeLists (8.0.1 L558-565; 7.9.3 byte-identical over every `USE_*`/`CAN_USE_*`/`OCCT_CHECK_AND_UNSET` line) DELETES our `-DUSE_TK=OFF` from the cache. The old literal-presence loop then aborted a configuration that was exactly what we asked for. Nine further keys can vanish the same way: `BUILD_USE_PCH USE_DRACO USE_FFMPEG USE_FREEIMAGE USE_FREETYPE USE_OPENGL USE_RAPIDJSON USE_TBB USE_VTK`.
- [x] New `scripts/occt-fingerprint.sh` — sourceable, side-effect-free, bash 3.2 compatible. Keys are classified REQUIRED vs DEPENDENT. An absent DEPENDENT key normalizes to `OFF` **only when the policy requested OFF** (OCCT deleted it precisely because the feature is not in the build); requested-ON-but-dropped is fatal, a missing REQUIRED key is fatal, a present key that disagrees with policy is fatal, and a value outside CMake's documented truth constants is fatal rather than guessed. Normalization, not relaxation.
- [x] New `scripts/tests/occt-fingerprint.test.sh` — 14/14, no network, no OCCT build. The 8.0.1 fixture is a verbatim capture of the fingerprinted lines from a real `CMakeCache.txt` (including the `:UNINITIALIZED` types a bare `-D` produces); every absence carries its CMakeLists citation. Covers both versions producing byte-identical fingerprints, alternate boolean spellings, duplicate cache lines, and all four failure modes.
- [x] `build-pinned-occt.sh` sources the library and restates the policy string, so an edit (or a pre-set env override, the seam the test uses) on either side cannot silently change what a real build is fingerprinted against.
- [x] CI: new fast `occt-fingerprint` job gates the hour-long OCCT jobs; both cache keys now hash `occt-fingerprint.sh` as well.
- [x] VERIFIED END TO END: pinned OCCT 8.0.1 built from scratch into a clean prefix and installed, `occt-build.json` carrying the correct `normalizedBuildOptions`; the reuse path then short-circuits. `USE_TK` is genuinely absent from the produced cache — the old script would have hard-failed here.

### M0.2 — frontend clean build
- [x] `bun run build` was red while `bun run test` was green, because vitest does not typecheck. Two `tsc` errors in `faceColors.test.ts`: a hand-written structural mock of the 24-field `BodyMeshView` (missing 8 fields) and readonly `FaceColor` assigned into a mutable tuple.
- [x] Fixed at the type level, no casts: `Rgba` is now `readonly [number, number, number, number]` (a color is a value, never mutated), and the ~20 duplicated raw `[number, number, number, number]` literals across `documentStore` / `mockClient` / `tauriCommandMap` / `meshSync` collapse onto it. Read-only consumers (`faceColors.ts`, `meshRegistry.ts`) take `ReadonlyMap<string, Rgba>`.
- [x] New `src/test/fixtures/bodyMeshView.ts` → `makeBodyMeshViewFixture({faces, idsHaveElementIds, …})`, deriving `faceRanges`/`faceIdOffsets`/`faceIdChars`/counts/flags/bbox so a fixture cannot disagree with itself. Prefer `parseMeshPayload(makeBoxMesh(...))` when MESH1 framing matters; the builder exists for shapes MESH1 fixtures cannot produce (ElementId-bearing face ids).
- [x] Hex gate regression found and fixed while gating: `InspectorPanel.rgbaToHex` returned the literal `#a9aeb6` — which IS `--color-body-fill`. Now resolved through `palette.bodyNeutral()`, so the no-color swatch follows the theme instead of freezing at the light value. `grep -rn '#[0-9a-fA-F]\{6\}' src` is back to zero.

### M1 — manual-QA debt triaged
- [x] All ~112 boxes of the historical `docs/MANUAL_GATES_RUN.md` classified into K (CTest/Kernelbench) · H (Rust real-worker) · F (Vitest/Playwright) · M (manual). Result: **78 already asserted by an existing automated test and retired** (each cited by `path:line`), **22 named gaps** that belong in an automated lane but have no test yet, **12 genuinely manual** checks.
- [x] `docs/qa/MANUAL_RELEASE_GATES.md` — the 12 survivors only, grouped by WHY a machine cannot judge them: visual (4), native (4), hardware (4). Carries an explicit "do not grow this file" rule — a new manual permutation means a missing automated lane, which is exactly the pressure advanced fillet would otherwise apply.
- [x] `docs/qa/MANUAL_GATES_TRIAGE.md` — the item-by-item classification plus the GAP-K/H/F backlog. Nothing was deleted: every check is cited as covered, listed as a gap with its target lane, or moved into the release checklist.
- [x] Historical checklist archived verbatim at `docs/qa/archive/MANUAL_GATES_RUN-2026-08-04.md`; stale pointers in `README.md` and TODO :155 updated.

### M0.4 — viewport auto-fit regression, found by the gate
- [x] The gate's e2e run failed in a large cluster (`constraint-apply`, `live-dim-line`, `marquee`, `measure`, `navigation`). BISECTED, not assumed: `measure.spec.ts` is 5/5 at `d1c5339`, **1/5 at `1fe0cef`**, and 5/5 at `1fe0cef` with only the auto-fit debounce reverted.
- [x] Cause: `1fe0cef` replaced "auto-fit once on the first body" with a 250 ms debounced re-fit (right intent — a multi-body assembly must not be framed on whichever body streamed in first), but the timer lived in the React bridge. A `fitView()` tween could therefore START after everything else already believed the camera had settled, invalidating any client coordinate computed from a probe. User-visible too: a queued fit can snap the camera mid-interaction, and one landing during sketch entry is saved as the restore pose.
- [x] Fix keeps the intent and makes the scheduled state real: the debounce moved onto `ViewportEngine` as `requestAutoFit()` / `autoFitPending` / `cancelAutoFit()`, cancelled by explicit `fitView`, `fitToBodies`, `enterSketch`, and `dispose`. `ViewportRoot` just calls `engine.requestAutoFit()`; `waitForCameraSettled` now requires no tween AND no pending auto-fit. Full suite 12+ failures → **7**.

### Gate
`bun install --frozen-lockfile` · `bun run build` (tsc+vite) · vitest **208 files / 3687 tests** · `scripts/build-pinned-occt.sh` 8.0.1 clean build + reuse path · `scripts/build-worker.sh Release` against the PINNED prefix · ctest **106/106** on the pinned worker · `cargo fmt --all --check` · `clippy --workspace --all-targets -D warnings` · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1000 passed / 0 failed** against the pinned worker · kernelbench T0 both-backend **136/136 pass, 0 rescued, 0 regressions, 0 replay-unstable** · fingerprint normalizer **14/14** · hex gate 0 hits · Playwright **183/190**.

### M0.5/M0.6 — the remaining 7 e2e failures, root-caused not retried
All seven reproduced on a pristine `1fe0cef` checkout; none was caused by this work package. Each is now fixed at its cause:
- [x] `transform-body ×4`: the spec's `bodyBounds` folded the fat-line EDGE layer into the bbox. `Line2` reports `isMesh === true`, and its `position` attribute is the instanced unit-quad TEMPLATE `(-1,-1,0)…(1,2,0)` — the real endpoints live in `instanceStart`/`instanceEnd`. Every body's min was therefore clamped to ≤ -1 while `max` stayed correct, which is exactly the observed `[-37, -1, -15]` (only the component inside that box was wrong). Traversal now takes `userData.kind === "face"` only.
- [x] `history-inline-dimension:213`: `toolApplicability.ts` is NEW in `1fe0cef` — the toolbar is selection-gated now, and committing an extrude hides its sketch, so none of sketch/region/sole-visible-sketch held and Extrude sat `aria-disabled` while the click burned the 45 s timeout. Also `getByLabel("Dimension value")` matched both the row editor and the tool chip. Re-show the sketch (eye toggle, NOT a selection change — the row selection has to survive) and scope the locator to the row.
- [x] `hole:226`: `findFaceOnBody` probed pixels before the camera settled, so the returned point sat on a face about to move ("no second pixel far enough on the same face"). Helper settles first.
- [x] `sketch-degenerate:35` + `live-dim-mouse-rounding:62`: `enterSketch` aims along the plane normal at `controls.getDistance()`, so the sketch view INHERITS the model camera distance — which sets `planePixelWorld`, hence the draw tools' screen-constant reject radius (`minSize = 4 × planePixelWorld`) AND the zoom-adaptive dimension quantum. Entering mid-fit made both non-deterministic run to run; every caller settled AFTER entering, too late. `enterSketchViaPlanePicker` settles first.
- [x] Verified 52/52 across `transform-body`, `history-inline-dimension`, `hole`, `sketch-degenerate`, `live-dim-mouse-rounding`, `measure`, `constraint-apply`, `marquee`, `navigation`.
- [ ] OPEN DECISION: `playwright.config.ts` uses `retries: 1` in CI, `0` locally. Every flake above was invisible in CI and hard-red locally. Now that they are root-caused rather than retried away, dropping the CI retry is the honest setting — a retry is what let the auto-fit regression look green.

### Concurrent-session caveat
A second session was editing this tree throughout (`TitleBar.tsx`, `ViewCube.tsx`, `CadOrbitControls.ts`, `Picker.ts` — `hasHitAt` removed — `ModelToolController.ts`, `navigation.spec.ts`, `document_runtime/tests.rs`) and added a **webkit** Playwright project (suite 190 → 386). A full-suite number is not attributable while that is in flight: one run hit a live `[vite] Transform failed` from a mid-edit save, and `theme.spec:121,145` fails on `getByRole('button', {name: /^Appearance:/})` — element not found — from the `TitleBar.tsx` change. Not from this work package.

### M2 part 1 — kernelbench case-v2 — DONE
- [x] `bench/robustness/schemas/case-v2.schema.json` (strict throughout) + `src-tauri/crates/onecad-kernelbench/src/case_v2.rs` (a SEPARATE type set, not an extension) + `bench/robustness/examples/fillet-matrix-plane-cylinder-v2.json` (ajv-validated; deliberately NOT in `regressions/`, which is contractual).
- [x] `selector` moved to top level — the same geometry+selection is reused across radius laws and later across operations, so a resolving selector is a PRECONDITION for the operation, not part of its definition.
- [x] `operation.definition` mirrors the kernel-level `FilletDefinition` (radius law + continuity), so `constant`/`linear`/`controlPoints` are all expressible and adding one later is additive on both sides.
- [x] `geometry.parameters` is a typed per-recipe union rather than the free-form object the plan sketched: a free-form bag lets a typo silently generate different geometry, which is the one failure mode a fuzzing corpus cannot survive.
- [x] `continuity` admits only `g1`, `sizeType` only `radius` (agent rules 11/15 — an option is not a capability; G2 and chord width become expressible only once a validator can prove the result).
- [x] Recipes expressible: the v1 four, plus `supportPair`, `valenceCorner`, `shortEdge`, `microEdge`, `sliverNeighborFace`, `tinyNeighborFace`, `periodicSeam`, `nearSeamEdge`, `faceNearlyConsumed`, `faceFullyConsumed`, `blendCollision`. Supports span plane/cylinder/cone/sphere/torus/bspline; `scaleBand` covers 1e-3 … 1e6.
- [x] Metamorphs expressible: translation, farOriginTranslation, rotation, mirror, uniformScale, parameterEpsilon, edgeOrderPermutation, contourSeed (M3's set, schema side).
- [x] Guards: v1/v2 provably disjoint (both directions, in Rust AND ajv) · v1 regressions still validate under `Case` · round-trip reproduces the committed file, which already caught three missing `#[serde(rename)]`. Cross-field rules that JSON Schema cannot express live in Rust and are authoritative because Rust is what executes: mode ⇒ (anchor count, adjacency relation) as ONE table; control-point parameters strictly increasing; torus `minorRadius < radius`; `edgeOrderPermutation` a real permutation; zero epsilon rejected.
- [x] Gate: kernelbench 50 lib + 5 integration green · `cargo fmt --all --check` · workspace `clippy -D warnings`.

### M2 part 2 — geometry generators + the v2 execution lane — DONE

The cross-language half: the worker can now execute a `schemaVersion: 2` case, and the supervisor can drive one end to end.

- [x] **C++ `CaseSpec` is normalized, not forked.** `Types.h` carries the v2 shape (`schema_version`, `generator.family`, `recipe` + typed `RecipeParameters`, `RadiusLaw`, `scale_band`, selector `convexity`/`vertexValence`) and the v1 parse fills the subset, so ONE execution path serves both formats. Duplicating the executor per version is where a v2-only bug would hide. `parse_case` dispatches on `schemaVersion`; v1 acceptance is unchanged.
- [x] `CaseParserV2.cpp` (new) mirrors `case_v2.rs::validate` field for field — the Rust side is authoritative for what a case may CONTAIN, the C++ side for what the runner will EXECUTE, and a disagreement would make a valid case fail as an invalid request. Shared primitives moved to `CaseParserShared.h` rather than being copied. A non-constant radius law is REFUSED at parse (`invalidRequest`), not silently flattened to its peak — variable radius is M12, with a validator that can prove the law.
- [x] `SelectorParser` gains the v2 selector: top-level, wider surface/curve vocabulary, `chain`/`closedContour` modes, `supportPairEdge`/`seamEdge`/`shortEdge` roles, and the mode ⇒ (anchor count, adjacency relation) table as ONE table. `parse_case_v2` re-checks `schemaVersion == 2` itself, so the entry point cannot be called directly on a v1 document — a gap the new fixture caught.
- [x] **`supportPair` generators** (`GeometrySupportPair.cpp`, new). Two constructions: the PRISM family (plane|cylinder × plane|cylinder) is one 2D profile extruded along Z, giving a straight shared edge through the origin and a FREE dihedral over `(0,180)`; the CONE family (plane × cone) is a frustum's base circle, where the dihedral is `90 - halfAngle` by construction — so the declared angle is rebuilt from the half angle and a disagreement is refused rather than silently generating different geometry than the file describes. Unimplemented kinds (sphere/torus/bspline) and concave supports refuse BY NAME.
- [x] Why the prism family and not the committed example's base-circle form: the example locked plane↔cylinder to a 90° dihedral, and the dihedral is the main conditioning axis for fillet failure. The prism form also keeps the blend a CYLINDER, which is what lets `constantRadius`/`cylindricalRadius` gate it. `examples/fillet-matrix-plane-cylinder-v2.json` updated to match (anchor, `curveKind`, `edgeLength`) — examples are explicitly not contractual; `regressions/` untouched.
- [x] **Rust `PreparedCase`** (`prepared.rs`, new) is the version-agnostic view the supervisor executes: identity, resource ceilings, metamorph tolerance, and the canonical document. `campaign`/`runner`/`child`/`result`/`result_validation` route through it, so none of them grew a `match` on the schema version. A v2 case reports the v1 `{name, version, seed}` generator shape because the RESULT schema is frozen at v1 — `Execution.cpp` now BUILDS that block instead of echoing the case's, which is what made the first v2 run fail 60/60 on strict result validation.
- [x] **`suite_v2.rs` — `fillet/matrix` preset `m1`**: 24 cases (3 prismatic pairs × 5 dihedrals supported + 2 near-tangent exploratory, plus 3 cone half-angles), 60 records with metamorphic variants. `cli` gained the suite and a version-dispatched `run-case`.
- [x] **Sizing is measured, not assumed.** A blend needs room; for a prismatic pair the throat is `R·(1 - cos θ)`. Swept against OCCT 8.0.1: every pair blends at 0.20 of the throat, the cylinder↔cylinder lens refuses at 0.40. Supported cases sit at 0.04–0.16, so a red there is a kernel regression and not a greedy case. Near-tangent dihedrals (170°, 178°) are `exploratory`, NOT `expectedLimit` — an expectedLimit case needs a radius the kernel is KNOWN to refuse, and guessing one produces a case that passes whatever happens.
- [x] **Validator finding: `cylindricalRadius` and `g1BoundaryTangency` are box-shaped.** The first counts EVERY cylindrical face in the output against the requested radius, so a cylindrical SUPPORT reads as a blend of the wrong radius (measured 17.0 error on a 20 mm support); the second only recognises plane↔cylinder tangency pairs and cannot see a blend meeting a curved support. Both are emitted only for all-planar pairs, pinned by test. Their general replacements (`supportTangency`, `crossSectionProfile`) are expressible in case-v2 but unimplemented — and an unimplemented validator reports `notApplicable`, which FAILS a required check rather than passing it, so emitting them now would red the whole suite. They land with M4.
- [x] `kernelbench_case_v2` ctest fixture asserts GEOMETRY, not the parser echoing itself: the built dihedral is measured from the two adjacent face normals at the selected edge's own midpoint and compared to the declared angle to 1e-6, across 3 pairs × 3 angles, plus the cone-mismatch refusal, unimplemented-kind refusal, version disjointness, and the variable-law refusal.

### Gate — M2 part 2
`ctest` **107/107** on the pinned OCCT 8.0.1 worker (106 → 107, the new `kernelbench_case_v2`) · `cargo fmt --all --check` · workspace `clippy -D warnings` clean · kernelbench **56 lib + 5 integration** (50 → 56) · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1032 passed / 0 failed** across 73 targets against the pinned worker (1000 → 1032) · **T0 both-backend UNCHANGED: 136/136 pass, replay 136 stable, metamorph 48 pass / 0 fail / 16 notRun, differential 136 same-status, `gatingFailures: 0`** · **M1 both-backend: 120 records, 114 pass + 6 characterization (the near-tangent exploratory refusals), replay 120 stable, metamorph 72 pass / 0 fail, differential 120 same-status, `gatingFailures: 0`, p50 10.3 ms / p95 62.3 ms** · the committed v2 example executes through `run-case` on both backends, pass + replay-stable · ajv-valid under Draft 2020-12.

### M3 — metamorph execution (DONE)

- [x] `VariantSpec` (`worker/src/benchmark/Types.h`) and `suite::Variant` widened to the full v2 metamorph set: `translation`, `rotation`, `mirror`, `scale`, `parameterEpsilon`, `edgeOrderPermutation`, `contourSeed`, plus `farOriginTranslation`. All fields serialize/deserialize on both sides with strict unknown-field rejection.
- [x] `apply_variant` (`Geometry.cpp`) implements mirror (about volume center or explicit center), uniform scale (about volume center or explicit center), and keeps the existing translation/rotation. `edgeOrderPermutation` and `contourSeed` reorder `selected_edges`, exercising selector order-independence.
- [x] `metamorph.rs` inverse-transforms mirror, scale, far-origin translation, rotation, and translation; identity no-ops for parameterEpsilon/edgeOrderPermutation/contourSeed. Shape-signature comparison is genuine, not fabricated.
- [x] `parameterEpsilon` is handled on the request side: `Execution.cpp` multiplies the effective fillet radius by `(1 + relativeDelta)` when the variant requests it.
- [x] `fillet/matrix:m1` preset includes the rigid/isometric metamorphs (translation, far-origin translation, rotation, mirror, edge-order permutation, contour seed). `uniformScale` and `parameterEpsilon` are expressible in the schema and runnable via `run-case`, but are NOT included in `m1` because they intentionally change mass properties and so fail the current metamorphic-equivalence gate (which compares volume/area exactly).
- [x] Gate passed: T0 **136/136** pass, replay stable, metamorph 48 pass / 0 fail / 16 notRun, differential same-status, `gatingFailures: 0`. `fillet/matrix:m1` **264 records** (24 base cases + 18 supported × 6 variants), **258 pass + 6 characterization**, replay stable 264, metamorph 216 pass / 0 fail, differential same-status, `gatingFailures: 0`. p50 **10.9 ms** / p95 **69.3 ms** (macOS AppleClang, Release worker, debug supervisor).
- [x] Manifest baselines updated: `bench/robustness/baselines/digests.json` now records 136 T0 rows + 264 m1 rows for `darwin-arm64`; `semantics.json` records both suites.

### M3.5 — non-isometric metamorph policy (OPEN)
- [ ] Decide how `uniformScale` and `parameterEpsilon` should gate. Options: scale the compared mass properties by `factor³`/`factor²` in the campaign; relax the metamorphic-equivalence tolerance for epsilon; or treat them as separate robustness probes with their own validator instead of `metamorphicEquivalence`.
- [ ] Once the policy is decided, add `uniformScale` and `parameterEpsilon` back to the `m1` preset and record updated baselines.
## NOW — roadmap completion program (plan `~/.claude/plans/now-lets-plan-next-sunny-lighthouse.md`)

Approved 2026-08-13. Seven tracks A–G; **TRACK A IS COMPLETE — A1–A6 done and committed**.
Sequence and full rationale live in the plan file; the short version is below. Track A was the
unbilled remainder of Phases 0–4 and came before any new Phase 5 breadth.

- [x] **A4 — Revolve body-edge axis. PRODUCT CALL TAKEN: the variant stays UI-HIDDEN.** Kernel and
      core implement it; the UI authors only `sketchLine`, and that is now what the contract row
      says (`uiExposure: "hidden"`, was a false `"exposed"`). This is also what WP1.5 asked for —
      "do not expose it until persistence, reopen and upstream-edit tests pass". All four required
      tests landed; detail in § ROADMAP A4 below.
- [x] A5 — the WP0.7 / WP0.8 tests. WP0.7 was **not** tests-only: preview failure was tracked
      GLOBALLY, so a secondary region's refusal outlived its own recovery and wedged every commit.
      Fixed per-session, red-first. Detail in § ROADMAP A5.
- [x] A6 — the two missing cross-track fixtures. The `ResolveRefs` one could not be written
      honestly: SCHEMA §7.5's snapshot echo was normative and implemented by NEITHER worker, with
      Rust manufacturing the values and validating nothing. Implemented and validated fail-closed.
      Detail in § ROADMAP A6.
- [x] C1 — the coverage manifest is true and its verifier has teeth (16 dead paths, a `ciJob` that
      exists in no workflow, four measured overclaims and one UNDER-claim, all corrected; the
      verifier now stats paths, resolves jobs and runs WP4.5's five registry cross-checks).
      Detail in § ROADMAP C1.
- [x] C2 + corpus completion — all 9 frozen cases execute against the real worker; classification
      is manifest-driven, expectations typed, unsupported table removed, zero silent skips. Frozen
      case JSON unchanged by the 9/9 completion. Detail in § ROADMAP C2 and top gate ledger.
- [ ] C5/C6, remaining P2 V3 bounds/performance, Phase 5/6 breadth. P4 remains partial until full
      zero-retry browser gate passes. C5 performance caps remain undocumented pending measured Tier
      A/B and cumulative Pattern cost.

- [x] C3 diagnostics: bounded worker diagnostic evidence reaches Inspector and Repair;
      successful retry clears stale evidence; edit/retry and rebind remain explicit actions.
- [x] C4 local lowering: Extrude/Revolve Intersect lowering exists, but both profile-operation
      Intersect controls are now UI-hidden until C++/real-worker/browser promotion is atomic.
      OffsetFace Total/Diameter remains Prepare-gated and re-edit-safe.
- [x] Corpus topology: `QueryBodyTopology` supplies actual BRep solid/face counts;
      corpus assertions no longer infer faces from coarse MESH1.
- [x] P2 bounded source collection: analytic profile refinement refuses above 256 sources
      before pair collection. `regionIdentityVersion:3` is now implemented for new authoring; measured
      pair/fragment/cancellation/performance closure remains open.
- [ ] C6 browser gate: persistent Vite on `4178` served correctly, but sandboxed Chromium
      aborts at Mach-port rendezvous; elevated run began cleanly then executor detached after
      five passes, leaving no valid full-lane result. Rerun Chromium + WebKit once per lane,
      retries `0`, retaining artifacts.

- [x] Pattern compatibility baseline: V1 aggregate replay stays source-preserving;
      only V2 applies connected-single-solid validation. Future numeric policy versions
      load/resave losslessly and worker execution refuses `UNSUPPORTED_PATTERN_RESULT_POLICY_VERSION`.

Detail per completed wave is recorded below, newest first.

## RENDER MODULE — STUB REGISTRATION + DESIGN DOCS (2026-08-13) — DOCS ONLY, NO GATE

Not part of Track A/roadmap — a separate, product-requested design pass for a
future Render workspace (Fusion 360 Render parity, materials on OpenPBR). Docs
+ an inert module scaffold only; no functional capability.

- [x] **`onecad.render` module scaffolded and registered**, reaches `"ready"`
      with zero contributions (`src/modules/render/manifest.ts`, `ids.ts`,
      `module.ts`, `register.test.ts`; wired in `src/app/bootstrap.ts`).
- [x] **ADR-0014** (`docs/adr/0014-render-module-openpbr.md`) records: Render is
      its own module (not folded into Modeling); its future UI targets the
      *existing* `onecad.shell` `Visualization` workspace placeholder rather
      than a new workspace id (product confirmed the placeholder already means
      "Render", just labeled differently); OpenPBR is the material schema
      baseline.
- [x] **`docs/RENDER_MODULE.md`** drafts the OpenPBR-to-schema mapping, a
      document-state shape, and a phased roadmap — all explicitly marked
      draft/unimplemented. Render backend (real-time preview vs. offline
      path-traced vs. other) is explicitly left open, not decided.
- [ ] Everything past registration — material schema, assignment UI, workspace
      migration, render backend — unstarted by design; this wave is scaffolding
      for a later implementer, not a first slice of the feature.

**Changed:** `src/modules/render/manifest.ts`, `ids.ts`, `module.ts`,
`register.test.ts`; `src/app/bootstrap.ts`; `docs/adr/0014-render-module-openpbr.md`;
`docs/adr/README.md`; `docs/RENDER_MODULE.md`; `TODO.md`.

**Gate:** `bunx tsc --noEmit` clean · `bun run test` — new
`src/modules/render/register.test.ts` passes, `src/platform/architecture.test.ts`
and `src/platform/registry.test.ts` unaffected (no forbidden import, no
namespace/duplicate-id regression). No Rust/C++ touched — ctest/cargo/Playwright
out of scope, not run.

**Next:** none scheduled — parked until product prioritizes Render.

## ROADMAP C1 — A MANIFEST THAT CAN BE FALSIFIED (2026-08-13) — GATE PASSED

`docs/qa/modeling-operation-coverage.json` is the machine-readable claim about where each
supported operation is proven, and nothing checked the claims. Sixteen cited paths did not exist —
nine `src/tools/modelTools/*` unit tests that were never written under those names, three e2e
specs, an `import_step.rs` that is really `step_import.rs` — and eleven rows named a CI job
`macos-full` that is in no workflow. All of it green.

- [x] **Every row cites a file that exists**, found by asking what actually covers each lane rather
      than by deleting the claim: Fillet/Chamfer/Shell are `ModelToolController.edgeShellPreview`,
      Boolean is `booleanPreview`, the sketch drag is `selectDrag.reconcile`, Shell's browser flow
      is `shell-preview.spec.ts`, ImportStep is `step_import.rs` + `step_import_gate.rs`. Two lanes
      have NO honest citation (an FE unit test for Boolean Intersect, one for XCAF) and are left
      EMPTY — an empty field says "unproven", a dead path says "proven" and is not.
- [x] **`ciJob` became a per-lane list**, because one job can never gate a row whose evidence spans
      C++, Rust, vitest and Playwright. The verifier resolves every id against
      `.github/workflows/*.yml` and requires each non-empty lane to name the job that runs it.
- [x] **Four overclaims now state their measured limit** (each verified against the test code, not
      assumed): Shell runs a SINGLE open face in every lane; the OffsetFace Rust lane executes
      `Radius` only and covers `Total` through the prepare handshake, with `Diameter` C++-only; the
      Hole browser flow configures countersink and never commits it; CircularPattern's partial
      sweep is C++ and FE-unit only.
- [x] **One "overclaim" was not one.** MirrorBody no-fuse IS covered — `ordinal_tripwire.rs` authors
      it and the C++ lane runs both `fuseWithOriginal` values — so the row says so instead of
      inheriting a review's guess. And Boolean Intersect was UNDER-claimed as
      deferred-with-no-evidence though its P4 vertical landed.
- [x] **Five WP4.5 cross-checks**: `KnownOperation`, worker dispatch, the SCHEMA §7.3 op catalogue,
      the frontend tool registrations, and the kernelbench families. Each scan asserts it found
      something first, so a moved source cannot turn a check vacuously green; rows that are
      deliberately not `KnownOperation`s declare themselves in the manifest's `nonOperationRows`.
- [x] **Twelve negative controls, up from one**, across both verifiers — including the acceptance
      test the plan names: renaming a cited spec reds the check, proven both through a mutated temp
      manifest and by renaming `e2e/hole.spec.ts` on disk.

Gates: both verifiers; `scripts/tests/verify-modeling-coverage.test.sh` 12/12. Committed `1c76c41`.

## ROADMAP C2 — THE CORPUS EXECUTES (2026-08-13) — GATE PASSED

**The finding that shaped the work: the corpus ran 1 of 9 because only case `a` carried complete
geometry, not because the interpreter was thin.** Case `b` extrudes `sk_base.region.r0`,
`sk_cut.region.r0`, `sk_2` and `sk_3` — four sketches its `opScript` never authors. Case `c`
references `sk_base` and mixes three independent C++ documents in one script. Case `i`'s entities
were prose: `"4 lines: (0,0)(10,0)(10,5)(0,5)"`. The numbers and citations were all there; the
runnable geometry was not.

Decision taken with the user: **enrich the cases** (rather than hiding the geometry in Rust or
recording the mismatch and moving on), because every footprint is READ BACK OUT of a frozen
assertion — `afterCut 3750 = 4000 − 5·5·10` fixes the base box at 20×20×10 and the cut at 5×5
through-all — so nothing is invented, and each addition carries its own `entitiesProvenance`.

- [x] **Case `i` is executable**: the four loop-detector scenarios carry SCHEMA §7.4 entities, with
      the original prose kept as `sketch.description`. Rectangle, square-with-hole, arc+chord and
      ellipse run through `SketchUpsert` → `SketchRegions`, the same lane a profile pick uses.
- [x] **Case `b` is executable**: its four sketches are authored, every op declares the `scenario`
      it belongs to (the C++ oracles are SEPARATE documents — running them in one timeline would
      boolean unrelated bodies together), and `bodyLabels` says which op mints each referenced
      label. ThroughAll cut → 3750, two-direction → 800, symmetric → 800 (derived-not-asserted,
      still labelled so).
- [x] **A DELIBERATE divergence, recorded rather than papered over.** The square-with-hole scenario
      detects TWO regions on the new stack (annulus + inner square) where the C++ LoopDetector
      reported one face with an inner loop. That is the planar-cell model `wire_contract.rs`
      already pins (`nested_inner_disk_parity_and_reopen_stability`), so the case now carries a
      `newStack` block citing it and the runner asserts the current contract while the frozen
      number stays visible.
- [x] **Case `a` asserts its whole expected block**, not just the last volume: the sketch step's
      region count, the extrude's body-lifecycle events, the solid count, the volume, AND the
      `faceCount` the case records as derived-not-asserted — which is the assertion that catches a
      prism built from the wrong profile at the right volume.
- [x] **Classification is manifest-driven.** The executor READS `corpusCases` from the coverage
      manifest, so the two artifacts cannot disagree; a case the manifest does not classify, and a
      manifest entry with no corpus file, are both errors.
- [x] **Structure + provenance are checked for all nine on every machine** (the worker-free half):
      `source[]` non-empty and naming a file, an `opScript`, `bodyLabels` pointing at real ops, and
      every scalar measurement carrying a citation — or an explicit `confidence`, which is how the
      corpus says "derived, not captured".
- [x] **A stale unsupported reason is now an error.** Six cases still carry one, each naming the
      machinery it needs (face-hosted sketches for `c`, MESH1 edge picking for `d`/`e`, a
      descriptor-tie fixture for `f`, the gesture lane for `g`, a rollback-cursor harness for `h`).
      A reason kept for a case that has become executable fails the test.
- [x] **Non-vacuity proved by mutation**: 2000→2500, 3750→3700 and regions 1→3 each red the run
      with the right message; the corpus was restored byte-identical afterwards.

Gates: `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1082 / 0** · fmt · clippy `-D warnings` ·
both modeling verifiers · negative controls 12/12 · every corpus file still parses.

**Next:** C3 (WP4.6 has zero tests — nothing asserts a structured diagnostic reaches the inspector)
and C4 (deepen the thin verticals, which is where the four measured limits above are closed).

## ROADMAP A4 · A5 · A6 — THE REST OF TRACK A (2026-08-13) — GATE PASSED

One gate, three items, and **two of them turned out not to be evidence debt at all**: the mandated
tests found live defects in shipped code, which is the whole reason the roadmap asks for them.

### A4 — Revolve body-edge axis: hidden, and now provable either way

The typed `AxisRef::Element` variant is complete in the worker (`RevolveOp.cpp:129-201`) and core,
and the UI authors only `sketchLine`. The contract row claimed `uiExposure:"exposed"` — false for a
whole phase, because no code path could produce one. **Product call: it stays hidden**, following
the `MirrorBody / fuse` precedent (`supported` + `hidden`) and WP1.5's own instruction not to expose
it before the persistence/reopen/upstream-edit evidence exists.

- [x] **The four WP1.5 tests.** Two are worker-level refusals (`worker/tests/test_wp6_ops.cpp`): a
      CURVED axis edge is `OP_FAILED` "axis edge must be a straight line" and explicitly NOT
      NeedsRepair (the edge resolved fine, it is simply not an axis), and a TWO-BODY fixture proves
      both ownership gates — a ref naming another body, and an elementId that agrees on paper but is
      bound to the other body in the partition. Two need a real document
      (`src-tauri/tests/revolve_ops.rs`): promote → revolve → **upstream edit** → **save/reopen**,
      and an axis edge CONSUMED by an upstream fillet ⇒ `NeedsRepair`, no body published.
- [x] **Non-vacuity, measured.** Removing the partition-ownership gate makes the revolve SUCCEED on
      the other body's edge — a silent cross-body axis swap; removing the curved-edge guard silently
      straightens a circle. The consumed-edge test carries an in-test NEGATIVE CONTROL (the same
      document without the fillet publishes cleanly), because the mutation that would prove it lives
      in `resolve_input_refs`, which returns no candidates on that path.
- [x] **A measured finding worth keeping.** An upstream edit that grows the axis edge +20% rebinds;
      +100% is `NeedsRepair` (ambiguous). A FILLET ref on the same edge answers identically at both
      sizes — checked, not assumed — so this is the shared ladder's descriptor-magnitude policy
      (auto-bind needs score ≥0.85 AND margin ≥0.10), not something about axes. The test pins both
      directions, so a future policy change has to be deliberate.
- [x] **The row now reads `hidden`**, and `ModelToolController.revolveAxisExposure.test.ts` binds the
      claim to the code from both sides: no lane authors an `edge` axis, and a record that ALREADY
      holds one survives a re-edit (hidden must not mean destroyed — the angle-only deep merge sends
      no `axis`; adding one reds the test). The frozen interaction contract was NOT edited: its
      revolve row already says `requiredSelections: "sketch region + axis line"`.

### A5 — WP0.7 preview ownership (a real defect) and WP0.8 Boolean re-arm

- [x] **WP0.7 was half-landed, and the missing half was the failure state.** Ownership of candidate
      bodies and visibility claims was per-session; `previewFailure` was ONE field. A secondary
      session's refusal set it, and only the PRIMARY branch ever cleared it — so a region that failed
      and then recovered left every commit blocked behind a stale error hint contradicting a preview
      that visibly worked again. `ToolPreviewSession.failure` is now per session, the lane's failure
      is the union, and a recovered secondary takes the status line back exactly like the primary.
- [x] **`ModelToolController.previewOwnership.test.ts`** delivers two region responses in BOTH
      orders (both candidates survive, replaced-body claims union), fails one secondary (only its own
      candidate and claim drop), then RECOVERS it and requires the commit to go through — red before
      the fix. A still-failing session keeping the commit blocked is the negative control. No existing
      harness could express this: the multi-region specs stub `onPreviewResult` away and one of them
      hands every `beginPreview` the same constant sessionId.
- [x] **WP0.8's two untested paths + the second Apply.** `booleanPreview.test.ts` covered the
      exact-preview barrier only. Added: a REJECTED `endPreview`, a RESOLVED regen failure (one
      `undo` per applied-but-failed attempt), a terminal-only failure with no `errorMessage`, and a
      successful SECOND Apply that commits the session the re-arm opened (`pv-2`), not the consumed
      one. Reverting `commitBoolean` to the old `errorMessage`-only check reds the terminal-only case.

### A6 — the two cross-track fixtures, one of which needed the contract to become true first

- [x] **SCHEMA §7.5's snapshot echo was normative and implemented nowhere.** Every resolution is
      required to carry `{snapshotId, revision, refId, bodyId}`, and a client must cache candidates
      by `{revision, snapshotId, refId}`. Neither the C++ worker nor the Rust stub emitted any of the
      three; `api/mod.rs` manufactured all of them from Rust's own state and `manager.rs` validated
      nothing — so a resolution computed on an older snapshot was cached under a freshly minted key.
      The worker now echoes them on every branch (`bodyId` only when a body was enumerated), the stub
      matches, and `wire::validate_resolve_refs_result` fails closed on a mismatched snapshot, a
      re-ordered `refId`, a wrong arity or a missing echo.
- [x] **`revision` stays Rust-owned in the DTO, deliberately** (decision D4): the repair store keys
      candidates on the same `(revision, snapshotId)` the `needs-repair` events carry, and the
      engine's own stamp legitimately lags an un-regenerated edit. The SNAPSHOT is the engine's echo.
      Recorded in SCHEMA §7.5 so the next reader does not have to re-derive it.
- [x] **`protocol/fixtures/resolve_refs_snapshot_echo.ndjson`** (new) covers the direct-hit and
      missing-body branches plus a stale-snapshot refusal; `bind_element_ids.ndjson` now asserts the
      echo on its own ResolveRefs step. Dropping the `revision` echo in the worker reds
      `canonical_resolve_refs_snapshot_echo`.
- [x] **`protocol/fixtures/circular_pattern_lineage.ndjson`** (new) pins Pattern V2 lineage on the
      wire: `count−1` children `body_<opId>:<k>`, the source preserved as instance zero with no
      lifecycle event, and the `perStepResults` body-id set. It deliberately does NOT pin the
      `angleDeg / count` step angle — NDJSON carries no geometry to measure it with, so that stays in
      `test_m6a_ops.cpp` and `patternPreview.test.ts`. Said so in the fixture header rather than
      asserting it by proxy.
- [x] **Fixture discovery is now ENUMERATED in both lanes**, which was the actual root cause of A6's
      gap: `boolean_empty_refusal.ndjson` (added last commit) ran in the ctest lane and in neither
      hardcoded list. `check_interop.sh` globs `protocol/fixtures/*.ndjson` (and fails on an empty
      glob) and the Rust parse test reads the directory with a ≥6 floor.

Gates: `ctest` **119/119** (117 → 119); `cargo fmt --all --check`; `clippy --workspace --all-targets
-D warnings`; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1082 / 0** (1078 → 1082);
`bunx tsc --noEmit`; `bun run build`; `bun run test` **249 files / 4173 tests** (247/4161 → 249/4173);
both `verify-modeling-*` verifiers; hex gate 0; targeted Playwright on the touched preview lanes
(`multiregion`, `boolean-preview`, `extrude-commit-gesture`, `extrude-multiselect`) **20/20** across
Chromium + WebKit at `retries: 0`. Kernelbench was NOT re-run: no benchmark, fillet or kernel-geometry
source changed (the worker delta is `ElementIdentity.cpp` plus test registration).

**Next:** C1 — make the coverage manifest true, then give `verify-modeling-coverage.mjs` teeth. Two
inputs for it are already on the table: it has no `body-edge axis` row at all, and its single Revolve
row cites `src/tools/modelTools/revolve.test.ts`, which does not exist.

## ROADMAP A1 — SHARED OPERATION-RESULT CLASSIFIER (2026-08-13, plan `now-lets-plan-next-sunny-lighthouse.md`) — GATE PASSED

Roadmap WP0.3, finished. It was recorded complete at `MODEL-CORRECTNESS-P0` but only the transport half had landed: `RegenTerminal` declared `needsRepair` and **neither client ever assigned it**, no shared consumer helper existed, `.terminal` was read nowhere outside its own test, and every consumer family still inferred success from body counts — the inference the spec explicitly forbids. That was the fix for risk **R-04** (severity 5, score 80), so it was a live defect, not missing evidence.

- [x] **`needsRepair` is now produced.** `regen-finished` gained `repairSteps` (record ids a published regen left in NeedsRepair), populated in `emit_regen_events` from the same `report.needs_repair` the sibling `needs-repair` event uses. It rides THIS payload because `needs-repair` is emitted AFTER `regen-finished`, so an awaiter settling there would always miss it. `tauriClient`'s recordId awaiter settles `needsRepair` when its own record appears.
- [x] **`src/ipc/regenOutcome.ts`** — one `classifyRegen()` consumed by every family, plus `keepsRecord()` and `failureReason()`. A resolved `errorMessage` is failure whatever the terminal claims (checked first). A result with no `terminal` falls back to the historical body-count inference verbatim, so legacy fixtures are unchanged.
- [x] **Two outcomes body counting got backwards are now right**: an empty *published* result and a *delete-only* result are no longer failures-with-rollback, and `needsRepair` is no longer a success.
- [x] **Consumers migrated**: `commitPattern`, the exact-preview fallback commit, the shell/edge commit tail and the boolean commit in `ModelToolController`; `suppressFeature`/`rollToIndex`/`deleteFeature`/`rebindCandidate` in `historyActions.ts` — the last four hydrated and announced success while catching only a rejected promise, so a resolved `errorMessage` printed "Feature deleted".
- [x] **A NeedsRepair record is never rolled back.** `keepsRecord()` covers published/noop/needsRepair; rollback stays for failed/timeout only. Rolling back the record repair operates on would delete the thing the user is about to fix.
- [x] **The table-driven test the spec asks for**, in two files over one shared table so the families cannot drift: `src/features/inspector/regenTerminals.test.ts` (3 families × 6 rows) and `src/tools/modelTools/ModelToolController.terminals.test.ts`. Asserts no success hint on error, one sticky diagnostic, no duplicate record on retry, authored values recoverable, and selection never moved onto a body that failed to publish.
- [x] **Both tables proved non-vacuous by mutation.** Bypassing the classifier in `historyActions.settle` reds 9 rows; reverting `commitPattern` to the `errorMessage`-only check reds 4 and reproduces R-04 verbatim — `needsRepair` and a message-less `failed` both printed the success hint "Linear pattern ×7".

Gates: `bunx tsc --noEmit` clean; `bun run test` **247 files / 4161 tests** (244/4124 → +3 files, +37 tests); `bun run build` clean; `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -D warnings`; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1076 / 0**. Worker and ctest untouched by this wave.

## ROADMAP A2 — DRAFT APPLIED-OR-REFUSED (2026-08-13) — GATE PASSED

Roadmap WP0.6. The implementation was already correct — three named refusals plus a semantic volume-delta check (`ExtrudeOp.cpp:248-302`) — but **no test pinned any of the three strings**, and the work package's FIRST mandated task, the circular-profile red probe, had never been run. The only draft coverage was one square prism asserting `500 < v < 990`, which no refusal path and no wrong angle could fail.

- [x] **The mandated red probe ran, and the answer is REFUSAL.** A circular profile extrudes to a cylinder whose only side face is curved, so `apply_draft` finds zero eligible faces and returns `Extrude draft refused: no eligible planar side faces`. The spec's escalation condition — "if OCCT returns success with unchanged geometry, promote to a confirmed P0 defect" — **does not fire**. Risk **R-10** is closed as not-present, with evidence rather than by inspection. The probe asserts both branches, so it still catches a silent no-op if the behaviour ever changes.
- [x] **`worker/tests/test_extrude_draft.cpp`** (new ctest `extrude_draft`, 117/117): closed-form frustum volume for ±10° (`V = h/3·(A₁+A₂+√(A₁A₂))`, matched to 0.5 mm³ — a wrong angle, a wrong neutral plane or a one-sided taper all fail it), the sign genuinely reversing the taper, the neutral plane keeping the base footprint, ±89° near-limit safe-refusal, a sub-epsilon angle staying a clean straight prism, and determinism across two runs.
- [x] **`src-tauri/tests/preview_extrude_draft.rs`** (real worker, +2): the drag shows the drafted frustum (preview **688.801** vs closed form **688.801**), the preview leaves the head byte-identical, the commit lands on the previewed volume, and a refused draft **refuses in the preview lane too** rather than showing the straight cylinder the commit would never publish.

**TWO FINDINGS, recorded not fixed** (both outside A2's scope):
- **`Arc` entities still reach the BRep as polylines while `Circle` stays analytic.** The slot probe reports 28 faces (2 flanks + **24** cap segments + top + bottom) and a volume 0.19% under analytic `(400+25π)·10` — exactly a 24-gon inscribed in the caps. So the "mixed planar/curved" case is not yet mixed at the BRep level, and this is direct evidence for the Phase 2 residual the plan tracks as B3 ("no polygon-fallback warning for supported analytic entities" is asserted by nothing).
- **The refusal already carries structured evidence.** The preview lane returned `Diagnostic { code: "OP_FAILED", stage: "build", … }`, so A3's remaining work on draft is the stable per-defect CODE, not the diagnostic envelope.

Gates: `ctest` **117/117** (116 → 117); `cargo fmt --all --check`; `clippy --workspace --all-targets -D warnings`; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1078 / 0** (1076 → 1078). No frontend file changed.

## ROADMAP A3 — STABLE DIAGNOSTIC CODES FOR THE P0 REFUSALS (2026-08-13) — GATE PASSED

Roadmap WP0.6 "Diagnostics". Phase 0 shipped two new refusals — zero-solid Boolean and Draft — and both returned a bare `OP_FAILED` with the reason only in the message. A caller wanting to tell "no planar wall to draft" from "the kernel rejected the walls I offered", or "your Cut consumed the target completely" from any other Boolean failure, had to match on message TEXT. That is exactly the message-text routing the diagnostics contract forbids.

Follows the established `EDGE_OP_TANGENT_CLOSURE_CHANGED` precedent: the §8 top-level code stays `OP_FAILED`, and the DIAGNOSTIC carries the stable per-defect code, `stage`, and bounded evidence.

- [x] **Draft vocabulary** (`stage:"build"`, evidence `{draft:{angleDeg,eligibleFaces,addedFaces}}`): `EXTRUDE_DRAFT_NO_PLANAR_FACE`, `EXTRUDE_DRAFT_NO_FACE_ACCEPTED`, `EXTRUDE_DRAFT_NO_CHANGE` (adds `volumeBefore`/`volumeAfter`), `EXTRUDE_DRAFT_BUILD_FAILED`. `apply_draft` now returns a `DraftFailure{code,message,evidence}` instead of a bare string.
- [x] **`BOOLEAN_EMPTY_RESULT`** (`stage:"publish"`, evidence `{boolean:{operation,targetBodyId,toolBodyId,solidCount:0}}`).
- [x] **The codes DISCRIMINATE, and that is asserted** — a vocabulary whose members never differ would be `OP_FAILED` spelled longer. `distinct_defects_get_distinct_codes` runs a no-planar-wall profile and a self-intersecting 89° taper and requires different codes: measured `EXTRUDE_DRAFT_NO_PLANAR_FACE` vs `EXTRUDE_DRAFT_BUILD_FAILED`.
- [x] **Both lanes agree.** `preview_extrude_draft.rs` pins that the PREVIEW refusal carries the same `EXTRUDE_DRAFT_NO_PLANAR_FACE` the commit does (SCHEMA §7.6 requires byte-equivalent diagnostics for the same candidate).
- [x] **Cross-track fixture extended** — `protocol/fixtures/boolean_empty_refusal.ndjson` now asserts the diagnostic on its refused step. This is one of the fixtures the Phase 3 review flagged as never extended. **Verified non-vacuous**: substituting a wrong code reds `canonical_boolean_empty_refusal`.
- [x] **SCHEMA §7.3 + §14 changelog** record both vocabularies as diagnostic-code-only additions; the top-level code is unchanged, so every existing fixture stays byte-valid.

Gates: `ctest` **117/117**; `cargo fmt --all --check`; `clippy --workspace --all-targets -D warnings`; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1078 / 0**. No frontend file changed.

- [ ] Next per the plan: A4 (Revolve body-edge axis — the four required tests, then reconcile the contract row that claims `uiExposure:"exposed"` against `src/ipc/types.ts:843`).

## MODELING UX HARDENING — WP0 through WP6 (2026-08-12/13, plan `plans/modeling-ux-wp0.md` + `act-as-senior-software-linked-graham.md`) — FE GATE PASSED, ALL 19 DEFECTS CLOSED

WP0 (red evidence, no production changes): frozen `src/test/contracts/modelingInteractionContract.ts` + 11 new red tests, one per confirmed defect (D7,D8,D9,D10,D11,D12,D13,D14,D17,D19 — 17 test cases). WP1+WP2 (2026-08-12) closed D9/D1/D2/D6(partial)/D13/D17; a follow-up pass (2026-08-13) closed the remaining D7/D8/D10/D11/D12/D14/D19. D6's structural refactor (`ToolChipState` discriminated union) remains the sole deliberate non-goal — no red test forces it, pure internal type-safety, its own dedicated pass.

- [x] **D9** — `commitPattern` and `commitPreviewedOp`'s fallback branch check `res.errorMessage` before treating a resolved result as success.
- [x] **D17 + D1** — boolean/linearPattern/circularPattern/mirror chips gained a visible ✕ (`CancelButton`), wired through `toolChipStore`'s `onCancel`.
- [x] **D2** — click-away commit removed entirely (all tools).
- [x] **D13** — shell re-edit chip no longer auto-commits on value change; `onConfirm` is the sole commit trigger.
- [x] **D14** — `sketchStatusText`/`sketchStatusSentence` treat DOF 0 as fully constrained regardless of a lagging solver status label.
- [x] **D11** — `repairStore.applyEvent` orders lexicographically on `(revision, snapshotId)`, not `revision` alone.
- [x] **D12** — `rebindCandidate` prefers `candidate.bodyId` (denormalized by `RepairPanel` from `ResolveRefResult.bodyId`, new field on the FE-only `ResolveCandidate` type — NOT a wire/dto.rs change) over the possibly-stale `deriveOperatedBody(item)`. Test fixture had two latent gaps fixed alongside: no seeded `features` entry for opId `"op1"` (path resolution returned null, so `promoteOne` was never reached — 0 calls, not a bodyId mismatch) and no `bodyId` on the candidate literal.
- [x] **D7** — `CircularPatternFsm` gained an `origin: [number,number,number]` field, threaded through `armCircular`'s new `seedOrigin` param and `editCircularPatternFeature` (`storedVec3(stored?.axisOrigin)`); `commitCircular` sends `this.circular.origin` instead of a hardcoded `[0,0,0]`.
- [x] **D8** — `MirrorFsm` gained `planePoint`; `armMirror` gained `seedPlanePoint`/`fuseWithOriginal` params (the latter reuses the existing shared `this.patternFuseResult` field, same as linear/circular); `editMirrorFeature` seeds both from stored params; `commitMirror` sends real values instead of hardcoded `[0,0,0]`/`false`.
- [x] **D10 + D19** — `commitPattern` and `commitTransform` now select `res.changedBodies` minus the original source/target ids (the newly created children/copies), falling back to the sources when nothing new was created (a fused-in-place result) — same policy in both, mirroring `finishExtrude`/`finishRevolve`'s changedBodies-derived selection.

Regressions caught and fixed during the WP1+WP2 pass (pre-existing tests that encoded now-removed behavior, not covered by the WP0 red set): `ModelToolController.regionPick.test.ts` "(a2) revolve two regions..." asserted a click-away commit — swapped for an explicit Enter. `ModelToolController.edgeShellPreview.test.ts` "shell re-edit's result hint SURVIVES..." called `onValue` alone expecting a commit — added the `onConfirm` call a real Enter also fires.

Gates: `bun run test` 244/244 files, 4124/4124 tests green; `bunx tsc --noEmit` clean; `bun run build` clean.

## MODELING CORRECTNESS P3 — PUBLICATION POLICY (2026-08-12) — COMPLETE

- [x] Machine-readable per-operation contract rows: `docs/qa/modeling-operation-contracts.json` (35 rows / 16 operations) covering support status, validation tier, body lifecycle, empty/multi-solid semantics, and `uiExposure`.
- [x] Transform policy row; wired `TransformBody` through the common `publication_decision` Tier A validator (`worker/src/ops/TransformOp.cpp`).
- [x] ImportStep policy row and explicit invalid-solid warning exception documented in contracts.
- [x] UI mode disposition recorded: Revolve Intersect, Mirror fuse, Extrude Intersect hidden; Pattern fuse hidden; Transform move/copy exposed; OffsetFace Total/Diameter deferred.
- [x] `scripts/verify-modeling-contracts.mjs` validates schema, required fields, uniqueness, and coverage-manifest cross-reference.

Gates: worker Release build; CTest 113/113 passed; `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` passed; contract/coverage verifiers passed.

## MODELING CORRECTNESS P4 — BOOLEAN INTERSECT VERTICAL (2026-08-12) — COMPLETE

- [x] C++ standalone fixtures: overlap, containment, identity, face/edge/vertex touching refusal (`worker/tests/test_boolean_intersect.cpp`). CTest 114/114.
- [x] Rust real-worker tests: preview/commit parity, disjoint refusal, save/reopen, undo restoration (`src-tauri/tests/preview_boolean.rs`).
- [x] Frontend + Playwright: Intersect tool selection, target/tool pick, Apply, single body row; added `data-testid` to boolean op chip buttons.

## MODELING CORRECTNESS P4 — REMAINING COVERAGE (2026-08-12) — COMPLETE

- [x] MirrorBody Playwright flow (`e2e/mirror-body.spec.ts`).
- [x] Linear/Circular Pattern Playwright flows (`e2e/linear-pattern.spec.ts`, `e2e/circular-pattern.spec.ts`).
- [x] Critical mode closure tests: Extrude/Revolve overflow hide Intersect; Mirror/Pattern chips have no fuse/union toggle (`src/features/toolbar/ModelToolChips.test.tsx`).
- [x] Real-worker corpus executor (`src-tauri/tests/corpus_executor.rs`): enumerates all `corpus/cases/*.json`, executes `a_sketch_extrude_blind` end-to-end (volume within tolerance), and records explicit unsupported reasons for the rest. Zero unclassified files.

Gates: worker Release build; CTest 114/114; `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` passed; TypeScript / Vitest passed; contract/coverage verifiers passed; targeted Playwright boolean/pattern/mirror specs 12/12 (Chromium + WebKit, retries 0). Full Playwright suite not re-run yet; prior unrelated `unsaved-guard` / `view-ux` flakes remain the only known blockers.

## MODELING CORRECTNESS P2 GATE (2026-08-12) — AUTOMATED LANES PASSED; MANUAL TAURI SMOKE OPEN

- [x] Build worker Release against pinned OCCT 8.0.1 fingerprint `0a6a1dce34181289`.
- [x] CTest 113/113 passed.
- [x] Cargo fmt/clippy/workspace tests with `ONECAD_REQUIRE_WORKER=1` all passed.
- [x] Kernelbench T0 both backends: 136 records, 0 gating failures, replay 136 stable, metamorph 48 passed, differential same-status 136. Summary matches baseline except timing.
- [x] TypeScript check, production build, Vitest 241 files / 4116 tests passed.
- [x] Playwright zero retries: Chromium 196/196 passed, WebKit 196/196 passed.
- [ ] Manual Tauri smoke (open project → extrude → fillet → undo → save → reopen) still owed; no automated equivalent exists.

Fixed P2 region-identity regression found by real-worker gate:
- `src-tauri/tests/sketch_on_face.rs`: `region_extrude_record` now authors `region_identity_version: 2` so fragmented projected regions resolve under the V2 exact-profile detector.
- `src-tauri/tests/wire_contract.rs`: `nested_inner_disk_parity_and_reopen_stability` now sets `region_identity_version: 2` when binding the exact disk/annulus region ids.

## MODEL-CORRECTNESS-P0 + REF-OWNERSHIP-AND-SNAPSHOT P1 (2026-08-11) — COMPLETE

- [x] P0 — deferred replacement guard; authoritative `SaveOutcome`; classified terminals; zero-solid Boolean refusal; circular `angle / count`; semantic Draft refusal; per-session preview ownership; Boolean re-arm.
- [x] P1A — typed-ref local ownership before ladder scoring; stale ToFace/Hole promotion and OffsetFace adoption fences.
- [x] P1B — provenance-versioned repair candidates; additive typed Revolve body-edge axis with no ordinal fallback.
  - [x] `ResolveRefs` echoes `{revision, snapshotId, refId, bodyId}`; candidate loads key on that tuple, old events/loads/clicks cannot promote stale ordinals. Focused FE 32/32; Rust lib 256/256.
- [~] Gates — worker Release + CTest 112/112; Cargo fmt/clippy/workspace worker tests; `npx tsc --noEmit`; Bun build; Vitest 241 files / 4115 tests all pass. Full Playwright retries 0: Chromium 196/196, WebKit 196/196 after `commitExtrudeAtHandle` releases a failed retry pointerdown. Manual Tauri smoke and T0 digest/semantic campaign remain open.

Gate evidence: baseline `1c11d49`; `scripts/build-worker.sh Release`; `ctest --test-dir worker/build --output-on-failure`; `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 cargo test --workspace`; `npx tsc --noEmit`; `bun run build`; `bun run test`; `bunx playwright test e2e/repair-rebind-multibody.spec.ts e2e/sketch-multi-object.spec.ts --project=chromium --project=webkit --retries=0`. Untracked roadmap bundle intentionally untouched.

## TRACK A — a CI that means something (2026-08-10)

Goal: before any R1 work, make "green" a fact CI enforces rather than a claim verified by hand. Four work packages, all landed.

### A1 — the inline-edit test was a REAL race, not a slow deadline
- [x] **My first diagnosis was wrong and the second attempt found the mechanism.** I read `InspectorPanel.test.tsx`'s failure as an arbitrary wall-clock deadline and widened it; it went red again in CI **with a 4 s budget**, on an assertion whose work is pure microtasks — which a timeout cannot explain.
- [x] Actual cause: `DimensionInput.commit()` reads the `text` STATE and calls `onCommit` only when the formatted new value differs from the current one. Firing `change` and `keyDown` inside ONE `act` nests their flushes into the outer scope, so the `keyDown` handler can still close over the pre-change text — `formatValue(n) === formatValue(value)` holds, the commit is CORRECTLY skipped, and `applyEditCommand` never fires. Split into two `act` scopes so Enter runs against a component that has re-rendered.

### A2 — the 12 red e2e specs were THREE unrelated causes, not one
The plan (and `TODO.md`) attributed all 12 to the live-dim tree. Wrong on 10 of them.
- [x] **Toolbar flyout families (8 of 12).** `d81f758` made the toolbar one slot per family, so Center rectangle (behind Rectangle) and Ellipse (behind Circle) have no button until picked from a flyout. `selectSketchTool` now tries the direct button, then DISCOVERS the owning flyout by opening each chevron rather than hardcoding a family table. Menu rows concatenate title+shortcut with no separator (`"EllipseO"`), so the title span is matched exactly. Note for future helpers: **`count()` does not auto-wait** and must never be the first thing asked of a mounting toolbar.
- [x] **Extrude gesture + a wrong precondition (2).** Confirming from the armed state leaves depth at zero and nothing commits (UNIFY-UX), so a real handle grab is required — restored as a shared `dragExtrudeDepth`, filling an orphaned docstring that had lost its function. The body-COUNT precondition was wrong in EITHER drag direction: the rectangle overlaps the seeded body, so auto Add/Cut resolves to `Cut` and the extrude modifies `body1`. Correct behaviour, irrelevant to reattach — the precondition now asserts a moved revision.
- [x] **The angle change (2), and production is right.** The chip is withheld on a first segment because `angleLadder` authors nothing without a `prev`, so a drivable-looking chip would be a lie. The spec now types on a chained second leg. Two things the rewrite had to learn from the code: the field types the **visual corner angle**, not the raw turn (`cornerAngleOf = 180 - |turn|`, so a typed 30 is a 30° corner and a 150° turn), and geometry cannot be asserted with absolute headings — the plane's +U runs opposite to screen +x and leg 2 is stored end-first — so it measures the undirected corner at the shared vertex.
- [x] **Full Playwright suite 392/392** locally, both browsers, run alone.

### A3 — `worker-7.9.3` deleted
- [x] It never compiled (four benchmark sites call `failure.what()`, absent from 7.9.3's `Standard_Failure`) and `continue-on-error` masked that on EVERY run including green ones. The two persistence jobs stay: different question, and they build only `test_occt_persistence`, which never touches the benchmark sources.

### A4 — `ci.yml` split into two lanes, self-hosted + macOS shipping gate
- [x] **Self-hosted (Linux):** `linux-worker` (hygiene, build, selftest+fingerprint, ctest 110/110, edge-op determinism) and `linux-kernelbench` (`-p onecad-kernelbench`, T0 both backends, linux-x64 digest gate, cross-host semantics gate). Uses the PERSISTENT OCCT prefix, so the 40-90 min kernel build is paid once.
- [x] **macOS shipping gate:** packaging linkage smoke (no Linux equivalent), `cargo test --workspace`, both e2e projects, persistence pair.
- [x] **SECURITY:** every self-hosted job is gated to trusted code — a push to this repo, or a PR whose head branch lives here. Fork PRs get the full GitHub-hosted lane. User has since set fork-PR approval in Settings → Actions as the backstop.
- [x] **e2e is now gated in CI** — chromium and webkit, 392 executions.

### The finding that changed a design decision: digests are SAME-MACHINE
- [x] The macOS digest gate failed comparing **darwin-arm64 against a darwin-arm64 baseline**: same pinned OCCT source, same build id, same architecture, but GitHub's `macos-14` AppleClang is not the laptop's — and the trig-heavy `valence4-*` family moved. So platform-keying (yesterday's conclusion) is still too coarse.
- [x] The digest gate therefore runs **only on the self-hosted runner**, the one persistent machine. Both lanes gate the portable thing: **semantics**. `bench/robustness/baselines/README.md` records the measurement.

### Two CI-only flakes, both "a local-machine number standing in for a condition"
- [x] `ModelToolController.wave2` asserted straight after `flush()` — a single `setTimeout(0)`, ONE macrotask tick — while `editExtrudeFeature` awaits `endPreview` then `beginPreview`. Passed 18/18 in isolation, red in CI. Now waits for the condition.
- [x] `boolean-preview`'s lane polls carried 5 s; the lane opens behind a body pick, a Combine arm and a preview round-trip. It passed on both browsers in one CI run and failed on both in the next — a deadline flake. Raised to 20 s behind a named constant, assertions unchanged.

### Still open
- [ ] **Runner root installs** (Proxmox SSH was refusing connections, so these could not be done): `unzip` (blocks `setup-bun`, which is why `frontend` is GitHub-hosted) and Playwright's chromium libraries. The exact one-liner is in `ci.yml` above `e2e-chromium`. Both jobs move to the runner once they land.
- [ ] **`boolean-preview` on ubuntu chromium** fails a body pick that succeeds on macOS chromium — a plausible SwiftShader/GL difference and a real finding. Chromium runs on macOS meanwhile; investigating this is what would let the whole e2e lane move to the runner.
- [ ] **OPEN DECISION — `retries` in CI.** With e2e gated, the suite runs ~99.5% clean: 195/196 per job, with a DIFFERENT spec failing each run (`boolean-preview`, `tree-visibility`, live-dim…). Every one so far has been a local-machine deadline standing in for a condition, and each is individually fixable — but it is a long tail. `playwright.config.ts` sets `retries: 0` deliberately, with the rationale that retries once hid flakes that were invisible in CI and hard-red locally (`TODO.md` OPEN DECISION, and the config's own comment). That rationale predates e2e being a gate at all. Choose: keep 0 and grind the tail down, or `retries: 1` on CI only and keep grinding without a red gate.
- [ ] Nothing is a REQUIRED check yet — worth turning on once the retries decision lands, so the gate actually blocks.

## SELF-HOSTED RUNNER — Linux benchmark host, S1-S4 ladder (2026-08-10) — GATE PASSED

`.github/workflows/self-hosted.yml`, `workflow_dispatch` ONLY. Runner `prx-lxc` = Proxmox LXC 107 "GithubRunner", Debian 13 trixie, unprivileged, **4 cores / 8 GB / 70 GB**. Purpose: long campaigns move off the Mac (wall-clock is free there); the Mac keeps the fast iteration lane.

- [x] **SECURITY — the repo is PUBLIC and the runner is on the home LAN.** Self-hosted jobs are in their own workflow because `ci.yml` triggers on unfiltered `pull_request:`; a fork PR reaching a self-hosted job executes the fork's code on that hardware. Every job additionally re-checks `github.repository` + `github.event_name`. Never add `pull_request`/`pull_request_target` there. 0 forks today.
- [x] **S1 environment report.** Container shipped with only curl/tar/python3/perl and **no sudo**. One-time root install via `pct exec 107`: `git ca-certificates build-essential cmake ninja-build pkg-config libboost-dev libeigen3-dev nlohmann-json3-dev`, then `libx11-dev libxext-dev libxmu-dev libxi-dev libgl-dev libglu1-mesa-dev`. gcc 14.2.0 · cmake 3.31.6 · ninja 1.12.1. (`git`'s absence is not cosmetic: `actions/checkout` silently falls back to the REST tarball, leaving no `.git`.)
- [x] **S2 pinned OCCT 8.0.1 from source.** Artifact provenance byte-identical to macOS (same `sourceCommit`, `buildId`, normalized option list). 120 MB prefix / 49 `libTK*.so`. Persistent prefix outside the workspace + an assert that a second invocation prints "Reusing pinned OCCT", so the 40-90 min build is paid once.
  - OCCT's `TKService` compiles against Xlib on Linux where macOS uses Cocoa. Installing X11 headers is the correct fix, NOT `USE_XLIB=OFF`: `HAVE_XLIB` is auto-detected and absent from the pinned option policy, so headers cannot move the fingerprint, whereas suppressing the module either edits the policy or lets two materially different builds share one fingerprint.
- [x] **S3 worker + ctest.** **`ctest` 110/110** and **`fingerprint 0a6a1dce34181289` identical to macOS** — the seed is `occtVersion|sourceCommit|buildOptions|buildId|kernelPolicyVersion`, all platform-independent, and that design now has evidence. Edge-op determinism `cmp` byte-identical.
- [x] **S4 kernelbench T0, both backends.** 136 records · `gatingFailures` 0 · replay 136 stable / 0 unstable · metamorph 48 passed / 0 failed · differential 136 same-status — semantically identical to the macOS baseline. Timing **p50 29.7 ms / p95 993 ms** vs macOS 10.3 / 62.3 (4 shared LXC cores); M5 sizing must use these, not the Mac numbers.

### Three repo defects the port surfaced (all fixed)
- [x] **`json_fwd.hpp` was never vendored** next to `json.hpp`, and nothing declares it — there is no `find_package(nlohmann_json)` in the build. Six benchmark headers resolved it from the system. macOS was green only because Homebrew also ships 3.12.0; Debian's 3.11.3 puts a second inline ABI namespace into `nlohmann` and every `json_pointer` reference goes ambiguous. **This was a live latent failure on the shipping platform** — any Homebrew bump past 3.12.0 breaks macOS identically. `VENDOR.txt` already stated the invariant this restores.
- [x] **`DT_RUNPATH` is not transitive.** GNU ld defaults to `--enable-new-dtags`, so the OCCT path resolved only the worker's own `DT_NEEDED` entries; all 26 OCCT-internal edges (`libTKOffset -> libTKG2d`) fell through to the system path and the worker died at startup. OCCT's libraries carry no RPATH of their own. `-Wl,--disable-new-dtags` under `if(NOT APPLE)` restores transitive `DT_RPATH`; the macOS link line is unchanged.
- [x] **`std::reverse` without `<algorithm>`** in `test_polygon_fill.cpp` — libc++ transitive, libstdc++ not. FLAGGED: 34 more files use `std::u?int*_t` without `<cstdint>` and currently compile only by transitive luck; a separate hygiene sweep, not landed here.

### Digests are platform-dependent; semantics are not
- [x] Same pinned OCCT 8.0.1, identical build id AND identical 16-hex fingerprint, yet **182 of 272 digest values differ**: `translated` inputDigest **0/32** (translation is exact in FP) · `rotated` **32/32** (trig) · `base` **20/72** — precisely the trig-built shapes (all 8 `valence4`, `overflow-02/-03`); every box and `valence3` is bit-identical · `normalizedDigest` **130/136**.
- [x] The 1e-9 quantization CANNOT fix this: rounding to a grid narrows but never closes boundary straddles, and with thousands of quantized values per record a straddle is near-certain. A digest is a **same-host** regression tripwire only.
- [x] `digests.json` keyed `suite|case|backend|variant|platform` (256 macOS rows migrated to `darwin-arm64`, 136 `linux-x64` rows recorded). `record` only replaces the current platform's rows; `compare` on an unrecorded platform exits 3.
- [x] **`semantics.json` is new and NOT platform-keyed** — the portability claim. Verified by running T0 on macOS against the baseline recorded on Linux: both hosts satisfy the same row. Timing and tolerance distributions excluded (host properties, not kernel behaviour).
- [x] Consequence for M5: "byte-identical `results.jsonl` across `--jobs`/`--shard`/`--resume`" stays valid (same host, same binary). Any CROSS-host claim must be semantic.

### Full verification sweep at `cb88ba9` (2026-08-10)
All four suites run on macOS with the three worker fixes in place, plus the manifest tool exercised against both suites and both platforms.

- [x] **ctest 110/110** · `cargo fmt --all --check` · `clippy --workspace --all-targets -D warnings` · `cargo test --workspace` with `ONECAD_REQUIRE_WORKER=1` — all green, so the vendored `json_fwd.hpp` and the `NOT APPLE` link-option guard leave macOS untouched.
- [x] **vitest 241 files / 4102 tests, all pass** · `bun run build` green.
- [x] **Manifest tool, tested rather than assumed.** `fillet/matrix:m1` 120 rows unchanged (the migration path I had NOT previously exercised) · `t0` 136 rows unchanged · macOS satisfies the Linux-recorded semantics row · guards: unrecorded platform → 3, bad mode → 2, missing semantics suite → 3 · **negative controls**: a tampered digest and a tampered `gatingFailures` both correctly report a mismatch · census 392 rows (`darwin-arm64` 256, `linux-x64` 136), every key 5 fields.
- [x] **DT_RPATH verified at the ELF level on Linux**, not inferred from a green build: tag `0x0f RPATH` (not `0x1d RUNPATH`) and **0 unresolved libraries, down from 26**.
- [x] Runner after the full ladder: 4.8 GB used of 69 GB. Persistent state ~1 GB (OCCT prefix 120 MB, worker build 142 MB, cargo 97 MB, workspace 638 MB).

### Two stale claims corrected by that sweep
- **`InspectorPanel.test.tsx:341` is CI-FLAKY, not broken.** `ci.yml` went red at `cb88ba9` on `expected "applyEditCommand" to be called at least once`; the preceding commit `b344ab6` was green with byte-identical frontend code (the only delta was `TODO.md`), the file passes 5/5 locally in isolation and 4102/4102 in the full suite, and a rerun of the SAME sha went green. It is a `vi.waitFor` timing out under CI load (that run spent 102 s in environment setup alone). **A flaky test inside a gate is a WP0.2 blocker** — wiring Playwright/vitest as required checks makes this fail the branch at random.
- **`HANDOFF.md:126`'s "4 fail (theme.spec, pre-existing)" is stale.** `theme.spec.ts` passes clean. A full-suite run under CPU contention reported 19 failures; re-running the 7 non-sketch ones in isolation (`offset-face`, `theme` ×2, `transform-body`, `history-inline-dimension`, `multiregion`, `point`) gave **38/38 pass**. They were contention artifacts of running vitest and Playwright concurrently — `workers: 1` + `retries: 0` makes this suite timing-sensitive, so e2e must be run alone.

### Playwright: 380/392, and the 12 are exactly WP0.1's scope
Re-run in isolation, deterministic and symmetric (6 chromium + 6 webkit), all four spec files from the landed sketch-angle work: `center-rect` ×1, `ellipse` ×3, `live-dim-line` ×1, `sketch-reattach` ×1, per browser. This confirms the plan's WP0.1 estimate exactly — the specs encode the OLD absolute-heading semantics and must be reconciled against the signed-turn chip before Playwright can be a CI gate.

### Still open
- [ ] **USER:** Settings → Actions → General → "Fork pull request workflows from outside collaborators" must require approval. Not readable via REST for a public repo. Consider making the repo private.
- [ ] De-flake `InspectorPanel.test.tsx:341` before vitest/Playwright become required checks (WP0.2).
- [ ] `m1` has `darwin-arm64` rows only; record `linux-x64` when the suite next runs there.
- [ ] `CLAUDE.md` still tells contributors to `brew install nlohmann-json`; that package is now inert (no `find_package`). Fold into the doc pass.
- [ ] The 34 `<cstdint>` transitive-include cases above.

## REF-H0 — Fresh Subelement Identity Contract (2026-08-09) — COMPLETE

Goal: a face, edge, or vertex promoted from the current published snapshot resolves
directly and uniquely on that unchanged worker head. Shell, OffsetFace, Hole, and
Fillet must consume the same trusted identity path; ambiguity thresholds and
operation-specific ordinal fallbacks remain unchanged.

### Investigation
- [x] Reproduce root cause from live code/history: `AcquireElementIds` returns worker
  evidence, Rust mints and caches the `ElementId`, but the authoritative worker-head
  `ElementMapPartition` never receives that binding. `PrepareOffsetFace` then treats
  the fresh id as authoritative and refuses its inevitable partition miss.
- [x] Confirm Shell's separate persistence gap: legacy `ShellParams.openFaces`
  stores bare ids, so Rust discards the frontend's typed face evidence before
  `PreviewOp` and full replay. The contract now adds typed `ShellParams.faces` in
  strict lockstep while preserving absent/empty legacy compatibility.
- [x] Audit `body_<uuid>` normalization: frontend/Rust/worker conversions are
  consistent in the clean-box path; not the root cause.
- [x] Audit mesh face/edge ordinals: tessellation and resolver use the same
  `TopExp::MapShapes` domains; not the root cause.
- [x] Find secondary MESH1 defect: partially persistent id tables mix `el_*` and
  TopoKeys, while the picker treats every label as persistent when the body-global
  flag is set.

### Red-first contracts
- [x] Box face/edge/vertex `AcquireElementIds` round trip: same head resolves
  `unchanged`, same id and TopoKey; `QueryElement(elementId)` reports present.
- [x] `AcquireElementIds` then `PrepareOffsetFace` with the returned id succeeds.
- [x] Fresh promoted top-face Shell `PreviewOp` returns no `NeedsRepair`, without a
  prior operation pre-seeding the partition.
- [x] MESH1 mixed-id pick classifies `el_*` as persistent and `f:N`/`e:N` as
  snapshot-scoped evidence.

### Implementation
- [x] Implement internal `BindElementIds`: validate exact/idempotent evidence and
  atomically install the full Rust-minted batch into the snapshot-fenced worker
  head; promotion returns and caches only after bind succeeds.
- [x] Make identity reads use one atomic published-state snapshot.
- [x] Implement `ShellParams.faces` in strict order/id lockstep with `openFaces` so
  preview, full replay, save/reopen, and fresh-worker replay retain typed evidence;
  accept absent/empty `faces` only for legacy bare-id behavior.
- [x] Fix shared preview diagnostics to report actual tool/op, never `extrude` for
  Shell/OffsetFace/Hole.
- [x] Keep safe refusal: no threshold relaxation, first-candidate fallback, or
  operation-specific identity path.

### Verification
- [x] Run focused worker ctests and real-worker Rust identity/OffsetFace/Shell tests.
- [x] Re-run Fillet fresh selection lifecycle without manual repair.
- [x] Run worker build + ctest, Cargo fmt/clippy/workspace tests, focused frontend
  tests, TypeScript, and production build.
- [x] Review final diff; preserve unrelated sketch/live-dimension worktree changes.

Gate: worker CTest 110/110; real-worker Rust workspace green; fmt + clippy
`-D warnings` green; focused frontend 127/127; full Vitest 240 files / 4051 tests;
`npx tsc --noEmit` + production build green. Fresh Shell cold reopen publishes
`needsRepair=0` at volume 2224; fresh promoted Fillet commit publishes
`needsRepair=0`. Final Shell body/load hardening: onecad-core 259 unit + 176
integration tests green; persisted foreign-body typed refs fail closed.

### Unresolved questions
- None.

## EXTRUDE DIRECT MANIPULATION — moving two-way arrow + dimension-only chip (2026-08-09, plan `simplify-this-chip-in-optimized-unicorn.md`) — FE GATE PASSED

Applying an extrude was a form, not a gesture: a twelve-control chip demanded every
decision before the user saw a result, while the arrow sat frozen at the profile
centroid and the chip sat on top of the prism. The arrow is now the operation — it
travels with the depth, points where material is going, reads two-way until the
first grab, and turns destructive on a Cut; the chip carries a dimension, a `⋯` and
✓/✕, and rides beside the arrowhead. **Extrude only** by decision; revolve, fillet,
hole, offset-face and transform keep today's clusters, and the new seams
(`DragHandle.setAxis`, overlay `axisFrom`, `ChipPlacement`) are built to generalize.

**TWO LATENT DEFECTS FOUND WHILE VALIDATING THE DESIGN, FIXED FIRST.**
- *The grab was a ratchet waiting for a moving arrow.* The drag reported the
  ABSOLUTE axis projection (`axisDepthFromRay`), which only ever worked because the
  arrow never moved. Anchored at `centroid + normal·depth`, every re-grab would add
  an arrow-length to the depth — and `commitExtrudeAtHandle` re-grabs on each
  `toPass` retry, so it would have drifted in the gate lane immediately. Now
  grab-relative, exactly like the offset-face arrow. `forceExtrudeGrab` zeroes the
  grab basis, which collapses to the old absolute mapping and keeps the depth-exact
  unit tests meaningful.
- *A press on the chip already started a depth drag.* The chip layer is a sibling of
  the canvas overlay inside the container the controller listens on, and the extrude
  branch — unlike fillet/shell/offset — never excluded chip targets. Parking the chip
  at the arrowhead would have made ✓ and the value field eat the grab.

DECISIONS WORTH CARRYING:
- **Two-way means UNDECIDED, not symmetric.** One glyph, one meaning: the second head
  retires at the first grab, and a symmetric extrude keeps a single head at the
  `+|depth|` face. The pick envelope tracks the drawn heads — a permanently symmetric
  envelope would reach backwards through the prism and the body (`depthTest:false`,
  no occlusion test in `raycast`) and swallow selection clicks.
- **`inside`, not `gap ≈ 0`, decides "into material".** A sketch coplanar with a body
  face reads gap ≈ 0 on BOTH sides; only the side whose ray exits through a BACK face
  actually starts in the solid. This is what lets the rule generalize past
  `hostFace` — a sketch on a datum plane through a block now cuts — without turning
  every coplanar second extrude into a wrong silent Cut.
- **The chip's live position never goes through the store.** `worldPos` is the mount
  effect's key, so a per-frame write would unmount the chip: focus lost and
  `commitOnBlur` firing on half-typed text. The controller calls `engine.moveChip`.
- **`offsetPx` is clearance to the chip's near EDGE.** The driver adds half the
  element's own size as a CSS percentage, so no layout read per frame. A centre-based
  offset was tried first and the chip still covered the arrow — which the e2e lane
  caught as an unclickable handle, because the chip-exclusion guard was already in.
- **The `⋯` button is a READOUT.** It shows the resolved boolean mode and raises a dot
  for any other non-default, because a mode the drag changes on its own must never
  change out of sight. `e2e/sketch-on-face.spec.ts` asserts that readout rather than
  the segments: a dismiss-on-outside-press popover cannot stay open across a drag.
- **Esc still cancels the TOOL, not the popover.** The controller owns Escape from a
  window listener registered in capture at construction, so nothing mounted later can
  preempt it; racing listener order to fake "first Esc closes" would be worse than Esc
  meaning one thing everywhere. Dismissal is an outside press or a second `⋯` click.
- `maybeNegativeDragHint` RETIRED — the arrow's colour, the prism tint and the `⋯`
  readout say it, and with the auto lane generalized its precondition rarely held.

BUG THE E2E LANE CAUGHT (and no unit test could): `LineSegments2` **extends Mesh**, so
the probe's `isMesh` filter admitted the fat edge lines, whose `raycast` reads
`raycaster.params.Line2` and throws on a plain raycaster — every extrude arm died in
the browser while jsdom stayed green. The probe now takes `userData.kind === "face"`.

GATE: `bunx tsc --noEmit` clean · `bun run build` green · vitest **240 files / 4040
tests** (from 236/3986) · hex gate 0 · Playwright chromium: the 25-spec
extrude/boolean/tree/revolve set **25/25**, with `revolve-commit` green UNMODIFIED
(proof the shared `BooleanModeSegments` was not collapsed out from under it).
NOT RUN: Rust, ctest, webkit — untouched by this wave.
STILL OWED: the manual `tauri dev` smoke, which is the only thing that can prove the
UX itself (arrow follows, chip never covers the prism, push-in cuts / pull-out joins).

FLAGGED: in the mock e2e lane the material probe finds committed face meshes, but the
generalized rule's coverage lives in `materialProbe.test.ts` +
`ModelToolController.extrudeGesture.test.ts`; the datum-plane-through-a-body case has
no e2e of its own yet. NEXT: generalize the moving arrow to revolve/offset-face, and
decide whether `tree-visibility.spec.ts` should hide its seed body.

## UNIFY-UX — Fillet/Revolve/Extrude chip parity (2026-08-09, plan `act-as-senior-ui-ux-tranquil-sparrow.md`) — FE GATE PASSED

Goal, from a user UI/UX review against Shapr3D/Fusion 360: unify the three tools'
armed-chip UX. Extrude already had a moving two-way arrow + leader-lined chip (see
above); Fillet/Chamfer had NO 3D handle at all (a whole-viewport screen-space claim)
and its chip sat centered directly ON the picked edge; Revolve showed nothing in the
viewport until both a face AND an axis were picked, guided only by a StatusBar string.
Landed in 4 phases, reusing the shared chip/store/overlay infra the extrude wave built
to generalize rather than growing three divergent implementations.

DECISIONS WORTH CARRYING:
- **Phase 0 (shared infra).** `ChipAnchorOpts` (`anchorAxisFrom`/`anchorOffsetPx`)
  promoted from `ExtrudeChipOpts`-only onto `EdgeOpChipOpts`/`RevolveChipOpts` too, with
  a shared `DEFAULT_CHIP_OFFSET_PX` (`toolChipStore.ts`) applied whenever an axis is
  given but no explicit offset — default-ON, not a per-call-site magic number.
  `HtmlOverlayDriver` now draws an actual DASHED LEADER LINE from the raw anchor to the
  offset chip position (previously the offset only repositioned the chip; no line
  existed) — a plain absolutely-positioned bordered `<div>`, rotated/scaled per frame,
  matching the driver's existing zero-React-render discipline. It piggybacks on
  whatever already mutates `worldPos`/`axisFrom` each frame (`setWorldPos`/
  `setAxisFrom`, called by `moveChip`) — no second write path, so extrude's
  continuously-moving arrow never desyncs from its line (regression-pinned:
  `HtmlOverlayDriver.test.ts` "tracks a LIVE move"). `ChipOverflow` extracted from
  `ExtrudeChipControls`'s `ExtrudeOverflow` as the shared `⋯`-button-plus-popover shell;
  extrude's own `chip-mode-readout` testid preserved via an override prop so the
  existing e2e/vitest contract didn't need touching.
- **Phase 1 (Fillet/Chamfer).** Reused `showValueHandle`/`hideValueHandle` (the same
  shared `DragHandle` instance extrude/offset-face already use) whenever
  `filletAxisSource !== "screen"` (a resolvable bisector/bbox direction); the degraded
  ("screen") tier keeps the old whole-viewport claim unchanged. Per explicit user
  decision, grabbing the handle is now REQUIRED in the non-degraded case — mirrors
  offset-face's `offsetDegraded` press-gating exactly, narrows "claims every press" to
  "claims presses on the handle," and click-away stays excluded for the degraded tier
  only, unchanged. Chip anchor moved from the raw picked-edge point to the handle's own
  base/tip pair, so it's leader-lined off the arrow instead of sitting on the edge.
  [Fillet|Chamfer] + the chamfer second leg moved behind a new `EdgeOpOverflow` (own
  file `EdgeOpChipControls.tsx`) — per user decision, NOT left inline, even though it's
  flipped more often than extrude's collapsed settings.
- **Phase 2 (Revolve).** New `revolveAxisPick` chip kind — text + ✕ only, anchored at
  the profile centroid — fills the axisPick phase's total silence (previously: zero
  chip, faint unlabeled candidate lines, a StatusBar string). A NEW screen-fixed,
  `pointer-events-none` top-anchored banner (`ViewportRoot.tsx`, `revolve-empty-hint`)
  covers the truly-nothing-selected state — the ONLY existing precedent for a
  viewport-space (not StatusBar, not world-anchored) hint in this codebase is the
  `chip==="cached"` pill, reused rather than inventing new visual language. Derived
  from existing reactive state only (`toolStore.modelTool==="revolve" &&
  toolChipStore.kind==="none"`) — no new controller-to-store plumbing needed, since
  BOTH new/existing revolve phases now publish a chip kind, leaving `"none"` true only
  during the genuine gap. Armed chip leader-lined off one axis-line endpoint
  (`revolveChipAxisFrom`). Boolean segments moved behind a new `RevolveOverflow`
  (`RevolveChipControls.tsx`); the Axis-reset button stays inline (primary action, per
  plan). Per explicit user decision: NO rotate-handle gizmo this wave — angle doesn't
  map cleanly onto `DragHandle`'s linear forward/twoWay model; the leader-lined chip +
  already-existing live lathe preview covers the gap for v1.
- **Phase 3 (Extrude).** Pure dedupe/regression pass: local `CHIP_AXIS_OFFSET_PX`
  removed in favor of the Phase-0 shared constant directly; the new leader-line code
  path verified against the ONE thing that could desync it (extrude's per-frame
  `moveChip`-driven arrow) via a dedicated `HtmlOverlayDriver` test rather than
  eyeballing it — see above.

WEBKIT-ONLY E2E FLAKE, FOUND AND FIXED WHILE VALIDATING FILLET'S NEW HANDLE: the FIRST
drag right after arming (no prior chip interaction to let a natural render happen) could
land a `mouse.down()` on a handle a JS-side hit-test had JUST confirmed hot, and
silently do nothing — reproduced directly: `hitExtrudeHandle` at the identical point
flips true→false→true across consecutive calls in WebKit headless specifically
(Chromium never showed it). A 300ms settle wait in the new `dragEdgeOpHandle` e2e helper
(`modelToolHelpers.ts`) fixes it — every OTHER handle-drag call site in this codebase
already has a prior interaction that incidentally buys the same margin, which is why
this never surfaced before. Verified with 20/20 repeat-each runs across both browsers
post-fix (0/10–9/10 failing before it, depending on the mitigation attempt).

GATE: `bunx tsc --noEmit` clean · `bun run build` green · hex-gate 0 on touched files ·
vitest **240 files / 4060 tests** (from 240/4051 pre-wave — +9: 4 leader-line + 1
live-move regression in `HtmlOverlayDriver.test.ts`, 4 in `ModelToolChips.test.tsx` for
the two new overflows/axisPick chip) · Playwright chromium+webkit, FULL suite (69 spec
files): **380/392 passed**. The 12 failures (`center-rect`, `ellipse` ×3,
`live-dim-line`, `sketch-reattach` — ×2 browsers) are ALL in sketch-drawing /
live-dimension specs, whose source (`LiveDimChips.tsx`, `liveDimStore.ts`,
`SketchController.ts`, `liveDimFrames.ts`, `liveDimension.ts`, `liveToolMachines.ts`,
`CornerCluster.tsx`) was mid-edit in this same working tree from a CONCURRENT session
before and during this wave (uncommitted, not touched by this diff) — same pattern the
EXTRUDE DIRECT MANIPULATION gate above flagged and preserved. Zero overlap with any
file this wave touched; every spec that exercises a file this wave DID touch
(`filletChamfer.spec.ts` 26/26, `revolve-{commit,preview,region}.spec.ts` 8/8 incl. 2
new guidance-banner cases, `extrude-{commit-gesture,boolean,draft,end-conditions,
multiselect}.spec.ts`, `offset-face.spec.ts`, `shell-preview.spec.ts`,
`boolean-preview.spec.ts`, `sketch-{on-face,fillet,hole-extrude,offset}.spec.ts`,
`history-inline-dimension.spec.ts`, `tree-visibility.spec.ts`) is green.
NOT RUN from here: Rust, ctest (untouched by this wave — no worker/backend changes).
NOT INVESTIGATED further: the 12 pre-existing sketch/live-dim failures above — outside
this wave's scope and file set, belongs to the concurrent session's own gate.

### Unresolved questions
- Dashed leader-line visual spec (color/dash pattern/min-max length) has no design
  token yet — engineering default used (`--color-border-strong`, 1px dashed, hidden
  under 4px), needs a design pass.
- Revolve's pre-axisPick copy ("Pick an axis line", "Select a sketch region to
  revolve") is a placeholder pending copy review.
- Whether `hitExtrudeHandle`/`showValueHandle`/etc. get renamed now that fillet is a
  third caller (e.g. `hitValueHandle`) — naming-only, not blocking.
- Revolve rotate-handle gizmo and Fillet's whole-viewport-vs-handle-required tradeoff
  were explicit user calls for v1; both are flagged in the plan as revisitable.


## PLATFORM REFACTOR — Milestones 1 + 2 (2026-08-08, plan `velvety-leaping-adleman.md`) — IN FLIGHT

Architecture-only refactor: modeling stops being synonymous with OneCAD and becomes the first built-in module on a Platform, and `.onecad` gains namespaced module state that survives a round trip without its owner installed. **No user-visible change, no modeling behavior change.** Out of scope: SDK package, test addon, addon manifest/loader/host, GitHub install, resource-store generalization, dynamic Tauri router, crate extraction, any file moves.

Laws: `docs/ARCHITECTURE.md` (normative) + `docs/adr/0001`–`0008` + the new CLAUDE.md § Architecture laws.

### Recorded baseline (W0, measured — not inherited from a doc)
- `bunx tsc --noEmit` was **RED** on `src/ipc/mockClient.import.test.ts:106` (`'snap' is possibly 'null'`, from concurrent commit `685efc2`). Fixed by narrowing, no cast. Now green.
- `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` against the **staged** sidecar (`worker/build/onecad-worker`, built 19:37, pre-VF-M5-gate): **1033 passed / 0 failed**. The VF-M5 red recorded in this file does NOT reproduce on that binary — consistent with the note that it needs a worker built from HEAD.
- Worker could NOT be rebuilt here: `scripts/build-worker.sh` aborts with "OCCT artifact metadata is absent" for `/opt/homebrew/opt/occt-8.0.1` (no `occt-build.json`; that prefix is not a `build-pinned-occt.sh` product). **Unrelated to this refactor — flagged, not worked around.** The VF-M5 FOLLOW-UP gate below therefore stays open and unverified from here.
- Frontend baseline at W0 close: vitest **213 files / 3720 tests** green; `cargo fmt --all --check` + workspace `clippy -D warnings` clean; `cargo test -p onecad-core --lib` 242/0.

### W0 — invariants captured ✅
- [x] Frozen behavior contracts in `src/test/contracts/` (+ a README stating they may not be edited to make a refactor pass): toolbar arrangement, the three keymap tables + cross-mode opt-out, editor mount order, inspector section order per state.
- [x] Four golden probes: `toolbarConfig.golden.test.ts`, `keymap.golden.test.ts` (full (key, shift, mode) resolution matrix against an independently-written oracle, not a call into the code under test), `editorMountOrder.golden.test.ts` (JSX scan; becomes a registry scan at W3 against the SAME contract), `InspectorPanel.golden.test.tsx`.
- [x] CORRECTED ASSUMPTION while writing them: the inspector's Constraints section is UNCONDITIONAL in sketch mode (label + "No constraints yet.") — the contract records shipped behavior, not the guess.
- [x] Rust `unknown_document_state_survives_open_modify_save` in `io/container.rs` — the W5 guarantee rehearsed on today's `Document.extra` lane: open → real modeling edit → save → reopen, foreign key byte-equal.
- [x] `docs/ARCHITECTURE.md`, `docs/adr/README.md` + ADR-0001…0008, CLAUDE.md § Architecture laws.

### W1 — platform core ✅ (pure addition, nothing wired)
- [x] `src/platform/ids.ts` — branded ids + reverse-DNS owner validation + namespace enforcement (an addon cannot construct an `onecad.*` contribution id).
- [x] `src/platform/registry.ts` — owner-scoped generic registry. Duplicate id ⇒ throw naming the holder; foreign namespace ⇒ throw; order is `(priority, insertion index)` with `group` as consumer metadata, NOT a sort key (group names sort alphabetically, which is never the intended visual order); snapshot reference is cached because `useSyncExternalStore` loops on a fresh array per call.
- [x] `src/platform/contributions.ts` — Command / Tool / Panel / Inspector / Viewport / Workspace contracts, domain-neutral (scopes are opaque strings so the platform never learns "model"/"sketch"). `ViewportContext.invalidate()` is the on-demand-render seam.
- [x] `src/platform/{slots,events,services,platform}.ts` — closed slot list, owned event subscriptions, service registry with a naming `require()`, module lifecycle with dependency topo-sort, cycle + missing-dependency rejection, and a failed activation that leaves nothing registered.
- [x] `src/platform/react/` — `PlatformProvider` (context, never a global singleton), `SlotHost` (renders no wrapper DOM — these are absolutely-positioned overlays and a wrapper would change stacking), `ContributionBoundary` (per-contribution isolation; the app-level boundary's full-screen fallback is the wrong shape for a 260px panel).
- [x] Tests 46/46: ownership, stale-handle safety, duplicate + namespace rejection, order independent of registration order, tie-break, notification, dependency order, cycle, failed-activation cleanup, scope teardown completeness, slot render order, error isolation.

### W2 — modeling owns its tools, commands and bindings ✅
- [x] `src/modules/modeling/tools.ts` + `bindings.ts` are now the SINGLE source of truth for the palette and the three key tables. `features/toolbar/toolbarConfig.ts` and `shortcuts/keymap.ts` DERIVE from them and keep their exported shapes, so every call site and test is untouched. Resolution rules (mode precedence, exact chord, cross-mode fallback + opt-out) stay in `keymap.ts` — the module owns WHICH keys exist, the shortcut layer owns HOW one resolves.
- [x] Descriptors are discriminated on `scope`, so a sketch-only tool placed in the model table is a compile error, and consumers narrow without a cast.
- [x] Separators are DERIVED from group boundaries rather than authored — `group` is consumer metadata, `priority` is the sort key.
- [x] Ids are SCOPE-QUALIFIED (`…tool.model.mirror` vs `…tool.sketch.mirror`): `select` and `mirror` exist in both unions and mean different things, so a flat map would have let one shadow the other.
- [x] `registryToolbar.ts` rebuilds the arrangement FROM the registry — the golden assertion runs against that, not against the table the registry was built from, or it would only prove the table equals itself.
- [x] Tool activation and command execution DELEGATE to the existing `activateTool` / `runAction`, so a registry-driven invocation and a toolbar click cannot diverge.
- [x] `ToolDefinition.shortcutLabel` added: Measure binds ⇧/ but is written "?", so glyph and chord are not derivable from each other.

### W3 — EditorShell + slot hosting ✅
- [x] `src/app/shell/EditorShell.tsx` renders permanent structure + one `SlotHost` per region; the 19 concrete imports are gone. `EditorScreen.tsx` is now a one-line bridge so `App.tsx`'s code-split specifier and `StartScreen`'s idle prefetch keep working.
- [x] Contributions register on EDITOR MOUNT, not at bootstrap: the editor tree is a deliberate code-split chunk, and hoisting those imports into the startup bundle to satisfy an architectural preference would make the start screen pay for the editor. That needed `platform.createScope(owner)` (independent child scope) and a fix to scope teardown — `dispose()` no longer sweeps the whole owner, which would have let the editor's scope tear down the module's bootstrap registrations.
- [x] Panel ids live in `panelIds.ts`, split from the files that import components, so a workspace definition can name a panel without dragging the editor chunk in.
- [x] TWO NEW SLOTS, deliberately: `viewport.chrome` (controls anchored to the viewport frame — nav pill, corner cluster; they sit above the docked panels, unlike scene-tracking overlays) and `shell.notification` (banners). Without them the frozen mount order could not be reproduced with contiguous slot regions.
- [x] The mount-order probe was REPLACED (JSX scan → registry scan) against the SAME frozen contract, plus two new checks: every registered panel lands in a region the shell actually renders, and re-registration after teardown is collision-free (StrictMode double-invokes).

### W4 — default workspace + composition root ✅
- [x] `src/app/bootstrap.ts` — `bootstrapOneCAD()` creates the Platform, registers `onecad.shell` + `onecad.modeling`, initializes in dependency order. It lives in `app/` and NOT in `platform/`: the composition root is the only place allowed to know both sides.
- [x] `App.tsx` builds it in a state initializer and wraps the tree in `PlatformProvider` — no global singleton, and every existing test that renders `<App/>` keeps working with no setup.
- [x] `platform.initializeSync()` added because the React root must have a Platform on its FIRST render; it throws if a module's `activate` is async, so the restriction is visible at startup rather than as a half-built registry.
- [x] `onecad.shell.workspace.design` reproduces the current layout declaratively. NAMING DEVIATION recorded: the spec sketches `onecad.workspace.design`, but a contribution id must sit under its owner's namespace, and this workspace is owned by `onecad.shell` because it composes several modules. No workspace switcher in the UI.

### W5 — Rust module-owned document state ✅
- [x] `onecad_core::document::modules` — `ModuleId` (reverse-DNS, validated on AUTHORING only), `ModuleState { schemaVersion, payload }`, `ModuleStateTable`. Deserialization is deliberately permissive: refusing an id a stricter build dislikes would destroy exactly the data preservation exists to protect.
- [x] `Document.modules` + the `DocumentData` mirror, `skip_serializing_if` empty — pinned by test that a document without module state writes NO `"modules"` key, in the document and in the manifest. No container-version bump, no user-document migration.
- [x] `Manifest.modules` descriptor table, DERIVED from the document at save so the two can never disagree — this is what makes "this project uses an addon you do not have" answerable without decoding a payload.
- [x] `EditCommand::SetModuleState` + `Inverse::RestoreModuleState`, dirty floor `None` (no timeline step can consume state the platform cannot interpret). Programmatic writes therefore use the SAME transaction path as user edits.
- [x] Proofs: unknown-module state byte-equal across open → real modeling edit → save → reopen; a module id this build would refuse still round-trips; undo restores the PRIOR slice rather than merely deleting the new one; clear-then-undo restores.

### W6 — wire + missing-module reporting ✅
- [x] `ModuleStateDto` / `DocumentModuleDto`; three typed Tauri commands (`get_module_state`, `set_module_state`, `list_document_modules`). NO dynamic router — spec §97's `platform_invoke` is deferred to the addon-host effort, where it will have a consumer.
- [x] `CadClient` gains three append-only methods; `tauriClient` and `mockClient` both implement them, so the whole persistence lane is exercisable with no backend. `set_module_state` refuses a `schemaVersion` without a payload rather than silently treating it as a clear.
- [x] `src/platform/documentState.ts` — the service binds the module id at construction, so a module cannot address another module's slice by accident. `missingModules()` reports, never blocks.

### W7 — enforcement + gate ✅
- [x] `src/platform/architecture.test.ts` scans the real import graph: Platform must not import `@/features`, `@/tools`, `@/modules`, `@/stores`, `@/viewport`, `@/app`; modules must not import the shell or deep-path into the platform. Carries a POSITIVE CONTROL (an edge that really exists) because every other assertion expects an empty list — which is also what a broken scanner returns.

### Flagged seams (carried forward, not fixed here)
- `zoomFit` / `home` are registered as MODELING commands because that is where their bindings live today; they are really view-navigation and belong to the platform once a selection/viewport service exists. — **P2 W11.**
- ~~The toolbar component still renders from the derived `MODEL_TOOLS`/`SKETCH_TOOLS` arrays~~ — **CLOSED by P2 W8.**
- Inspector sections, tree nodes and viewport layers are NOT yet contributions — `InspectorContribution` / `ViewportContribution` / `TreeProvider` exist as contracts with no producers. — **P2 W9/W10/W12.**
- **DECISION OWED before P3 — is the toolbar extensible?** W8 made it read the registry live, but it stays a MODELING PROJECTION: `registryToolbar.ts:52` skips any tool id absent from modeling's reverse map, and `FloatingToolbar` activates through `activateTool` (the store `Tool` union) rather than `ToolDefinition.activate`. So an addon could register a valid tool and be silently absent from the toolbar. Opening it needs applicability to become a contribution concern (`toolApplicability.ts` is typed on the modeling `Tool` union) and `ToolEntry` to give way to `ToolDefinition` — which changes the currency the frozen toolbar contract is written in. P3 freezes the SDK surface over whichever answer we pick, so pick it first.
- Module state is stored in `document.json` (ADR-0004); moving to `modules/<id>/state.json` later is a container-format change.
- **CORRECTED (was mis-reported as a blocker):** `scripts/build-worker.sh` failed with "OCCT artifact metadata is absent" only because it was pointed at `/opt/homebrew/opt/occt-8.0.1`, a plain Homebrew install. The PINNED prefix `~/.onecad-occt/8.0.1` carries `share/onecad/occt-build.json` and configures fine. Nothing is blocked. CONSEQUENCE: this session's worker-backed 1045/0 ran against `worker/build/onecad-worker` (19:37), which HANDOFF.md § VF-M5 identifies as a **stale pre-gate build** — so it did not exercise `069bb48`'s worker changes. Re-run against a HEAD build before trusting that number for the worker lane (see NEXT SESSION P1).

## P2.5 — GENERIC EXTENSION SEMANTICS BEFORE THE SDK (2026-08-09, plan `act-as-senior-software-encapsulated-balloon.md`) — IN FLIGHT

P3 does not start over a surface where three registries are generic at the contract and modeling-specific at the runtime: an addon tool is silently dropped by the toolbar, `defaultShortcut` never reaches the keyboard, and `TreeNode.id` collides across providers. Sequence: WP0 baseline → WP1 tool runtime → WP2 shortcuts → WP3 tree + settings → WP4 module lifecycle → WP5 `@onecad/sdk` → WP6 reference addon → WP7 enforcement. Workspace runtime is deliberately DEFERRED past the reference addon (it is not in the SDK's tool/tree/shortcut currency).

### WP0 — green baseline ✅ (2026-08-09)
- [x] VF-M5 flagship regression closed — see § VF-M5 FOLLOW-UP below. **A RESIDUAL remains open** (§ VF-M5 RESIDUAL): the gate is off, and the F12 fallback lane it should protect is real. Re-arming it is a protocol change.
- [x] The four Playwright failures were stale specs, not an open UX question. Both removals were already decided AND pinned elsewhere (`TitleBar.test.tsx:67-69` asserts the Appearance toggle is absent; the unified start-screen `Import…` is recorded in § Wave 4), so the specs moved to the shipped entry points rather than the UI moving back.
      - `theme.spec` — the two title-bar-toggle tests became a Settings-modal pair: the modal drives the theme AND the engine (the half CSS cannot fake), and modal ↔ popover stay in step. Same intent, the control that exists.
      - `project-import` / `step-import` — one `Import…` button now routes on extension, so a `.onecad` pick is an OPEN (asserted through `appStore`, since the mock's `openDocument` pushes no projection) and the append lane stays covered by the in-editor File ▸ Import Project… test. `mockClient.importFileDialog()` gained `?mockimport=step` (same dev-only URL-flag pattern as `?vpdemo`) so the router's STEP half is still reachable from a browser lane.
- [x] Gate: `bunx tsc --noEmit` clean · vitest **224 files / 3837** · worker ctest **107/107** · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1045 passed / 0 failed** across 72 targets (worker built from HEAD against `~/.onecad-occt/8.0.1`, staged) · `cargo fmt --all --check` · workspace clippy `-D warnings` · full Playwright **387 passed / 3 failed**.
- [x] **The 3 remaining Playwright failures are PRE-EXISTING and are not this work.** All three are `boolean-preview.spec.ts` (chromium :276, webkit :227 + :276). Verified by bisect in a throwaway worktree: the same test fails on `4145f3f` (before P2) and on `cf75bda` (before the Platform refactor entirely). Symptom, from the failing run's `fe-logs`: after the sketch is re-shown between the two extrudes, the click on the circle region does not select it — Extrude then arms in multi-select ("Select regions to extrude") instead of the depth drag. `sketchHitTestReady` already passed, so the sketch WAS hit-testable; the suspect is the click racing the visibility commit's projection push. Machine-dependent: the 2026-08-08 full run had it green.
- [ ] **Manual `tauri dev` smoke still owed** (open project → extrude → fillet → undo → save → reopen). Unchanged from P0 below: the one Definition-of-Done item with no evidence.

### WP1 — generic tool runtime ✅ (`4b8ec0d`, ADR-0010)
- [x] `ToolHost` on the platform owns the active id + the activate/deactivate handshake; `FloatingToolbar` renders `ToolDefinition`s directly (title/icon/shortcutLabel, `group` boundary ⇒ separator, `priority` ⇒ order) and activates through the host. `toolFromId` no longer stands between a registration and the screen.
- [x] Modeling stays authoritative and REPORTS: `toolStore` changes are mirrored into the host, so AUTO-MODE, the Esc ladder and self-arming controllers cannot leave the highlight disagreeing with the store. `deactivate()` runs only on a CROSS-owner swap.
- [x] `canActivate` returns `ToolAvailability` (the toolbar has always explained WHY a tool is grayed) and gained `subscribe(onChange)` — the same staleness rule W14 gave `TreeProvider`. Modeling backs every tool with ONE shared emitter over two stores.
- [x] A tool with NO scopes now appears everywhere (`CommandDefinition.scopes`'s documented "empty ⇒ always"); the old projection dropped that case, which made a zero-knowledge contribution impossible. Icons resolve defensively — `def.icon` is an open string and the old cast crashed inside `Icon`.
- [x] **Real regression caught by e2e**: `mirror` is in BOTH tool unions, so routing every tool through the model applicability matrix disabled the SKETCH Mirror tool with nothing selected. Applicability is model-scope only, with a vitest case pinning it.
- [x] `toolbarFromRegistry` demoted to the golden probe's projection (the contract is written in `ToolEntry` and may not be edited); new case pins that a FOREIGN registration leaves modeling's arrangement byte-identical.

### WP2 — registered shortcuts reach the keyboard ✅ (`eb59b6d`, ADR-0011)
- [x] `defaultShortcut` was write-only — three producers, no reader. `platform.shortcuts` resolves chords over both registries; `useShortcuts` asks modifier chords → `resolveBinding` → the registry, in that order, so a contribution CANNOT shadow a built-in and the golden keymap oracle stays byte-identical.
- [x] Conflicts never resolve by load order: scope-specific → explicit priority → built-in over addon → otherwise the chord fires NOTHING and is reported. A keystroke does not bypass `canExecute`. The ⌘-chords stay hardcoded and are explicitly not addon-reachable in v1.

### WP3 — tree public surface + settings ✅ (`7dee177`, ADR-0012)
- [x] Rows are addressed by `(providerId, nodeId)`. Ids stay provider-local: demanding globally unique ids pushes a naming burden onto every contributor to fix a host bug.
- [x] Modeling's provider implements `subscribe`; the host dropped its five modeling store subscriptions and now watches only providers.
- [x] `TreeNodeAction` is command-backed with a declarative `confirm`, so a row action is reachable from a palette rather than living inside one popover. Delete-datum/delete-sketch became modeling commands; the panel lost its `kind` branches for them. **Reattach stays the one flagged modeling-specific branch** — it needs a fact the node does not carry AND a second anchored popover.
- [x] Settings moved from `ModelTreePanel` to the shell's `StatusBar`. `settingsStore` ownership is untouched and out of scope.

### WP4 — module lifecycle ✅ (`bbe1ceb`)
- [x] `ModuleDefinition.deactivate` was declared and called from NOWHERE. It now runs on `disposeOwner` and `platform.dispose()`, before the registrations go, in reverse initialization order. Not for a failed activation, not for a short-lived child scope, and a throw does not abort disposal. New `deactivating` state; `platform.dispose()` also sweeps the registries per owner (it only walked tracked scopes).

### WP5/WP6/WP7 — SDK boundary + reference addon + enforcement (ADR-0013)
- [x] `@onecad/sdk` → `src/sdk/` (tsconfig + vite alias; not published — a real package buys nothing until something outside this repo builds against it). Re-exports ids + checked constructors, `Disposable`, the contribution contracts, `Slots`, `SelectionRef`, document-state types and `ExtensionContext`.
- [x] `ExtensionContext` replaces `ModuleScope` for addons: same registrations, no `platform`, no `createScope`, and `document.state` pre-bound to the addon's namespace. Narrowed BY CONSTRUCTION, not by cast — a cast leaves `platform` present at runtime. `createExtensionContext`/`registerExtension` are host-side and absent from the SDK.
- [x] `src/addons/reference/` contributes command + tool + panel + inspector section + tree section with a command-backed action + viewport layer + workspace + its own document namespace, over its OWN domain type (`com.onecadtest.reference.item`), importing `@onecad/sdk` and `react` only. Tests drive the REAL toolbar, tree, shortcut lane and viewport host.
- [x] **Deliberately NOT registered in `bootstrap.ts`** — shipping a fake "Widgets" panel into the product UI to prove an architectural point is a worse trade than proving it in tests. It becomes the addon loader's first package when the loader lands.
- [x] `architecture.test.ts` enforces both directions (addon ⇏ application, SDK ⇏ application implementation), plus a runtime-surface snapshot of the SDK barrel and an explicit "the SDK does not export the host" check. Both new rules carry the positive-control pattern.

## MODULAR-PLATFORM UI (2026-08-09) — design turn 2 implemented, FE gate PASSED

Source: `claude.ai/design/p/f68f85fa` → `OneCAD UI Explorations.dc.html`, turn 2
("Modular platform — workspaces, extensions, palette, missing add-ons"), options
2a–2d. Read via the `claude_design` MCP; nothing from the design project is
vendored into the repo.

**This is the first DELIBERATE user-visible change since the platform refactor.**
`src/test/contracts/shellContract.ts` was amended for it (six new shell
contributions, nothing existing moved) — recorded here because the contracts
README requires an explicit decision, never a refactor-driven edit.

### What is REAL (bound to something that exists)
- [x] **Workspace selector** in the title bar, projected off `platform.workspaces`. Four shell workspaces registered (Design + Simulation/Drawing/Visualization); an add-on's workspace lands in its own ADD-ONS group automatically, and an empty group renders nothing.
- [x] **Workspace panel filtering**: `SlotHost` takes a `filter` predicate; `modules/shell/workspaceLayout.ts` resolves it. Rule is CONSERVATIVE — a panel is hidden only on an explicit `visible:false` placement or a user override. "Unlisted ⇒ hidden" would silently swallow every tool overlay.
- [x] **⌘K command palette**, projected off `commands` + `tools` + `workspaces` registries. No palette registry: anything registered is findable, nothing opts in. Disabled entries stay visible with their owner's `reason`.
- [x] **Missing-extension banner + details dialog + "Unavailable data" explorer section**, all from the REAL `listDocumentModules()` ⋂ `platform.moduleIds()` diff. This is ADR-0005 made visible.
- [x] **Extensions manager** — Installed tab lists real modules with their lifecycle state and a count of what each contributed.
- [x] **Viewport layers menu** on the NavPill's previously-dead "View presets" button. New `ViewportEngine.setLayerVisible()` toggles `bodiesRoot`/`sketchRoot`/`contributionsRoot`; grid reuses the existing `viewportStore` flag (one piece of state, two entry points).
- [x] **Collapsible explorer sections** + `TreeNode.meta` / `TreeNode.problem` / `TreeSection.defaultCollapsed` / `TreeSection.emptyNote` (additive platform contract).
- [x] **Customize workspace sheet** — panels/tool groups/layers all DERIVED from the registries, never enumerated. Tool-group hiding filters `FloatingToolbar` only; the tool stays registered and reachable by shortcut and palette.
- [x] **Start-screen Extensions entry** (2d), with Settings below the rule — configuration, not project content.
- [x] **Title-bar buttons unified** behind one `TitleBarButton` (28px · 6px radius · ghost). Home/File/workspace/⌘K were three different hand-rolled geometries in a row.
- [x] **File ▸ Rename…** (moved out of the title bar at user request — a title inside the drag region is a rename you trigger by accident).

### What is UI-ONLY, and why (no backend exists)
- **Extensions Browse / Updates** render an explicit "no registry configured" empty state. Deliberately NOT a mock catalog: dead Install buttons teach users the feature is broken, and fake listings outlive the mock. There is no addon loader (`platform/extension.ts` closing note).
- **Enable/disable + uninstall** are absent rather than inert — a half-unloaded built-in module is not a state the app is designed to run in.
- **Background-tasks chip** (`tasksStore`, status bar) has a real `begin/setProgress/end` API and renders an INDETERMINATE bar when `progress` is undefined. Nothing calls it yet: regen is request/response over OCW1 with no progress frames. **FLAGGED** — first producer should be regen once the protocol can report progress.
- **Simulation / Drawing / Visualization** are real registered arrangements with no module behind them: each switches modeling's tool surfaces OFF and shows a `WorkspacePlaceholder` naming what is missing. Leaving Extrude on screen under "Drawing" would have been worse.
- **Rename is display-only** (`DocumentState.displayTitle`). There is no `RenameDocument` in the protocol — the Rust runtime derives the title from the file it loaded, and `renameRecentProject` needs a path the editor does not hold. The dialog SAYS the file is untouched. **FLAGGED — next backend tranche.**

### Flagged seams
- `CommandPalette` imports `useModelingToolContext` from `@/modules/modeling`. Shell UI reaching a specific module: the palette needs a real selection context to evaluate `canExecute`, and no neutral selection SERVICE exists yet. Precedent: `ViewportRoot` → `datumViewport`. Fix is a module-published context service, not a cast.
- `paletteStore`/`workspaceStore`/`layersStore` overrides are session-only — nothing persists yet.
- `Popover` gained `top-start`, which measures the panel after mount (one frame at the seeded position, same as every other placement).

### Gate (2026-08-09)
- `bunx tsc --noEmit` clean · `bun run build` green · vitest **236 files / 3986 tests** green (was 227/3919; +9 files) · hex-token grep empty · Playwright **387 passed / 3 failed**.
- The 3 e2e failures are pre-existing, verified not assumed: `sketch-reattach` (chromium) passes 3/3 isolated (load flake); both webkit `boolean-preview` failures reproduce in a clean worktree at HEAD `352ddd1` — the same failure already bisected to before the Platform refactor.
- Rust/ctest untouched by this wave.
- REMAINING: manual Mac smoke (`bun run tauri dev`) — the workspace filter and the new `shell.overlay` region are layout changes no jsdom test can prove.

## NEXT SESSION — three work packages (2026-08-08, handoff)

Read `HANDOFF.md` § Session 4 first. P1 is another program's open gate; P2 finishes what the platform refactor left half-done; P3 is the next real tranche. **Do P2 before P3** — an SDK frozen over a half-converted surface freezes the wrong shape.

### P0 — verify before touching anything (30 min)
- [ ] Manual smoke, the only Definition-of-Done item with no evidence: `bun run tauri dev` → open an existing project, extrude, fillet, undo, save, reopen. Chrome and layout must look identical to before `4145f3f`. Any visual difference is a slot-order or z-index regression — `src/test/contracts/shellContract.ts` is the contract it violated.
- [ ] Rebuild the sidecar against the PINNED prefix and re-run the worker lane, so the numbers describe HEAD:
      ```bash
      ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" \
      ONECAD_WORKER_BUILD_DIR="$PWD/worker/build-pinned" scripts/build-worker.sh Release
      ctest --test-dir worker/build-pinned --output-on-failure          # expect 107/107
      cd src-tauri && ONECAD_WORKER_PATH=$PWD/../worker/build-pinned/onecad-worker \
        ONECAD_REQUIRE_WORKER=1 cargo test --workspace
      ```
      EXPECTED: `topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean` FAILS. That is P1, not a platform-refactor regression — it is the VF-M5 gate that `069bb48` opened.

### P1 — close the VF-M5 gate regression — CLOSED (2026-08-09, resolution (a))
Full diagnosis already existed in `HANDOFF.md` § "VF-M5 gate regression" and § VF-M5 FOLLOW-UP above.
- [x] The discriminator was wrong. Worker `PlanExecutor` used `job.partition.size() == 0` to mean "from-zero replay", but the RegenPlanner emits full-replay-from-0 plans for EVERY regen and `Session::fence_and_clone` clones an empty base for those (D5), so it is ALWAYS true — the gate degenerated to `edited_from.is_some()` and the flagship edit lane (`ToEnd { from: 1 }`) was falsely treated as a replay, disabling the anchor-exact carve-out and flagging `NeedsRepair`.
- [x] **Resolution (a) taken.** `job.from_zero_replay = false` in `PlanExecutor.cpp`, with the derivation and the re-enable condition (a genuine restored-basis signal, never partition emptiness) recorded at the assignment. The field is re-documented in `ScratchJob.h` and `Ladder.h` as "replay onto a RESTORED basis", which is what it always meant; V1 has no restore (SaveCheckpoint/RestoreCheckpoint UNSUPPORTED, `baseCheckpoint` never read), so the hazard cannot occur. The ladder behaviour stays wired and stays covered by `worker/tests/test_wp6_ladder.cpp`, which builds `LadderEditContext` directly.
- [x] Red-first, verified on a worker built from HEAD against the pinned prefix: `topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean` reproduced RED (`needsRepair` 1, expected 0) BEFORE the change and passes after. The gate was not weakened.
- [x] The 4 Playwright failures were STALE SPECS, not the platform refactor and not an open UX question — both removals are already pinned elsewhere: `TitleBar.test.tsx:67-69` asserts the Appearance toggle is absent (it lives in `DisplayModePopover` + `SettingsModal`), and `TODO.md` § Wave 4 records the unified start-screen `Import…` button with the sidebar duplicates removed. Specs rewritten to the shipped entry points; `mockClient.importFileDialog()` gained `?mockimport=step` so the extension router's STEP half stays reachable from the browser lane.

### P2 — finish Milestone 1 (plan `~/.claude/plans/resume-parallel-muffin.md`, Codex-reviewed terra/high → revise, 3 blockers + 4 high all folded)
- [x] **W8 — toolbar reads the registry live.** `FloatingToolbar` now takes `usePlatform()` + `useRegistryEntries(platform.tools)` and memoizes `toolbarFromRegistry` off that snapshot — the projection allocates a fresh array per call and would loop `useSyncExternalStore` as the snapshot itself. `toolbarConfig.ts` was NOT deleted as first planned: `src/test/contracts/toolbarContract.ts:6` imports `ToolEntry` from it, and rewriting that import edits a frozen contract file. It shrank to the entry shape (`ToolItem`/`ToolSeparator`/`ToolEntry`/`isSeparator`) and is now the contract's type anchor; the runtime derivation (`MODEL_TOOLS`/`SKETCH_TOOLS`/`toolsForMode`/`toolEntriesFor`) is gone. Golden probe moved to `modules/modeling/registryToolbar.golden.test.ts` (probe follows the mechanism; contract does not move) and gained a live-registry case: disposing modeling's scope empties the toolbar. New `src/test/renderWithPlatform.tsx` boots a REAL platform (not a stub registry, which would let a registration bug pass) for `FloatingToolbar.test.tsx` + `SketchEntry.test.tsx`, which rendered bare.
- [x] **W9 — inspector sections become contributions.** The eight labelled sections (Appearance body/face, Dimensions, History selection/feature, Constraints hint/list, Depends on) moved out of `InspectorPanel.tsx` into `src/features/inspector/sections.tsx` and register through `src/modules/modeling/inspectorSections.ts`. The panel keeps the frame, the branch tree, headings, the DOF card and the trailing hints; EMPTY and REPAIR host no sections at all, which is why the contract is untouched by leaving them chrome. **ONE platform change:** `InspectorContext` gained `scopes: readonly string[]` — opaque tokens, the same currency as `ToolDefinition.scopes` — because three sections gate on mode and an `InspectorContext.mode` would teach the platform a modeling concept. `SlotHost` could not be reused (it is bound to `platform.panels`), so `InspectorSectionHost` is modeling-owned and subscribes to `selectionStore` + `toolStore` as well as the registry: `canRender` is a plain predicate and a registry-only host would keep rendering the last selection's sections. Sections render their OWN `SectionLabel` — the label is what the order contract counts, so a section with nothing to show must render nothing at all. `subElement` carries the ElementId only, never the topoKey, or an unpromoted face would look promoted to every `canRender`. **Probe change, contract untouched:** `InspectorPanel.golden.test.tsx`, `InspectorPanel.test.tsx` and `RepairPanel.test.tsx` gained a `renderWithPlatform` harness (the panel needs a Platform now); every assertion, `sectionsOf` and the `tracking-[0.07em]` marker are byte-identical, and `git diff --exit-code src/test/contracts/` is clean. New `InspectorSectionHost.test.tsx` (7) covers the staleness cases a registry-only host would pass: selection change, mode change, register-after-mount, dispose, priority-over-registration order, throwing-section isolation, and the EntityRef→SelectionRef mapping including the topoKey guard.
- [x] **W10 — TreeProvider beneath the existing rendering.** `TreeProvider` did not exist anywhere (not in `contributions.ts`, not in the ADRs — only the two unused `Slots.Tree*` constants), so the shape was designed here: `TreeNode { id, label, icon, kind, selected, dimmed?, visible?, select(), activate?, toggleVisible?, rename? }` + `TreeSection` + `TreeProvider.sections()`, with a `platform.tree` registry mirroring `platform.inspector`. **`select()` is separate from `activate()`** because click and double-click mean different things on every shipped row (a sketch selects on click, re-opens on double-click); collapsing them would either change behavior or push modeling's selection semantics back into the generic panel. **An ABSENT capability is how a row says it lacks one** — a datum supplies no `toggleVisible` and no `rename`, so the eye and the Rename item disappear on their own; the menu now branches on the node's capabilities rather than on `menu.kind`. `sections()` READS the stores (`getState()`) and the panel keeps the subscriptions purely to re-render — that is what lets the contract stay a plain function instead of a hook, and it is commented as load-bearing in both files. The visible tree is unchanged. **Flagged seam:** the context menu still lives in the panel and its datum/sketch branches still switch on `node.kind`; Reattach needs its own anchored popover and two-click delete is panel state, so a provider-supplied `TreeNodeAction[]` is the next step. P3's addon therefore gets rows but not menu items.
- [x] **W13 — ADR-0007 corrected.** It documented iteration order as `(group, priority, insertion index)`; the runtime sorts `(priority, insertion)` (`registry.ts:73-78`) and `group` is consumer metadata. Sorting by group first would order the UI by how a group is SPELLED. Text fixed and pinned by a new `registry.test.ts` case whose groups are spelled so that alphabetical order is the reverse of the intended one.
- [x] **W11 — view navigation off modeling.** `zoomFit`/`home` are `onecad.shell.command.*` now, with the two chords in a new `modules/shell/bindings.ts` (`keymap.ts` concatenates modeling's global table then the shell's, which reproduces the frozen `GLOBAL_KEYS` order exactly). `Escape`/`Enter` stay modeling — global in reach, modeling in meaning — and `isolate` stays modeling because it masks BODIES. **The trap this wave was really about:** implementing the service over `engineBridge.fitView()` — the obvious reading of "zoom to fit" — silently turns ⇧F from *frame the selection* into *frame everything*, and neither the type system nor the keymap contract notices. `modules/shell/viewportNavigation.ts` therefore delegates to `viewportStore.zoomFit/homeView`, where the selected-body semantics live, and `runAction` calls THAT object rather than the store, so the command and the keystroke are one call. Registered in `shell/module.ts` (bootstrap), NOT `register.ts` — that file imports the chrome components and would drag them into the startup bundle. New `viewportNavigation.test.ts` (8) pins the selection matrix through both entry points, the service lookup, the no-op-before-engine case, and the chords.
- [ ] **Deferred, flagged:** a registry-driven keymap. `useShortcuts` installs a window keydown handler outside React, so routing resolution through the command registry is its own change; `keymap.ts` does a static two-owner merge for now.
- [x] **W12 — ViewportContribution + the full DatumLayer port.** `ViewportContext` carried only `invalidate()`, which was enough to declare the contract and nothing else; the first real producer showed what a layer actually needs. It now also carries `root`, `createLabel`, `onFrame`, `onThemeChange`, `raycastFromClient` and `registerSecondaryHover`, with type-only `three` imports recorded in **ADR-0009** (the forbidden edge is `@/viewport`, not the rendering library, and `slots.ts` already called in-scene contributions Three.js objects). `createLabel` is a façade rather than the `HtmlOverlayDriver` itself — handing out the driver would put a renderer internal in the public surface and let a contribution move labels it does not own.
  - **Six methods deleted from `ViewportEngine`** (`syncDatums`, `datumHitTest`, `setDatumHover`, `setDatumSelected`, `setDatumGhost`, `isDatumGhostVisible`) plus the layer field. A datum is a MODELING record; a viewport that knows what one is cannot host a module that has none. Five caller sites rerouted through `modules/modeling/datumViewport` (`datumSync`, `SketchController` ×3, `ModelToolController` ×3, `ViewportRoot`), and `Picker`'s hardcoded datum branch became `registerSecondaryHover`.
  - **The host is `ContributionHost.ts`, and reconciles — it does not attach once.** Contributions register on editor mount while `engine.init()` is still awaiting the renderer, so a one-shot attach drops whichever side loses that race; `ViewportRoot` hands the engine the REGISTRY and it stays subscribed. Teardown is reverse-order. Failure policy is explicit: a throwing `attach` is rolled back (labels included) and skipped, a throwing frame/theme/hover callback detaches that contribution — one bad layer may not kill a frame.
  - **Ordering is pinned by test**, not by luck: frame callbacks run at the ladder position the datum layer used to occupy, and secondary hover keeps built-ins first, then contributions in registry order, first non-null wins.
  - **`themeRefresh.test.ts` gained its first DatumLayer case, negative-checked** (neuter `refreshColors` ⇒ red, restore ⇒ green). It never had one, and the silent-failure invariant is now *harder* to satisfy for a contribution: it is not in `applyTheme()`'s list at all, so a forgotten `onThemeChange` is invisible to the engine. `viewport/engine/README.md` § Theming says so.
  - Probes followed the mechanism: the e2e datum surface moved from `window.__vpEngine.datumHitTest`/`.isDatumGhostVisible` to `window.__datumVisuals` (DEV-only), published by the contribution.
- [x] **W14 — hardening pass** (Codex implementation review, `gpt-5.6-sol`/high over the committed W8–W12 range; 1 high + 5 medium + 1 low, all verified in source and all fixed, each with a negative-checked regression test).
  - **HIGH — the late-attach hole.** `DatumSync.attach()` pushed the projection ONCE. The layer is a contribution now, so it attaches on the viewport host's schedule: whenever it landed second, that sweep went into a `null` and the datums stayed invisible until an unrelated store change re-pushed them. `datumViewport` gained `onDatumVisualsChanged` (fires immediately with the current value), and `DatumSync` replays the full sweep + selection through it.
  - **Removal did not schedule a frame.** `reconcile()` only invalidated on ADD, so a disposed contribution's objects left the scene graph while the on-demand renderer drew nothing — the last framebuffer kept showing them. Now any change invalidates, including `setRegistry(null)` and a guard-triggered detach.
  - **The label cleanup skipped every second label.** Each `dispose()` splices itself out of the array the host was iterating. Snapshot + `finally`, so a contribution's own throwing disposer can no longer strand host-owned overlay items either. The first version of this test did NOT catch it (the fake tidied up after itself) — rewritten so the host's loop is what runs.
  - **A crashed contribution came straight back.** `guard()` detached it but its registration remained, so the next unrelated register/dispose re-attached it and it threw again. Failed ids are held back until their registration goes away — a re-register is still a genuine second chance, pinned by test.
  - **`canRender` ran outside its own `ContributionBoundary`** (admission is decided before there are children), so a throwing predicate escaped the section and would have taken the whole inspector down. Guarded; a predicate that cannot decide does not admit.
  - **`TreeProvider` gained optional `subscribe`.** A provider backed by data the panel does not watch — i.e. any non-modeling one — could never update its rows after mount. Optional only because modeling's is backed by stores the host already watches for its own reasons; the contract says so.
  - `docs/DEBUGGING.md:243` still told developers to call the deleted `__vpEngine.datumHitTest`; now documents `window.__datumVisuals`.
  - Gate: tsc clean · vitest **224 files / 3837** · contracts byte-identical · hex gate empty · Playwright chromium datum/sketch-on-face/acceptance **14/14**.

### P3 — Milestone 3: SDK boundary + bundled test addon (the next real tranche)
Validates the boundary on OneCAD's own code before any third party sees it (spec §197).
- [ ] `@onecad/sdk` — a package that re-exports ONLY the public surface: ids, contribution contracts, `ModuleScope`, the document-state service, selection/event types. It must NOT re-export `createPlatform`, the registries' internals, or anything from `@/features`/`@/tools`.
- [ ] A bundled test addon (an architectural fixture, NOT a product feature) that registers a command, a panel, an inspector section, a viewport contribution and its own document namespace — importing `@onecad/sdk` and nothing else.
- [ ] Extend `src/platform/architecture.test.ts`: the addon's imports must resolve only to the SDK. That test already carries a positive control; keep that pattern.
- [ ] Prerequisite from P2: the addon cannot contribute an inspector section until inspector sections have a host.

## VF-M5+VF-M6 DEFECT FIXES + IMPORT PROJECT (2026-08-08) — GATE PASSED

Defect fixes from the review round (worker ladder + import blob lifecycle), then the "Import Project" feature: append another `.onecad` document's timeline to the open document. Assembly/joints explicitly out of scope; static import now, live/XREF later.

### VF-M5 — from-0 replay rebases onto stale WORLD anchors
- [x] Root cause: on a from-0 replay an inherited edit ("ALL" buckets, whose refs were animated to the source model) resolves WORLD anchors through the post-edit scene; the replayed model telerecorders scored anchor-exact, which vetoed their legit `NeedsRepair` — replay silently misassigned. The far-edge blend captured it and anchor-rebased nothing.
- [x] Fix (attempt, RE-OPENED — see VF-M5 FOLLOW-UP below): from-0 replay now runs with edit-context `from_zero_replay`, which disables the anchor-exact carve-out during that loop. The checkpoint path (from-zero carrying the user's original edit) still binds AutoBind; the from-ZERO (checkpoint-recreated) replay surfaces the same case as `NeedsRepair` + "ambiguous" so a follow-up repair/step runs as authored. **The gate as built regresses the flagship real-worker edit lane and must be reworked — V1 has no checkpoint restore, so the carve-out should stay ON until a genuine restore basis is plumbed.**
- [x] Plumbed `from_zero_replay` through `LadderEditContext` → `ScratchJob` → `PlanExecutor` → into the four edit ops (`Shell/Hole/Fillet-Chamfer/Extrude`); `job.partition.size()==0` is the from-zero gate — **WRONG as a discriminator, see FOLLOW-UP**
- [x] Docs: gap-closed note in `protocol/IRR/SCHEMA.md` + `docs/qa/…` (records the accepted residual + the gap this runs through).

### VF-M6 — import-blob lifecycle (3 defects)
- [x] `materialize()`: now checks the staged path is actually a FILE (a collided digest that resolved to a directory would previously arm the job against a wall, then corrupt the model). Sweeper now also kills stale per-workspace... clean.
- [x] `sweep_stale_workspaces()`: sweeping is now PID-aware — a workspace survives while its importing process is alive (unix: `libc::kill(pid, 0)`; non-unix lanes disable sweeping rather than guess). Parse + sweep covered by unit tests.
- [x] `prepare_import()`: converted-geometry byte cap enforced (defense in depth; `add_import_record` already caps raw blob bytes).
- [x] Tests: `imports.rs` unit tests for the materialize/parent-dir/absolute path, the PID-aware sweep, and the over-cap reject. Full ctest + cargo green.

### Wave 3 — Import Project (backend)
- [x] `onecad_core::io::project_import` — `read_project_for_import` (open container → records + sketches + import blobs), `find_import_collisions` (refuses a source `RecordId` or `SketchId` that already exists in the target), `ProjectImportError` taxonomy.
- [x] `DocumentRuntime::import_project(path)` — collision-refuse → stage every blob into the import workspace/carrier → one transaction appending each record (`AddOperation` at cursor) + each sketch, merging all outcomes. Refused imports leave the target untouched.
- [x] `api::import_project` — Rust-owned `.onecad` picker; with an open runtime it appends, with none it seeds a blank document (`make_backend` + `new_blank` + adopt), then `PROJECTION_UPDATED` + snapshot + scheduler. No runtime → `NoDocument` error surfaced to the UI hint.
- [x] Wire: `client.ts` interface, `tauriClient` mapping, `commandMap`.
- [x] Tests: `document_runtime/tests.rs` import-append / collision-refuse / blank-runtime import; `imports.rs` lifecycle tests; full cargo workspace green.

### Wave 4 — import through the UI (mock + e2e)
- [x] `mockClient.importProject()` — merges the STEP-import fabrication (one body + `Import` row) so the whole frontend lane is mockable. Mock-unit test covers the appended snapshot.
- [x] `FileMenu` "Import Project…" (Open/Save group, above the Export hairline) → `fileActions.importProject` (dialog-backed, cancel = no-op) — shines one hint "Project imported".
- [x] `StartScreen` — sidebar "Import Project…" below Import STEP→; both entry points pinned by `e2e/project-import.spec.ts` (in-editor + start-screen lanes).
- [x] Unified start-screen import — the header button now says `Import…`, and the start screen routes `.onecad` vs STEP/STP by extension through one dialog. Sidebar duplicate removed. Gate: focused vitest on `StartScreen`, `tauriClient`, and `mockClient.import` green; `cd src-tauri && ONECAD_REQUIRE_WORKER=1 cargo test --workspace --lib` green.

### Gate
- [x] `bun run build` (tsc+vite) green · vitest **209 files / 3697 tests** · Playwright `project-import`/`step-import` spec **12/12** · full Playwright **386/390** (4 pre-existing `theme.spec` failures root-caused to the WORK `TitleBar` changes, not this package) · worker ctest **106/106** · `cargo fmt --all --check` · workspace clippy `-D warnings` · `cargo test -p onecad --lib` **253 passed / 0 failed**. Full `cargo test --workspace` (real-worker lane) and kernelbench left for the follow-up gate below.

### GH-0 — GATE PASSED 2026-08-09 (WP0.0 → WP0.3)
- [x] **`boolean-preview.spec.ts` is GREEN** — 16/16 across chromium + webkit at `--repeat-each=4`. Closed by three fixes, and the root cause was NOT what the plan assumed (see WP0.2 below for the full correction).
- [x] **Kernelbench baseline is now a RETAINED MANIFEST, not a summary.** `bench/robustness/baselines/digests.json` freezes 256 rows of `(suite, caseId, backend, variant) → {inputDigest, normalizedDigest}` for `fillet/foundation:t0` (136) and `fillet/matrix:m1` (120), with `scripts/kernelbench-manifest.mjs record|compare`. The aggregate the gate used to quote cannot answer "did this change move a digest?" — a case can change shape and still pass — and replay only compares two runs of the SAME build. Comparator proven by mutating one saved digest and watching `compare` exit 1.
- [x] Suites re-measured against the HEAD-built pinned worker: T0 **136 records, 136 pass, 0 fail, gatingFailures 0**; m1 **120 records, 114 pass + 6 characterization, gatingFailures 0** — both identical to the previously recorded numbers.
- [x] Gate: ctest **109/109** · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1048 / 0** across 73 targets · `cargo fmt --all --check` + workspace `clippy -D warnings` clean · `bunx tsc --noEmit` clean · vitest **240 files / 4043 tests** (from 236/3986) · hex gate 0 · Playwright **380 passed / 10 failed**.
- [x] **The 10 Playwright failures are NOT this work, and that was verified rather than assumed.** All 10 are 5 specs × 2 browsers (`ellipse` ×3, `center-rect`, `sketch-reattach`) and ALL of them reproduce at `d81f758` in a throwaway worktree — i.e. before any commit of this tranche. They are the concurrent session's `feat(toolbar): tool flyout families` commit: `Ellipse` / `Center rectangle` moved behind a flyout and those specs still query them as top-level buttons (`getByRole("button", {name: "Ellipse"})` ⇒ element not found). `boolean-preview` PASSES at `d81f758` in that same worktree and passes here, so this tranche took the suite from 3 boolean-preview failures to 0 and introduced none. **Owner: the toolbar session** — the specs need to open the flyout, or the flyout needs to keep the family's active tool queryable by name.

### GH-0 WP0.2 — boolean-preview: THREE DEFECTS, root cause NOT what the plan predicted (2026-08-09)
- [x] **Defect 1 — a REJECTED region fetch wiped live selection.** `sketchStaticSync.loadSketch` fell through its `catch` into `reconcileRegionRefs(id, ∅)`, so a transient `getSketchRegions` failure dropped every `sketchRegion` ref for that sketch. An empty set from a FAILED fetch is not evidence a region disappeared; a DELETED sketch still reconciles (that set is authoritative).
- [x] **Defect 2 — a pick landing during a refill read as "empty space".** A token-driven reload calls `removeSketch` and only restores the fills two awaited round trips later; in that window `sketchStaticHitTest` returns null and `ViewportRoot` called `sel.clear()`. Now `ViewportEngine.sketchStaticPickState` returns an ATOMIC `hit | miss | unsettled` and an unsettled pick DEFERS (one bounded retry, cancelled by the next pick, released on detach) instead of clearing. Polling a readiness flag cannot fix this — the window can open between the poll and the click, which is why the old `sketchHitTestReady` passed immediately before the failure.
- [x] Both proven red-first: reverting either fix turns the three new `sketchStaticSync.test.ts` cases red (verified by actually reverting them, not by inspection).
- [x] **Defect 3, and the ACTUAL root cause — a stale click coordinate, not a race in the pick lane.** Instrumenting the pick with both hits on a failing webkit run showed `state:"miss"` with **no body hit AND no sketch hit**: the ray lands on empty space. `extrudeRegionAt` projected the sketch-plane point to client coordinates without waiting for the camera; re-showing a sketch schedules the DEBOUNCED auto-fit, which then moves the camera and invalidates the coordinate. `waitForCameraSettled` already existed for precisely this and its own comment describes the failure ("a fit can be scheduled-but-not-started … invalidating any client coordinate the caller computed from a probe") — `extrudeRegionAt` simply never called it.
- [x] **Two hypotheses were wrong and are recorded so they are not re-tried.** (a) The plan's assumption that the refill teardown window was the cause: `sketchHitTestReady` polls the tri-state and reports `hit` immediately before the click, and the deferral only engages on `unsettled`. (b) The follow-up hypothesis that HIT PRECEDENCE was returning the body ref for a point over the region: refuted by the probe — there was no body hit at all. Fixes 1 and 2 are still correct defects on their own merits; neither was this one.
- [x] **Defect 4 — `clickApplyButton` could silently not click.** Its `toPass` retry only asserted that a rect EXISTED, so a click that missed (the chip is overlay-positioned and can move between measuring and clicking) still returned successfully and left the caller to time out on a lane that was never applied. It now retries on the OUTCOME (lane closed), re-checking first so a successful apply is never clicked twice.
- [x] An earlier attempt to make the spec wait for the region to appear in `selectionStore` before clicking Extrude was REVERTED: it made things worse (4 failures) because it asserts a precondition the product does not guarantee at that point.
- [x] Result: **16/16** on `--repeat-each=4` across both browsers.

### VF-M5 RESIDUAL — CLOSED 2026-08-09 by `checkpointFallbackReplay` (GH-0 WP0.0+WP0.1)
- [x] **RED FIRST, and the entrance criterion was real.** `topology_rebind::vfm5_lane_d_checkpoint_fallback_replay_must_not_bind_the_decoy`: checkpoint selected → worker killed → restore reports `restored:false` → F12 replay → the fillet **silently consumed a congruent decoy parked on the stale anchor** (`needsRepair=0`, `removed=10.3009` = the analytic wedge, `centroidY=69.00107` matching the decoy prediction exactly against the authored rib's `68.99535`). Green after the field lands.
- [x] **Three constraints found by measurement, all of which had made the earlier fixtures vacuous.** (a) A sketch at step 0 can only regen with `ToEnd { from: 0 }`, which claims no `editedFrom` — so `post_upstream_edit` is false and the veto never arms. That is why every H5 test uses `regen_all` and why NONE of them reach this carve-out; the fixture puts the comb sketch at step 2 behind a throwaway body. (b) Editing a `TransformBody` **seeds** NeedsRepair downstream (`transform_body.rs:906`), so it never reaches the ladder and cannot be the driver. (c) At 20 mm of rib separation the ordinary auto-bind margin gate refuses first (measured margin 0.0745 < 0.10) and the carve-out never decides — the scene needs 48 mm.
- [x] Independently confirmed the veto is reachable end to end by forcing `from_zero_replay = true` and running the suite: the ONLY test that flips is `h6a` (the false-positive direction), and neither H5 decoy test changes.
- [x] **The fix is the protocol change this entry demanded.** SCHEMA §7.2 `checkpointFallbackReplay` (OPTIONAL, omitted when false), set by exactly one place — `regen/executor.rs`'s F12 branch — and consumed by `PlanExecutor.cpp` as the sole source of `job.from_zero_replay`. §14 entry + §13 lockstep note (the worker is a bundled `externalBin` sidecar, so no capability negotiation is needed — but that property is now written down as load-bearing). Two canonical fixtures added, `protocol/fixtures/execute_plan_{ordinary,checkpoint_fallback}.ndjson`, run by BOTH tracks (`messages.rs` list, `worker/tests/CMakeLists.txt`, `check_interop.sh`).
- [x] **The ordinary edit lane is deliberately UNCHANGED** and now pinned by its own characterization test (`vfm5_teleport_on_the_ordinary_edit_lane_is_the_accepted_residual`): the teleport still binds the decoy there. That is the documented H6a residual, not a bug this tranche fixes — a change to it must be a decision, not a side effect.
- [x] Accepted trade: a legitimate anchor-exact rebind occurring inside an F12 fallback now returns `NeedsRepair` (a conservative false positive on a rare lane). Deterministic `NeedsRepair` beats a silent wrong bind.
- [x] Gate: ctest **109/109** (107 → 109, the two new canonical fixtures) · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1048 / 0** across 73 targets (from 1045/0 across 72) · `cargo fmt --all --check` + workspace `clippy -D warnings` clean · worker built from HEAD against `~/.onecad-occt/8.0.1`.
- [x] Stale "V1 has no restore / checkpoints UNSUPPORTED" prose corrected everywhere it was asserted: `worker/src/session/Session.cpp`, `ScratchJob.h`, `Ladder.h`, `CURRENT_STATE.md` (both sites), `HANDOFF.md`.

### VF-M5 FOLLOW-UP — real-worker lane RED (the flagship regression) — CLOSED 2026-08-09
- [x] `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` against a worker built from HEAD **FAILED** `topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean` (`needsRepair` expected 0, got 1). Reproduced here on a fresh `worker/build-pinned` build before changing anything.
- [x] Root cause confirmed by reading, not inferred: `Session::fence_and_clone` clones an EMPTY base for any plan whose expectedBaseHash is the empty anchor (D5, `Session.cpp:153-175`), which is every ordinary regen — so `job.partition.size() == 0` in `PlanExecutor` is always true and `from_zero_replay` degenerated to `edited_from.is_some()`.
- [x] Fix: resolution (a). `job.from_zero_replay = false`, restoring the shipped edit lane. **The SECOND half of that entry — "the hazard cannot occur in V1" — was wrong and is corrected in § VF-M5 RESIDUAL above.** Ladder behaviour untouched and still covered by `worker/tests/test_wp6_ladder.cpp`.
- [x] Re-verified on the real-worker lane: `topology_rebind` 13/13, `cargo test --workspace` **1045 passed / 0 failed** across 72 targets, ctest **107/107**.

## ADVANCED-FILLET ROADMAP — M0 REPRODUCIBLE-GREEN + M1 QA-AUTOMATION (2026-08-08) — GATE PASSED

First two milestones of the Advanced-Fillet program (kernel robustness 70% / direct modeling 20% / UI 10%; KPI = raw-OCCT rescue rate, with `raw PASS → OneCAD FAIL` treated as a serious regression and invalid geometry always worse than a safe refusal). No production fillet algorithm was touched — by design, the benchmark discovers the problem before the kernel guesses at a solution.

### M0.1 — OCCT build fingerprint: effective configuration, not literal presence
- [x] Root-caused against the pinned source AND a real configure, not assumed. OCCT's `OCCT_CHECK_AND_UNSET` does `unset(VAR CACHE)`, and `OCCT_IS_PRODUCT_REQUIRED` derives each `CAN_USE_*` by scanning `BUILD_TOOLKITS`. With `BUILD_MODULE_Draw=OFF` nothing requires `CSF_TclTkLibs`, so `CAN_USE_TK` is OFF and CMakeLists (8.0.1 L558-565; 7.9.3 byte-identical over every `USE_*`/`CAN_USE_*`/`OCCT_CHECK_AND_UNSET` line) DELETES our `-DUSE_TK=OFF` from the cache. The old literal-presence loop then aborted a configuration that was exactly what we asked for. Nine further keys can vanish the same way: `BUILD_USE_PCH USE_DRACO USE_FFMPEG USE_FREEIMAGE USE_FREETYPE USE_OPENGL USE_RAPIDJSON USE_TBB USE_VTK`.
- [x] New `scripts/occt-fingerprint.sh` — sourceable, side-effect-free, bash 3.2 compatible. Keys are classified REQUIRED vs DEPENDENT. An absent DEPENDENT key normalizes to `OFF` **only when the policy requested OFF** (OCCT deleted it precisely because the feature is not in the build); requested-ON-but-dropped is fatal, a missing REQUIRED key is fatal, a present key that disagrees with policy is fatal, and a value outside CMake's documented truth constants is fatal rather than guessed. Normalization, not relaxation.
- [x] New `scripts/tests/occt-fingerprint.test.sh` — 14/14, no network, no OCCT build. The 8.0.1 fixture is a verbatim capture of the fingerprinted lines from a real `CMakeCache.txt` (including the `:UNINITIALIZED` types a bare `-D` produces); every absence carries its CMakeLists citation. Covers both versions producing byte-identical fingerprints, alternate boolean spellings, duplicate cache lines, and all four failure modes.
- [x] `build-pinned-occt.sh` sources the library and restates the policy string, so an edit (or a pre-set env override, the seam the test uses) on either side cannot silently change what a real build is fingerprinted against.
- [x] CI: new fast `occt-fingerprint` job gates the hour-long OCCT jobs; both cache keys now hash `occt-fingerprint.sh` as well.
- [x] VERIFIED END TO END: pinned OCCT 8.0.1 built from scratch into a clean prefix and installed, `occt-build.json` carrying the correct `normalizedBuildOptions`; the reuse path then short-circuits. `USE_TK` is genuinely absent from the produced cache — the old script would have hard-failed here.

### M0.2 — frontend clean build
- [x] `bun run build` was red while `bun run test` was green, because vitest does not typecheck. Two `tsc` errors in `faceColors.test.ts`: a hand-written structural mock of the 24-field `BodyMeshView` (missing 8 fields) and readonly `FaceColor` assigned into a mutable tuple.
- [x] Fixed at the type level, no casts: `Rgba` is now `readonly [number, number, number, number]` (a color is a value, never mutated), and the ~20 duplicated raw `[number, number, number, number]` literals across `documentStore` / `mockClient` / `tauriCommandMap` / `meshSync` collapse onto it. Read-only consumers (`faceColors.ts`, `meshRegistry.ts`) take `ReadonlyMap<string, Rgba>`.
- [x] New `src/test/fixtures/bodyMeshView.ts` → `makeBodyMeshViewFixture({faces, idsHaveElementIds, …})`, deriving `faceRanges`/`faceIdOffsets`/`faceIdChars`/counts/flags/bbox so a fixture cannot disagree with itself. Prefer `parseMeshPayload(makeBoxMesh(...))` when MESH1 framing matters; the builder exists for shapes MESH1 fixtures cannot produce (ElementId-bearing face ids).
- [x] Hex gate regression found and fixed while gating: `InspectorPanel.rgbaToHex` returned the literal `#a9aeb6` — which IS `--color-body-fill`. Now resolved through `palette.bodyNeutral()`, so the no-color swatch follows the theme instead of freezing at the light value. `grep -rn '#[0-9a-fA-F]\{6\}' src` is back to zero.

### M1 — manual-QA debt triaged
- [x] All ~112 boxes of the historical `docs/MANUAL_GATES_RUN.md` classified into K (CTest/Kernelbench) · H (Rust real-worker) · F (Vitest/Playwright) · M (manual). Result: **78 already asserted by an existing automated test and retired** (each cited by `path:line`), **22 named gaps** that belong in an automated lane but have no test yet, **12 genuinely manual** checks.
- [x] `docs/qa/MANUAL_RELEASE_GATES.md` — the 12 survivors only, grouped by WHY a machine cannot judge them: visual (4), native (4), hardware (4). Carries an explicit "do not grow this file" rule — a new manual permutation means a missing automated lane, which is exactly the pressure advanced fillet would otherwise apply.
- [x] `docs/qa/MANUAL_GATES_TRIAGE.md` — the item-by-item classification plus the GAP-K/H/F backlog. Nothing was deleted: every check is cited as covered, listed as a gap with its target lane, or moved into the release checklist.
- [x] Historical checklist archived verbatim at `docs/qa/archive/MANUAL_GATES_RUN-2026-08-04.md`; stale pointers in `README.md` and TODO :155 updated.

### M0.4 — viewport auto-fit regression, found by the gate
- [x] The gate's e2e run failed in a large cluster (`constraint-apply`, `live-dim-line`, `marquee`, `measure`, `navigation`). BISECTED, not assumed: `measure.spec.ts` is 5/5 at `d1c5339`, **1/5 at `1fe0cef`**, and 5/5 at `1fe0cef` with only the auto-fit debounce reverted.
- [x] Cause: `1fe0cef` replaced "auto-fit once on the first body" with a 250 ms debounced re-fit (right intent — a multi-body assembly must not be framed on whichever body streamed in first), but the timer lived in the React bridge. A `fitView()` tween could therefore START after everything else already believed the camera had settled, invalidating any client coordinate computed from a probe. User-visible too: a queued fit can snap the camera mid-interaction, and one landing during sketch entry is saved as the restore pose.
- [x] Fix keeps the intent and makes the scheduled state real: the debounce moved onto `ViewportEngine` as `requestAutoFit()` / `autoFitPending` / `cancelAutoFit()`, cancelled by explicit `fitView`, `fitToBodies`, `enterSketch`, and `dispose`. `ViewportRoot` just calls `engine.requestAutoFit()`; `waitForCameraSettled` now requires no tween AND no pending auto-fit. Full suite 12+ failures → **7**.

### Gate
`bun install --frozen-lockfile` · `bun run build` (tsc+vite) · vitest **208 files / 3687 tests** · `scripts/build-pinned-occt.sh` 8.0.1 clean build + reuse path · `scripts/build-worker.sh Release` against the PINNED prefix · ctest **106/106** on the pinned worker · `cargo fmt --all --check` · `clippy --workspace --all-targets -D warnings` · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1000 passed / 0 failed** against the pinned worker · kernelbench T0 both-backend **136/136 pass, 0 rescued, 0 regressions, 0 replay-unstable** · fingerprint normalizer **14/14** · hex gate 0 hits · Playwright **183/190**.

### M0.5/M0.6 — the remaining 7 e2e failures, root-caused not retried
All seven reproduced on a pristine `1fe0cef` checkout; none was caused by this work package. Each is now fixed at its cause:
- [x] `transform-body ×4`: the spec's `bodyBounds` folded the fat-line EDGE layer into the bbox. `Line2` reports `isMesh === true`, and its `position` attribute is the instanced unit-quad TEMPLATE `(-1,-1,0)…(1,2,0)` — the real endpoints live in `instanceStart`/`instanceEnd`. Every body's min was therefore clamped to ≤ -1 while `max` stayed correct, which is exactly the observed `[-37, -1, -15]` (only the component inside that box was wrong). Traversal now takes `userData.kind === "face"` only.
- [x] `history-inline-dimension:213`: `toolApplicability.ts` is NEW in `1fe0cef` — the toolbar is selection-gated now, and committing an extrude hides its sketch, so none of sketch/region/sole-visible-sketch held and Extrude sat `aria-disabled` while the click burned the 45 s timeout. Also `getByLabel("Dimension value")` matched both the row editor and the tool chip. Re-show the sketch (eye toggle, NOT a selection change — the row selection has to survive) and scope the locator to the row.
- [x] `hole:226`: `findFaceOnBody` probed pixels before the camera settled, so the returned point sat on a face about to move ("no second pixel far enough on the same face"). Helper settles first.
- [x] `sketch-degenerate:35` + `live-dim-mouse-rounding:62`: `enterSketch` aims along the plane normal at `controls.getDistance()`, so the sketch view INHERITS the model camera distance — which sets `planePixelWorld`, hence the draw tools' screen-constant reject radius (`minSize = 4 × planePixelWorld`) AND the zoom-adaptive dimension quantum. Entering mid-fit made both non-deterministic run to run; every caller settled AFTER entering, too late. `enterSketchViaPlanePicker` settles first.
- [x] Verified 52/52 across `transform-body`, `history-inline-dimension`, `hole`, `sketch-degenerate`, `live-dim-mouse-rounding`, `measure`, `constraint-apply`, `marquee`, `navigation`.
- [ ] OPEN DECISION: `playwright.config.ts` uses `retries: 1` in CI, `0` locally. Every flake above was invisible in CI and hard-red locally. Now that they are root-caused rather than retried away, dropping the CI retry is the honest setting — a retry is what let the auto-fit regression look green.

### Concurrent-session caveat
A second session was editing this tree throughout (`TitleBar.tsx`, `ViewCube.tsx`, `CadOrbitControls.ts`, `Picker.ts` — `hasHitAt` removed — `ModelToolController.ts`, `navigation.spec.ts`, `document_runtime/tests.rs`) and added a **webkit** Playwright project (suite 190 → 386). A full-suite number is not attributable while that is in flight: one run hit a live `[vite] Transform failed` from a mid-edit save, and `theme.spec:121,145` fails on `getByRole('button', {name: /^Appearance:/})` — element not found — from the `TitleBar.tsx` change. Not from this work package.

### M2 part 1 — kernelbench case-v2 — DONE
- [x] `bench/robustness/schemas/case-v2.schema.json` (strict throughout) + `src-tauri/crates/onecad-kernelbench/src/case_v2.rs` (a SEPARATE type set, not an extension) + `bench/robustness/examples/fillet-matrix-plane-cylinder-v2.json` (ajv-validated; deliberately NOT in `regressions/`, which is contractual).
- [x] `selector` moved to top level — the same geometry+selection is reused across radius laws and later across operations, so a resolving selector is a PRECONDITION for the operation, not part of its definition.
- [x] `operation.definition` mirrors the kernel-level `FilletDefinition` (radius law + continuity), so `constant`/`linear`/`controlPoints` are all expressible and adding one later is additive on both sides.
- [x] `geometry.parameters` is a typed per-recipe union rather than the free-form object the plan sketched: a free-form bag lets a typo silently generate different geometry, which is the one failure mode a fuzzing corpus cannot survive.
- [x] `continuity` admits only `g1`, `sizeType` only `radius` (agent rules 11/15 — an option is not a capability; G2 and chord width become expressible only once a validator can prove the result).
- [x] Recipes expressible: the v1 four, plus `supportPair`, `valenceCorner`, `shortEdge`, `microEdge`, `sliverNeighborFace`, `tinyNeighborFace`, `periodicSeam`, `nearSeamEdge`, `faceNearlyConsumed`, `faceFullyConsumed`, `blendCollision`. Supports span plane/cylinder/cone/sphere/torus/bspline; `scaleBand` covers 1e-3 … 1e6.
- [x] Metamorphs expressible: translation, farOriginTranslation, rotation, mirror, uniformScale, parameterEpsilon, edgeOrderPermutation, contourSeed (M3's set, schema side).
- [x] Guards: v1/v2 provably disjoint (both directions, in Rust AND ajv) · v1 regressions still validate under `Case` · round-trip reproduces the committed file, which already caught three missing `#[serde(rename)]`. Cross-field rules that JSON Schema cannot express live in Rust and are authoritative because Rust is what executes: mode ⇒ (anchor count, adjacency relation) as ONE table; control-point parameters strictly increasing; torus `minorRadius < radius`; `edgeOrderPermutation` a real permutation; zero epsilon rejected.
- [x] Gate: kernelbench 50 lib + 5 integration green · `cargo fmt --all --check` · workspace `clippy -D warnings`.

### M2 part 2 — geometry generators + the v2 execution lane — DONE

The cross-language half: the worker can now execute a `schemaVersion: 2` case, and the supervisor can drive one end to end.

- [x] **C++ `CaseSpec` is normalized, not forked.** `Types.h` carries the v2 shape (`schema_version`, `generator.family`, `recipe` + typed `RecipeParameters`, `RadiusLaw`, `scale_band`, selector `convexity`/`vertexValence`) and the v1 parse fills the subset, so ONE execution path serves both formats. Duplicating the executor per version is where a v2-only bug would hide. `parse_case` dispatches on `schemaVersion`; v1 acceptance is unchanged.
- [x] `CaseParserV2.cpp` (new) mirrors `case_v2.rs::validate` field for field — the Rust side is authoritative for what a case may CONTAIN, the C++ side for what the runner will EXECUTE, and a disagreement would make a valid case fail as an invalid request. Shared primitives moved to `CaseParserShared.h` rather than being copied. A non-constant radius law is REFUSED at parse (`invalidRequest`), not silently flattened to its peak — variable radius is M12, with a validator that can prove the law.
- [x] `SelectorParser` gains the v2 selector: top-level, wider surface/curve vocabulary, `chain`/`closedContour` modes, `supportPairEdge`/`seamEdge`/`shortEdge` roles, and the mode ⇒ (anchor count, adjacency relation) table as ONE table. `parse_case_v2` re-checks `schemaVersion == 2` itself, so the entry point cannot be called directly on a v1 document — a gap the new fixture caught.
- [x] **`supportPair` generators** (`GeometrySupportPair.cpp`, new). Two constructions: the PRISM family (plane|cylinder × plane|cylinder) is one 2D profile extruded along Z, giving a straight shared edge through the origin and a FREE dihedral over `(0,180)`; the CONE family (plane × cone) is a frustum's base circle, where the dihedral is `90 - halfAngle` by construction — so the declared angle is rebuilt from the half angle and a disagreement is refused rather than silently generating different geometry than the file describes. Unimplemented kinds (sphere/torus/bspline) and concave supports refuse BY NAME.
- [x] Why the prism family and not the committed example's base-circle form: the example locked plane↔cylinder to a 90° dihedral, and the dihedral is the main conditioning axis for fillet failure. The prism form also keeps the blend a CYLINDER, which is what lets `constantRadius`/`cylindricalRadius` gate it. `examples/fillet-matrix-plane-cylinder-v2.json` updated to match (anchor, `curveKind`, `edgeLength`) — examples are explicitly not contractual; `regressions/` untouched.
- [x] **Rust `PreparedCase`** (`prepared.rs`, new) is the version-agnostic view the supervisor executes: identity, resource ceilings, metamorph tolerance, and the canonical document. `campaign`/`runner`/`child`/`result`/`result_validation` route through it, so none of them grew a `match` on the schema version. A v2 case reports the v1 `{name, version, seed}` generator shape because the RESULT schema is frozen at v1 — `Execution.cpp` now BUILDS that block instead of echoing the case's, which is what made the first v2 run fail 60/60 on strict result validation.
- [x] **`suite_v2.rs` — `fillet/matrix` preset `m1`**: 24 cases (3 prismatic pairs × 5 dihedrals supported + 2 near-tangent exploratory, plus 3 cone half-angles), 60 records with metamorphic variants. `cli` gained the suite and a version-dispatched `run-case`.
- [x] **Sizing is measured, not assumed.** A blend needs room; for a prismatic pair the throat is `R·(1 - cos θ)`. Swept against OCCT 8.0.1: every pair blends at 0.20 of the throat, the cylinder↔cylinder lens refuses at 0.40. Supported cases sit at 0.04–0.16, so a red there is a kernel regression and not a greedy case. Near-tangent dihedrals (170°, 178°) are `exploratory`, NOT `expectedLimit` — an expectedLimit case needs a radius the kernel is KNOWN to refuse, and guessing one produces a case that passes whatever happens.
- [x] **Validator finding: `cylindricalRadius` and `g1BoundaryTangency` are box-shaped.** The first counts EVERY cylindrical face in the output against the requested radius, so a cylindrical SUPPORT reads as a blend of the wrong radius (measured 17.0 error on a 20 mm support); the second only recognises plane↔cylinder tangency pairs and cannot see a blend meeting a curved support. Both are emitted only for all-planar pairs, pinned by test. Their general replacements (`supportTangency`, `crossSectionProfile`) are expressible in case-v2 but unimplemented — and an unimplemented validator reports `notApplicable`, which FAILS a required check rather than passing it, so emitting them now would red the whole suite. They land with M4.
- [x] `kernelbench_case_v2` ctest fixture asserts GEOMETRY, not the parser echoing itself: the built dihedral is measured from the two adjacent face normals at the selected edge's own midpoint and compared to the declared angle to 1e-6, across 3 pairs × 3 angles, plus the cone-mismatch refusal, unimplemented-kind refusal, version disjointness, and the variable-law refusal.

### Gate — M2 part 2
`ctest` **107/107** on the pinned OCCT 8.0.1 worker (106 → 107, the new `kernelbench_case_v2`) · `cargo fmt --all --check` · workspace `clippy -D warnings` clean · kernelbench **56 lib + 5 integration** (50 → 56) · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1032 passed / 0 failed** across 73 targets against the pinned worker (1000 → 1032) · **T0 both-backend UNCHANGED: 136/136 pass, replay 136 stable, metamorph 48 pass / 0 fail / 16 notRun, differential 136 same-status, `gatingFailures: 0`** · **M1 both-backend: 120 records, 114 pass + 6 characterization (the near-tangent exploratory refusals), replay 120 stable, metamorph 72 pass / 0 fail, differential 120 same-status, `gatingFailures: 0`, p50 10.3 ms / p95 62.3 ms** · the committed v2 example executes through `run-case` on both backends, pass + replay-stable · ajv-valid under Draft 2020-12.

### M3 — metamorph execution (DONE)

- [x] `VariantSpec` (`worker/src/benchmark/Types.h`) and `suite::Variant` widened to the full v2 metamorph set: `translation`, `rotation`, `mirror`, `scale`, `parameterEpsilon`, `edgeOrderPermutation`, `contourSeed`, plus `farOriginTranslation`. All fields serialize/deserialize on both sides with strict unknown-field rejection.
- [x] `apply_variant` (`Geometry.cpp`) implements mirror (about volume center or explicit center), uniform scale (about volume center or explicit center), and keeps the existing translation/rotation. `edgeOrderPermutation` and `contourSeed` reorder `selected_edges`, exercising selector order-independence.
- [x] `metamorph.rs` inverse-transforms mirror, scale, far-origin translation, rotation, and translation; identity no-ops for parameterEpsilon/edgeOrderPermutation/contourSeed. Shape-signature comparison is genuine, not fabricated.
- [x] `parameterEpsilon` is handled on the request side: `Execution.cpp` multiplies the effective fillet radius by `(1 + relativeDelta)` when the variant requests it.
- [x] `fillet/matrix:m1` preset includes the rigid/isometric metamorphs (translation, far-origin translation, rotation, mirror, edge-order permutation, contour seed). `uniformScale` and `parameterEpsilon` were held out here because they intentionally change mass properties and so failed the equality-only gate; M3.5 below replaced that gate with a per-variant relation and put both back in the preset.
- [x] Gate passed: T0 **136/136** pass, replay stable, metamorph 48 pass / 0 fail / 16 notRun, differential same-status, `gatingFailures: 0`. `fillet/matrix:m1` **264 records** (24 base cases + 18 supported × 6 variants), **258 pass + 6 characterization**, replay stable 264, metamorph 216 pass / 0 fail, differential same-status, `gatingFailures: 0`. p50 **10.9 ms** / p95 **69.3 ms** (macOS AppleClang, Release worker, debug supervisor).
- [x] Manifest baselines updated: `bench/robustness/baselines/digests.json` now records 136 T0 rows + 264 m1 rows for `darwin-arm64`; `semantics.json` records both suites.

### M3.5 — non-isometric metamorph policy (2026-08-13) — DONE, GATE PASSED

Roadmap Phase 5 WP5.1 residual. Policy: the comparison carries a RELATION, chosen by the variant name, so nothing new is recorded in the frozen result-v1 `metamorphEvidence` block (`additionalProperties:false`, and the relation is a pure function of a field already there).

- [x] `campaign::Relation` — `Equivalence` (the six rigid variants, unchanged), `Similarity{factor}` (`scaled`: `k³·V`, `k²·A`, points already inverse-scaled by `metamorph.rs`), `Continuity{delta}` (`parameterEpsilon`: bounded signed response, shape tolerance widened to `8·radius·|δ|`).
- [x] Not a relaxation, and that is asserted: a kernel that scales the solid but leaves the blend at the original radius fails similarity (`a_similarity_that_skips_the_blend_fails`); a volume that GROWS as the radius grows fails continuity, as does a jump far larger than the nudge.
- [x] Coefficients are measured, not guessed. Over m1, both backends: worst similarity residual **5.6e-12** (allowance 1e-9), worst continuity response **0.037** volume / **0.080** area per unit δ (allowance 0.5), worst sample displacement **3.58·r·δ** (allowance 8). The last one is geometric, not noise — tangency lines move by ≈`r·δ/tan(θ/2)`, so the 30° pair dominates and the allowance covers down to ~14°.
- [x] `uniformScale` (factor 2.0) and `parameterEpsilon` (δ 1e-3) are back in the `m1` preset: **336 records** (24 base + 18 supported × 8 variants), 330 pass + 6 characterization, metamorph 288 pass / 0 fail, replay 336 stable, differential 336 same-status, `gatingFailures: 0`.

**A REAL DEFECT, found by the two new variants and fixed here.** `cylindricalRadius` and `g1BoundaryTangency` failed on every plane-plane case under both. Cause: `Execution.cpp` fillets with an `effective_radius` (scaled / nudged) but `validate_output` measured the blend against `benchmark_case.radius`, the DECLARED one — so the validators were checking the wrong shape and reporting a false red on a correct result. `effective_radius` is now threaded into `validate_output` and reaches `cylinder_evidence`, `tangency_evidence`, `constantRadius`, `cylindricalRadius` and the `radiusTolerance` scale. Red-first proven: `worker/tools/kernelbench-runner/metamorph_fixtures.cpp` (new ctest `kernelbench_metamorph`) fails with `cylindricalRadius=fail g1BoundaryTangency=fail` against the old call and passes against the new one.

Baselines: m1 digests + semantics re-recorded for `darwin-arm64` (336 rows). **Every m1 `inputDigest` moved, and that is expected** — `input_digest` hashes the canonical case document, which now declares eight metamorphs instead of six; `normalizedDigest` follows because it hashes a result containing `inputDigest`. T0 is byte-unchanged: 136 rows, `compare` OK.

Gates: `ctest` **115/115** (114 → 115); `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -D warnings`; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1076 passed / 0 failed**; kernelbench lib **61** (56 → 61); T0 both backends **136/136**, metamorph 48 pass, replay stable, `gatingFailures: 0`; m1 as above. p50 **11.8 ms** / p95 **68.9 ms**. No frontend file changed, so vitest/Playwright are not implicated.

- [ ] `m1` still has `darwin-arm64` rows only; record `linux-x64` when the suite next runs on the self-hosted host (roadmap Phase 5 WP5.6).

### M4 — recipe-agnostic validators (2026-08-13) — DONE, GATE PASSED

Roadmap Phase 5 WP5.2. All six land and are REQUIRED on every m1 case, curved pairs included.

- [x] **The blend is taken from the builder's history, not guessed from surface type.** `AdapterResult` gained `blend_faces` (`Generated`) and `support_faces` (`Modified` of the selected edge's own supports) — both backends fill them from the same `BRepFilletAPI_MakeFillet` history. This is the fix for the assumption `cylindricalRadius` encodes: "the cylinder in the output is the fillet" is false the moment a SUPPORT is a cylinder.
- [x] **`supportTangency`** — angle between the blend's and the support's outward normals, sampled along every shared boundary. The blend's boundary with the END CAPS is excluded: it is a right angle by design, and including it failed every non-cone case at exactly π/2 during bring-up.
- [x] **`crossSectionProfile`** — `1/|k|` for the larger principal curvature versus the requested radius. A constant-radius blend is a canal surface whose circular sections are lines of curvature, so this is exact on plane, cylinder and cone alike. Measured worst deviation over the whole matrix: **2.7e-14 mm**.
- [x] **`manifold`, `noSelfIntersection`, `microTopology`, `toleranceGrowth`** — the itemized halves of `deepAudit`, so a red campaign names its cause. `toleranceGrowth` always reports input/output tolerance maxima and their ratio, and gates on the ceilings the CASE declares; declaring none yields `notApplicable`, which fails a required check. m1 now declares 1e-6 vertex/edge/face (ten times the 1e-7 measured) and zero micro-edges/slivers.
- [x] `notApplicable` fails a required check, as the spec demands — that is precisely what these six returned before they were implemented, and what the new ctest `kernelbench_validators` reproduces against the old dispatch (`plane-plane: supportTangency = notApplicable`).
- [x] **Tolerances carry a conditioning term.** `farOriginTranslation` rebuilds the model 1.7e6 mm out, where double precision costs six orders of magnitude: the boundary angle goes 3.3e-15 → **7.5e-9 rad** and the section error 6.1e-16 → **2.7e-9 mm**. The allowance is `feature scale + coordinateMagnitude·1e-14` (divided by the radius for the angle, since an angle error is a position error over the feature size). Worst measured/allowed over m1, both backends: **0.090** tangency, **0.084** section — an 11× margin that still fails a real defect by orders of magnitude.
- [x] `cylindricalRadius` / `g1BoundaryTangency` stay exactly where their assumptions hold (all-planar pairs) and are untouched in frozen v1.

Gates: `ctest` **116/116** (115 → 116); `cargo fmt --all --check`; `clippy --workspace --all-targets -D warnings`; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1076 / 0**; T0 both backends **136/136** with digests AND semantics byte-unchanged; m1 **336 records**, 330 pass + 6 characterization, metamorph 288/0, replay 336 stable, `gatingFailures: 0`. m1 digests re-recorded (the case documents gained six validators and the quality ceilings); m1 semantics unchanged, so no verdict moved. No frontend file changed.

- [ ] Next in Phase 5: WP5.3, the Boolean foundation campaign (`boolean/foundation:t0`).

### Then — see HANDOFF.md for the resume recipe

### Then — see HANDOFF.md for the resume recipe
M3 metamorph execution (the v2 metamorph set — mirror, uniformScale, farOriginTranslation, parameterEpsilon, edgeOrderPermutation, contourSeed — is expressible and validated on both sides but only translation/rotation are EXECUTED) → M4 failure taxonomy + the recipe-agnostic validators (`supportTangency`, `crossSectionProfile`, `noSelfIntersection`, `manifold`, `toleranceGrowth`, `microTopology`), which is what unblocks required validators on curved-support pairs → M5 `--jobs/--shard/--resume` → M6 minimizer + regression promotion → M7 SQLite ingest + static HTML dashboard → M8 large characterization campaign incl. the `scaleBand` sweep (production fillet algorithms still UNTOUCHED) → M9 baseline KPIs → M10 `FilletBuilder(FilletDefinition)` → M11 OCCT capability spike → M12 variable radius → M13 critical-radius diagnostics → M14 second campaign → M15 first evidence-driven rescue strategy → M16 measure rescue improvement → M17 chord width / verified G2 / overflow / corners → M18 Shapr3D goldens → M19 DirectEditPlanner + reblend. KBR case-v1 stays frozen.

Also still open from M2 part 2: the remaining v2 recipes (`valenceCorner`, `shortEdge`, `microEdge`, `sliverNeighborFace`, `tinyNeighborFace`, `periodicSeam`, `nearSeamEdge`, `faceNearlyConsumed`, `faceFullyConsumed`, `blendCollision`) parse but refuse at generation by name; sphere/torus/bspline supports likewise; concave support pairs likewise.

## BODY/FACE COLOR (2026-08-07) — GATE PASSED

User-authored body color + per-face color, persisted by `ElementId`, baked to vertex colors, editable from InspectorPanel.

- [x] Rust: `BodyMeta` gains `color` and `face_colors: BTreeMap<ElementId, [u8; 4]>`; additive serde keeps legacy documents byte-identical. Split/merge winners inherit both. `SetBodyColor` and `SetFaceColor` commands with metadata-only `RestoreBodies` inverse. Projection DTO emits `color` + `faceColors`; save path merges them into the persisted registry.
- [x] Frontend types + actions: `BodyMeta`/`BodyProjection` carry `color` and `faceColors`; `treeActions` exposes `setBodyColor` and `setFaceColor` with optimistic local writes and revert-on-rejection. `tauriCommandMap` wires both to `EditCommand` with bare body-id normalization.
- [x] Mock client: `setFaceColor` case, persistent `faceColors` in owned metadata, deterministic elementId→topoKey resolution so persisted face colors survive reload, and `elementInfo` resolves by mapped or scanned element id.
- [x] Rendering: `faceColors.ts` bakes body color and authored face colors (precedence: face > body > theme neutral) into de-indexed `Float32Array` vertex colors, looking up by the mesh's actual id (ElementId when `idsHaveElementIds`, else TopoKey). `meshSync` resolves `faceColors` directly when mesh ids are ElementIds; falls back to `elementInfo` only for TopoKey meshes. Rebuilds on change.
- [x] UI: InspectorPanel shows an Appearance section for body selections (color + opacity + reset) and for face selections when `elementId` is present. Fix: the face section had been dropped from `InspectorPanel.tsx`; re-added with correct status label, name fallback, and DOF suppression.
- [x] Gates: `cargo fmt --all --check`, workspace `clippy -D warnings`, `cargo test --workspace --all-targets` all green; `bun run build` and `bun run test` green (206 files / 3636 tests). Added focused regression tests: Rust `face_colors_are_absent_from_wire_until_authored_and_survive_round_trip`; Vitest `treeActions` face-color wire payload + clear.
- [ ] Follow-up: real-worker integration smoke (the Rust command is present, but face-color rendering was verified on mock lane only because the real worker path mints its own element ids through `promoteSelection`); imported STEP per-face colors (`MESH1 FACE_COLORS`) already flow through the same baker but are not keyed by persistent id and will not survive rebind.

## KERNELBENCH KBR-0 + FILLET SLICE (2026-08-07) — GATE PASSED

- [x] Scope/provenance gate: current fillet hardening, dirty worktree, OCCT 8.0.1/7.9.3 CI lanes, worker/Cargo ownership, and frozen `corpus/` boundary inspected.
- [x] Case/result schemas, frozen T0 preset, and supported/expected-limit examples under `bench/robustness/`; Draft 2020-12 validation, negative unknown-field/seed probes, JSON parse, and diff checks pass.
- [x] C++ `benchmark_core` and isolated `onecad-kernelbench-runner`: raw/production adapters, strict request/result JSON, deep audit fixtures, semantic evidence, bounded artifacts, and failure-path SIGSEGV guard; focused CTest 7/7.
- [x] Rust `onecad-kernelbench` supervisor: disposable children, timeout/RSS limits, bounded I/O/artifacts, replay/differential/metamorph policy, non-monotonic search, report, strict cases; 17 unit + 4 supervisor tests and clippy pass.
- [x] CMake/Cargo workspace plus required OCCT 8.0.1 T0 and informational 7.9.3 characterization CI artifact wiring; YAML and diff checks pass.
- [x] **Finished interrupted selector/validation refactor and closed every outstanding review gap found by re-inspecting live files** (previous agents' handoff correctly flagged these as unverified):
  - C++ `Geometry.cpp` semantic-selector migration was broken (`select_indices()`/`selector.role` referenced removed fields). Replaced ordinal selection with deterministic nearest-anchor matching (unique match required within `1e-6`, corner mode resolves via nearest common-vertex + valence check); no persisted OCCT ordinals.
  - `Execution.cpp`/`SemanticValidation.cpp` had duplicate validator logic from the interrupted split; `Execution.cpp` now calls `SemanticValidation::validate_output()` and dropped to 270 lines (was exactly 500).
  - `exception_result()` hashed an empty string regardless of request; now hashes the request's semantic case+variant input when geometry generation failed.
  - `DeepAudit.cpp`: `BRepCheck_Analyzer` reports a topologically-empty solid (0 faces) as valid (verified empirically against OCCT 8.0.1) — `exactValid` now also requires `faces > 0`.
  - `SelectorParser.cpp` allowed 1–8 `adjacentSurfaceKinds` per surface descriptor; schema requires exactly 2 (every edge borders exactly 2 faces) — tightened to match.
  - `suite.rs` hardcoded exactly one `surfaceDescriptor` regardless of anchor count, which made every `disconnected`/`multiple`-mode box case (8 of 12 supported cases) fail at runtime with "edge selector descriptors do not match anchors" — only surfaced by actually executing T0, not by unit tests. Fixed to emit one descriptor per anchor; added the matching cross-check to `case_validation.rs`.
  - `suite.rs`'s `add_exploratory()` assigned `cornerIncident` mode to overflow wedges, which the schema forbids for `overflowEdge`. Overflow wedges now use `Single` mode against the wedge's actual tapered edge — anchor coordinates were verified against real `BRepPrimAPI_MakeWedge` output via a standalone OCCT probe, not derived from documentation alone.
  - `case_validation.rs`'s `validate_selector` didn't cross-check `selector.provenance` against `geometryRecipe`, didn't enforce the topologyRole↔recipe mapping, and allowed any `adjacentSurfaceKinds` length; all three now checked.
  - Rust hardening: `result_validation::validate()` now takes `&Case` and requires exact generator/expectedDomain equality (previously only checked `caseId`/`backend`); `persisted()`'s dead top-level `probeRadius` handling removed (radius lives at `search.probeRadius`); `result_validation_blocks::search()` was missing `probeRadius` entirely, so every real search-mode result failed strict validation — fixed and confirmed against a live `search-critical` run. `ValidatedState`'s previously-unused fields now drive a real execution/operation-state coherence check. `runner.rs` split at 519 lines into `runner.rs` (public API) + `child.rs` (process isolation), both under 500. macOS `RLIMIT_AS` is unreliable (OS reserves large VM before user code runs) — gated to non-macOS Unix only; macOS keeps RSS-polling enforcement. Supervisor synthetic artifacts (`case.json`/`request.json`/`stderr.txt`) now share one cumulative budget against the configured `artifactBytes` cap instead of each being independently 1 MiB-capped and only flipping verdict after the fact if the total ran over.
  - **Metamorph evidence was fabricated** (`surfaceSamplesMatch = semanticEvidenceMatch`, `pointClassificationMatch = normalizedPropertiesMatch`). Replaced with real evidence: the worker emits a `shapeSignature` (deterministic face-centroid samples tagged by surface kind, plus solid-classification of 7 centroid-anchored probe points, both computed post-fillet) and the shared pre-transform `rotationCenter`; `metamorph.rs` (new module) inverse-transforms the variant's points using the wire-reported translation/rotation and genuinely compares them (order-independent nearest-match for face samples, index-correspondence for probe points, since probe generation order is fixed) against the base variant within the case's declared `pointTolerance` (new `metamorphicEquivalence` validator entry, `required:false` since the runner itself always reports `notRun` for it — gating is campaign-level). Verified: rotating/translating a point forward then inverse-transforming it round-trips to <1e-9 in unit tests, and a real `--backend raw-occt` T0 run shows `surfaceSamplesMatch`/`pointClassificationMatch` genuinely `true` for all 12 supported-domain cases, `notRun` (not gated) for the 4 expected-limit cases where both variants correctly refuse.
- [x] **Full gates, all green**: worker build via `scripts/build-worker.sh Release` (OCCT 8.0.1, `homebrew-occt-8.0.1-20260807`), CTest **106/106**; `cargo fmt --all --check`, workspace `clippy -D warnings` clean; `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` full pass (worker-backed); `onecad-kernelbench` unit+integration **35/35**. T0 `--backend both`: **136/136 canonical records pass**, `executionState=completed` everywhere, replay stable **136/136**, differential **136/136 same-status** (zero rescues/regressions/mismatches), metamorph **48 pass / 0 fail / 16 notRun** (the 16 are the 4×2×2 expected-limit variants where both backends correctly refuse on all variants). `report` exits 0, `gatingFailures:0`. Timing (both backends, includes full deep-audit overhead — not comparable to production incremental-audit numbers): **p50 10.5 ms / p95 878.5 ms**. `search-critical` on the supported-single regression fixture: 57 probes, 1 transition found near **16 mm** (matches the historical baseline), `report` on the search output confirms the `probeRadius` fix. Every generated T0 case (68), both regression fixtures, the frozen preset, and all 320+ result records across the T0/search/raw-occt-only runs (332 documents total) validate under the committed Draft 2020-12 schemas. `git diff --check` and CI YAML parse clean.
- [ ] Deferred (not correctness-blocking, noted for follow-up): dedicated per-scenario CTest fixtures for supported/oversized/partial/valence/disconnected runner behavior (currently proven via the real T0 execution above, not isolated fixtures); boolean/later operations, history adapter, scale/mirror, minimizer/promotion, HTML/trends, STEP/reference ingestion, imported defects, Windows resource limits, advanced fillet modes (all explicitly out of scope per the original spec); no relative performance threshold yet (this p50/p95 is the first same-host baseline to compare against).

## FILLET-KERNEL-HARDENING (2026-08-07) — IN PROGRESS

- [x] K0 reproducible OCCT 8.0.1 baseline: pinned source cache, exact CMake resolution, derived RPATH/fingerprint, shared CI artifact, 7.9.3 informational characterization, required 7-to-8 persistence and packaging gates. Local exact-version/configure, selftest, relocated bundle, persistence, shell/YAML, and diff checks pass; pinned source archives verified, full source build remains CI-only.
- [x] D1 structured failure diagnostics: worker terminal/preview arrays, Rust transport/runtime ownership, frontend correlation, malformed-detail compatibility. Focused CTest 2/2, core 15/15, app diagnostic 3/3, Vitest 97/97, TypeScript, fmt, and parallel reviews pass.
- [x] F0 invalid oversized Fillet/Chamfer fallbacks removed; publication now rejects null, invalid, non-single-solid, or non-positive/non-finite results. 10 mm box at 11 mm refuses atomically; focused CTest 4/4 and byte-identical fresh-process NDJSON pass. OCCT 8.0.1 gates exact characterization; 7.9.3 gates safety only and records broader differences.
- [x] F1 constant Fillet core: fail-closed analyzer/builder/audit, semantic diagnostics, history-safe publication, robustness suites, and benchmark smoke. Focused CTest 7/7 plus fresh-process digest pass; parallel code/test reviews clean. Same-host OCCT 8.0.1 baseline p50/p95 ms: single 1.731/1.792, four-edge 3.466/3.665, boundary 1.748/1.833, refusal 0.935/0.994; all outputs valid. Stable OCCT partial-result fixture unavailable, so deterministic injected state-query coverage gates partial/BadShape refusal.
- [x] F2 frozen tangent intent: shared snapshot-atomic PrepareEdgeOp, fail-closed Rust batch promotion, full typed version-1 closures, contour dedup/drift refusal, legacy byte stability, type-swap retention, and exact-preview commit barrier. Chain-off, disconnected/tangent contours, post-edit rebind drift, identity/fallback collision, stale snapshot, partial promotion, and rollback gates pass; parallel code/test reviews clean.
- [x] Final automated gates: pinned worker build/selftest, CTest 104/104, Rust fmt/clippy/full worker-backed workspace, TypeScript, Vitest 207 files/3648 tests, production build, Fillet/Chamfer Playwright 13/13, bundled `@rpath` smoke, diff/scope audit.
- [ ] Manual Tauri gate: drag into an OCCT refusal; preview must stay last-valid, confirm blocked, diagnostic visible. Requires interactive user run.

## REPOSITORY-GUIDE (2026-08-07) — GATE PASSED

- [x] Reconciled root contributor guide with current project structure, commands, conventions, and Git history; Markdown, word count, and repository-specific command paths verified.

## MESH-FIDELITY (2026-08-06) — GATE PASSED

- [x] Worker: Fine uses clamped absolute+relative deflection with a 5-degree angular cap; coarse remains bounded for drag. Focused ctest `tessellation_quality` proves a cylinder Fine mesh is >=3x denser than coarse.
- [x] Worker: curved topology-edge polylines use the same adaptive chordal/angular policy (Fine exceeds the retired 16 spans); `tessellation_quality`, `wp5_mesh1`, and `wp6_meshexport` pass.
- [x] Runtime/viewer: `DISPLAY_LOD=Fine` drives regen artifacts, published keys, fetches, and explicit-save/open-paint caches; preview defaults coarse. Focused Rust 73+5+2, meshSync 31.
- [x] Mock/L1: one pure policy (5-degree cap + 0.02mm chord error) drives closed sketch curves, default cylinder meshes, and lathes; explicit mock segment overrides survive. Focused Vitest 67/67 + `npx tsc --noEmit` pass.
- [x] Gate: worker ctest 96/96; real-worker cache integration 7/7; targeted frontend suite 98/98; `bun run build`, `cargo fmt --all --check`, workspace clippy, and diff check pass.

Plan: `~/.claude/plans/act-as-senior-software-transient-popcorn.md` (approved 2026-07-16).
Tracks: W = C++ worker, R = Rust core, F = frontend. Gates in **bold**.

## STARTUP-STALL (2026-08-06) — the real ~15–30 s "frontend takes forever to load", root-caused and fixed
User report: app window opens, frontend needs up to 15 s. Bisected by config, not guessed: `GET /` alone (plain `curl`, no browser) blocked **31.1 s** on every fresh Vite process; dropping `@tailwindcss/vite` took the same request to **0.17 s**.
- [x] **ROOT CAUSE: Tailwind v4 automatic content detection walks the repo from the git root.** This tree is a terrible host for that walk — `src-tauri/target` is **58 GB** and `worker/build` another 162 MB. The scan is synchronous ahead of the first index.html response, so the webview sat on a blank/splash page for the whole walk. Nothing in the previous session's diagnosis (bundle size, mock-client eviction, store imports, worker prewarm) touched it.
- [x] **FIX**: `src/styles/globals.css` → `@import "tailwindcss" source(none);` + explicit `@source "../"` (src) and `@source "../../index.html"`. Measured: `GET /` **31.1 s → 0.013 s**; first paint **30.6 s → 20–56 ms**; start screen interactive **~275 ms**; cold `optimizeDeps` no longer matters (10 ms). **`vite build` 40.89 s → 1.65 s** — CI and `tauri build` were paying it too.
- [x] CSS output verified rule-by-rule against the pre-fix bundle: **0 added, 6 dropped**, all six phantoms the repo-wide walk had invented from prose/CSS strings — `.contents` ("Table of contents" in SCHEMA.md), `.bg-white`/`.text-white`/`.bg-black\/40` (TODO.md's own W1 de-hardcode notes, removed from `src/` in the theme WP), `.ease-in-out` (a CSS string inside `scripts/vite-splash-plugin.ts`), `.top-2`. None appears in `src/`, `index.html`, or `e2e/`. Net dead-CSS removal, 34,285 → 33,740 B.
- [x] Live verification in the real Tauri window (`bun run tauri dev`, WKWebView — not just Chromium): first FE IPC (`check_recovery`/`list_recents` from StartScreen mount) at **+2.17 s from process start**, `tMono` 1.56 s; **zero ERROR lines** in `logs/dev.jsonl`; sidecar prewarmed and heartbeating `GetWorkerHead` on the start screen.
- [x] **Reverted from the previous session as misdiagnosis-driven** (all four premised on the wrong cause, each a net negative): (a) `client.ts` lazy mock **Proxy** — `vite.config.ts` ALREADY evicts `mockClient` from prod via `evictMockClientInProd`, whose contract is that `createClient()` stays sync; the proxy was redundant and broke 4 vitest specs (identity + spy-on-method). (b) `localSolver.ts` dynamic mock imports — bought ~35 KB and made `updatePreview` **async on the drag lane**. (c) Rust `prewarm_before_open` — setup's `prewarm()` already ran on `tauri::async_runtime::spawn` and never blocked the webview; the replacement spawned the sidecar and called `make_backend()` microseconds later, so the first open paid full cold start instead of using the start-screen dwell. (d) `BootingScreen` + `html.app-booted` splash handoff — held a splash over the start-screen shell+skeleton for longer, no flash was being prevented. `main.tsx`'s `setTimeout(exposeStores, 0)` also reverted: top-level `import()` cannot block React's synchronous first render, so it bought nothing and delayed `window.__stores` by a macrotask.
- [x] **KEPT + finished**: start-screen boot-failure UX. `recentsStatus`/`recoveryStatus` gain `"error"`, so a failed `listRecents`/`checkRecovery` no longer leaves the skeleton spinning forever; StartScreen renders "Could not load the start screen." + per-probe Retry. **Changed from the draft**: both now `logError` and **do NOT re-throw** — `loadRecents` is called as `void get().loadRecents()` from `openProject`/`openDialogAndOpen`/`closeProject`, so re-throwing made every failure an unhandled rejection; this follows the file's own `importStep` precedent ("A failure must not reject into the caller's `void importStep()`"). `vite.config.ts` warmup keeps EditorScreen+ViewportRoot (warmup is background, never blocks a request) and adds StartScreen.
- [x] **GATE (2026-08-06)**: tsc clean · vitest **204 files / 3625** green (+5 new: 3 appStore error-capture, 2 StartScreen retry-affordance) · all 5 **negative-checked** (restoring `throw e` fails the 3; un-suppressing the skeleton fails the retry spec; the first draft of that spec had a **vacuous** `queryByLabelText("Loading recent projects")` assertion — no such label exists — replaced with a real `.animate-pulse` absence check) · prod build clean.
- [x] e2e **182/187**. Not regressions, proven not assumed: the 4× `transform-body` failures reproduce on **clean HEAD** (26 passed / 4 failed with the whole diff stashed) — exactly the debt STARTUP-PERF's FINAL GATE already records against the OFFSET-FACE value-handle refactor. `filletChamfer` is **flaky**: run 1 = 13/13, run 2 failed a *different* case (`:231`, not the full run's `:193`).
- [ ] Follow-up: `@source "../"` must be widened if class-bearing sources ever land outside `src/` + `index.html`. Worth a grep gate alongside the existing hex gate.

## VIEWPORT-VISUAL (approved 2026-08-06, plan `~/.claude/plans/act-as-senior-3d-hazy-star.md`) — Shapr3D-style rendering + hover/selection feedback
Fable-orchestrated (3 explore + 1 plan-map + 4 impl agents). User-confirmed color language: sketch blue(under)/green(done)/orange(selected) · cyan hover everywhere · blue select (bodies/faces/edges) · body edges near-black BOTH themes (mode-conditional: wireframe keeps inverting via new `--color-body-edge-wire`).
- [x] P1 tokens+palette: 5 new tokens BOTH blocks (`--color-sketch-under/done/sel`, `--color-viewport-hover`, `--color-body-edge-wire`) + `hover3d()`/`bodyEdgeWire()` roles + themeRefresh contract rewrite ("wireframe edges go LIGHT in dark"; body edge near-black both themes).
- [x] P2 sketch: widths now CSS px × dpr (`cssLineWidth`, cap = engine MAX_DPR 2 — root cause of "thin": 2 device px = 1 CSS px on Retina). Edit sketch 2.5/selected 3/preview 2; static layer gets fat `LineSegments2` DRAW pass @2px + invisible plain-LineSegments PICK PROXY (explicit-list raycast ⇒ zero GPU cost); static base = blue always; region selected fill orange @0.45.
- [x] P3 body edges → `LineSegments2` @1.5px, `edge`(near-black)/`edgeWire`(inverting) pair selected by new `RenderModeDef.edgeStyle`; Picker raycasts fat lines NATIVELY: `faceIndex`=segment ordinal (no `>>1`), anchor=`pointOnLine`, `line2PickThreshold` cancels drawn width (pick stays 6 CSS px), resolution self-flush pre-raycast (kills raycast-before-first-render silent miss). HighlightLayer: cyan hover 0.45 / select 0.55 / whole-body 0.45 (new `selBodyMat`), fat 3px highlight edges via per-highlight `LineSegmentsGeometry` subarray slices — never-dispose contract amended for edge overlays only (`isLineSegmentsGeometry` discriminator). README §Theming/§Picking rewritten. WebGPU lane now lacks body edges (jsm lines WebGL-only, triad precedent).
- [x] P4 interaction: (a) hover blackout closed — `setToolHover`/`clearToolHover` (ONE-HOVER-WRITER) in hole facePick+armed (flat-face-gated), boolean pickTool, extrude facePick/targetPick, revolve targetPick; (b) regen rebind — `rebindPick.ts`: anchor-position + PREVIOUS-entry direction evidence (dot≥0.9, tol 1e-3×bbox-diag), REFUSES without evidence (house rule: no silent wrong bind); `promotePick` writes back topoKey (id stays identity); `ordinalForRef` elementId fallback (inert until IDS_HAVE_ELEMENTIDS); (c) orbit gates on all 7 modal pick phases incl. a found pre-existing leak (`cancelPreview` never released extrude facePick suppression). Exit-path audit table in agent report.
- [x] P5 chips: pill restyle (`rounded-full`+`shadow-popover`, segmented groups round the CONTAINER) across LiveDimField/ModelToolChips/HoleChipCluster; `overflow-hidden` removed from ViewportRoot container (children all inset-0) — edge-anchored chips no longer clipped.
- [x] **GATE (2026-08-06)**: vitest 204 files/3613 green · tsc clean for this WP (remaining errors = OFFSET-FACE session's new test files) · hex gate 0 · e2e 171/187 on isolated port; all 16 fails = OFFSET-FACE in-flight lane (4× offset-face.spec = their open P2; 4× transform-body probe-attributed: rebind-off AND align-suppression-off probes still fail, value-handle refactor overlap) · our-scope specs re-proven isolated 36/36 (filletChamfer edge-pick gate, measure, theme incl. wireframe flip, hole, boolean-preview). SwiftShader e2e needs quiet machine + OWN `E2E_PORT` when two sessions run — shared test-server caused a full round of phantom goto-timeouts.
- [ ] Manual gate (USER, `bun run tauri dev`): sketch blue/green/orange states + thicker lines; body edges black both themes; cyan hover on faces/edges/sketches; hover feedback while hole/boolean/extrude-target armed; drag during pick no longer orbits; selection highlight survives a parametric edit; chips pill-look + unclipped at viewport edge; Retina line weight OK (dial `cssLineWidth` constants if heavy).
- [ ] Follow-ups: dpr change mid-session (monitor move) keeps construction-time widths; `SketchObject`/`ViewportEngine` MAX_DPR kept in step by hand; OFFSET-FACE's facePick phase wants the same `setToolHover` treatment (helper ready).

## OFFSET-FACE (approved 2026-08-06, plan `~/.claude/plans/act-as-senior-rust-fizzy-goose.md`) — Shapr3D-style direct-modeling face offset (planar+cylindrical V1, Offset/Total/Radius/Diameter, tangent chain, drag+live preview)
Codex gates: brainstorm (sol/xhigh, live OCCT probes) + plan review (terra/high): revise → 3 blockers folded pre-approval (PrepareOffsetFace authoring handshake; core `InputPath` repair write-paths; mandatory targetBodyId). SCHEMA §7.3 `op.offsetFace` + §7.6 `PrepareOffsetFace` + §14 (orchestrator-signed 2026-08-06).
- [x] **P0 spike GATE (2026-08-06)**: `worker/tests/test_offsetface_spike.cpp` (ctest `offsetface_spike`) — GO. Characterized: kernel AUTO-PROPAGATES offsets across G1-tangent junctions (⇒ chain-off + tangent-adjacent unselected = authoring refusal, not honorable); face lineage lives in `OffsetFacesFromShapes()` image, public `Generated/Modified` face-empty (⇒ image-based history adapter, Codex's Generated claim corrected); frustum taper preserved (716.16 exact vs prism 725.33); hole σ trap (d=+1 SHRINKS r5→r4); r+σd≤0 → "valid" inside-out cylinder; collapse → zero-volume passes IsDone.
- [x] **P1 backend GATE (2026-08-06)**: full chain live — `OffsetFaceParams` (dual faceIds+typed refs, HoleParams both-ways conditional validation, never-clamp) · `InputPath::OffsetFaceFace{index}`/`OffsetFaceOpposite` + `set_input` full-ElementRef rebind · add+update validation · wire lowering (slot order normative: faces then opposite LAST) · `PrepareOffsetFace` verb (snapshot fence MANDATORY — lead-review fix: absent snapshotId was silently unfenced, now PROTOCOL_ERROR) · worker `OffsetFaceOp` (σ=sign(n_out·r̂) geometric, coaxial-set gate, Total anti-parallel+positive-t, semantic postconditions: exact plane shift + coaxial predicted radius, single-solid, min-volume 1e-9) · `OffsetImageHistory` adapter (conservative IsDeleted: positive evidence only) · 4th kernel fact found in impl: compound-wrapped body input → bare SHELL output, executor normalizes to single solid first. Suites: ctest 95/95 (offsetface 16-test suite + spike + harness fixture) · cargo workspace 66/66 suites w/ real worker · fmt+clippy clean · `offset_face.rs` gate 6/6 (exact volumes 12000/π·144·25/14000/180 via QueryMassProperties; destructive edit → deterministic NeedsRepair → repaired via `InputPath::OffsetFaceFace{0}` → green; prepare handshake+fence incl. STALE_PREVIEW; cross-process signature `5cf677944dbbe982` identical). One transient full-run failure re-proven foreign (parallel-session mid-edit; 8-test candidates 8/8+8/8 isolated, rerun 66/66).
- [ ] LADDER-ENVELOPE finding (gate-test agent, recorded for policy review): translating planar face survives Δdepth 25→30 but scores 0.847/0.097 at 25→40 — just under the 0.85/0.10 auto-bind gate (anchor is the only distance-sensitive term). Deterministic NeedsRepair = designed direction, but push-pull re-edits surviving bigger upstream moves is a LADDER-policy question, not OffsetFace's.
- [x] **P2 frontend GATE (2026-08-06)**: full lane live — fail-closed authoring transaction (`armOffsetFaceFromSelection` → `prepareOffsetFace` (fenced; refusal = arm hint, never arms) → promote EVERY closure face via AcquireElementIds → freeze typed refs → arm) · `commitPreviewedOp(…, {requireExactPreview})` + `ExactPreviewOutcome.timedOut` (NEW opt-in strictness — "barrier timed out" now distinguishable from "kernel approved"; existing callers byte-unchanged) · Offset-only drag (grab-delta axisDepthFromRay, degraded screen-space fallback), absolute types numeric + seeded from `currentDims` · L1 ghost = drawRange face clones via GhostLayer `range` (hidden when exact candidate lands) · shared DragHandle `showValueHandle` · chip DistanceTypeSegments (single-face-gated, planarity-filtered) + TangentToggle (disabled-not-hidden on Total; toggle re-runs prepare) · no-clamp (chip error state) · toolbar `pushpull` ⇧O + inspector rows (`FeatureKind::Fillet` bucket, opType-routed) · mock prepare (identity closure, crossBody refusal). REAL BUG found by tests: multi-await arm published after `dispose()` — controller now `invalidateArm()`s on dispose. Suites (lead-verified): build clean · vitest 3619/3619 · `e2e/offset-face.spec.ts` 8/8 (2 contention flakes re-proven 2/2 isolated at 2-4s — same SwiftShader-load finding as ICONS/STARTUP-PERF gates) · hex 0. FULL e2e sweep deferred to a quiet machine (per standing finding); foreign lanes untouched (`localSolver.ts` diff is STARTUP-PERF's).
- [ ] Manual gate (USER, `bun run tauri dev`): box top-face drag w/ live preview; hole Ø resize via Diameter; tangent chain on split cylinder (chain-off must refuse); destructive upstream edit → repair panel flow.

## STARTUP-PERF (approved 2026-08-05, plan `~/.claude/plans/act-as-senior-rust-misty-oasis.md`) — fast app open, fast project open, honest loading UX
Codex plan review (terra/high): revise → B1/B2/H1/M1/M2/M3 all folded pre-approval (see plan §Independent review record).
FE lane (FE-1→2→3 strict; 0/4/5 independent; 6 last) · RS lane (RS-1, RS-2→RS-3; RS-4 parallel). Commits at gate boundaries.
- [x] FE-0 instant paint: token-derived splash via transformIndexHtml (scripts/vite-splash-plugin.ts throws on token rename) + index.html #splash (pointer-events:none) + App useLayoutEffect removal
- [x] FE-1 entry→three cut + prod alias evicts mockClient (mockRegions extraction verbatim; localSolver dynamic import; evictMockClientInProd resolveId→mockClientStub). **GATE PASS: zero modulepreload links · setMockLatency absent from ALL dist/assets · entry 1,435,296 → 356,285 B**
- [x] FE-2 React.lazy EditorScreen+DevGallery + BootPanel + idle prefetch (StartScreen.test findByRole timeout 1000→5000ms ×2 = planned edit, not flake)
- [x] FE-3 devDemos.ts split (import.meta.env.DEV-gated dynamic import; demo flags dead in prod builds = recorded behavior change; stub's 5 extra exports removed; 34/34 demo-flag specs green)
- [x] FE-4 RecentGridSkeleton + aria-busy (no title attr/name text — StartScreen.test queries safe)
- [x] FE-5 geometryPending chip (count-based, visible-scoped predicate; pointer-events-none; defers to error statusHint) — meshSync retry deliberately UNCHANGED (pinned by meshSync.test "publish with NO further event")
- [x] FE-6 optimizeDeps include three+jsm-lines+RoomEnvironment / exclude three/webgpu + server.warmup boot chain. Chunk table (prod): entry 356 KB · EditorScreen 472 KB · three.module 563 KB (lazy) · three.webgpu 586 KB (flag-gated) · mockRegions 1.1 KB (never fetched under Tauri)
- [x] **FE GATE (2026-08-05)**: vitest 3420/3420 · build clean · grep gates pass · hex clean · e2e 175/179 under heavy concurrent cargo load, all 4 failures re-proven as load flake — point + sketch-drag passed isolated retry; filletChamfer:193 + sketch-degenerate:35 passed 3×/3× in 4-5s each on quiet machine (vs 45s timeouts under contention). SwiftShader e2e is CPU-bound: full-suite runs need a quiet machine, same finding as the ICONS gate's discarded HMR run.
- [x] RS-1 worker retire + pre-warm (875→878 tests). Cancellation observed in FOUR states (agent found the 4th: ping-await inside ticker arm parked the loop past the retire signal). WorkerState::Retired sticky, maps to "failed" on wire. warm/live slots; prewarm at setup + rewarm at close_document; retire_all at confirm_exit. **DESIGN AMENDMENT (agent-flagged, accepted)**: two-phase swap — make_backend stages the displaced backend (worker + ALL six side seams snapshotted), commit_backend retires it only after the runtime publishes, rollback_backend restores everything on a failed open/import (also fixes a PRE-EXISTING half-break where side seams were swapped on failed opens). Red-first pin: `a_failed_open_leaves_the_previous_documents_worker_alive` (real OCW1 ping through document A's own engine Arc). Manual pgrep gate still open (user).
- [x] RS-2 plan-artifact mesh ingest (878→895). response_with_bin → parse_plan_artifact_meshes (never fails plan; sha+size+MESH1 verify per handle; snapshotId cross-check vs preparedSnapshotId; 256 MiB cap) → MeshSink seeded at publisher.publish → drained in finish_regen INSIDE the commit fencing guard. MeshCache: peek + 256 MiB byte budget + cap 512 (MRU never evicted for byte bound). SCHEMA §7.2 descriptive artifacts block + §14 (orchestrator-signed; §5.2 chunking tension documented as open seam). Red-first proven: no-op'd seed → open_render fails "tessellated twice". Retransmit path logs artifact_meshes=0.
- [x] RS-3 persisted meshes+thumbnails (Rust +21 tests; FE vitest 3442→3505). Rust: container reader APIs (mesh_cache_path/parse pair %25+%3A-escaped — agent finding: core BodyId is a UUID newtype, `body_x:k` is wire-form only, so the colon escaping is defensive not required; mesh_cache_entries; read_caches single-open; standalone read_preview 2 MiB cap, sha-verified, Option not Result, deliberately ignores opsHash staleness — a preview is a picture of the last save) · SaveCaches explicit-only (autosave writes NO caches; preview carried forward on capture-less saves; ContainerCaches::MeshCache.bytes → Arc so peek stays copy-free under the lock, VF-B7 discipline) · cached_meshes served ONLY while latest_snapshot none + generation None|0 (0 unmintable — publisher starts at 1), cleared at first commit; stale test tampers manifest.opsHash not ops.jsonl (derived projection — tampering it proves nothing, agent-corrected) · geometry_source none|cached|live · save_document(preview_png) base64 512 KiB cap never fails save · recents data-URI thumbnails via one spawn_blocking. open_render assertion #2 INVERTED (module doc-comment updated — it is that file's spec). FE: ViewportEngine.captureThumbnail (sync renderFrame + preserveDrawingBuffer read; WebGPU→null — async render can't guarantee swap-chain contents at drawImage time); capture BEFORE the Save-As dialog (native modal can blank webview); seam extended on saveDocument AND saveDocumentAs (As = the first save of a new doc, the case that matters most); "Last saved geometry" chip top-center (persists whole sessions — dead-center would obscure the model), coexists with error hint by design, pending chip wins; mock RECENTS first-2 entries get data-URI thumbnails. base64 crate = one Cargo.lock line (already in dep graph).
- [x] RS-4 open-path hygiene (895→898). `open_runtime_over_new_backend_blocking` (spawn_blocking wrap; rollback on Err AND JoinError; sync sibling kept — it is prewarm.rs's test seam) · async stale-workspace sweep off window-creation path · WorkerReadiness as 10th facet (full snapshot/rollback symmetry, side-seam restore test extended) + `schedule_initial_regen` readiness-gated ToEnd{from:0} (8s timeout, fires regardless) in open/import/recover — closes the cold-worker no-retry race. Harness bug found: scheduler channel closed by dropped SharedScheduler ref in spawned task → clone like production.
- [x] **FINAL GATE (2026-08-06)**: cargo 944/0 w/ REQUIRE_WORKER · fmt+clippy clean · ctest 95/95 · vitest 3505/3505 (201 files) · hex clean · prod chunk table: entry 356 KB (was 1,435 KB) / EditorScreen 472 KB / three.module 563 KB lazy / three.webgpu 586 KB flag-gated / mockRegions 1.1 KB / zero modulepreload / setMockLatency absent from all assets. **e2e rerun on quiescent tree (12.7 m): 181/187.** Solo re-proof: point + sketch-degenerate GREEN isolated (6.4 s vs 45 s timeouts — contention class, per standing finding). **4× transform-body (fold :186, multi-body :296, Z-quad :410, Align :557) fail DETERMINISTICALLY isolated too — real regression vs HEAD's 179/179, attributed to the OFFSET-FACE value-handle/DragHandle refactor overlap (their gate's own flag); owed by that lane as a follow-up.** Committed at user's order after their manual native test passed, with this debt recorded rather than hidden.
- [ ] MANUAL GATES (user): (1) pgrep onecad-worker ≤2 across open→close→open cycles, 0 after quit; (2) save → quit → relaunch → recents card shows real thumbnail → open → last-saved geometry paints with "Last saved geometry" chip BEFORE regen completes → chip flips to live; (3) packaged-Tauri smoke: real client constructs with mockClientStub aliased in; (4) cold `tauri dev` + packaged app: themed splash paints before any content, no light flash under dark preference; (5) record save-size delta on a real multi-body doc (64 MiB mesh budget, 512 KiB preview cap).

## SAVE/OPEN-CONNECT-RACE residuals (2026-08-06, plan `~/.claude/plans/act-as-senior-rust-synthetic-heron.md`) — hardening riders on STARTUP-PERF's RS-4 fix
Root cause of the user-reported "Geometry rebuild failed — worker crashed: worker not connected" on reopen was the cold-worker dispatch race; RS-4's `schedule_initial_regen` + RS-1 prewarm closed it (different design than the plan's stream_plan gate — Codex sol/xhigh review of that gate found a pre-dispatch-cancellation blocker the RS-4 gate-before-enqueue shape doesn't have; plan's core superseded, only deltas shipped).
- [x] D1 `EngineError::NotConnected {message}` (no serde/golden pins) — the three absence-of-connection sites (`stream_plan` conn-None, `not_connected()`, `retired_err()`) stop claiming "crashed"; mid-call deaths (`protocol_err()`, "crashed mid-plan", poison terminal) stay `Crashed`. `From<EngineError>` arms + `worker_chaos` convergence drill widened (dispatch landing in a scripted-kill restart window now honestly reports absence, same no-partial-publish guarantee). Unit pin: kind `"worker"`, message never contains "crashed".
- [x] D2 `sketchStaticSync` region-fetch retry — rejected `getSketchRegions` no longer strands a sketch curves-only forever: failed ids queue on `documentStore.revision` (backend publish stamp, both lanes; deliberately NOT worker-ready — it precedes the replay), edge-triggered once per publish, in-flight attempt clears the queue entry so a retry racing a token reload single-fetches. 3 vitest pins (retry lands / no spin / once-per-publish).
- [x] D3 readiness-gate coverage (`spawn_gated_regen` previously had ZERO tests): unit `withholds_the_request_until_readiness_resolves` added beside the concurrent session's fires-promptly/fires-at-timeout pins; stub `ONECAD_STUB_HELLO_DELAY_MS` (+ `env_millis`, unparsable ⇒ no delay); `tests/worker_connect_race.rs` regression pins (NOT red-first — fix predates them, doc-commented): gated regen publishes over a 300 ms-delayed hello AND ungated immediate dispatch still fails fast as `NotConnected` (manager fail-fast is intentional; the API-layer gate is the load-bearing fix). Trap found: `cargo test --test worker_connect_race` alone doesn't rebuild the stub — stale-stub panic names `cargo build -p onecad-worker-stub`.
- Verified (orchestrator-run, targeted — full suites deferred to STARTUP-PERF FINAL GATE on quiescent tree): connect_race 2/2 · lifetime 5/5 · chaos 14/14 · prewarm 4/4 (real worker, REQUIRE_WORKER=1) · readiness_gate 4/4 · error unit · onecad-core 411 · fmt/clippy/check clean · sketchStaticSync vitest 15/15.
- [x] D4 rider (2026-08-06, after the editing session quiesced): `tauriClient.ts` sticky "Geometry rebuild failed —" hint now clears ONLY on a clean `published` (the failure branch already captures publishes carrying failedSteps, so the else-branch check is publish-and-clean by construction) with the owned `REGEN_FAILED_HINT_PREFIX` — `superseded`/`cancelled`/`noop` retain it, unrelated sticky hints untouched (exitIsolate discipline). 3 vitest pins (clear-on-publish / retain-matrix — non-vacuous: dropping the outcome guard turns it red / unrelated-hint). tauriClient.test.ts 93/93 · tsc clean.
- [x] Codex implementation review (terra/high, custom-prompt scoped to D1-D4 in the shared dirty tree): no blocker/high; two P2 concurrency findings, both verified real and fixed as D5/D6. (Finding 1 mis-attributed to D3 — it lives in RS-4's `spawn_gated_regen`, pre-existing in-tree; fixed here since that session had quiesced.)
- [x] D5 stale-gate guard: a `spawn_gated_regen` task can outlive its document (close/open during the 8 s wait retires the watched worker and installs a NEW runtime whose own gate is pending) — the global scheduler carries no document identity, so the stale gate's from-0 request would land on the NEW document, racing its handshake (the exact bug class this gate closes) or superseding its active regen. Fix: `still_current` probe param (`FnOnce() -> Future<bool>`); `schedule_initial_regen` (now async, 3 call sites `.await`) captures `document_uuid` at schedule time and fires only while that document is still the open one; runtime-None ⇒ skip. New unit pin `skips_when_the_document_was_replaced_while_waiting` (+ 3 existing calls updated `|| async { true }`). readiness_gate 5/5.
- [x] D6 missed-revision grant: a publish landing WHILE `getSketchRegions` is in flight ran `retryFailedRegions` against a still-empty set — the rejection then queued the id for a next publish that may never come (permanently curves-only, the exact race D2 exists to repair). Fix: capture `revision` at attempt start; on rejection with revision advanced, `queueMicrotask` one immediate retry (bounded: the retry captures the NEW revision — a second rejection with no further publish just re-queues). New pin `grants a publish that landed DURING the failing fetch`. sketchStaticSync 16/16.
- Verified after D5/D6: connect_race 2/2 · prewarm 4/4 · open_render 2/2 · lifetime 5/5 · readiness_gate 5/5 · clippy/fmt clean · sketchStaticSync+tauriClient vitest 109/109 · tsc clean.
- [ ] Follow-ups (supervision, out of scope here): `spawn_and_connect` has no timeout on the hello read (a spawned-but-mute worker stays `Starting` forever); `auto_open_session` result is discarded before `Ready` publishes (failed OpenSession still reports Ready); `PendingBackend::not_ready()` still labels the no-worker build `Protocol` — `NotConnected` fits better (has test dependents).

## ICONS-TWOTONE (approved 2026-08-05, plan `~/.claude/plans/do-thorough-analysis-of-snazzy-pixel.md`) — port the OneCAD-CPP two-tone CAD icon family
- [x] **GATE (2026-08-05)**: the 88 `OneCAD-CPP/resources/icons/*.svg` masters are now the app's icon language, replacing the single-path monochrome prototype glyphs. `scripts/gen-icons.mjs` (Node built-ins, no dep) compiles them to `src/icons/cppIcons.generated.ts` — committed, so no build/CI ever needs the CPP checkout; flattens the 2 `<g>` wrappers (`orbit`/`pan`), folds `<svg>`-root paint inheritance (`ic_overflow` sets `fill`/`stroke` on the root and has no `stroke-width` at all), maps the accent sentinel → `tone:"accent"` and `fill=<color> stroke="none"` → `filled:true`. Asserts on every unmodelled element/attribute/color rather than dropping it silently; deterministic (rounded ratios, sorted keys) — reruns are byte-identical. **`sw` is a ratio against the FAMILY base 2 (DESIGN.md §2), not each icon's own root**: 17 masters (solid-body group, boolean ops, view cubes) are authored at 1.75 on purpose because they carry more detail, and per-icon normalisation would have flattened all of them back to full weight — against the shared base, the caller's `strokeWidth` still scales everything AND the relative weights survive. `Icon.tsx` maps over an element array; accent emits `var(--color-icon-accent)` (new token, BOTH theme blocks, light `#2d7ff9` = CPP intent / dark `#5aa2ff` lifted for `--color-panel`) — a `var()` rather than a resolved color specifically so an accent-painted container can collapse the glyph to mono, which `ToolButton` active + `SketchChromeBar` penEdit now do via `[--color-icon-accent:currentColor]` (DESIGN.md §3 requires every icon to survive that). Call sites unchanged: `paths.ts` composes generated + authored + a 12-entry ALIAS map (`rect`→`rectangle`, `x`→`close`, `eye`→`eyeOn`, …) rather than churning 18 files; `as const satisfies` had to be split from the published type (literal narrowing removes the optional fields, so the renderer could not read `tone` at all). **Constraints: 18 drawn symbols replace the Unicode text glyphs** on the 4 DOM surfaces (`SketchConstraintToolbar`, `ConstraintContextChips`, `ConstraintList`, `InspectorPanel`); `CONSTRAINT_PRESENTATION` keeps `glyph` because `ConstraintBadgeLayer`/`badgeLayout.ts` place canvas badges from MEASURED TEXT WIDTH — converting those needs fixed-size boxes + collision re-verification, deliberately out of scope. `EyeToggle` gets the real `eyeOff` (state was opacity-only, so hidden and merely-dimmed looked identical). 15 keys have no CPP master: 9 chrome glyphs stay mono BY POLICY (a chevron has no operation verb; CPP draws its own `close`/`menu`/`overflow` flat), 7 CAD-tool glyphs + the concurrent session's `arc3p` were authored two-tone against DESIGN.md. **4 of those 8 were redrawn after looking at them**: `centerRect`'s half-diagonals read as a crossed-out rect (now centre+corner click dots, parallel to `rectangle`'s two corner dots); `polygon`'s circumscribed circle read as a dotted halo at the hexagon's own diameter (now centre + circumradius); `measure`'s accent lay exactly on top of the primary outline so NO second tone was visible (now the graduation ticks); `mirrorBody`'s dashed ghost turned to noise and its dimetric top face closed the gap to the axis (now both bodies solid like `mirror`, sized to keep ~3.5u of air). `arc3p`'s through-point was 1.7u OFF its own curve — it is the arc's apex, not the chord midpoint. Verification beyond the suites: `?gallery` renders 117/117 with 0 empty and 0 console errors in both themes, and a computed-style probe of the live app proves the token resolves — every idle toolbar icon reports exactly 2 distinct colors (`ink-4` + `#2d7ff9`), every active one exactly 1 (`#2e6fe0`), across all 3 toolbars (14 model / 17 sketch / 16 constraint). The mono rule is one exported `ICON_MONO` constant rather than inline strings at 7 sites, because it kept needing to be applied somewhere new; its `FloatingToolbar` pin was NEGATIVE-CHECKED (dropping the override from `ToolButton` turns it red). Suites: tsc clean · build clean · vitest 3408/3408 (200 files) · **e2e 179/179 (10.5m, first full-suite run on a stable tree — an earlier run was discarded as invalid because HMR reloaded the app mid-run while these files were still being edited)** · hex gate 0 · generator re-run byte-identical. `Icon.test.tsx` rewritten for the multi-element contract (+ accent/filled/ratio/dash/alias cases); `HistoryList.test.tsx`'s 6 exact-`d` assertions now compare ordered element lists. No e2e spec needed changing — they select by `aria-label`, never by glyph.
- [ ] Follow-ups: `ConstraintBadgeLayer` canvas badges still text glyphs (needs fixed-box layout); ~50 imported icons have no UI consuming them yet (view presets, undo/redo/save/delete, orbit/pan/zoom, loft/sweep/draft/pushpull) — available, unused by design; legibility floor at the 11px sites (`FileMenu` chevron, `SketchChromeBar`) is chrome-only today, but any CAD glyph dropped to that size needs re-checking.
- [ ] Manual gate (USER, `bun run tauri dev`): toolbar idle icons show the blue accent, ACTIVE tool collapses to one flat accent color (not two competing blues); sketch mode → constraint pill shows drawn symbols and they stay readable when enabled; tree row eye toggles between open and struck-through; dark theme — accents readable on panels AND on an active tool's `sel-bg`; history rows show per-op icons.

## SKETCH-PRO (approved 2026-08-04, plan `~/.claude/plans/act-as-senior-software-virtual-axolotl.md`) — sketch audit → hardening + Shapr3D live dims
Full audit (3-agent exploration): strengths + gaps table in plan. SP-2..6 queued (direct manipulation w/ SCHEMA §7.4 handle kinds · IntersectionManager port · 3-pt/tangent arc + Extend · polar tracking + context menu · Project/Convert + sketch patterns).
- [x] **SP-0 FE GATE (2026-08-04)**: doc debt (CLAUDE.md:91 gesture-verbs claim was FALSE — verbs live since R-WP12; worker Constraints.h stale TODOs for shipped constraints); D5 dead snap toggles REMOVED from popover (`guidePoints3d`/`distantEdges` fields RESERVED in store, no version bump, exactly-live-rows pin test); D6 Concentric/Equal auto-inference ported from legacy AutoConstrainer (tol 2.0/0.5mm strict-<, nearest-wins, legacy order, DIVERGENCE: same-batch Center-Coincident suppresses Concentric — exact-snap centers would false-OverConstrain the mock count; ellipse excluded; 19 new tests); D4 mock-lane conflict parity — NEW `mockConflicts.ts` rules R1 dup-incompatible-dimension / R2 H+V / R3 Par+Perp / R4 Fixed-contradiction, `solveSketch` wrapper (solveDof untouched), worker-precedence status, NEW e2e `dimension-conflict.spec.ts` = first automated coverage of the named-conflict reject UX (was manual-gate-only). NOT DONE from SP-0: D1a VF-M4 cache = SUPERSEDED by HISTORY-HARDEN H4 (deeper fix); D3 region-vs-drag RAII claim + D1b host-face re-entry query + D2 revolve verify-first = deferred, must coordinate with HISTORY-HARDEN session (same files).
- [x] **SP-2/4/5 GATE (2026-08-05, commit ec7780c)**: pro-CAD tools + direct manipulation. **SP-4**: `arc3p` (⇧A) on new pure `arcMath` (circumcenter + CCW swap rule — the through-point decides which of the two arcs is meant); tangent-continuation arc as a MODE of lineTool (drag or `A` mid-chain; `ToolState.chainTangent` RECORDS each commit's exit tangent because an arc's exit ≠ its chord, so arcs chain tangentially); Extend (⇧T) — REPLACES the entity with a fresh id (`marshalUpsert` diffs by id, so a moved endpoint emits no wire op at all) + per-kind constraint cascade (drop moved-endpoint welds/line dimensionals, keep re-minted arc Radius/orientation/far-end). **SP-5**: polar tracking (0/45/90/135 + parallel/perpendicular to the previous segment) as a tier that outranks grid but never a geometry snap, ties going to H/V; configurable snap radius S/M/L — `SNAP_PX` is now read by NOTHING outside snapEngine, finally making its own "one constant governs every point-like pick" docstring true (5 sites, incl. one Extend added the same day). **SP-2**: four gesture kinds (point|arcEnd|radius|entityBody) — whole-entity translate, arc-endpoint reshape, ring resize. SCHEMA §7.4 ADDITIVE (absent `kind` ≡ point ⇒ every existing request byte-identical, json_subset matchers ⇒ **NO fixture bump**; §14 entry + cross-track sign-off recorded here). New `curves` response channel is MANDATORY, not convenience: Rust's `Arc` is {center,radius,startAngle,endAngle} with NO endpoint entities, so `positions` structurally cannot carry a reshape. Vendored PlaneGCS has no initMove/movePoint — dragging IS tag(−1) temporary driving constraints; per-kind pin sets (arcEnd frees the sibling endpoint, pins the centre; radius pins the centre; entityBody pins nothing). **3 latent defects closed**: (a) endpoint weld + entity Tangent is PlaneGCS-redundant ⇒ false OverConstrained — inferred Tangent now suppressed when the batch welds (a snapped center-start-end arc already hit this); (b) a Tangent-propagated radius reached the worker store but never the Rust document ⇒ next upsert silently reverted it (red-first proven, closed by `curves`); (c) undo/redo ran against an OPEN drag gesture and was silently un-done by the pointer-up commit ⇒ reverts now stand down. **D3**: region-query-vs-drag TOCTOU closed by an RAII per-sketch solver-lane claim held across the unlocked drive (`document_runtime/solver_lane.rs`); the old comment misdiagnosed the damage — it is EndGesture's store write-back onto the query's stale clone, with `SketchStore::put` assigning the revision exactly so it can REGRESS. DEFERRED+flagged: worker `SketchStore::put` monotonicity (SCHEMA §7.4 response-field semantics — needs §14 + sign-off); gesture verbs bypass the H2 circuit breaker; typed-radius-mid-drag. Suites: vitest 3415/3415 · ctest 92/92 · cargo 860/0 vs real worker · tsc/hex clean.
- [ ] **SP remaining**: in-canvas context menu (design in plan §SP-5.6) · SP-4/SP-5/SP-2 e2e specs (helpers already landed in `e2e/helpers.ts`) · D1b `hosted_sketches_on_face` (face re-entry survives process restart; plan §B-D1b) · D2 revolve residual pins (re-scoped: the "re-edit regions[0] fallback" premise is DEAD, fixed pre-e2af9a0 in 49089bc — pin the legitimate fresh-arm fallback + the finishSketch commit boundary vs H8's checkpoint-restore sketch-state guard instead).
- [x] **SP-1 GATE (2026-08-04)**: Shapr3D-style live dimension input, all draw tools except point. W1 pure core: `liveDimension.ts` (input FSM w/ pinned transition table, zoom-adaptive `dimQuantum` = 1/2/5 decade-floor of grid minor in DISPLAY unit, Ø↔R alias), `liveDimFrames.ts` (per-tool measure/rebuild, `rebuild(measure(p))≈p` property-tested), `liveDimConstraints.ts` (typed locks → driving Distance/Radius/Diameter/Angle constraints; angle ladder 0/90→H/V else Angle-vs-prev-segment; dedupe vs inference), `liveToolMachines.ts` decorator (raw machines untouched + transparency deep-equal pin; locks-before-quantum so preview==commit==chip). W2 controller: rounding REPLACES grid tier only mid-gesture (geometry snaps always win), dims ride the SAME upsert as the entity = ONE undo step, conflict → re-upsert without dims (entity survives) + named hint, settings v6→7 (`snapTo.dimensionRound`, `show.liveDimensions`). W3 chips: `liveDimStore`/`LiveDimField`/`LiveDimChips` via engine chip hosts, digit-over-viewport focuses chip (DOM-focus priority rule — zero keymap changes; polygon idle=sides/armed=radius), Tab locks+cycles, Enter commits through the SAME `applySteppedClick` as a mouse click. W4 e2e: 6 new specs + helpers; **e2e found a real bug jsdom couldn't**: Tab's synchronous DOM blur from the departed field fired a field-agnostic `blurCommit` that nulled FSM focus one tick after landing → every keystroke swallowed; fixed w/ field-aware onBlur guard + stale-blur regression test. Suites: vitest 3016/3016 (192 files) · e2e 166 in-sweep + 3 boot-flake stragglers solo-green (polygon/degenerate/drag = vite cold-start >45s under load, product-green) · tsc/build/hex clean. Uncommitted; cohabits HISTORY-HARDEN tree.

## HISTORY-HARDEN (approved 2026-08-04, plan `~/.claude/plans/act-as-senior-software-mutable-turing.md`; internal adversarial review REVISE → 4 BLOCKER + 9 MAJOR + 6 MINOR folded pre-approval)
Parametric history correctness + editing past decisions. Root causes verified: rollback visually inert (count/index domain bug `session.rs:849`); `IntentQuery` never constructed (production rebinding = pure nearest-centroid at weight 0.25); rolled-back append trap (atCursor hardcoded false); undo/redo blind from-0 replay. Halt-at-failure kept (visible + suppress-to-continue). P0 = WP-FIX W5 landed ✓ (4062304).
- [x] **H1 GATE (2026-08-04)**: rollback actually republishes — `set_rollback` emits `PreviewTo(cursor−1)` (count↔index domain split documented at both the emit and the pin); same-cursor roll = FULL no-op (`Inverse::Noop` now DROPPED at the `apply` funnel — sole producer, so no undo slot is occupied and none is evicted); `fully_suppressed_target` re-keyed off the COMPILED plan's `target_step` (the old `start != 0 ⇒ None` guard refused the clear for every suffix request — roll into an all-suppressed prefix kept stale bodies on screen AND in the container; plan-keyed also composes with the §7.3 repair ceiling, which the request-keyed rule ignored); `SetRollback` EXEMPT from checkpoint `invalidate_from` (cursor motion rewrites no record bytes — same exemption class as undo/redo). Red-first ×2 recorded (inert roll "noop"≠"published"; all-suppressed clear). NEW `tests/rollback_lane.rs` (3 real-worker, drives the PRODUCTION RegenScheduler + regen_driver_with_emitter so the hint→request mapping is exercised; geometry via head_body_ids/removed_bodies, never record fields): roll-mid publishes prefix only, roll-forward restores, roll-to-0 clears, all-suppressed roll clears, redundant roll = no regen within 1.5s + undo depth + checkpoint count unchanged.
- [x] **H2 GATE (2026-08-04)**: VF-M7 + VF-M1. Worker: `ElementMapPartition::topokey_for_element_in_body` (returns "" when entry absent OR foreign-body — NOT in OpCommon, W5 owns it); HoleOp/ShellOp tracked-rung goes through it, "" falls to the descriptor ladder. Red-first ctest `test_cross_body_element_ref`: cross-body hole on unfixed code SILENTLY drilled 565.49 mm³ through a wall the user never named (11434.513 vs pristine 12000); + wrong-reason OP_FAILED case + silently-wrong shell wall; all three now exact-volume asserted. Real-worker mirror in `hole_ops.rs` (instrumented probe showed f:5 on B = B's BOTTOM face bound from A's element). VF-M1: poison key = `prefix_hashes[i]|record_id|fingerprint` (prefix hash folds the timeline THROUGH op i's params ⇒ editing the crashing param mints a new key = circuit heals; checkpoint-independent ⇒ restart-stable); defensive `.get(i).unwrap_or(base)` + `debug_assert` at the consumption site (the brief's 3 "hand-rolled" constructors turned out to route through compute_hashes — invariant holds, fallback kept for the public fields); empty-fingerprint ⇒ NO keys minted (agent narrowed the disconnected-bug fix: `hello` is never cleared on disconnect so mid-restart keys DO match — moving the connection check first would have regressed worker_chaos's poisoned-plan diagnostics); fingerprint-flip prune + LRU 128 (open-key eviction trade-off documented) + `clear_worker_circuit` command via new `CircuitControl` seam (BackendBundle 8→9). 8 new unit tests red-first-verified. ctest 88/88.
- [x] **H3 GATE (2026-08-04)**: inline dimension editing — `FeatureDto.primaryValue/primaryValueKind` additive (ONE `feature_value` match decides text + primary together so a row can never display one number and edit another; `value_text` BYTE-UNCHANGED, pinned per op; Chamfer edits d1 only — d2 is a two-field edit that belongs in the tool; Boolean/Transform/Pattern/Import = text-only read-only rows); history row value ALWAYS visible (display-unit rendered), click → `DimensionInput` → `updateScalarParamsCommand` scalar patch, NO tool session; guards tested: `modelTool==="select"` only + `index < appliedOps` + not suppressed; `penEdit` cluster icon → full editor. H3b: `CadClient.setSketchDimension(sketchId, constraintId, type, value)` (4-arg — `toWireDimensionValue` NEEDS the constraint type, 3-arg spec was impossible; both clients) + sketch-row Dimensions section. `appliedOps` hydrated into documentStore + `nextAppliedOps` derivation on ApplyOperationResult paths (carries no cursor — H7b reconciles). 3 bugs surfaced+fixed en route: `(opType && FIELD[opType]) ?? null` passes `""` through `??`; value chip broke row dblclick (target swap between clicks — 220ms deferred open, `detail>1` cancel); selection hint above the list shifted rows mid-dblclick (moved below). M18 pinned: valueText stays mm-fixed + produced regardless of display unit (3 parser consumers). e2e `history-inline-dimension.spec.ts` 2/2.
- **H1–H3 combined gate (orchestrator-run 2026-08-04)**: ctest 88/88 · cargo workspace ALL suites 0-failed vs real worker · vitest 2968/2968 (188 files) · e2e 164 passed / 3 failed — ALL 3 = the CONCURRENT session's untracked in-flight `live-dim-*.spec.ts` (its own feature, zero overlap; every HISTORY-HARDEN + prior spec green incl. history-inline-dimension 2/2) · tsc/build/fmt/clippy clean · hex 0. Not committed with this gate: `CadOrbitControls.*`, `liveDimension`/live-dim specs (concurrent session's tree).
- [x] **H4 GATE (2026-08-04)**: snapshot-scoped promotion (VF-M3 + VF-M4). RUST primary gate `DocumentRuntime::gate_stale_pick` — refuses a promote whose snapshot ≠ published head, recoverable `OpFailed` naming both ids + "re-pick"; SKIPPED when `latest_snapshot` is None or its id is `0` (`drive_clear` publishes `SnapshotId(0)`; the FE only adopts positive ids). WORKER defense-in-depth in `handle_acquire_element_ids` — present-and-not-head `snapshotId` ⇒ `REF_UNRESOLVED` + `detail{requested,head}`; absent = no claim. Anchor-fallback sanity veto in `resolve_pick`: KEEPS `nearest_subshape` (deliberately NOT the 0.85 descriptor gate — a corner hit is legitimately far from its face centre and would be dropped), vetoes when `dist(pick, candidate.bbox centre) > 1.0·descriptor.size + 1.0 mm`; strict geometric bound is HALF the bbox diagonal, so k=1.0 is 2× slack, +1 mm floor keeps a degenerate (vertex, size 0) candidate pickable. VF-M4 cache re-key `(SnapshotId,BodyId,TopoKey)→PromotionEntry{element_id,kind,descriptor}` (worker descriptor was parsed in `wire.rs` and DISCARDED at promote — now stored, H5 consumes it); 2-generation prune in `commit_snapshot` placed AFTER `self.regen = scratch` and after `ordinal_tripwire::evaluate` (which must read the pre-swap mirror); cross-generation reuse is DESCRIPTOR-PINNED (byte-equal `serde_json::Value` + same kind), older than 2 gens re-mints — the safe direction. 6 FE sites route through new `src/ipc/promote.ts::promoteOne` (hint "Selection is out of date — pick again", sticky/error, never aborts the tool; extrude-facePick + hole suppress their arm-hint overwrite so the stale hint survives). **Red-first ×5**: Rust gate isolated on `FakeBackend` (no worker gate) returned the pre-edit id; VF-M4 key revert handed back the identical pre-fillet id for the renumbered `f:6`; descriptor-rung revert minted a fresh id for an unmoved face; ctest showed 4 failures (future/stale snapshotId, detail payload, 40 mm anchor miss binding the nearest face). NEW `src-tauri/tests/element_identity.rs` (3 real-worker) + `worker/tests/test_element_identity_gate.cpp` (6 checks) + `src/ipc/promote.test.ts` (6) + 2 runtime unit tests. SCHEMA §7.5 one normative sentence + §14 entry (additive semantics on an existing field, **no fixture bump**; sign-off recorded in the entry). Suites: ctest 89/89 · cargo workspace 781 passed / 0 failed vs real worker · vitest 2975/2975 (189 files) · tsc/fmt/clippy clean · hex 0.
- [x] **H5 GATE (2026-08-04)**: frozen descriptor AUTHORED — `KnownOperation::element_refs_mut()` (Fillet/Chamfer edges, Extrude ToFace target(s), Hole face; NO Sketch — wire-stripped = hash churn; NO Shell — bare ids, recorded gap) + `hydrate_ref_intents` at the single-writer `apply` (fill-IFF-None, never overwrite — pinned by complement test; mint/re-edit only, load never routes through apply — legacy fixture pins bytes + prefix hash + re-save identity + "warm cache later still stamps nothing"). RED-FIRST headline reproduced exactly: decoy fillet consumed the WRONG 12mm rib on the stale anchor, removed=10.3009, ZERO NeedsRepair, dry-run score 0.9688 — the silent-wrong-bind class, now LOUD (needsRepair=1, correct rib ranked #0 at 0.9069). Congruent twin: exact tie 0.9628/0.0000 ⇒ NeedsRepair (no-silent-wrong floor holds). M16 golden op-bytes + hash pinned (descriptor keys enter the planner hash; serde_json sorted-key order pinned). Op-set agreement test fences element_refs_mut vs wire_op_inputs across all 15 variants. Existing h5b destructive test legitimately diverged (stored ref now carries evidence) — probe now read from the record. Suites: cargo 791/0 vs real worker (topology_rebind 11/11 · m2_gate · sketch_on_face 14/14 · step_import · element_identity 3/3) · fmt/clippy clean · ctest/FE untouched. FOLLOW-UP flagged: `element_ref_input` (document_runtime.rs:3255) lacks a Hole arm — third list diverging from wire_op_inputs/element_refs_mut; fold into H9.
- **H6 DECISION (2026-08-04, measurements-driven)**: H6b two-stage restructure = **NO-GO**. Measured: benign 25→100 edit got WORSE with evidence (0.7077→0.5992; `magnitude` weight 0.25 penalizes the exact dimension a parametric edit changes, and down-weighting it would un-catch decoys — length evidence is discriminative for decoys, misleading for the edited edge; no static re-weighting satisfies both). The failing case's real cure is an OCCT-history rung 1 for the from-0 path (`resolve_input_refs` has none — incremental path proves history-based rebind works) — DEFERRED as future WP "from-0 history rung" (needs persisted/replayed partition, design-level). Deterministic NeedsRepair with the correct candidate ranked #0/#1 is the spine-honest interim. Decoy margin miss 0.0014 vs own rim-twin: NOT auto-collapsible (top/bottom rim = different fillets), accepted.
- [x] **H6a GATE (2026-08-04, design AMENDED twice from the approved plan — orchestrator decisions)**: EDIT-SCOPED, ANCHOR-EXACT-CARVED descriptor-tie veto. Amendment 1: the approved blanket veto (descriptor-tie ⇒ anchor never decides) would have broken EVERY symmetric part on plain reopen (from-0 replay L2-resolves every ref; rim/edge twins tie in descriptor space; the anchor is the only — and there trustworthy — separator) ⇒ veto gated on edit context: additive `editedFrom` on ExecutePlan (§7.2; absent = no claim; only the edit lane `from>0` sets it — open/undo/redo/preview absent; from==0 = deliberate under-claim). Amendment 2 (post-red-first): even edit-scoped, the veto regressed the FLAGSHIP gesture — box fillet NeedsRepair after every upstream dim edit (every no-checkpoint edit-lane regen L2-resolves all refs; box edges are exact twins) ⇒ ANCHOR-EXACT CARVE-OUT: the veto fires only when the tie-winner is NOT anchor-exact (dist to SUB-SHAPE — not centroid, so an edge sliding along its own axis reads unmoved — > `kAnchorExactEps 0.05`·anchor-scale). Net: DRIFT class (twin merely nearer the stale anchor) ⇒ NeedsRepair; TELEPORT class (edit parks an exact twin precisely at the stale anchor) = ACCEPTED DOCUMENTED RESIDUAL, locally undecidable, pinned by a visible test that a future from-0 history rung flips; flagship (anchor-exact winner) auto-binds. + Proportional anchor floor 1.0→1e-7 (0.3mm-apart features on a 0.77mm body: margin 0.075→0.195, red-first). + `kResolverVersion` 2; checkpoint-envelope `resolverVersion` axis stays pinned 1 — CROSS-TRACK SIGN-OFF RECORDED HERE: the two axes measure different compatibilities (replay/envelope vs ladder policy), §9/§10 document the split normatively. Fixture bump: 2 worker ndjson fixtures (`scoringVersion` 1→2, subset matchers, no shape moves); protocol/fixtures untouched. Red-first ×3 (blanket-veto probe: inside-eps + teleport both flip; drift-outside-eps guards too-wide). Calibration set (h5b ×2 + H5 ×3 + symmetric-tie) UNCHANGED, nothing retuned; new `h6a_flagship_edit_lane_fillet_survives_and_reopens_clean` + drift/teleport ctest pair. Worker build now warning-free (rank_key `{}` inits swept). ctest 89/89 · cargo 796/0 · topology_rebind 13/13.
- [x] **H7a**: atCursor:true flip + positional inert-append predicate (trap fix) — shipped in 7e614b3 ("H4+H7a": `tauriCommandMap.ts:593` unconditional atCursor, `tauriClient.ts:689` positional predicate); checkbox was stale, flipped by the SKETCH-PRO session's rebase audit 2026-08-04
- [x] **H7b GATE (2026-08-04)**: history panel UX + failure visibility. Cursor viz: rows ≥ appliedOps grayed-italic "Not applied" + 1px warn marker + "N operations rolled back [Roll to end]" banner (FeatureState only — global indices; SelectionState slice deliberately excluded). Context menu on history rows (ModelTreePanel Popover shape): Edit…/Roll to here/Roll to end/Suppress/Delete-2-click; hover cluster pinned visible for suppressed|selected|error rows. Failure viz: status→tone map (error red · needsRepair warn+⚠ · post-halt dirty grayed via haltIndexOf, pre-halt dirty normal); NEW `TimelineStoppedBanner` derived purely from features[] (no wire change) — names the halt row, click-scrolls, primary Suppress-feature (cascade), secondary "Reset failure breaker" → new `CadClient.clearWorkerCircuit()` (H2 command, both clients); errored rows carry always-visible "Suppress to continue rebuild". `REGEN_STARTED` tauri event (REGEN_PROGRESS left reserved) → documentStore.regenBusy (clamped, reset on doc replacement) → StatusBar "Rebuilding…"; mock mirrors transitions. Mock-lane rollback UNDER BUDGET (102 lines): mockAppliedOps cursor + body masking + `insertAtMockCursor` mirroring `Timeline::insert_at_cursor` exactly (cited), undo carries cursor; Rust↔mock parity vitest 6-case. Cursor rides ApplyOperationResult (appliedOps/totalOps both clients) with projection hydration authoritative; `totalOps == features.length` documented + pinned rather than duplicated as a store field. Halt-banner e2e → vitest component test (pre-authorized: mock lane cannot produce status:"error" — no kernel to fail). EditorScreen banner mount committed via clean index entry (file cohabits live-dim work). vitest 3015/3015 (191) · cargo 796/0 · e2e ours green (history-rollback 2, insert-at-cursor 1, suppress +2; 6 contention flakes solo-green; 3 live-dim fails = concurrent session's) · tsc/build/fmt/clippy/hex clean.
- [x] **H8 GATE (2026-08-04)**: undo/redo acceleration. NEW `RegenRequest::RevertToEnd{from}` — plans identically to ToEnd but `edited_from()` = None ALWAYS (origin rides the request VALUE, minted by the runtime not the api, so no caller can arm the H6a veto with a revert; red-first (d): ToEnd on the undo lane re-armed the veto against the state it just restored, needsRepair=1→0). `Inverse::dirty_floor` exhaustive (conservative-Some(0) rule; RestoreSketch shares `sketch_dirty_step` with the edit lane so the two can't disagree); session undo returns folded floor, redo folds previously-DISCARDED outcomes; runtime `revert_report`: invalidate_from(RAW floor) + request at B1-clamped `min(floor, applied_after−1)` (red-first: unclamped undo-of-append NoOps, body stays on screen). Rollback-head mint on the ToStep publish lane, cancel-token debounced, best-effort. UNPLANNED BLOCKER FOUND+GUARDED: checkpoint restore loses worker sketch/region state — a checkpoint whose base swallows a consumed Sketch producer kills the feature ('profile sketch not found in plan'); pre-existing but H8 made it reachable (reverts can now select checkpoints; mints at arbitrary heads) — `suffix_sketches_stay_planned` refuses such checkpoints (degrade to replay, Invariant 7); REAL FIX (worker carries sketch defs across restore) → ROADMAP. Brief's 'sketch-geometry hash hole' premise CORRECTED: prefix hash covers sketch content via record params (upsert funnel invalidates); eviction still load-bearing post-H8 (reverts read the cache now) — doc comments state the true rationale. `undo_checkpoint.rs` 5 real-worker cases incl. H8-payoff restore-from-checkpoint assert. cargo 820/0 at wave gate.
- [x] **H9 GATE (2026-08-04)**: reattach V1 + repair generalization. Core `InputPath::{ShellOpenFaces,HoleFace}` + set_input arms (Shell bare-id w/ set_fillet_edge discipline — evidence DROPPED, no slot; HoleFace whole ElementRef, refuses primary-less). `NeedsRepairItemDto.bodyId` (derive_inputs.bodies.first() → outputs.first()) — multi-body rebind refusal now fallback-only. H5-flagged `element_ref_input` Hole arm fixed (dry-run and regen agreed to disagree about slot 1). Slot table PINNED BOTH SIDES (19 cases each, tests cite each other + name the silent-mis-repair failure mode; Extrude trap pinned: Blind+ToFace2 collapses targetFace2 to slot 0 — inputPathFor reads stored params or returns null, never guesses). Hole host-face rebind VOLUME-PROVEN (9293.14165 top → break exact-10000 NeedsRepair → 9434.51332 side = π·9·20 on the re-picked 20mm stock — proves WHERE the drill landed). Shell rebind pinned honest: explicit re-pick writes through (2224→10000→2224), untracked id has no evidence rung (typed ShellParams::faces = deferred §7.3 change). TWO EXTRA BLOCKERS FOUND+FIXED: (1) unrepairable ref with empty anchor ⇒ FATAL 'needsRepair parse: missing field worldPoint' tore the worker down — the most ordinary broken-ref case could never reach the repair panel; Rust reader now treats anchor-without-worldPoint as absent (structural fields stay strict, pinned); (2) REATTACH WAS INERT — UpdateSketchAttachment never restamped the timeline record's plane, every regen replayed the old plane (renamed `restamp_sketch_record` now moves plane+host_face; 2 core pins). Reality check: projectedBoundaryVersion never 0 in practice and never reaches FE ⇒ V1 = world⇄datum only (authorized fallback), host-face reattach refused + menu withheld; produces_before drop UNREACHABLE in V1 (documented). `RepairMarkerOverlay` finally consumes `hoveredWorldPos` (mountChip pattern). Mock reattach HONEST (moves plane + re-synthesizes standing extrudes, mesh-bytes asserted). e2e repair-rebind-multibody 3 + sketch-reattach 3 (real draw→extrude→reattach flow). ctest untouched 89/89.
- [x] **H10 GATE (2026-08-04)**: dependency view V1 — `feature_dependencies` command (graph upstream/downstream transitive, unknown → typed error, suppressed still listed); both clients (mock derives from featureTouched, seeded-fixture limit documented not faked); delete/suppress confirms show "— N dependent(s)" (row tooltip + full context-menu label, fetched on open); InspectorPanel "Depends on / Used by" clickable sections below the list. Real N>0 chain proven on the mock lane e2e (Fillet→Shell via genuine UI). `PendingBackend` command test 3-record chain. +59 vitest at wave gate.
- **HISTORY-HARDEN COMPLETE 2026-08-04** — final gate (orchestrator-run): ctest 89/89 · cargo 834/0 vs real worker · vitest 3090/3090 (194 files) · e2e 179/179 zero-fail · tsc/build/fmt/clippy/hex clean. 10 waves, 6 gate commits, 4 blocker-class root causes fixed that predate the plan (rollback inert · IntentQuery never constructed · rolled-back append trap · empty-anchor fatal), 3 more found en route (checkpoint-swallows-sketch · reattach inert · H6a blanket-veto reopen regression — all guarded/amended). NOTE: 049ffe4 accidentally omitted ScratchJob.h/OpTypes.h/test_wp6_ladder.cpp (H6a threading) — folded into the final commit; that one commit won't build C++ standalone.
- Deferred (recorded): DAG error isolation · from-0 OCCT-history rung (kills the H6a teleport residual + the benign-large-edit NeedsRepair) · worker sketch-state across checkpoint restore (H8 guard degrades to replay today) · host-face re-projection reattach · durable promoted-id (TODO.md:20) · Shell open_faces→ElementRef §7.3 shape · wire_op_inputs Sketch arm · reorder · folders

## ROADMAP QUEUE (approved 2026-08-02, plan `~/.claude/plans/act-as-senior-software-reflective-swing.md`; internal adversarial review REVISE → 3 BLOCKER + 7 MAJOR + 8 MINOR folded)
User priorities: 3D-print + machined parts + daily driver; light multi-part (no mates); STEP import high value.
- [ ] **Step 0 (USER)**: consolidated manual-gate run — SUPERSEDED 2026-08-08 by the P1 triage: 78 of the ~112 boxes are now cited against existing automated tests and retired, 12 remain in `docs/qa/MANUAL_RELEASE_GATES.md`, the rest are named gaps in `docs/qa/MANUAL_GATES_TRIAGE.md`. Run the 12-item release checklist instead; the historical list is archived at `docs/qa/archive/MANUAL_GATES_RUN-2026-08-04.md`.
- [x] **WP-0 GATE (2026-08-02)**: `split_origin` probe + `MAX_SPLIT_CHILDREN=256` cap DELETED — `split_child_uuid` (sole mint path: parse_body_id, open/restore re-intern, plan lowering) now memoizes `derived→(op,k)` at derivation, so origin recovery is exact + unbounded (a >256-solid op used to silently lose `split_of` ⇒ cross-process REF_UNRESOLVED). Regression-locked: core k=300 stamp + producer-mismatch guard; wire full chain parse→fold→fresh-process-reintern→exact render at k=300; M5a `split_persist_survives_cold_interner` re-run green. SCHEMA §2 single-vs-multi minting rule stated + §7.2 `:<k>` widened to "ordered children of any N-body op, no `deleted`-parent coupling, ordinal domain unbounded" + §14 entry (doc-only, no fixture bump, sign-off recorded). Suites: cargo 608/0 vs real worker · clippy/fmt clean · ctest/FE untouched (no worker/FE files in diff).
- [ ] **WP-A STEP-IMPORT** (in flight 2026-08-02): §7.8 verb→§7.3 `ImportStep` RECORD; content-addressed `imports/<sha256>` authoritative container section; XCAF names+colors via MESH1 `FACE_COLORS` (W4.5); re-import in scope.
  - [x] **W0 GATE (2026-08-02, commit 0598369)**: pure `io/StepRead` heal pipeline v1 (RAII `Interface_Static` guard — ExportStep's own `write.step.schema` leak observed+logged; messenger-scoped stdout guard, fd1=0 bytes asserted WITHOUT main.cpp redirect); cross-process determinism PASS (PIE binary, 5 runs 1 digest, canonical-geometric-order face keys make the probe real); BinTools 100-112× faster + 4.8× smaller than STEP re-parse, digest-identical round-trip. **CODEC DECISION: brep-primary replay** (STEP co-stored as provenance; `InspectStep {includeBrep}` is the conversion lane, SCHEMA §7.8 amended; unitScale applied at execute time both codecs so brep bytes stay canonical-unscaled). Finding: `ShapeFix_Shape` reports DONE on pristine geometry ⇒ STEP_HEALED alone is noise — f/e/v count-delta distinguishes restructure from touch-up, exact counts pinned. ctest 81/81 ×3.
  - [x] **W2 GATE (2026-08-02, commit 14a22ea)**: core `KnownOperation::ImportStep` (validated add+update, no inputs) + `io/imports.rs` third container class — sha-verified read (typed error, doc still OPENS with unrenderable body), 256 MiB per-blob cap, refcount-at-save keyed on record PRESENCE (suppressed/past-cursor records pin blobs), write-side hash verify, `imports/` excluded from cache API (never reported discardable). document.json byte-stable, snapshots untouched. SCHEMA §7.3 op + §7.8 InspectStep + §2 + capabilities + 2×§14 signed off, no fixture bump ($any capabilities in hello fixture confirmed). dto.rs interim arms (kind→Boolean, label "Import"). Core 303/0 · workspace 639/0 vs real worker · clippy/fmt clean.
  - [x] **W1 GATE (2026-08-02, commit bbfba0a)**: `ops/ImportOp` both codecs (brep lane = BinTools compound in stored ordinal order, NEVER re-sorted; unitScale at execute time both codecs; healPolicy≠v1 ⇒ OP_FAILED); `io/InspectStep` probe + includeBrep conversion (bin section, brepFormat=4 static_assert-pinned, not _CURRENT); `OpOutcome::diagnostics` → existing planStep.diagnostics[]. **PRODUCTION DEFECT FOUND+FIXED**: `BinTools_ShapeSet::Read` prints version-mismatch to std::cout BYPASSING the messenger redirect (80B mid-OCW1-frame corruption) — banner validated before OCCT touches bytes. Cancel wired into TransferRoots (already-fired-token semantics pinned; Cancelled ≠ Failed). ndjson harness fixture self-produces its STEP via ExportStep (plan 88→89). BRepCheck advisory both lanes (STEP_INVALID_SHAPE diagnostic, not fail — codec parity). ctest 83/83 ×3 orchestrator-verified · hygiene clean. **+provenance amendment (7bdb505)**: brep-primary would orphan-drop the original STEP at first save — params gain `provenanceSha256`, refcount pins both, SCHEMA §7.3.
  - [x] **W3 GATE (2026-08-02, commit 4d58f0d)**: `imports.rs` blob workspace (per-doc temp dir pid+seq-disambiguated — bare docId would cross-delete on reopen-while-old-runtime-lives; Drop-clean + 7d sweep) + global sha→path registry read at lowering (missing blob ⇒ empty path ⇒ worker OP_FAILED, plan never aborts, pinned); `StepImport` 8th facet; `import_step` probe-before-publish (bad file leaves start screen intact); `insert_step` at_cursor + apply_edit correlation; `step_file_dialog` (found: start lane could never pick .step); brep-primary authoring (provenance+replay both pinned); productNames insert-only pre-adoption (user rename wins, asserted); save/autosave → save_with_imports (autosave carries blobs — recovery requirement). 7 real-worker tests NEGATIVE-CHECKED (path stub ⇒ 6/7 red; save revert ⇒ exactly 2 red; vacuous-green reopen test caught+fixed). Workspace 652/0 · clippy/fmt clean. Flags: blobs resident in memory (~512 MiB worst case) + rewritten per autosave tick; registry TOCTOU impossible in V1 single-doc.
  - [x] **W4 GATE (2026-08-02, commit 6ad9c46)**: FileMenu Import STEP + start-lane `stepFileDialog`; ImportStep rows opType-routed to hint-not-editor (kind route would open BOOLEAN editor — non-vacuity asserted); mock lane fabricates through real projection events; ModelTreePanel pre-existing vitest race ROOT-FIXED (in-flight applyEdit landing after next test's resetStores — the SAVE/OPEN-RENDER flagged flake). tsc 0 · FE 2190/161 ×4 · e2e 114/114 full incl. step-import 3/3.
  - [x] **W5 GATE (2026-08-02, commit 6c14a81)**: imported bodies first-class — fillet on imported edge (0 needsRepair, wrong-bind ceiling proven), boolean exact, sketch-on-face + host-boolean Add on imported face (host MODIFIED), MESH1 all bodies, process-death: identical hash chain + ElementIds verbatim + ladder dry-run verdict reproduced in a process that never saw the promotion; delete-import ⇒ NeedsRepair "no-candidates" (never a survivor grab), undo exact. Ordinal-agnostic by construction (survived the parallel brep→xbf codec flip unchanged). 653/0.
  - [x] **W4.5 BACKEND GATE (2026-08-02, commit 7a39f47)**: XcafCodec canonical BinXCAF (storage v12 pinned; AddShape label-collapse guarded), XcafRead GEOMETRIC face binder (TShape identity impossible across readers; ambiguous keys dropped never guessed; solid-label color FILL — XCAF doesn't inherit downward, whole-part-blue was 0/6), ImportOp xbf codec + BodyRecord face_colors → Mesh1 FACE_COLORS (color-less bodies byte-identical, asserted). **THIRD stdout defect**: TDocStd_Application PRIVATE messenger → 58B ANSI on fd1 per bad .xbf — re-pointed, 0-byte pinned. Per-ordinal productNames (per-root named nothing on multi-solid). xbf byte-deterministic (content-addressing). ctest 84/84 · cargo 654/0 · colored-fixture match 10/10.
  - [x] **W4.5-FE GATE (2026-08-02, commit 885d92e)**: FACE_COLORS ingestion — de-index preserves triangle ordinals (Picker/Highlight/Ghost verified + real-raycast pin of three.js non-indexed faceIndex semantics); shadedVertex MaterialKind white-base; unset faces baked from body token + theme REBAKE in-place (negative-checked ×3); mock lane + e2e live-scene assertions. tsc 0 · FE 2223/162 · e2e step-import 4/4 · hex clean.
  - **WP-A COMPLETE 2026-08-02** (W6 probe-preflight-dialog + progress frames deferred by design). Real STEP import: Start Screen + File menu → XCAF names+colors → first-class downstream ops → process-death-stable identity.
  - [x] SEAM (W5 finding, pre-existing SKETCH-ON-FACE) — **LATENT, not user-reachable.** Re-traced
        2026-08-14. The identity half is REAL and unchanged: `DocumentRuntime::promoted` is in-memory
        (`document_runtime.rs:503`), `host_face` is stripped from the wire before the worker sees it
        (`worker/wire.rs:275`, pinned by `sketch_host_face_is_dropped_from_the_wire_params`), so the
        partition can never echo an `existing` id and a post-reopen promotion mints a fresh UUID
        (`regen/engine.rs:759`). What is GONE is the consumer: the dblclick re-entry that compared the
        two was deleted in `1fe0cef` — double-click now selects the connected body
        (`ViewportRoot.tsx:414-437`), and `e2e/sketch-on-face.spec.ts:378,392` pins that it must never
        enter sketch mode. Surviving `hostFace` readers use presence or `bodyId` only
        (`reattachActions.ts:46`, `ModelToolController.ts:1576`). The finding's second clause is stale
        too: the promotion cache has been keyed `(SnapshotId, BodyId, TopoKey)` since VF-M4, with
        descriptor-pinned cross-generation reuse, so an ordinal renumber yields a MISS, never a wrong
        bind. **What would wake it:** any new consumer comparing a fresh promotion to a persisted
        `hostFace.elementId`. The two doc comments that advertised that contract now say so
        (`src-tauri/src/dto.rs`, `document_runtime.rs::sketch_host_face`). No reopen+re-entry test
        exists in any suite — `sketch_on_face.rs:1597` reopens but never re-promotes.
- [ ] **WP-B BODY-TRANSFORM** (in flight 2026-08-03):
  - [x] **W0 GATE (2026-08-03, commit 0dc8c91)**: core TransformBodyParams + `can_fold_transform` lineage query; **edit-safety gate REFINED from blanket-ban to edit-time seeding** — level-1 partition rebinds stay exact under rigid motion (new worker `apply_placement` moves stored anchors WITH the body, fixing the pre-existing stale-anchor latent in `apply_history`), so healthy transform-then-fillet resolves 0 needsRepair; params edit or suppress toggle on a TransformBody seeds NeedsRepair on downstream lineage refs (rides the command's undo entry, cleared by repair resolution). Worker TransformOp (normative T∘R, copy:false modify-in-place id-preserved, copy:true §2 N-body minting). Proofs: ctest 85/85 · cargo 680/0 vs real worker (transform_body 8/8: healthy flow clean, edit ⇒ seeded NeedsRepair never silent re-bind, undo exact hash, multi-target, T∘R order pin). NOTE: implementing agent died at spend limit AFTER completing code+tests; orchestrator verified all suites + safety pieces directly and committed.
  - [x] **W1 GATE (2026-08-03, commit a540e74)**: FSM full-vector state (chip value = view of addressed component — axis switch can't clobber siblings); fold = backend query AND stored-targets==selection (per-body query would widen single-body records); NO PreviewOp lane (rigid ghost kernel-exact, documented); mock transformMesh1 re-derives bbox/FACE_BBOXES from moved data (pinned numerically). FE 2299/166 · e2e 124/124.
  - [x] **W2 GATE (2026-08-03, commit 2ab14ac)**: TransformGizmo 9 handles + extrude-precedent orbit arbitration (empty space orbits while armed); transformDrag math pinned (ring seam unwrap 360-not-0, MIN_VIEW_SIN degenerate guards negative-checked); Copy sticky (one record = one flag; Alt at grab writes the visible flag). FE 2369/169 · e2e 130/130 zero flakes.
  - [x] **W2.5 GATE (2026-08-03, commit 7898e7d)**: align face-to-face — planarity honesty gate refuses curved faces; deterministic 180° branch (stable perpendicular); round-trip 1e-12; two-pick flow on the armed record, tangential nudge stays live. FE 2429/171 · e2e 132/132 (clean-machine rerun; the 42-min contended run's 9 non-passes all contention flakes).
  - **WP-B COMPLETE 2026-08-03** — move/rotate/copy/multi-body/align, parametric + foldable, edit-safety gated. W3 pickFrame stays deferred (gate makes it safe); riders (pattern-axis Tier B, DeleteBody op, scale) stay backlog.
- [ ] **WP-C PRO-OPS sweep** (tranche 1 SHIPPED 2026-08-03):
  - [x] **C1 GATE (commit cf5a236)**: `QueryMassProperties` worker verb (GProp, Huygens-pinned) → ElementQuery seam → MeasurePanel mass card + two-plane angle (60/120 convention, cylinder refused, parallel ⇒ perpendicular separation); mock EXACT (divergence theorem + Jacobi; cylinder asserted as its 24-gon prism, never smooth numbers). ctest 86/86 · cargo 681/0.
  - [x] **C2 GATE (commit 425c175)**: displayUnit mm/cm/m/in (settingsStore v6, DisplayModePopover row); 24-site audit — valueText re-parse lanes PINNED mm (would seed 2mm fillet as 0.079); marshalling independence pinned; area factor². units e2e 7/7. Seam flagged: history valueText renders backend mm string verbatim (re-edit seed lives in controller).
  - [x] **C3 GATE (commit e2af9a0)**: [Draft] segment on extrude (±89° oracle-cited clamp, re-edit byte-identical no-op pinned); revolve typed-region selection binding via getSketchRegions (persisted sketch, exact-id, finishSketch-never-at-arm mock-rejected). FINDING: TODO:123 revolve clause mostly stale — TRUST T3 + EXTRUDE-COMMIT-FIX 2 had fixed the throw; residual was selection binding (fixed; e2e fails on old lane). Combined gate: ctest 86/86 · cargo 681/0 · FE 2534/173 · e2e 144/144 · tsc/clippy/fmt/hex clean.
  - [x] **T2 GATE (2026-08-03, commit 671ff40)**: two-distance chamfer (reference face = smallest topokey face ordinal; determinism proven by CENTROID — volume is leg-symmetric and cannot see a flipped face; Fillet⇄Chamfer flip rejected while d2 set, red-first ×3; Fillet-extra resurrection guarded; mock spread-merge fidelity fix) + sketch fillet `F` / offset `⇧O` (closed-form math 1e-9 both orientations; welds NOT tangents — slot-tool redundancy precedent; over-offset FOLD detection; offset additive ⇒ locked sources allowed). ctest 86/86 · cargo 688/0 · FE 2620/176 · e2e 149/149.
  - [x] **T3 GATE (2026-08-03)**: HOLE tool, full cross-layer. Core `HoleParams` + `HoleType` + validate matrix (conditional `cb*`/`cs*` REQUIRED-and-EXCLUSIVE both directions — a stale block on the wrong profile is the only defect this layer can catch, since the worker never reads a param it does not use; `csAngleDeg ∈ {82,90,100,120}`), session wiring on BOTH authoring entry points, `Hole` in `KNOWN_OP_TYPES`, dto Boolean bucket + label "Hole" + valueText `Ø5.5`, wire `inputs[]` = [host body, typed host FACE ref] (ladder-rebindable like a fillet edge). Worker `HoleOp.cpp` + `HoleTool.cpp`: partition-then-ladder face resolve → `planar_face_plane_normal` (non-planar ⇒ named OP_FAILED) → re-project the frozen point (1e-3 plane fence + `BRepClass_FaceClassifier` boundary test, both named) → axis = INWARD normal → tool solid → ONE `checked_boolean` Cut → FilletChamfer publish tail (modified host, nothing minted; explicitly NOT `publish_boolean_result`, which would mint split children the contract forbids). Tool-solid math: every face-seated piece starts `kFaceOvershoot`=1e-2mm OUTSIDE the face and is lengthened by the same amount — the overshoot lies in void, so blind depths stay EXACT while the tool cap never sits coplanar with the host face; the countersink applies it as a FRUSTUM extension (top radius +`o·tan(half)`) so the cone's profile AT the face is still exactly `csDiameter`; `csDepth = (csD−d)/2 / tan(csAngle/2)` derived, never authored. Through-all = bounded bbox ray extent (ExtrudeOp parity), NOT a magic length. FE: `holeMachine.ts` (own file — `modelToolMachine.ts` is 1445 lines; conditional blocks KEPT in FSM state, EMITTED only when active), `holeStandards.ts` (ISO 273 clearance / DIN 974-1 row-1 SHCS counterbore / DIN 74 form A countersink, raw mm only — thread designation never persists), `HoleChipCluster.tsx`, toolbar entry + `⇧H` chord (plain `h` is GLOBAL home and mode keys win — `filletChamfer.spec.ts` pins that), authored `hole` glyph, re-edit route. Re-edit commits WHOLE params, not a scalar patch: a spread cannot DELETE a key, so counterbore→simple would strand `cb*` and the session would reject it. Tests: ctest `hole_op` (through-all/blind/cbore/csink analytic volumes on the exact BRep via GProp at 1e-6 REL, both point fences, non-planar seat, cb/cs invariants, bit-equal determinism); `hole_ops.rs` 7 real-worker (same volumes via `QueryMassProperties` at 1e-9 rel, through-all on 25mm AND 40mm hosts, Ø6→Ø10 re-edit, preview≡commit, point-fence recoverable, save→reopen identity + document.json byte-stability); FE FSM/standards/controller/chip-conditional/previewOps-pin; `e2e/hole.spec.ts` 8 (mock-limit header: no CSG in the mock, geometry pinned kernel-side). Gate: ctest 87/87 · cargo 704/0 vs real worker · FE 2688/180 · e2e 157/157 · tsc/clippy/fmt/hex clean. SCHEMA untouched (the 2026-08-03 §7.3 Hole block + §14 entry were already landed; additive, no fixture bump).
    Flags: the counterbore column follows DIN 974-1 row 1 and the countersink DIN 74 form A — both transcribed, worth a spot-check against the paper standards before release (the ISO 273 clearance column is test-pinned size-by-size). A hole that SPLITS its host (drilling through a narrow bridge) publishes one multi-solid body rather than minting children — SCHEMA §7.3 says "nothing minted", so splitting is out of contract; revisit if a real part hits it. `holeStandards` stops at M12 by design.
NOT next: assemblies/mates, Loft/Sweep, drawings.

## VALIDATION PHASE V (2026-08-03, plan `~/.claude/plans/act-as-senior-software-shimmying-blanket.md`, approved; orchestrator + 3 adversarial review agents, top findings orchestrator-verified in source)
- [x] **V1 fresh 4-suite re-run**: ctest 87/87 · cargo 704/0 vs real worker · FE vitest 2688/180 · e2e 157/157 (9.0m, zero flakes) · fmt/clippy/tsc clean · hex gate FULLY clean (inputProbe.ts deleted by DEV-OBSERVABILITY). All counts match the gate claims exactly.
- [x] **V2 probes** (scratchpad `vprobe`, real worker): SAVE-GROWTH CONFIRMED — checkpoints accumulate 1/save, never evicted (12 after 12 saves incl. 12 kept after undo×4 = orphans retained), container grows quadratically (4.0→57.6 KB over 12 toy saves, per-save delta monotonically rising), save wall time 19→154 ms; autosave = same cost UNDER the runtime lock (~150 ms at toy scale). BIGSTEP: 40-solid 639 KB STEP imports in ~260 ms, autosave-with-blobs 128 ms (blob deflate scales linearly → ~seconds at 100 MB imports). SOAK 120 regens: RSS flat (self 15.4→15.9 MB, worker 27.3→29.0 MB) — no leak.
- [x] **V3 adversarial review** (identity spine / regen+protocol / imports+transform+hole): findings below; BLOCKERs orchestrator-verified in source.

### WP-FIX (approved 2026-08-03, plan `act-as-senior-software-shimmying-blanket.md`; internal adversarial review REVISE → 4 BLOCKER + 8 MAJOR + 3 MINOR all folded). 5 waves: W1 quick kills → W2 checkpoint policy → W3 save off-lock → W4 transform gate (L) → W5 ordinal tripwire (L).
- [x] **W1 GATE (2026-08-03)**: VF-B1 `FLAG_HAS_FACE_COLORS` in `FLAGS_KNOWN_MASK` + checked-in golden colored fixture (`tests/fixtures/colored_boxes.step`, generated once via `worker/tests/step_fixture_util.h` helper; FILE_NAME timestamp ⇒ not byte-reproducible) + real-lane `mesh_face_colors.rs` (red-first: `unknown flags bits set: 0x0010`); VF-B2 per-runtime `instance: Uuid` threaded through Prepared/DrivenRegen, `finish_regen` gates commit AND the EngineFailed-downgrade branch (red-first: doc A Published into doc B); VF-B4 `GeometryEngine::current_epoch()` default + WorkerManager override, constructors seed `FencingCell` from live epoch, `adopt_current_epoch()` under the slot lock at all 4 insert sites, `RestoreRequest.worker_epoch` unifies restore+plan fencing (red-first vs stub: `workerEpoch fencing mismatch` forever); VF-M2 `prepare_checkpoint`/`adopt_checkpoint` split w/ `head_epoch` stamp — refuses mint when head geometry predates the live epoch (agent deviation, orchestrator-verified: ticket-only can't go red today since the mint holds the lock; `head_epoch` is the load-bearing guard, ticket recheck becomes load-bearing at W3), SaveCheckpoint call-count asserted not checkpoint_count (overwrite made count vacuous). Suites (orchestrator-run): cargo 708/0 vs real worker (+4) · fmt/clippy clean · FE 2688/180 · ctest untouched. NOTE: `restore_checkpoint_args` JSON keys unchanged but UNPINNED by any test (wire_contract gap — W2 rider).
- [x] **W2 GATE (2026-08-03)**: checkpoints IN-SESSION ONLY — `checkpoint_caches()`/`load_checkpoints()` deleted, save/autosave write `ContainerCaches::none()` (container format unchanged, legacy `checkpoints/` ignored + logged once); `InMemoryCheckpointStore` bounded ladder ≤5 (anchored at max stored step — M7: rollback-save never evicts a later valid checkpoint) + NEW `CheckpointStore::invalidate_from` called at the ONE `DocumentRuntime::apply` funnel keyed on `outcome.dirty.from` (undo/redo bypass BY DESIGN — redo-side checkpoints stay valid, hash filter guards); `AdoptingEngine` derives `known_ops` PER REQUEST (constructor field deleted — RetryFromZero's wider replay now validates by construction, VF-B3c). Red-first: restore-failure retry `does not match any known opId (D1 malformation)` → Published; reopen→append vs real worker + legacy-container variant + bounded-growth (12 cycles ≤5 ckpts, no `checkpoints/` in zip, undo orphans evicted); `signature_drift` retargeted to core planner unit (container doctoring obsolete); `restore_checkpoint_args` wire keys PINNED (W1 rider closed). SCHEMA §7.7 in-session-only note + §14 doc-only entry, sign-off recorded here, no fixture bump. Suites (orchestrator-run): cargo 717/0 vs real worker (+9) · fmt/clippy clean. PROBE before→after: container @24 ops 57.6→7.0 KB (quadratic→linear), save 154→30 ms, autosave 150→28 ms, ckpts 12→2. Flags: ladder mid-rungs THIN under save-every-step (settles `{0,2,head−1,head}` — greedy prune; bounded holds, deep-edit acceleration weaker than intended; pinned in `ladder_stays_bounded_and_keeps_head_and_floor`, bucket-rule fix ~10 lines → WP-FIX2 rider); `suppression.rs` pin 2→1 (truncation eviction now removes the hash-stale checkpoint the test itself doctors — subject unchanged).
- [x] **W3 GATE (2026-08-03)**: save/autosave OFF the single-writer lock (VF-B7 rest + F1a). `ImportBlob.bytes: Arc<Vec<u8>>` (Eq preserved; carrier clone = refcount, no memcpy); NEW `SavePayload` + `build_save_payload(&self)` / `write_payload(path, &payload)` (no `self`) — `save`/`write_autosave` keep their signatures over the pair. `autosave_current` now snapshots under the runtime lock, DROPS it, then writes on `spawn_blocking` with the join handle **awaited** (M3 — `None` is the failure contract); marker written only after an Ok write. `save_document` restructured into 5 phases: prepare_checkpoint+engine_arc (locked) → SaveCheckpoint **unlocked** → adopt_checkpoint (W1's ticket recheck is now load-bearing) → build_save_payload+revision (locked) → write+`clear_recovery_state` under the lane (unlocked) → NEW `mark_saved(path, rev_at_build)` which adopts the path always but clears `dirty` ONLY if no edit landed mid-write; every re-lock re-checks `document_uuid` (close/open can now interleave). NEW `AppState.persistence: Arc<Mutex<()>>` — lock order **runtime → release → persistence, never nested**; shared `autosave::write_save_payload` is the save's lane half so both writers use one code path. Red-first (all four verified by reverting to the naive shape): `mark_saved` unconditional-clean ⇒ dirty pin fails; `adopt_checkpoint` without the fencing recheck ⇒ ticket-discard pin fails (count 1, want 0); per-task lanes ⇒ `concurrent_autosaves_on_one_path_all_land` fails on a lost temp (M3); lock-held-across-write ⇒ the lock-order pin times out after 5 s. NEW `tests/persistence_lane.rs` (4: lock-order, M3 3-writer, M2 recovery-consistency, real-worker colored-STEP import autosave with 40 concurrent edits) + 4 unit pins (payload≡save byte-identical, `Arc::ptr_eq` no-deep-copy, dirty semantics, ticket discard). Suites (agent-run): cargo **725/0** vs real worker (+8) · fmt/clippy clean · zero FE/worker/SCHEMA changes (no FE re-run needed). PROBE (16 MiB import blob, lock-held → off-lock): max edit lock-wait during an autosave **2998 ms → 0.02 ms**, edits landing during the write **15 → 957**, write itself unchanged (~3.0 s, off-lock); on-lock snapshot <0.05 ms. Flags: accepted+documented residual — an autosave that snapshotted BEFORE a save but reaches the lane after it can re-write a spurious marker; harmless (it names a real autosave holding a superset of the saved work), pinned as "marker exists ⇒ autosave exists" rather than closed.
- [x] **W4 GATE (2026-08-03)**: transform edit-safety gate holes closed (VF-B5 a/b/c) + repair-inverse ordering (VF-M8). Single-snapshot rule: all 6 repair-mutating commands capture `prior_repair` ONCE at entry, `fold_repair_inverse` emits exactly one `RestoreRepair` FIRST in a FLAT Composite (undo `.rev()` applies it last — red-first: first transform's seeds survived undo). `SketchOpParams.host_face: Option<ElementRef>` (skip-if-None; NEVER backfilled — inputs are inside the golden prefix hash and re-derived per deserialize; byte-stability + hash pinned on legacy docs) stamped at mint/refresh + `update_sketch_attachment` restamp (with `produces_before` anti-time-travel guard — load-bearing: legacy front-inserted records hard-error without it); `derive_inputs` Sketch arm pushes host body+element (Hole-arm shape); legacy docs gated via `transform_gate_items` attachment bridge (takes &Document now). Delete-TransformBody seeds the gate over post-removal indices (was: bypass); `shift_seeded_for_insert/remove` remaps seeded step_index (ladder items self-heal). WIRE: `strip_sketch_host_face` in lowering — §7.3 'attachment never crosses the wire' stays literally true, zero SCHEMA/§14/fixture churn, pinned. Cascade semantics: hosted sketch now downstream of its host's producer — new pin `suppressing_the_host_bodys_producer_cascades_to_the_hosted_sketch`; all 8 existing suppression pins untouched (World-attached). Red-first ×7 recorded (M8 undo leak, empty-gate real-worker Move edit, legacy bridge, delete bypass, step shift, wire leak, guard hard-error). 20 new tests. Suites (orchestrator-run): cargo 745/0 vs real worker (+20) · FE 2693/180 · fmt/clippy clean. W5 flag: `commit_snapshot` is not an EditCommand — OrdinalPermutation seeding needs its own inverse story.
- [x] **W5 GATE (2026-08-03) = WP-FIX GATE**: split-ordinal identity tripwire (VF-B6) — a parametric edit that flips an N-body op's children's geometric rank is now LOUD. WORKER: `ordered_solids` split into `ranked_solids` (returns `{shape, RankKey}`) + a thin shape-only wrapper — the quantized key is computed ONCE per solid and RETAINED instead of being discarded inside the comparator; same `stable_sort`, same `llround(v * 1e6)` lexicographic tuple ⇒ ordinal assignment byte-identical (existing wp6_split/revolve_split/step_read ordinal pins untouched and green). `session::BodyEvent` gains `std::optional<RankKey> rank_key`; `publish_boolean_result` attaches it to every split child AND to the single-solid `modified` (free — same GProps). Emitted as `bodyEvents[].rankKey` ONLY when present. DELIBERATELY NOT attached: TransformOp copy path (ordinal = the caller's target-list index — lineage, not geometry) and ImportOp (ordinal fixed by the content-addressed blob; no edit can permute it) — both noted in SCHEMA §7.2 as "absence = no claim". RUST: `PlanStepEvent.body_rank_keys: BTreeMap<BodyId, [i64;5]>` (side map, NOT a `BodyLifecycleEvent` variant field — that enum is PERSISTED in the registry log, widening it would move every document's bytes); wire parse is additive + **infallible** (a bad diagnostic must never tear down a valid regen); executor stamps `BodyMeta.geom_stamp` after the lifecycle fold, under the same `last_valid` gate as the geometry. `RepairReason` gains `#[serde(other)]`-equivalent `Unknown` via a HAND-WRITTEN `Deserialize` (serde's `#[serde(other)]` is internally/adjacently-tagged only — it does not compile on a bare string enum) + `OrdinalPermutation`; `adopt_regen_bodies` now refreshes `geom_stamp` in place (derived, not user-authored — otherwise in-memory disagrees with what a save writes). NEW `src/document_runtime/ordinal_tripwire.rs` evaluated in `commit_snapshot` BEFORE `self.regen = scratch` (the mirror is the only source both seeded from the persisted registry at open and refreshed every commit; `document.bodies` adoption is insert-only). **DESIGN DEVIATION (approved-plan said "nearest by L2 on the quantized tuple")**: that provably fails the plan's own prescribed case — when the two pieces swap sizes the volume VALUES merely change owner, so nearest-key pairs each ordinal with itself and reports identity (worked the arithmetic on 8500/9500: identity cost < swap cost under both raw and spread-normalized L2). Volume is the component that DEFINES the ordinal, so matching on it is circular. Shipped instead: the ordinal→**positional rank** map (children ranked by quantized centroid, the lineage proxy) must not change — no metric, no normalization, no thresholds, and independent of edit magnitude. **SELF-HEAL without a command inverse** (W4 handoff flag): `commit_snapshot` is not an `EditCommand`, so each gate carries the `OrdinalAnchor` (op + pre-flip `k→rankKey`) it was raised against and every later regen re-tests against THAT, not against the previous regen — a gate stays raised across intervening regens, undo/revert restores the anchored ordering and the tripwire drops its own seeds, and closing it via the repair flow destroys the anchor so the new ordering silently becomes the baseline (no re-seed, document never bricked). No-claim cases (never a guess): unstamped child, changed ordinal SET, coincident centroids, op no longer splitting. Seeding reuses W4's `gate_items_for_moved` (now `pub`) so the edit gate and the tripwire can never disagree about what stands on a body. RED-FIRST (captured by neutering `evaluate`): `seeds=0  bodies=3 (healthy=3)  :0 y-span=(23.0, 40.0) (was (0.0, 17.0))` — the mirror EXECUTED on the other solid, silently. SCHEMA §7.2 additive `rankKey` (normative quantization + no-claim rule) · §9 fourth `reason` token + normative tolerate-unknown rule · §14 entry; sign-off recorded here; **NO fixture bump** (ordinals byte-identical; the NDJSON fixtures on both tracks are SUBSET matchers so the extra key is tolerated by construction — verified in `worker/tools/harness/main.cpp:151`). NEW `tests/ordinal_tripwire.rs` (4 real-worker) + 7 detection units + 4 core serde/stamp pins + 8 new ctest checks in `test_wp6_split`. Suites (agent-run): ctest **87/87** · cargo **761/0** vs real worker (+16) · FE **2693/2693 (180 files)** · fmt/clippy clean · hex gate empty. Flags: (a) gate LIFT takes two regens — the ceiling is sampled in `begin_regen` before the commit that clears it, so the healing regen still stops below the gated step and the published `repair_summary` still counts it while `repair_items()` is already empty (benign, self-correcting, documented on `apply_ordinal_tripwire`); (b) deliberate eager direction — children that genuinely traded POSITIONS while keeping their ordinals report a spurious permutation (H5-B bar prefers it to a silent mis-bind; the repair flow closes it permanently); (c) coincident quantized centroids (nested/shelled children) = blind spot, no claim by design; (d) FE deviation: added one `reasonText` arm in `RepairPanel.tsx` so the panel does not show the raw `ordinal-permutation` token (brief said no FE changes — the panel IS generic over `reason`, nothing breaks, this is presentation only).
- [x] **W5 GATE (2026-08-03)**: split-ordinal identity tripwire (VF-B6). Worker: `ranked_solids` returns `{shape, RankKey[i64;5]}` (key computed once, SAME quantization+stable_sort — ordinal assignment byte-identical, ctest determinism pins unmodified green); `BodyEvent` optional `rankKey` emitted on boolean split children + single-solid modified; deliberately NOT on TransformOp-copy (ordinal = target-list index, lineage) nor ImportOp (content-addressed, unpermutable) — SCHEMA §7.2 'absence = no claim' + §14 additive entry, NO fixture bump (ndjson harness is a subset matcher by construction, `harness/main.cpp:151`). Rust: `PlanStepEvent.body_rank_keys` SIDE MAP (widening persisted `BodyLifecycleEvent` would move every registry log's bytes — agent deviation, verified right); `BodyMeta.geom_stamp: Option<[i64;5]>` (Eq kept); tripwire at `commit_snapshot` BEFORE the scratch overwrite reading `self.regen.bodies` (the one source that survives reopen AND refreshes per commit — documented). DETECTION DEVIATION (agent-proven, orchestrator-verified): plan's nearest-rank-key matching is CIRCULAR (volume defines the ordinal; on the plan's own 8500/9500 swap it reports identity) — shipped ordinal→centroid-rank correspondence instead (no metric, no thresholds). Self-heal: each gate carries the `OrdinalAnchor` (op + pre-flip k→rankKey) it was raised against and every regen re-tests vs THAT (consecutive-regen compare would self-clear next tick); repair-flow close destroys the anchor = new baseline, doc can't brick; regen-side seeds carry NO command inverse BY DESIGN (stamps-derived state; undo re-runs regen ⇒ anchor matches ⇒ tripwire clears its own seeds). `RepairReason`: hand-written Deserialize (serde(other) illegal on bare string enums) — unknown token ⇒ `Unknown` never an open failure; +`OrdinalPermutation`; `ordinal_anchor` skip-if-None (legacy items byte-identical). Red-first: ':0 y-span=(23.0,40.0) (was (0.0,17.0))' — mirror EXECUTED on the swapped solid, zero seeds. RepairPanel one string arm (reason text). Residuals (documented+pinned): gate lift takes TWO regens (ceiling sampled in begin_regen pre-clear); eager direction (position-traders w/ stable ordinals = spurious gate — H5-B-preferred); coincident quantized centroids = no claim. Suites (orchestrator-run): ctest 87/87 · cargo 761/0 vs real worker (+16) · FE 2693/180 · hex clean · probes: growth linear (7.0 KB @ 24 ops), ckpts ≤2, autosave 30 ms · e2e full 156/157 + auto-mode solo-green rerun (the documented pre-existing contention class: different 1-2 specs per full run, 45 s timeouts, solo-green — hole.spec 8/8 in-run). GATE RERUN FOUND+FIXED a pre-existing product bug: chip clusters rendered UNDER the z-20 side panels near the right edge (unclickable for users on ANY armed tool; e2e hole.spec:226 caught it — fails at baseline 596d30e too, NOT a WP-FIX regression) — dedicated z-30 chip layer in ViewportEngine (sketch glyphs deliberately stay below panels); why identical bytes passed the morning run is UNRESOLVED (sub-pixel projection drift suspected), fix removes the class either way.
- [x] **VF-B1 colored STEP invisible on real lane** (FIXED W1): `onecad-protocol/src/mesh.rs:41` `FLAGS_KNOWN_MASK` omits `HAS_FACE_COLORS` 0x0010 which `worker/src/tess/Mesh1.cpp:182` sets → `validate_mesh_blob` rejects every colored mesh → `get_mesh` None → body never renders. Untested seam: ctest color gate is C++-side, FE/e2e are mock-lane. ONE-LINE FIX + real-lane colored-mesh regression test.
- [x] **VF-B2 cross-document regen commit** (FIXED W1): `PreparedRegen.expected` = (revision, epoch) only, no DocumentId; `api/mod.rs:62` `new_document` swaps the runtime slot without cancelling the in-flight job; fresh `FencingCell::new(1)` collides with the old doc's initial tokens → `lib.rs:134` phase-3 commits doc A's scratch/bodies INTO doc B. Fix: DocumentId (or generation counter) in the fencing tuple.
- [x] **VF-B3 reopen→append regen wedge** (FIXED W2): chain of three — (a) `load_checkpoints` (`document_runtime.rs:1466`) adopts `occt_fingerprint` FROM the loaded envelope so the §7.7 compat gate is self-satisfying; (b) worker restore is in-session-map-only (`Session.cpp:269`, artifacts in the request are ignored) so a fresh process always reports restored:false; (c) `RetryFromZero` (`executor.rs:388-397`) reuses the incremental plan's `AdoptingEngine` whose `known_ops` (`document_runtime.rs:755`) excludes pre-checkpoint ops → every pre-checkpoint NewBody `created` = "D1 malformation" → EngineFailed. Net: save(checkpoint)→reopen→ADD A FEATURE = regen hard-fails, and retries repeat the loop. ZERO test coverage of reopen+append vs real worker (durability test never regens after reopen; restart drill forces from:0). Fix: rebuild known_ops for the from-zero attempt (small); decide fingerprint re-adoption + artifact-restore separately (ties into plan F1b dropping container checkpoints — which ALSO kills this whole class).
- [x] **VF-B4 epoch desync with no doc open** (FIXED W1): restart hook (`state.rs:231`) no-ops when the runtime slot is None; `FencingCell::new(1)` hardcodes epoch 1 — worker crash on the start screen ⇒ every later document's plans carry epoch 1 vs worker head 2 ⇒ PROTOCOL_ERROR forever. Also `restore_checkpoint_args` uses manager epoch while `execute_plan_args` uses the cell — two verbs can disagree in one plan.
- [x] **VF-B5 transform edit-safety gate holes (H5-B class)** (FIXED W4): (a) `record.rs:372` Sketch derives NO inputs, so `transform_gate_items` (`session.rs:1511`) can't see host-face sketches — edit a Move under a face-hosted sketch ⇒ downstream cut lands offset, ZERO NeedsRepair (the exact silent-wrong-geometry the gate exists to kill); (b) `remove_operation` (`session.rs:710` area) never calls `seed_transform_gate` — DELETING a TransformBody bypasses the gate that suppressing it triggers; (c) `repair.rs:70` seeded `step_index` is positional and never remapped on insert/delete — gate silently evaporates (delete before it) or truncates the wrong step (insert before it); persisted in document.json.
- [x] **VF-B6 split-child ordinal is a geometric rank** (FIXED W5 — rank-change tripwire, NOT lineage matching; worker publishes its sort key, Rust detects the ordinal↔position permutation at `commit_snapshot` and seeds deterministic `OrdinalPermutation` NeedsRepair on downstream refs): `OpCommon.cpp:247-296` sorts children by (quantized volume, centroid, faceCount) and mints `body_<op>:<k>` by rank — a param edit that flips two pieces' sizes swaps their identities; every downstream ref re-resolves cleanly to the WRONG solid, no NeedsRepair. Design-level (SCHEMA §2 only promises within-run determinism); needs lineage-based child matching or at minimum a rank-change NeedsRepair tripwire.
- [x] **VF-B7 save/autosave under the single-writer lock + unbounded growth** (FIXED: growth W2, off-lock W3; = plan F1a/F1b): `autosave.rs:103` holds the runtime lock through `write_autosave` (whole checkpoint cache re-serialized as JSON int-arrays ~4× BREP + every import blob re-DEFLATED, `container.rs:13`); `save_document` (`api/mod.rs:206`) additionally holds it through the `SaveCheckpoint` worker round-trip (hung worker = 10 s global stall). Checkpoints never evict (`checkpoint.rs:229`), orphans survive undo (probe: 12/12 retained).
- [ ] VF-M1 poison circuit unclearable: key = base_hash|record_id|fingerprint excludes the op's own params (`manager.rs:1144`) — fixing the crashing fillet radius can never close the circuit; no reset on restart.
- [x] VF-M2 (FIXED W1) `take_checkpoint_at_head` after a worker restart (before replay completes) saves an empty-prefix checkpoint OVER the valid one (`document_runtime.rs:1385`, `checkpoint.rs` insert overwrites) — silent acceleration loss + corrupt container cache.
- [x] VF-M3 (FIXED H4) `AcquireElementIds` stale-snapshot fallback promotes nearest-centroid with NO score/margin/NeedsRepair (`ElementIdentity.cpp:50-66`; `snapshotId` request field ignored) — a pick against an already-regenerated snapshot mints a persistent id for an arbitrary face. Fixed on BOTH halves: Rust `gate_stale_pick` + worker `REF_UNRESOLVED` on a non-head `snapshotId`, plus a proportional anchor-fallback veto (`1.0·descriptor.size + 1 mm`, NOT the 0.85 descriptor gate).
- [x] VF-M4 (FIXED H4) promoted-ElementId cache (`document_runtime.rs:2024`) keyed (body, topoKey) never invalidated by regen — returns a WRONG id (not just missing) after ordinal renumber; blast radius: face-dblclick re-entry, sketch-on-face arming, selection stamps, worker double-mint (one face, two ids). Supersedes the TODO.md:20 process-death seam (in-session aliasing is the sharper half). Re-keyed `(SnapshotId,BodyId,TopoKey)→PromotionEntry`, 2-generation prune, descriptor-pinned cross-generation reuse.
- [ ] VF-M5 from-0 replay ladder scores stale WORLD anchors (`Scoring.cpp:82-86`, localFrame migrated but unread) — congruent-decoy binds at the old anchor position after a translating edit (H5-B residual; incremental path immune, so behavior differs by checkpoint availability). Plus 1 mm anchor-scale floor makes sub-2 mm parts NeedsRepair-by-default.
- [x] VF-M6 imports — **ALL THREE FIXED 2026-08-08**, see § VF-M5+VF-M6 DEFECT FIXES. This box was
      never ticked and read as an open data-integrity defect for six days; re-verified against the
      live tree 2026-08-14. Converted-blob cap `src-tauri/src/imports.rs:369` (rejects before the
      record is authored, so the "permanently unsaveable document" cannot be created); PID-aware
      sweep `:266` via `is_process_alive` (`libc::kill(pid,0)`, and non-unix treats unknown pids as
      ALIVE so it never guesses); `materialize` `:183` gates on `path.is_file()`, so a $TMPDIR purge
      re-creates the blob instead of arming the job against a missing file. Original finding text:
      converted-blob size never checked pre-authoring; 7-day sweep ignored the pid it embeds;
      `materialize` did a bookkeeping check rather than `is_file`.
- [ ] VF-M7 `HoleOp.cpp:106` (and `ShellOp.cpp:89`) apply a partition entry's topoKey ordinal to the TARGET body without checking `entry->body_id` — cross-body elementId binds the Nth face of the wrong map silently (`FaceProjection.cpp:96` does it right).
- [x] VF-M8 (FIXED W4) undo of a multi-transform cascade suppression restores repair state in the wrong order (`undo.rs:174` .rev() vs `session.rs:803` outward wrap) — first gate's seeds survive undo, regen stays truncated until a bogus manual repair.
- [ ] Minors (agent transcripts, fold into adjacent fixes): malformed-frame worker teardown deferred to ping (~10 s window); 1 GiB per-frame reserve on header trust; `existing` set always empty in AdoptingEngine (D1 uniqueness vs base inert); checkpointId minted differently Rust vs worker; ladder margin two conventions + Hungarian-assigned candidate absent from evidence; split/merged events bypass D1 validation (unreachable today); interner silent no-op on lock poison; Hole depth (0,1e-3) session-accepts/worker-rejects + no cbDepth<depth cross-check; settingsStore no quota guard.

### Clean (verified, don't re-audit): OCW1 framing codec (bounds/overflow/partial-read/EOF all correct), client correlation + restart isolation, MESH1 section-table validation (except VF-B1 mask), planner hash discipline, AcceptPrepared adoption fencing, interner bijection, countersink frustum math re-derived exact, T∘R order, container import read path + refcount, hole re-edit whole-params, fold gate seeding, worker stdout hygiene in all reviewed files. Suites reproduce exactly; soak shows no leaks.

## FILLET-CHAMFER-UNIFY — one direction-driven edge tool, Shapr3D parity (plan `~/.claude/plans/act-as-senior-software-peppy-quilt.md`, approved 2026-08-02; internal adversarial review REVISE → 1 BLOCKER + 6 MAJOR + 6 MINOR + 2 NOTE all folded pre-approval)
- [x] **W0 GATE (2026-08-02)**: pure direction math — NEW `src/tools/preview/edgeDirection.ts` (bisector/bbox tiers reconstructed from MESH1: FE has no edge→face topology, `HAS_FACE_BBOXES` never emitted by real worker; T1 = sum of adjacent flat face normals, sign-correct convex AND concave; T2 bbox proxy convex-only; `averageOutward` refuses a disagreeing mean |Σ|<0.2·N); `filletRadius.ts` additive signed lane (`EDGE_OP_FLIP_HOLD` 0.25 > `EDGE_OP_MIN_VALUE` 0.1 deliberately, `screenDragAxis` w/ edge-tangent removal + 3px collapse floor, `signedValueFromDrag` — `radiusFromDrag` byte-identical, shell untouched); `ViewportEngine.projectPoint` (w>0 valid off-frustum). Cylinder cap-rim test pins the HONEST T1→T2 degradation (smooth side-face normals fail the tangent-plane test ⇒ bbox tier). Suites: tsc 0 · FE 2100/154.
- [x] **W1 GATE (2026-08-02)**: FSM owns kind — `FilletFsm {edgeOp, auto, touched}`, `drag{signed}`, `setEdgeOp` (kills auto; pristine reseed via `touched`), `autoEdgeOp` hysteresis mirror of `autoBooleanMode` (flip only past 0.25 band — never strobes the preview session); locked type projects the drag onto its own HALF-LINE (no V-bounce). Controller: `edgeOpKind` field → FSM getter; outward computed ONCE at arm (cap 8 edges), screen axis per grab (finite difference through `projectPoint`, guards for mock engines); **auto ONLY on the bisector tier** (review BLOCKER: bbox proxy points INTO material on a concave edge — silent wrong op); `applyEdgeOpKindChange` = the ONE flip point: `++armGen` BEFORE close+reopen (two hysteresis crossings inside one in-flight `beginPreview` would install a stale-kind session + leak — race-tested with deferred-promise client), re-edit gate (`!filletEditFeatureId` — PreviewOp on current head double-applies), mid-drag 80ms trailing restore. `[Fillet|Chamfer]` chip segments beside the boolean ones (shell chip shares the branch, gets none). NEGATIVE-CHECKED: armGen bump off ⇒ only race test red; bisector guard off ⇒ only tier test red. DISCOVERY: mock e2e lane is bisector-LIVE (`MeshIngest.reconcile` loads `body1`'s box under plain `?vpdebug`) — real drag-direction e2e exists, no `?vpdemo` needed. Standalone chamfer tool drag re-semanticized (into-body grows) for its one remaining wave. Suites: tsc 0 · FE 2122/156 · e2e 108/108 (chamfer.spec UNMODIFIED green).
- [x] **W2 GATE (2026-08-02)**: `chamfer` tool id + `H` DEAD — `ModelTool` union, toolbar entry (label → "Fillet / Chamfer"), `MODEL_ONLY`, keymap binding all removed; global `h → home` un-shadowed in model mode (POSITIVE keymap pin, not a discovery); chamfer ICON kept (history rows). **Teardown fence** (review F5, pre-existing leak): `editEdgeOpFeature` now `invalidateArm()` + `cancelFillet()` AFTER `setTool` — `setTool("fillet")` is a no-op when fillet already armed, so the prior arm's in-flight `beginPreview` used to land on the re-edit state; regression test holds the promise open across the re-edit and asserts teardown-not-install. InspectorPanel needed NOTHING (routes on wire opType already). e2e: `chamfer.spec.ts` + `fillet-direction.spec.ts` → `filletChamfer.spec.ts` (9: one button, h inert, drag-flip both ways, segment lock, release stays armed, ✓ both labels, Enter/✕). Suites: tsc 0 · FE 2126/156 · e2e 110/110 (first-boot toolbar-visibility flake seen once, solo+rerun green).
- [x] **W3 GATE (2026-08-02)**: re-edit TYPE FLIP — core `op_type_edit_allowed` widens `update_operation_params`' same-opType guard to the ONE sanctioned **Fillet⇄Chamfer** pair, ENUM-VARIANT matched (a string list would admit Known⇄Opaque where `validate_temporal` is vacuous); params field-identical + shared lockstep + worker dispatches on opType alone + `Inverse::RestoreRecord` makes undo exact; legacy `mode` string normalized IN CORE on the cross-type branch (`{"opType":"Chamfer","mode":"Fillet"}` unrepresentable through this path; never invented on documents that had none). FE: re-edit chip promoted to full armed cluster w/ segments (`touched:true` — committed size survives a segment click; NECESSARY not cosmetic: `DimensionInput.commit` only fires on a CHANGED number, so a pure type flip could never commit from the bare chip); flip = chip + record only (no preview reopen); commit sends possibly-flipped opType through existing `updateScalarParamsCommand`. Mock mirrors the allow-list rejection + reflects sanctioned flips into `opType`+`label`. **Real-worker proof** (`fillet_reedit_swaps_to_chamfer_and_regens`): base=20000, fillet(r=2)=19976.537, swap→chamfer=19950.000 — removed-volume ratio 2.131 (analytic 2.33, floor 1.8, ambiguity = HARD FAIL; never asserts record opType — tautology — nor label — fixture carries `name`); undo restores fillet volume; zero needsRepair both regens. Suites (orchestrator-verified): ctest 79/79 · cargo workspace 600/0 vs real worker + edit_session 34/34 + topology_rebind 7/7 re-run standalone · tsc 0 · FE 2135/156 · e2e filletChamfer 10/10 + full 111/111 · hex 1 pre-existing (inputProbe). SCHEMA untouched (no wire shape moved).
  Flags: in-type updates can still carry a contradictory legacy `mode` (pre-existing hole, worker ignores it); user-RENAMED rows keep `rec.name` so their label won't flip after a swap (default-labeled rows follow); mock opType guard tolerant of legacy rows without `opType`; `boolean-preview.spec.ts` flaked once in the W3 full run (extrude-handle hit-scan timing class, both pass isolated — pre-existing family); **workspace-wide `cargo fmt`/`clippy` at gate time reflect the CONCURRENT DEV-OBSERVABILITY session's in-flight edits** (fmt diff + `DrivenRegen` pattern errors are in THAT wave's files — `api/mod.rs log_event`, `document_runtime.rs`; this WP's crates compiled + tested clean before those edits landed and touch none of the same paths). Backlog: 2-distance chamfer / chordal / G2 (SCHEMA+worker); drag-arrow gizmo at edge midpoint; typed-negative never flips (matches host-boolean, open decision there too).
- [ ] USER manual Tauri gate (FILLET-CHAMFER-UNIFY): box → pick edge → F → drag AWAY = rounded preview + chip "Fillet"; drag INTO = bevel + chip flips "Chamfer" live; segment click locks type (later opposite drag must NOT flip); ✓ commits correct label + icon; concave pocket edge → auto OFF (chips control type — axis honesty); cylinder cap edge → same; dblclick committed row → segments show committed type → flip → Enter → SAME row swaps label + geometry changes; ⌘Z restores type AND geometry; radius-too-large → ✓ blocked with named OCCT reason, edges kept; `h` = Home view now.

## DEV-OBSERVABILITY — unified JSONL dev logging + debugging, Claude-Code-optimized (plan `~/.claude/plans/act-as-senior-rust-smooth-puzzle.md`, approved 2026-08-02; internal adversarial review REVISE → 4 BLOCKER + 7 MAJOR folded pre-approval)
- [x] **Wave R GATE (2026-08-02)**: `logging.rs` layered subscriber (stderr fmt + JSONL `logs/dev.jsonl` via `tracing_appender::non_blocking`, truncate-per-session, `session.start` first line, shared `EnvFilter`/`RUST_LOG`, `DEFAULT_FILTER` pins `fe=debug,worker=debug`), tauri `tracing` feature baseline (free `ipc::request::handler` span on every command) + 6 hand-`#[instrument]`ed domain commands (open_document/save_document/apply_edit_command/enter_sketch/finish_sketch/add_sketch_on_face), `log_event` FE bridge (`FeLogEvent`→`target:"fe"`, ctx capped 2048B), regen `job/rev/epoch/base/steps` threading + `regen.drive` span + `elapsed_ms` terminal + Superseded/downgrade warns (`original_error` preserved) + `regen:` postmortem moved into `finish_regen` (fires on every outcome incl. failed-step warns), worker stderr piped+forwarded (`target:"worker"`, `epoch`, level-sniffed from WLOG prefix, 8KiB line cap), lifecycle (`Ready`/`Restarting`/`Failed`/`CircuitOpen` w/ `last_crash`) + poison-msg logging, `onecad-protocol` `tracing` feature + `onecad_protocol::frames` tx/rx trace (debug-gated), panic hook (`target:"panic"`), regen-CLI stderr subscriber, `error.rs`/`wire.rs` error-context logs (`StalePreview` demoted to debug). `cargo test --workspace` green vs real worker, `cargo clippy --workspace --all-targets -- -D warnings` + `cargo fmt --all --check` clean, `onecad-protocol` feature-matrix (no-default/client/client+tracing/tracing-only) clean.
- [x] **Wave C GATE (2026-08-02)**: `Log.h` `Level{Error=0,Warn,Info,Debug}` + atomic min-level, `enabled()` short-circuits BEFORE mutex/format, `init_level_from_env()` (`ONECAD_WORKER_LOG`, case-insens, unknown→info+one warn) called as literal first statement of `main()` (covers `--selftest`); `WLOG_DEBUG`. Dispatcher::execute: steady_clock id+elapsed on success (debug) + both catch paths (error) + unknown-verb branch (error, previously silent). NEW `scripts/check-worker-stdout-hygiene.sh` — grep gate for printf/puts/putchar/std::cout/fprintf(stdout,...) scoped to skip `snprintf`/`fprintf(stderr,...)` (word-boundary) + full-line comments; wired into `build-worker.sh` (before cmake) AND as its own CI `worker`-job step (that job runs raw cmake, not the script) + a debug-level `--selftest` smoke step. Verified: gate passes on tree, dry-run on a scratch copy catches all 4 violation forms (printf/std::cout/fprintf(stdout,…)/puts) with zero false positives on snprintf/fprintf(stderr,…)/comment mentions; `ctest` 79/79; selftest under debug shows a `WLOG_DEBUG "verb '...' ok in N ms"` line; garbage env value warns once + still exits 0.
- [x] **Wave F GATE (2026-08-02)**: `src/debug/log.ts` structured logger (dependency-free core, `(DEV || ?trace) && !vitest` gate, ring 2000 head-index, ctx capped at entry, `window.__logs`/`__logsDump()`/`__logsClear()`, `trace.ts` back-compat shim), `logSink.ts` batched forwarding (`invoke` direct from `@tauri-apps/api/core`, 50/250ms flush, 3-failure session disable, `sink` tag never forwarded), `ErrorBoundary.tsx` (componentDidCatch→`err`) + `main.tsx` `error`/`unhandledrejection` listeners, `tauriClient.ts` `call<T>` IPC chokepoint (`ipc` tag, `IPC_LOG_EXEMPT={solve_drag,log_event}`), `statusHint`→`hint` tag (90+ callers, one funnel) / `workerStore`→`worker` tag / `repairStore`→`repair` tag, `fsmLog.ts` `withPhaseLog` (phase-change-only `fsm` events, 8 model-tool reducers wrapped at the import alias), 10-site console migration (ViewportEngine/ViewportRoot/meshRegistry/meshSync/SketchController×4/tauriClient/sketchWireMap), DELETE `src/viewport/debug/inputProbe.ts` + `?inputprobe` wiring. FE vitest green, `bun run build` clean, hex gate (`grep -rn '#[0-9a-fA-F]\{6\}' src`) EMPTY, e2e 111/111.
- [x] **Wave E GATE (2026-08-02)**: NEW `e2e/fixtures.ts` — `auto:true` fixture wrapping the built-in `page` fixture, buffers `console`/`pageerror`, on fail-or-any-pageerror writes `console.log`/`pageerror.log`/`fe-logs.json` (`__logsDump()`) via `fs.writeFile(testInfo.outputPath(name))` + direct `testInfo.attachments.push({path})` — NOTE: `testInfo.attach(name,{body})` only keeps an in-memory Buffer under this project's `"list"`/`"line"` reporter (`playwright/lib/util.js normalizeAndSaveAttachment` only writes to disk for `{path}`), caught + fixed via a forced-failure dry run before landing. Import rewrite `@playwright/test`→`./fixtures` across all 45 `*.spec.ts` (24 single-line, 21 split to preserve `import type { Page }`), `helpers.ts`/`modelToolHelpers.ts` excluded (legit `@playwright/test` type-only users). CLAUDE.md `## Debugging & logs` (~15-line dense section, placed after Testing conventions). NEW `docs/DEBUGGING.md` (lane locations, full JSONL schema w/ 4 worked examples incl. the fe-lane double-JSON-string `ctx` gotcha and the worker-lane dual-timestamp gotcha, tag taxonomy table incl. which "taxonomy" tags have zero live call sites, enabling matrix, correlation glossary, worker-lane no-span caveat, jq+grep/python cookbook, failure-signature table, white-box surfaces, extension policy, known limitations). Verified: `bunx playwright test --list` 111 tests/45 files (stable across the whole wave); 2 full-suite runs, each with a DIFFERENT 1-2 spec flake (`theme.spec.ts:159`; `tree-rename.spec.ts:107` + `tree-visibility.spec.ts:131`) — none touch this change's own files, all 45.0s timeouts (full-suite WebGL/webserver contention class), all solo-green on immediate rerun — pre-existing flake, not a regression. Forced-failure dry run on `point.spec.ts` proved all 3 artifacts land as REAL FILES (not just terminal-inline) with real content (fe-logs.json showed genuine `vp`/`fsm`/`hint` tagged events) — caught the `{body}`-vs-`{path}` attachment bug this way BEFORE it shipped — then cleanly reverted (`git diff` = import line only).
- [ ] **GATE**: all 4 suites green + hex clean + manual smoke (dev.jsonl cross-lane correlation, worker-kill lifecycle lines, mock-lane no-op)

## SAVE/OPEN-RENDER — reopen showed tree but never rendered bodies (plan `~/.claude/plans/act-as-senior-software-greedy-plum.md`, approved 2026-08-02)
- [x] **GATE (2026-08-02)**: W0 diagnosis — backend EXONERATED empirically: `onecad-regen` CLI replay of the user's real `Untitled.onecad` + all 8 autosave containers vs the real worker = 8/8 `published`, 0 failed steps (incl. the sketch-on-face host-boolean Cut doc); new `src-tauri/tests/open_render.rs` drives the EXACT `open_document` chain (real save w/ checkpoint-at-head → FRESH worker → `DocumentRuntime::open` → production scheduler/driver `ToEnd{from:0}` → emitter) and proves pre-regen projection lists the saved body, pre-publish `get_mesh` is a documented miss, the open-regen's `document-changed` names the body, post-publish `get_mesh` serves MESH1 (magic-checked) — the previously UNTESTED app-glue layer. Defect class = FE mesh lane failing SILENTLY at every link. W1 fixes (all four confirmed real): (1) `ensureEvents` boolean→memoized PROMISE, eager at client creation, AWAITED **before** invoking open/new/import/recover (the pre-regen projection fires DURING the round-trip; callbacks used to register while `listen()` was still pending — events in that window were lost forever, and a lost mesh event = a body that never renders); (2) `MeshIngest.reconcile()` — every applied projection loads store-visible handle-less bodies (`pending` in-flight guard) + drops handles gone from the store; closes the exact wedge: first fetch hits the pre-publish window (empty), `document-changed` missed, post-regen projection re-applies the SAME visible flag ⇒ old per-id diff saw nothing and the body stayed invisible FOREVER; (3) `loadBody` try/catch — fetch/parse/build failure → statusHint error + console.error, per-body (others keep loading); `get_mesh` miss now trace-logged + bounded self-retry 3×300ms (no unbounded polling — negative-checked: retries stop at 4 calls); (4) cross-document revision guard — additive `documentId` on `DocumentProjection` (Rust `projection()` + dto; `protocol/SCHEMA.md` untouched, zero worker frames), hydration resets-then-applies on documentId change and compares revisions only WITHIN one (a new runtime restarts at rev 1, so the old cross-doc compare dropped every projection of an in-session reopen while the store held the old doc's high revision; missing id = mock-lane same-doc semantics, no mock parity shim). W2 visibility — uncorrelated regen failure (no commit awaiter: open-replay, restart-replay) now raises a STICKY error hint naming the reason (was: tinted history row only); `take_checkpoint_at_head` skip (head not fully regenerated at save) now logged. Suites: cargo workspace 48/48 result-blocks green vs real worker (incl. `open_render`) · ctest 79/79 · e2e 100/100 · FE 2072/2073 + tsc/build/clippy/fmt/hex clean.
  Flags: FE 2073rd = `ModelTreePanel` "refuses an empty name" — PRE-EXISTING parallel-run flake, REPRODUCED AT CLEAN HEAD 55454d0 in an isolated worktree (passes solo, fails under multi-file load; mock-lane rename straggler suspected, untouched by this WP — same stale-async-write family, worth its own look). W3 (persist coarse meshes in the container at save; serve from `get_mesh` pre-publish for instant render-on-open — format + `ContainerCaches` seam already exist) DEFERRED to backlog by scope decision. USER manual Tauri gate below.
- [ ] USER manual gate: `bun run tauri dev` → fresh-boot open `Untitled.onecad` → body renders + fitView; in-session File>Open a DIFFERENT doc (cross-doc guard case); reopen ×2 round-trip; optional worker-missing run → tree intact + visible error hint, no silent blank.

## DARK-MODE — Light/Dark/System appearance, chrome + live viewport re-theme (plan `~/.claude/plans/act-as-senior-software-transient-puddle.md`, approved 2026-08-02)
- [x] **GATE (2026-08-02)**: W0 foundation — `src/theme/themes.ts` registry mirroring `renderModes.ts` (`ThemePref` light/dark/system vs `ResolvedTheme` light/dark, `coerceTheme`, `resolveTheme`), `themeController.ts` as the SOLE writer of `data-theme`+`color-scheme` on `<html>` (one cached `matchMedia`, idempotent start, listeners fire ONLY on a real resolved-value change because each notification costs a PMREM rebuild); `settingsStore` v4→v5 persisting the PREFERENCE only (resolved value derived at runtime — the `navigation.inputDevice` precedent), coerced in `migrate` AND `merge`; `index.html` inline no-FOUC stamp before first paint + `<meta name="color-scheme">`; **jsdom `matchMedia` stub in `src/test/setup.ts`** (jsdom implements none — hard blocker for every theme test). W1 de-hardcode — 27 sites across 21 files swept to new tokens (`bg-white`→`bg-surface`, `text-white`→`text-on-accent`, `bg-black/40`→`bg-scrim`), both hatch gradients + the `?vpdebug` label tokenized; light values chosen so the wave is a visual no-op. W2 dark palette — ~60 authored values in `:root[data-theme="dark"]` (prototype has none; annotated in-file), Appearance `SegmentedToggle` folded into `DisplayModePopover` rather than a 4th cluster button. **Shadow trap FOUND + closed**: Tailwind v4 INLINES `--shadow-*` at build time (`.shadow-ctrl{--tw-shadow:0 1px 3px var(--tw-shadow-color,#0000000f)}`) so overriding those tokens is a NO-OP — verified against `dist/assets/index-*.css`; dark shadows go through `--tw-shadow-color` via a UNIVERSAL selector, required because Tailwind registers it `@property{syntax:"*";inherits:false}`. W3 live re-theme — theme-keyed `FALLBACK` table in `palette.ts` (jsdom loads no CSS, so without this dark is invisible to vitest and no `refreshColors` could be tested at all), `ViewportEngine.applyTheme()` with load-bearing order (clear color → light levels → `buildEnvironment()` → 11 layer refreshes → invalidate; the environment MUST rebuild after the clear color because PMREM fills uncovered directions with it), `refreshColors()` on 11 layers + `setColors()` geometry rebuilds on `GridPlane` (baked vertex fade) and `OriginTriad` (baked axis colors), `BodyMaterialLibrary.refreshColors()` preserving an active Cut tint while unconditionally re-reading the edge, `MeshIngest.refreshColors()`, single-orchestrator wiring in `ViewportRoot` (the two material-library owners have no common parent and independent subscribers would race the cache-drop ordering), `theme`+`clearColor` added to `debugSnapshot()`. W4 tuning — `LIGHT_LEVELS` becomes a theme × backend table (the hemisphere GROUND half is the canvas token, so dark must lower it or undersides muddy), `--color-body-edge` INVERTED (wireframe draws edges with no faces behind them — near-black on a dark canvas is invisible). W5 — `getCurrentWindow().setTheme()` guarded on `__TAURI_INTERNALS__` + `core:window:allow-set-theme` (macOS draws the traffic lights and they follow the OS, not our CSS), title-bar `ThemeToggle` cycling the preference in registry order (a cycler is the wrong control for picking one of N — which is why the display button stopped being one — but this is a SHORTCUT, not the picker; the accessible name always states the current value so the changing icon is never ambiguous), 3 authored `themeLight`/`themeDark`/`themeSystem` glyphs (same not-from-prototype precedent as `datum`/`measure`/`isolate`), `e2e/theme.spec.ts` 8 cases + `theme-boot.spec.ts` 3, all asserting the LIVE engine clear color rather than the store. **Palette cache re-keyed BY THEME** after a user-reported light viewport: a flat cache is correct only if `resetPaletteCache()` fired at exactly the right moment, so anything reading a color earlier (engine mid-construction, a module evaluated before the theme was stamped) poisoned it silently — a read taken while the document says dark can no longer return a light color, and correctness no longer depends on the reset having run. Suites: FE 2085/153 (+73/+4: `themes` 15, `themeController` 13, `themeRefresh` 22, `TitleBar` 5, `settingsStore` +5, `DisplayModePopover` +4) · e2e 11/11 new · tsc/hex clean. **All 4 traversal-based `themeRefresh` cases NEGATIVE-CHECKED** — neutering each `refreshColors()` turns them red.
  Flags: the first `themeRefresh` draft was VACUOUS — most layers keep shared materials off the scene graph until something draws with them, so an idle-layer traversal found zero colors and passed while proving nothing (caught by the negative control, fixed with a non-vacuity guard in the helper + real registry/mesh content per case). `--tw-shadow-color` is a Tailwind INTERNAL — the dark-shadow mechanism rests on an undocumented emission detail, verified at v4.3.3, and no test can catch a regression (jsdom never runs the Tailwind pipeline) — re-verify after any Tailwind upgrade. Dark loses per-shadow alpha differentiation (one tint for all). `--color-on-accent` goes near-black on the accent in dark: white on `#4d8bf0` is ~3.0:1 and fails AA, this is ~7:1 — legible but unfamiliar. `palette.ts`'s FALLBACK table now mirrors ~40 token values by hand across two themes with nothing enforcing the match. `cargo`/`ctest` NOT re-run this wave (only `capabilities/default.json` changed on the Rust side, and a concurrent SAVE/OPEN WP is mid-edit in `src-tauri/`).
- [ ] UNRESOLVED (user-reported 2026-08-02): dark chrome + LIGHT viewport in `tauri dev`. Not reproducible in the mock lane on either path — toggle-after-boot and cold-boot-already-dark are both green vs real WebGL, and the dark `--color-canvas` is confirmed present in the built CSS. Palette cache re-keyed by theme to remove the ordering-bug class that would explain it. Leading remaining theory is a Vite HMR artifact rather than a product bug: a CSS-only hot reload re-themes chrome instantly but `data-theme` never CHANGES, so no resolved-theme event fires and the engine's already-built materials stay stale. Needs a cold `tauri dev` restart + `document.documentElement.dataset.theme` / `getComputedStyle(document.documentElement).getPropertyValue('--color-canvas')` / `__vpEngine.debugSnapshot()` from the real webview to close.
- [ ] USER manual Tauri gate: title-bar appearance button cycles Light → Dark → System and the viewport follows on each step; toggle Light/Dark/System with a model loaded (camera pose, selection and an OPEN SKETCH SESSION all survive — no remount); toggle while in sketch mode (dimmed body + sketch overlay + snap indicator all recolor); toggle with a live extrude/revolve preview (Cut tint preserved); **all three render modes in dark — wireframe is the failure case if the edge token did not invert**; OS appearance change on "System" (native traffic lights follow); reload shows no flash of light chrome

## RENDER-MODE — display-mode registry + studio shading (continuation of MODELING-REACH W3; no plan doc — direct WP)
- [x] **GATE (2026-08-02)**: display mode becomes DATA. `renderModes.ts` registry (`RenderModeDef {faceVisible, edgeVisible, materialKind}` + `RENDER_MODE_ORDER` + `coerceRenderMode`) with the extension contract stated in-file — a new mode is a table entry plus, if it shades differently, a new `MaterialKind`; no per-mode branching in `BodyObject`, the ingest controller, or the store. `BodyObject.createBodyMaterials` DELETED → `applyMode(def)` repoints both children at `BodyMaterialLibrary.get(def.materialKind)`. **`bodyMaterials.ts` is now the sole owner of body materials**: shared per-kind sets (N bodies = one pair), dim save/restore that replays the OBSERVED prior state rather than hardcoded resets (and dims a set born while dimmed), tint COPIES never retains the shared palette `THREE.Color`; `MeshIngest`'s own ~30-line dim implementation deleted in favor of it. `displayMode` moved off `viewportStore` (session-only) to persisted `settingsStore` v3→v4, coerced in `migrate` AND in a new `merge` hook — `migrate` only fires on a version MISMATCH, so a same-version hand-edited or rolled-back blob carrying an unknown id needed the second seam. Chrome: cycling button → `DisplayModePopover` (`role="menuitemradio"` rows off the registry, labels from the same table so chrome and engine cannot disagree); `CornerCluster` gains one shared `openPopover` slot (display + snap can never both be open) and `ClusterButton` emits `aria-expanded` for popover triggers vs `aria-pressed` for plain toggles. **Shading**: `NeutralToneMapping` @ exposure 1.0 (Khronos PBR neutral over ACES so a body's albedo token still reads as itself), `RoomEnvironment`→PMREM IBL built once at `init()` + once per context restore (never in `renderFrame` — the idle-zero-rAF contract holds), `scene.environmentRotation` (π/2,0,0) rotates SAMPLING only so the Z-up invariant is untouched, camera-relative key/fill rig (`lightRig.ts`, pure math, `KEY_EL_FLOOR` 20° so a top-down camera never flattens the key, allocation-free via a new `sphericalToOffset` out-param). WebGL-only-by-CONSTRUCTION: `createEnvironment` is simply ABSENT on the WebGPU handle and on mocked handles, so callers write `handle.createEnvironment?.()` and no `isWebGPU` branch lives in the engine; backend-split intensities carry the r185 rationale (`BRDF_Lambert` divides by π ⇒ the old 0.75 headlight delivered ≈0.24×albedo). Caller owns the PMREM render target; env disposed BEFORE the renderer. Global `toneMapped:false` sweep across all 14 overlay/annotation layers — tone mapping is for lit body faces only, everything else renders its design token exactly. New authored tokens `--color-body-fill`/`--color-body-edge` (NOT from the prototype — annotated in-file); `palette.bodyNeutral`/`bodyEdge` repointed to them and a new `referenceNeutral()` added so datum/region/revolve overlays keep the old `--color-ink-5` gray. Suites: FE 2012/149 (+32/+5: `renderModes` 60, `bodyMaterials` 135, `lightRig` 122, `DisplayModePopover` 69, `settingsStore` 58) · e2e 94/94 (`view-ux` rewritten cycle-click → popover rows, asserting the LIVE scene graph via `?vpdebug` — asserting the store would prove only that a boolean flipped) · ctest 79/79 + cargo 594/0 UNTOUCHED (no `src-tauri/` or `worker/` files in the diff) · tsc/build/hex clean.
  Flags: **the IBL path has ZERO automated coverage** — `ViewportEngine.test.ts` mocks the renderer with a handle that deliberately omits `createEnvironment`, and e2e asserts `visible` flags, not pixels; the studio look is verified by the manual gate only. The WebGPU lane runs lights-only at higher intensities and is untested. PMREM fills directions `RoomEnvironment` does not cover with the renderer CLEAR COLOR, so `--color-canvas` now silently drives body shading — a future canvas-token change is a shading change. `palette.ts` still caches `THREE.Color` forever and `resetPaletteCache()` remains unwired (zero callers) — every material built at construction is frozen for the life of the engine. Two `BodyMaterialLibrary` instances exist (`MeshIngest`'s committed one, the engine's lazy preview one) and no single owner can reach both. `src/viewport/debug/inputProbe.ts:87` raw hex unchanged (PRE-EXISTING, file already marked TEMPORARY).
- [ ] USER manual Tauri gate: all three modes (Shaded / Shaded + edges / Wireframe) draw correctly and the popover checkmark tracks; studio shading reads as a machined surface across an orbit (key light follows camera, no flat/blown faces at top-down); display mode survives a reload; body dim on sketch entry restores cleanly on exit

## SKETCH-ON-FACE — face pick → sketch + projected locked boundary (plan `~/.claude/plans/act-as-senior-software-twinkly-crown.md`, approved 2026-08-01; internal adversarial review REVISE → 2 BLOCKER + 6 MAJOR + 6 MINOR all folded pre-approval)
- [x] **W0 GATE (2026-08-01)**: W0a `face_sketch_plane` topoKey rung (flagged latent; red-first vs real worker: promoted-but-unconsumed face failed `not present in the current snapshot`, rung fixes; FE forwards `face.topoKey`; first-ever tests on shipped sketch-on-face path — real `tauri::test::mock_app` command harness + 5 controller vitest cases). W0b `referenceLocked` END-TO-END, zero producers: flag on all 5 core entity variants (`skip_serializing_if` — snapshots only DROP `false` lines), `SketchError::ReferenceLocked` L1 guards (remove incl. cascade via shared `doomed_by_remove`, reposition, construction-flip), wire emits only-when-true (byte-identical existing sketches), worker reads flag + child-point inherit, **oracle's `addConstraint` veto REMOVED** (silently dropped every non-Fixed constraint naming locked geometry) — immobility moved to solver: tag-0 pins (`referenceLockPins` → CoordinateX/Y + Equal on radius/arc ANGLES — endpoint-position pinning is singular on 180° arcs; skips Fixed-held points), naive-dof subtracts pin equations, `hasInternalCouplings` blocks teleport drag path, translate skip→REFUSE (shear = silent desync), LoopDetector 4-site pins (locked PARTICIPATES in regions), FE hydration + marshal latch (locked ids stay in session+id-map; destructive-op refusal), `matReference` token, `stamp_datum_plane` HostFace no-op pinned. SCHEMA §7.3 `referenceLocked` + §7.4 narrowed + §14 SIGNED OFF (no fixture bump). Suites: ctest 78/78 (new `sketch_reference_locked` 7 cases, red-first — pin pass disabled = 6 failures) · cargo 576/0 vs real worker · FE 1907/140 · build · e2e 90/90 · clippy/fmt/hex clean.
  Flags: locked bare Point / Ellipse carry no solver pins (Point immobility = W1b explicit Fixed; Ellipse solver-free by design); `beginGroupDrag` lacks locked pre-filter (fails loudly via pins — better message deferred to producer wave); W0a-found hint-clobber (`tryEnterOnSelectedFace` rejection hint overwritten by `beginPlanePick` generic prompt) → W3.
- [x] **W1 GATE (2026-08-01)**: W1a worker `FaceBoundaryProjector` REWRITE w/ oracle algorithm parity (pure over local buffer, provenance header @ b4ddcccc; exact Line/Circle/Arc, CCW-via-D1-tangent, polyline fallback 24-seg, normative ordering: seed outer wire → holes → coplanar faces in TopExp order, points by first use, byte-identical determinism); `CoplanarFacePatch::collectCoplanarFaces` all-faces scan (NOT BFS — disconnected coplanar prongs found; U-shape ctest asserts scan 2 vs bfs 1) reusing shipped predicate; **pre-existing predicate BUG found+fixed**: `gp_Dir::Dot` via implicit `gp_Vec→gp_Dir` made the 1e-3mm distance test ANGULAR (0.05mm-off face at 100mm lateral passed) + threw `Standard_ConstructionError` on coincident origins; `ProjectFaceBoundary` verb (SCHEMA §7.6 SIGNED OFF: frameOnly plane handshake, elementId→topoKey→shape rung, plane-authoritative UV, response-local `p<N>` refs, present:false semantics, no fixture bump); `planarFacePlaneAndNormal` hoisted public (extrude lane delegates). ctest 79/79, red-first proofs (merge off = 8 fail, dedup off = entity-count fail, BFS swap = hard fail). W1b Rust: `FaceBoundaryProjection` trait seam (7th backend facet), wire args/parse, core `sketch/projection.rs` pure translation (all entities+points locked, Fixed on every projected point, ccw verbatim — worker pre-swaps to CCW), `add_sketch_on_face` (FE-minted SketchId adopted VERBATIM; RT1 frameOnly → `plane_from_point_normal` → RT2 in that basis; 1e-9 exact tripwire; resolved rung REUSED across RTs — re-laddering could bind different faces; worker IO outside lock; failure = document untouched; timeline record stays `finish_sketch`'s job). 11 real-worker tests: locked boundary + Fixed + version 1 + HostFace attachment; dof 0; 1 region; extrude lands OUTSIDE host face; **tilted 10°-draft face: origin-on-plane ≤1e-4 (f32 mesh bound) + volume = kernel-area×h to 8 sig figs**; pocket → circle + 2 regions; non-planar loud + doc untouched; Distance-to-locked unmoved; RemoveEntity → ReferenceLocked; save→fresh-reopen survival; line-across → 2 regions volumes summing. Suites: ctest 79/79 · cargo 592/0 vs real worker · FE 1907/140 · build · e2e 90/90 · clippy/fmt clean.
  Flags: `FaceExtrudeProfileBuilder` keeps its own anon-namespace coplanar scan (correct math, repoint = shipped-op behavior change, deferred); process-level verb determinism covered in-process only (pure read over head copy); `exact` tripwire ≠ fence (stale-head guard = caller's job).
- [x] **W2 GATE (2026-08-01)**: real lane — `tauriClient.enterSketch` face branch → NEW `add_sketch_on_face` command (FE-minted uuid adopted verbatim; enter_sketch + orphan compensation unchanged; `buildAddSketchOnFace`/`WireFaceRef`/hostFace-attachment arm DELETED — FE can no longer author a host-face AddSketch); mock parity — `mockFaceGeometry.ts` per-face plane+boundary DERIVED from `mockMeshes` shared corner exports (six inline literals refactored to consume them; byte-parity probe-verified pre-delete; `planeFromPointNormal` ported verbatim from plane.rs), cylinder caps circle-boundary, side face refuses; `localSolver` newOnFace seeds session through the REAL hydration path (worker-wire form → frontendEntitiesFromDto — zero duplicated rules); mock DOF rule: locked entity contributes 0, all-locked-operand constraint removes 0 (projected boundary = FullyConstrained dof 0); L3 guards — drag arm refuse + Trim refuse (controller), Delete guard in `sketchService.deleteEntitiesNow` single funnel (mixed set deletes user lines, all-locked hints), Mirror already-unlocked-copies pinned; ConstraintList VIEW-ONLY machine-Fixed filter (session.constraints intact — marshal seeded-set invariant); static layer renders locked (no per-entity styling exists — pinned); **hint-clobber FIXED early** (tryEnterOnSelectedFace returns {entered, refusal} like datum path — non-planar reason now reaches the picker prompt); autoConstrain locked-as-target pinned. e2e `sketch-on-face.spec.ts` (real raycast via new `findFaceOnBody` probePick scan; drag/Delete refusals; projected-region extrude → body; non-planar refusal) + `openEditorDebug {mockBody}` vpdemo boot. R7 re-entry pin negative-checked (latch disabled = test fails). Suites: FE 1945/142 (+38) · e2e 92/92 (+2) · tsc/build/hex clean (Rust/C++/SCHEMA untouched).
  Flags: synthesized (extrude/boolean) mock bodies keep the +Z fallback frame (MOCK LIMIT, pinned); `sketchSelectionStore` not on `__stores` (e2e asserts via hint); one pre-existing `boolean-preview` body-hit-scan e2e flake observed once, passed in isolation + clean re-run.
- [x] **W3 GATE (2026-08-01)**: the two remaining entry triggers + the DTO that makes re-entry possible. **(b) plane-pick accepts a body FACE** — resolve order datum → world quad → body face (pinned with all three stacked under ONE pointer; the datum + quads are chrome the picker itself raised, so model geometry under them must never steal the click), hover via `probePick` with the standard face tint routed through `selectionStore.setHover` (ONE writer — a direct `setHighlightState` would be clobbered by the next selection event) + a prompt that NAMES the body, **no hover-time planarity check by design** (one RPC per pointer move to grey out a face is not worth it — the click validates), refusal keeps the picker up carrying the reason (the W2 hint-clobber pattern, now a stored `planePickHint` the face-hover line swaps out and restores verbatim), `endPlanePick` drops the hover on EVERY teardown path (Esc/exit/switch/dispose all funnel through it). **(c) double-click a face in model mode** — one shared mode+tool gate with the static-sketch branch (structurally cannot diverge), sketch layer keeps priority (a face sketch lies FLUSH on its host, so it is always the more specific target), **re-entry wins over creation**: `newestSketchOnFace` matches the promoted pick's ElementId against `hostFace` and opens the newest match through the same `setMode('sketch', id)` path, else creates — and validates BEFORE flipping the mode, so a non-planar face hints with NO mode change, no chrome, no picker. Direct `controller.enterOnFace(hit)` call, not a selection mutation. ONE promotion serves both branches. **DTO (additive, internal only — `protocol/SCHEMA.md` untouched, zero worker frames moved)**: `SketchDto.hostFace {bodyId, elementId}` from `SketchAttachment::HostFace.face.primary`, `skip_serializing_if` so world/datum rows stay byte-identical; ids render exactly as `BodyDto.id` / a promoted `elementId` so a pick compares `==` with no normalization; through `types.ts` → `projectionHydration` → `SketchMeta`. Mock lane needs no parity shim — `SketchController` mints `hostFace` on the optimistic row it already fabricates `name`/`dof`/`geometryToken` on, and the mock's projection IS `documentStore` (real lane overwrites it with backend truth on the next `projection-updated`). Suites: FE 1962/143 (+17, new `SketchController.faceEntry.test.ts` 16 cases) · cargo 592/0 vs real worker · ctest 79/79 (untouched) · e2e 94/94 (+2) · tsc/build/clippy/fmt clean. Re-enter rule NEGATIVE-CHECKED (rule disabled ⇒ 2 vitest + the e2e re-entry all go red).
  Flags: "newest" = LAST in projection iteration order, and the real backend projects sketches from a `BTreeMap` keyed by SketchId ⇒ uuid-lexicographic, not creation order — with TWO sketches on the SAME face the choice is arbitrary-but-deterministic (both are valid hosts; never a wrong bind). Mock lane inserts in creation order, so it is literally newest there. A body face is NOT in the orbit gate during plane picking (same deliberate choice as datum planes) — a click resolves, a drag orbits. `probePick` on plane-pick hover is uncoalesced (matches the existing datum/quad hover cost profile; the model-mode hover path is rAF-coalesced). `src/viewport/debug/inputProbe.ts:87` carries a raw hex (`#e8e8e8`) — PRE-EXISTING since 14cbd50 (2026-07-21), in the file already marked TEMPORARY; every other `src/**/*.ts(x)` is clean.
- [x] **HOST-BOOLEAN GATE (2026-08-01, user-reported)**: modeling op off a face-hosted sketch modified nothing — every extrude spawned a NEW body. Now: fresh arm on a hostFace sketch seeds boolean Add + host body target (`BooleanSeed` on the arm event; controller `hostBooleanSeed` reads `SketchMeta.hostFace` synchronously; edit re-arm NEVER clobbered; hidden/absent host → old behavior); extrude is DIRECTION-AWARE while `booleanAuto` (drag away = Add, into = Cut, live chip+tint via existing `applyBooleanState`; depth 0 holds; symmetric never flips; any explicit chip click kills auto for the session); revolve seeds Add + target, no direction logic; bound target now survives Add↔Cut switches (was: re-prompted body pick on ≥2-body docs); `maybeNegativeDragHint` suppressed while auto (flip IS the affordance). Real-worker proofs: Add on host → bodyEvents MODIFIED, 1 body, vol 80·60·32=153600 exact; Cut inward → 86400 exact + bbox handedness. Suites: FE 1980/144 (+18) · cargo 594/0 (+2, sketch_on_face 14/14) · e2e 94/94 (drag-in chip flips Cut, drag-out flips back, commit "Extrude (Add)", body count unchanged) · ctest 79/79 · tsc/build/clippy/fmt clean.
  Flags: typed negative depth in the chip does NOT flip mode (drag only — open decision); host-seeded arm on ≥2-body docs keeps global canBoolean count; mock hostFace row is FE-optimistic (real-lane overwrite covered by cargo/vitest only).
- [ ] USER manual Tauri gate: tilted-face sketch+extrude flush; snap+dimension to projected corner; L-body coplanar projection; dblclick re-enter; non-planar refusal hint; **push/pull: extrude on face-sketch grows host outward (Add) / pockets inward (Cut) with live chip flip, no stray body**

## M0 — Foundations
- [x] Repo bootstrap: git init, rm empty OneCAD/, tauri.conf (bun, window 1300×864 Overlay)
- [x] protocol/ contract docs: SCHEMA.md (1157) + mesh_format.md (315) + fixtures — GATE PASSED. Notable resolutions: Rust mints ElementIds (worker returns evidence only); magic BYTES normative (LE u32 read-back 0x3157434F); u16 header fields for 64B MESH1; PROTOCOL_ERROR vs UNSUPPORTED split; opType values keep PascalCase.
- [x] R-WP0 cargo workspace scaffold (3 crates, capabilities core:default, CI yml) — GATE PASSED; opener dep removed from package.json too
- [x] W-WP0 worker skeleton: CMake + TKDraw filter verbatim + OCW1 framing + dispatcher + Hello/selftest + harness, ctest 3/3, OCCT 7.9.3 — GATE PASSED (verified independently)
- [x] F-WP0 toolchain: bun deps (three 0.185.1/zustand 5.0.14/tailwind 4.3.3/vitest 4), tokens.css verbatim, alias, smoke test — GATE PASSED
- [x] Characterization corpus: 9 cases w/ provenance, 5 stdout recordings @ b4ddcccc, v1 samples, descriptor constants — GATE PASSED (STEP N/A: UI-only exporter; Symmetric/fillet volumes not-asserted in old tree)
- [x] R-WP6 protocol crate + real stub: OCW1 codec (pure/blocking/async layers), messages (`t` tag), MESH1 validator, ProtocolClient, chaos stub, 50+7 tests — GATE PASSED (verification re-run pending after R-WP1/2 lands). Remaining co-sign half: C++ side speaks to Rust client (W-WP1 build integration + cross fixtures).

## M1 — Cores (parallel per track)
- [x] R-WP1+2 ids/math + OperationRecord schema — review verdict APPROVE-WITH-FIXES
- [x] R-WP2.1 schema fixes: M1–M5 + minors all applied, SCHEMA §7.3 typed targetFace + §7.4 normative RegionId + §14 changelog, existing 14 snapshots UNCHANGED, 134 Rust tests green (verified) — GATE CLOSED. Note: Fillet/Chamfer carry dual edge_ids (bare, wire-aligned) + edges (typed refs) — commands must populate both (R-WP5/R-WP7 rule).
- [x] R-WP4 timeline+graph: cursor=appliedOpCount port, Kahn deterministic (24 perms×64), corpus case (h) reproduced, proptest ×300 — GATE PASSED
- [x] R-WP3 sketch domain: 24 snapshots, 18 constraints C++-verified (line-form H/V, Fixed non-dim, radians), RegionId=FNV-1a-64 lock-tested — GATE PASSED (RegionId must go normative into SCHEMA §7.4 → R-WP2.1)
- [x] R-WP5 Document+EditCommand/session: 21 variants w/ exact inverses, txn batching, selection port, review APPROVE-WITH-FIXES → R-WP5.1 all 12 fixes applied (txn auto-cancel, lockstep both paths, NeedsRepair seeding, producer dirty, re-derive, cycled guard), 169/169 — GATES CLOSED
- [x] R-WP7 regen executor+engine trait+FakeEngine: golden fixtures a–j, 200/200 tests, SHA-256 historyPrefixHash (normative — W-WP4 notified to match), fencing via RevisionGate — provisional PASS, independent review in flight
- [x] R-WP8 scheduler: driver-seam (policy only, no session ownership), preview>regen priority, latest-wins, 120ms debounce, cancel-timeout guard, 10/10 virtual-time tests — GATE PASSED
- [x] R-WP9 file IO: atomic v2 container, attack-surface caps (fuzz: no panics), migration registry + read-only policy, autosave/marker layout, 262/262 — GATE PASSED (verified). Decisions accepted: ops.jsonl derived (document.json authoritative), sketches inline (no sketches/ dir — plan divergence, sound).
- [x] R-WP10 app shell: DocumentRuntime single-writer (Tauri-free, tested), thin commands (new/open/save/close/apply_edit_command/undo/redo/get_projection/get_mesh ipc::Response/dialogs), DTOs mirror frontend stores, RegenScheduler wired via driver seam, mesh LRU (BodyId,Lod,generation), D1 AdoptingEngine decorator + validate_created (reject at prepare→accept, core untouched), SCHEMA §2/§7.2/§14 D1 amendment SIGNED OFF (no fixture bump needed), capabilities core:default unchanged. 282/282 + clippy clean (orchestrator-verified) — GATE PASSED. Flags→R-WP11: parse body_<opId>→BodyId(uuid) at wire layer; release runtime lock across real-worker IO (fencing currently inert); sketch dof/status placeholders; regen-body visibility V1 gap.
- [x] **R-WP11 WorkerManager+chaos (RISKY)**: tokio::process spawn/supervise (ping 5s×2→SIGKILL, backoff .5/1/2×3→Failed, epoch bump + restart hook mark-dirty+replay), poison circuit breaker (base-hash|op|fingerprint, transport-loss only, 3→CircuitOpen), wire Backend (body_<opId> parse, ExecutePlan args w/ opaque tokens, planStep→PlanEvent w/ scoringVersion passthrough, MESH1 inline+chunked SHA-verified w/ credit flow), fencing LIVE (FencingCell atomics, 3-phase driver, lock released across worker IO, supersede-at-accept tested), chaos gate 9 tests (crash/hang/garbage/circuit/convergence 25× env→100), ProtocolClient post-crash hang BUG FOUND+FIXED (closed-latch + regression test). 303/303 + clippy clean orchestrator-verified; REAL-WORKER SMOKE PASSES vs actual C++ binary (agent had wrong path — verified w/ ONECAD_WORKER_PATH). R-WP11-R verdict APPROVE-WITH-FIXES: F1 MAJOR revision-fencing break vs real worker (edit-counter vs accept-counter — live-probe-confirmed; every post-edit regen rejected) → D4 DECISION: worker fences expectedBaseHash+workerEpoch only, documentRevision = Rust-owned advisory stamp adopted at AcceptPrepared (SCHEMA §7.2 amendment); F2 restart flap unbounded for connect-then-die; F3 poison key uses last-op not crashing-op + CircuitOpen kills whole worker vs doc; F4 stub doesn't fence (gate blind to F1); F5 mesh integrity optional-field gaps + manifest sha never cross-checked; F6 jobId low-bits note + restart-replay noop + drill body-id pin. Confirmed sound: 3-phase fencing (no deadlock/lost-update), supervisor (no zombies), closed-latch fix, wire edges, D1 adoption. → R-WP11.1 ALL APPLIED (verified: worker 52/52, Rust 309/309 + clippy, chaos 13/13 ×3, real-worker 2/2 incl. F1 repro vs real binary): D4 fencing (worker fences epoch+hash only, adopts revision at accept; stub mirrors — F4 closed; SCHEMA §2/§7.2/§8/§14 SIGNED OFF, no canonical fixture bump), F2 flap budget (rapid-death strikes), F3 poison keys crashing-op + CircuitOpen keeps worker alive, F5 segment-tiling gap/overlap reject + manifest/resp cross-check, F6 drill body-id pin + restart-hook-on-Ready + jobId invariant. **R-WP11 gate CLOSED.** NEW FLAG → D5: V1 replay-from-0 sends empty expectedBaseHash but worker head hash advances after accept → sequential regens fence-reject. D5 DECISION: from-0 plan (empty anchor base) is always base-valid (epoch still fenced); accept replaces head wholesale; incremental plans keep strict head-hash fence → R-WP11.2 DONE (verified 52/52 worker, 312/312 Rust, real-worker 3/3 incl. sequential two-cycle regen vs real binary): from-0 = empty-anchor detection, empty-base clone at fence, wholesale head replace (was already D4-correct at accept), stub mirrors, D1-uniqueness already correct (empty `existing` set — regression-locked), SCHEMA §7.2 D5 bullet + §14 SIGNED OFF. Sequential-regen M2 blocker CLEARED.
- [x] R-WP12 solver bridge: SolverEngine trait (separate from GeometryEngine), gesture lifecycle (enter/upsert/begin/solve_drag latest-wins superseded-tolerant/end→ONE SketchDragGesture undo cmd), real dof/status in projections, promotion (worker evidence → Rust mints via mint_element_ids, promoted cache = same pick → same id, Invariant 1 verified), ResolveRefs passthrough, regen-finished {revision,outcome} emission, stub solver verbs, 9 commands + sketch-solved event. 312→328 tests + clippy/fmt clean w/ real worker (orchestrator-verified); real PlaneGCS drag + identity verbs live (5/5 real-worker tests) — GATE PASSED. Flags: ellipse untranslated (RESOLVED 2026-08-01, SKETCH-POWER W3); radius-drag not captured (W-WP3b parity); resolve_refs DTO lossy (repair-UI WP); worker autoBind returns topoKey vs SCHEMA elementId (parser tolerates — worker fix w/ repair UI); drag holds writer lock ~ms (lock-free deferred per plan). Flags: accept-window self-heal documented; auto-OpenSession revision alignment unverified vs real worker (M2); solver lane → R-WP12; SaveCheckpoint/Restore/AcquireElementIds UNSUPPORTED in V1; externalBin path → M3.
- [x] **W-WP5-R independent review**: APPROVE-WITH-FIXES, D1 UPHELD (body_<opId> deterministic+collision-safe). Verified: atomicity/fencing, opaque tokens (no op hashing), descriptor reuse by construction (no fork), MESH1 424B byte-identical probe, determinism 2× fresh runs, stderr-only. Findings: (1) MINOR Standard_Failure not caught at Dispatcher boundary → W-WP5-F fix in flight; (2) NOTE split binds Modified().First() unscored → W-WP6 MUST close; (3) NOTE fast-mode parallel TopoKey ordering unverified → W-WP6 must diff determinism-vs-fast TopoKey tables on corpus; (4) NOTE volumes 4064/3936 bounded not pinned → W-WP5-F; (5) NOTE scratch planStep frames stamped base snapshotId not preparedSnapshotId — optional SCHEMA §3.4 tightening, deferred.
- [x] W-WP5-F fixes: Standard_Failure catch at Dispatcher boundary (GetMessageString fallback DynamicType name) + injected-throw recoverability test + Fuse/Cut volumes pinned 4064/3936 — 46/46 verified by orchestrator. **W-WP5 gate CLOSED.**
- [x] **W-WP6 (RISKY)**: ladder scoring resolverVersion=1 (normalized [0,1], weights type .20/magnitude .25/direction .20/anchor .25/adjacency .10, auto-bind ≥0.85 + margin ≥0.10), Hungarian assignment + greedy counterexample test, scored split lineage (closes W-WP5-R finding 2 — ambiguous tie ⇒ NeedsRepair, entry dropped), fast-mode TopoKey diff (finding 3: Invariant 5 HOLDS, parallel BOP does not perturb ordinals), calibration corpus (symmetric tie ⇒ NeedsRepair; history-only ⇒ scorer never consulted; fillet-survives-small-edit / NeedsRepair-on-large), Fillet/Chamfer/Revolve (Pappus 9424.78 verified), ToNext/ToFace/draft (OneCAD-CPP BRepOffsetAPI_DraftAngle port), ExportStep + reader roundtrip, primary.topoKey REMOVED (D3), determinism extended fillet+revolve byte-identical. 51/51 ×2 verified by orchestrator, zero warnings. SCHEMA §9 scoringVersion amendment SIGNED OFF (additive; canonical fixtures carry no NeedsRepair payloads — no bump needed). Gate: W-WP6-R verdict APPROVE-WITH-FIXES — zero correctness bugs; scoring bounds/gate/assignment/lineage/determinism all independently verified (7-case Hungarian probe, STEP stdout probe 526B→0B w/ redirect, adjacency-0.10 independently validated, symmetric-tie ~0.84 accepted as non-normative). Findings F1 stub scoringVersion unfixtured, F2 ExportStep stdout-hygiene untested, F3 ResolveRefs unfixtured, F6 extrude kMinValue 1e-6 vs reference 1e-3, F7 Assignment.h comment overstates binding → W-WP6-F ALL APPLIED (stub+fixture pin; stdout-hygiene assert probe-verified fail-without-redirect; resolve_refs.ndjson 3-outcome fixture — note sketch XY basis catch; kMinValue→1e-3 parity; comment corrected). 52/52 orchestrator-verified. **W-WP6 gate FULLY CLOSED.** F4 (fast-mode model too small to force real threading) + F5 (edgeIds ignored by design, no lockstep assert) accepted as NOTEs. Rust RepairItem scoringVersion FIXED separately (commit after adf1a8e).
- [x] F-WP8 real-backend swap: tauriClient (same CadClient interface), runtime switch __TAURI_INTERNALS__ (real→tauri, browser/vitest/Playwright→mock), localSolver.ts extracted seam (sketch+preview lanes shared, commit(op) injected — preview drag local, commit REAL), sync-over-async correlation adapter (revision-gated document-changed await, 8s fallback), MESH1 ArrayBuffer verbatim through existing parser, ApiError normalization. 297/297 + build + hex gate verified — GATE PASSED. M2 flags: (1) sketch/region+edge refs local until R-WP12/AcquireElementIds — real Extrude/Fillet commit rejected until then; (2) projection-updated→documentStore hydration bridge MISSING (M2 frontend task); (3) recommend emitting regen-finished for exact correlation; (4) controller try/catch minor; (5) get_mesh bodyIds must come from events not seeds.
- [x] W-WP2 kernel port + PlaneGCS vendor: 12 kernel + 8 loop + 2 modeling files, ctest 8/8, elementmap byte-parity PROVEN w/ negative control — GATE PASSED (loop/modeling copied-not-compiled pending sketch stack; proto_loop_detector/face_builder deferred to W-WP3)
- [x] W-WP3a sketch stack port: 28 files/8.7k LOC Qt-stripped byte-faithful, loop+modeling compiled (BooleanMode.h adaptation), ctest 17/17, terminator-parity gates w/ negative control — GATE PASSED (verified: zero Qt tokens outside comments). Flag: ConstraintApplicability+SelectionTypes ported into worker for test parity — UI/selection layer, dedup vs Rust selection.rs later.
- [x] W-WP3b solver lane + verbs: 2-lane dispatcher, latest-wins w/ CANCELLED/superseded terminals, WireSketch translator, RegionId byte-match vs Rust (r_fbf1e34acfb51ba4), **BENCHMARK GATE PASS** (solver p95 2.50ms / rtt p95 2.66ms @200ents; busy-kernel invariant), 26 ctests — GATE PASSED. V1 limits doc'd (holes not subtracted in preview fill, arc handles, redundant-status quirk).
- [x] W-WP3c envelope alignment: SCHEMA §3 exact (t/args/result/error, u64 id, stamps, unsolicited hello), canonical Rust-authored fixtures PASS vs C++ worker, 29/29 ctests, no latency regression — GATE PASSED (verified)
- [x] W-WP4 transactional shell: Session/ScratchJob/PlanExecutor, fence-clone-execute-lockfree-swap atomicity, stub ops + test hooks, concurrent-lanes proof (solver 68ms during 500ms plan), determinism across fresh processes, 41/41 — GATE PASSED. Decisions: same-jobId re-send idempotent / different-jobId rejected (SCHEMA changelog pending); lastValidStep null=base-only.
- [x] X-WP1 + R-WP7.1: hash authority = Rust (prefixHashes[] opaque tokens), planner hash = geometry-relevant wire-op form, ALL R-WP7 review MAJORs fixed (bodyId partitions, checkpoint reseed+dedup, replay-from-0 retry, fold gating, cancel recheck, epoch gate), SCHEMA §7.2 amended, 209/209 — GATES CLOSED (R-WP7 now fully closed)
- [x] W-WP5 real OCCT Extrude+Boolean + ElementMap V2 partitions + history mapping + MESH1 tessellation + opaque-token switch: corpus volumes EXACT, 46/46 ctests (verified), determinism across fresh processes — provisional PASS; independent review HELD at user pause point. Cross-track flags: NewBody id worker-minted "body_<opId>" (Rust must adopt from bodyEvents); primary.topoKey non-SCHEMA field used for deterministic minting (W-WP6 ladder replaces); ToNext/ToFace/draft/Revolve/Fillet deferred W-WP6. solver lane verbs + **latency benchmark gate** → transactional shell → **Extrude+Boolean (RISKY)** → **ElementMap V2+ladder+calibration (RISKY)** → Fillet/Chamfer/Revolve → Tessellation+MESH1
- [x] F-WP1 primitives+icons: 9 primitives + 32 icons verbatim + DevGallery, 52/52 tests, hex gate clean — GATE PASSED (Popover radius corrected to 8px prototype value)
- [x] F-WP2 start screen: 1a faithful, 57/57 tests, 3 new tokens — GATE PASSED (Button lg=36px added for action row per prototype)
- [x] F-WP3 editor shell 1c (**flagship pixel gate**): 5 stores (document/selection/tool/viewport/settings-persisted) + keymap/useShortcuts (mode-scoped, F=fillet vs ⇧F=zoom-fit) + full floating chrome (titlebar, toolbar, tree, inspector 3-state, sketch chrome, snap popover, corner cluster, nav pill, status bar) over hatched canvas placeholder. 25 new tests (82 total), build+hex-grep clean, Playwright model/sketch/snap screenshots verified faithful to 1c. Deviations: 4 new tokens (tree-label #33383f, titlebar-text #3a3f46, warn-strong #8a5b10, shadow-sketch-pill); seed tree mirrors prototype (1 body/3 sketches) not "2/2" per pixel gate; grid default off + traffic-lights = OS overlay reservation; sketch-inspector uses 1e warn card.
- [x] F-WP4 viewport core: engine class (owns canvas — StrictMode context-loss fix), CadOrbitControls, CameraRig persp⇄ortho, adaptive grid, HtmlOverlayDriver, CSS-3D ViewCube, render-on-demand (idle=0 verified), Z-up invariant doc'd, 118/118 tests + browser verification — GATE PASSED
- [x] F-WP5 IPC+mesh+picking: MESH1 parser byte-identical to worked example, zero-copy views + lazy ID decode, registry double-buffer + leak tripwire, rAF picking + drawRange highlights, orbit hit-test gating, 169/169 — GATE PASSED (verified; .playwright-cli artifact removed)
- [x] F-WP6 sketch mode: tools (line chain/rect/circle/arc center-start-end), snapEngine + Alt suppress, AutoConstrainer port (±5°), badges + DimensionInput, Line2 px-width, plane-ortho camera flow, 233/233 + browser verified — GATE PASSED (verified)
- [x] F-WP7 tools+preview: extrude drag (auto-arm from finish-sketch, Alt symmetric, flip-through-zero), fillet radius chip, boolean chip, undo/redo, live HistoryList + dbl-click edit seed, **60fps GATE PASS** (p95 ~10ms @300ms L2 lag, epoch race-free), 278/278 — GATE PASSED (verified). F vertical slice COMPLETE; F-WP8 real-backend swap blocked on R-WP10/11.
- [ ] R-WP5.1 session review fixes (in flight: txn auto-cancel MAJOR + lockstep + repair seeding + hardening)

- [x] F-WP9: real sketch lane (sketchWireMap marshaller w/ point synthesis + UUID id-map), drag latest-wins client reconcile, L2 preview stays local (no backend preview verb — seam-marked), hydration bridge (revision >= rule, empty always resets), regen-finished correlation (8s fallback → safety net only), pick→promote wired in ViewportRoot, status-bar error surfacing. 297→319 tests + build + hex verified — GATE PASSED. M2 script gaps (from report): (1) snapshotId not exposed to frontend — promote sends 0 (BLOCKER, needs small Rust event/DTO change); (2) real sketch round-trip unproven; (3) PlaneGCS dof-0 convergence of synthesized rectangle; (4) sketch re-entry returns [] constraints; (5) solvedPositions reverse map missing (M4); (6) drag has no frontend caller yet (gate drives client directly); (7) non-XY plane targets; (8) dual store authorship (optimistic vs hydration).

## M2 — **First micro-slice integration gate**
- [x] **M2 GATE PASS** (2026-07-18, src-tauri/tests/m2_gate.rs vs real worker, all 8 steps asserted): (1) rectangle dof=0, region r_ac127d88469498d7; (2) extrude → body, bbox 40×20×25; (3) MESH1 parse, FACE_RANGES tile; (4) promote → el_… minted, re-pick same id (Invariant 1); (5) fillet e:1 applied 6→7 faces; (6) save→reopen→replay FRESH worker: identical hash chain/bodies/signatures + document.json byte-identical; (7) STEP 20259B ISO-10303-21; (8) undo fillet reverts, signature match. Plus non-XY basis (XZ) + authoritative projection asserted. snapshotId plumbed (document-changed → promoteSelection real id). TWO cross-layer defects found+fixed by gate: wire_op inputs sent graph-view shape not SCHEMA §7.3 semantic-ref array (latent — Extrude fallback masked it); worker mesh handle key "section"→"bin" (SCHEMA §5.2 — every real mesh fetch was broken). Suites: worker 52/52, Rust 330, frontend 320, build green. M3 flags: STEP writer stderr chatter (confirm hygiene under packaging); extrude profile binding = last_sketch_id + first-region fallback (multi-region/multi-sketch = M4 gap).

- [x] M2-R implementation review (2026-07-19, 39-agent workflow over a4a10e2 → 22 verified findings) + fixes: CONFIRMED systemic BodyId wire-form mismatch — params body fields crossed as bare uuids (core serde, schema-frozen) while worker BodyStore keys `body_<uuid>` → standalone Boolean / Extrude Cut-Add pocket / ToFace / bare-edge fillet all failed on the real wire (m2_gate only covered NewBody+typed-edge fillet). Fixed at wire layer: `to_wire_body_form` recursive params rewrite (idempotent, body-keys only; core serde + planner untouched — planner hash golden-pinned), `element_ref_wire` → serde form (preserves `extra` flatten maps, kills element_kind_str dup), bare-edge_ids fallback carries operated body from graph inputs, worker `input_body` kind=="body" guard (ExtrudeOp/RevolveOp — ToFace ref can't misbind as boolean target), shared `MeshHandle` builder for §7.6 handles (closes the "section"/"bin" divergence class), SCHEMA §14 conformance bullet SIGNED OFF (no fixture bump — fixtures already spec-form). Pre-resolver/ExtrudeOp ToFace split-brain analyzed: complementary owners, aligned by the fix, pinned end-to-end. NEW real-worker regression gate `wire_contract.rs` 6/6 (boolean cut=10000/union=30000, pocket=18000, ToFace z=25 vol=5000, bare-fillet ladder-resolved, hash-stability golden); pre-fix failure REPRODUCED (boolean vol 20000 with transform stashed). m2_gate cleanups (body_id_wire reuse, protocol LE readers pub, add_op/regen_all helpers). Deferred: snapshot_id Option-vs-0-sentinel refactor; SCHEMA-normative extrude SketchRegion inputs[] ref → M4 multi-region WP. Suites: worker 52/52, Rust 347, frontend 346. **Independent review (RISKY convention): APPROVE, zero blocking** — heuristic completeness enumerated field-by-field, idempotency/double-application proven impossible, hash decoupling proven by crate boundary (core cannot depend on app crate), kind-guard verified vs FilletChamferOp's own scan + ToFace's params-path, MeshHandle superset safe vs all consumers incl. stub, wire_contract volumes re-derived + each test proven fix-requiring. 3 NOTEs: (1) intent/descriptor subtree recursion → HARDENED (transform now skips `intent` wholesale — worker-authored evidence round-trips verbatim, unit-pinned); (2) Opaque params transform harmless (worker rejects unknown opType pre-consumption; stored record untouched); (3) attach_tessellate snapshotId=prepared_snapshot_id non-load-bearing, accepted.
- [x] FX-WP file/app UX (2026-07-19): `StepExporter` seam (export.rs; WorkerManager + PendingBackend impls) + `export_step_file` command (.step dialog, head bodies via read-only `head_body_ids()`, AP214IS, cancel→null); real `list_recents` (`<app_config_dir>/recents.json`, atomic temp+rename, dedup by path, cap 10, recorded on successful save/open); worker-status forwarder (WorkerLifecycle broadcast → `worker-status` events, initial state on subscribe, CircuitOpen skipped) + StatusBar dot/label on restarting/failed; FileMenu popover in TitleBar (Open ⌘O / Save ⌘S / Save As ⇧⌘S / Export STEP) via shared fileActions bridge, save-with-no-path → Save-As fallback; sketch re-entry constraints hydrated (enter_sketch DTO → frontendConstraintsFromDto → sketchStore; InspectorPanel hardcoded SKETCH_CONSTRAINTS deleted, live ConstraintList); documentStore mock seed gated to non-Tauri. Frontend 320→346, Rust lib 40→51, hex gate clean — GATE PASSED (orchestrator-verified).

## M3 — **Packaging gate (early)**
- [x] M3 Linux-verifiable portion (2026-07-18): tauri.conf `bundle.externalBin` sidecar; `resolve_worker_path` 3-rung chain (env → exe-adjacent bundled → dev) w/ pure testable core + 4 unit tests; `scripts/bundle-dylibs.sh` (otool closure → @rpath → ad-hoc re-sign, shellcheck-clean, Darwin-only); `docs/PACKAGING.md` (pipeline + clean-Mac checklist incl. bundled `--selftest` + STEP stdout-hygiene cite `wp6_exportstep`); CI rust job builds worker + sets ONECAD_WORKER_PATH so real-worker/m2 tests run. Linux smoke: `bun run tauri build --bundles deb` → deb carries `usr/bin/onecad-worker` beside `usr/bin/onecad`; release-dir sidecar `--selftest` exit 0 (occt 7.9.3). Env note: this container = Ubuntu + conda-forge OCCT 7.9.3 at /opt/occt793 (apt 7.6.3 too old for TopTools_ShapeMapHasher functor API).
- [ ] M3 Mac-side verification (DEFERRED to a Mac): signed app on clean Mac w/o Homebrew, bundled worker --selftest, bundle-dylibs.sh first real run — checklist in docs/PACKAGING.md §5

## M4 — Topology slice
- [x] M4a backend (2026-07-19): (1) multi-region profile binding — ExtrudeParams.profile already existed (no schema change; snapshots untouched); wire lift_profile_to_params → params.sketchId/regionId (§7.3, Extrude+Revolve), worker build_profile_face matches normative FNV region id, **STRICT: non-empty no-match = deterministic OP_FAILED naming requested+available ids (orchestrator-required tightening of the agent's initial lenient fallback — "never a silent wrong bind"), empty = first-region V1 fallback**; additive perStepResults[].message carries failed-step reason (failed steps emit no planStep); real-worker proof: two regions extruded by id → 4000 vs 1000, stale-id-after-sketch-edit → deterministic OP_FAILED replay-identical. (2) autoBind elementId conformance: partition-minted elementId in the §7.5 slot, topoKey demoted to evidence (§9), strict Rust parser w/ one-release tolerance, resolve_refs.ndjson fixture bumped. (3) resolve_refs DTO un-lossy: full ladder result (score/margin/reason/anchor/uiLabel + candidates[] w/ worldPos/summary/featureContributions). (4) needs-repair events on every published regen (empty = cleared) + RegenReport.needs_repair + repair_items/timeline_records accessors. (5) **H5-B PROOF** topology_rebind.rs vs real worker: small edit → fillet re-applies, NO NeedsRepair, bound ElementId STABLE; destructive edit → deterministic NeedsRepair (published body = unfilleted, never wrong bind), event payload replay-identical; symmetric tie → NeedsRepair never a guess (calibration fixture + live 5-candidate tie). SCHEMA §14 ×2 bullets SIGNED OFF. Worker 53/53 (new region_nomatch harness), Rust 356, fmt/clippy clean — GATE PASSED (orchestrator-verified; independent review next).
- [x] M4b repair UI (2026-07-19): repairStore + needs-repair listener, RepairBanner (amber pill, auto-dismiss on empty event), RepairPanel inspector state (expand → resolveRefs, score-meter candidates, hover publishes worldPos to store — engine marker seam), click-to-rebind = promote candidate topoKey→minted elementId → `EditOperationInput{FilletEdges{index}}` (backend rewrites edge_ids+edges in lockstep — command.rs-verified; frontend dual-rewrite also implemented+pinned for UpdateOperationParams), refId "<opId>.input<k>" parsing pinned vs dto example; HistoryList suppress/roll-to-here/delete (two-click confirm) affordances + suppressed dimming (optimistic historyStore overlay — projection lacks a Suppressed flag, seam noted); fillet re-edit dblclick (radius-only, mirrors revolve); solvedPositions reverse map (backend point-UUID keys → frontend ids via sketchWireMap point map; applied on upsert/endGesture/commit). Seams flagged: resolveRefs sends refId-only (lean event carries no ElementRef); >1-body operated-body derivation; candidate viewport highlight (data seam only). Frontend 373→421 (+48), build + hex clean — GATE PASSED (orchestrator-verified incl. FilletEdges backend contract read).

- [x] M4a-F review fixes (2026-07-19, verdict APPROVE-WITH-FIXES → all applied, M4 CLOSED): MINOR-1 tautological stable-id assertion → worker resolve_refs dry-run re-resolution; **exposed + fixed a REAL latent bug: resolve_refs_args sent bare-uuid primary.bodyId (M2 defect class) → whole repair-UI resolve path failed vs real bodies; now renders via element_ref_wire**. Geometric inversion flagged by agent + accepted: a fillet CONSUMES its edge, so re-resolving the promoted edge must yield NeedsRepair (border-edge tie) — autoBind would mean the fillet mis-bound; asserted deterministically w/ same minted id + topoKey evidence. MINOR-2 skip guards (env-set-but-missing ⇒ panic; ONECAD_REQUIRE_WORKER=1 ⇒ panic if unresolved; CI sets it — vacuous-green closed, panic proven). NOTE-3 io::migrate sanitize_region_ids on open (non-`^r_[0-9a-f]{16}$` → "" + region-id-legacy-reset diagnostic, caches stale; 2 tests). NOTE-1 Sweep-lift doc'd. Orphaned timeline_records() removed. Worker 53/53, Rust 358, fmt/clippy clean — orchestrator-verified. NOTEs accepted: solve() asymmetry (masked by topological id), same-process determinism scope, stale-region terminal-op scope.

## M5 — Lifecycle + recovery
- [x] Revolve frontend tool (2026-07-19): revolveStep FSM (idle→axisPick→armed→dragging→committing; plain-click=360°), axis-pick w/ region-straddle validity, Rodrigues lathe L1 preview (RevolvePreview engine object), 45° snap detents + Alt, angle chip + axis reset, dblclick re-edit (param-only), RevolveParams wire mapping pinned (angleDeg degrees, AxisRef sketchLine, profile lift), mock lane parity. Frontend 346→373 — GATE PASSED (orchestrator-verified).
- [x] M5a exports+checkpoints+splits (2026-07-19): (1) STL(bin/ascii)/OBJ export — worker MeshExport verbs reusing viewport tessellation (tessellate_raw; determinism), Rust GeometryExporter (widened StepExporter) + export_stl_file/export_obj_file dialogs + FileMenu entries; (2) Checkpoints — worker Save/RestoreCheckpoint (BinTools + 3 signatures + prefixHash, fenced, in-session restore V1; artifacts inline resp tail; container persist via CheckpointArtifacts JSON; policy: checkpoint at head on explicit save), Rust WorkerManager real impls + planner reseed wired; **checkpoints.rs proves incremental regen byte-identical to from-0**; latent stepIndex wire bug found+fixed (§7.3); (3) Boolean/Cut split children body_<opId>:<k> — deterministic ordered_solids (quantized volume/centroid/face-count key), Rust derived-uuid (SHA-256 domain-sep) + wire interner + validate_created split acceptance; **orchestrator diff review caught MAJOR: interner was process-global parse-time-only → downstream-op-on-split-child docs failed REF_UNRESOLVED on reopen; fixed: BodyMeta.split_of{op,k} additive persist (snapshots byte-stable) + re-intern on open/restore before plan compile; pre-fix failure unit-demonstrated + cross-process real-worker proof (fresh runtime+worker, 7500/5000 volumes, signature-identical to warm baseline)**. §7.7 divergences signed off (inline tail vs bulk; in-session restore w/ restored:false→from-0 Invariant 7; partition placeholder). Worker 58/58, Rust 366, frontend 431 — GATE PASSED (orchestrator-verified).
- [x] Revolve frontend tool (2026-07-19): revolveStep FSM (idle→axisPick→armed→dragging→committing; plain-click=360° quickCommit; re-edit skips axis-pick), axis validity via axisSplitsRegion (strict-straddle reject, touching allowed), lathePreview Rodrigues sweep L1 (RevolvePreview engine object, candidate-axis highlight), revolveAngle px→deg + 45° detents (3° window, Alt suppress), editable ° chip + Axis reset, Revolve→addOperation/updateOperationParams mapping pinned (angleDeg Scalar DEGREES, internally-tagged AxisRef sketchLine, profile {sketchId,regionId}), HistoryList dblclick re-edit, mock lane parity (makeRevolveBodyMesh MESH1). Commit path = applyOperation (fillet-style; L2 preview lane is extrude-specific — seam noted). Frontend 346→373, build + hex gate clean — GATE PASSED (orchestrator-verified).
- [ ] boolean/split BodyId rules, checkpoints+envelopes, crash drills, 3-signature drift, onecad-regen CLI + CI gate, autosave/recovery, STL/OBJ

- [x] M5b lifecycle+recovery (2026-07-19, **M5 CLOSED**): autosave driver (watch mutation-tick seam bumped by every mutating command + published regens — chosen over scheduler piggyback since sketch edits bypass it; 30s debounce; io::recovery layout <app_data>/autosave/<docId>.onecad + pid marker; AUTOSAVE event; zero activity w/o doc; clean save/close clears), check_recovery/recover_document commands + StartScreen RecoveryCard (stale-marker scan via pid_alive); onecad-regen CLI (crates/onecad-regen, depends on app crate for byte-parity replay through real WorkerManager/DocumentRuntime; exit 0=published∧noFailed, NeedsRepair=warn (--strict upgrades), 1/2/3 taxonomy, --json; CARGO_BIN_EXE test in CI via cargo test); crash drills: worker-kill-with-checkpoint → restart from-0 replay converges (restart hook enqueues from:0, checkpoint bypassed — correct path), autosave crash round-trip; signature-drift: doctored descriptor_version → checkpoint loads but version-gate rejects → from-0, signature==baseline, no error. **Latent pre-existing edge flagged (not fixed, out of scope): checkpoint selected-then-restore-fails mid-run would re-mint base body outside incremental known_ops → D1-rejected; production restart path never hits it (always from:0) — future WP if restore-fallback ever goes live mid-plan.** V1 scope: startup-only recovery (newer-autosave-on-open seam-commented). Rust 373, frontend 439, worker 58/58 — GATE PASSED (orchestrator-verified).

## MODELING CORRECTNESS P2-P4 — live gate delta (2026-08-11)

- [x] Baseline recorded: HEAD `5d2081895a8b1f6798d935526cce4a6a95cce554`; OCCT 8.0.1 fingerprint `0a6a1dce34181289`.
- [x] Worker baseline: `ctest --test-dir worker/build -R region_table --output-on-failure` passed.
- [x] P2 evidence at `055d31f7d8edcb106a881590b9b2c5a9d1e37982`, OCCT 8.0.1 `0a6a1dce34181289`: `scripts/build-worker.sh Release`; `ctest --test-dir worker/build --output-on-failure` = 112/112; `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; real-worker `cargo test --workspace`; `npx tsc --noEmit`; `bun run build`; Vitest 241 files / 4115; Playwright Chromium 196/196 and WebKit 196/196, retries 0. No product failures.
- [x] Chromium zero-retry Boolean preview: `bun run e2e -- e2e/boolean-preview.spec.ts --project=chromium --retries=0` = 2/2 passed. Sandbox launch failed before test execution (`bootstrap_check_in ... Permission denied`); approved desktop retry passed.
- [x] Full Chromium + WebKit zero-retry lanes: 196/196 each. The initial all-red Chromium run was a spawned-Vite `ERR_CONNECTION_REFUSED`; persistent server passed. WebKit reproduced two shared helper failures alone, then passed after pointer cleanup in `finally`.
- [ ] Manual Tauri smoke: Open -> Extrude -> Fillet -> Undo -> Save -> Reopen.
- [ ] Unchanged T0 semantic/digest campaign: started against `worker/build/onecad-kernelbench-runner`, artifacts emitted but no completed `results.jsonl`; do not count as a pass.
- [~] P2: implemented analytic `CurveFragment` graph + exact Line/Circle/Arc/Ellipse BRep authoring, v2 identity, bounded/cancellable OCCT refinement, tangency collapse and overlap refusal (2026-08-11). `region_table` covers overlap/chord/arc/ellipse/tangent/coincident/cancellation; worker CTest 112/112, real-worker Rust, TypeScript/Vitest, and zero-retry Chromium/WebKit passed. Manual Tauri smoke and completed unchanged T0 remain before P2 gate pass.
- [~] P3: common Tier A/B `PublicationPolicy` gates Extrude/Revolve, Boolean, Fillet/Chamfer, Shell, Hole, fused Pattern, and Mirror; fused disconnected Pattern refuses `PATTERN_DISJOINT_RESULT`; strict worker readers cover Pattern/Mirror/Revolve/Transform/Shell/Boolean; Pattern/Transform cap at 128 and poll cancellation. Versioned Pattern V2 is now normative: non-fuse preserves source instance 0 and mints `body_<opId>:<k>` for transformed instance `k+1`; fused V2 modifies source in place; absent version remains V1. Frontend authors V2, legacy re-edit preserves absence; child visibility/color inherits source; source/children survive count edits, suppression, undo, save/reopen (real-worker gate); direct ExecutePlan gate proves source has no lifecycle event. Next: finish per-operation contract artifact, Transform/import policy rows, and UI mode disposition.
- [x] P4 bootstrap: `docs/qa/modeling-operation-coverage.json` classifies all nine corpus cases; `scripts/verify-modeling-coverage.mjs` enforces exact census in CI.
- [ ] P4: execute each classified corpus case against real worker or record one explicit unsupported reason.

## M6 — Hardening + backlog
- [x] M6a breadth ops backend (2026-07-19): ShellOp (MakeThickSolidByJoin, legacy-verbatim negated thickness, Modify lineage w/ history fold; open faces resolve via partition binding OR ladder, else NeedsRepair — bare Vec<ElementId> schema documented as the anchor-less limitation), PatternOp (Linear+Circular shared build_pattern; includes source, ONE NewBody, fuse-or-compound per fuseResult, step=angleDeg/count legacy-exact, empty delta = ID-on-demand like legacy rebindBody), MirrorOp (gp_Trsf SetMirror, fuse optional). Wire inputs branches (face_input_refs, body source refs). SCHEMA §7.3 extended w/ 4 normative payloads + §8/§14 (SIGNED OFF). Worker ctest m6a_ops 10 cases (Shell 4112 EXACT both paths, patterns 30000, mirror 10000/20000, guards) + 2 harness fixtures; Rust breadth_ops.rs 6 real-worker tests incl. fresh-process determinism + upstream-edit re-run (30000→60000). Worker 61/61, Rust 379 — GATE PASSED (orchestrator-verified).
- [x] M6b breadth tools frontend (2026-07-19): Shell (fillet-pattern drag + mm chip, faces=removed), Linear/CircularPattern + Mirror (chip-driven config, GhostLayer translucent registry-clone previews, orbit free), 4 icons, K/P/C/M keys, wire mappings pinned vs record.rs (Vec3 arrays, bare u32 count), re-edit chips, mock parity. 439→481 tests, hex 0 — GATE PASSED (orchestrator-verified).
- [x] M6c sketch parity (2026-07-19): snap Quadrant/Intersection/OnCurve (SnapManager.cpp math verbatim: segment-segment/line-circle/circle-circle; strict tier ladder endpoint>mid>center>quadrant>intersection>onCurve>guides>grid; 2mm→8px doc'd; settings v1→v2 migrate, default on; distinct sprite markers dot/diamond/cross/ring); autoconstrain Perpendicular/Parallel/Tangent (AutoConstrainer.h tolerances exact: 5°=0.0872665, coincidence 2mm, MIN_GEOMETRY 0.01mm; H/V-over-Parallel precedence test-pinned; tangent = legacy arc-start-on-line rule); Dimension tool real (pick FSM: circle→Diameter, arc→Radius, line→Distance→2nd line upgrades Angle, 2 points→Distance; existing sketchUpsert pipeline; OverConstrained/Conflicting → auto-undo + hint; angle in degrees — unit flagged). 481→559 tests (+78), hex 0 — GATE PASSED (orchestrator-verified). Seams: H/V-Dist modifiers V2; redundant-vs-conflicting granularity awaits real-lane signal.
- [x] PR#1 external (Codex) review fixes (2026-07-19, all 4 findings VALID): P1 fillet mapper dropped typed edge refs → dual edges+edgeIds from OperationOp.inputs (bare-uuid bodyId normalization in shared edgeElementRef — **also fixed the same latent form bug in the M4b rebind path**; real-worker test applies the EXACT mapper JSON, faces 6→7); P2 refId-only resolve_refs → backend hydrates stored ref at <recordId>.input<k> (lean==explicit proven, 5 candidates vs prior empty); P2×2 re-edit param clobber (revolve axis / shell openFaces / latent fillet) → get_operation_params command + fetch-merge-submit scalar-only updates. Rust 383, frontend 567. Known pre-existing: bareBodyId can't invert split-child wire form (rebind-on-split-edge was already broken; tracked). CI: worker+frontend GREEN on macOS Homebrew OCCT; rust job fixed (sidecar staging before clippy).
- [ ] Backlog: Loft/Sweep, snap parity (Quadrant/Intersection/OnCurve), autoconstrain parity (Perp/Tangent/Parallel), datum planes e2e, checkpoint heuristics/scrubbing, expressions, v1 importer, Channel tessellation, backend L2 preview verb, STEP import, PlaneGCS ellipse constraints (vendored GCS.h:409-515 has full ellipse support incl. PointOnEllipse/Tangent/InternalAlignment — unused; V1 ellipse = solver-free legacy parity w/ naive DOF, SCHEMA §7.4), ellipse quadrant/intersection snaps + radius/rotation drag, slot arc-endpoint wire handles (`.start`/`.end` — would restore wall↔cap Coincidents + real-lane DOF 5) + Equal/Midpoint real-worker DOF proofs
- [x] Resolved from EXTRUDE-REGION-PARITY: Extrude/Revolve profile refs carry `regionIdentityVersion:2`; intersected analytic fragments now commit trimmed Line/Circle/Arc/Ellipse BRep edges, not chord-approximated wires.
- [ ] Backlog (EXTRUDE-REGION-PARITY 2026-07-29): `PreparedSketchRegions::drive` re-upserts the sketch into the solver cache — unguarded vs a concurrently active drag gesture on the same sketch (no test); `Picker.secondaryHitWins` lets a coplanar sketch fill beat a body face at equal distance (intended for sketch-on-face; ToFace picks use probePick directly and are immune — re-check in manual gate); `historyActions.toFeatureMeta` drops `statusMessage` (errored-feature tooltip lost on that path)

## EXTRUDE-COMMIT-FIX part 2 — frontend wiring (2026-07-31, user re-reported "✓ does nothing")
- [x] Gap found: part 1 minted the Sketch record only in `finish_sketch` — but NO frontend path called it for extrude (SketchController.exit → cancelSketch only; arm = pure-read getSketchRegions; revolve alone finished at arm). Live-doc forensics: doc 95ff7fd0 = 1 Extrude / 0 Sketch records; post-rebuild doc a87ece69 = 0 records (rollback hid the failure). Working-tree replay of the broken doc publishes (backfill) — live sessions stayed broken.
- [x] Fix: (1) `tauriClient.finishSketch` id-map fallback `?? sketchId` (kills the reopened-sketch throw — also unblocks the revolve backlog bug); (2) `SketchController.exit` = cancel (gesture teardown + squash) THEN finish (record mint/refresh) on every keep-exit; (3) `confirmExtrude` profile-record guarantee: gen-gated `finishSketch` before the commit loop, failure re-arms w/o touching sessions; (4) mock-lane sessionless finish answers from the region cache instead of clobbering it.
- [x] Tracing shipped both sides (EXTRUDE-COMMIT investigation): Rust `tracing` — apply_edit_command digest, begin_regen step list, regen outcome + per-failed-step warn, finish/cancel_sketch, record MINT/REFRESH, backfill, preview_op failures; FE `src/debug/trace.ts` — [extrude] confirm lifecycle, [ipc] applyEdit/regen-finished/document-changed, [lane] commit, [sketch] exit.
- [x] Suites: tsc 0 · FE 1232/1232 (commit tests re-pinned: double-settle + finishSketch-before-endPreview) · e2e 38/38 · cargo workspace green vs real worker (38 targets) · clippy clean
- [ ] USER manual gate: fresh doc → sketch → extrude → ✓ creates body; Esc-exit sketch → extrude later → ✓ works; reopen old broken doc → ✓ works

## EXTRUDE-COMMIT-FIX — apply silently failed (2026-07-30, SHIPPED)
- [x] Root cause: interactive flow never authored the sketch's `Sketch` TIMELINE record → planner can't resolve any profile → every extrude commit failed "profile sketch not found in plan" (user doc forensics: 20 Extrude / 0 Sketch records); each failed ✓ also STACKED a duplicate errored record
- [x] `finish_sketch` mints/refreshes the record (+ `finish_sketch_with_outcome` → scheduler feed, projection emit; no-op when unchanged); failed commits now `rollbackFailedCommit` (undo pops the errored record before re-arm)
- [x] `from_document` BACKFILLS missing Sketch records at open/recover (front insert, cursor shifted by k, in-memory until next save, fixed-point) — reopened legacy documents extrude again
- [x] Real-worker gates: `interactive_sketch_flow_mints_the_timeline_record_and_extrude_commits` + `legacy_container_without_sketch_records_extrudes_after_open` (red-first proven via kill-switch), scheduler_commit 6/6, workspace green vs real worker, clippy/fmt clean, FE 1232/1232 (stale StartScreen tab marker → File menu), e2e 38/38
- [ ] USER manual gate: reopen the previously-broken document → region → Extrude → ✓ applies; then delete the legacy stacked errored Extrude rows from HISTORY

## VP-FIX — Blank viewport (plan approved 2026-07-20, `~/.claude/plans/do-thorough-analysis-of-reactive-kahn.md`)
- [x] P1 never-blank: grid default ON, OriginTriad (vertex-color LineSegments, chooseGridStep-sized), canvas → --color-canvas, dofBadge null, inspector fake counts dropped
- [x] P3a persisted sketch entry: tauriClient adopts projection id as backend SketchId (no dup AddSketch), seedIdMapFromWire (re-marshal = 0 ops; Circle/Arc .Center unseeded — wire inlines coords, seam doc'd), mock seeds rectangle for document sketches
- [x] P2 geometry on open: MeshIngest initial sweep (visible-only, loadSeq-idempotent), CadClient.getProjection (revision-reconciled pull closes missed-event race), empty-buffer guard, fitView once on first body
- [x] P4 robustness: webglcontextlost preventDefault (disposed-guarded) + restored→invalidate, visibility/focus wake repaint
- [x] Verify: 582/582 tests + build green; browser smoke via playwright (body+grid+fit, sketch entry hydrated, frames=32, gridVisible true, bodiesChildren 1). Tauri-app manual gate (?vpdebug) pending user run.

## SKETCH-PLANE-PICK — Fusion-style plane selection (plan approved 2026-07-20, SHIPPED)
- [x] P1+P2 tokens (--color-plane-xy/xz/yz) + PlanePicker (3 quads 120mm, geometric labels vs legacy-swapped kinds, hover chip) + ViewportEngine API (setPlanePickerVisible w/ iso nudge, planePickerHover/HitTest) + orbit gate
- [x] P4 tauriClient id adoption: {newOnPlane} w/o sketchId → minted UUID = backend SketchId = session.sketchId = projection key
- [x] P3+P5 both fallbacks killed (DEFAULT_SKETCH_ID + `?? "sketch"`), SketchController planePick phase (pointer/Esc/exit/selectMachine guards, enter→openSession split, addSketch+nextSketchName+select), chrome pick variant + ?sketchdemo explicit id
- [x] P6: 603/603 tests + build green; playwright walkthrough verified (New sketch → 3 quads + hint, hover "XY" chip + tint, click → Sketch 6 created/selected/ortho TOP/editing chrome, Esc → clean model restore)
- Deferred: face/datum planes, zoom-adaptive quad sizing, toolbar disable during pick
- [x] HOTFIX real-wire AddSketch (2026-07-20): TS `buildAddSketch` omitted REQUIRED `SketchData.plane` (no serde default) → every real-backend sketch create failed deserialization pre-handler (mock lane masked it; F-WP9 "round-trip unproven" gap). Fix: wire carries canonical planeFor basis (== Rust SketchPlane::xy/xz/yz verbatim, Vec3=[x,y,z]). NEW Rust contract pins (edit_session.rs): frontend AddSketch JSON parses+applies AND no-plane payload REJECTS; sketch_upsert ops JSON (all op/entity/constraint tags) parses. UX: openSession→bool; plane-pick failure re-shows picker w/ error hint; existing-sketch failure falls back to model (hint preserved across exit's clear). 603/603 FE + 211 core + clippy clean.

## SKETCH-E2E-FIX — stale sidecar + Codex findings (2026-07-20, SHIPPED; Codex plan-review "revise"→deltas applied)
- [x] P1 restaged via scripts/build-worker.sh (Release, 2.2MB) + refreshed target/debug copy; both selftests show `upsert dof=12` solver marker
- [x] P2 resolver `prefer_dev` (cfg!(debug_assertions)): dev-tree build beats staged sidecar in debug (stale-drift class killed), debug-only shadow warning, 6 unit tests pin both orders, PACKAGING.md updated
- [x] P3 AWAITED DeleteSketch compensation (wire `{cmd:"deleteSketch", sketch:<uuid>}` pinned; cleanup-failed suffix surfaces; retry mints fresh); mode-flip bail cancels+deletes fresh create; typed documentStore.removeSketch; nextSketchName single source (sketchNameSeq dead); PlanePicker updateMatrixWorld constructor+hitTest (test mask removed)
- [x] P4 verified: FE 610/610 + build; cargo --workspace 30 suites 0 fail + clippy clean; browser walkthrough — IMMEDIATE quad click enters correct vertical plane (ViewCube BACK, ortho). USER gate: relaunch `npm run tauri dev` + draw

## SKETCH-STATIC-LAYER — always-visible selectable sketches in model mode (P3b, 2026-07-20, SHIPPED)
- [x] Rust `get_sketch`: pure read (sketch_or_err → sketch_wire, dof/status from sketch_solve cache, never-solved → dof 0/UnderConstrained), no worker/events; registered; 3 runtime tests. 68/68 lib + clippy -D warnings clean
- [x] TS: SketchStaticLayer (286 ln; per-sketch group: LineSegments curves + Points dots + region-fill mesh from finishSketch previewTriangles (u,v)→plane; hover/selected tints via existing tokens; hitTest w/ 8px line threshold + updateMatrixWorld) + sketchStaticSync (documentStore diff + visibility, toolStore edit-hide + exit refetch, selection mirror, loadSeq latest-wins); CadClient.getSketch (tauri pure invoke; mock lane peekSession/seeded)
- [x] Picking: Picker onHover/onPick carry coords + secondaryHoverKey (hover-key coalescing across sketches); ViewportRoot empty-click fallback → {kind:"sketch"} selection → feeds EXISTING armExtrude/RevolveFromSelection. BONUS FIX: Picker traverse→traverseVisible — hidden bodies were pickable (own-flag check missed group-level hide; walkthrough-caught)
- [x] Verify: FE 632/632 + build; Rust 68/68 + clippy; browser walkthrough — seeded sketches render (curves+dots), viewport edge-click selects "Sketch 2" w/ real DOF, tree syncs. USER gate: real app draw → finish → sketch visible w/ fill → click → Extrude

## AC-USABILITY — sketch/extrude gap-closure (plan approved 2026-07-20, `~/.claude/plans/act-as-senior-software-twinkling-pony.md`; Codex plan-review "revise" → deltas folded)
Target: AC1 sketch-on-plane, AC2 shapes+constraints+dims (legacy parity), AC3 extrude. Bugs: BUG-1 Tangent wire-dropped, BUG-2 angle deg/rad ×3 sites, BUG-3 OverConstrained never emitted, BUG-4 orphan points on delete, BUG-5 re-entry representation split.
- [x] F-WP-S0 re-entry hydration + centerRef + angle canonicalization (2026-07-20): centerRef emitted in wire_entity (NOT core serde — core Circle.center already an EntityId ref; fixtures untouched), ownershipFromWire (child points skipped as entities, `${id}.Center` seeded, constraint refs remapped to {entityId,position}), angleUnits.ts deg↔rad at toWireConstraint + setDimension + hydration (constraintValue cache UI-domain), marshalUpsert child-point removal (absorbed BUG-4 fix), sketch_reentry.rs real-worker gate (center drag (20,20)→(25,35) proven, orphan-free delete dof-parity, angle π/2→edit π/4 round-trip) — GATE PASSED (orchestrator-verified)
- [x] F-WP-S1 sketchSelectionStore + sketchHitTest (pickDimensionTarget delegates, behavior-preserving) + SketchObject hover mat (selected>hover>construction>status) + engine.setSketchHover + ViewportRoot subscription + enter/exit clear — GATE PASSED
- [x] F-WP-S4a constraintTarget.ts (toConstraintTarget/resolveTargetPoint, Point-alias normalize) + constraintApplicability.ts (verbatim C++ matrix, hasLineBetweenPoints=coord-coincidence returning joining-line target, WIRE_ENCODABLE pre-filter, deterministic order) 51 tests — GATE PASSED. Equal/Midpoint/Tangent user-apply deferred (C++ parity)
- [x] F-WP-M1 regionPick (2026-07-20): controller FSM state + armGen generation guard after every await, pickable=profileFromRegion filter, single-region + re-edit paths byte-identical, RegionPickLayer engine object (plane-basis fills, token tints), pure regionAtPoint hit-test, orbit suppressed + capture-phase Esc, picked regionId → draft/commit (extrude :330, revolve :516) — GATE PASSED
- [x] W-WP1 4-state upsert_state (Conflicting>OverConstrained>FullyConstrained>UnderConstrained) + redundant drag/end status (gesture-fixed at BeginGesture — drag pins would false-flag), Sketch::hasRedundantConstraints(), 3 new fixtures, ctest 64/64 + m2_gate 2/2 vs new binary; SCHEMA §7.3/§7.4/§14 integrated + sidecar restaged — GATE PASSED
- [x] T-WP-scaffold Playwright harness (chromium+swiftshader, serial, port 4177 default — 1420 = Tauri dev collision found+fixed) + line/rect/circle/arc specs via REAL plane-picker path, 0 app-code changes, 4/4 ×2 — GATE PASSED. FOUND REAL BUG: Enter finish-sketch never auto-armed extrude (setPendingExtrude before setMode("model") → consumer guard dropped it) — fixed inline (useShortcuts.ts order swap), 7/7
- [x] F-WP-S3 deletion (2026-07-20): sketchService deleteEntities/deleteConstraints (cascade predicate mirrors Rust `c.entities.some(doomed)`; child-point removeEntity already synthesized by S0 marshal), Delete/Backspace SKETCH_KEYS → useShortcuts (empty selection falls through, no preventDefault), sketch_edit.rs 3/3 real-worker (RemoveConstraint dof-rise — first-ever coverage, cascade, unknown-id noop) — GATE PASSED
- [x] F-WP-S2 select tool (2026-07-20): selectGesture.ts pure helpers + selectActive controller path (gated tool==="select", draw/dimension byte-identical), click/shift-toggle/miss-clear, point-handle drag via gesture lane (rAF-coalesced solveDrag latest-wins by seq, immutable dragBase + engine-only preview, endGesture one-undo commit, fast-flick double-end guard), Esc = endGesture(originalCoord) (no wire cancel verb — no-op undo step seam noted), 30 new tests — GATE PASSED. Orchestrator follow-up fixes: tauriClient.solveDrag now reverse-maps positions like endGesture (real-worker drag preview was frozen — one-line symmetry); e2e line.spec .first() vs per-row list
- [x] F-WP-S4b constraint apply (2026-07-20): toWireConstraint +5 kinds (fixed{at}/onCurve/tangent/concentric/symmetric — BUG-1 closed, autoconstrain Tangent reaches solver), applyConstraint reject-on-conflict, dimensional → toolChipStore.showDimension chip flow (type-directed Distance vs Angle), SketchConstraintToolbar pill + ConstraintContextChips (overlay at selection centroid, shared useApplicableConstraints), sketch_constraints.rs real-worker DOF proofs (Fixed −2, Concentric −2, Tangent −1, Symmetric −2, OnCurve −1) — GATE PASSED
- [x] F-WP-S5 per-row ConstraintList (2026-07-20): row = glyph/label/entity-summary/value(Angle °)/delete, hover → constraintHover → engine highlight (entity hover wins), delete → deleteConstraints fire-and-forget, ConstraintList+InspectorPanel test rewrites enumerated — GATE PASSED. Debt: glyph map now duplicated ×3 (badgeLayout/ConstraintList/constraintCatalog) — consolidation candidate
- [x] F-WP-S6 Trim + Mirror (2026-07-20): Trim = click→hitTest→deleteEntities (cascade); Mirror FSM (phase from selection: empty=select clicks, non-empty=plain click on non-selected Line = axis; Shift keeps editing set), entity matrix Point→Symmetric / Line→2×Symmetric / Circle→Symmetric+Equal / Arc→copy-only (endpoints not solver handles), arc reflection = endpoint swap (2φ−θ, matches legacy MirrorTool), ONE upsert commit + selection→copies, mirrorMath.ts pure + 23 tests — GATE PASSED
- [x] T-WP tests (2026-07-20): hole_extrude ctest (rect 40×20 + circle hole r5 → (800−25π)·10 exact to 1e-13, 7 faces; no-hole control 8000/6; NO defects — pipeline correct), ctest 65/65; acceptance.spec.ts (AC1 plane-pick → constraint apply via toolbar (Fixed on vertex; H/V pre-applied by autoconstrain — reject-path avoided) → Distance dim typed → Delete round-trip → Enter auto-arm → real handle-drag extrude → Body row) + multiregion.spec.ts (rect+circle 2 regions, mock orders circle first → clicking RECT proves non-first pick → armed → commit → Body); camera-tween settle helper + plane-point forward-projection targeting; e2e 6/6 ×5 runs — GATE PASSED
- [x] EXTRUDE-PREVIEW-FIX + Codex implementation review (2026-07-21, terra/high over 1529c9f..HEAD): USER-REPORTED twisted-prism L1 preview vs real worker — ROOT CAUSE: profileFromRegion assumed mock FAN layout (positions[0..1]=centroid hub) but real SolverLane ear_clip emits hub-less polygon vertices → ring dropped vertex 0, handle centroid = a corner (revolve lathe same). FIX: ring derived from triangulation topology (boundary edges used once → chained loop → largest-|area| → CCW), ringCentroid area centroid for bounds+side normals; 3 ear-clip regression tests (hub-less ring, CW→CCW, multi-loop). Codex independently confirmed root cause + 3 P2 findings ALL verified+fixed: (1) rapid constraint applies raced shared id-map → sketchService mutation queue (enqueueSketchMutation serializes all 5 exported mutators, session read inside queued turn; delayed-client test); (2) SolveDrag deltas are incremental vs PREVIOUS response (g.last_reported) but preview reapplied each onto dragBase → coupled points snapped back; dragAccum merge (two-response test); (3) select-tool idle pointermove never wrote hover → S1 hover path dead; idle hitTest → setHover w/ sameSketchSel dedup (test). FE 838, e2e 6/6 — GATE PASSED
- [x] TRACKPAD-NAV (2026-07-21): macOS/precision-touchpad navigation — two-finger scroll = pan, shift+two-finger = orbit, pinch = zoom-to-cursor; mouse wheel keeps dollying. All routing in a PURE reducer `viewport/engine/navInput.ts` (`navReduce(state, ev, ctx) -> {state, op, device, detected}`): per-segment device classification with a carried prior (a lifetime latch would strand a docked MacBook in trackpad mode), definitive-evidence override (deltaMode!==0 ⇒ mouse, small ctrl-wheel ⇒ trackpad — a BIG integer ctrl-wheel is a mouse user holding Ctrl, deliberately not trackpad), shift latched at segment start so a momentum tail never flips orbit→pan, and single-owner pinch arbitration (first source claims it for the session + 200ms post-`gestureend` window, so a WebKit gesture and a ctrl-wheel tail can never double-apply). `CadOrbitControls` is now a thin adapter; ops arrive pre-scaled so `PAN_SENS`/`ORBIT_SPEED` stay the single camera-feel constants. Also fixed along the way: `deltaMode` was never normalized (a DOM_DELTA_LINE mouse got exp(0.0045)≈no zoom); shift+wheel is folded onto deltaX by macOS so it did nothing AND scored as trackpad evidence (would have flipped a mouse user into panning); RMB drag now pans instead of orbiting; canvas `touch-action: none`; meta viewport blocks WKWebView page magnification. Settings → Navigation: Auto | Trackpad | Mouse (persisted v2→v3 migration), Auto shows live detection from a NON-persisted `viewportStore.detectedInputDevice`. Tests: navInput 52, first-ever CadOrbitControls behavioural suite 20, e2e navigation 11 (synthetic `page.mouse.wheel` is a MOUSE stream — trackpad paths use the override). FE 921, e2e 17/17. PENDING: `?inputprobe` diagnostic still in tree; constants are reasoned placeholders awaiting the real-hardware log
- [x] ORBIT-POLE-FIX (2026-07-21): USER-REPORTED view self-rotating near TOP. ROOT CAUSE: `CameraRig.apply` swapped camera `up` world-Z→world-Y once `|offset.z|/len > 0.99999`, flipping the basis in one frame; near the pole `lookAt`'s roll also depended on a vanishing horizontal component, so it spun faster and faster on approach. FIX: build the turntable basis analytically — right = normalize(-offset.y, offset.x, 0) (horizontal, yaw-only, no degeneracy), up = right × forward — continuous through the poles with no self-alignment. Only an explicit ViewCube/named-view snap changes heading; `yawForDirection` supplies the canonical -90° heading for an EXACT pole snap, where yaw is genuinely undefined and `atan2(0,0)` would silently pick a sideways TOP. 7 CameraRig orientation tests + e2e no-self-align
- [ ] Manual Mac gate (USER): `npm run tauri dev` — AC1 plane-pick sketch; AC2 draw line/rect/circle/arc, select/drag points (hover highlights now live), apply constraints (toolbar + chips), dimensions incl. angle, delete entity+constraint (Delete key + list ×), trim, mirror; exit + re-enter sketch, repeat drag/delete; AC3 extrude chosen region of multi-region sketch — preview must be a clean prism now; undo sweep
- [ ] TRACKPAD-NAV Mac gate (USER): `bun run tauri dev` — (1) two-finger scroll pans, incl. horizontal/diagonal, no page scroll or rubber-band; (2) shift+two-finger orbits, and releasing shift mid-momentum does NOT flip to pan; (3) pinch zooms toward the cursor and the page itself never magnifies; (4) plug in a mouse WITHOUT restarting — wheel zooms again, Auto sublabel flips to "mouse", RMB pans; unplug → flips back; (5) Trackpad/Mouse overrides pin behaviour; (6) shift+two-finger while a tool is ARMED orbits, but during a depth/radius/angle DRAG it does not and the value does not jump; (7) idle = 0 frames. THEN: run `?inputprobe`, capture `window.__inputProbeDump()`, calibrate the navInput constants against it, and DELETE `src/viewport/debug/inputProbe.ts` + its ViewportRoot mount

## Wave plan (user-approved 2026-07-17; pause LIFTED — "continue autonomously, implement full plan")
- Autonomous run to full plan: W-WP6-R ∥ R-WP11 ∥ repair.rs scoringVersion fix → R-WP12 solver bridge ∥ F-WP8 → M2 gate → /codex-implementation-review → M3 packaging → M4 → M5 → M6.
- Wave A (parallel): W-WP5-R independent review + R-WP10 app shell.
- Wave B: W-WP6 (after W-WP5-R clean) ∥ R-WP11 (after R-WP10).
- Wave C: F-WP8 → M2 gate → /codex-implementation-review (approved for M2 AND M4) → M3 packaging.
- D1 APPROVED: worker-minted deterministic BodyIds `body_<opId>` (splits later `body_<opId>:<k>`); Rust adopts from bodyEvents + validates; SCHEMA amendment in Wave A.
- D2: ExportStep lands in W-WP6 (M2 needs it). D3: primary.topoKey removed in W-WP6.

## SKETCH-HARDEN — production hardening + UX (plan approved 2026-07-21, `~/.claude/plans/act-as-senior-software-cryptic-taco.md`; Codex plan-review terra/high "revise" → B1/B2/B3+H1/H2+M1-M3 all folded)
Waves: W0 correctness (C0 Rust squash txn, C1 mutation coordinator, C2 FE sketch undo, C3 dimension validation, C4 degeneracy minSize, C5 dispose parity, C6 mock DOF, P2 hover rAF, T1 dedup) → W1 conflict-ids protocol change (RISKY) → W2 UX (finish/cancel, statusHint, chain close, badges/format/glyphs, dim bodies, cursors) → W3 real trim (RISKY) → W4 snap cache → W5 tests+debt. Commit per wave.
- [x] W0 correctness gate (2026-07-21): C0 Rust squash txn (undo.rs squash + squash_sketch_session full-replace forward / RestoreSketch exact inverse; enter/finish/cancel watermark wiring; sketch_squash.rs 3/3 real-worker) + C3-Rust positivity guard (Distance/Radius/Diameter only, H/V signed non-regression pinned); C1 mutation coordinator (exported queue + flushSketchMutations, sessionGeneration fence after every await, transactional id-map clone-commit in tauriClient.sketchUpsert, finishSketch drains queue); C2 FE sketch undo (snapshot stacks cap 50, ⌘Z mode-gated, editConstraintValue coalescing; orchestrator fix: failed undo/redo un-pops stacks); C3-UI kind-aware DimensionInput validation (no-kind path byte-identical); C4 minSize=4px zoom-normalized degeneracy (line/rect/circle/arc); C5 dispose parity + disposed rAF guards; C6 mock DOF Concentric/Midpoint=2; P2 hover rAF coalescing; T1 SNAP_PX export + inferHV dedup. Suites: FE 956/92 files, build clean, e2e 17/17, Rust 402/0 + clippy/fmt clean. Hex-gate hit = pre-existing inputProbe.ts (tracked :125).
- [x] W1 conflict feedback gate (2026-07-21): per-constraint conflict ids on EVERY solve surface — worker on_upsert/on_end emit `conflicting` (on_end = final-solve else gesture-fixed precedence), SCHEMA §7.4+§14 additive amendment SIGNED OFF, SketchUpsertDto+SketchSessionDto+wire str_array parse, enter_sketch threads ids (re-entering a conflicting sketch tints rows), FE frontendConflictingIds reverse map (unknown dropped) at upsert/drag/end/enter — **fixed latent solveDrag raw-UUID bug**, sketchStore.conflictingIds single-ownership reducer (solve replaces / enter seeds / exit clears / reject-restore replaces), ConstraintList+badge `text-traffic-close` tint, reject hint names clashing constraint. Independent adversarial review: **APPROVE-WITH-FIXES, 0 blockers** — applied: unconditional endGesture conflict write-back (stale-tint on error path), clean-sketch emptiness pinned in sketch_conflicts.rs (subset matcher can't), inert serde(default) removed, fixture comment honest. Proofs: sketch_conflicts.rs 2/2 vs real worker (ids = authored constraint uuids both surfaces; clean ⇒ empty), ctest 65/65, Rust 405+2, FE 985+, clippy/fmt clean.
- [x] W2 UX gate (2026-07-21): U1 Finish≠Cancel (Finish → runAction finishSketch: queue drain + auto-arm; Cancel plain exit); U3 statusHint {message,severity,sticky} — 4s auto-dismiss non-sticky, token-guarded latest-wins timer, 28 prompt/22 error/~20 info classified sites, StatusBar error tint, fileActions bespoke 2.5s timer removed; U4 line chain: anchors accumulate, same-point click / Enter ends chain, click-first-anchor closes loop (capture-phase Enter skips INPUT/TEXTAREA, falls through to finishSketch when no chain); U6 formatDimensionValue (≤3dp trim zeros) ×6 sites + no-op gate compares formatted (spurious rounded-commit-on-blur killed); U7 bodies dim 0.35 in sketch (shared face material, full state save/restore, edges stay); U8 crosshair cursors draw/dimension tools; U9 badge offsetIndex stagger (quantized-anchor groups, 16px row, margin not transform — overlay driver owns transform); U10 glyph single-source constraintCatalog (Coincident "•", H↔/V↔ collision fix); mirror idle hover via rAF path; T2 angleUnits.test.ts 8 cases. e2e acceptance spec updated to trimmed format. Suites: FE 1023/98, build clean, e2e 17/17. Debt noted: vitest parallel-run flake in useShortcuts/InspectorPanel single-macrotask flush (pre-existing, W5).
- [x] W3 real trim gate (2026-07-21): trimMath.ts pure parametric trim — param-returning crossings (line t / arc sweep-relative s incl. 0-rad crossing / circle ring wraparound), ε=1e-9 param-space dedupe, endpoint-crossing exclusion, tangency counts, minSize pieces merged into doomed span (ghost honest), Line≤2/Arc≤2 CCW/Circle≥2→complementary arc, null ⇒ whole-delete fallback; trimEntity queued+fenced+undo("trim")+cascade; destructive doomed-piece hover ghost (palette.destructive = --color-traffic-close rgb-mirror, SketchObject.setTrimGhost Line2 overlay, cleared on click/tool-switch/exit/session-update — rejected upsert can't strand it); sticky prompt. Independent adversarial review: **APPROVE, 0 blockers** (hand-traced 350°→20° arc + 90°/270° circle counterexamples; degenerate inputs all graceful; ghost lifecycle leak-free; MINOR epsilon doc fixed). Suites: FE 1048/100 (35 new trim), build clean, e2e 18/18 (sketch-trim spec new). NOTEs accepted: pieces inherit no constraints (V1, documented), full-turn-arc extreme degenerate unguarded.
- [x] W4 perf gate (2026-07-22): buildSnapCache — entity-derived candidates (guide/quadrant points, O(n²) all-pairs intersections, H/V refs) precomputed once per edit, keyed by session.entities REFERENCE (immutable-replace verified); computeSnap additive optional cache param, onCurve stays live; behavior byte-identical (378-comparison equivalence property ×6 fixtures, original 42 tests unchanged); controller invalidation spy-tested. FE 1057, e2e 18/18 at land time.
- [x] W5 tests+debt gate (2026-07-22): T2 angleUnits 8 tests; T3 SketchController.draw.test.ts 8 integration tests (pointer→snap→FSM→queued commit ×4 tools, degenerate re-arm, chain close, Enter mid-chain, minSize zoom-normalization ×2 scales); T4 e2e +5 specs (sketch-undo full ⌘Z/⇧⌘Z round-trip, sketch-drag gesture lane, sketch-mirror reflection asserted to 6dp, sketch-snap endpoint float-exact coincidence, sketch-degenerate ×3 tools) — 23/23 ×2 no flake; T5 stale comments (localSolver header = mock-sketch+shared-preview truth, dimensionTool post-W1 seam); T6 solverPolicyHash reserved-slot docs; vitest parallel flake ROOT-CAUSED (bare setTimeout(0) racing mutation chain) → flushSketchMutations() drain in useShortcuts+InspectorPanel tests, 12/12 repeat runs each.
- [x] FINAL GATE (2026-07-22): FE 1065/102 · build clean · cargo 406/0 vs real worker · fmt+clippy clean · ctest 65/65 · e2e 23/23 · hex gate 0 (sole hit = pre-existing inputProbe.ts, tracked :125)
- [ ] Manual Mac gate additions (USER, `bun run tauri dev`): trim a segment on crossing lines + hover shows red doomed-piece ghost; ⌘Z sweep inside sketch (draw/dim/drag/trim each one step) + AFTER finish exactly ONE model undo step reverts whole sketch session; over-constrain a dimension → reject hint NAMES the clashing constraint + re-enter conflicting sketch → rows/badges tinted red; bodies dim ~35% in sketch and restore exactly on exit; crosshair cursor on draw/dimension tools; line chain: Enter ends, click-first-point closes loop; dimension input rejects ≤0 radius/distance + >3-decimal display trimmed; status errors red + transient hints self-clear ~4s

## MODEL-HARDEN — Extrude/Revolve commit fix + professional UX (plan approved 2026-07-22, `~/.claude/plans/act-as-senior-software-parsed-sunbeam.md`; Codex plan-review terra/high "revise" → all 10 findings folded)
Root cause: append-at-end AddOperation → RegenHint::None → no regen ever scheduled in real app (HISTORY row real, geometry never computed; silent teardown). Waves: W0 regen root fix (core cursor promotion + scheduler_commit.rs) → W0.5 undo hygiene + correlation provenance + error surfacing → W1 commit-gesture rework (release keeps armed, Enter/✓/click-away) → W2 booleans + multi-select→N ops → W3 revolve full (worker split fix + SCHEMA §7.2 amendment + revolve_ops.rs) → W4 final gate.
- [x] W0 regen root fix (2026-07-22): frontier append joins applied prefix (session.rs cursor promotion, cursor_changed delta, header table), rolled-back append stays draft; lib.rs pub regen_driver_with_emitter seam (RegenEmitter, zero app behavior change); DocumentProjection +appliedOps/totalOps (additive, TS optional mirror); 4 core tests + scheduler_commit.rs 2/2 REAL worker through PRODUCTION driver (would have caught the defect; redo-draft regression pinned). Gate A: cargo 412/0 + clippy/fmt clean, FE 1065 — GATE PASSED (orchestrator-verified)
- [x] W0.5 (2026-07-22): squash all-or-nothing guard (trailing_run == count else keep granular — clamp would corrupt undo; interleaved-op tests core + real-worker sketch_squash 2 new), enter_sketch same-sketch watermark keep, model-mode arms → pure getSketch ×3 (+ lane.cacheSketchPlane in tauriClient.getSketch), correlation provenance (RegenReport.source_revision all outcomes, RegenFinished{sourceRevision,message}, FE awaiter targetRev table: publish≥R resolves / failed≥R → errorMessage / superseded+cancelled ignored / noop≥R empty / lastPublishedChange missed-event buffer; rapid_double_commit real-worker test), error surfacing (FeatureDto.statusMessage → FeatureMeta → HistoryList tint+tooltip; legacy-draft one-shot hint appliedOps<totalOps). Gate B: cargo 418/0 + clippy/fmt, FE 1070 + build — GATE PASSED (orchestrator-verified)
- [x] W1 (2026-07-22): commit-gesture rework — release keeps ARMED (live preview + chip cluster), commit = Enter (capture, input-guarded) / chip ✓ / chip-input Enter via onConfirm contract (DimensionInput single-fire) / click-away (window-capture pointerdown/up, DRAG_PX true-click, chip/toolbar/input/button exclusions, handle-grab = re-drag, region-pick suppressed), Esc cancels via commitGen-invalidated cancelAll + bulk endPreview(false) loop; commitFailed → re-armed preview + hint names errorMessage (work never lost); FSM v2 complete (ExtrudeFsm/RevolveFsm targetPick+booleanMode transitions + regionSelectStep reducer unit-tested now, UI wiring W2; revolve quickCommit DELETED — plain click = click-away 360° commit); toolChipStore v2 handlers + ConfirmButtons/SymmetricToggle cluster; ?vpdebug phase surface; e2e helpers migrated + extrude-commit-gesture/revolve-commit specs. Gate: FE 1088/103 + build + e2e 27/27 ×2 — GATE PASSED (orchestrator-verified)
- [x] W2+W3-FE (2026-07-22): boolean picker [New Body|Add|Cut] segments (0 bodies=disabled, 1=auto-target, >1=targetPick via SAME engine.probePick path as Boolean tool, preview ids rejected, Esc→NewBody, click-away suppressed in targetPick), Cut→destructive preview tint (palette.destructive, prisms+lathe+L2), negative-drag one-shot Cut tip, buildOpFromSession booleanMode/targetBodyId pass-through; multi-select regions (regionSelectStep wired: toggle/dblclick-fast-confirm/Enter/chip ✓, RegionPickLayer.setSelected, single-region fast path byte-identical) → N lane sessions → sequential commit stop-on-first-failure ("k of n failed" hint, committed stay, failed re-beginPreview'd, remaining kept; pre-loop onBodyLoaded set fixes early-body reconcile race) → select all, sketch auto-hide on full success; revolve N-op loop + all-regions axisSplitsRegion validity + boolean segments (L1 lathe = primary region only, doc'd); mock Add=concat/Cut=no-op parity doc'd. Gate: FE 1108/105 + build + e2e 29/29 ×2 (+multiselect ×3) + hex 0 — GATE PASSED (orchestrator-verified)
- [x] W3-backend (2026-07-22): RevolveOp boolean tail → publish_boolean_result (ExtrudeOp parity; bisecting Revolve Cut now splits deterministic body_<opId>:<k>), SCHEMA §7.2 widened to boolean-mode Extrude/Revolve any non-NewBody mode + §14 entry SIGNED OFF (fixture grep: no revolve payloads, no bump), test_revolve_split ctest (frame-mapped washer cut → exact 8000/8800 children, replay-stable), revolve_ops.rs FIRST real-worker Revolve wire coverage 5/5 (Pappus fine-LOD 9409.161 obs vs 9424.78, bound 40 w/ 2.6× headroom; 180° half; Cut split adopted via split_child_uuid exact volumes; stale-axis → 0 bodies + StepState::Error + statusMessage "not found in sketch"; explicit + "" region binding). Gates: ctest 66/66, cargo 423/0 + clippy/fmt — BACKEND GATE PASSED (orchestrator-verified; FE parity rides W2/W3-FE)
- [x] W3-FE revolve parity — folded into W2 gate (see W2+W3-FE above)
- [x] W4 final (2026-07-22): worker/build-w3 throwaway removed; final re-run cargo 423/0 (real worker) + ctest 66/66 + FE 1108/105 + e2e 29/29 + clippy/fmt + hex 0; CURRENT_STATE.md MODEL-HARDEN section — GATE PASSED
- [x] REVIEW-FIX (2026-07-22): dual independent review — codex exec (terra/high, verdict revise: 1 BLOCKER + 6 MAJOR) + 24-agent 5-dimension adversarial-verify workflow (16 confirmed) → 24 deduped findings ALL FIXED across 3 parallel agents. Key: (B1) published-regen-with-failed-step no longer correlates as success — RegenFinished{failedSteps,affectedBodies} additive DTO, recordId awaiters settle exactly-once on the regen-finished sibling, op-scoped changedBodies incl. split children; correlation state reset on new/open/recover (doc-A buffer can't settle doc-B); sketch_session dropped/re-anchored on undo/redo + set only after solver success; UNDO_CAP eviction counter refuses unsafe squash; production driver emits noop completion on empty plan; EngineFailed+moved-fencing → Superseded; SCHEMA §7.2 split wording contradiction (":563 deferred/parent-survives") fixed + §14 note; SVG click-away closest() fix; dispose ends lane sessions; beginRevolveArmed post-await guard; bounded 4s body-load wait; re-arm epoch/L2-handle/lathe cleanups; 0° revolve confirm refused; booleanMode whitelist; wrong-regionId loud-fail wire test; e2e mid-drag phase assertions; worker single-solid Add/disjoint-Cut/Intersect exact-volume ctests. FINAL: ctest 67/67 · cargo 437/0 · FE 1125/105 · e2e 29/29 · clippy/fmt/hex clean — GATE PASSED (orchestrator-verified)
- [ ] Manual Mac gate (USER, `bun run tauri dev`): triangle sketch → Finish → extrude drag → release stays armed w/ chip cluster → Enter → body PERSISTS after tool close AND save/reopen; ⌘Z once = extrude gone, twice = sketch session; rect+circle sketch → multi-select both regions → confirm → drag → Enter → 2 bodies + 2 rows; Cut on existing body subtracts; revolve triangle edge 360° Enter; stale-region error = named hint, never silence; toolbar-icon click while armed switches tool WITHOUT committing

## MODEL-OPS — sketch-based modeling correctness + breadth (plan approved 2026-07-26, `~/.claude/plans/act-as-senior-software-gleaming-whistle.md`)
Scope chosen by user: W0 profile correctness + W1 extrude end conditions/Chamfer + W2 sketch-on-face/datums + W3 backend preview verb. **Loft/Sweep explicitly OUT** (stays backlog :78).
- [x] W0 profile correctness — holes end-to-end (2026-07-26): ROOT DEFECT — `SolverLane` ear-clipped a region's OUTER loop only while `FaceBuilder` builds the face WITH hole wires, so a rect+inner-circle preview showed a slab and the commit produced a tube; the same triangles are the region hit-test, so a click INSIDE a hole selected the enclosing region. Fix: new `worker/src/loop/PolygonFill.{h,cpp}` (extracted from SolverLane so it is unit-testable) — bridge-based hole merging with SHARED (non-duplicated) vertex indices so every bridge lands in exactly two triangles and reads as an INTERIOR edge; shortest valid bridge per hole (no crossing, no vertex touch, midpoint inside the merged loop and outside every pending hole); an unbridgeable hole is SKIPPED (fill degrades, never corrupts) and reported via additive `previewTriangles.holesSubtracted`. **The shared-index/bridge-interiority property is load-bearing, not cosmetic**: the frontend recovers its extrusion rings from single-use-edge topology, so a duplicated bridge vertex would fabricate a phantom wall — pinned by `bridges_are_interior()` in `test_polygon_fill` (9 cases: areas exact, multi-hole, concave-L, winding normalization, closing-point dedup, degenerate graceful, determinism). FE: `PrismProfile` gains `holes: [number,number][][]`; `profileFromRegion` keeps ALL closed loops (largest-|area| = outer CCW, rest = holes CW) instead of discarding all but the largest; `prismLocal` grows an inner wall per hole via a shared `addWall(ring, outward)` (hole rings wind CW ⇒ triangle winding flips for free; normal negated so it points INTO the hole); caps come straight from the producer's triangulation so they are correct with zero change. `regionAtPoint`/`RegionPickLayer` needed NO change — the hole is simply absent from the triangles — pinned anyway. Mock parity (e2e is the mock lane, so without this the tube was untestable): `detectRegions` now mirrors the worker's containment rule — a circle wholly inside the closed loop folds in as a HOLE (crossing or separated circles stay their own region, so `multiregion.spec.ts` is unaffected), with a ray-sampled quad-strip `annulusTriangles` (parameter-based and angular stitching were both tried and both self-overlap; sampling BOTH rings on the shared sorted direction set is non-overlapping by construction and preserves the outer corners). Cleanups: SCHEMA §7.3 profile-binding prose corrected (it documented a `SketchRegion` semantic ref in `inputs[]` that no layer has ever produced or consumed — the flat `params.sketchId`/`regionId` form is now normative, with what `inputs[]` genuinely carries spelled out) + §7.4 hole-subtraction semantics + bridge-topology requirement + `holesSubtracted`, ×2 §14 entries SIGNED OFF (no fixture bump — no canonical fixture carries a hole-bearing region). `Sketch::set_regions` reviewed for deletion and KEPT: it is pinned by the frozen document serde (`sketch_freeze.rs`) and v1 corpus samples carry the key, so removing it is a schema change for zero functional gain — doc now states plainly that nothing populates it and why (worker is asked live on every `finish_sketch`/arm). NEW `src-tauri/tests/sketch_regions.rs` 3/3 vs REAL worker (contained circle nests as ONE region with 1 hole + fill area = 40·20−π·5² ≈ 721.5 not 800; boundary-edge count == vertex count, i.e. bridges stayed interior across the whole wire; plain-rectangle control). `?vpdebug` +`profileHoleCounts`; e2e `sketch-hole-extrude.spec.ts`. GATE: FE 1139/105 (+14) · build clean · cargo 440/0 vs real worker · ctest 68/68 (+1) · e2e 30/30 (+1) · clippy/fmt clean · hex 0 (sole hit = pre-existing inputProbe.ts, tracked :125)
- [x] W1 extrude end conditions + real Chamfer (2026-07-26): the worker has implemented Blind/ThroughAll/Symmetric/ToNext/ToFace + twoDirections + draft since W-WP6 and the wire type always carried the fields — the TOOL authored only Blind/Symmetric, so most of the kernel's extrude was unreachable. FSM: `ExtrudeEndCondition = Blind|ThroughAll|ToNext|ToFace` (Symmetric deliberately NOT a member — it stays the ⇔ toggle so there is ONE representation; the marshaller folds them: `extrudeMode = symmetric ? "Symmetric" : endCondition`), new `facePick` phase w/ per-direction `facePickFor`, `setEndCondition`/`setDraftAngle`/`setTwoDirections`/`setDepth2`/`pickFace`/`cancelFacePick`. **Symmetric ⊥ twoDirections enforced in the reducer** (worker rejects the combination at `ExtrudeOp.cpp:284` — made unrepresentable rather than doomed-at-commit); abandoning a face pick falls back to Blind so an unreachable ToFace can never be armed. ToFace target = TYPED semantic ref (FE `ExtrudeParams.targetFace/targetFace2` were missing entirely) built via the SAME probe→`promoteSelection`→ref chain Shell uses — a bare id carries no anchor/intent and could not be ladder-rebound (SCHEMA §7.3 Invariants 2/3); `faceElementRef` mirrors `edgeElementRef`'s bare-uuid body normalization; marshalled ONLY when the matching mode is ToFace (SCHEMA "absent for non-ToFace") so a mode switch cannot strand a stale ref. Chip: end-condition segment group, distance input + ⇔ hidden for the derived conditions (the kernel computes the distance), body-reaching conditions disabled at 0 bodies. **Chamfer**: was absent from BOTH the `ModelTool` union and the authorable `WireOperation` union — no UI path existed despite worker `execute_chamfer` + SCHEMA + `ChamferParams` all shipping. Added as one shared edge-op lane over `FilletFsm` with a `kind` discriminator (a parallel byte-identical reducer was written, then deleted — two copies of the same gesture is two places to drift); toolbar + icon + `H` key (F=fillet, ⇧F=zoom-fit). **Two latent defects fixed on the way**: (1) `default_label` keyed off the coarse `FeatureKind` bucket, so a Chamfer read "Fillet", a Shell read "Fillet" and a Linear Pattern read "Boolean" in the history tree — now keyed off the Operation, with `Operation::op_type()` added to core; (2) `InspectorPanel.editFeature` routed re-edits on `kind`, which the backend folds — so Chamfer would have opened the fillet editor and Shell/patterns/Mirror were unreachable on the REAL lane (mock-only kinds masked it) — new additive `FeatureDto.opType` (+TS mirror, mock stamps it) and re-edit routes on it with a `kind` fallback. Proofs: `wire_contract.rs` +5 REAL-worker (ThroughAll Cut 7000, ToNext Cut 7000, two-direction Add 8400, draft-removes-less-than-straight, Chamfer reaches `execute_chamfer`), FSM 13 new cases, marshalling 6 (incl. the two absent-unless-ToFace rules), chip render 5, e2e `extrude-end-conditions` + `chamfer`. GATE: FE 1162/105 (+22) · build clean · cargo 445/0 vs real worker (+5) · ctest 68/68 · e2e 32/32 (+2) · clippy/fmt clean · hex 0
- [x] W2 sketch-on-face + datum resolution (2026-07-26): `SketchAttachment::HostFace` was typed in Rust AND mirrored in C++ since M1 with **zero constructors**, so every sketch that ever reached the worker was a world XY/XZ/YZ plane — the cap that limited the app to single-sketch parts. **No worker change was needed**: `plane_kind_str` already mapped a hostFace attachment to wire kind `"custom"` and `WireSketch::parse_plane` has always accepted an arbitrary custom basis; the attachment simply had no producer. W2a: `ElementQuery` seam (a separate trait, like `MeshProvider` — `QueryElement` is a read-only side query that neither mutates nor fences, so it does not belong on the regen-contract `GeometryEngine`) + `WorkerManager`/`PendingBackend` impls + `BackendBundle` widened to 5 facets; the `QueryElement` verb had shipped since W-WP6 with NO Rust caller at all. `face_sketch_plane` command derives the plane from the KERNEL's descriptor (`surfaceType`/`center`/`normal` — for a planar face the bbox centre lies ON the plane, so `{center, normal}` is exact), **rejects a non-planar face loudly** instead of approximating, and takes `snapshotId` from the caller like `promoteSelection` so the pick resolves against the snapshot its mesh was tessellated at; worker IO runs OUTSIDE the runtime lock. W2b: `plane_from_point_normal` in core — `x = normalize(z × n)`, falling back to `n × x_seed` when n ∥ ±Z, then `y = n × x`; **the fallback ORDER is load-bearing** (`z × n` would give the XY basis mirrored) so a sketch on a flat top face reproduces the named XY basis exactly, and any other face gets a level `x`. 6 lock tests (orthonormal + right-handed over 5 normals, ±Z fallbacks, bitwise determinism, scale invariance to tolerance, degenerate → XY not NaN) — the frame is frozen with the sketch and every entity coordinate is expressed in it, so a drifting rule would silently rotate existing sketches on reopen. `buildAddSketchOnFace` + `EnterSketchTarget.newOnFace` + `tryEnterOnSelectedFace` (a selected face wins over the world-plane picker; a promote fills in a missing ElementId so the attachment never stores a snapshot-scoped TopoKey). W2c: pure `resolve_datum` + `DatumContext` in core (OffsetFromPlane carries the base axes over verbatim; OffsetFromFace/AngledFromEdge return **None** without their context rather than resolving somewhere arbitrary; ThreePoint reserved → None), 7 tests. **V1 POLICY: the frame is FROZEN at creation** — matching the OneCAD-CPP oracle (`datum.rs`: a sketch on a datum "copies the resolved frame at creation, frozen, like sketch-on-face"). Upstream edits leave a face-hosted sketch where it was; re-derivation needs the dormant `UpdateSketchAttachment` + a regen epilogue = separate WP. Proofs: `sketch_regions.rs` +2 REAL-worker — a custom-basis sketch solves (dof 8) and derives 1 region with plane-local area 800 (basis-independent), and **an extrude off a sketch hosted 10mm up lands at bbox z = [10, 15]**, not at the origin (a dropped basis would give [0, 5]). SCHEMA §7.3 custom-plane provenance note + §14 (text-only, no wire change, no fixture bump). NOT DONE (flagged): datum CREATION UI + viewport datum layer + `addDatumPlane` in the FE wire union — the resolver and the core command/undo exist, the authoring surface does not. GATE: FE 1162/105 · build clean · cargo 460/0 vs real worker (+15) · ctest 68/68 · e2e 32/32 · clippy/fmt clean
- [x] W3 backend preview verb (2026-07-26): the drag-time "exact" L2 mesh was synthesized in JAVASCRIPT by `mockMeshes.makeExtrudeBodyMesh` — the same function the mock client uses — so a **Cut preview never subtracted** (`mockMeshes.ts:386` said so outright) and Revolve/Fillet/Shell/Boolean had NO 3D preview at all; the commit could produce something the preview had never shown. New kernel-lane `PreviewOp` verb (`worker/src/session/PreviewOp.{h,cpp}`): runs ONE candidate op through **the same `run_single_op` a real plan step uses** (exported from PlanExecutor's anon namespace for exactly this — a preview computed by a parallel code path would be a preview of something else) over a throwaway `bodies_copy()`/`partition_copy()`. **Deliberately NOT `fence_and_clone`** — that takes the fencing path and bumps `snapshot_counter_`; the fencing-free readers are the §7.5 identity-verb precedent. Never calls store_prepared/Accept/Discard, so head bodies, partition, history hash, snapshotId, revision and epoch are all untouched — a preview is invisible to fencing. Only op-touched bodies are tessellated (a drag re-issues per frame). Net-new plumbing: the profile sketch is pre-seeded from the committed `SketchStore` because `run_single_op` resolves a profile ONLY from `OpContext::sketches`, which a real plan fills from its own preceding Sketch op. Rust: `PreviewEngine` seam (separate from `GeometryEngine` — a preview participates in none of the regen contract), `WorkerManager`/`PendingBackend` impls, `BackendBundle` → 6 facets, `preview_op` command with worker IO outside the runtime lock; preview meshes ride inline (coarse LOD, touched bodies only) so no bulk-chunk drain, and each blob goes through the same `validate_mesh_blob` the committed meshes do. FE: `LocalSolverDeps.previewOp` OPTIONAL injected dep — `tauriClient` wires it, `mockClient` omits it and keeps the local synthesis, so ONE lane still serves both clients (session bookkeeping/epochs/emitPreviewResult are not forked); a mid-drag kernel refusal keeps the last good mesh instead of flashing. Proofs: `test_preview_op` ctest (NewBody ships only the new body; **Cut modifies the target**; REF_UNRESOLVED/UNSUPPORTED/PROTOCOL_ERROR paths; 5 repeated previews byte-stable — and the head fingerprint (snapshot id + body count + total volume) is re-asserted after EVERY case, which is the load-bearing invariant) + `wire_contract.rs preview_matches_the_commit_and_leaves_no_trace` REAL-worker: **preview Cut = 7500 while the real body stays 8000, revision unmoved, and committing the same op then lands on 7500 — preview == commit, which is the entire point**. SCHEMA §7.6 `PreviewOp` block + §14 (additive verb, no existing shape changed, no fixture bump). NOT DONE (flagged): (1) no latency gate — `solverbench`'s p95 + busy-kernel harness is the template and a preview budget should be pinned there; (2) no worker-side latest-wins coalescing — the dispatcher's is hard-keyed to `SolveDrag`, and `PreviewThrottle` already bounds in-flight to ≤1 per session, so queue depth is bounded by region count, but a slow `ExecutePlan` will still delay a preview; (3) Fillet/Shell/Revolve now CAN be previewed but their tools do not open a preview session yet, so they still show nothing.
- [x] W4 final gate (2026-07-26): FE 1162/105 · build clean · cargo 461/0 vs real worker · ctest 69/69 · e2e 32/32 · clippy/fmt clean · hex 0 (sole hit = pre-existing inputProbe.ts, tracked :125). NOTE: one full-suite e2e run showed a single failure in `extrude-commit-gesture` "click-away"; it passed 3/3 in isolation and the full suite re-ran 32/32 — recorded as an observed FLAKE, not root-caused.
- [ ] Manual Mac gate (USER, `bun run tauri dev`) for MODEL-OPS: (1) rect + circle INSIDE it → preview is a TUBE, and clicking in the hole does not select the region; (2) extrude with Through all / To next / To face against an existing body, plus two-direction and draft — each survives a double-click re-edit; (3) Chamfer an edge (H key) and re-edit its distance; the history tree must say "Chamfer", not "Fillet"; (4) select a body FACE → new sketch starts ON it → draw → extrude; save, reopen, geometry identical; a non-planar face must refuse with a named reason; (5) during a Cut drag the preview actually SUBTRACTS (this is the W3 verb — confirm it is the kernel's shape, not a prism)

## EXTRUDE-REGION-PARITY — exact selected profile + preview/commit parity (2026-07-29, Codex session hit usage limit mid-P3; continuation completed same day by Claude)
Root cause: live Tauri preview bypassed Rust's typed lowering — FE sent nested `params.profile.regionId`, C++ expected flat `params.regionId`, got none, extruded the FIRST region; commit used the correct mapper → preview/commit diverged. Design note: the plan's "opaque `r2_*` ids" shipped as `r_<16hex>` over a `cell-v2|outer{…}|holes{…}` canonical signature instead (RegionTable.cpp) — deliberate, keeps the M4a `migrate.rs` sanitizer compatible; simple hole-free cells keep the legacy FNV outer-loop id.
- [x] P1 worker: shared canonical `RegionTable`; nested/overlap cells independent; material-complete identity; unique legacy fallback; strict hole/section/profile failures; CTest 70/70
- [x] P2 Rust: typed preview op + shared commit lowering; exact region/body ids; stale/NeedsRepair/deletion lifecycle preserved; read-only regions + sketch scheduler propagation; real-worker wire 15/15, regions 5/5
- [x] P3 frontend (continuation, 2026-07-29): persistent `sketchRegion` selection / typed picking / no-fallback one-profile extrude / full-FSM param snapshots (stable opId = commit recordId) / `geometryToken` stale-profile signal were already landed by Codex; continuation closed the 6 open gaps: (1) **commit barrier wired** — confirm flushes final params as newest epoch then `waitForExactPreview` holds `endPreview(true)` until that exact candidate answers; failure → `endPreview(false)` + re-armed commitFailed (work never lost); timeout 4s PROCEEDS (backend re-validates authoritatively; barrier surfaces in-flight failures, never wedges a commit); re-edit sessions skip it (lane is L1-only on `featureId`); `dispose()` now bumps `commitGen` (latent resume-after-teardown hole); (2) `resetDocumentScopedUi` wired at all 5 new/open/import/recover entries BEFORE the store swap (was orphaned with zero callers); (3) `opType` plumbed at all 3 FeatureMeta write paths (projectionHydration/toFeatureMeta/historyActions) — real-lane Chamfer re-edit routed to the FILLET editor before this; (4) armed-extrude proactive invalidation — documentStore subscription cancels an armed/dragging extrude + hints when the armed sketch's `geometryToken` bumps (undo/redo under an armed tool); (5) stale test typings fixed (wave2 `geometryToken`, sketchStaticSync fakes); (6) preview opId pinned real (Rust nil-UUID fallback unreachable from FE). Baseline red found+fixed: wire_contract pinned worker-wire `body_<uuid>` on preview `changedBodies` — Rust-domain bare uuid is correct (FE matches the `document-changed` mesh registry), manager normalization already right.
- [x] REVIEW-DEFECTS (continuation, 2026-07-29 — the 3 confirmed-open adversarial findings): (a) partial ear-clip published incomplete material silently (`PolygonFill.cpp` no-ear break) → `RegionFill.complete` (exact `loop−2` triangle-count law) + `SketchRegions` OP_FAILED fail-closed + 5 new polygon_fill pins (collinear stall reported, degenerate-hole outer still completes); (b) `to_next_distance` bound the nearest ray-PLANE from one origin — a face plane the profile never crosses could bind → rewritten to bounded-face casting (`IntCurvesFace_ShapeIntersector`, rays from profile vertices + area centroid, coincident host face skipped via 1e-4 start) + 2 wp6 ctests (laterally-missed pillar FAILS loudly; nearer-missed-face skipped → binds z=8 not z=5, vol 800 exact); (c) = P3 item 1. Plus: orphan `AnalyticFragmentWire.h` deleted — overlap-fragment chord approximation documented as V1 limitation (SCHEMA §7.4 + backlog); `STALE_PREVIEW` structured error code end-to-end (worker envelope → protocol `ErrorCode` → `OpFailureCode` → `ApiError::StalePreview`; api string-prefix sniffing killed; wire test pins the CODE not the message) — SCHEMA §7.4/§7.6/§8/§14 SIGNED OFF (no fixture bump: error path + op semantics only).
- [x] P4 proof: `nested_inner_disk_parity_and_reopen_stability` (wire_contract, real worker) — inner disk by exact id: preview volume ≈ π·25·7 AND == commit volume ±1; annulus sibling binds independently ((800−π·25)·4 ±3%); save → FRESH worker reopen → identical `historyPrefixHash` chain + read-only `prepare_sketch_regions` answers the SAME region-id set. Pre-existing: non-first disjoint region preview==commit (wire 15/15), annulus/disk cells + ring topology (regions 5/5), e2e multiregion/hole-extrude/boolean. NEW e2e: commit-barrier assertion (`committedEpoch === finalEpoch` after Enter-commit).
- [x] **GATE PASSED** (2026-07-29): ctest 70/70 · cargo 486/0 vs real worker (REQUIRE_WORKER) · clippy/fmt clean · tsc 0 errors · FE 1205/107 · build clean · e2e 35/35 · hex 1 (sole hit = pre-existing inputProbe.ts, tracked :125). One review-caught regression fixed en route: the new document-lifecycle reset clobbered the mock lane's boot-seeded prototype selection (navigation e2e) → reset gated to fire only when REPLACING an open document (`resetIfReplacing`), first-boot seed pinned by test.
- [x] SKETCH-RECORD-FIX (2026-07-29, USER-REPORTED "✓ does nothing"): autosave forensics (`onecad-regen --json` replay of the live document) showed **20 Extrude records and ZERO Sketch records** — NO production path ever minted a `Sketch` timeline record (only tests did), so the regen plan had no Sketch step, `find_sketch` failed every real-lane extrude ("profile sketch not found in plan"), and each retried ✓ stacked another errored op (UI masked the stack: inspector slices to 1 extrude row; preview looked fine because PreviewOp pre-seeds from the SolverLane store, not the plan — preview and commit resolved the profile from DIFFERENT sources). Fix: (1) `finish_sketch` now UPSERTS the sketch's timeline record (`upsert_sketch_record`: append on first finish / `UpdateOperationParams` refresh when content changed / no-op when unchanged so an edit-free finish never dirties regen), api `finish_sketch` forwards the outcome to the regen scheduler + emits PROJECTION_UPDATED + notes mutation (autosave); host-face/datum frames serialize their frozen custom basis (`plane_ref_of`). (2) Controller `rollbackFailedCommit`: an applied-but-regen-failed commit (extrude AND revolve, fresh + re-edit result branches) undoes exactly its own command before re-arming — retried ✓ replaces, never stacks; rollback failure keeps the row visible and never masks the original hint. NOTE: finish is now squash + record = 2 undo steps on first finish (txn fold = future nicety). Proof: `scheduler_commit.rs interactive_sketch_flow_mints_the_timeline_record_and_extrude_commits` — the EXACT app flow (AddSketch → enter → upsert → finish, no manual records) through the PRODUCTION scheduler: record minted once, sketch-only regen publishes, extrude by exact region id publishes 1 body, edit-free re-finish = no outcome, edited re-finish REFRESHES (never duplicates) and republishes; 2 controller tests pin one-rollback-per-attempt. Old documents saved WITHOUT sketch records (the user's broken autosave) stay broken — open a fresh document; on-open record synthesis = backlog if legacy docs matter.
- [x] TOOL-FIRST-PICK (2026-07-30, USER-REPORTED "Extrude tool selected, can't apply or select region"): region picking is select-tool-gated (AUTO-MODE decision), so pressing Extrude with nothing selected dead-ended — the pressed tool ignored every region click and only a bottom-bar hint explained. Fix in `armExtrudeFromSelection`: 1 selected region → arm (unchanged); >1 → reject with an actionable hint (extrude is single-profile by design — `beginExtrudeArmed` boundary rejects >1; the old W2 N-session arm is gone with EXTRUDE-REGION-PARITY); 0 → tool-first REGION PICK on the selected sketch (else the document's sole visible sketch, else hint): `enterRegionPick` generalized with a `kind: extrude|revolve` discriminator (was revolve-hardcoded — `armPickedRevolveRegions` → `armPickedRegions`), extrude pick = FIRST CLICK ARMS (no toggle set, no confirm chip; Esc cancels; orbit suppressed; click-away suppressed during pick so AUTO-MODE's "early arm turns clicks into commits" trap can't reproduce); single pickable region arms directly, whole-sketch selection NEVER guesses a profile (pick, not fallback — pinned). `armRevolveFromSelection` gains the same selected-sketch → sole-visible-sketch ladder. Tests: regionPick suite rewritten pins — multi-select reject, whole-sketch → pick opens (getSketchRegions called, nothing armed), click-in-pick arms the CLICKED region (r1 not r0); e2e multiselect spec message updated. Suites: tsc 0 · controller 82/82 · FE 1231/1232 (sole red = StartScreen "tab Model", AUTO-MODE's in-flight titlebar-toggle deletion, not this change) · e2e 38/38.
- [ ] Manual Tauri gate (USER, `bun run tauri dev`): rect+circle sketch → click inner disk vs annulus — preview == committed body BOTH times; Enter/✓ commit; Cut actually subtracts in preview AND commit; edit sketch under an ARMED extrude → tool cancels with hint (never commits stale); stale region after sketch edit → named error; reopen file → regions still selectable + extrude works; ToNext onto a body the profile misses → named error, never a nearer-plane bind; Chamfer re-edit opens CHAMFER editor on a saved/reopened doc

## AUTO-MODE — tool+context-driven mode switching (2026-07-29, SHIPPED; design approved via 4 user decisions)
Mode is derived intent, never a manual toggle. `toolStore.mode` stays the single state (all ~20 consumers untouched); only WHO sets it changed. TitleBar SegmentedToggle DELETED (StatusBar keeps the "Sketch mode — {name}" indicator).
- [x] P1 core: `src/tools/activateTool.ts` dispatcher — model+sketch-only-tool ⇒ `setMode("sketch", undefined, {tool})` (new `opts.tool` preserve: Circle stays Circle, not default Line; plane-pick/face entry flow unchanged); "sketch" id stays explicit new-sketch intent; select/mirror context-local. FloatingToolbar + useShortcuts both route through it. keymap `resolveBinding` cross-mode fallback — a key bound ONLY in the other mode resolves (tool actions ONLY: L in model ⇒ sketch+Line, E in sketch ⇒ finish+Extrude path; shared letters R/C/M/H never leak — current-mode table wins first; Delete/Backspace can NOT cross into model). SketchController beginPlanePick forces default cursor (preserved-tool crosshair no longer leaks into pick phase).
- [x] P2 auto-finish: model-only tool from sketch mode ⇒ drained finish (same squash path as Enter — ONE undo step) ⇒ `setMode("model", undefined, {tool})` ⇒ arm-from-selection; Extrude rides the existing pendingExtrude handoff byte-identical (hint → region pick → arm). DESIGN REVERT: an auto-arm-on-single-region prototype was CUT — it broke the pinned multi-select rejection flow (extrude-multiselect e2e: picking is select-tool-gated, so an early arm locks selection mid-gesture AND turns a region click into a click-away commit — extrude-boolean/end-conditions e2e). Hint-only handoff is the approved behavior.
- [x] P3 viewport dbl-click: model mode + select tool + static sketch hit ⇒ `setMode("sketch", id)` (mirrors tree dbl-click; gated to select so it never fights armed tools / region-pick dbl-click accelerator).
- [x] GATE PASSED (2026-07-29): tsc 0 · FE 1230/109 files (+25: activateTool matrix, setMode opts.tool, keymap cross-mode) · build clean · e2e 38/38 (+3: auto-mode spec — L→plane-pick with Line armed, E→finish+region-pick+arm, dbl-click→edit) · hex 1 pre-existing (inputProbe, tracked :125). No Rust/worker/SCHEMA change (mode is frontend-only).
- [ ] Manual Tauri gate (USER, `bun run tauri dev`): NO mode toggle in titlebar; L key from model mode → plane pick → draw immediately; E from sketch → finishes (ONE undo step) → pick region → E → drag; Esc ladder unchanged (tool → deselect → exit sketch); dbl-click a finished sketch curve → edit session; tree dbl-click still works; undo after auto-finish reverts the whole sketch session in one step

## TRUST + PREVIEW waves (plan approved 2026-07-31, `~/.claude/plans/mossy-foraging-muffin.md`; internal adversarial review REVISE → all 2 blockers + 11 majors folded; Codex gate unavailable until Aug 5)
Goal: kill silent-wrong-behavior class + commit-blindness. User decisions: all-5-op preview; fillet/chamfer/shell armed-commit gesture (Enter/✓/Esc only — click-away impossible: armed tool claims every left press as value drag); suppression hash-filter + cascade:true; tree omits body Delete; T0/T1 Rust prerequisites in scope.
### Wave 0 — baseline
- [x] Commit AUTO-MODE + EXTRUDE-COMMIT-FIX (part 2) batch (this commit); manual Tauri gates above stay open
### Wave 1 — TRUST (implemented 2026-07-31; dual adversarial review REVISE → all findings fixed)
- [x] T0 suppression real: `Timeline::from_records` derives Suppressed (all 4 rebuild callers); ONE predicate (planner filters records, state display-only); hash filter same-commit; `FeatureDto.suppressed`. **Deeper defect found+fixed: `OperationRecord::outputs` was NEVER populated in production → dependency graph had zero body edges → cascade AND validate_temporal/produces_before anti-time-travel were inert** → `sync_record_outputs` after commit (executed-records-scoped per review F1 — checkpoint-accelerated regen must not wipe the prefix; red-first proven). suppression.rs 8/8 red-first (pre-fix publishes 2 bodies).
- [x] T1 body-metadata durability: `adopt_regen_bodies` (insert-only, non-undoable, in after_mutation — survives RestoreBodies undo), ONE merge helper (regen wholesale + name/visible overlay), save keeps doc-only rows (rename of suppressed body survives reopen), suppressed-producer rows filtered from open seed, DeleteBody rejected for timeline bodies. DEVIATION (accepted): projection membership = regen mirror ONLY — suppressed feature's body leaves the tree (phantom rows were the alternative). body_metadata.rs 7/7, 3-pass red-first.
- [x] T2 cheap honesty: shared `toFeatureMeta` (statusMessage+suppressed); doc-lifecycle store resets (direct-call tested); pattern/mirror re-edit from stored `sourceBodyId` + full shape seed (axis/spacing/angle/plane), loud refusals; repair rebind ambiguity = sticky hint + explicit-selection honor; cascade flag wired.
- [x] T3 revolve parity: re-edit reads stored profile (exact regionId bind, volume-proven 3.2× gap vs wrong region); arm = pure-read getSketchRegions; **confirmRevolve record guarantee above both branches — NEGATIVE tripwire proven vs real worker ("profile sketch not found in plan" without it)**; stored-axis restore + loud refusal; geometryToken cancel for revolve; exit cancel-then-finish pinned. revolve_ops.rs 8/8.
- [x] T4 boolean re-edit: op-swap only via updateOperationParams (bodies verbatim), opType-gated, retired-tool chip-only; dto value_text now shows the operation (real-lane row was signal-less).
- [x] T5 tree UX: 4 WireEditCommand variants (serde-verified), METADATA_ONLY no-awaiter transport (deleteSketch excluded), treeActions w/ optimistic+revert, context menu + F2 rename + sketch delete two-click, MenuItem → src/ui, auto-hide rerouted post-commit-loop (real-lane pop-back bug fixed), mock metadata registry so e2e assertions are non-vacuous.
- [x] T6 suppress overlay killed: rows read `item.suppressed` (fixes un-suppress-after-reopen: overlay `!undefined→true` lock), historyStore.ts DELETED, mock cascade derived from real body-consumption edges (under-approximates, never fakes).
- [x] T7 unsaved-changes guard: webview dialog at App root (start screen covered — review B2: EditorScreen-only mount latched ExitGuard → app unclosable), self-healing guard (emit-fail/no-window → clear+proceed), intent races fixed (second ⌘W during quit prompt, cancelExit release), confirm_exit clears recovery marker (Don't-Save-quit no longer resurrects discarded work as crash recovery), ⌘Q via RunEvent::ExitRequested code-gate (verified vs tauri 2.11.5 source).
- [x] Extras: trace.ts gated (DEV or ?trace); e2e +7 (tree-visibility w/ survives-commit assert, tree-rename, history-suppress, model-undo, unsaved-guard×5-in-1)
- [x] Review findings fixed (2 waves): B1 pattern/shell re-edit was DEAD on real lane (kind-guard vs folded FeatureKind; mock emitted nonexistent kinds — postmortem false-green class; guards now opType, mock emits real kinds, dto pin test) · B2 ExitGuard start-screen latch · B3 checkpoint outputs wipe · M4 suppressed-flag unread · M5 all-suppressed → Clear publish (Rust-side empty snapshot through normal accept path) · M6 cascade only-on-suppress (un-suppress no longer resurrects deliberate suppressions) · M7 intent overwrite · M8 recovery marker
- [x] **Wave 1 GATE PASSED** (2026-07-31): tsc 0 · FE 1315/114 · build clean · cargo workspace green vs real worker (REQUIRE_WORKER; suppression 8, body_metadata 7, revolve_ops 8 new) · clippy/fmt clean · e2e 50/50 (auto-mode E-handoff flaked once under suite load, 3/3 isolated + 50/50 rerun) · hex 1 pre-existing (inputProbe)
- [ ] Manual Tauri gate (USER, `bun run tauri dev`) — Wave 1: suppress feature mid-chain → downstream follows, geometry actually changes, un-suppress restores (and a separately-suppressed downstream stays suppressed); reopen doc w/ suppressed feature → row dim + un-suppress works; tree eye toggle survives save/reopen + an op commit; right-click rename body/sketch → survives reopen; revolve re-edit on 2-sketch doc opens the RIGHT sketch/region + axis; dirty doc → ⌘W/⌘Q/titlebar × prompts Save/Don't Save/Cancel (start screen too), Don't-Save-quit does NOT offer recovery next launch; pattern/mirror/shell re-edit opens on real lane with stored values
- Follow-ups (accepted V1, next waves or backlog): HistoryList FEATURE_ICON keys on kind (patterns render boolean icon — parity w/ real lane, opType routing nicer); vestigial FeatureKind variants + kindFallback; editEdgeOpFeature kind-guard (works via explicit routing); doc.bodies unbounded growth + stale-row DeleteBody no-op; appStore↔fileActions circular import (function-body only); merged_bodies clone per projection (perf); core set_suppression still honors cascade-on-unsuppress if a caller sends it (policy lives at callers)
### Wave 2 — PREVIEW (all 5 ops, zero protocol change; implemented 2026-07-31)
- [x] P0 lane generalization: previewOps.ts OP_BUILDERS — each builder fixture-pinned equal to its commit call-site literal (source quoted inline); dual edges+edgeIds refuse-on-mismatch (3 throw pins + order pin); inputs-fillet-only rule documented in-module; local-synth revolve/fillet/shell SKIP w/ epoch-settling empty result, boolean emits replacedBodyIds:[tool]; wire.rs lowering-equality table over all 6 opTypes. +42 vitest.
- [x] P1 controller generalization: ToolPreviewSession/previewSessions/previewOwner/previewParamsFn/sendPreview() (15 sites), closePreviewSessions(), setTrailingMs seam, previewPending (non-extrude only), setPreviewTint rename. ZERO behavior change proven — all pre-existing tests green unchanged (only mechanical mock renames), extrude parity pins cited untouched, full e2e 50/50.
- [x] P2 revolve kernel preview: N sessions at axis pick; commit = flush→barrier→endPreview(true) w/ applyOperation fallback when no session; rearmRemainingRevolve failure hygiene; L1 lathe kept under L2; Cut tint verified. preview==commit EXACT (NewBody 90° 2352.411; Cut vs 20000-box → 9502.555 both sides; head fingerprint byte-identical). BONUS: rect_sketch fixture plane-mismatch bug found+fixed (record plane ≠ document plane silently diverges preview vs commit in tests).
- [x] P3+P4 fillet/chamfer/shell: armed-commit gesture (release→armed; Enter/✓ commit; Esc cancel; NO click-away — B1), kernel refusal BLOCKS ✓ with OCCT's reason (any live failure, not structural-only — opFailed is the headline case), previewPending hint, revision-change → fail-closed cancel + re-select hint, rollbackFailedCommit reused, editEdgeOpFeature opType-tightened (Shell folds into fillet kind). preview==commit EXACT (fillet 19981.229, chamfer 19960, shell 2224 == analytic). Worker ctest cases 12–14 (fillet/shell/cancelled-token). TWO shared-infra bugs found+fixed: scheduleTrailing dropped coalesced params when trailingMs>90 (preview froze at last value); progress hint buried error hints (stale "preview failed" contradiction).
- [x] P5 boolean: session at armed, single-shot per mode change, Cut tint, both head bodies hide via replacedBodyIds, commit barrier; re-edit sessionless. preview==commit EXACT (Union 30000, Cut 10000). previewArmHint seam adopted (arm hint was clobbered by pending machinery).
- [x] **Wave 2 P6 GATE PASSED** (2026-07-31): tsc 0 · FE 1404/118 · build clean · ctest 70/70 · cargo workspace green vs real worker · clippy/fmt clean · e2e 59/60 + boolean-Apply spec rerun-green (known coordinate-click flake; P5's own full run was 60/60) · hex clean · SCHEMA §14 doc-only entry
- Follow-ups (accepted V1): rollbackFailedCommit trace tag hardcodes "extrude" (cosmetic); Apply-button e2e click flake worked around by coordinates (root cause unpinned); preview p95 latency budget in solverbench still unpinned (TODO.md:178 flag stands); worker-side PreviewOp coalescing deferred (cancel-frame groundwork = ctest case 14)
- [ ] Manual Tauri gate (USER): fillet radius to OCCT refusal → ✓ blocked with named reason; shell preview hollow; boolean 3 modes preview == commit; revolve Cut subtracts in preview AND commit; suppress feature #2 of N → downstream cascade-suppressed not errored; tree rename/hide survives save/reopen; close with unsaved changes prompts

## SKETCH-MULTI-OBJECT (2026-07-31, SHIPPED 267af13)
- [x] USER-REPORTED: only latest-drawn object survived Finish — root cause `seedIdMapFromWire` merge-not-rebase → first post-re-entry upsert emitted removeEntity for all prior geometry; fixed (clear before seed), red-first, 4-lane pins (unit / full-stack vitest / e2e / real-worker)
- [ ] USER manual check: rect → Finish → re-enter → line → Finish → re-enter — both persist
- [ ] Flagged latent (next wave): `tauriClient.getSketch` passes raw frontend id (siblings resolve `backendSketchId ?? id`); tree sketch-switch while in sketch mode = controller no-op (chrome/new vs controller/old session)

## SKETCH-POWER wave (plan approved 2026-07-31, `~/.claude/plans/do-thorough-exploration-and-rosy-lollipop.md`; internal adversarial review REVISE → 2 MAJOR + 2 MINOR all folded; Codex gate dead until Aug 5)
Scope (user decisions): construction toggle + T/E/M constraints + point + shapes (centerRect/slot/polygon) + marquee + ellipse; 5 verified latents = Wave 0; manual Tauri gates run by user in parallel (fallout = interrupt work).
### Wave 0 — latent fixes (all red-first) — SHIPPED 2026-07-31
- [x] W0.1 `tauriClient.getSketch` resolves `backendSketchId ?? sketchId` (sibling parity; red-first: raw frontend id pinned on the wire)
- [x] W0.2 sketch→sketch tree switch: `switchTo` = teardown → awaited flush→cancel(A)→finish(A) → supersession check → openSession(B); SELF-SWITCH GUARD (decision vs OPEN SESSION only + `selfActiveSketchWrite` bracket around openSession's echo — without it test [d] unpassable), `pendingSwitchId` latest-wins drained post-open (just-opened target dropped), `teardownSession` factor (exit byte-identical; returns session, caller nulls — preserves cancel-before-null order), `priorProjection ??=` (original entry's projection survives to eventual exit), rejected re-open → `failOutOfSketchMode`. 10 tests, 4 red-first (wrong-sketch upsert pin incl.)
- [x] W0.3 `prepare_sketch_regions` REFUSES during same-sketch drag (`op_failed` recoverable, names sketch + "retry after pointer-up"; other-sketch gesture unblocked); `finish_sketch` take-once clears `active_gesture` scoped to finishing sketch + best-effort worker `end_gesture` (Enter-mid-drag race + orphaned worker gesture both killed). 6 stub unit (red-first) + 2 real-worker (sketch_regions 7/7)
- [x] W0.4 Alt pick-through: `refFromModelHits(body, sketch, pickThrough)` — Alt suppresses sketch tie-break, body wins, sketch only when no body hit; no-Alt path proven byte-identical (new sketch fallback unreachable); engine hover/pick mods gain `alt` append-only; hover tint follows next pointermove (mods-less hover architecture, keydown listener deliberately not added). Picker.test tie pin untouched
- [x] W0.5 history icons: `OPTYPE_ICON[item.opType ?? ""] ?? FEATURE_ICON[item.kind]` — opType strings verified vs `Operation::op_type()` (record.rs:321); Chamfer/Shell/patterns/Mirror rows now correct on real lane. Red-first render pins
- [x] **Gate W0 PASSED** (2026-07-31): tsc 0 · FE 1426/121 · cargo 534/0 vs real worker (REQUIRE_WORKER) · clippy/fmt clean · ctest 70/70 · e2e 61/61 · hex clean (inputProbe pre-existing)
- Flagged (accepted): `sketchStaticSync` refetches only on sketch→model — after A→B switch, A's static layer relies on the real lane's geometryToken bump from finish(A) to self-heal (mock lane may show stale until exit); follow-up if manual gate shows it
### Wave 1 — construction geometry toggle — SHIPPED 2026-07-31/08-01
- [x] W1-A backend (commit c2566e8): SCHEMA §7.3/§7.4 `entities[].construction` + §14 "2026-07-31 (b)" (no fixture bump — region-id stability PINNED at runtime-computed baseline, not hardcoded hash); worker `WireSketch` `is_construction()` at all 5 sites, child points inherit parent flag, `SketchEntity.h` default-true trap commented; red-first ctest `test_sketch_construction` 3 cases (all-construction rect published a region PRE-fix — bug pin; construction+real → 1 region id == real-alone; constraint on construction line still solves, DOF drops); core `SetEntityConstruction` + total `entity_with_construction` over all 5 variants (memento inverse free; arm proven load-bearing by no-op substitution; squash pin +97); real-worker `sketch_construction.rs` 2 cases (flip → region-set change → undo restore; production p0Ref + inline wire forms BOTH pinned). Gate: ctest 71/71 · cargo 541/0 · clippy/fmt clean
- [x] W1-B frontend: wire op `setEntityConstruction`; `SketchIdMap.entityConstruction` last-SENT cache at all 5 touchpoints (clone/seed = BUG-5 class, tested; re-entry seed mirrors `frontendEntitiesFromDto` so first marshal after re-entry emits no spurious flip); `marshalUpsert` flip branch (exactly one op, backend uuid); `sketchService.setEntitiesConstruction` (queued, undo snapshot); X = selection flip (mixed rule `!every(construction)` owned SOLELY by `runToggleConstruction`, useShortcuts) / empty = sticky `sketchStore.constructionMode` (reset on doc close); chrome pressed button (mode-only, aria-pressed honest); controller `decorate()` OR-s the flag at the single FSM→engine/commit crossing — RAW preview still feeds `updateGhost` (H/V hint must show for construction lines, pinned); revolve construction-centerline axis pinned BOTH lanes (e2e + beginPreview vitest). e2e construction.spec 3 scenarios. Gate: tsc 0 · FE 1456/122 · e2e 64/64 · hex clean
- Flagged follow-ups: pre-existing `enterRegionPick` hint-clobber (`setStatusHint` then `setTool("select")` whose subscriber clears it — "No closed region" never visible; `beginRevolveArmed` works around the same ordering); FE full-suite exit-1-with-all-passing observed ONCE (teardown flake class, 2 subsequent runs exit 0)
### Wave 2 — FE-only tools batch — SHIPPED 2026-08-01 (zero Rust/worker/SCHEMA, verified)
- [x] W2-A Tangent/Equal/Midpoint user-apply: 3 files exactly (applicability rules + catalog + authoring); matrix solver-bounded (Tangent line×circular + circular×circular, never line+line; Equal line+line / circular pairs, never mixed; Midpoint = `marshalsAsPoint`-gated point + Line, same-entity degenerate excluded — arc endpoints / virtual line-Midpoint would marshal null + silently drop); dup Tangent → existing reject-on-conflict (no new dedup, documented); e2e ×3 incl. Midpoint via circle-Center (feasible — named-point tier resolves Center)
- [x] W2-B point tool (P — shadows sketch→linearPattern handoff, documented; click-commit-rearm; snap + Coincident inference free)
- [x] W2-C shapes: centerRect (⇧R, mirrored-about-center, standard inference only — honest V1) · slot (S, 3-click, caps sweep outside 180°, committedConstraints Tangent×4+Equal ONLY — arc-endpoint Coincidents unmappable, REAL-LANE DOF ≈13 in FSM header + e2e labeled mock-only) · polygon (G, digits 3-9 sticky, clamp 3-12, n lines + construction circumcircle + Coincident ring + OnCurve×n + Equal×(n−1) ⇒ DOF 4 any n). Shared enabler `ToolConstraintSpec`/`resolveToolConstraints` (index refs, out-of-range dropped never guessed, intra-batch inference suppressed — draw path has no reject-on-conflict; cost: axis-aligned slot/polygon gets no H/V, documented)
- [x] W2-D marquee (select tool, empty-space LMB drag — deliberately claims LMB orbit in sketch select; RMB pan + shift orbit remain): rightward=window (real containment incl. sweep-gated arc extrema) / leftward=crossing (touch via entityIntersections vs rect edges); implements what legacy UI PROMISED (its findInRect was bbox-both-ways — cited divergence); plane-AABB = conservative superset off-normal (deliberate: superset recoverable, subset loses picks; corner missing plane aborts); `cancelMarquee` idempotent on ALL 8 teardown paths incl. window-level pointerup/cancel net (release outside viewport); overlay = controller div, `color-mix` over --color-accent (no new token, no hex)
- [x] **Wave 2 GATE PASSED** (2026-08-01): tsc 0 · FE 1556/124 · e2e 73/73 (full) · hex clean (inputProbe pre-existing) · cargo 541/0 + ctest 71/71 stand from W1-A (no backend files touched — verified)
### Wave 3 — ellipse end-to-end (protocol change) — SHIPPED 2026-08-01
- [x] P0 SCHEMA: Ellipse un-UNSUPPORTED (Spline stays); wire `{id, type:"Ellipse", center:[cx,cy], centerRef?, majorR, minorR, rotation?(rad), construction?}`; NORMATIVE normalization-echo clause (reader enforcing majorR≥minorR MUST echo the normalized triple); §7.4 naive-DOF deviation block (redundancy unreported, OverConstrained = naive<0; NO constraint kind accepts an ellipse entity, center Point ordinary) + chord-note extension (pure loop = exact Geom_Ellipse); §14 2026-08-01, no fixture bump
- [x] P1 worker red-first: 5 fixtures FAILED at the single choke point pre-fix ("unsupported entity type 'Ellipse'" on BOTH lanes — plan-profile line proves OpCommon shares the translator); one WireSketch branch mirrors Circle (construction-inherited minted center, `.center` primary handle, raw params → addEllipse owns the swap) + apply_solved_positions echoes NORMALIZED triple (proven: sent 3/6/0 → echoed 6/3/π·½); rect region id byte-identical with disjoint ellipse present (additive-safety pin)
- [x] P2 Rust: wire.rs Ellipse arm (Circle clone) + real-worker ×4 (region vs corpus regions_ellipse — area 56.186 vs π·6·3 within 0.64% tessellation; extrude PURE loop vol 564.42 vs π·a·b·h 0.19% at Lod::Fine; reentry centerRef hydration + center drag; naive dof). KNOWN-GOOD deviations: core-authored ellipse dof=7 not 5 (pre-existing Circle/Arc dual-center-point behavior verbatim, centerRef re-owns); bbox assertions use POSITIONS buffer (BRepBndLib inflates ~0.35mm — LATENT: other bbox_dims exactness tests share it)
- [x] P3 FE: `ellipseMath.ts` single source (normalize/sample-72/extents/nearestOnEllipse); 3-click tool ported from oracle EllipseTool.cpp incl. LIVE swap-normalization (preview === commit byte-pinned); key O (e = extrude handoff, pinned); render Line2 sampled polyline both layers; center snap + drag; mock parity (entityFreedom 5, detectRegions ClosedCurve abstraction — circle path byte-identical pinned); trim = whole-DELETE (oracle parity), mirror copy-only rot'=2φ−θ no center-Symmetric (rotation untieable — documented); applicability BAILS to [] on any ellipse target (silent-drop would degrade 2-pick to 1-pick offering wrong constraints — 7 proto-cited tests); marquee window = exact rotated-bbox support function, crossing = 72 sampled chords; e2e ×3
- [x] **Wave 3 GATE PASSED** (2026-08-01, merged tree re-verified by orchestrator): tsc 0 · FE 1660/126 · ctest 76/76 · cargo 547/0 vs real worker · clippy/fmt clean · e2e 76/76 · hex clean; TODO:26 flag retired; PlaneGCS-ellipse + snaps/drag backlogged (:78)
### Manual Tauri gate (USER, `bun run tauri dev`) — SKETCH-POWER
- [ ] Tree: while editing sketch A, double-click sketch B in the tree → controller ACTUALLY edits B (draw lands in B, chrome/status agree); rapid A→B→A settles correctly; exit restores the original camera projection
- [ ] Alt+click a body face lying under a sketch-on-face → face selected (fillet/shell reachable); without Alt the sketch fill still wins
- [ ] Construction: X with nothing selected toggles the chrome button + subsequent lines draw dashed; select a real rect edge → X → its region disappears from extrude pick; draw a profile + dashed centerline → revolve accepts the centerline as axis; save/reopen keeps flags
- [ ] New tools: P point (snap to endpoint → coincident); ⇧R center rect; S slot (drag a wall — caps stay tangent); G polygon (digits 3-9 live, drag a vertex — stays regular); O ellipse (3 clicks, rotated; extrude it; trim-click deletes whole)
- [ ] Constraints: select line+circle → Tangent chip applies (DOF drops); two lines → Equal; circle-center+line → Midpoint; re-apply Tangent on an auto-constrained pair → rejected with named clash
- [ ] Marquee: rightward drag = solid border, selects only fully-inside; leftward = dashed, touch-selects; Shift adds; Esc cancels; after any marquee LMB orbit still works in model mode
- [ ] History: pattern/mirror/chamfer/shell rows show their OWN icons

## MODELING-REACH wave (plan approved 2026-08-01, `~/.claude/plans/do-thorough-exploration-and-rosy-lollipop.md`; adversarial review REVISE → 2 MAJOR + 3 MINOR folded)
Scope (user): datum offset planes + measure V1a + units-display + display-mode/fit/isolate UX + SKETCH-POWER debt incl. arc-endpoint handles (own gate). Feature rename NOT chosen.
### Wave 0a — debt quick batch — SHIPPED 2026-08-01
- [x] Equal/Midpoint real-worker DOF proofs: Equal circles 10→9 (−1), Midpoint off-midpoint point 6→4 (−2); all 7 kinds now proven (Fixed 2→0, Concentric 10→8, Tangent 9→8, Symmetric 8→6, OnCurve 6→5)
- [x] Hint-clobber → `resetToSelect(hint?)` (setTool FIRST, last word wins; StatusHintOpts derived from the store so it can't drift): 20 sites converted (8 hazards + 3 inline workarounds + 9 same-shape preventives — class can't regrow), 8 bare no-hint setTool calls deliberately untouched; 9 red-first pins (zero-region extrude/revolve, success/failure hints survive commit tails)
- [x] Stale static layer → `documentStore.bumpSketchGeometry` (`local:<n>`, no-op unregistered, hydration-collision-safe) called unconditionally in switchTo's closing block BEFORE superseded returns (A is closed either way); sketchStaticSync needed NO change (token diff already reloads); red-first [j]/[j2] + control [j3]
- [x] **Gate W0a PASSED** (2026-08-01): tsc 0 · FE 1674/126 · e2e 76/76 · cargo 547/0 vs real worker · clippy/fmt clean
### Wave 0b — arc-endpoint wire handles — SHIPPED 2026-08-01 (SCHEMA §7.3 positions + §7.4 e3.start now TRUE)
- [x] Worker: SketchArc optional endpoint ids (all-or-nothing bind, removal cascade owns them) + WireSketch mints `.start`/`.end` (construction-inherited) + `addArcRules` GCS tag-0 (4 eqs consume 4 params — DOF-neutral; tag 0 absent from gcsTagToConstraint_ AND GCS excludes tag-0 from redundancy blame; solver rebuild-only so no lifetime hazard) + angle-form echo + naive-DOF conditional −4
- [x] Rust: Coincident `point1_position/point2_position` (`skip_serializing_if is_arbitrary` — frozen snap BYTE-IDENTICAL, verified vs git) + wire `positions` (Arbitrary = `""` — worker resolve_point needs the empty role)
- [x] FE: `arcEndpointRef` (arc uuid + role, was silently dropped) + hydration positions merge (also fixed a latent filter index-misalignment) + slot welds restored
- [x] **FINDING 1 — slot = Coincident×4 + Equal, Tangents DROPPED, DOF 9 not 5**: entity-level Tangent(line,arc) is distance==radius, at its MAXIMUM once endpoint-welded → gradient vanishes → PlaneGCS diagnoses all 4 redundant → false OverConstrained badge for zero DOF gain (measured: welds+Equal dof 9 clean; +Tangents dof 9 + hasRedundant). Pinned by ctest. Proper endpoint tangency = new constraint kind (FreeCAD-style tangent-via-point) — BACKLOG
- [x] **FINDING 2 — latent drag-teleport bug fixed**: solve/solveWithDrag/group-drag short-circuit on `constraints_.empty()` teleported geometry — with arc rules that tore endpoints off their circle; `hasInternalCouplings()` guards all 3 paths
- [x] **FINDING 3 — dup center points CONFIRMED (+4 on FE-shaped slot: 22→14 vs worker-authored 18→10)**: wire emits each arc/circle/ellipse center BOTH as standalone Point entity AND inline coords → worker mints a second unconstrained center. Pre-existing, all round types. BACKLOG: worker honors centerRef / wire dedupe
- [x] Red-first ctest ("Coincident: unresolved point handle" verbatim) + bare-arc `.start` SolveDrag radius-preserved (welded-drag = pre-existing pin-all limitation, documented); cargo −8 invariant on the measured FE baseline; seam comments + SCHEMA §7.3/§7.4/§14 (additive, no fixture bump, interop note on the tangency degeneracy)
- [x] **Gate W0b PASSED** (2026-08-01, merged tree re-verified): ctest 77/77 · cargo 550/0 vs real worker · tsc 0 · FE 1679/126 · e2e 76/76 · clippy/fmt clean. Flagged seam: solved `.start` handle positions skipped by frontendSolvedPositions (matches pre-existing arc radius/angle non-writeback — needs the radius on the same path)
### Wave 1 — datum offset planes V1 — SHIPPED 2026-08-01 (worker + protocol byte-identical)
- [x] Core: `add_datum` resolves AT CREATION (frozen; world XY/XZ/YZ base or chain off a resolved datum; client frame ALWAYS overwritten — Rust is basis authority; unresolvable → lenient `resolved_valid:false`); `DeleteDatum` + referenced-guard naming blockers; `stamp_datum_plane` in add_sketch AND update_sketch_attachment (bypass closed); DatumKind::name() lock-tested
- [x] Rust: DatumDto (camelCase, SketchPlaneDto From impl folded the api/mod hand-copy) + DocumentProjection.datums (Eq dropped — f64) + projection fill straight off doc (datums never cross the OCW1 wire — §1805 statement still true)
- [x] FE data: wire arms + builders (8 serde-required fields, placeholder frame discarded by core — REAL frame read back from store), documentStore.datums + nextDatumName, hydration (`?? {}` back-compat), mock full mirror incl. `mockAttachSketchToDatum` (attach tracked mock-side — guard non-vacuous, proven: enterSketch(newOnDatum) alone makes delete reject)
- [x] FE surface: datum tool (D; PlanePicker verbatim + geometric labels vs legacy-swapped repo kinds; offset chip w/ live ghost; ✓/Enter commit via resetToSelect; NO click-away authoring; endDatumPick idempotent ×6 teardown paths) + DatumLayer (433 LOC PlanePicker clone: quads/labels/hitTest/tints/ghost) + datumSync + tree "Datums" section (dbl-click = sketch-on-datum; delete two-click, call-then-remove matching deleteSketch — optimistic would break the guard error; NO dead eye toggle) + sketch-on-datum both clients (tryEnterOnSelectedDatum before plane-pick; datum quads pickable DURING plane-pick; unresolved-datum refusal rides INTO the picker prompt — avoids the resetToSelect hint-clobber class)
- [x] Proofs: real-worker `datum_planes.rs` 3/3 — datum-hosted extrude bbox z=[10,15] EXACT, stamped origin via getSketch; legacy-swapped-basis pin (XZ offset 10 → world +X); chaining 10+5→15; delete undo/redo; FE wire-shape pins incl. kind-required negative. e2e datum-create + datum-sketch (rejection rides the real guard path)
- [x] **Gate W1 PASSED** (2026-08-01, merged tree re-verified): tsc 0 · FE 1765/132 · cargo 565/0 vs real worker · e2e 84/84 · clippy/fmt clean · ctest 77/77 stands (no C++). Flags: chained-datum deletes unguarded (coherent w/ frozen frames — revisit if resolution ever re-derives); datum visibility/rename deferred
### Wave 2 — units foundation + measure V1a — SHIPPED 2026-08-01
- [x] `src/units/format.ts` single seam (formatLength/formatArea/parseLength — 25/25mm/2.5cm/0.5m/1in → mm; dimensionFormat = re-export shim, ~6 import sites stable); routed DimensionInput/filletRadius/shellThickness/ModelToolChips ×5/constraintAuthoring ×5; valueText round-trip guards BOTH directions (trimmed FE form AND Rust %.1f form parse back — re-edit seeds can't silently corrupt)
- [x] Rust: ElementInfoDto += size/magnitude/curveType; `element_info` command. **LATENT FOUND**: element-map partition mints entries ON DEMAND (only when an op resolves the element) — a promoted-but-unused ElementId is legitimately ABSENT, so the naive clone returned None for every fresh pick → new `ElementQuery::query_element_by_topo_key` rung (SCHEMA §7.5's documented second arg form, exact-lookup ladder topoKey→elementId, absence pinned so the ladder can't be optimized away). **FLAGGED FOLLOW-UP: `face_sketch_plane` (sketch-on-face) has the SAME bug vs the real backend** (ElementId-only query, no worker-backed test) — fix = one call to the new trait method
- [x] FE: measure tool (`?` shift-exact; NEW `NO_CROSS_MODE` set in resolveBinding — ambient keystroke INERT in sketch mode, explicit toolbar click still finishes; 10 keymap pins) + measureTool keep-latest-2 FSM + measureStore/MeasureOverlay (mountChip id-keyed portals) + labels honest ("center ↔ center" — bbox center NOT centroid) + mock FNV-synthesized values (MOCK LIMIT; numeric truth = Rust test)
- [x] Real-worker proof: face area 799.9999999999999, center [−10,20,25] EXACT (frozen non-standard basis), edge 25, bbox diag ≈44.72; unknown id + stale topoKey → None
- [x] **Gate W2 PASSED** (2026-08-01, merged tree re-verified): cargo 570/0 vs real worker · clippy/fmt clean · tsc 0 · FE 1846/136 · e2e 87/87 · ctest 77/77 stands (no C++)
### Wave 3 — display-mode + fit-selection + isolate — SHIPPED 2026-08-01
- [x] Display mode REAL (was a dead button): BodyObjectHandle.setDisplayMode toggles face/edge children (never shared-material wireframe flag); MeshIngest owner (subscription + loadBody); default flipped → shadedEdges (the look the renderer always produced); dynamic button label; wireframe faces genuinely unpickable (pick what you see, documented)
- [x] ⇧F fits SELECTION (face/edge → their body; empty → fit-all unchanged); `Box3.expandByObject` recurses invisible children UNCONDITIONALLY (verified vs three source :379) → explicit `child.visible` filter with a can't-rot test beside it; CadOrbitControls.fitView(bounds?) reuses sphere-fit
- [x] Transient isolate (⇧I / NavPill; NEVER persisted — tree-eye is a document fact): effective visibility = docVisible && in-set at subscription/loadBody/flag-change; Esc rung BEFORE deselect; tree dims isolated-away rows (eyes untouched); exits on doc close + on tool ARM (not disarm — Esc rungs stay distinct); ⇧I ignored ENTIRELY while a preview holds hidden bodies (both directions of the raw-snapshot stomp guarded via new hasPreviewHiddenBodies())
- [x] **Gate W3 PASSED** (2026-08-01, merged tree re-verified): tsc 0 · FE 1893/138 · e2e 90/90 · backend untouched (cargo 570/0 + ctest 77/77 stand)
### Manual Tauri gate (USER, `bun run tauri dev`) — MODELING-REACH
- [ ] Datum: D → pick XY quad → offset 10 on the live ghost → ✓ → quad + label in viewport, "Datum 1" in tree; dbl-click it → sketch opens ON it → draw → extrude lands 10mm up; save/reopen keeps it; delete rejected (named hint) while the sketch references it; undo removes a fresh datum
- [ ] Measure: ? (model mode) → click a face → exact area label; click an edge → length + center↔center distance + deltas; Esc clears; ? while sketching does NOTHING
- [ ] Units input: type "2.5 cm" into a dimension chip → becomes 25 mm; "1 in" → 25.4
- [ ] Slot drag (FE lane): drag a wall — caps stay welded to wall ends (tangency is as-drawn, not enforced — expected V1)
- [ ] Display button cycles Shaded / Shaded+Edges / Wireframe visibly; ⇧F with a body selected frames IT; ⇧I isolates selection, Esc restores; isolate + boolean preview don't fight
- [ ] Hints now visible: extrude pick with no closed region shows "No closed region to extrude"; success hints ("Union applied" etc.) survive tool reset

## RENDER-MODES wave (plan approved 2026-08-01, `~/.claude/plans/act-as-senior-software-elegant-fiddle.md`; internal adversarial review, Codex gate dead until Aug 5) — BUILT 2026-08-02, uncommitted
Scope (user decisions): studio lights + IBL (no postprocessing), dark 1px edges, popover mode UI, persisted mode. Two-commit split: (1) modular restructure, (2) shading.
### Commit 1 — modular render-mode system (no visual change beyond token repoint)
- [x] `renderModes.ts` descriptor registry (RENDER_MODES/ORDER/DEFAULT/coerceRenderMode — future xray = new entry + MaterialKind, zero engine branching) + `bodyMaterials.ts` BodyMaterialLibrary (lazy per-kind shared sets; absorbs sketch-dim save/restore-verbatim incl. set-born-while-dimmed → constructor-default prior; setFaceColor COPIES — palette cache never aliased; two instances: MeshIngest committed + engine preview, preview stays lazy at setPreviewBody — no-init engine tests)
- [x] `BodyObject.applyMode(def)` (visibility + material from def; eager default materials at build — preview bodies never get applyMode; userData/group contract untouched — Picker/HighlightLayer/e2e childVisibility all stand; stale "shared engine-wide" rationale rewritten)
- [x] displayMode → settingsStore v3→4 (migrate backfill + custom `merge` coercion — same-version garbage blob still coerced; hydration synchronous, verified zustand 5.0.14) · viewportStore display exports deleted · tokens `--color-body-fill` #a9aeb6 / `--color-body-edge` #3a3f47 + `palette.referenceNeutral()` split (DatumLayer/RevolvePreview/RegionPickLayer keep ink-5)
- [x] DisplayModePopover (menuitemradio + aria-checked, SnapPopover pattern; ClusterButton `expanded` prop → aria-expanded on triggers; single openPopover slot — two popovers can't both open)
### Commit 2 — professional shading
- [x] NeutralToneMapping + optional `RendererHandle.createEnvironment` seam (PMREM RoomEnvironment, WebGL-only BY ABSENCE — no isWebGPU branch in engine; caller owns RT, PMREMGenerator.dispose doesn't free it) · env rebuilt on context restore DEFERRED via queueMicrotask (engine listener registers before WebGLRenderer's — regen would hit un-reinit'd GL) + disposed/handle guard · env disposed BEFORE rendererHandle in dispose() · `environmentRotation(π/2,0,0)` Y-up room → Z-up world (sampling only, Z-up invariant intact) · envReady in debugSnapshot
- [x] `lightRig.ts` pure math (key az+40° el+35° FLOOR 20° abs — top face brightest from every orbit incl. under-views; fill −75°/−15° unfloored; sphericalToOffset gained optional `out` — zero per-frame alloc, 3 existing callers untouched) · per-backend intensities applied POST-init (buildScene runs before isWebGPU known): WebGL env .35/key 1.9/fill .6/hemi .25, WebGPU lights-only 2.4/.85/.7 · hemi ground = palette.clear() (hard-coded Color(0.4,0.43,0.48) killed) · face roughness .5/metalness 0/envMapIntensity 1
- [x] `toneMapped:false` sweep — 30 overlay material sites / 13 files + body edge (tone mapping is for lit faces ONLY; README rule added) — design tokens render exact
- [x] Tuned against pixel-measured face values (projected bbox face centers off live canvas): top #b7bdc6 / front #979ca4 / right #81858c on #eaecef canvas — three-value separation, dark edges
- [x] **Gate PASSED** (2026-08-02, full tree): tsc 0 · FE 2012/149 · e2e 94/94 · hex clean (inputProbe pre-existing) · backend untouched (cargo/ctest stand). Known flake: ModelTreePanel F2-rename cross-file ordering (pre-existing, passes standalone/rerun)
- Flagged seams: sketch-dim opacity .35 + DoubleSide shows interiors (pre-existing, more visible on brighter body); WebGPU experimental = lights-only (no PMREM); persisted mode survives doc switches BY DESIGN; future xray = MaterialKind + RenderModeDef + popover row (opacity-mode stays pickable — traverseVisible follows `visible`, not opacity)
- [ ] Manual Tauri gate (USER): body reads studio-shaded w/ dark edges; display popover picks modes + persists across relaunch; sketch mode dim still works; context restore (sleep/GPU switch) keeps shading

## Execution rules
- Orchestrator: decisions/review only. WPs → Opus 4.8 subagents.
- RISKY WP = extra independent review pass.
- protocol/ or Descriptor.* or serde schema change = cross-track sign-off + fixture bump.
- Git: commit at gate boundaries (user-approved 2026-07-17). Initial commit e14774d.
