# Live Implementation Delta

Reviewed: 2026-08-13  
Implementation state: uncommitted worktree after `c7df7c8`  
OCCT: 8.0.1, fingerprint `0a6a1dce34181289`

## Since the 2026-08-11 review

- **P3 is complete** (`f36c960`): 35 machine-readable contract rows over 16
  operations (`docs/qa/modeling-operation-contracts.json`), Transform through
  the Tier A validator, ImportStep policy row, UI mode disposition recorded,
  and `scripts/verify-modeling-contracts.mjs` enforcing all of it.
- **P4 remains partial.** C1/C2 landed (`1c76c41`, `39f5839`): Boolean Intersect
  vertical across C++/Rust/frontend/Playwright, Mirror and Linear/Circular
  Pattern Playwright flows, critical mode closure tests, and a real-worker corpus
  executor with zero unclassified corpus files. C3/C4/C5/C6, Pattern
  compatibility, and corpus-trust hardening remain before P4 can close.
- **Phase 5 WP5.1 landed** (`b7b47e2`): mirror, far-origin translation,
  edge-order permutation and contour seed execute in the runner.
- **Phase 5 WP5.1 residual (M3.5) closed, uncommitted** (2026-08-13):
  `uniformScale` and `parameterEpsilon` are back in `fillet/matrix:m1` behind a
  per-variant RELATION (equivalence / similarity / continuity) rather than a
  single equality gate; m1 is 336 records with `gatingFailures: 0`. It found and
  fixed a real defect — the semantic validators measured the blend against the
  case's declared radius rather than the radius the operation ran with. Detail in
  `TODO.md` § M3.5 and `CURRENT_STATE.md`.
- **Phase 5 WP5.2 (M4) done, uncommitted** (2026-08-13): `supportTangency`,
  `crossSectionProfile`, `manifold`, `noSelfIntersection`, `microTopology` and
  `toleranceGrowth` implemented and required on every m1 case including the
  curved pairs. The blend now comes from the fillet builder's history rather than
  from surface type, which is what makes a curved support stop being a special
  case. Detail in `TODO.md` § M4.
- **TRACK A of the completion plan is complete** (2026-08-13, `772b3d2` · `6b08e27` · `4b62965`
  plus the A4/A5/A6 gate). Track A is the unbilled remainder of Phases 0–4: the shared
  operation-result classifier (WP0.3, risk R-04), the Draft applied-or-refused evidence and its
  mandated red probe (WP0.6, R-10 closed as not-present), stable diagnostic codes for the two new P0
  refusals, the Revolve body-edge-axis contract (WP1.5 — recorded UI-HIDDEN with all four tests),
  the WP0.7/WP0.8 tests, and the two missing cross-track fixtures.
- **Three of those items were live defects rather than missing evidence**, which is the point of
  running the mandated tests: `needsRepair` was declared and never produced; exact-preview failure
  was tracked globally, so a recovered secondary region still blocked every commit; and the SCHEMA
  §7.5 `ResolveRefs` snapshot echo was normative but implemented by neither worker, with Rust
  manufacturing the values it then cached candidates under.
- **Plan items C1 and C2 are complete** (2026-08-13). P4's coverage manifest was false in sixteen
  places and named a CI job that exists in no workflow; it is now true, and its verifier stats every
  cited path, resolves every job against the workflows, and runs WP4.5's five registry
  cross-checks. WP4.4's corpus runs three cases instead of one — the blocker was that only case `a`
  carried complete geometry, so `b` and `i` were enriched with executable entities derived from
  their own frozen numbers — and classification now comes from the manifest, so the two artifacts
  cannot disagree.
- **Still owed from P2:** the manual Tauri smoke. Unchanged (plan Track D replaces it with an
  automated Tauri lane).
- **Next in roadmap order:** finish C3/C4/C5/C6, Pattern compatibility, and
  corpus-trust hardening before Phase 5 WP5.3's Boolean campaign. C5 performance
  caps await measured Tier A/B and cumulative Pattern cost; C6 needs a full
  Chromium and WebKit zero-retry pass from persistent Vite with retained failure artifacts.

The sections below are the 2026-08-11 review, kept for its evidence trail.

---

Reviewed: 2026-08-11  
Implementation state: current uncommitted worktree after `055d31f7d8edcb106a881590b9b2c5a9d1e37982` (`055d31f`)

## Status language

"Implemented" means code exists at the reviewed commit. "Gate passed" means a
named command completed against that state. They are separate claims.

## Gate evidence

- `ctest --test-dir worker/build --output-on-failure`: 113/113 passed.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, and real-worker `cargo test --workspace`: passed.
- `npx tsc --noEmit`, `bun run build`, and targeted Pattern/IPC Vitest: 89 tests
  passed. The preceding full Vitest baseline remains 241 files / 4115 tests.
- Zero-retry Playwright: Chromium 196/196 and WebKit 196/196 passed. The
  spawned-Vite Chromium failure was infrastructure (`ERR_CONNECTION_REFUSED`);
  WebKit exposed a helper retry leaving its pointer down, fixed with `finally`
  cleanup and then verified by a full rerun.
- Manual Tauri smoke remains unrun. T0 was started with the freshly built
  runner, but emitted no completed `results.jsonl`; it is not a pass.

## Phase delta

- P2 is implemented. `CurveFragment` retains analytic entity kind, unwrapped
  parameter interval, traversal orientation, and shared endpoints. OCCT
  `Geom2dAPI_InterCurveCurve` refines accepted intersections; tangencies
  collapse, overlapping/coincident supports refuse, and periodic seams use
  unwrapped intervals. Face construction creates trimmed analytic
  Line/Circle/Arc/Ellipse BRep edges with shared vertices and refuses an
  unconnected analytic wire.
- P2 profile identity is versioned. `SketchRegions` emits
  `regionIdentityVersion:2`; new profile refs persist it through Rust and UI
  preview/commit paths. V2 requires one exact canonical region id. Absent
  versions replay V1; ambiguous legacy aliases refuse rather than selecting a
  first region.
- P2 regression coverage includes overlapping circles, line/circle,
  crossing arcs, rotated ellipse/chord, tangency, coincident refusal,
  cancellation, exact curve census, and analytic area conservation.
- P3 is partially implemented. Shared Tier A/B publication evidence gates
  Extrude/Revolve, Boolean, Fillet/Chamfer, Shell, Hole, fused Pattern, and
  Mirror. Pattern, Mirror, Revolve, Transform, Shell, and Boolean use strict
  worker readers; Pattern and Transform cap work at 128 and poll cancellation.
- Pattern V2 is normative. Absent `resultPolicyVersion` remains frozen V1.
  V2 non-fused Pattern preserves the source as instance zero and creates only
  `body_<opId>:<k>` for transformed instance `k+1`; it emits no source lifecycle
  event. V2 fused Pattern modifies the source in place and rejects disconnected
  results with `PATTERN_DISJOINT_RESULT`. Children inherit body visibility/color,
  not face identity/colors. Legacy re-edit preserves absence; V2 surviving child
  IDs persist across count edits, suppression, undo, save, and reopen.
- Direct ExecutePlan and real-worker tests cover V2 lifecycle, source downstream
  validity, metadata inheritance, tail-only child removal, suppression, undo, and
  reopen. Remaining P3 work: machine-readable per-operation contract rows,
  Transform/import policy rows, and explicit UI mode disposition.
- P4 bootstrap is implemented: `docs/qa/modeling-operation-coverage.json`
  classifies the frozen corpus and `scripts/verify-modeling-coverage.mjs`
  enforces the census in CI. It is not yet a real-worker executor for every
  corpus case.

## Next gate order

1. Complete logged manual Tauri Open -> Extrude -> Fillet -> Undo -> Save ->
   Reopen and the unchanged T0 semantic/digest run; then mark the P2 gate
   passed.
2. Finish P3 contract rows and remaining Transform/import/UI decisions.
3. Add the P4 real-worker corpus executor and operation evidence.
