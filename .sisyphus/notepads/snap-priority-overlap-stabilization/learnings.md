# Learnings

## Session Start
- Plan: snap-priority-overlap-stabilization (7 tasks, 3 waves)
- Baseline: 48/48 snap tests pass, all proto tests pass
- Key files: SnapManager.h/cpp, SketchToolManager.cpp, proto_sketch_snap.cpp

## Task 1 Learnings
- Added explicit overlap arbitration contract in `SnapResult` docs to lock co-located winner semantics and determinism.
- Added `SnapResult::kOverlapEps = 1e-6` as canonical co-location epsilon in sketch coordinates.
- Documented parity requirement: preview and commit must resolve same snap winner for identical input via `resolveSnapForInputEvent()` path.

- Added RED overlap priority tests (axis crossing determinism, point vs intersection, endpoint vs intersection, repeated-run winner) that currently pass because the existing comparator is deterministic; tests document desired behavior for future fixes.

## Parity Test Learnings
- Added `findBestGuideCandidate()` helper to mirror `resolveSnapForInputEvent()` preview logic and reuse `selectEffectiveSnap()` for parity assertions.
- Implemented three parity-focused tests: (1) `test_parity_no_guide_overlap` enforces preview/commit equality when only overlapping candidates exist, (2) `test_parity_guide_adjacent_to_overlap` reproduces a guide-bearing horizontal alignment near the overlap (cursor at y=4.8 forces the horizontal guide to snap at y=4.9), and (3) `test_parity_findBestSnap_stable_across_calls` reaffirms determinism across repeated `findBestSnap()` invocations.
- Tests now ensure preview picks the guide when available and that the commit winner remains the deterministic non-guide result in the overlapping scenario.

## Test Results
- `cmake --build build --target proto_sketch_snap` && `./build/tests/proto_sketch_snap`
- Existing `test_overlap_point_beats_intersection` is still red (expected behavior from baseline); all other tests, including the new parity checks, pass.

## Task 4 Learnings
- Fixed overlap arbitration in `SnapManager::findBestSnap()` by post-sort stabilization for co-located `Vertex` candidates at the same best distance.
- For those co-located `Vertex` ties, winner is now chosen by earliest point insertion order from `sketch.getAllEntities()` (point index), with comparator fallback for deterministic equality.
- This removes UUID-string randomness from same-position Vertex ties while preserving existing `SnapResult::operator<` contract and snap priority ordering.
- Validation: full build succeeded; `proto_sketch_snap` now `55/55`; no regressions in `proto_sketch_solver`, `proto_sketch_geometry`, `proto_sketch_constraints`.

## Task 5 Learnings
- Hardened `SketchToolManager::resolveSnapForInputEvent()` parity contract with Doxygen: commit path uses `findBestSnap()` directly, preview may prefer a guide candidate, and both paths are identical when no guide exists.
- Added commit fast path: when `preferGuide=false` and `allSnapsOut==nullptr`, function returns `findBestSnap()` result immediately and skips expensive `findAllSnaps()`.
- Added debug-only parity assertion on the non-fast path: if no guide-bearing candidate exists in `findAllSnaps()`, guide-preferred resolution must match `findBestSnap()` result.
- Validation: `cmake --build build --target proto_sketch_snap` passed; `./build/tests/proto_sketch_snap` `55/55`; `proto_sketch_solver`, `proto_sketch_geometry`, `proto_sketch_constraints` all passed.

## Ambiguity Hook Implementation
- Added `AmbiguityState` struct and hooks (`hasAmbiguity`, `ambiguityCandidateCount`, `cycleAmbiguity`, `clearAmbiguity`) to `SnapManager`.
- `findBestSnap` now populates `ambiguityState_` when multiple candidates are co-located within `kOverlapEps`.
- `ambiguityState_` is `mutable` to allow population within `const` `findBestSnap`.
- Methods `cycleAmbiguity` and `clearAmbiguity` are marked `const` as they only modify `mutable` state.
- Verified with a new test case in `proto_sketch_snap`.
2026-02-10: Task 7 full verification - all proto snaps/solver/geometry/constraints built and tested; snap overlap/parity/ambiguity suite ran 20x without flake; evidence stored under .sisyphus/evidence/task7-*.log
