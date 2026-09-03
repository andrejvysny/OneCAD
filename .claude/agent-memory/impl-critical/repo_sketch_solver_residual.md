---
name: repo-sketch-solver-residual
description: How SCHEMA 7.4 maxResidual is measured in the worker — the shared rule, the two exact-solve sites, and the EndGesture branch that would silently report a fake zero
metadata:
  type: project
---

The worker's sketch-solve residual reporting (WP-B T4.4, 2026-09-03).

**Why:** PlaneGCS answers `Converged` when its STEP fell below the convergence criterion whether or
not the ERROR did, so `success` alone cannot say whether a sketch is actually satisfied.

**How to apply:**

- The rule lives once, as `sk::maxConstraintResidual(sketch, constraints)` — a template in
  `worker/src/sketch/SketchConstraint.h` taking any pointer-like range (raw or `unique_ptr`). It is
  `max` over `getError()`, which already returns each kind's OWN dimension (mm or radians), skipping
  non-finite. Do not re-derive per kind; do not compare the number against a tolerance.
- `getError()` returns `+infinity` for an unreadable entity. The skip is load-bearing: a non-finite
  number is rejected on the OCW1 wire, so letting one through kills the frame, not just the value.
- Only the two EXACT solves report it: `ConstraintSolver::solve(const Sketch* residualContext)`
  (defaulted null) measures into `SolverResult::residual`; the three drag entries pass nothing and
  leave it 0 = UNMEASURED. `SketchUpsert` emits `solve.residual`.
- `SolverLane::on_end` must NOT source it from its own `SolveResult`: with `commit.finalTarget`,
  `on_end` reaches its final pose through `run_step()` (a drag entry), so `r.residual` is 0 on
  roughly half of all pointer-ups — a satisfied-looking lie. It measures from the sketch instead
  (`maxConstraintResidual(*g.sketch, g.sketch->getAllConstraints())`). Guarded by
  `test_solver_residual.cpp`; proven red-first.
- `ConstraintSolver::addConstraint` REFUSES a constraint it cannot translate, so the solver's own
  set never holds an unreadable one — the `+inf` skip only bites on the `getAllConstraints()` path.
- An unsupported/mismatched constraint cannot reach `Sketch::solve()`'s refusal from the wire:
  `wire::translate` rejects it first with `OP_FAILED: constraint '<id>' (<Type>) rejected`.
- Measured reference values: contradictory Distance 10 + Distance 20 on one point pair →
  `Diverged`, `success=false`, 2 conflicting, residual exactly 10.0 (the pose is RESTORED on
  failure); a satisfied dimension → 0.0.
