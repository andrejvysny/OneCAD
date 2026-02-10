# Fix Line Drag + H/V Guide Commit Regressions

## TL;DR

> **Quick Summary**: Regressions came from interaction-layer coupling: guide-first snap override is applied during commit (`mousePress/mouseRelease`) and can collapse line endpoint identity, while point-drag behavior in select mode needs isolated verification in solver drag path. Fix with TDD using two separate red-green tracks and no broad solver/snap redesign.
>
> **Deliverables**:
> - Commit-phase snap policy fix for line click-commit under H/V guides
> - Regression fix/validation for select-mode point drag stability
> - New integration-level regression tests (manager+tool and drag semantics)
> - Full prototype verification evidence
>
> **Estimated Effort**: Medium
> **Parallel Execution**: YES - 2 waves
> **Critical Path**: Task 1 -> Task 2 -> Task 4 -> Task 6

---

## Context

### Original Request
Latest changes broke: (1) line/point drag behavior in edit-existing-sketch mode (no active tool), and (2) line point click-commit when horizontal/vertical guides are active. User requested iterative deep analysis and fix.

### Interview Summary
**Key Discussions**:
- Drag repro context is **no active tool** (selection/point-drag path).
- Test strategy selected by user: **TDD**.
- H/V guide visuals + cursor snapping work, but commit click fails.

**Research Findings**:
- Click-commit path: `src/ui/viewport/Viewport.cpp` -> `src/core/sketch/tools/SketchToolManager.cpp` -> `src/core/sketch/tools/LineTool.cpp`.
- Drag path bypasses tool manager snap policy: `src/ui/viewport/Viewport.cpp` -> `src/core/sketch/Sketch.cpp` (`beginPointDrag/solveWithDrag/endPointDrag`) -> `src/core/sketch/solver/ConstraintSolver.cpp`.
- Existing tests are strong at `SnapManager` level, weak at tool-manager integration path.

### Metis Review
**Identified Gaps** (addressed in this plan):
- Missing test isolation between click-commit regression and drag regression.
- Missing guardrails to prevent scope creep into broad solver/snap refactors.
- Missing integration-level acceptance criteria for line commit identity (`startId != endId`).
- Missing explicit constraint-policy checks for drag under constrained conditions.

---

## Work Objectives

### Core Objective
Restore pre-regression interaction behavior: line point commit must work under H/V guides, and select-mode point drag must preserve expected geometry stability semantics.

### Concrete Deliverables
- `src/core/sketch/tools/SketchToolManager.cpp`: commit-phase-safe snap policy behavior.
- `src/core/sketch/tools/LineTool.cpp` (only if needed by failing tests): endpoint identity handling aligned with commit policy.
- `tests/prototypes/proto_sketch_snap.cpp` and/or new prototype(s): integration regression tests for guide commit + drag semantics.

### Definition of Done
- [ ] New failing tests added first (red), then passing (green).
- [ ] H/V guide active: second click creates valid line endpoint (no `startId == endId` collapse unless user truly clicks same point).
- [ ] Select-mode point drag behavior matches expected fixture invariants.
- [ ] `cmake --build build` exits 0.
- [ ] `./build/tests/proto_sketch_snap` exits 0.
- [ ] `./build/tests/proto_sketch_solver` exits 0.
- [ ] `./build/tests/proto_sketch_fixed_and_move` exits 0.

### Must Have
- Guide-first behavior preserved for preview quality, but commit-phase must not invalidate endpoint commit.
- No regression in existing snap priority guardrails unless explicitly test-justified.
- Drag verification done on clean fixture geometry and post-creation geometry.

### Must NOT Have (Guardrails)
- Do NOT redesign full snap architecture.
- Do NOT broad-refactor `ConstraintSolver` internals beyond regression necessity.
- Do NOT alter `SnapType` enum ordering globally.
- Do NOT change coordinate mapping semantics (`SketchPlane::XY`).
- Do NOT touch ElementMap/Kernel/STEP/undo-redo layers.

### OneCAD Context Checks
- **Layer impact**: UI -> Core/Sketch -> Solver (no Kernel).
- **ElementMap impact**: none expected.
- **PlaneGCS impact**: verify drag solver behavior only; no API redesign.
- **Coordinate system implications**: preserve sketch-space behavior; no world mapping edits.
- **STEP I/O + undo/redo note**: unaffected.

---

## Verification Strategy (MANDATORY)

> **UNIVERSAL RULE: ZERO HUMAN INTERVENTION**
>
> All acceptance checks must be executable by agent/tool commands.

### Test Decision
- **Infrastructure exists**: YES
- **Automated tests**: **TDD**
- **Framework**: CMake prototype executables

### If TDD Enabled

Each regression task follows RED-GREEN-REFACTOR:
1. **RED**: add failing regression test
2. **GREEN**: minimal fix for pass
3. **REFACTOR**: clean while keeping tests green

### Agent-Executed QA Scenarios (MANDATORY)

Scenario: H/V guide click-commit works
  Tool: Bash (`proto_sketch_snap` or dedicated new prototype)
  Preconditions: regression test for H/V guide commit added
  Steps:
    1. Build target(s): `cmake --build build --target proto_sketch_snap`
    2. Run: `./build/tests/proto_sketch_snap`
    3. Assert: test `hv_guide_click_commit_creates_distinct_endpoint` PASS
  Expected Result: line commit succeeds under active H/V guide
  Failure Indicators: `startId == endId` path still blocks creation
  Evidence: terminal output capture

Scenario: Select-mode point drag stability
  Tool: Bash (`proto_sketch_solver` and/or dedicated drag regression prototype)
  Preconditions: drag fixture test added
  Steps:
    1. Build: `cmake --build build --target proto_sketch_solver proto_sketch_fixed_and_move`
    2. Run both binaries
    3. Assert: target drag regression test PASS and existing drag tests PASS
  Expected Result: only intended geometry changes; invariants hold
  Failure Indicators: unrelated points move outside expected constraints
  Evidence: terminal output capture

Scenario: No broader snap regression
  Tool: Bash (`proto_sketch_snap` full suite)
  Preconditions: commit policy fix integrated
  Steps:
    1. Run full suite
    2. Assert all existing + new tests PASS
  Expected Result: no break in existing snapping behavior
  Failure Indicators: failures in existing tests (priority/guide metadata/etc.)
  Evidence: pass count output

---

## Execution Strategy

### Parallel Execution Waves

Wave 1 (parallel):
- Task 1: Add RED tests for H/V click-commit regression
- Task 3: Add RED tests for select-mode point drag invariants

Wave 2 (sequential):
- Task 2: Implement minimal fix for click-commit path
- Task 4: Implement minimal fix for drag path (only if RED persists after Task 2)

Wave 3:
- Task 5: Refactor/cleanup + regression hardening
- Task 6: Final full verification

Critical Path: 1 -> 2 -> 4 -> 6

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|------|------------|--------|---------------------|
| 1 | None | 2,5,6 | 3 |
| 2 | 1 | 5,6 | None |
| 3 | None | 4,5,6 | 1 |
| 4 | 3 | 5,6 | None |
| 5 | 2,4 | 6 | None |
| 6 | 1-5 | None | None |

### Agent Dispatch Summary

| Wave | Tasks | Recommended Agents |
|------|-------|-------------------|
| 1 | 1,3 | `task(category="tests", load_skills=["proto-test"], run_in_background=false)` |
| 2 | 2,4 | `task(category="sketching", load_skills=[], run_in_background=false)` |
| 3 | 5,6 | `task(category="quick", load_skills=[], run_in_background=false)` |

---

## TODOs

- [ ] 1. RED: Add H/V guide click-commit regression tests

  **What to do**:
  - Add test proving H/V guide active still allows line endpoint commit.
  - Assert endpoint identity is valid (`startId != endId`) when second click is intended as new endpoint.
  - Add negative/control case around near-start-point behavior.

  **Must NOT do**:
  - Do not weaken existing snap tests.

  **Recommended Agent Profile**:
  - **Category**: `tests`
    - Reason: regression-first TDD work.
  - **Skills**: [`proto-test`]
    - `proto-test`: existing prototype harness patterns.
  - **Skills Evaluated but Omitted**:
    - `render-gl`: no rendering change in this task.

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Task 3)
  - **Blocks**: 2,5,6
  - **Blocked By**: None

  **References**:
  - `tests/prototypes/proto_sketch_snap.cpp` - existing snap test style and registration pattern.
  - `src/core/sketch/tools/SketchToolManager.cpp` - commit-phase snap selection currently applying guide-first.
  - `src/core/sketch/tools/LineTool.cpp` - commit guard path using `startId/endId`.
  - `src/core/sketch/SnapManager.cpp` - H/V guide snap metadata (`pointId`, `hasGuide`, `guideOrigin`).

  **Acceptance Criteria**:
  - [ ] Failing test added first (RED).
  - [ ] Test precisely detects pre-fix line-commit failure mode.

- [ ] 2. GREEN: Fix commit-phase guide handling for line creation

  **What to do**:
  - Implement minimal policy split/filter so move-preview keeps guide UX but commit phase does not collapse endpoint identity.
  - Ensure second-click commit can create endpoint under H/V guide when intended.
  - Keep behavior deterministic.

  **Must NOT do**:
  - No global snap priority redesign.
  - No broad line tool workflow rewrite.

  **Recommended Agent Profile**:
  - **Category**: `sketching`
    - Reason: interaction + tool-manager logic.
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `constraint`: not a new constraint feature.

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2
  - **Blocks**: 5,6
  - **Blocked By**: 1

  **References**:
  - `src/core/sketch/tools/SketchToolManager.cpp` - current `handleMousePress/Release` snap path.
  - `src/core/sketch/tools/LineTool.cpp` - endpoint creation semantics.
  - `src/ui/viewport/Viewport.cpp` - event flow into manager/tool.

  **Acceptance Criteria**:
  - [ ] RED test from Task 1 passes.
  - [ ] Existing snap suite remains green.

- [ ] 3. RED: Add select-mode point-drag invariants regression tests

  **What to do**:
  - Add fixture-based drag tests covering no-active-tool path semantics.
  - Assert which points/entities must stay fixed under drag for constrained scenarios.
  - Add at least one over/under-constrained negative case.

  **Must NOT do**:
  - Do not encode vague visual expectations; only numeric invariants.

  **Recommended Agent Profile**:
  - **Category**: `tests`
    - Reason: solver/drag invariant spec by tests.
  - **Skills**: [`proto-test`]
    - `proto-test`: drag fixture and prototype harness patterns.
  - **Skills Evaluated but Omitted**:
    - `render-gl`: no rendering assertions needed.

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Task 1)
  - **Blocks**: 4,5,6
  - **Blocked By**: None

  **References**:
  - `tests/prototypes/proto_sketch_solver.cpp` - existing drag/solver fixture patterns.
  - `tests/prototypes/proto_sketch_fixed_and_move.cpp` - fixed-point and move behavior expectations.
  - `src/core/sketch/Sketch.cpp` - `beginPointDrag/solveWithDrag/endPointDrag` behavior.
  - `src/core/sketch/solver/ConstraintSolver.cpp` - drag solve fixed-point enforcement.

  **Acceptance Criteria**:
  - [ ] Failing drag regression test added first (RED).
  - [ ] Test isolates no-active-tool drag path expectations.

- [ ] 4. GREEN: Fix select-mode drag instability (if RED persists)

  **What to do**:
  - Apply smallest fix needed in drag path/solver handoff to satisfy Task 3 tests.
  - Preserve existing successful drag scenarios.

  **Must NOT do**:
  - No broad solver architecture refactor.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: constraint/drag behavior can be subtle.
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `ultrabrain`: avoid unless tests show deep nonlinear solver issue.

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2
  - **Blocks**: 5,6
  - **Blocked By**: 3

  **References**:
  - `src/ui/viewport/Viewport.cpp` - point-drag state machine.
  - `src/core/sketch/Sketch.cpp` - active drag fixed-points management.
  - `src/core/sketch/solver/ConstraintSolver.cpp` - drag solve implementation.

  **Acceptance Criteria**:
  - [ ] RED tests from Task 3 pass.
  - [ ] Existing solver/move tests remain green.

- [ ] 5. REFACTOR: Consolidate policy and harden regressions

  **What to do**:
  - Refactor minimal duplicated logic introduced by fixes.
  - Add/adjust tests for edge cases: near-threshold click, near-start-point click, constrained drag failure handling.
  - Ensure no stale guide state across transition paths.

  **Must NOT do**:
  - No behavior expansion outside two regressions.

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: targeted cleanup/hardening.
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `writing`: docs not required for closure.

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3
  - **Blocks**: 6
  - **Blocked By**: 2,4

  **References**:
  - `src/core/sketch/tools/SketchToolManager.cpp`
  - `src/core/sketch/tools/LineTool.cpp`
  - `tests/prototypes/proto_sketch_snap.cpp`
  - `tests/prototypes/proto_sketch_solver.cpp`

  **Acceptance Criteria**:
  - [ ] No duplicate/contradictory snap policy paths.
  - [ ] Added edge-case tests pass.

- [ ] 6. Final verification and evidence capture

  **What to do**:
  - Run final command suite and collect outputs.
  - Ensure both regressions closed and no snap/drag regressions introduced.

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Final
  - **Blocked By**: 1-5

  **References**:
  - `tests/CMakeLists.txt` - canonical target names.

  **Acceptance Criteria**:
  - [ ] `cmake --build build --target proto_sketch_snap proto_sketch_solver proto_sketch_fixed_and_move` exits 0
  - [ ] `./build/tests/proto_sketch_snap` exits 0
  - [ ] `./build/tests/proto_sketch_solver` exits 0
  - [ ] `./build/tests/proto_sketch_fixed_and_move` exits 0

---

## Commit Strategy

| After Task | Message | Files | Verification |
|---|---|---|---|
| 1 | `test(sketch): add hv guide click-commit regression` | proto tests | proto target run |
| 2 | `fix(sketch): prevent guide commit endpoint collapse` | manager/tool files | snap + new tests |
| 3-4 | `test/fix(sketch): harden select-mode point drag invariants` | solver/sketch tests + minimal fix | solver + fixed_and_move |
| 5-6 | `chore(test): regression hardening and final verification` | tests only/minimal cleanup | full command suite |

---

## Success Criteria

### Verification Commands
```bash
cmake --build build --target proto_sketch_snap proto_sketch_solver proto_sketch_fixed_and_move
./build/tests/proto_sketch_snap
./build/tests/proto_sketch_solver
./build/tests/proto_sketch_fixed_and_move
```

### Final Checklist
- [ ] H/V guide active + second click creates valid line endpoint when intended
- [ ] No `startId == endId` false-abort regression under guide commit path
- [ ] Select-mode point drag preserves expected constrained geometry invariants
- [ ] Existing snap and solver suites remain green

---

## Defaults Applied (override if needed)
- Commit semantics default: guides influence preview strongly; commit path may apply guard/filter to preserve valid endpoint identity.
- Scope default: line commit + no-active-tool point drag only; no arc/rectangle behavior expansion unless failing tests prove shared impact.
- Priority default: close click-commit regression first, then drag regression.

## Unresolved Questions
- guide commit exact rule: preview-only vs commit-with-guard?
- drag expected behavior detail under constrained cases?
