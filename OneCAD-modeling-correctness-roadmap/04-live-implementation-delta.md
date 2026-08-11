# Live Implementation Delta

Reviewed: 2026-08-11  
Implementation state: current uncommitted worktree after `055d31f7d8edcb106a881590b9b2c5a9d1e37982` (`055d31f`)
OCCT: 8.0.1, fingerprint `0a6a1dce34181289`

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
