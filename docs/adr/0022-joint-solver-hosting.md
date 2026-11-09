# 0022 — The joint solver is pure Rust in the host process

- Status: Bounded implementation decision (2026-11-06). Delivery and qualification status belongs in
  Plane (ONECAD-341); this record does not establish implementation or gate
  completion.
- Date: 2026-11-04
- Related: ADR-0016 (sidecar worker), ADR-0020, ADR-0021.

## Context

Joints position occurrences relative to each other (ADR-0020). Interactive drag
needs a solve per frame. ADR-0016 keeps OCCT/PlaneGCS in a sidecar behind
OCW1, where each request pays IPC and a fenced round trip. PlaneGCS is a 2D
sketch solver and is not a 3D rigid-body joint solver.

The pure `crates/onecad-assembly` solver, bounded application service, native
joint authoring and persistent picked-origin bindings are present in this source
tree. Imported-reference replay, final native qualification and physical-input
evidence remain open.

## Decision

1. **The joint solver is pure Rust in `onecad-assembly`, in the host process.**
2. **Origin frames are resolved against checked definition-local geometry.**
   Picked origins can require a transient component-worker replay and are fenced
   by session, occurrence path and mesh generation. The numerical solve itself
   runs in Rust off the document lock; this does not establish a no-IPC drag path.
3. **Algorithm.** The solver uses Levenberg-Marquardt with a finite-difference
   Jacobian. A grounded-forest certificate or numerical rank reports remaining
   DOF and redundancy; uncertain outcomes refuse publication.
4. **Failure is atomic.** A failed or non-converged solve never partially moves
   the assembly; the previous transforms stay.
5. **Publish is fenced** on runtime, epoch and base hash, like other worker-
   derived state, so a stale solve is dropped.
6. **Joint types v1:** rigid, revolute, slider. Cylindrical, planar and ball
   are cut candidates.
7. **Performance target:** 8 ms p95 for 20 occurrences. This is a target
   pending measurement, not a result.

### Rejected alternatives

- **OndselSolver.** Not selected for this bounded integration. The maintained
  [FreeCAD fork at 4be80eef](https://github.com/FreeCAD/OndselSolver/tree/4be80eef02a3486cda0d78f3ccbb308d207a9639)
  builds as a standalone C++17 shared library; it is not categorically coupled
  to FreeCAD. Its C++ object graph has no verified stable C interface, and the
  tested assembly entry point returns `void`, without the typed status, DOF,
  cancellation and atomic publication contract required here. The bounded
  comparison below supports retaining the existing solver, not rejecting the
  library for broader assembly work. Its
  [LGPL 2.1 license](https://github.com/FreeCAD/OndselSolver/blob/4be80eef02a3486cda0d78f3ccbb308d207a9639/LICENSE)
  requires a separate distribution design and review, including notices,
  corresponding library source and modifications, and the applicable
  replacement or relinking mechanism. A process boundary alone does not
  discharge those obligations.
- **Solve in the C++ worker.** Rejected: IPC and fencing per drag frame, and
  the worker has no assembly state (ADR-0020 keeps placement in Rust).
- **Reuse PlaneGCS.** Rejected: 2D sketch constraints, not 3D rigid frames.

## Consequences

- Frame acquisition and geometry readiness remain worker-dependent; no
  end-to-end drag latency or worker-failure independence is claimed.
- The solver is deterministic Rust, testable without OCCT, but we own
  conditioning, rank decisions and redundancy semantics.
- Finite-difference Jacobians cost O(variables) residual evaluations per
  iteration; analytic Jacobians may be needed if the target is missed.
- Definition edits can mark picked-origin bindings `NeedsRepair`; consumers
  must resolve or explicitly re-pick under the current fencing and ownership.
- The application captures explicit local joint frames and solves at most 20
  occurrences off the document lock. It accepts grounded acyclic components,
  fences publication by runtime session, persisted generation, accepted snapshot,
  worker epoch and expected root history hash, and commits all joints and poses
  in one undoable edit. Superseded or failed solves do not publish poses.
- The host permits one active and one waiting solve. Admission is retained until
  blocking solver work terminates, even if its caller is cancelled.
- Rack-and-pinion and other couplings, loops and ungrounded components are
  unsupported. Persistent picked-origin resolution remains under qualification.
  Rigid, revolute and slider joints are supported. Revolute and slider joints
  admit independent drives and limits; rigid joints do not admit drives.
  Reported DOF is the solver's estimate, and uncertain rank refuses publication.
- A single 20-occurrence debug-run sample does not qualify the 8 ms p95 target;
  native and iPad timing, full workflow integration and broader numerical
  qualification remain open.

## Bounded comparison, 6 November 2026

The pinned fork built locally with Apple Clang 21, CMake 4.1.3 and Release
settings, with upstream tests disabled because GoogleTest was unavailable.
Isolated adapters checked final poses against analytic expectations for rigid,
slider and revolute fixtures, including translated frames. With matching
slider limits of [-2, 2], 100 rigid and 100 free-slider samples passed those
external checks. Observed solver-only p95 values were 3.498 ms and 3.665 ms.
Logging, numerical tolerances, process setup and concurrent load differed from
the Rust measurements; these are not controlled comparative timings or native
end-to-end acceptance of the 8 ms target.

Driven sliders with limits crashed with SIGSEGV in isolated two- and
20-body adapters, including translated frames. Adapter misuse versus an
upstream defect remains unresolved. In negative cases, the entry point returned
without a typed status while final poses retained antiparallel fixed axes or
violated the coupled-slider limit. An integration must independently reject
those poses. The matching Rust regressions returned `Inconsistent` and
`NotConverged` without publishing poses. This limited evidence does not qualify
either solver for unsupported loops or couplings.

Evidence is recorded in ONECAD-341 and canonical QA. Local retained artifacts:
`onecad-ondsel-probe-limited-samples.json`, SHA-256
`f39182c249fd86d14bf4085c30425b9106266930113b754004ee1ab81d399d52`;
`onecad-ondsel-negative.log`, SHA-256
`c22be7412a66cdc01b9cc7c3262bcd11dbec8ccb0e6282d51abd52bfc48946e7`;
`onecad-rust-negative.log`, SHA-256
`07ce6021b98544ae72c4ff73bfec8b918719e9ab65cd00b02eed78caf2e6bf06`.
These local artifacts are not published CI evidence.

## Open questions

- A controlled comparison of convergence basin, redundancy and timing remains
  necessary before reconsidering the solver for a broader workload.
- Whether finite-difference LM meets 8 ms p95 at 20 occurrences under the native
  end-to-end workload. The 2000-occurrence storage bound is not a solve-capacity
  promise; this integration admits at most 20 solver bodies.
- What further numerical and resource evidence is needed for workloads beyond
  this slice. Cylindrical, planar and ball joints, cross-component sketch
  references and mirrored occurrence authoring remain outside the approved slice.
