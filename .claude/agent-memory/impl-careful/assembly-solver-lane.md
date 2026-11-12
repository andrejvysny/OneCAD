---
name: assembly-solver-lane
description: Where the joint solver and its topology gate live, what the pure solver does NOT refuse, free-joint order dependence, cheap real-worker joint harness
metadata:
  type: project
---

Facts from the 2026-11-11 assembly seed-behaviour lane. Verify against code first.

- The pure joint solver is `crates/onecad-assembly/src/solver.rs` (+ `solver/seed.rs`, `solver/forest.rs`), NOT onecad-core; briefs may name the wrong crate. Its integration tests only see the public API (`solve`, `SolveRequest`, `SolveOutcome`, math types).
- The pure solver SOLVES loops, two grounds and floating groups (floating keeps 6 global DOF). Topology refusal lives in `crates/onecad-app/src/joint_execution.rs::validate_tree` (one ground per component, acyclic, <=20, endpoint present) plus core `AssemblyState::validate` (nested occurrence, unknown endpoint, joint limits). App bodies = ALL occurrences, so even a lone unjointed ungrounded occurrence refuses every joint solve.
- Free joints from far/fresh starts (all occurrences at identity) converge to a DIFFERENT valid configuration per joint/body order (measured up to 16.6 mm); small offsets agree to ~4e-9 mm. Only validity/DOF is order-invariant; do not assert equality for free far starts.
- A drive on a suppressed joint is accepted and ignored by the pure solver (`check_drives` does not filter suppressed); the app gate refuses it.
- Cheap real-worker joint harness: `native_plate_support::Harness` + `api::new_document` + `joint_execution::solve_joints` (unbound joints, empty definitions) runs in ~0.2 s; see `crates/onecad-app/tests/assembly_seed_joints.rs`.

**Why:** each took investigation time that a future joint/solver task would repeat.
**How to apply:** before adding solver tests or "fixing" topology refusals; see [[native-gate-lane]] for target-dir traps.
