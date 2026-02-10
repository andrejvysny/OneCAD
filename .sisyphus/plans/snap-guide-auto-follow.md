# Snap Guide Auto-Follow + Infinite Guide Visuals

## TL;DR

> **Quick Summary**: Unify snapping and guide rendering so any active rendered guide directly drives current draw position (guide-first always), and render guides as viewport-clipped long lines/rays (CAD-like) instead of short segments.
>
> **Deliverables**:
> - One authoritative effective-snap selection path in `SketchToolManager`
> - Guide-first snapping policy applied to active drawing flow
> - Infinite-like guide rendering via viewport clipping/projection
> - New prototype tests for effective-snap policy and guide visual data behavior
>
> **Estimated Effort**: Medium
> **Parallel Execution**: YES - 2 waves
> **Critical Path**: Task 1 -> Task 2 -> Task 4 -> Task 5

---

## Context

### Original Request
Guides now render, but current cursor/draw line does not auto-snap to them. User wants CAD-like behavior: whenever guide is rendered, current drawing follows it. Also requested thorough review/optimization of autosnap behavior without scope creep.

### Interview Summary
**Key Decisions**:
- Guide precedence: **Guide-first always** when guide is active.
- Keep work scoped to snap pipeline + guide visuals + verification.
- No unrelated UI/settings expansions.

**Research Findings**:
- `handleMouseMove` currently splits logic: `findBestSnap` for tool position and `findAllSnaps` for guide rendering.
- This decoupling causes visual guides to exist without affecting `snappedPos`.
- `SnapManager::findBestSnap` remains enum-priority based.
- Existing tests are strong for SnapManager logic (44 tests), weak for effective-snap integration policy.

### Metis Review
**Addressed Gaps**:
- Need single effective-snap object consumed by both tool and indicator.
- Need explicit multi-guide conflict rule (default below).
- Need render-only infinite-guide projection (no geometry semantic change).
- Need precedence regression tests (guide/no-guide/multi-guide).

---

## Work Objectives

### Core Objective
Ensure rendered guides are not decorative: they become authoritative snapping input for active drawing, and their visual presentation behaves like long/infinite CAD guide lines.

### Concrete Deliverables
- `src/core/sketch/tools/SketchToolManager.cpp`: unified effective snap selection and guide-first policy.
- `src/core/sketch/SketchRenderer.cpp` (+ small API extension if needed): long/infinite guide rendering via viewport-clipped lines/rays.
- `tests/prototypes/proto_sketch_snap.cpp`: policy/behavior tests for guide-first auto-follow.

### Definition of Done
- [x] `cmake --build build` exits 0
- [x] `./build/tests/proto_sketch_snap` exits 0 with all tests passing
- [x] `./build/tests/proto_sketch_geometry` exits 0
- [x] `./build/tests/proto_sketch_constraints` exits 0
- [x] `./build/tests/test_compile` exits 0

### Must Have
- Active tool position follows guide when guide is rendered (guide-first always).
- Winner indicator/hint and tool snapped position use same effective snap decision.
- Guide visuals render as long/infinite-like lines/rays clipped to viewport bounds.
- No regression when no guide is active (existing point/grid snaps behave as before).

### Must NOT Have (Guardrails)
- Do NOT change SnapType enum ordering or global priority model in `SnapManager`.
- Do NOT alter snap radius/threshold constants globally.
- Do NOT touch 3D snapping (`ActiveLayer3D`) behavior.
- Do NOT add new settings UI/toggles.
- Do NOT change solver/constraint semantics (PlaneGCS integration unaffected).
- Do NOT touch ElementMap/Kernel behavior (no topological naming impact expected).

### OneCAD Context Checks
- **Layer impact**: UI -> Render -> Core touched; Kernel untouched.
- **ElementMap impact**: none expected.
- **PlaneGCS impact**: none expected (snap preview only).
- **Coordinate system implications**: preserve sketch-space math (Sketch X->World Y+, Sketch Y->World X- mapping unchanged).
- **STEP I/O + undo/redo note**: no additional wiring required/expected from this change.

---

## Verification Strategy

> **UNIVERSAL RULE: ZERO HUMAN INTERVENTION**
>
> Every acceptance criterion must be agent-executable via commands/tests.

### Test Decision
- **Infrastructure exists**: YES
- **Automated tests**: Tests-after (extend existing proto tests)
- **Framework**: CMake prototype executables

### Agent-Executed QA Scenarios (MANDATORY)

Scenario: Effective snap follows active guide
  Tool: Bash (`proto_sketch_snap`)
  Preconditions: New policy tests added
  Steps:
    1. Build `proto_sketch_snap`
    2. Run test case validating guide-first selected position
    3. Assert test PASS
  Expected Result: Guide candidate drives effective snapped position when rendered
  Evidence: Test output lines in terminal

Scenario: No-guide fallback unchanged
  Tool: Bash (`proto_sketch_snap`)
  Preconditions: Existing baseline tests remain
  Steps:
    1. Run full `proto_sketch_snap`
    2. Assert existing baseline snap tests still PASS
  Expected Result: Non-guide behavior preserved
  Evidence: Full pass count output

Scenario: Renderer compiles with infinite-guide projection
  Tool: Bash (`test_compile`)
  Preconditions: Renderer updates integrated
  Steps:
    1. Build `test_compile`
    2. Run executable
    3. Assert exit code 0
  Expected Result: Render path compiles and links
  Evidence: exit code and output

---

## Execution Strategy

### Parallel Execution Waves

Wave 1 (parallel):
- Task 1: Effective snap resolver in SketchToolManager
- Task 3: Add/extend unit tests for guide-first selection behavior

Wave 2 (sequential):
- Task 2: Infinite-guide rendering projection/clipping
- Task 4: Integrate + align tests with renderer-facing guide data assumptions

Final:
- Task 5: Full build/test verification

Critical Path: 1 -> 2 -> 4 -> 5

---

## TODOs

- [x] 1. Introduce unified effective-snap policy in `SketchToolManager`

  **What to do**:
  - Add local policy function in `handleMouseMove` to derive `effectiveSnap` from:
    - winner (`findBestSnap`)
    - guide candidates from `findAllSnaps`
  - Implement **guide-first always**:
    - if any valid guide candidates exist, choose guide-effective result
    - otherwise use winner
  - Ensure `effectiveSnap` is used for:
    - `activeTool_->setSnapResult(...)`
    - `snappedPos` passed to `onMouseMove`
    - indicator state used by render path
  - Keep deterministic multi-guide tie-break default:
    - choose nearest guide candidate by `distance`
    - tie -> stable order from `findAllSnaps`

  **Must NOT do**:
  - Do not modify `SnapManager::findBestSnap` ranking logic.
  - Do not modify tool state machine structure.

  **Recommended Agent Profile**:
  - Category: `quick`
  - Skills: []

  **Parallelization**:
  - Can Run In Parallel: YES (with Task 3)
  - Blocks: 2, 4, 5
  - Blocked By: None

  **References**:
  - `src/core/sketch/tools/SketchToolManager.cpp` - current split paths in `handleMouseMove`
  - `src/core/sketch/SnapManager.h` - `SnapResult` structure contract
  - `src/core/sketch/SnapManager.cpp` - `findBestSnap` + `findAllSnaps` semantics

  **Acceptance Criteria**:
  - [x] `handleMouseMove` computes one `effectiveSnap`
  - [x] guide-active case updates tool snapped position from guide
  - [x] no-guide case identical to current behavior

- [x] 2. Render guides as viewport-clipped long/infinite-like lines

  **What to do**:
  - Convert guide rendering from strict `{origin,target}` short segment to projected long line/ray endpoints clipped by current viewport bounds.
  - Preserve guide color/style and existing VBO path (`appendDashedPolyline`, existing VAO/VBO).
  - Keep math in sketch-space coordinates; do not alter world mapping code.

  **Must NOT do**:
  - No new shaders.
  - No changes to point snap geometry computations.

  **Recommended Agent Profile**:
  - Category: `quick`
  - Skills: [`render-gl`]

  **Parallelization**:
  - Can Run In Parallel: NO
  - Blocks: 4, 5
  - Blocked By: 1

  **References**:
  - `src/core/sketch/SketchRenderer.cpp` - guide line VBO build path
  - `src/core/sketch/SketchRenderer.h` - `GuideLineInfo` and renderer state

  **Acceptance Criteria**:
  - [x] guide visuals extend across meaningful viewport span
  - [x] dashed rendering path unchanged except endpoints
  - [x] build and compile checks pass

- [x] 3. Add guide-first effective-snap tests (`proto_sketch_snap`)

  **What to do**:
  - Add deterministic tests for policy behavior at SnapManager/selection-layer seam:
    1. `test_effective_snap_prefers_guide_when_present`
    2. `test_effective_snap_falls_back_when_no_guide`
    3. `test_effective_snap_nearest_guide_tiebreak`
  - Reuse existing helper patterns (`TestResult`, `approx`, table registration).

  **Must NOT do**:
  - Do not remove or weaken existing tests.

  **Recommended Agent Profile**:
  - Category: `unspecified-low`
  - Skills: [`proto-test`]

  **Parallelization**:
  - Can Run In Parallel: YES (with Task 1 scaffolding)
  - Blocks: 4, 5
  - Blocked By: None

  **References**:
  - `tests/prototypes/proto_sketch_snap.cpp` - current 44 tests + registration pattern

  **Acceptance Criteria**:
  - [x] new guide-first policy tests present
  - [x] full proto snap suite passes

- [x] 4. Align integration behavior + regression hardening

  **What to do**:
  - Ensure press/release paths do not diverge from move-path policy where relevant for line creation flow.
  - Ensure rendered hint text/winner marker corresponds to effective snap used by tool.
  - Add/adjust tests to lock this invariant.

  **Must NOT do**:
  - No behavior changes outside sketch snap interaction.

  **Recommended Agent Profile**:
  - Category: `quick`
  - Skills: []

  **Parallelization**:
  - Can Run In Parallel: NO
  - Blocks: 5
  - Blocked By: 1, 2, 3

  **References**:
  - `src/core/sketch/tools/SketchToolManager.cpp`
  - `src/core/sketch/SketchRenderer.cpp`
  - `tests/prototypes/proto_sketch_snap.cpp`

  **Acceptance Criteria**:
  - [x] effective snap and indicator data remain consistent
  - [x] no stale guide states on no-snap transitions

- [x] 5. Final verification and performance sanity

  **What to do**:
  - Run final command suite:
    - `cmake --build build`
    - `./build/tests/proto_sketch_snap`
    - `./build/tests/proto_sketch_geometry`
    - `./build/tests/proto_sketch_constraints`
    - `./build/tests/test_compile`
  - Confirm no new warnings/errors in touched files.

  **Recommended Agent Profile**:
  - Category: `quick`
  - Skills: []

  **Parallelization**:
  - Can Run In Parallel: NO
  - Blocked By: 1-4

  **Acceptance Criteria**:
  - [x] all listed commands exit 0
  - [x] no regressions in existing snap suite

---

## Commit Strategy

| After Task | Message | Files | Verification |
|---|---|---|---|
| 1 | `fix(snap): unify effective snap with guide-first policy` | `SketchToolManager.cpp` | `proto_sketch_snap` |
| 2 | `fix(render): project snap guides as viewport-clipped long lines` | `SketchRenderer.cpp` (+ header if needed) | `test_compile` |
| 3-4 | `test(snap): add guide-first effective snap regression cases` | `proto_sketch_snap.cpp` | `proto_sketch_snap` |

---

## Success Criteria

### Verification Commands
```bash
cmake --build build
./build/tests/proto_sketch_snap
./build/tests/proto_sketch_geometry
./build/tests/proto_sketch_constraints
./build/tests/test_compile
```

### Final Checklist
- [x] Guide rendered => active draw position follows guide
- [x] Multi-guide conflict resolves deterministically (nearest guide)
- [x] Guide visuals look infinite-like (viewport-clipped long lines/rays)
- [x] No regression for non-guide snapping flows
- [x] All proto/build checks pass

---

## Defaults Applied (override if needed)
- Multi-guide conflict default: nearest guide candidate wins.
- Infinite guide implementation default: viewport-clipped long line/ray projection.
- Optimization scope default: only snap decision path + guide render endpoint generation (no architecture rewrite).

## Unresolved Questions
- none
