# Sketcher Snap/Guides CAD-Parity Reliability Plan

## TL;DR

> **Quick Summary**: Stabilize OneCAD sketch interactions by enforcing a deterministic split between preview and commit decisions, explicit-snap-first commit policy (AutoCAD/Onshape baseline), and transactional drag rollback semantics.
>
> **Deliverables**:
> - Formal behavior spec (XY decision matrix) for Sketcher/Constraints/Snap/Guides
> - Line-first deterministic commit resolver + reject-reason pipeline
> - Deterministic snap tie-breakers and commit-boundary latching
> - Select-mode drag rollback contract + diagnostics
> - Regression tests and propagation path to Arc/Rectangle/Ellipse
>
> **Estimated Effort**: Large
> **Parallel Execution**: YES - 4 waves
> **Critical Path**: 1 -> 2 -> 3 -> 5 -> 6 -> 8 -> 9

---

## Context

### Original Request
User reports intermittent sketch regressions after recent auto-snap + visual-guide updates: clicks sometimes no-op, sometimes appear to remove last polyline segment, and guide behavior is inconsistent. User asked for deep web research + deep code analysis, then a specific optimized specification aligned with major CAD behavior.

### Interview Summary
**Key Decisions**:
- Commit fallback: when guide-based commit would resolve to same endpoint as start (`startId == endId`), **ignore guide for commit** and re-resolve explicit/raw candidate.
- Drag failure policy: if select-mode drag target is unsolved, **rollback to drag-start pose**.
- Rollout scope: **line-first**, then propagate to Arc/Rectangle/Ellipse.
- Baseline precedence: **AutoCAD/Onshape style** (explicit snaps authoritative; guides/inference advisory unless explicitly accepted).
- Test strategy: **TDD**.

**Research Findings**:
- Guide-first policy currently applied in all phases (`press/move/release`) at `src/core/sketch/tools/SketchToolManager.cpp`.
- Line no-op paths are silent at `src/core/sketch/tools/LineTool.cpp` (`MIN_GEOMETRY_SIZE` and `startId == endId`).
- Select-mode drag path is independent from tool-manager snaps: `src/ui/viewport/Viewport.cpp` -> `src/core/sketch/Sketch.cpp` -> `src/core/sketch/solver/ConstraintSolver.cpp`.
- Cross-CAD anti-patterns confirm need to block null-length commits and make finish/cancel deterministic.

### Metis Review
**Addressed Gaps**:
- Added explicit guardrails against guide-overriding explicit commit intent.
- Added deterministic tie-break and single commit-boundary requirements.
- Added reject taxonomy + telemetry requirements.
- Added line-first propagation gate criteria.

---

## Work Objectives

### Core Objective
Make sketch interactions deterministic and CAD-consistent across line creation, snapping/guides, and select-mode drag, eliminating intermittent no-op/rollback-perception regressions.

### Concrete Deliverables
- `docs/sketch/INTERACTION_SPEC.md` (or equivalent): formal CAD-parity behavior contract and XY decision tables.
- `src/core/sketch/tools/SketchToolManager.cpp`: preview-vs-commit policy split and single commit resolver.
- `src/core/sketch/SnapManager.cpp` (+ possibly `.h`): deterministic tie-break guarantees.
- `src/core/sketch/tools/LineTool.cpp`: explicit reject reasons for commit failures.
- `src/ui/viewport/Viewport.cpp` + `src/core/sketch/Sketch.cpp`: drag rollback contract wiring and diagnostics.
- New/updated prototypes under `tests/prototypes/` for line commit determinism and drag rollback determinism.

### Definition of Done
- [ ] Spec document includes complete XY matrix for Sketcher/Constraints/Snap/Guides.
- [ ] H/V guide visible + explicit endpoint available => explicit endpoint commit wins.
- [ ] `startId == endId` via guide collapse no longer causes silent no-op.
- [ ] Select-mode unsolved drag reliably restores drag-start pose.
- [ ] Deterministic tie-break for equal-priority/equal-distance snaps verified by tests.
- [ ] All required build/tests pass.

### Must Have
- Explicit-snap-first commit contract (AutoCAD/Onshape baseline).
- Guides may lead preview, but cannot silently corrupt commit intent.
- One commit decision per commit boundary event.
- Structured reject/failure reasons (machine-readable + user-facing status text).

### Must NOT Have (Guardrails)
- Do NOT redesign full solver architecture.
- Do NOT reorder global `SnapType` enum semantics blindly.
- Do NOT mix preview rendering state with commit mutation state.
- Do NOT propagate to Arc/Rectangle/Ellipse before line-first tests are green.
- Do NOT touch Kernel/ElementMap/STEP/undo-redo architecture.

### OneCAD Context Checks
- **Layer impact**: UI -> Core/Sketch -> Solver (no kernel changes expected).
- **ElementMap impact**: none expected.
- **PlaneGCS impact**: drag solve semantics and diagnostics only (no algorithm replacement).
- **Coordinate system implications**: preserve sketch-space behavior and existing XY mapping.
- **STEP I/O + undo/redo note**: no scope change.

---

## Verification Strategy (MANDATORY)

> **UNIVERSAL RULE: ZERO HUMAN INTERVENTION**
>
> All verification must be fully agent-executable.

### Test Decision
- **Infrastructure exists**: YES
- **Automated tests**: TDD
- **Framework**: CMake prototype executables

### TDD Structure
For each regression family:
1. **RED**: add failing regression test
2. **GREEN**: minimal fix
3. **REFACTOR**: cleanup while preserving green

### Agent-Executed QA Scenarios (MANDATORY)

Scenario: Guide vs explicit endpoint commit arbitration
  Tool: Bash (prototype target)
  Preconditions: commit-policy prototype includes explicit-vs-guide case
  Steps:
    1. `cmake --build build --target proto_sketch_snap`
    2. `./build/tests/proto_sketch_snap`
    3. Assert case: guide visible + explicit endpoint available => explicit endpoint selected
    4. Assert case: guide collapse (`startId==endId`) => explicit/raw fallback commit path
  Expected Result: deterministic commit winner and no silent no-op
  Evidence: terminal output with named PASS cases

Scenario: Polyline no-op reject reasons are explicit
  Tool: Bash (prototype target)
  Preconditions: reject reason codes instrumented
  Steps:
    1. Build/run line-commit regression prototype
    2. Trigger tiny-segment case
    3. Trigger same-endpoint case
    4. Assert reject codes: `too_short`, `same_endpoint`
  Expected Result: no geometry mutation on reject + explicit reason emitted
  Evidence: terminal output includes reject reason assertions

Scenario: Select-mode drag rollback determinism
  Tool: Bash (solver/drag prototype)
  Preconditions: drag-start snapshot contract implemented
  Steps:
    1. Build/run drag rollback prototype
    2. Trigger unsolved target
    3. Assert final geometry equals drag-start snapshot
    4. Assert failure reason emitted once
  Expected Result: deterministic rollback, no partial drift
  Evidence: terminal output with coordinate equality assertions

Scenario: Full regression sweep
  Tool: Bash
  Preconditions: all patches integrated
  Steps:
    1. `cmake --build build --target proto_sketch_snap proto_sketch_solver proto_sketch_fixed_and_move proto_sketch_geometry proto_sketch_constraints`
    2. Run all listed prototypes
    3. Assert all exit codes 0
  Expected Result: no regression across existing suites
  Evidence: command outputs and final pass counts

---

## Execution Strategy

### Parallel Execution Waves

Wave 1 (parallel, RED/spec lock):
- Task 1: Finalize formal interaction spec document (XY contract)
- Task 2: Add failing tests for line commit determinism
- Task 3: Add failing tests for drag rollback determinism

Wave 2 (sequential, GREEN line-first):
- Task 4: Implement preview-vs-commit resolver split
- Task 5: Implement line reject taxonomy + explicit fallback behavior
- Task 6: Add deterministic snap tie-break keys

Wave 3 (sequential, GREEN drag):
- Task 7: Implement drag rollback contract + diagnostics
- Task 8: Harden commit boundary (single decision per commit event)

Wave 4 (sequential, propagation + verify):
- Task 9: Propagate shared resolver contract to Arc/Rectangle/Ellipse (no behavior drift)
- Task 10: Final full verification and evidence capture

Critical Path: 1 -> 2 -> 4 -> 5 -> 6 -> 8 -> 9 -> 10

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|------|------------|--------|---------------------|
| 1 | None | 4-10 | 2,3 |
| 2 | None | 4-10 | 1,3 |
| 3 | None | 7-10 | 1,2 |
| 4 | 1,2 | 5,6,8,9,10 | None |
| 5 | 4 | 6,8,9,10 | None |
| 6 | 5 | 8,9,10 | None |
| 7 | 3 | 8,9,10 | None |
| 8 | 6,7 | 9,10 | None |
| 9 | 8 | 10 | None |
| 10 | 1-9 | None | None |

### Agent Dispatch Summary

| Wave | Tasks | Recommended Agents |
|------|-------|-------------------|
| 1 | 1,2,3 | `tests` + `proto-test` for RED tests; `writing` for spec doc |
| 2 | 4,5,6 | `sketching` / `quick` for resolver + tie-break changes |
| 3 | 7,8 | `unspecified-high` for drag contract + state-machine hardening |
| 4 | 9,10 | `quick` with targeted tool files and full verification |

---

## TODOs

- [ ] 1. Finalize formal CAD-parity interaction specification (XY matrix)

  **What to do**:
  - Produce authoritative spec doc with MUST/SHOULD clauses for Sketcher/Constraints/Snap/Guides.
  - Include XY outcome matrix for failure-prone interactions.
  - Encode user-decided defaults (explicit-first commit; drag-start rollback).

  **Must NOT do**:
  - No ambiguous prose without testable outcomes.

  **Recommended Agent Profile**:
  - **Category**: `writing`
    - Reason: precise, normative spec artifact.
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2,3)
  - **Blocks**: 4-10
  - **Blocked By**: None

  **References**:
  - `.sisyphus/drafts/sketcher-snap-guides-research-spec.md` - research synthesis + confirmed decisions.
  - `src/core/sketch/tools/SketchToolManager.cpp` - current policy coupling points.
  - `src/core/sketch/tools/LineTool.cpp` - reject branches and line FSM behavior.

  **Acceptance Criteria**:
  - [ ] XY matrix includes explicit outcomes for all reported failure classes.
  - [ ] All MUST clauses are machine-testable.

- [ ] 2. RED: Add failing line-commit determinism tests

  **What to do**:
  - Add tests for:
    - explicit endpoint vs guide conflict
    - guide collapse (`startId==endId`) fallback
    - tiny-segment reject reason behavior
  - Ensure tests fail under current behavior (RED).

  **Must NOT do**:
  - Do not loosen existing test assertions.

  **Recommended Agent Profile**:
  - **Category**: `tests`
  - **Skills**: [`proto-test`]

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1
  - **Blocks**: 4-10
  - **Blocked By**: None

  **References**:
  - `tests/prototypes/proto_sketch_snap.cpp`
  - `src/core/sketch/tools/LineTool.cpp`
  - `src/core/sketch/tools/SketchToolManager.cpp`

  **Acceptance Criteria**:
  - [ ] New failing tests added and reproducible.

- [ ] 3. RED: Add failing drag rollback determinism tests

  **What to do**:
  - Add tests asserting unsolved drag restores drag-start pose exactly.
  - Add tests for fixed-point blocked drag reason.
  - Add at least one coupled-constraint drag case.

  **Must NOT do**:
  - Do not rely on manual viewport checks.

  **Recommended Agent Profile**:
  - **Category**: `tests`
  - **Skills**: [`proto-test`]

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1
  - **Blocks**: 7-10
  - **Blocked By**: None

  **References**:
  - `src/ui/viewport/Viewport.cpp`
  - `src/core/sketch/Sketch.cpp`
  - `src/core/sketch/solver/ConstraintSolver.cpp`
  - `tests/prototypes/proto_sketch_solver.cpp`
  - `tests/prototypes/proto_sketch_fixed_and_move.cpp`

  **Acceptance Criteria**:
  - [ ] New failing drag tests added and reproducible.

- [ ] 4. GREEN: Split preview and commit snap resolver paths

  **What to do**:
  - Introduce explicit commit resolver separate from preview resolver.
  - Restrict guide-first behavior to preview channel by default.
  - Ensure commit uses explicit-first arbitration unless explicit guide acceptance mode is enabled.

  **Must NOT do**:
  - No broad SnapManager redesign.

  **Recommended Agent Profile**:
  - **Category**: `sketching`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2
  - **Blocks**: 5,6,8,9,10
  - **Blocked By**: 1,2

  **References**:
  - `src/core/sketch/tools/SketchToolManager.cpp`
  - `src/core/sketch/SnapManager.cpp`

  **Acceptance Criteria**:
  - [ ] Task 2 RED tests now pass.

- [ ] 5. GREEN: Implement line commit fallback + reject taxonomy

  **What to do**:
  - Implement fallback: if commit resolves to `startId==endId` via guide, demote guide and re-resolve explicit/raw candidate.
  - Emit deterministic reject reasons (`too_short`, `same_endpoint`, etc.).
  - Preserve previous committed geometry on reject.

  **Must NOT do**:
  - No silent reject paths remaining for these cases.

  **Recommended Agent Profile**:
  - **Category**: `sketching`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2
  - **Blocks**: 6,8,9,10
  - **Blocked By**: 4

  **References**:
  - `src/core/sketch/tools/LineTool.cpp`
  - `src/core/sketch/tools/SketchToolManager.cpp`

  **Acceptance Criteria**:
  - [ ] Guide-collapse no longer produces silent no-op.
  - [ ] Reject reason emitted and asserted by tests.

- [ ] 6. GREEN: Add deterministic snap tie-break rules

  **What to do**:
  - Add stable tie-break keys after type/distance (e.g., pointId/entityId/stable index).
  - Ensure equal-distance ties are deterministic across runs.

  **Must NOT do**:
  - No nondeterministic container-order dependence.

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2
  - **Blocks**: 8,9,10
  - **Blocked By**: 5

  **References**:
  - `src/core/sketch/SnapManager.cpp`
  - `src/core/sketch/SnapManager.h`

  **Acceptance Criteria**:
  - [ ] Equal-tie scenarios resolve reproducibly.

- [ ] 7. GREEN: Implement drag-start rollback contract + diagnostics

  **What to do**:
  - Enforce drag-start snapshot usage for unsolved drag release rollback.
  - Ensure failure status emitted exactly once per failed drag attempt.

  **Must NOT do**:
  - No partial geometry drift after unsolved release.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3
  - **Blocks**: 8,9,10
  - **Blocked By**: 3

  **References**:
  - `src/ui/viewport/Viewport.cpp`
  - `src/core/sketch/Sketch.cpp`
  - `src/core/sketch/solver/ConstraintSolver.cpp`

  **Acceptance Criteria**:
  - [ ] Task 3 RED tests now pass.

- [ ] 8. Harden commit boundary and event determinism

  **What to do**:
  - Ensure one commit decision per commit event boundary.
  - Remove press/release dual-decision ambiguity.
  - Add commit decision telemetry fields for debugging.

  **Must NOT do**:
  - No hidden multi-commit side effects per click cycle.

  **Recommended Agent Profile**:
  - **Category**: `sketching`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3
  - **Blocks**: 9,10
  - **Blocked By**: 6,7

  **References**:
  - `src/core/sketch/tools/SketchToolManager.cpp`
  - `src/core/sketch/tools/LineTool.cpp`

  **Acceptance Criteria**:
  - [ ] Commit decision logged once and consistent per event.

- [ ] 9. Propagate shared contract to Arc/Rectangle/Ellipse (line-first complete)

  **What to do**:
  - Apply same resolver/commit/reject contract to Arc/Rectangle/Ellipse integration points.
  - Add tool-specific regression checks for no-op/short-geometry/cancel determinism.

  **Must NOT do**:
  - No tool-specific divergence from shared contract semantics.

  **Recommended Agent Profile**:
  - **Category**: `sketching`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 4
  - **Blocks**: 10
  - **Blocked By**: 8

  **References**:
  - `src/core/sketch/tools/ArcTool.cpp`
  - `src/core/sketch/tools/RectangleTool.cpp`
  - `src/core/sketch/tools/EllipseTool.cpp`
  - `src/core/sketch/tools/SketchToolManager.cpp`

  **Acceptance Criteria**:
  - [ ] Shared contract tests remain green after propagation.

- [ ] 10. Final verification and evidence capture

  **What to do**:
  - Run full build and regression command suite.
  - Capture pass/fail evidence for all new and existing tests.

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Final
  - **Blocked By**: 1-9

  **References**:
  - `tests/CMakeLists.txt`

  **Acceptance Criteria**:
  - [ ] `cmake --build build --target proto_sketch_snap proto_sketch_solver proto_sketch_fixed_and_move proto_sketch_geometry proto_sketch_constraints` exits 0
  - [ ] `./build/tests/proto_sketch_snap` exits 0
  - [ ] `./build/tests/proto_sketch_solver` exits 0
  - [ ] `./build/tests/proto_sketch_fixed_and_move` exits 0
  - [ ] `./build/tests/proto_sketch_geometry` exits 0
  - [ ] `./build/tests/proto_sketch_constraints` exits 0

---

## Commit Strategy

| After Task | Message | Files | Verification |
|---|---|---|---|
| 1 | `docs(sketch): define cad-parity interaction contract` | spec doc | lint/readability check |
| 2-3 | `test(sketch): add deterministic commit and drag rollback regressions` | prototype tests | target prototype runs |
| 4-6 | `fix(sketch): split preview/commit resolver and stabilize snap arbitration` | manager/snap/line | snap + line tests |
| 7-8 | `fix(sketch): enforce drag rollback and single commit boundary` | viewport/sketch/solver + manager | drag + solver tests |
| 9-10 | `fix(sketch): propagate deterministic contract and verify suites` | arc/rect/ellipse + tests | full suite |

---

## Success Criteria

### Verification Commands
```bash
cmake --build build --target proto_sketch_snap proto_sketch_solver proto_sketch_fixed_and_move proto_sketch_geometry proto_sketch_constraints
./build/tests/proto_sketch_snap
./build/tests/proto_sketch_solver
./build/tests/proto_sketch_fixed_and_move
./build/tests/proto_sketch_geometry
./build/tests/proto_sketch_constraints
```

### Final Checklist
- [ ] Intermittent no-op click regressions eliminated in line-first flow
- [ ] Guide-visible commit respects explicit-snap-first contract
- [ ] `startId == endId` collapse handled via explicit/raw fallback
- [ ] Select-mode unsolved drag rolls back to drag-start pose
- [ ] Deterministic tie-break verified for equal candidate ties
- [ ] Line-first propagation readiness confirmed for Arc/Rectangle/Ellipse

---

## Auto-Resolved
- Tie-break default set to stable key chain after type/distance (`pointId` -> `entityId` -> deterministic insertion order).
- Drag rollback cadence defaulted to: keep last-valid preview during move, hard rollback to drag-start on unsolved release.
- Feedback channel defaulted to: structured reason code + status bar message.

## Defaults Applied (override if needed)
- Commit boundary default: single authoritative commit decision per commit event.
- Inference default: advisory in preview, guarded in commit.
- Propagation default: line-first gate must pass before expanding to Arc/Rectangle/Ellipse.

## Unresolved Questions
- none
