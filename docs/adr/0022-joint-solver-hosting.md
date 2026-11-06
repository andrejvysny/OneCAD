# 0022 — The joint solver is pure Rust in the host process

- Status: Proposed (2026-11-04). Delivery and qualification status belongs in
  Plane (ONECAD-341); this record does not establish implementation or gate
  completion.
- Date: 2026-11-04
- Related: ADR-0016 (sidecar worker), ADR-0020, ADR-0021.

## Context

Joints position occurrences relative to each other (ADR-0020). Interactive drag
needs a solve per frame. ADR-0016 keeps OCCT/PlaneGCS in a sidecar behind
OCW1, where each request pays IPC and a fenced round trip. PlaneGCS is a 2D
sketch solver and is not a 3D rigid-body joint solver.

The crate scaffold `crates/onecad-assembly` (`joint.rs`, `solver.rs`, plus
`math.rs` and `occurrence.rs`) exists, untested.

## Decision

1. **The joint solver is pure Rust in `onecad-assembly`, in the host process.**
2. **The worker resolves joint origin frames once per regen** (per definition,
   in its local frame). **Drag solves frames in Rust with no IPC.**
3. **Algorithm.** Levenberg-Marquardt with a finite-difference Jacobian
   initially. Numerical rank of the Jacobian reports remaining DOF and
   redundancy.
4. **Failure is atomic.** A failed or non-converged solve never partially moves
   the assembly; the previous transforms stay.
5. **Publish is fenced** on runtime, epoch and base hash, like other worker-
   derived state, so a stale solve is dropped.
6. **Joint types v1:** rigid, revolute, slider. Cylindrical, planar and ball
   are cut candidates.
7. **Performance target:** 8 ms p95 for 20 occurrences. This is a target
   pending measurement, not a result.

### Rejected alternatives

- **OndselSolver.** Rejected: LGPL, coupled to FreeCAD, maintenance uncertain,
  and a solve per drag frame would need IPC or in-process linkage with the
  ADR-0016 licensing review. Kept as a **spike benchmark only**, to compare
  convergence and cost, not as a dependency.
- **Solve in the C++ worker.** Rejected: IPC and fencing per drag frame, and
  the worker has no assembly state (ADR-0020 keeps placement in Rust).
- **Reuse PlaneGCS.** Rejected: 2D sketch constraints, not 3D rigid frames.

## Consequences

- Drag latency depends on host CPU only; worker crash or wedge does not stop
  interactive positioning.
- The solver is deterministic Rust, testable without OCCT, but we own
  conditioning, rank decisions and redundancy semantics.
- Finite-difference Jacobians cost O(variables) residual evaluations per
  iteration; analytic Jacobians may be needed if the target is missed.
- Joint frames go stale across regen; consumers must re-resolve and re-solve
  under the fencing in item 5.
- Nothing is implemented beyond the scaffold; no convergence or timing
  evidence exists.

## Open questions

- Solver spike scope: how much of OndselSolver comparison (convergence
  basin, redundancy reporting, timing) is needed before committing?
- Whether finite-difference LM meets 8 ms p95 at 20 occurrences, and what the
  real scale ceiling is against the 2000-occurrence cap.
- Whether cylindrical, planar and ball joints are v1 after all.
- Occurrence transforms outside the timeline vs captured positions: what is
  persisted after a solve?
- Deferral of ONECAD-399 (cross-component sketch references) and its effect on
  joint frame sources.
- Mirrored occurrences refusal, and joint frames on imported mirrored parts.
- Workbench and tree visual designs are supplied by the user; none are
  assumed here.
