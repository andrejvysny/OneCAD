# Fix Visual Guide Rendering + Crossing Snap Bugs

## TL;DR

> **Quick Summary**: Fix 3 interrelated bugs in sketch visual guide system: dashed rendering appearing solid, missing crosshair at guide crossings, and guide-guide crossing snap not winning arbitration.
>
> **Deliverables**:
> - Visible dashed guide lines at all zoom levels (fixed 8px/6px screen-space pattern)
> - Crosshair marker rendered whenever ≥2 non-collinear active guides intersect
> - Guide-guide intersection snap candidate properly generated and winning when appropriate
> - Expanded proto_sketch_snap tests covering H/V guide crossing scenarios
>
> **Estimated Effort**: Short
> **Parallel Execution**: YES — 2 waves
> **Critical Path**: Task 1 → Task 3 → Task 5

---

## Context

### Original Request
Fix 3 visual guide bugs identified from screenshot analysis:
1. Guide lines render SOLID instead of DASHED
2. No crosshair marker at guide line crossings
3. Cursor doesn't auto-snap to guide line crossings

### Interview Summary
**Key Decisions**:
- Vertex/Endpoint always wins over Intersection at same location (preserve contract)
- Standard 2mm snap radius for crossing detection (no expansion)
- Crosshair renders whenever ≥2 non-collinear active guides intersect (regardless of resolved snap type)
- Fixed screen-space dash pattern: 8px dash, 6px gap

**Root Cause Analysis**:

**Bug 1 (solid guides)**: `guideDash = 1.5 * pixelScale`, `guideGap = 3.0 * pixelScale` at `SketchRenderer.cpp:1005-1006`. At typical zoom, pixelScale ≈ 0.05-0.2mm/px → dash = 1.5px, gap = 3px → sub-pixel pattern appears solid.

**Bug 2+3 (crossing detection/crosshair)**: H/V snaps DO set `hasGuide=true` (confirmed at `SnapManager.cpp:1178` and `~1305`). The guide-guide intersection loop at `SnapManager.cpp:241-299` DOES process them. The issues are:
- (a) The intersection candidate's position may be > 2mm from cursor even when cursor is visually near the crossing of two guide LINES (because the guide lines extend to viewport edge but intersection is tested against cursor radius)
- (b) `applyGuideFirstSnapPolicy` at `SketchToolManager.cpp:29-31` picks closest guide-bearing snap by distance — H or V may be closer than the intersection
- (c) Crosshair render at `SketchRenderer.cpp:1105` requires `snapType == Intersection && snapHasGuide` — if H/V wins, no crosshair
- (d) The crosshair should instead be computed from active guides geometry, independent of snap type

### Metis Review
**Identified Gaps** (addressed):
- Snap precedence regression risk → guardrails: preserve SnapType enum order, preserve Vertex/Endpoint protection in applyGuideFirstSnapPolicy
- Renderer-only fix could mask logic bug → paired visual + commit-path tests
- Near-parallel guide edge case (cross < 1e-12 in infiniteLineIntersection) → add explicit test
- Existing test `test_guide_crossing_snaps_to_intersection` only tests SketchGuide type, not H/V → extend coverage

---

## Work Objectives

### Core Objective
Fix visual guide rendering and crossing snap behavior so that: dashed patterns are visible at all zoom levels, guide crossings show a crosshair marker, and the cursor correctly snaps to guide-guide intersection points.

### Concrete Deliverables
- Modified `src/core/sketch/SketchRenderer.cpp` (dash params + crosshair logic)
- Modified `src/core/sketch/SnapManager.cpp` (crossing detection robustness — if needed)
- Modified `src/core/sketch/tools/SketchToolManager.cpp` (guide policy — if needed)
- Extended `tests/prototypes/proto_sketch_snap.cpp` (new crossing tests)

### Definition of Done
- [x] `cmake --build build && ./build/tests/proto_sketch_snap` → all tests pass including new ones
- [x] `cmake --build build --target test_compile` → exits 0
- [x] Guide lines are dashed at all zoom levels (verified by code review of dash params)
- [x] Crosshair appears at guide crossings when ≥2 guides are active (verified by test + code path)

### Must Have
- Visible dashed pattern on all guide lines at any zoom
- Crosshair at intersection of ≥2 non-collinear active guides
- H+V guide crossing produces `SnapType::Intersection` candidate in `findAllSnaps`
- Intersection candidate wins over H/V in `applyGuideFirstSnapPolicy` when co-located
- No regression in existing 57 proto_sketch_snap tests

### Must NOT Have (Guardrails)
- Do NOT change SnapType enum ordering in `SnapManager.h:38`
- Do NOT change `operator<` comparator in `SnapResult` (`SnapManager.h:97`)
- Do NOT alter Vertex/Endpoint protection in `applyGuideFirstSnapPolicy` (line 22-25 stays)
- Do NOT change snap radius (keep 2mm)
- Do NOT touch spatial hash, ambiguity cycling, or external 3D snaps
- Do NOT add new settings, UI toggles, or theme changes
- Do NOT refactor unrelated snap subsystems (Perpendicular, Tangent, OnCurve)
- No AI-slop: no premature abstractions, no over-validation, no documentation bloat

---

## Verification Strategy

> **UNIVERSAL RULE: ZERO HUMAN INTERVENTION**
>
> ALL tasks verifiable WITHOUT human action. Agent runs build + test commands.

### Test Decision
- **Infrastructure exists**: YES (proto_sketch_snap, 57 tests)
- **Automated tests**: YES (tests-after, extend existing proto)
- **Framework**: proto executables via CMake, run from build dir

### Agent-Executed QA Scenarios (MANDATORY — ALL tasks)

Verification via:
```bash
cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap
# Assert: exit code 0, all tests pass including new ones
cmake --build build --target test_compile
# Assert: exit code 0
```

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Start Immediately):
├── Task 1: Fix dash pattern (SketchRenderer.cpp only)
└── Task 2: Add crosshair-from-active-guides logic (SketchRenderer.cpp only)

Wave 2 (After Wave 1):
├── Task 3: Fix guide crossing snap priority (SnapManager + SketchToolManager)
└── Task 4: Add proto_sketch_snap tests for H/V crossing

Wave 3 (After Wave 2):
└── Task 5: Integration verification + regression check
```

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|------|------------|--------|---------------------|
| 1 | None | 5 | 2 |
| 2 | None | 5 | 1 |
| 3 | None | 4, 5 | (sequential with 4 for test alignment) |
| 4 | 3 | 5 | — |
| 5 | 1, 2, 3, 4 | None | — |

### Agent Dispatch Summary

| Wave | Tasks | Recommended |
|------|-------|-------------|
| 1 | 1, 2 | Both in SketchRenderer.cpp — can be done by single agent sequentially to avoid merge conflicts |
| 2 | 3, 4 | Sequential (test 4 depends on logic 3) |
| 3 | 5 | Final build + run all tests |

---

## TODOs

- [x] 1. Fix guide line dash pattern to be visible at all zoom levels

  **What to do**:
  - In `SketchRenderer.cpp` at line 1005-1006, replace:
    ```cpp
    const double guideDash = 1.5 * std::max(pixelScale, 1e-9);
    const double guideGap = 3.0 * std::max(pixelScale, 1e-9);
    ```
    with fixed screen-space pattern:
    ```cpp
    const double guideDash = 8.0 * std::max(pixelScale, 1e-9);
    const double guideGap = 6.0 * std::max(pixelScale, 1e-9);
    ```
  - `pixelScale` is mm-per-pixel, so `8.0 * pixelScale` = 8 screen pixels of dash, `6.0 * pixelScale` = 6 screen pixels of gap
  - Verify that `appendDashedPolyline` at line 182-230 correctly handles these values (it does — no changes needed there)

  **Must NOT do**:
  - Do not change `appendDashedPolyline` logic
  - Do not change other dash patterns in the file (only guide lines)
  - Do not add zoom-adaptive logic — keep it simple fixed screen-space

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: [`render-gl`]
    - `render-gl`: OpenGL rendering patterns, viewport/sketch renderer context

  **Parallelization**:
  - **Can Run In Parallel**: YES (but same file as Task 2 — recommend sequential by same agent)
  - **Parallel Group**: Wave 1
  - **Blocks**: Task 5
  - **Blocked By**: None

  **References**:

  **Pattern References**:
  - `src/core/sketch/SketchRenderer.cpp:1005-1006` — Current `guideDash`/`guideGap` computation (THE lines to change)
  - `src/core/sketch/SketchRenderer.cpp:1091-1092` — Where `appendDashedPolyline` is called for multi-guide path
  - `src/core/sketch/SketchRenderer.cpp:1100-1101` — Where `appendDashedPolyline` is called for single-guide fallback
  - `src/core/sketch/SketchRenderer.cpp:182-230` — `appendDashedPolyline` implementation (read-only reference, no changes)

  **API/Type References**:
  - `src/core/sketch/SketchRenderer.cpp:968` — `pixelScale` variable definition (sketch coords per pixel)

  **Acceptance Criteria**:

  > AGENT-EXECUTABLE VERIFICATION ONLY

  - [ ] `guideDash` changed from `1.5 *` to `8.0 *` at line 1005
  - [ ] `guideGap` changed from `3.0 *` to `6.0 *` at line 1006
  - [ ] `cmake --build build --target test_compile` → exit 0

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Build succeeds after dash parameter change
    Tool: Bash
    Preconditions: Build dir exists at ./build
    Steps:
      1. cmake --build build --target test_compile
      2. Assert: exit code 0
    Expected Result: Compiles without errors
    Evidence: Build output captured
  ```

  **Commit**: YES (groups with Task 2)
  - Message: `fix(sketch): increase guide line dash/gap for visible dashed rendering`
  - Files: `src/core/sketch/SketchRenderer.cpp`
  - Pre-commit: `cmake --build build --target test_compile`


- [x] 2. Render crosshair at active guide intersections (independent of snap type)

  **What to do**:
  - In `SketchRenderer.cpp` at line 1105, the current crosshair logic is:
    ```cpp
    if (snapType == SnapType::Intersection && snapHasGuide) {
    ```
    Change to: compute crosshair from `activeGuides` geometry whenever ≥2 non-collinear guides intersect near the snap position, regardless of snap type.
  - New logic (insert BEFORE existing crosshair block or replace it):
    1. If `activeGuides.size() >= 2`, iterate all pairs of active guides
    2. For each pair, compute intersection using same `infiniteLineIntersection` formula (inline or duplicate the math — it's 10 lines)
    3. Check if intersection is within viewport bounds
    4. Render crosshair at that intersection point
    5. Use `crossHalf = 5.0 * std::max(pixelScale, 1e-9)` for visible screen-space cross (5px each arm)
    6. Keep existing crosshair color logic (`crossColor`)
  - Also keep the existing `snapType == Intersection && snapHasGuide` crosshair as a fallback for when intersection snap wins but activeGuides are empty (single-guide scenario from `findGuideSnaps`)

  **Must NOT do**:
  - Do not add more than 4 guide crosshairs (cap at 4 guides already enforced at line 1086)
  - Do not change guide line rendering logic (only add crosshair computation)
  - Do not change snap indicator point rendering

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: [`render-gl`]
    - `render-gl`: OpenGL rendering, guide line VBO data, viewport math

  **Parallelization**:
  - **Can Run In Parallel**: YES (same file as Task 1 — recommend sequential by same agent)
  - **Parallel Group**: Wave 1
  - **Blocks**: Task 5
  - **Blocked By**: None

  **References**:

  **Pattern References**:
  - `src/core/sketch/SketchRenderer.cpp:1105-1118` — Current crosshair rendering code (TO MODIFY)
  - `src/core/sketch/SketchRenderer.cpp:1083-1094` — activeGuides iteration pattern (follow for pair iteration)
  - `src/core/sketch/SketchRenderer.cpp:1109` — crossHalf definition pattern (adjust to 5.0)
  - `src/core/sketch/SketchRenderer.cpp:1025-1067` — `clipGuideRayToViewport` lambda (viewport bounds reference)
  - `src/core/sketch/SketchRenderer.h:381-384` — `GuideLineInfo` struct (origin + target)

  **API/Type References**:
  - `src/core/sketch/SnapManager.cpp:33-52` — `infiniteLineIntersection` formula to duplicate inline (p1,p2,p3,p4 → optional Vec2d)
  - `src/core/sketch/SketchRenderer.cpp:1012-1013` — `kGuideColor`, `kGuideColorFaint` constants

  **Acceptance Criteria**:

  > AGENT-EXECUTABLE VERIFICATION ONLY

  - [ ] Crosshair rendered when `activeGuides.size() >= 2` and pairs have non-parallel directions
  - [ ] Crosshair at computed intersection point (not at snap position)
  - [ ] Existing `snapType == Intersection` crosshair path still works as fallback
  - [ ] `cmake --build build --target test_compile` → exit 0

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Build succeeds after crosshair logic change
    Tool: Bash
    Preconditions: Build dir exists at ./build
    Steps:
      1. cmake --build build --target test_compile
      2. Assert: exit code 0
    Expected Result: Compiles without errors
    Evidence: Build output captured
  ```

  **Commit**: YES (groups with Task 1)
  - Message: `fix(sketch): render crosshair at active guide intersections regardless of snap type`
  - Files: `src/core/sketch/SketchRenderer.cpp`
  - Pre-commit: `cmake --build build --target test_compile`


- [x] 3. Fix guide crossing snap to win over individual H/V when cursor is near crossing

  **What to do**:
  The guide-guide intersection candidate IS already generated by `findAllSnaps()` at `SnapManager.cpp:267-298`. The issue is that `applyGuideFirstSnapPolicy` picks the CLOSEST guide-bearing snap by distance, and H or V may individually be closer to cursor than the intersection point.

  Fix in `applyGuideFirstSnapPolicy` at `SketchToolManager.cpp:20-35`:
  - After the existing guard for Vertex/Endpoint (line 22-25), add a NEW check:
  - Scan `allSnaps` for any `SnapType::Intersection` candidate that has `hasGuide = true`
  - If found, check if it's within snap radius AND if the corresponding H and V guide candidates also exist (verifying this is a guide-guide crossing, not a geometry intersection)
  - If yes, prefer the Intersection candidate over individual H/V guides
  - Implementation approach:
    ```cpp
    // After Vertex/Endpoint guard...
    // Prefer guide-guide Intersection over individual H/V when crossing exists
    for (const auto& snap : allSnaps) {
        if (snap.snapped && snap.type == SnapType::Intersection && snap.hasGuide) {
            return snap;  // Guide crossing wins over individual guides
        }
    }
    // Then fall through to existing nearest-guide logic
    ```

  **Must NOT do**:
  - Do NOT change the Vertex/Endpoint early-return at line 22-25
  - Do NOT change SnapType enum ordering
  - Do NOT change `operator<` comparator
  - Do NOT change `findAllSnaps()` logic (intersection generation is already correct)
  - Do NOT change snap radius

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO — Task 4 tests depend on this
  - **Parallel Group**: Wave 2 (sequential with Task 4)
  - **Blocks**: Task 4, Task 5
  - **Blocked By**: None (logically independent of Wave 1 but recommend Wave 2 for clean testing)

  **References**:

  **Pattern References**:
  - `src/core/sketch/tools/SketchToolManager.cpp:20-35` — `applyGuideFirstSnapPolicy` (THE function to modify)
  - `src/core/sketch/tools/SketchToolManager.cpp:62-103` — `resolveSnapForInputEvent` (caller context, understand flow)
  - `src/core/sketch/tools/SketchToolManager.cpp:256-318` — `handleMouseMove` guide extraction (understand how allSnaps become activeGuides)

  **API/Type References**:
  - `src/core/sketch/SnapManager.h:38-54` — SnapType enum (Intersection = 6, Horizontal = 11, Vertical = 12)
  - `src/core/sketch/SnapManager.h:59-137` — SnapResult struct and operator< contract
  - `src/core/sketch/SnapManager.cpp:267-298` — Guide-guide intersection generation in findAllSnaps (produces the candidate we want to prefer)

  **Test References**:
  - `tests/prototypes/proto_sketch_snap.cpp:1260-1295` — `test_guide_crossing_snaps_to_intersection` (existing test for SketchGuide crossing; our new test will cover H/V crossing)

  **Acceptance Criteria**:

  > AGENT-EXECUTABLE VERIFICATION ONLY

  - [ ] `applyGuideFirstSnapPolicy` prefers `Intersection` with `hasGuide=true` over individual H/V
  - [ ] Vertex/Endpoint early-return unchanged
  - [ ] `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap` → all 57 existing tests pass
  - [ ] `cmake --build build --target test_compile` → exit 0

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Existing tests pass after policy change
    Tool: Bash
    Preconditions: Build dir exists at ./build
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. Assert: build exit code 0
      3. ./build/tests/proto_sketch_snap
      4. Assert: exit code 0, all 57 tests pass
    Expected Result: No regression in existing snap tests
    Evidence: Test output captured

  Scenario: Compile check passes
    Tool: Bash
    Steps:
      1. cmake --build build --target test_compile
      2. Assert: exit code 0
    Expected Result: Full build succeeds
    Evidence: Build output captured
  ```

  **Commit**: YES (groups with Task 4)
  - Message: `fix(sketch): prefer guide-guide intersection over individual H/V in snap policy`
  - Files: `src/core/sketch/tools/SketchToolManager.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`


- [x] 4. Add proto_sketch_snap tests for H/V guide crossing scenarios

  **What to do**:
  Add 3-4 new test functions to `tests/prototypes/proto_sketch_snap.cpp`:

  1. **`test_hv_guide_crossing_produces_intersection_candidate`**:
     - Setup: Sketch with point at (5,0) and point at (0,3). Enable H, V, SketchGuide, Intersection snap types.
     - Query at (5.1, 3.1) — near the crossing of horizontal-from-(5,0) and vertical-from-(0,3)
     - Assert: `findAllSnaps` contains a candidate with `type == Intersection`, `position ≈ (5,3)`, `hasGuide == true`

  2. **`test_hv_guide_crossing_wins_over_individual_hv`**:
     - Same setup as above
     - Query at (5.1, 3.1)
     - Use `findBestSnap` or replicate guide-first policy logic
     - Assert: best snap is `Intersection` at (5,3), not `Horizontal` or `Vertical`

  3. **`test_hv_guide_crossing_loses_to_vertex`**:
     - Setup: Sketch with point at (5,3) (exactly at crossing), plus other points that create H/V guides crossing at (5,3)
     - Query at (5.0, 3.0)
     - Assert: best snap is `Vertex` (not `Intersection`) — preserves contract

  4. **`test_near_parallel_guides_no_spurious_intersection`**:
     - Setup: Two points with nearly the same Y (differ by 1e-13) — their horizontal guides are near-parallel
     - Query between them
     - Assert: no spurious intersection candidate with huge coordinates

  Register all new tests in the test runner array.

  **Must NOT do**:
  - Do not modify existing tests
  - Do not change test infrastructure or framework
  - Do not add renderer tests (proto_sketch_snap is snap-logic only)

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: [`proto-test`]
    - `proto-test`: OneCAD prototype test patterns, test registration, assertion patterns

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on Task 3 logic changes
  - **Parallel Group**: Wave 2 (after Task 3)
  - **Blocks**: Task 5
  - **Blocked By**: Task 3

  **References**:

  **Pattern References**:
  - `tests/prototypes/proto_sketch_snap.cpp:1260-1295` — `test_guide_crossing_snaps_to_intersection` (existing crossing test pattern to follow: setup sketch, create manager with specific types, findAllSnaps, assert intersection candidate, findBestSnap, assert winner)
  - `tests/prototypes/proto_sketch_snap.cpp:1297-1324` — `test_parity_findBestSnap_stable_across_calls` (example of stability/determinism test)

  **API/Type References**:
  - `tests/prototypes/proto_sketch_snap.cpp:1265` — `createSnapManagerFor({types...})` helper usage pattern
  - `tests/prototypes/proto_sketch_snap.cpp:1272` — `approx(val, expected, tolerance)` helper usage
  - `tests/prototypes/proto_sketch_snap.cpp:1271` — Pattern for iterating `allSnaps` and checking specific candidates
  - Look for `RUN_TEST` or similar macro at bottom of file to register new tests

  **Test References**:
  - `tests/prototypes/proto_sketch_snap.cpp` — End of file for test registration array/runner

  **Acceptance Criteria**:

  > AGENT-EXECUTABLE VERIFICATION ONLY

  - [ ] 3-4 new test functions added
  - [ ] All new tests registered in test runner
  - [ ] `cmake --build build --target proto_sketch_snap` → exit 0
  - [ ] `./build/tests/proto_sketch_snap` → exit 0, all tests pass (57 old + 3-4 new)

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: All tests pass including new crossing tests
    Tool: Bash
    Preconditions: Task 3 logic changes applied
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. Assert: build exit code 0
      3. ./build/tests/proto_sketch_snap
      4. Assert: exit code 0
      5. Assert: output shows 60+ tests passed (57 existing + 3-4 new)
    Expected Result: All tests pass, including new H/V crossing tests
    Evidence: Test output captured showing pass count

  Scenario: New tests fail if policy change reverted
    Tool: Bash (verification thought — not required to execute)
    Note: This validates tests are meaningful — crossing test would fail without Task 3 changes
  ```

  **Commit**: YES (groups with Task 3)
  - Message: `test(sketch): add H/V guide crossing snap tests`
  - Files: `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`


- [x] 5. Integration verification + full regression check

  **What to do**:
  - Full rebuild of all targets
  - Run proto_sketch_snap (all tests including new ones)
  - Run test_compile
  - Verify no compiler warnings related to changed code
  - Review final diff to ensure no unintended changes leaked in

  **Must NOT do**:
  - Do not make code changes in this task
  - Do not commit — this is verification only

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO — final verification
  - **Parallel Group**: Wave 3 (after all others)
  - **Blocks**: None (final)
  - **Blocked By**: Tasks 1, 2, 3, 4

  **References**:

  **Test References**:
  - Build: `cmake --build build` (full rebuild)
  - Snap tests: `./build/tests/proto_sketch_snap`
  - UI compile: `./build/tests/test_compile` (if target exists as executable)

  **Acceptance Criteria**:

  > AGENT-EXECUTABLE VERIFICATION ONLY

  - [ ] `cmake --build build` → exit 0, no warnings in changed files
  - [ ] `./build/tests/proto_sketch_snap` → exit 0, 60+ tests pass
  - [ ] `cmake --build build --target test_compile` → exit 0

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Full build + all tests pass
    Tool: Bash
    Preconditions: All Tasks 1-4 complete
    Steps:
      1. cmake --build build 2>&1
      2. Assert: exit code 0
      3. Assert: no warnings mentioning SketchRenderer.cpp, SnapManager.cpp, or SketchToolManager.cpp
      4. ./build/tests/proto_sketch_snap 2>&1
      5. Assert: exit code 0
      6. Assert: all tests pass (60+)
    Expected Result: Clean build, all tests green
    Evidence: Build output + test output captured

  Scenario: Verify git diff is minimal and correct
    Tool: Bash
    Steps:
      1. git diff --stat
      2. Assert: only 3-4 files changed: SketchRenderer.cpp, SketchToolManager.cpp, proto_sketch_snap.cpp (and optionally SnapManager.cpp)
      3. git diff -- src/core/sketch/SnapManager.h
      4. Assert: EMPTY (no changes to SnapType enum or operator<)
    Expected Result: Changes scoped to expected files only
    Evidence: Diff output captured
  ```

  **Commit**: NO (verification only)

---

## Commit Strategy

| After Tasks | Message | Files | Verification |
|-------------|---------|-------|--------------|
| 1+2 | `fix(sketch): visible dashed guides + crosshair at guide crossings` | `src/core/sketch/SketchRenderer.cpp` | `cmake --build build --target test_compile` |
| 3+4 | `fix(sketch): guide crossing snap priority + tests` | `src/core/sketch/tools/SketchToolManager.cpp`, `tests/prototypes/proto_sketch_snap.cpp` | `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap` |

---

## Success Criteria

### Verification Commands
```bash
cmake --build build                           # Full build succeeds
./build/tests/proto_sketch_snap               # All tests pass (60+)
cmake --build build --target test_compile     # UI compile check
git diff --stat                               # Only expected files changed
git diff -- src/core/sketch/SnapManager.h     # Empty (no enum changes)
```

### Final Checklist
- [x] Guide lines render with visible 8px/6px dashed pattern
- [x] Crosshair appears at intersection of ≥2 non-collinear active guides
- [x] H/V guide-guide crossing produces Intersection snap candidate
- [x] Intersection candidate preferred over individual H/V in guide-first policy
- [x] Vertex/Endpoint still wins over Intersection (contract preserved)
- [x] No SnapType enum changes
- [x] No operator< comparator changes
- [x] No snap radius changes
- [x] All 60+ proto_sketch_snap tests pass
- [x] No compiler warnings in changed files
