# Snap Priority Overlap Stabilization

## TL;DR

> **Quick Summary**: Stabilize OneCAD snap arbitration when multiple candidates overlap (axis crossing, point+intersection) using deterministic scoring + explicit overlap policy while preserving existing baseline snap priority.
>
> **Deliverables**:
> - Deterministic overlap arbitration contract in SnapManager
> - Preview/commit parity contract in SketchToolManager
> - Overlap-focused prototype tests (TDD)
> - Optional ambiguity fallback hook (without full UI cycling rollout)
>
> **Estimated Effort**: Medium
> **Parallel Execution**: YES - 3 waves
> **Critical Path**: 1 -> 2 -> 4 -> 5 -> 7

---

## Context

### Original Request
Improve auto-snap priority when multiple entities overlap. Current behavior can feel unstable or like no snap at axis crossing and crossing+point situations.

### Interview Summary
**Key Discussions**:
- User confirmed overlap default: **point/endpoint wins over intersection**.
- Main pain: unstable/no-snap perception under overlap.

**Research Findings**:
- Internal audit: overlap arbitration currently depends on candidate ordering/tie-break; preview/commit divergence can exist.
- External CAD baseline: nearest-in-aperture + ambiguity fallback (cycle/lock), inference guides advisory, clear feedback.

### Metis Review
**Identified Gaps** (addressed):
- Need explicit guardrails to prevent scope creep into full UX redesign.
- Need concrete overlap matrix tests and anti-flake repeated-run criteria.
- Need strict preview/commit parity acceptance criteria.

---

## Work Objectives

### Core Objective
Make overlapping snap candidate selection deterministic and predictable, with explicit policy for co-located candidates and no ghost-snap behavior.

### Concrete Deliverables
- `src/core/sketch/SnapManager.h`: explicit arbitration/tie-break contract.
- `src/core/sketch/SnapManager.cpp`: overlap scoring/selection logic updates.
- `src/core/sketch/tools/SketchToolManager.cpp`: preview vs commit parity enforcement.
- `tests/prototypes/proto_sketch_snap.cpp`: overlap and parity regression tests.

### Definition of Done
- [x] Overlap scenarios produce same winner across repeated runs.
- [x] Point/endpoint wins over intersection when co-located.
- [x] Preview indicator candidate == commit candidate in covered overlap flows.
- [x] All targeted proto tests pass.

### Must Have
- Preserve baseline priority ladder from repo guidance.
- Deterministic tie-break independent of container insertion randomness.
- No regressions in existing snap types.

### Must NOT Have (Guardrails)
- No kernel/ElementMap/STEP/undo-redo work.
- No solver architecture redesign.
- No full QuickSnap UI system in this change set.
- No hidden behavior where guide preview contradicts commit target.

---

## Verification Strategy (MANDATORY)

> **ZERO HUMAN INTERVENTION**
>
> Every acceptance criterion is agent-executable only.

### Test Decision
- **Infrastructure exists**: YES
- **Automated tests**: TDD
- **Framework**: existing prototype executables via CMake

### If TDD Enabled
Each task adding behavior first adds/updates failing overlap tests in `proto_sketch_snap`, then implements minimal passing logic, then refactors.

### Agent-Executed QA Scenarios (MANDATORY)

Scenario: Axis crossing overlap deterministic winner
  Tool: Bash
  Preconditions: build configured
  Steps:
    1. `cmake --build build --target proto_sketch_snap`
    2. `./build/tests/proto_sketch_snap`
    3. Repeat overlap-target subset 20x (loop command)
  Expected Result: same winner and pass on every run
  Failure Indicators: intermittent FAIL or winner mismatch
  Evidence: terminal logs saved in `.sisyphus/evidence/task-qa-axis-overlap.log`

Scenario: Point/endpoint beats intersection when co-located
  Tool: Bash
  Preconditions: overlap tests added
  Steps:
    1. Run overlap-specific test binary command
    2. Assert expected snap type in test output/assertion
  Expected Result: point/endpoint selected
  Failure Indicators: intersection selected in co-located policy case
  Evidence: `.sisyphus/evidence/task-qa-policy-colocated.log`

Scenario: Preview/commit parity under overlap
  Tool: Bash
  Preconditions: parity tests added
  Steps:
    1. Run parity regression tests
    2. Assert previewCandidateId == committedCandidateId checks pass
  Expected Result: no parity mismatch
  Failure Indicators: mismatch assertion failure
  Evidence: `.sisyphus/evidence/task-qa-preview-commit.log`

---

## Execution Strategy

### Parallel Execution Waves

Wave 1 (parallel)
- Task 1: Specify arbitration contract + overlap matrix
- Task 2: Add RED tests for overlap determinism
- Task 3: Add RED tests for preview/commit parity

Wave 2 (sequential)
- Task 4: Implement SnapManager arbitration updates
- Task 5: Implement SketchToolManager parity enforcement

Wave 3 (sequential)
- Task 6: Optional minimal ambiguity fallback hook (defer full UI)
- Task 7: Full verification and anti-flake run

Critical Path: 1 -> 2 -> 4 -> 5 -> 7

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|---|---|---|---|
| 1 | None | 4,5 | 2,3 |
| 2 | 1 | 4 | 3 |
| 3 | 1 | 5 | 2 |
| 4 | 2 | 5,7 | None |
| 5 | 3,4 | 7 | None |
| 6 | 5 | 7 | None |
| 7 | 4,5 | None | None |

---

## TODOs

- [x] 1. Formalize overlap arbitration contract

  **What to do**:
  - Define explicit ranking for co-located candidates.
  - Define tie-break sequence and epsilon policy.
  - Define preview/commit parity rule.

  **Must NOT do**:
  - Do not change global priority semantics beyond explicit overlap policy.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: behavior contract with multiple edge cases
  - **Skills**: `sketch-tool`, `constraint`
    - `sketch-tool`: snap/tool flow knowledge
    - `constraint`: interaction side effects awareness

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1
  - **Blocks**: 2,3,4,5
  - **Blocked By**: None

  **References**:
  - `src/core/sketch/SnapManager.cpp` - current candidate generation/sort path
  - `src/core/sketch/SnapManager.h` - comparator contract
  - `src/core/sketch/tools/SketchToolManager.cpp` - preview/commit routing
  - `docs/SPECIFICATION.md` - behavior baseline expectations

  **Acceptance Criteria**:
  - [x] Contract document section added in plan/task notes and unambiguous.
  - [x] Co-located rule explicitly states point/endpoint > intersection.

- [x] 2. RED tests: overlap determinism matrix

  **What to do**:
  - Add failing tests for axis crossing overlap.
  - Add failing tests for point+intersection overlap.
  - Add repeated-run determinism assertion hooks.

  **Must NOT do**:
  - Do not implement fixes before RED assertions fail.

  **Recommended Agent Profile**:
  - **Category**: `tests`
  - **Skills**: `proto-test`, `sketch-tool`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1
  - **Blocks**: 4
  - **Blocked By**: 1

  **References**:
  - `tests/prototypes/proto_sketch_snap.cpp` - existing snap tests
  - `src/core/sketch/SnapManager.cpp` - candidates under test

  **Acceptance Criteria**:
  - [x] New overlap tests fail before implementation.
  - [x] Tests include co-located point/endpoint + intersection case.

- [x] 3. RED tests: preview/commit parity in overlap

  **What to do**:
  - Add failing test coverage for move-preview selected candidate vs commit candidate.
  - Cover guide-adjacent overlap case.

  **Must NOT do**:
  - Do not rely on manual viewport checks.

  **Recommended Agent Profile**:
  - **Category**: `tests`
  - **Skills**: `proto-test`, `sketch-tool`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1
  - **Blocks**: 5
  - **Blocked By**: 1

  **References**:
  - `src/core/sketch/tools/SketchToolManager.cpp` - effective snap paths
  - `src/core/sketch/tools/LineTool.cpp` - commit snap consumption

  **Acceptance Criteria**:
  - [x] New parity tests fail before implementation.

- [x] 4. Implement deterministic overlap arbitration in SnapManager

  **What to do**:
  - Add explicit overlap-aware scoring and stable tie-break.
  - Preserve baseline priority for non-overlap situations.

  **Must NOT do**:
  - Do not reorder unrelated snap type semantics globally.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `sketch-tool`, `constraint`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Sequential Wave 2
  - **Blocks**: 5,7
  - **Blocked By**: 2

  **References**:
  - `src/core/sketch/SnapManager.h`
  - `src/core/sketch/SnapManager.cpp`

  **Acceptance Criteria**:
  - [x] Overlap determinism tests pass.
  - [x] Co-located policy enforced.

- [x] 5. Enforce preview/commit parity in SketchToolManager

  **What to do**:
  - Use one canonical selected candidate identity for preview and commit where policy requires parity.
  - Keep guides advisory unless explicitly selected by arbitration.

  **Must NOT do**:
  - Do not reintroduce ghost-preview/commit mismatch.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `sketch-tool`, `qt-widget`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Sequential Wave 2
  - **Blocks**: 7
  - **Blocked By**: 3,4

  **References**:
  - `src/core/sketch/tools/SketchToolManager.cpp`
  - `src/core/sketch/tools/LineTool.cpp`

  **Acceptance Criteria**:
  - [x] Parity tests pass.
  - [x] No existing snap regression tests break.

- [x] 6. Add minimal ambiguity fallback hook (deferred UX)

  **What to do**:
  - Add internal hook/state for candidate cycling/lock future support without exposing full UI now.
  - Ensure default behavior remains deterministic and unchanged for normal cases.

  **Must NOT do**:
  - Do not implement full hotkey/UI workflow in this plan.

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `sketch-tool`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3
  - **Blocks**: 7
  - **Blocked By**: 5

  **References**:
  - `src/core/sketch/SnapManager.h`
  - `src/core/sketch/tools/SketchToolManager.cpp`

  **Acceptance Criteria**:
  - [x] Hook present and covered by unit/proto assertion where possible.

- [x] 7. Full verification + anti-flake pass

  **What to do**:
  - Build and run relevant prototypes.
  - Run overlap subset repeatedly to catch nondeterminism.
  - Record evidence.

  **Recommended Agent Profile**:
  - **Category**: `tests`
  - **Skills**: `proto-test`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Final
  - **Blocks**: None
  - **Blocked By**: 4,5,6

  **Acceptance Criteria**:
  - [x] `cmake --build build --target proto_sketch_snap proto_sketch_solver proto_sketch_geometry proto_sketch_constraints`
  - [x] `./build/tests/proto_sketch_snap` passes
  - [x] overlap-focused subset repeated 20x without failure
  - [x] evidence files saved in `.sisyphus/evidence/`

---

## Commit Strategy

| After Task | Message | Files | Verification |
|---|---|---|---|
| 2-3 | `test(sketch): add overlap arbitration regression matrix` | proto tests | proto_sketch_snap fails as expected |
| 4-5 | `fix(sketch): stabilize overlap snap arbitration and parity` | SnapManager, ToolManager | proto_sketch_snap passes |
| 6-7 | `chore(sketch): add ambiguity hook and anti-flake verification` | core/test touchpoints | repeated runs pass |

---

## Success Criteria

### Verification Commands
```bash
cmake --build build --target proto_sketch_snap proto_sketch_solver proto_sketch_geometry proto_sketch_constraints
./build/tests/proto_sketch_snap
./build/tests/proto_sketch_solver
./build/tests/proto_sketch_geometry
./build/tests/proto_sketch_constraints
```

### Final Checklist
- [x] Overlap arbitration deterministic
- [x] Co-located policy enforced (point/endpoint > intersection)
- [x] Preview/commit parity verified
- [x] No regressions in existing snap tests
- [x] Scope guardrails respected

---

## Unresolved Questions
- Ambiguity fallback default: stick-last vs explicit no-snap indicator?
- Pixel aperture weighting vs pure world-space weighting?
- Tab-cycle/lock UX now or strictly deferred?
