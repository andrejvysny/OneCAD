# Current State

Last verified: 2026-09-02 11:24 — DAILY DRIVER v2 PAUSED BY USER mid-WP-P: five WPs committed (`3a82910` WP0 · `a60da42` WP-C · `e7010ce` WP-E · `1d7b4a0` WP-V · `a38b940` WP-T1, all full-L3), WP-P worker+Rust halves LANDED UNCOMMITTED (2,718 insertions + 10 new files), FE half + gate + commit owed, on `master` (5 ahead of origin — push not authorized)

## NOW — KERNEL HARDENING (2026-09-02, plan `~/.claude/plans/act-as-senior-cad-glimmering-wreath.md`)

Last verified: 2026-09-02 12:41 — `master`, DIRTY: two uncommitted programs interleaved
(this one + Session 23's WP-P; see HANDOFF.md § Session 24 for the split). No commits this session.

- **Changed by this program:** worker `ops/{OpCommon.cpp,.h,BooleanOp,ExtrudeOp,RevolveOp,MirrorOp,
  HoleOp,TransformOp,PatternOp,ComponentOp}.cpp`, `kernel/validation/{ShapeAudit.cpp,.h,
  GeometryPrecision.cpp}`, `elementmap/{ElementMapPartition.cpp,.h,Ladder.cpp,Scoring.cpp,.h}`,
  `kernel/elementmap/ElementMap.h`, `session/ElementIdentity.cpp`; tests
  `test_extrude_boolean_modes.cpp` (new), `test_commit_tier_validation.cpp` (new, unbuilt),
  `test_wp6_ladder.cpp`, `test_wp5_partition_history.cpp`, `test_preview_op.cpp`,
  `test_revolve_boolean_modes.cpp`, fixtures `resolve_refs`/`executeplan_needsrepair`
  (`scoringVersion` 4), `worker/tests/CMakeLists.txt` (two add_test blocks appended);
  protocol `SCHEMA.md` (§7.2, §7.3 Extrude/Boolean/Revolve/Mirror, §9, §10, §14 ×3),
  `fixtures/extrude_add_disjoint_refusal.ndjson` (new), `fixtures/publication_refusal.ndjson`;
  Rust `regen/planner.rs` (`edited_from` claims 0), tests `hole_ops.rs`, `topology_rebind.rs`,
  `face_color_reopen.rs`, `step_export_attributes.rs`; FE `PreviewMesh.ts`,
  `ModelToolController.ts` (symmetric head), `ExtrudeChipControls.tsx` (ThroughAll gate),
  two FE tests; ledger `TODO.md`, this file, `HANDOFF.md`.
- **Gate measured BEFORE WP-E was written (main thread):** ctest **166/166** ·
  `ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` **93 targets / 1471 / 0** ·
  vitest **311 files / 5502 passed / 78 skipped** · tsc/clippy/fmt/hex/hygiene/verifiers clean ·
  Playwright chromium targeted 2/2 · full `bun run e2e` **in flight: 295 passed / 0 failed**
  (chromium done, webkit running) — log `scratchpad/e2e_gate.log` · sidecar restaged (pre-WP-E).
- **WP-E BUILT, FIXED, REVIEWED, GATED (evening session, plan
  `~/.claude/plans/act-as-senior-cad-whimsical-sedgewick.md`):** did not compile as written
  (missing include); fix-round for cancel propagation + `stage_boolean()` non-destructive
  auxiliary BOPs; **ruling D3b** — Tier B only where NEW geometry is published, Tier A kept for
  isometries (TransformBody, unfused Mirror/Pattern children) and `generator` components after
  `test_component_ops` ran > 5 min in `BOPAlgo_CheckerSI` on a modeled thread face; protocol
  audit `approve` after 4 blockers + code fixes (cancel on 5 pre-existing Tier B sites,
  non-destructive `PrepareOffsetFace`/`ExportGeometry`, prose, 2 cross-track fixtures);
  adversarial review 4 HIGH fixed red-first (edge-extrema crosses-axis classifier, 0.01 mm
  anchor-decisive threshold, fallback lane drops `editedFrom: 0`, h6a pins WHICH corner);
  SG90 ingest regression fixed (Boolean ceiling from the worse input). ctest 168/168. Full
  cargo / vitest / e2e in flight — rows in TODO.md § KERNEL HARDENING. **Also landed: WP-P P2b**
  (`SketchSessionDto.projections` provenance) so P3 can survive a reload.
- **Key decisions:** non-touching Add / empty Cut REFUSE by name (never split / silent delete);
  Symmetric distance is the TOTAL; ThroughAll needs a target; resolverVersion 4 = signed
  `outward` + sub-shape anchor + anchor-decisive tie-break (relative anchor REJECTED — see
  HANDOFF dead-ends); every committed NewBody is Tier B (WP-E, unverified); no dynamic workflows.
- **Blockers:** none technical. `document_runtime.rs` is under concurrent WP-P edit — the redo
  claim and the `finish_sketch` reorder wait for coordination.

## NOW — DAILY DRIVER v2 (2026-09-01, plan `~/.claude/plans/act-as-senior-software-abstract-eclipse.md`)

- **Program (user-chosen after a full state review, a Codex brainstorm and a local adversarial plan
  review):** WP0 → **WP-C** vendor STEP components → WP-E expressions+units → WP-V section view →
  WP-T1 cosmetic threads → WP-P project edges → WP-S sweep → WP-T2 modelled threads → WP-L loft →
  WP-X dogfood exit gate. Sweep and Loft are APPROVED (reversal recorded). One master commit per WP
  gate; `TODO.md` § DAILY DRIVER v2 is the ledger with every decision and gate row.
- **WP0 (`3a82910`):** the four unpushed commits pushed (CI 9 green; `tauri-composition` cancelled at
  the 95-min job timeout inside "Build and stage worker" — infra; `linux-worker` queued — self-hosted
  runner offline); stale sidecar restaged; autosave durability trio (`durable_write`, quarantined
  corrupt `recents.json`, FE `Autosave failed` hint); regen timing split (`RegenTimings`) and a
  40-feature perf baseline. **Finding: every edit replays from step 0** (checkpoints only on
  explicit save) — ~480 ms for 40 pocket features; next-program candidate.
- **WP-C (this commit):** `PlaceComponent` `source.kind = "profile"` (length-parametric extrusion of a
  canonical planar face) + §7.8 `ExtractPrismProfile` (protocol-audited before AND after landing;
  adversarial review found and fixed a non-prism acceptance and an off-centre face); the first
  writer of `embedded`-kind packages; package revision now folds referenced blob digests; a headless
  ingest core + `onecad-library-ingest` CLI + `ingest_components`/`pick_component_files` commands +
  FE "Import components…"; tracked recipe `STEP/ingest.toml`. **Ingest: 6 of 7 vendor files** — five
  Rollco profiles as `profile` packages (any length), SG90 as a fused three-solid embedded package;
  **NEMA17 refuses honestly** (its envelope solids fail OCCT's self-intersection check, so Tier B
  refuses the fuse) — options: a cleaner download, or seed the worker's existing `nema17` frame
  generator. Full L3: ctest **160/160** · cargo **91 / 1379 / 0** · vitest **306 / 5281 / 78 skipped** ·
  e2e **496 / 0** (27.6 min) · kernelbench 136 unchanged · fmt/clippy/hex/hygiene/verifiers clean.
- **Retracted plan claim:** `imports.rs` `unit_scale = 1.0` is correct — the STEP reader always
  converts to mm.
- **WP-E expressions + units LANDED (third commit):** `=` expressions with arithmetic, chained
  variables (cycle detection), unit suffixes (`mm cm m in deg rad`), trig-requires-an-angle;
  bare literals inside `=` are canonical mm/deg with guardrails (live preview echoes the
  display unit; a pure-literal `=2` stays display-unit); the expression STRING is stripped
  from the planner hash and the wire (Opaque untouched; goldens unmoved); edit-time refusals
  are edit-scoped; later breakage = `EXPR_UNRESOLVED` diagnostics, plan gated below the step;
  `rename_variable` rewrites references as one undo step; detach clears placement bindings
  (adversarial F1). Both lanes honest (TS port parity 64/64 on the shared 53-case fixture).
  Full L3: ctest 160/160 · cargo 92/1426/0 · vitest 5413/0 · e2e 499/1→isolated 2/2 (known
  ledger signature) · kernelbench 136 unchanged.
- **WP-V section view LANDED (fourth commit):** stencil-capped cut against XY/XZ/YZ with
  offset+flip, `⇧X`, NavPill + Layers controls; offset seeds from the scene-bounds midpoint;
  highlights/ghosts join the clip; picker filters clipped hits then takes the first survivor;
  cap unpickable, hidden during a sketch session; WebGPU refused with a hint. Adversarially
  reviewed (stencil algebra hand-verified), two HIGH findings fixed red-first.
- **WP-T1 cosmetic threaded holes LANDED (fifth commit):** `HoleParams.thread` (presence-
  discriminated; `cosmetic|simplified|modeled` — the shipped vocabulary; only cosmetic
  implemented, the worker refuses the rest by name), **Option B**: `diameter` IS the drill
  (FE fills tap-drill = major − pitch), worker geometry untouched; `pitchMm`/`depthMm`
  expression-drivable; byte-identity pinned by a pre-field golden
  (`cf8f35ac…`); fixture `hole_threaded.ndjson` with the identical-matcher trick.
  Combined tip gate: ctest 161/161 · cargo 92/1433/0 · vitest 5500/0 · e2e **514/0** ·
  kernelbench 136 unchanged.
- **Next:** WP-P project edges (wire change; design + review deltas in the plan file:
  `ProjectToSketchPlane` verb, snapshot+`PROJECTION_STALE` on projected-UV comparison,
  `promote_selection` promotion, sketch-level `Sketch.projections`, Update/Detach). Owed user input: the 2–3 dogfood
  parts for WP-X; `.clang-format` (`DisableFormat: true`) to neutralise the `auto-format.sh` hook;
  whether to push `3a82910` + the WP-C commit. Owed user-run gates unchanged (19-row checklist,
  Tauri sketch smoke, dirty vendor STEP for G4).

## PREVIOUS — WP6 CLOSE-OUT + NEXT PROGRAM (2026-08-24, plan `act-as-senior-software-purrfect-moler.md`)

- **Sketch-UX hardening merged to master by the USER** (`c731a76`); post-merge sanity measured:
  tsc clean, vitest 300 files / 5145 / 0 failed (first time the merged combination was gated).
- **Phase 1 = WP6 small-caliber batch COMPLETE at FULL L3, measured on the main thread:**
  ctest **154/154** · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **87 targets / 1329
  passed / 0 failed** (teed) · tsc clean · vitest **302 files / 5180 passed / 78 skipped**
  (one un-teed single-drop, unnamed — flake ledger) · kernelbench **136 rows unchanged** +
  semantic-compare OK · fmt/clippy/hygiene/hex/verifiers clean · e2e chromium **241/241**;
  webkit **241/241 alone at calm load** (the loaded 51.8-min combined run went 479/3, all
  three webkit browser-process stalls, triaged and named in TODO.md — not a product defect).
  Sidecar restaged and current.
- **Landed:** chamfer distance-angle mode `angleDeg` end to end (SCHEMA §7.3 + §7.6
  equal-leg-oracle normative + §14, first canonical chamfer fixture, worker AddDA, Rust
  core validation + flip gate, FE chip with last-authored-wins exclusion, real-worker Rust
  proof 991.3397 exact) · MirrorBody fuse toggle (`chip-mirror-fuse`) · evidence gaps closed
  with no defect (Shell multi-face, countersink browser, circular partial-sweep) · both QA
  manifests corrected · FE clamp no longer applies the equal-leg range bound to asymmetric
  chamfers (live `distance2` hole closed) · `previewOps.ts` mode-drop found and fixed.
- **Phase 2 LANDED (`bb70e1f`):** near-action error pulse (+ six refusal sites raised to
  error severity) · Esc-exit finished-hint via `exitSketch(reason)` · screen-constant badge
  standoff (signed `offsetPx` in `HtmlOverlayDriver`) · donut-hole fill (even-odd,
  boundary-vote containment). Final e2e 491/1 (the drop named, 6/6 isolated).
- **Phase 3 LANDED:** `entityStates` per-entity constrained state end to end — worker
  (`d572d66`, PlaneGCS dependent-set derivation, SCHEMA §7.4, first canonical sketch
  fixture) + Rust typed pass-through + tauriClient uuid→frontend re-key + honest mock
  subset + per-entity viewport materials. Adversarial review SOUND (four conditions
  discharged; circle can't earn green on the real lane — the recorded dup-centre gap,
  under-report direction, disclosure-pinned). Full L3: ctest 156/156 · cargo 88/1336/0 ·
  vitest 5261/0 · kernelbench 136 unchanged · e2e **494/0** both projects.
- **Next (queue, not started):** dup-centre fix (worker honours `centerRef`) unlocks circle
  green + kills the +2 dof inflation · remaining deferred-wire sketch items (slot tangency,
  centerRect symmetric, quadrant axis-align, W3 project-edges) · Daily Driver W2 section
  view (seam located). Owed user-run items unchanged (19-row checklist · Tauri sketch smoke
  incl. the new per-entity tint · dirty vendor STEP for G4 micro).

## PREVIOUS — KERNEL CONTINUATION RESUMED (2026-08-20, plan `use-the-fable-orchestrator-skill-kind-dawn.md`)

- **Branch `master`, clean (only untracked `.claude/`), 17 commits this session
  (`3afbdf9` → tip `9f03b72`), tip gated at FULL L3, measured on the main thread:**
  ctest **153/153** · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **86 targets / 1313
  passed / 0 failed** (teed) · `bunx tsc` clean · `bun run test` **297 files / 5012 passed /
  78 skipped** · `bun run e2e` **464 passed / 0 failed** (33.0 min, both projects, retries 0) ·
  kernelbench `compare` **136 rows unchanged** + `semantic-compare` OK at every gate ·
  fmt/clippy/hygiene/hex clean. Sidecar staged and current.
- **Landed:** WP1 complete (G1r renames · G2 `diagnostics[].reasonCode` · G3 micro/sliver
  redefinition + 120-row census · G4 SLIVER bound only — micro bound held for a characterized
  dirty-import fixture) · WP2 ToNext adversarial campaign (9 exact cases, no defect) ·
  **WP3 C1–C6 complete: OffsetFace `resultPolicyVersion: 3`** — side push preserves the fillet
  (1191.4159265358979 vs V2's 1405.6637061435916; V2 replays byte-identically forever;
  multi-blend works; the rib decoy proved V2 destructive) · WP5 exact tilted ToFace ·
  WP4 `AnalyzeEdgeOpRange` (measured fillet/chamfer range → FE slider clamp).
- **The reviews were load-bearing:** adversarial passes found and red-first-fixed a wrong
  `gp_Lin::Distance` branch, a co-surface false-Proved, a cross-key identity leak that could
  auto-bind a support id onto the rebuilt blend face, and a mixed-radius silent rebuild;
  protocol audits found the `bounded_diagnostic` allowlist blocker, two latent V2 preview/
  re-edit defects, and a false §14 history claim. Full evidence: TODO.md § KERNEL
  CONTINUATION — RESUMED (gate rows, seams, flake ledger).
- **Next:** WP6 small-caliber batch (chamfer angle-distance · MirrorBody-fuse UI · evidence
  gaps) — scoped in the plan file, NOT started (user paused). Owed: user-run 19-row release
  checklist (predates this session) · dirty vendor STEP for the G4 micro half · linux
  `clean_build` dispatch. `HANDOFF.md` (session 22) is the entry point.

## PREVIOUS — DAILY DRIVER v1 (2026-08-19, plan `act-as-senior-cad-floofy-locket.md`)

Capability + ship, chosen with the user over kernel continuation. Waves: W0 baseline + gate
triage + the evidence hole · W1 ship lane (unsigned) · W2 section view (capped) · W3 project
edges into the active sketch · W4 UX trust polish · W5 close-out. **W0 and W1 are committed
(`0d4e11c`, `8e121b5`); W2 has not started.** Full record: `TODO.md` § DAILY DRIVER v1.

- **The app is INSTALLED and healthy for the first time.** `/Applications/onecad.app`, ad-hoc
  signed, built by the new `scripts/package-macos.sh --install`. Second launch: 89 log lines,
  zero WARN/ERROR, `geometry worker pre-warmed`, OCCT 8.0.1 fingerprint `0a6a1dce34181289`.
- **W1's finding is the one to carry forward: a bundle built by following `docs/PACKAGING.md`
  could never start its worker.** The embedded manifest pins the STAGED sidecar's SHA-256, and
  `bundle-dylibs.sh` rewrites that sidecar — so the bytes can never match, and the app refused its
  own worker four times then gave up. `ci.yml`'s two-pass seed/lockstep dance was always correct;
  it lived ONLY in the workflow file. A procedure that exists only in CI is not a procedure the
  project has. No Rust changed — the check was right, the pipeline was wrong.
- **W0's finding: the e2e lane was deleting its own evidence.** Playwright wipes the whole
  `outputDir` at the start of every run, so the triage re-run destroyed the failure being
  triaged — the measured reason the 2026-08-17 sweep left only `.last-run.json`. Every run now
  gets `test-results/run-<stamp>`. Two things running it found: the fixture already captured on a
  hard timeout (so capture was never the defect), and the first fix was wrong because Playwright
  re-loads the config in every worker process.
- **Manual gates triaged 30 → 7 new rows.** 19 were already retired by the 2026-08-08 round and
  never ticked; "manual Tauri smoke" appeared SEVEN times and is one check, covered by
  `e2e-tauri/specs/composition.e2e.ts:226`. `docs/qa/MANUAL_RELEASE_GATES.md` is 19 rows.
- **Baseline at `33dd36c`, each suite run alone:** ctest **136/136** · cargo **86 targets / 1319
  passed / 0 failed** · fmt/clippy clean · tsc clean · vitest **297 files / 4985 passed / 78
  skipped** · `bun run e2e` **464 passed / 0 failed** (31.9 min, both projects, retries 0) · hex
  gate empty.
- **OWED, USER-RUN:** the 19-row checklist in `docs/qa/MANUAL_RELEASE_GATES.md` against the
  installed app — § 4 autosave crash recovery above all, the only block with data-loss risk.
- **Next:** W2 section view (frontend-only, no wire change). Seam already located —
  `BodyMaterialLibrary` (`src/viewport/engine/bodyMaterials.ts:56`) owns every body face/edge
  material and has a `refreshColors()`, so clipping planes attach per-material there rather than
  globally on the renderer (global would clip sketches and gizmos too). Note `CadRenderer`
  (`renderer.ts`) exposes no clipping surface at all and has a WebGPU path behind it.

## PREVIOUS — TRUST & DELIVERABLE COMPLETE (2026-08-17)

## PREVIOUS — TRUST & DELIVERABLE W0–W2 (2026-08-16, superseded by the section above)

- **Branch:** `master`, **3 commits pushed this session** (`b626f15`, `c17b497`, `971ef41`) and the
  12-commit backlog with them — CI had never seen the sketch-snap or autosave work before today.
- **Working tree DIRTY: W2 (DI-4) is complete and green but NOT committed.** 7 files:
  `element_index.rs` · `checkpoint.rs` · `document_runtime.rs` · `lib.rs` · `worker/wire.rs` ·
  `tests/face_color_reopen.rs` · `TODO.md`.
- **Local gate, measured, not inferred:** `bunx tsc --noEmit` + `bun run build` clean ·
  `bun run test` **295 files / 4949 passed / 78 skipped** · `ctest` **135/135** ·
  `cargo fmt`/`clippy -D warnings` clean ·
  `ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` **1284 passed / 0 failed / 84
  targets** · hex gate empty · `bun run e2e:tauri` **1 passing** · `bun run e2e` **462 passed /
  0 failed** (26.2 min, both projects, retries 0).
- **Browser lane progression this session, same command each time: 451/11 → 458/4 → 462/0.**
- **CI is NOT fully green yet.** At `971ef41`: 11 jobs green (`frontend`, `linux-worker`,
  `linux-kernelbench`, `occt-fingerprint`, `worker-8.0.1`, `rust-8.0.1`, both persistence jobs,
  OCCT build), **`e2e-chromium` and `e2e-webkit` each 230 passed / 1 failed** on
  `sketch-snap-rendering.spec.ts` — and at a DIFFERENT assertion (`:118`) than the one W1b fixed,
  so it is progress, not a regression. `tauri-composition` was still running at snapshot time.
  Detail + the fix direction: `TODO.md` § TRUST & DELIVERABLE → NOW.
- **W1 closed MC-R9 as a product regression, not a flake.** The model-tool chip covered the value
  arrow's grab area (measured: the arrow's own grab pixel resolved to `chip-cancel`), so the arrow
  could not be grabbed wherever the chip sat on it. `HtmlOverlayDriver` now takes a per-frame
  keep-out box — the consumer `ce3d6bf` said "V2 will" add.
- **W2 closed DI-4**, and the ladder's refusal of the first attempt is itself the finding: a box's
  two caps score 1.0 with margin 0 on descriptor evidence alone, so the ANCHOR is what makes a
  persisted id re-bindable. A repair-lane defect fell out of it — a `needsRepair` whose anchor was
  `{}` was silently dropped by the response parser, so a dialog with five candidates would have
  rendered as "nothing to resolve".
- **The packaged-app lane (MC-R4) has now RUN, and passes** — first execution on any machine, with
  both dev worker paths hidden so the bundled sidecar is what answers. Its first run also found a
  false assertion in its own spec (`documentRevision` is session-scoped; the spec expected it to
  survive a reopen), corrected with the evidence recorded in place.
- **The staged sidecar is CURRENT.** `src-tauri/binaries/onecad-worker-aarch64-apple-darwin` and
  `worker/build/onecad-worker` hash identically and match the manifest, so every "blocked on T0's
  stale sidecar" note below — LGU-1's WP-D/F/G/I sequencing especially — is obsolete.
- **Program:** W0 baseline truth + CI · W1 chip stops covering the value arrow (MC-R9 is a real
  product regression, not a flake) · W2 DI-4 ElementId rebind at open · W3 DI-5 XCAF STEP export ·
  W4 3MF export · W5 T2 result truth ("saved + loud failure banner") · W6 close-out.
  Windows stays out of scope by decision. Full record: `TODO.md` § TRUST & DELIVERABLE.

## PREVIOUS — KERNEL HARDENING CLOSE-OUT (branch `kernel/semantic-publication-hardening`, PR #4 — merged as `5106f80`)

- **Where:** git worktree `../OneCAD-kernel-hardening` on
  `kernel/semantic-publication-hardening`, continuing from `69be0c2`. 23 branch commits
  plus the merge of `master` at `def327b` (5 commits, all frontend). The merge had
  exactly ONE textual conflict — this header — and `TODO.md` / `e2e/variables.spec.ts`
  auto-merged.
- **What landed:** the four gaps an independent review left open — exact `Extrude.ToNext`
  directional extremum (edge/face interiors, plus explicit SEATED-profile semantics),
  the OffsetFace worker trust boundary (strict typed arrays, `inputs[]` arity, typed-ref↔id
  equality), Revolve strict `booleanMode` + checked axis-sketch solve, and the missing
  closure-face rebind regression test. Plus the Gear modeling-coverage row and the two
  evidence lanes it required. Full detail + the load-bearing proofs: `TODO.md` §
  "GATE — kernel semantic-publication hardening close-out (2026-08-15)".
- **Explicitly NOT done** (deferred, recorded in `TODO.md`): production OffsetFace reblend,
  a central precision/tolerance module, micro-edge/sliver-face publication enforcement.
  All three are now planned as WP1–WP3 of the continuation program — see `TODO.md`
  § "KERNEL CONTINUATION".
- **Sidecar:** restaged from this worktree's build, so `src-tauri/binaries/` matches the
  worker under test.

## PREVIOUS — LGU-1, LIBRARY & GENERATORS UNIFICATION (session 20, on `master`)

- **Branch:** `master`. Two commits this session, both frontend-only.
  1. The UI/UX pass that was sitting uncommitted in the tree (library browser
     becomes a full-size modal, Variables takes the sidebar tab, titlebar
     reorder). It was marked LANDED in `TODO.md` but had never been committed;
     verified green first, then committed on its own so it stays bisectable.
  2. **LGU-1 WP-A** — truth & vocabulary. See `TODO.md` § LGU-1 for the full
     record, including the three corrections the specification needed and the
     two items refused/deferred by name.
- **Gate, measured:** `bunx tsc --noEmit` clean · `bun run build` clean ·
  `bun run test` **4501/4501** (276 files) · hex gate empty ·
  **`bun run e2e` 425 passed / 1 failed** at retries 0.
- **The e2e lane is NOT green.** `filletChamfer.spec.ts:198` failed — a drag
  gesture that did not flip the armed edge-op type. Empty `pageerror.log`, the
  FSM arming normally, 13/13 in isolation, and WP-A touches nothing on that
  path. Recorded as a SECOND MC-R9 datapoint (§ T4) rather than dismissed: the
  ledger's own rule is that a browser-lane nondeterminism closes on a measured
  root cause and never on a clean re-run. Two specs now share the signature —
  `revolve-commit.spec.ts:111` and this one — and both are pointer-drag
  gestures failing under load.
- **NOT claimed: `cargo` and `ctest` did not run.** T0's staged sidecar
  (`src-tauri/binaries/onecad-worker-aarch64-apple-darwin`, 12:02) predates
  `worker/build/onecad-worker` (18:12), and `bundle.externalBin` plus
  `manager.rs`'s SHA-256 manifest check make every cargo command in this
  worktree untrustworthy until it is restaged. WP-A touches no Rust or C++, so
  nothing was skipped that WP-A could have broken — but this BLOCKS LGU-1's
  WP-D/F/G/I, which is why the program is sequenced frontend-first.
- **Next:** WP-B (re-edit convention + single commit surface), WP-C (armed =
  right panel), WP-E (generators gallery) — all frontend. Then restage the
  worker and take the wire-level packages.
- **Standing:** T1's data-integrity items (DI-1/2/3) still formally outrank this
  program; they remain unstarted by explicit decision, not oversight.

## SUPERSEDED — GEAR GENERATOR (session 19)

> The section below described the gear work as uncommitted on `f5b686f`. It
> LANDED as `b9bcaf7`, and `TODO.md` § GEAR GENERATOR records G1-h.1–h.4 as
> done — including the chip UI this header once said was missing. `master`'s
> copy of this note also listed `src-tauri/tests/gear_ops.rs` and
> `e2e/gear.spec.ts` as owed; THIS BRANCH ADDED BOTH (they were required by the
> `Gear` modeling-coverage row), and the four-suite re-run is recorded in the
> 2026-08-15 gate. Still genuinely owed: `protocol/fixtures/gear_*.ndjson`.
> Read the rest of this section as history.

- **Branch:** `master` at `f5b686f` (the Component-Library merge landed DURING this
  session — the tree moved under the gear work; nothing conflicted, the gear files
  are additive). Working tree **dirty**: 21 modified + 20 new files, **+1474
  insertions** in tracked files plus the untracked gear tree. **Nothing committed.**
- **What this is:** a NEW feature track (port of `freecad.gears` as a native `Gear`
  op), deliberately parallel to the T0–T2 queue below. It touches no
  data-integrity code.
- **Full gate, measured this session:**
  - `cargo fmt --all --check` clean · `cargo clippy --workspace --all-targets -D warnings` clean
  - `ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` **1259 passed / 0 failed**
  - `ctest --test-dir worker/build` **131/131** (was 119 before this work)
  - `bunx tsc --noEmit` clean · `bun run build` clean · `bun run test` **4455/4455** (271 files)
  - hex gate empty
  - Cross-check vs the real reference implementation: **2673/2673 compared agree**,
    24 refusals all one legitimate degenerate case
- **Key decisions:**
  - opType is **`Gear`**, not `Generator` — `ComponentSourceRef::Generator` already exists.
  - Publication is **`TierB`** (the spec said TierA; the spec had the tiers inverted).
  - Helical/herringbone are refused **BY NAME** as `UNSUPPORTED`, not silently
    flattened to a spur gear; the fields exist and round-trip so the payload will
    not change shape when the sweep infrastructure lands.
  - Fillets deliberately **not** implemented — upstream's index-based insertion is
    the fragility this port refuses to carry.
  - Matched an upstream BUG on purpose (its undercut trim branch is unreachable);
    parity is the goal and the deviation is documented in `InvoluteMath.h`.
  - Two frozen contracts changed (`toolbarContract`, `keymapContract`) under the
    explicit user-visible-change decision recorded in `TODO.md` § G1.
- **Blockers:** none for the gear work.
- **Caveat that will bite the app (not the tests):** the STAGED sidecar
  `src-tauri/binaries/onecad-worker-aarch64-apple-darwin` is from 12:02; the built
  worker is 18:12. `cargo test` was valid via `ONECAD_WORKER_PATH`, but **the
  packaged app does not contain the Gear op**. Restage with
  `ONECAD_OCCT_ROOT=~/.onecad-occt/8.0.1 scripts/build-worker.sh Release`
  (this is TODO's existing T0 item, now with a second reason).
- **Owed for the G1 gate:** the controller + chip UI (`ModelToolController`,
  `toolChipStore.showGear`, `GearChipCluster.tsx`), plus
  `src-tauri/tests/gear_ops.rs`, `protocol/fixtures/gear_*.ndjson`, `e2e/gear.spec.ts`.
- **Defect found in the repo's own test convention:** `test_polygon_fill.cpp`'s
  "exit code == failure count" idiom reports PASS when the failure count is a
  multiple of 256 (8-bit exit status). Hit for real during mutation testing. All
  gear tests clamp; that file still has the raw idiom.

## BRANCHES MERGED (2026-08-14) — one trunk again

`OneCAD-Component-Library` and `master` have merged. Every section below belongs
to one of the two programs that were running in parallel until now — the
Component Library + document variables on one side, the modeling-UX unification
U0–U7 + publication/identity-V3 hardening + the data-integrity audit on the
other. Both are live; neither supersedes the other.

Three things changed that a reader of the older sections would not expect:

- **`PlaceComponent`, `SetComponentParams` and `DetachComponent` now return an
  `ApplyOperationResult`**, not `void`. They go through `applyEdit` like every
  other mutating command, so callers must read the terminal instead of treating a
  resolved promise as success.
- **A preview mesh degrades its LOD rather than exceeding the transport limits.**
  `PreviewOp` was the last inline-mesh producer with no ceiling; a handle now
  reports the tier the bytes were actually built at, which may be coarser than
  the one requested.
- **SCHEMA §7.2 carries an explicit mate carve-out.** A published `ok` step may
  carry `planStep.needsRepair[]` when — and only when — the repair target is a
  mate. This was always the behavior; only the prose was missing.

Still exempt from the result-truth doctrine: `upsertVariable`, `removeVariable`,
`replaceComponent`. See `TODO.md` § MERGE for why and the two ways out.

## COMPONENT LIBRARY — CATALOG TRIMMED TO ONE FAMILY (2026-08-13, CL-TRIM)

**Read this before anything below that counts packages.** The shipped catalog
is now ONE component: `onecad.std.iso4762`, the socket head cap screw. The nine
other seeded families are deleted — manifests, worker generators, dimension
tables, mirror tables and tests — as is the `nema17-mount` starter that placed
one of them. `SEED_VERSION` is 4; there are 2 starters, not 3. Everything below
describing "10 seeded packages" or "3 starters" is history, accurate at the time
it was written.

The dispatch, the tables' generic shape and `GeneratorRequest::text_params`
survive on purpose: they are what a re-added family needs on day one.

Seeding now also PRUNES a retired built-in (`SEED_VERSION` 5): a
`.seed-ledger.json` records the SHA-256 of every manifest this app writes, and a
directory is removed only when it is ledgered, no longer shipped, holds nothing
but that one file, and still hashes to the ledgered value. Anything else is
adopted — left alone, dropped from the ledger, it is the user's. Roots at marker
≤ 4 have no ledger and need one manual `rm`; the dev machine's was cleared.
Templates are not pruned (their bytes are generated per install, so there is no
unmodified-proof to check).

Full rationale and the flagged seams are in TODO.md § CL-TRIM / CL-TRIM.2.

## COMPONENT LIBRARY — LIVE DELTA (2026-08-13, session 17, PROGRAM COMPLETE)

Session 17 closed the Component Library program (spec §12) end-to-end. After
the hardening below, the finishing waves landed, each gate-green:

- **WP-F2a/b** — ISO 15 bearings + NEMA 17/23 generators (self-authored
  tables, 10 seeded packages) and 3 seeded starter templates (Blank,
  3D-Printed Part with a real Build-plate datum, NEMA 17 Mount with a real
  PlaceComponent record).
- **WP-F1.1** — attachment local frames end-to-end (`[attachments].frame` →
  frozen `mate.selfFrame` → `S ∘ F⁻¹` in both solvers, verbatim parity).
- **WP-F1.2** — authoring UI: viewport attachment picking (frame origin =
  clicked point) + union-at-bake on the MULTI_SOLID_BODY refusal.
- **WP-F3** — free-space ghost follow/drop; mock lane measures real cylinder
  classification (auto-size finally e2e-covered); mock catalog/template
  parity.
- **WP-VE.1** — variables actually drive geometry: `Scalar.expr` substituted
  on the regen mirror before the planner hash; loud per-step failure on a
  missing/unsupported binding; no-expr documents hash byte-identically.
  (Found blocked-honest by the F1.3 agent: expr was storage-only in every
  layer; proven with kernel volumes before building.)
- **WP-VE.2** — variables surface: list/upsert/remove commands, Variables
  inspector section (hosted in the empty state), `=name` binding in
  Scalar-backed fields rendering only backend-recorded `primaryExpr`.
- **WP-F1.3** — the re-bake lane: document-source components declare free
  params mapped to source variables; `setComponentParams` replays the frozen
  source on an EPHEMERAL worker (Drop-guarded, never touches the open
  session), requires exactly one solid, re-bakes the blob, updates the same
  RecordId. Proven: depth 10→20 moves the placed body 4000→8000 mm³ and
  survives reopen with the library deleted.

Known deferrals (recorded in TODO.md): WP-VE.2b (Hole binding), WP-VE.3
(sketch-dim variables), re-bake one-body/union limits, P4 registry.

## COMPONENT LIBRARY — session 17 hardening (WP-H0…H4)

Session 17 (this one): full-branch review + hardening. `master` (39f5839)
merged as `f242712` — the earlier in-tree resolution had dropped all four
master-side SCHEMA hunks; re-merged keeping both sides. Then: WP-H1 (path-safe
component ids), WP-H2 (the gesture now RECORDS its mate — spec §12's
re-seat flow reachable from the UI; also fixed `register.ts` dropping `params`
so auto-size commits matched their ghost), WP-H3 (WebGL context-loss recovery;
mock lane resolves authored components), WP-H4 (spec §13 ratifies the four
deviations). Full gate green at each step — see TODO.md WP-H entry.
Next (approved): WP-F1 authoring completion, WP-F2 bearings/NEMA + starter
templates, WP-F3 gesture polish.

## COMPONENT LIBRARY — session 16 (Phase A + P3 CLOSED)

Last verified 2026-08-13. Branch `OneCAD-Component-Library`, clean at
`0166ac4`. Seven commits this session, each its own gate:

- `970b3db` WP-3.2 (the previous session's delta, committed as-is)
- `68a3089` **WP-A1** — per-family generator dispatch + six ISO fastener
  families. `source.generatorId` was read but never dispatched: EVERY id built
  an ISO 4762 socket cap screw. Now `iso4014` `iso4017` `iso4032` `iso4762`
  `iso7089` `iso7093` `iso7380`, each with an analytic exact-volume ctest, and
  an unregistered id fails loudly.
- `be73bf8` **WP-A2** — the catalog SHIPS. There was no seeded library anywhere
  in the repo, so a real user's panel was empty regardless of what the kernel
  could build. Seven `component.toml` packages are embedded in the binary and
  installed on first run; a user's own copy is never overwritten.
- `bee0d43` **WP-A3** — auto-size on hole rims, plus the free-param path that
  made it possible: neither `placeComponent` nor the preview's generator source
  carried params before, so every ghost previewed the DEFAULT size.
- `d42f47a` **WP-B1** — `platform.menus`: cross-module context-menu items. A
  provider's own rows could declare actions; nothing let a different module add
  one, which is what "Save as Component" on a modeling-owned body row needs.
- `8ffb66c` **WP-B2** — Save as Component. Author a body → `document` package →
  place it back as the same solid, proven against the real worker.
- `816e8a2` **WP-B4** — ReplaceComponent + opt-in upgrade UI (in place at the
  same RecordId; a mate rides across by attachment NAME or is dropped and
  named).
- `0166ac4` **WP-B3** — project templates: save, list, start from. The start
  screen's `templates` key is no longer a stub.
- **WP-B5 + WP-B6** — e2e for authoring and templates, and real 3D previews for
  every component: card thumbnails from ONE shared offscreen renderer (a
  context per card is how a browser runs out of them), a live orbitable detail
  view, and `component_preview_mesh`, which runs with NO document open because
  the most useful place to browse a catalog is the start screen.

**Spec §12's definition of done is now reachable end to end in the shipped
app**: the library arrives populated, a hovered hole auto-sizes the fastener,
the ghost and the commit agree, a placed instance can be re-parameterized,
replaced, upgraded or detached, a user can author their own component from a
body, and a project can start from a template.

Known limits, all recorded rather than hidden: an authored component seats at
its MODEL ORIGIN (no per-attachment frames on the wire yet); `document`-source
components have no re-bake lane, so their params are not editable; auto-size has
no Playwright coverage because the mock lane's `classifyElement` cannot report a
cylinder; bearings/NEMA tables and the opinionated starter templates are
deferred by decision.

Gate at the session's end: worker ctest **119/119** · `cargo test --workspace`
green · `cargo fmt`/`clippy -D warnings` clean · `tsc` clean · vitest **251
files / 4206** · Playwright **416/416** · hex gate empty. (One Playwright sweep run under heavy
concurrent load failed `revolve-preview.spec.ts`, a foreign spec, and it passed
2/2 in isolation — the machine-contention flake this repo's gate notes already
describe.)

## COMPONENT LIBRARY — LIVE DELTA (2026-08-13, session 15, P3 WP-3.2)

Last verified 2026-08-13 12:15. Branch `OneCAD-Component-Library`, **dirty** —
the whole WP is uncommitted on top of WP-3.1 (`81f535a`). 29 modified files +
3 new (`worker/src/io/ExportGeometry.{h,cpp}`,
`worker/tests/test_component_blob_source.cpp`). This session:
the `embedded` + `document` component source kinds (spec §2.1) on one shared
blob lane, plus the `ExportGeometry` verb that bakes what they carry.

Scoping found `embedded` had never shipped either (resolve + worker both refused
it despite spec §10's P1 line), so the spec §12 differentiator — reopen with the
library deleted and still see the part — had no automated proof for anything but
a generator. Both kinds land together, and that claim is now a real-worker test.

Decided before implementing: a `document` package carries `source.onecad` PLUS a
baked geometry blob; placement copies the blob and nothing replays a frozen
document. Replay would need a SECOND worker process (one session per process)
and the library present at every regen — which spec §4 rules out anyway.

Worker: `io/ExportGeometry.{h,cpp}` (§7.8, the inverse of InspectStep's
conversion lane) + `ComponentOp.cpp::read_source_blob`, one arm shared by both
blob kinds, exactly-one-solid enforced, falling through to WP-3.1's mate reseat.

Rust: `ComponentSourceRef::Document` + `Embedded.brep_format` behind a shared
`blob_ref()`; `referenced_import_shas` now pins component blobs (without it the
blob is dropped at the first save); `DocumentRuntime::stage_component_blob`;
`library.rs` resolves + stages all three kinds under the edit's own lock.

Frontend: `resolveComponentSource` (new command) resolves the REAL source and
stages its bytes at arm time — the ghost lane previously hardcoded a generator
source, so a blob-backed component previewed the M6 screw and committed
something else.

Two defects found by running it: `ExportGeometry` first required a bare
`TopAbs_SOLID` and refused the first real body (published bodies are solid-LIKE
— a compound wrapper is legal), and `xbf` face colors can only be carried for a
body that flattens to one solid.

Gate (all four suites RUN this session, none inferred): worker ctest 118/118 ·
fmt/clippy clean · `cargo test --workspace` 100% green · tsc clean · vitest 245
files / 4162 · Playwright 404/404 (21.5 min) · hex gate empty.

Blockers: none. One flake seen and dismissed: `ModelToolController.wave2.test.ts`
> "editExtrudeFeature while already armed…" failed once in a full-suite run and
passed 18/18 in isolation — its own comment already documents that flake, and it
was green in the final full run.

## COMPONENT LIBRARY — LIVE DELTA (2026-08-13, session 14, P3 WP-3.1)

Branch `OneCAD-Component-Library` (worktree), on top of the WP-2.6 commit
below. This session: persistent mate re-seating on regen (spec §5.5), P3's
first WP, deliberately chosen first to de-risk the highest-architecture-
risk piece early.

Worker: new `ComponentMateSolver.h/.cpp` (verbatim port of `placementSolver
.ts`'s WP-1.5 math, numerically pinned against its own test cases),
`ClassifyElement`'s frame-classification logic exposed in-process (no wire
round trip) as `session::classify_shape`, and `ComponentOp.cpp::resolve_
mate_reseat` — cross-body-safe ladder resolution + epsilon-gated re-seat,
resolved entirely mid-`ExecutePlan` so it sees same-tick geometry.

**Two real defects found by RUNNING the real worker + DocumentRuntime
pipeline, invisible to the worker-only ctest suite**: (1) `mate.target`
used to ride in the wire `inputs[]`, which the worker's generic pre-flight
resolves BEFORE the op runs and blocks on failure — an unresolvable mate
published ZERO bodies, not the component at its frozen placement. Fixed by
removing mate from `wire_op_inputs` entirely; resolution now lives
entirely in `resolve_mate_reseat`. (2) `merge_outcome` unconditionally
downgraded any step with `needs_repair` to a NeedsRepair status that
discards `body_events` — correct for every OTHER op (all resolve-then-
build, early-returning before geometry exists on failure) but wrong for
mate, which is the first op that legitimately publishes geometry AND
flags a repair simultaneously. Fixed; verified safe for all existing ops
(worker ctest unchanged).

Rust: `PlanStepEvent.mate_placement` (boxed, clippy size limit) threaded
through `Scratch`/`Timeline::set_place_component_placement`/`DocumentSession
::sync_mate_placements`, wired into `commit_snapshot` beside `sync_record_
outputs` — same "derived, no-undo writeback" treatment.

Gate: worker ctest 117/117 (2 new targets) · fmt/clippy clean · `cargo test
--workspace` 100% green (two pre-existing wire.rs unit tests updated to
match the new, correct `PlaceComponent` inputs[] shape; two new real-worker
integration tests in `component_ops.rs`) · frontend untouched.

## COMPONENT LIBRARY — LIVE DELTA (2026-08-13, session 13, WP-2.6, P2 CLOSED)

Branch `OneCAD-Component-Library` (worktree), on top of the merge commit
below. This session: merged `master` in (see next paragraph), then landed
**WP-2.6**, the last P2 WP — closing P2 (WP-2.1 through WP-2.6 all done).

**Merge**: master (sibling worktree `OneCAD-Tauri`, `b7b47e2`) had 3 commits
this branch lacked — P3/P4 modeling-correctness work + a kernelbench
metamorph-variant widening, all unrelated to Component Library. Two
conflicts (`TODO.md`, `CURRENT_STATE.md`), both append-log splices,
resolved by keeping both sides' content. Post-merge, `cargo test
--workspace` is **100% green** — master's region-identity fix resolved the
two tests (`sketch_on_face`, `wire_contract`) every prior WP-2.x gate had
been citing as pre-existing failures.

**WP-2.6**: kernelbench is architecturally fillet-only (closed enums, edge-
blend-shaped selectors) — a real `OperationFamily::Component` extension is
a multi-part fork, not a config change. Chose the lighter mechanism
instead: a full cross-product ctest matrix (9 thread sizes × 3
`thread_detail` values = 27 cases) in `worker/tests/test_component_ops.cpp`,
same mechanism WP-2.5 used, now exhaustive. Found a REAL kernel limit doing
it: `modeled` thread at M2×60mm (~150 turns, tightest pitch) makes
`BRepOffsetAPI_MakePipeShell::Build()` fail — safely (`OP_FAILED`, no
crash, no partial shape). The existing `!pipe.IsDone()` guard from WP-2.5
already handles this correctly; the new test asserts that safe-refusal
contract rather than requiring success everywhere. Zero Rust/frontend
changes.

Gate: worker ctest 115/115 · Rust/frontend unaffected by WP-2.6 itself (no
source touched there); the merge's own full gate (run once, before WP-2.6):
`cargo fmt`/`clippy -D warnings` clean · `cargo test --workspace` **100%
green, no known failures** · `tsc` clean · vitest **245 files / 4159 tests**
· Playwright **404/404** (up from 396 — master's new pattern/mirror specs).

## COMPONENT LIBRARY — LIVE DELTA (2026-08-12, session 12, WP-2.5)

Branch `OneCAD-Component-Library` (worktree), on top of `6d26b57` (P0–P2.4 +
start-screen browser, already committed). This session landed **WP-2.5**:
`thread_detail` (`cosmetic`/`simplified`/`modeled`, spec §6.4) on the ISO
4762 generator, worker+Rust-table only — the configurator UI built in
WP-2.4 already renders the new domain-enum free param generically, zero
frontend logic changed.

Files touched: `worker/src/ops/ComponentOp.{h,cpp}` (new `pitch_mm` column,
`ThreadDetail` dispatch, `cut_simplified_thread`/`cut_modeled_thread`),
`worker/tests/test_component_ops.cpp` (+5 assertions), `onecad-library::
tables.rs` (`pitch_mm` column + cross-pin test widened),
`src-tauri/src/library.rs` (test fixture gained `thread_detail`),
`src/ipc/mockClient.ts` (`MOCK_LIBRARY_FIXTURE.parameters` gained
`thread_detail`).

Real defect found and fixed in passing: a helical edge built via
`BRepBuilderAPI_MakeEdge(Geom2d_Curve, Geom_Surface, ...)` has no 3D
approximation curve by default, which `BRepOffsetAPI_MakePipeShell::Build()`
needs — the missing curve raised `Standard_NullObject` with an empty
message, silently, deep inside OCCT. Fixed with `BRepLib::BuildCurves3d`.

Deferred, not built here: the StatusSection modeled-thread progress
producer (no async-task API exists in this codebase yet) and the
kernelbench table-extremes suite (already named **WP-2.6**).
`component.toml` stays test-fixture-only (Q4, still open).

Gate: worker ctest 114/114 · `cargo fmt`/`clippy -D warnings` clean ·
`cargo test -p onecad-library` 28/28 · `ONECAD_REQUIRE_WORKER=1 cargo test
--workspace` green except the same 2 pre-existing failures every prior
WP-2.x gate reconfirms (`sketch_on_face`, `wire_contract`) · `tsc` clean ·
vitest 245/4154 (unchanged) · Playwright 396/396 (unchanged, 0 regressions).

## COMPONENT LIBRARY — LIVE DELTA (2026-08-12, session 11, start-screen browser)

Branch `OneCAD-Component-Library` (worktree), uncommitted on top of session
10's delta below. User-requested (with a screenshot of the start screen's
sidebar): the library must be explorable WITHOUT opening a project. Landed
as a new `StartNavKey = "library"` entry — `StartSidebar` gained a fourth
nav row, `StartScreen.tsx` routes it to two new components:
`StartLibraryPanel.tsx` (list/search/reindex, full-width card grid) and
`ComponentDetails.tsx` (identity/category/tags/attachments/free-params side
pane). Zero Rust changes — `list_library_components`/`reindex_library`
already work with no document open (`library_root(app)` only reads
`app_data_dir()`, verified by reading `library.rs`, not assumed).

**Deliberately read-only** — reuses none of the editor `LibraryPanel`'s
placement-arm gesture (`placementController.ts` needs a live
`ViewportEngine`/`DocumentRuntime` this screen doesn't have); a selected
card's details pane says "Open a project to place this component" instead
of faking a drag with nowhere to land it.

Gate: `bunx tsc --noEmit` clean · vitest **245 files / 4154 tests** (up from
244/4148) · fmt/clippy unaffected (no Rust touched) · manual
`playwright-cli` visual verification against `?mocklibrary=1`, matches the
user's requested screen. Full Playwright not re-run (no e2e spec targets
`StartScreen`; the prior WP-2.4 396/396 run already covers every adjacent
start-screen spec).

## COMPONENT LIBRARY — LIVE DELTA (2026-08-12, session 10, WP-2.4)

Branch `OneCAD-Component-Library` (worktree), uncommitted on top of session
9's delta below. This session landed **WP-2.4**, the configurator UI —
scoped to editing an already-placed instance (pre-placement sizing + live 3D
preview cut, recorded in TODO.md, not silently dropped).

Backend: `onecad-library::index::IndexEntry` + `dto::LibraryComponentDto`
gained `parameters` (full `ParameterSpec` map) + `designation`
(`metadata.designation`) — the index/DTO previously carried names only, not
enough to render a control or know the BOM template. `LibraryComponentDto`
dropped `Eq` (`ParameterSpec`'s `toml::Value` fields only have `PartialEq`).

**Real bug found and fixed**: `tauriClient.ts::placeComponent` silently
dropped its `rotate` argument — the real backend ignored the flip gesture
entirely, masked by the mock lane honoring it. One-line fix; no automated
coverage of the real-backend half exists (flagged, this repo has no
real-worker Playwright lane).

New: `CadClient.setComponentParams` (both clients — mockClient's is a real
role=free-checked implementation, not a "not yet" stub), `src/features/
library/ComponentParametersSection.tsx` (+ its own inspector-contribution id
file, `modules/library/inspectorSectionIds.ts`), registered in
`modules/library/register.ts`. Widened: `mockClient.ts` (fixture gained
`parameters`/`designation`, `commitPlaceComponent` now stores
`featureParams`), `types.ts` (`ComponentParameterSpec`/`ComponentParamValue`
types, `LibraryComponent` gained `parameters`/`designation`).

Gate: `bunx tsc --noEmit` clean · vitest **244 files / 4148 tests** (up from
243/4142) · full Playwright **396 passed / 0 failed** (unchanged from
WP-1.7's own number) · `cargo fmt --all --check` + workspace `clippy -D
warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace
--no-fail-fast` green except the SAME two pre-existing failures · `cargo
test -p onecad-library` unaffected 28/28 · `cargo test -p onecad --lib
library::` 5/5 (up from 4).

Next: WP-2.5 (thread detail levels) or WP-2.6 (kernelbench table-extreme
cases) — independent per the plan's dependency graph.

## COMPONENT LIBRARY — LIVE DELTA (2026-08-12, session 9, WP-2.3)

Branch `OneCAD-Component-Library` (worktree), uncommitted on top of session
8's delta below. This session landed **WP-2.3**: the `SetComponentParams`
command (`src-tauri/src/library.rs::set_component_params_at` +
`#[tauri::command] set_component_params`, registered in `lib.rs`) — the
app-crate half of the role=free enforcement `onecad-core`'s
`validate_place_component` doc comment flags as structurally out of its
reach. A new `component_package_at` helper loads the full `component.toml`
(the index only carries `parameter_keys` names, not `role`); every requested
key is checked against `ParameterRole::Free` before merging, an unknown or
non-free key is rejected loud. `source.params` (the `Generator` variant's
resolved field) is set to the merged free-param map only — the worker reads
`role: free` keys by name and derives `role: table`/`computed` values from
its own table, so shipping those into `source.params` would duplicate data
nothing on the wire reads. Widened: `src-tauri/src/lib.rs` (+1
`invoke_handler` entry). Zero frontend/e2e changes — confirmed via `git
status`.

Gate: 2 new tests in `library.rs` (merge-and-reach-source-params,
reject-non-free-and-reject-unknown) both green · `cargo fmt --all --check` +
workspace `clippy -D warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test
--workspace --no-fail-fast` green except the SAME two pre-existing failures
(`sketch_on_face`, `wire_contract`) · `cargo test -p onecad-library`
unaffected, 28/28 unchanged (no crate files touched, app-crate bridge only).

Next: WP-2.4 (configurator UI — param edit → live designation). First WP
with a real frontend surface since WP-1.7; needs a
`CadClient.setComponentParams` method on both client impls.

## COMPONENT LIBRARY — LIVE DELTA (2026-08-12, session 8, WP-2.1+2.2) — P2 KICKED OFF

Branch `OneCAD-Component-Library` (worktree), uncommitted on top of session
7's delta below. P1 closed; this session landed P2's first two WPs:
**WP-2.1** (worker `ComponentOp.cpp`'s hardcoded M6×20 constants replaced by
a BOLTS-seeded `iso4762Table()`, M2–M12, keyed by `source.params.thread`;
`source.params.length` drives shank length; unknown thread/non-positive
length both fail loud) and **WP-2.2** (`onecad-library::tables` real
content — `Iso4762Table`/`Iso4762Row`, spot-check tests, new root
`THIRD_PARTY_NOTICES`).

New: `THIRD_PARTY_NOTICES`. Widened: `worker/src/ops/ComponentOp.{h,cpp}`
(table-driven generator), `worker/tests/test_component_ops.cpp` (+5 cases:
M6-explicit parity, M2, M12, unknown-thread, non-positive-length),
`src-tauri/crates/onecad-library/src/tables.rs` (real content, was a 7-line
stub). Zero frontend/e2e changes — confirmed via `git status`, not assumed.

**Data provenance, verified not assumed**: BOLTS dimension data fetched via
`gh api` from `github.com/boltsparts/BOLTS_archive` (`data/hex_socket.blt`,
class `hexsocketheadcap`). The repo's GitHub-reported license (GPL-3.0) is
a whole-repo default; the actual data file carries its own LGPL 2.1+
header — spec §6.3's "LGPL 2.1+, per-part license tracking" claim checked
true at the file level, recorded in the new `THIRD_PARTY_NOTICES`.

**Real plan-doc deviation caught before coding**: the top-level P2 sketch's
item 2 ("SetComponentParams C++ dispatch") assumed a new op-level dispatch
arm. WP-1.2 already made `SetComponentParams` an in-place
`PlaceComponentParams` edit via the generic `UpdateOperationParams` path —
no new C++ needed; this WP's table lookup is reached automatically by any
edited record once WP-2.3 builds the authoring command that constructs the
edit.

Gate: `worker/tests/test_component_ops.cpp` 12 assertions / 0 failures
(`ctest -R component_ops`) · full worker ctest **114/114** · `cargo test -p
onecad-library` **28/28** (up from 22) · `cargo fmt --all --check` +
workspace `clippy -D warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test
--workspace --no-fail-fast` green except the SAME two pre-existing
failures (`sketch_on_face`, `wire_contract`) already bisected to baseline.

Next: WP-2.3 (SetComponentParams command + role enforcement — first WP
reachable from a live gesture), then WP-2.4 (configurator UI). Full P2 WP
breakdown: `~/.claude/plans/resume-implementation-of-component-twinkling-
glade.md`.

## COMPONENT LIBRARY — LIVE DELTA (2026-08-12, session 7, WP-1.6+1.7) — P1 CLOSED

Branch `OneCAD-Component-Library` (worktree), uncommitted on top of session
6's delta below. This session landed the last two P1 work packages:
**WP-1.6** (`Slots.StatusSection`'s first real producer — `LibraryStatus
Section` in `StatusBar`, tasks-chip begin/end around `reindexLibrary()`)
and **WP-1.7** (`e2e/library-browse-place-snap.spec.ts` — WP-1.5's manual
verification converted into a repeatable Playwright gate: browse → arm →
hover-snap ghost → commit → tree update, plus Escape-cancel).

New: `src/features/library/LibraryStatusSection.tsx`,
`e2e/library-browse-place-snap.spec.ts`. Widened: `StatusBar.tsx` (+
`SlotHost`), `modules/library/register.ts` (+ `StatusSection` panel
registration), `LibraryPanel.tsx` (+ `tasksStore` begin/end),
`editorMountOrder.golden.test.ts` (rendered-slot set amended for the new
nested `StatusSection` slot — the frozen contract array itself untouched),
`LibraryPanel.test.tsx` (+1 test), `StatusBar.test.tsx` (all 9 `render()`
calls wrapped in `<PlatformProvider>` — `SlotHost` needs one, the bare
`render(<StatusBar/>)` calls predate this WP). Zero Rust/C++ changes.

**Real finding, not assumed from the plan doc**: the mock Playwright lane
cannot literally prove "save → close → reopen → still see the screw with
the library folder deleted" (spec §12) — `mockClient` has no document
persistence for a `page.reload()` to round-trip through. That invariant
stays proven at the Rust level (`component_ops.rs::place_component_
survives_save_and_a_fresh_worker_reopen`, generator-source case). The
EMBEDDED-source variant of the same invariant — the actual spec §12
differentiator — has no automated test anywhere; flagged as a residual for
P2/P3, not fixed here (WP-1.3 shipped generator-source only, per its own
recorded scope note).

Gate: `bunx tsc --noEmit` clean · vitest **243 files / 4142 tests** (up
from 243/4141) · full Playwright **396 passed / 0 failed** (up from 392 —
the new spec's 4) · `cargo fmt --all --check` + workspace `clippy -D
warnings` clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace
--no-fail-fast` green except the SAME two pre-existing failures already
bisected to baseline (`sketch_on_face`, `wire_contract`, both `fetch body
mesh` panics) · `cargo test -p onecad-library` 22/22 unchanged · worker
binary untouched (no C++ changes, no rebuild needed).

**P1 is now closed** per spec §10's gate language (kernel CTest + Rust
tests + a Playwright spec for browse→place→snap→[save→reopen, scope-noted
above]). Next: P2 (parametric fasteners) — needs a follow-up plan; open
question Q3 (BOLTS ingestion tooling ownership) gates starting it.

## COMPONENT LIBRARY — LIVE DELTA (2026-08-12, session 6, WP-1.5)

Branch `OneCAD-Component-Library` (worktree), uncommitted on top of session 5's
delta below (P0 + P1.1–1.4). This session landed **WP-1.5, the placement
gesture**: hover → classify → attachment match → snap ghost via the real
`PreviewOp` lane (both clients) → flip (`A`) → commit via `CadClient.
placeComponent` (now carrying `rotate`, not just `translate`). New:
`src/modules/library/placementController.ts` (the gesture — a module-level
singleton reaching `ViewportEngine` via `engineBridge`, exactly like
`ModelToolController`, NOT a `ViewportContribution` — see TODO.md's WP-1.5
entry for why `configurePicking`'s single hardwired pick seat forced that
choice) + `placementSolver.ts` (pure candidate-transform math, 16 tests).
Widened: `onecad-library::IndexEntry`/`LibraryComponentDto`/`LibraryComponent`
(attachments + generator identity), `CommandApiService` (rotate on
`placeComponent`, plus a generic preview-lane pass-through per ADR-0002),
`ipc/previewOps.ts`/`OpType`/`OperationOp` (a `PlaceComponent` ghost-preview
arm), `mockClient`/`mockMeshes.ts` (fixture behind `?mocklibrary=1` +
`placeComponentGhostMesh` fabrication, mirroring the M6 SHCS the worker
already hardcodes). Zero C++ changes — the real-worker ghost needed none,
`execute_place_component` (WP-1.2) already handles it.

Verified end to end via `playwright-cli` against
`/?vpdebug&vpdemo&mocklibrary=1`: armed card → hover the demo body → correctly
oriented ghost (both flip states screenshotted) → click commits → named body
in the tree, `bodiesChildren` 1→2 → Escape cancels cleanly. Gate: tsc clean,
vitest 243/4141, fmt/clippy clean, `cargo test --workspace --no-fail-fast`
green except the SAME two pre-existing failures already bisected to baseline
below, ctest 114/114 (worker untouched). Full detail: TODO.md § COMPONENT-
LIBRARY WP-1.5.

Scope cuts recorded, not discovered late: no free-space ghost follow (spec
step 6's Move-tool fallback has no integration point yet), no auto-size (P1
has one hardcoded generator, nothing to size to — P2), no `mate` persistence
(worker's `ComponentOp` doesn't consume `mate` on regen yet — recording one
now would be inert-at-best data; spec §5.5 stays P3).

Next: WP-1.6 (StatusSection + tasks-chip real producer), then WP-1.7 (e2e).

## COMPONENT LIBRARY — LIVE DELTA (2026-08-12, session 5, P0–P1.4)

Branch `OneCAD-Component-Library` (worktree), 30 files changed vs `5036597` (+1970/−13, `diff --stat`), nothing staged, nothing committed. New: `src-tauri/crates/onecad-library/` (whole crate), `src-tauri/src/library.rs`, `src-tauri/tests/{classify_latency,component_ops}.rs`, `worker/src/ops/ComponentOp.{h,cpp}`, `worker/src/session/ClassifyElement.{h,cpp}`, `worker/tests/test_component_ops.cpp`, `src/modules/library/`, `src/features/library/`, `src/stores/sidebarTabStore.ts`, `src/features/shell/SidebarTabHeader.tsx`.

Implements P0 (both WPs) and P1.1–P1.4 of `~/.claude/plans/do-thorough-analysis-of-abstract-thunder.md` against `TheComponentLibrary/onecad-component-library-spec.md`. `KnownOperation` gained `PlaceComponent`/`DetachComponent` (full mirror-site set: `record.rs`, `edit/session.rs` validators wired into both `add_operation`/`update_operation_params`, `wire.rs::wire_op_inputs` + its pinned slot-order test, `document_runtime.rs::element_ref_input`); `DetachComponent` is a sanctioned in-place op-type swap onto `PlaceComponent` records (`op_type_edit_allowed`, one-directional). New read-only `ClassifyElement` kernel verb (SCHEMA §7.5) backs `ModelingServices.GeometryQuery`, the codebase's first real registration of that previously-declared-only service; `ModelingServices.CommandApi` also got its first real registration (`placeComponent`/`detachComponent`). New `onecad-library` crate (package/blob/index/resolve/registry, embedded-only in P1, `onecad-core`-independent by design per plan Q1's default). Frontend: `src/modules/library/` UI module + `LibraryPanel` sharing `Slots.ShellLeft` with `ModelTreePanel` via a new shared-store tab pattern (`sidebarTabStore` + `SidebarTabHeader`) — no platform/slot contract change. `shellContract.ts` amended (recorded, dated).

Gates run this session, all green except two PRE-EXISTING failures verified by `git stash` bisect against clean `5036597` (identical failure at baseline, not this work): `sketch_on_face.rs::a_line_across_the_projected_rect_yields_two_extrudable_regions`, `wire_contract.rs::nested_inner_disk_parity_and_reopen_stability` (both `fetch body mesh` panics, unrelated to Component Library). `classify_latency.rs` go/no-go: p95 = 0.16 ms, **GO** — WP-1.5's live-hover snap gesture is cleared, no click-to-classify fallback needed. `cargo test -p onecad-library` 22/22. `test_component_ops.cpp` 7/7 (ctest). `component_ops.rs` 3/3 worker-backed, including full place→detach round trip. Vitest: `register.test.ts` (+2, both services proven to resolve inside a mounted scope), `LibraryPanel.test.tsx` (6, loading/empty/error/populated/search/reindex), `editorMountOrder.golden.test.ts` (3, LibraryPanel mount-order pinned). Manual `playwright-cli` pass on the panel: tab switch, empty state, reindex button, search filter — screenshotted, no console errors.

Real bug caught and fixed: `ComponentSourceRef::Generator`'s `generator_id`/`generator_version` fields serialized snake_case despite the enum's `rename_all="camelCase")]` — that attribute doesn't cascade into internally-tagged struct-variant fields (each needs its own `#[serde(rename=...)]`). Pinned by `place_component_source_fields_are_camel_case`.

Deviation from the plan, recorded: `SetComponentParams`/`ReplaceComponent` (WP-1.2) turned out to be in-place param edits needing no new `KnownOperation` variant — only `DetachComponent` needed one. Caught mid-implementation, not pre-planned.

Not started: WP-1.5 (snap solver + `PreviewOp` drag ghost — the largest remaining P1 WP, first one to touch the viewport engine), WP-1.6 (StatusSection/tasks-chip), WP-1.7 (e2e), all of P2/P3/P4. Plan's open questions Q1 (crate boundary) and Q6 (library-service-vs-CommandApi routing) resolved by implementation; Q2 (service-build scope) moot, done; Q3 (BOLTS ingestion ownership), Q4 (template content sourcing), Q5 (`document` source nested-replay) remain fully open — none block WP-1.5.

Next action: WP-1.5. See `HANDOFF.md` new top entry for full resume detail.
# Current State

Last verified: 2026-08-16 — autosave/crash-recovery hardening landed (DI-1/DI-2/DI-3 closed + four further defects); manual Tauri gate owed

## NOW — AUTOSAVE / CRASH-RECOVERY HARDENING (session 10)

- **Question:** why does "Unsaved changes recovered" not always recover the changes?
  Full findings + gate table in `TODO.md` § AUTOSAVE HARDENING.
- **The write path was never the problem.** The container write is atomic, the persistence lane is
  sound, the round-trip test was real. The defects were in WHEN the autosave fires, HOW an offer is
  discovered, and WHAT the restored document looks like.
- **Symptom 1, work missing:** the debounce had no ceiling, so sustained modelling starved the
  writer indefinitely — the user who never pauses got no autosave at all.
  `AUTOSAVE_MAX_AGE` (120 s, built from the previously-dead `AUTOSAVE_INTERVAL_SECS`) is now an
  absolute deadline, plus a throttled flush on window blur.
- **Symptom 2, restored document looks wrong:** `mark_recovered` restored the path but not the
  TITLE, and recovery reads `<documentId>.onecad` — so the document came up named with a raw UUID.
  `SessionMarker` gains `title`; the card now shows a time, not just a date.
- **Symptom 3, offers for untouched documents:** the autosave fired on a mutation TICK and never
  read `dirty`, and a published regen ticks — which `open_document` schedules. Merely opening a
  project armed a crash marker. Now gated on `is_dirty()`.
- **Biggest find, not in the audit:** opening a project from Recent silently destroyed its crash
  autosave (same `documentId`, so the first autosave overwrote the container and re-stamped the
  marker), and the banner sits above a fully clickable recents list. `open_document` now refuses
  with `RecoveryPending` until the user chooses; the shadowed card is badged.
- **DI-1 closed harder than scoped:** discovery is keyed to the CONTAINER with the marker as owner
  evidence, so a lost or consumed marker costs a label rather than the document. The audit's
  characterization test is inverted into a fix pin.
- **Gates:** cargo 1283/0 vs real worker · vitest 4942/0 · new `e2e/recovery.spec.ts` 14/14 on both
  browsers (a lane with zero prior coverage) · full `bun run e2e` 451/11, with all 11 failures
  proved pre-existing against a clean worktree at HEAD (they belong to the concurrently-committed
  sketch-snap work) · fmt/clippy/tsc/hex clean · four fixes mutation-proved.
  **Owed:** the manual Tauri gate (five steps, listed in `TODO.md`).
- **Deferred, documented:** pid-based liveness never offers recovery on Windows and breaks under
  pid reuse (fix: per-session lock file) · the frontend still never subscribes to `events::AUTOSAVE`
  · marker/recents writes are not fsynced · `recents.json` corruption is silent data loss.

## Previous — DATA-INTEGRITY AUDIT (session 9)

- **Question:** can OneCAD lose or corrupt a user's document? Eight probes over the persistence
  lane; full findings table in `TODO.md` § DATA-INTEGRITY AUDIT.
- **Safe, with evidence:** crash recovery exists (30 s debounced autosave + pid marker + one
  persistence lane) · the container write is atomic (temp → fsync → rename → parent fsync, with a
  crash-simulation test) · the import-blob carrier is insert-only and the undo stack is memory-only,
  so no unreplayable-record window · a save snapshots under the runtime lock · unknown module state
  round-trips verbatim · every `EditCommand` has a real inverse.
- **Five findings.** DI-1 (recovered document unprotected against the next crash), DI-2
  (`recover_document` never ticks the autosave loop) and DI-3 (`promote_selection` /
  `prepare_edge_op` persist state with no tick and no dirty flag) are **CLOSED** by the session-10
  hardening pass above. Still open: DI-4 authored face colours reopen as data but stop being
  paintable · DI-5 STEP export drops XCAF names/colours.
- **Two probes committed as executable evidence:** `src-tauri/tests/face_color_reopen.rs` (real
  worker, save → fresh worker → reopen; mutation-proved) and
  `io::recovery::tests::an_autosave_whose_marker_was_consumed_is_not_offered`.
- **Ledger corrections:** VF-M6 was fixed 2026-08-08 and never ticked; the W5 seam is latent, not
  live; MC-R8 is closed (`19088d0`) and MC-R9 was missing from the residual register entirely. The
  register now carries an explicit `status` field instead of closure buried in prose.
- **Blockers:** none. Which findings get fixed is a product call, not a defect queue.

## Previous — MODELING UX UNIFICATION (sessions 8–9)

- **Branch:** `master`, **in sync with `origin/master` at `4ac8565`** (session 8's "2 ahead,
  nothing pushed" is stale: `cf6273d`/`dc4bd5e` plus `9559b8f`, `19088d0`, `f9df6b7`, `4ac8565`
  were all pushed by 2026-08-14 09:22, per `git reflog show origin/master`). Working tree
  **dirty**: 43 modified files + 5 new test files + the untracked `UX/` source documents.
  **+2513 / −385.** The U0–U7 program itself is still uncommitted.
- **What landed:** all eight packages of the delta program in
  `~/.claude/plans/bright-munching-oasis.md` — U0 (red evidence + two new frozen-contract
  columns), U1 (result truth), U2 (one confirmation grammar), U3 (live numeric contract),
  U4 (OperationHUD + result summaries), U5 (gizmo arcs + collision-safe HUD), U6 (tool entry,
  roles, ranges, unique body labels), U7 (repair + sketch truth). Per-package detail, evidence
  and every deliberate omission are recorded in `TODO.md` § MODELING UX UNIFICATION.
- **Gates measured this session:**
  - `bunx tsc --noEmit` clean · `bun run build` clean
  - `bun run test` **255 files / 4250 tests, all green** (baseline was 250 / 4182)
  - worker Release build + stage · `ctest --test-dir worker/build` **119/119**
  - `cargo fmt --all --check` clean · `cargo clippy --workspace --all-targets -D warnings` clean
  - `ONECAD_WORKER_PATH=… ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast`
    **810 passed / 0 failed over 69 targets**
  - coverage + contract verifiers pass · hex gate empty
  - Playwright, retries 0: **chromium 200/200 · webkit 200/200** at the U4, U6 and U7 gates
- **U5's browser lanes: RUN AND GREEN (session 9, 2026-08-14).** `chromium 200/200` in 14.2 min
  (`E2E_PORT=4191`) and `webkit 200/200` in 8.6 min (`E2E_PORT=4193`), retries 0, one lane at a
  time. That was the program's last owed gate — U5 changed gizmo geometry, screen scale and chip
  anchoring, and only the browser can check those. No failures, so no triage; MC-R8/MC-R9 did not
  reappear.
- **Blockers:** none. Every gate this program owes is now measured green.

### Deliberate omissions (each with its reason recorded in TODO.md)

- **D6's discriminated `ToolEditorDescriptor` union** — deferred a second time. No red test forces
  it, it is pure internal type-safety, and the store gained four fields this session, so a union
  rewrite would land on a moving target. The behavioural half of D6 shipped in U2/U3/U4.
- **Shell's thickness drag handle** — a handle needs a direction and the shell arm has none
  (`EntityRef.anchor` carries no normal). Guessing one is worse than omitting it. Needs a face
  normal on the prepare response: a wire change with its own gate.
- **U8 typed face/datum/axis references** — never in scope for this program; still queued.
- **OffsetFace's shared publication gate has no end-to-end negative control** and none is claimed;
  its own postconditions already cover every Tier A check but the audit-error path.

### Residuals opened/carried

- **MC-R8** — **CLOSED**, root-caused and fixed in `19088d0` (the debounced auto-fit moved the
  camera under the boolean spec's unsettled screen-point probe; one `waitForCameraSettled` at the
  top of `findBodyScreenPoint`). Not a projection-push race, which was the earlier guess.
- **MC-R9** (NEW) — `e2e/revolve-commit.spec.ts:111` failed in ONE loaded full chromium run
  (19 min vs the usual 14) and passes in isolation and on the clean re-run. Same signature class.
- Manual Tauri smoke for this program has **not** been run.

## Previous — modeling-correctness roadmap (session 7)

Last verified: 2026-08-13 23:10 — ladder run, tranche committed, MC-R7 closed on both browser lanes

- **Branch:** `master`, 2 commits ahead of `origin/master` (`cf6273d` code +
  `dc4bd5e` docs), **dirty** (the MC-R7 fix + these doc updates). Nothing pushed.
- **MC-R7 CLOSED.** Commit `c7df7c8` removed the click-away commit on purpose (D2)
  and the frozen contract pins `clickAwayPolicy: "cancel"`; the e2e spec and the
  arm hint text were never updated, so both were asserting/promising a gesture that
  no longer exists. `e2e/extrude-commit-gesture.spec.ts` now asserts click-away does
  NOT commit and leaves the tool armed;
  `src/tools/modelTools/ModelToolController.ts` `armHintFor` no longer promises it.
- **Browser lanes, retries 0, measured 2026-08-13:** chromium **200/200** (rerun),
  webkit **200/200**. The first chromium run of the pair was 199/1 on
  `e2e/boolean-preview.spec.ts:356` (Intersect chip): `previewOwner` never became
  `"boolean"` inside the 20 s lane poll, so the boolean lane never opened. That spec
  is 9/9 in isolation with `--repeat-each=3` and 200/200 on the immediate full rerun,
  and the signature matches the boolean-preview projection-push race already bisected
  to before the Platform refactor. Recorded as **MC-R8**, unclassified
  nondeterminism — do not add a retry to hide it.
- **Use `E2E_PORT`** for any lane run: a concurrent session in
  `OneCAD-Component-Library` holds 4177, and a stray node process was found holding
  4187. The runs above used 4191 / 4193.
- **Blockers:** none.

- **Baseline:** `master` at `9933689`, one commit ahead of `origin/master`; no commit/push/pull
  authorized. All status below distinguishes implementation from gates.
- **Implemented now:** strict plan-stream validation before `AcceptPrepared`; release-only bundled
  worker resolution plus embedded SHA/protocol/OCCT manifest verification; prepared meshes bounded
  by advertised transport limits; strict ResolveRefs revision/body provenance; Modified+Generated
  history union; import exact-tie refusal; Region Identity V3 new authoring with frozen V1/V2;
  structured publication evidence/timings; Tier-A modeling-input preflight; explicit
  `(operation,mode,supportStatus,uiExposure)` coverage checks; Extrude/Revolve Intersect hidden;
  corpus 9/9; case-v2 Boolean foundation; feature-gated real-Tauri composition lane.
- **Full gate ladder, run end to end 2026-08-13 (local mac, unsandboxed):** worker Release build +
  restaged sidecar/manifest, `ctest` **119/119** · `cargo fmt`/`clippy -D warnings` clean ·
  `ONECAD_REQUIRE_WORKER=1 cargo test --workspace --no-fail-fast` **767/767 over 60 targets** ·
  real-worker corpus **9 of 9, zero skips** · `cargo check --features tauri-e2e` clean ·
  `npx tsc --noEmit` 0 · `bun run build` clean · Vitest **250 files / 4182 tests** ·
  coverage + contract verifiers pass, negative controls **15/15** · hex gate empty ·
  kernelbench T0 both backends **136/136**, 0 regressions, semantics baseline unmoved ·
  Playwright retries 0 **chromium 199/1**, **webkit 199/1** — both single failures were MC-R7,
  now fixed and re-measured at **chromium 200/200 · webkit 200/200** (see the top block).
- **Three defects the ladder caught (fixed this session):** the worker stub emitted an `autoBind`
  ResolveRefs resolution with no `bodyId`, which SCHEMA §7.5 allows only on a non-promotable
  missing-body `needsRepair`; `topology_rebind` fed a real V3 region id into a version-less
  profile and the worker correctly refused it; `src-tauri/src/tauri_e2e.rs` had never been
  compiled and did not (E0597 in `composition_status`).
- **Former pre-existing failure, now CLOSED:** e2e click-away extrude commit, both browsers,
  deterministic, reproduced at `9933689`. Residual MC-R7 — it was stale evidence, not a product
  defect; the spec and the arm hint were fixed and both lanes now run 200/200.
- **Open:** P2 measured ceilings/performance; P3 full semantic/overhead closure and Pattern budget;
  real-Tauri/WDIO composition run (compiles, never executed); Chamfer and Boolean campaign breadth;
  kernelbench m1; Linux/macOS/Windows aggregate release enforcement and 20-run stability sample.
  See `docs/qa/modeling-residuals-v1.json`.

The per-gate detail for every wave, newest first, follows.

## ROADMAP C1 + C2 — EVIDENCE THAT CAN FAIL (2026-08-13) — GATE PASSED

Two plan items whose whole subject is whether a claim can be falsified. Full detail in `TODO.md`
§ ROADMAP C1 and § ROADMAP C2.

**C1 — the coverage manifest.** Sixteen cited test paths did not exist and eleven rows named a CI
job (`macos-full`) that is in no workflow, all under a green verifier that never opened a file.
Every row now cites something real; `ciJob` became a per-lane list resolved against the workflows;
four rows state a measured limit instead of an overclaim (single-face Shell, `Radius`-only
OffsetFace in Rust, an uncommitted countersink in the browser, C++-only partial sweep); one
"overclaim" was disproved (MirrorBody no-fuse IS covered) and one under-claim corrected (Boolean
Intersect). The verifier stats every path, resolves every job, runs WP4.5's five registry
cross-checks with a found-something guard on each scan, and is backed by twelve negative controls
— including the plan's acceptance test, renaming a cited spec.

**C2 — the corpus.** It ran 1 of 9 not because the interpreter was thin but because only case `a`
carried complete geometry: `b` referenced four sketches it never authored, `i`'s entities were
prose. With the user's decision, the cases were ENRICHED — every footprint read back out of a
frozen assertion (`3750 = 4000 − 5·5·10`), each with its own `entitiesProvenance` — and the
executor now runs `a`, `b` and `i`. Case `a` asserts its whole expected block (regions, body
events, solids, volume, faceCount) rather than one volume; classification comes from the coverage
manifest so the two artifacts cannot drift; structure and provenance are checked for all nine on
every machine; and a stale unsupported reason is an error.

One divergence is recorded rather than smoothed over: the square-with-hole sketch detects TWO
regions on the new stack (annulus + inner square) where the C++ LoopDetector reported one face with
an inner loop. That is the planar-cell model `wire_contract.rs` already pins, so the case carries a
`newStack` block citing it and keeps the frozen number visible.

- **Changed:** `docs/qa/modeling-operation-coverage.json` · `scripts/verify-modeling-coverage.mjs` ·
  `scripts/tests/verify-modeling-coverage.test.sh` · `src-tauri/tests/corpus_executor.rs` ·
  `corpus/cases/{b_extrude_throughall_symmetric_twodir,i_multiregion_loop_detection}.json`.
- **Gates:** real-worker workspace **1082 / 0** · fmt · clippy · both verifiers · controls 12/12 ·
  every corpus file parses; case `a` byte-identical.
- **Next:** C3 (WP4.6 has zero tests) and C4 (deepen the thin verticals — where C1's four measured
  limits get closed).

## ROADMAP A4 · A5 · A6 — THE REST OF TRACK A (2026-08-13) — GATE PASSED

Track A is complete. Full per-item detail is in `TODO.md` § ROADMAP A4 · A5 · A6; the parts worth
carrying:

**A4 — the Revolve body-edge axis is recorded as UI-hidden, and now provable.** The contract row
said `uiExposure:"exposed"` while no code path could author one. The product call is HIDDEN: kernel
and core keep the capability, the UI keeps authoring only `sketchLine`. All four WP1.5 tests landed
— curved-edge and cross-body refusals in `test_wp6_ops.cpp`, promote→revolve→upstream-edit→reopen
and a fillet-CONSUMED axis edge ⇒ NeedsRepair in `revolve_ops.rs`. Removing the partition-ownership
gate makes the revolve silently swap to the other body's edge; that is what the test now catches.
Measured aside: a +20% upstream growth of the axis edge rebinds, +100% is NeedsRepair, and a FILLET
ref answers identically — the shared ladder's descriptor-magnitude policy, not an axis quirk.

**A5 — WP0.7 was a live defect, not missing evidence.** Candidate ownership was per-session but
`previewFailure` was one field: a secondary region's refusal outlived its own recovery and blocked
every commit behind a stale error hint. Failure now lives on `ToolPreviewSession`; the lane's
failure is the union. The mandated test (both delivery orders, secondary fail-and-RECOVER) was red
before the fix. WP0.8 gained its two untested re-arm paths (rejected commit, resolved regen failure)
and the "permits a successful second Apply" clause, committing the re-armed session rather than the
consumed one.

**A6 — one fixture needed the contract to become true first.** SCHEMA §7.5 has always required every
`ResolveRefs` resolution to echo `{snapshotId, revision, refId, bodyId}`; neither worker emitted any
of it, Rust manufactured all three app-side and validated nothing, so a resolution computed on an
older snapshot was cached under a freshly minted key — the silent wrong bind the rule exists to
prevent. Worker + stub now echo it, `wire::validate_resolve_refs_result` fails closed, and the DTO
keeps Rust's own `revision` by decision D4 (the repair store keys on it) while taking the SNAPSHOT
from the echo. Two new canonical fixtures, and fixture discovery is now enumerated in both lanes —
the hardcoded lists were why `boolean_empty_refusal.ndjson` ran in only one of them.

- **Changed:** `worker/src/session/ElementIdentity.cpp` · `worker/tests/{test_wp6_ops.cpp,CMakeLists.txt,interop/check_interop.sh}` ·
  `protocol/{SCHEMA.md,fixtures/README.md,fixtures/bind_element_ids.ndjson}` + two new fixtures ·
  `src-tauri/crates/onecad-core/src/regen/engine.rs` · `src-tauri/crates/onecad-protocol/src/messages.rs` ·
  `src-tauri/crates/onecad-worker-stub/src/main.rs` · `src-tauri/src/{api/mod.rs,dto.rs,worker/wire.rs,worker/manager.rs}` ·
  `src-tauri/tests/revolve_ops.rs` · `src/tools/modelTools/ModelToolController.ts` + three test files ·
  `docs/qa/modeling-operation-contracts.json`.
- **Gates:** ctest **119/119** · fmt · clippy · real-worker workspace **1082 / 0** · tsc · build ·
  vitest **249 files / 4173 tests** · both verifiers · hex 0 · targeted Playwright 20/20.
- **Next:** C1 — make the coverage manifest true, then give `verify-modeling-coverage.mjs` teeth.

## ROADMAP A3 — STABLE DIAGNOSTIC CODES FOR THE P0 REFUSALS (2026-08-13) — COMMITTED `4b62965`

Phase 0 shipped two new refusals — zero-solid Boolean and Draft — that returned a
bare `OP_FAILED` with the reason only in the message, forcing message-text routing
the diagnostics contract forbids. Both now carry a stable per-defect diagnostic
code, `stage`, and bounded evidence, following the
`EDGE_OP_TANGENT_CLOSURE_CHANGED` precedent: the §8 top-level code is unchanged,
so this is additive on the wire.

Four `EXTRUDE_DRAFT_*` codes plus `BOOLEAN_EMPTY_RESULT`. The codes are asserted to
DISCRIMINATE (no-planar-wall vs 89° self-intersecting taper produce different
codes), both lanes are asserted to agree (the preview refusal carries the same code
as the commit), and `boolean_empty_refusal.ndjson` — one of the fixtures the Phase 3
review flagged as never extended — now asserts the diagnostic and was verified to
fail on a wrong code.

- **Changed:** `worker/src/ops/{ExtrudeOp,BooleanOp}.cpp` · `worker/tests/{test_extrude_draft,test_boolean_empty}.cpp` ·
  `src-tauri/tests/preview_extrude_draft.rs` · `protocol/{SCHEMA.md,fixtures/boolean_empty_refusal.ndjson}`.
- **Gates:** ctest **117/117** · fmt · clippy · real-worker workspace **1078 / 0**.
- **Next:** A4 — the four required Revolve body-edge-axis tests, then reconcile the
  contract row claiming `uiExposure:"exposed"` against `src/ipc/types.ts:843`.

## ROADMAP A2 — DRAFT APPLIED-OR-REFUSED (2026-08-13) — COMMITTED `6b08e27`

WP0.6. The implementation was already correct; the evidence was missing. No test
pinned any of the three refusal strings, and the work package's first mandated
task — the circular-profile red probe — had never been run.

**The probe ran and the answer is refusal**, so risk R-10 is closed as not-present
with evidence rather than by inspection: a circular profile refuses with
`Extrude draft refused: no eligible planar side faces`, in the preview lane as
well as on commit. New ctest `extrude_draft` pins the closed-form frustum for both
signs, the near-limit refusal, the sub-epsilon no-op and determinism; new
real-worker `preview_extrude_draft.rs` pins preview == closed form == commit
(688.801) and a head left byte-identical.

- **Findings recorded, not fixed:** `Arc` entities still reach the BRep as polylines
  while `Circle` stays analytic — the slot probe measures 28 faces and a 24-gon cap,
  direct evidence for the Phase 2 residual. And the draft refusal already carries a
  structured `Diagnostic{stage:"build"}`, so A3's remaining work there is the stable
  per-defect code, not the envelope.
- **Gates:** ctest **117/117** · fmt · clippy · real-worker workspace **1078 / 0**.
- **Next:** A3, stable diagnostic codes for the zero-solid Boolean and Draft refusals.

## ROADMAP A1 — SHARED OPERATION-RESULT CLASSIFIER (2026-08-13) — COMMITTED `772b3d2`

WP0.3 finished. It was recorded complete but only the transport half had landed:
`needsRepair` was declared and never produced, no shared helper existed, and every
consumer still inferred success from body counts — a live R-04 defect, not missing
evidence.

`regen-finished` gained `repairSteps` so a commit can tell "my op could not resolve
its refs" from "my op published"; it rides that payload because the sibling
`needs-repair` event is emitted after it. `src/ipc/regenOutcome.ts` is now the one
place a result becomes a verdict, and a NeedsRepair record is never rolled back —
rolling back the record repair operates on would delete what the user is fixing.

- **Changed:** `src-tauri/src/{dto.rs,api/mod.rs}` · `src/ipc/{types.ts,tauriClient.ts,mockClient.ts}` ·
  `src/ipc/regenOutcome.ts` (new) + its test · `src/tools/modelTools/ModelToolController.ts` ·
  `src/features/inspector/historyActions.ts` · two new terminal-table tests.
- **Gates:** tsc clean · vitest **247 files / 4161 tests** (from 244/4124) · build clean ·
  fmt · clippy · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1076 / 0**.
- **Both new tables proved non-vacuous by mutation** — reverting `commitPattern` to the
  old check reproduces R-04: a `needsRepair` result printed the success hint.
- **Next:** A2, pin the Draft refusals and run the mandated circular-profile red probe.

## KERNELBENCH M4 — RECIPE-AGNOSTIC VALIDATORS (2026-08-13) — COMMITTED `8bd0fdb`

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
