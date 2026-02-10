# Snap Multi-Guide Rendering Fix

## TL;DR

> **Quick Summary**: Fix single-winner architecture in snap system so ALL active inference guide lines (H/V alignment, perpendicular, tangent, extension, angular) render simultaneously, even when a geometry snap (Vertex/Endpoint) wins. Also fix perp/tangent zero-length guide line bug.
> 
> **Deliverables**:
> - SketchToolManager calls `findAllSnaps()` alongside `findBestSnap()` and extracts guide info
> - SketchRenderer accepts vector of guides instead of single guide, iterates `appendDashedPolyline` for each
> - Perpendicular/Tangent finders use cursor position as `guideOrigin` (not snap position)
> - Near-zero-length guide epsilon check before VBO append
> - Updated proto_sketch_snap tests covering multi-guide scenarios
> 
> **Estimated Effort**: Medium
> **Parallel Execution**: YES - 2 waves
> **Critical Path**: Task 1 → Task 3 → Task 5

---

## Context

### Original Request
After completing snap-system-upgrade (10/10 tasks) and snap-visual-fix (7/7 tasks, 38/38 tests), user reported that inference snap guide lines (dashed orange lines for H/V, perpendicular, tangent, extension, angular) disappear whenever a geometry snap (Vertex, Endpoint, Midpoint, etc.) wins. The colored dot and hint text work, but guide lines are lost.

### Interview Summary
**Key Discussions**:
- Root cause: `findBestSnap()` returns ONE winner; all other candidates (including inference snaps with `hasGuide=true`) are discarded
- Secondary bug: `findPerpendicularSnaps/findTangentSnaps` set `guideOrigin == position` → zero-length line → invisible
- Renderer holds single `snapIndicator_` struct with one guide line's data
- Architecture choice: Approach A — separate calls (`findBestSnap` for tools + `findAllSnaps` for guide extraction)
- Perp/Tangent guideOrigin: cursor position (shows relationship visually)
- Hint text: winner-only (less clutter)

**Research Findings**:
- `findAllSnaps()` exists at SnapManager.cpp L89-156, returns sorted `vector<SnapResult>`
- `appendDashedPolyline` works for dashed guide lines (verified in Plans 1&2)
- `guideLineVAO_`/`guideLineVBO_` already allocated and rendered
- QPainter hint text overlay works for single winner
- 38/38 proto_sketch_snap tests passing

### Metis Review
**Identified Gaps** (addressed):
- Winner/guide inconsistency risk → enforce same-frame input; debug assert winner == all.front()
- Stale visual artifacts → clear guide vector atomically on every mouse move
- Overdraw/clutter → cap at 4 guides + dedupe collinear
- Near-zero guide lines → epsilon check (1e-6) before VBO append
- Performance drift → one VBO rebuild per frame only (already the case via `vboDirty_`)

---

## Work Objectives

### Core Objective
Enable simultaneous rendering of ALL active inference guide lines (H/V, perpendicular, tangent, extension, angular) alongside the winning snap point indicator, so guide lines are never discarded by the single-winner architecture.

### Concrete Deliverables
- `src/core/sketch/tools/SketchToolManager.cpp` — dual-query: `findBestSnap` + `findAllSnaps`, guide extraction
- `src/core/sketch/SketchRenderer.h/.cpp` — multi-guide API and VBO iteration
- `src/core/sketch/SnapManager.cpp` — fix perp/tangent `guideOrigin` to use cursor position
- `tests/prototypes/proto_sketch_snap.cpp` — multi-guide preservation tests

### Definition of Done
- [x] `cmake --build build` → 0 errors, 0 new warnings
- [x] `./build/tests/proto_sketch_snap` → all tests pass including new multi-guide cases
- [x] `./build/tests/test_compile` → exit code 0
- [x] All existing 38 tests remain green + new tests

### Must Have
- Multiple guide lines render simultaneously (not just winner's guide)
- Perpendicular/tangent guide lines are visible (non-zero length)
- Guide lines clear atomically when cursor moves away from guide zones
- Winning snap dot + winner-only hint text still work as before
- Guide count capped at 4 to prevent visual clutter
- Near-zero-length guides filtered out (epsilon 1e-6)

### Must NOT Have (Guardrails)
- DO NOT change snap scoring/ranking heuristics or priority order
- DO NOT change `findBestSnap()` return type or signature
- DO NOT add new SnapType enum values
- DO NOT add per-snap-type UI toggles in SnapSettingsPanel
- DO NOT use OpenGL text rendering — QPainter overlay only
- DO NOT touch 3D snap (ActiveLayer3D) logic
- DO NOT change snap radius, distance calculations, or threshold values
- DO NOT introduce new OpenGL shaders — reuse existing `guideLineVAO_`/`appendDashedPolyline`
- DO NOT expand scope to new overlays or settings panels

---

## Verification Strategy

> **UNIVERSAL RULE: ZERO HUMAN INTERVENTION**
>
> ALL tasks verified by running commands or proto tests. No manual visual testing.

### Test Decision
- **Infrastructure exists**: YES (proto_sketch_snap with 38 tests, test_compile)
- **Automated tests**: Tests-after (extend existing proto_sketch_snap)
- **Framework**: Proto executables (CMake targets)

### Agent-Executed QA Scenarios (MANDATORY — ALL tasks)

| Type | Tool | How Agent Verifies |
|------|------|-------------------|
| C++ code changes | Bash (cmake --build) | Build succeeds, 0 errors |
| Snap logic fixes | Bash (proto_sketch_snap) | All tests pass including new |
| UI compile | Bash (test_compile) | Exit code 0 |

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Start Immediately):
├── Task 1: Fix perp/tangent guideOrigin in SnapManager.cpp
└── Task 2: Expand SketchRenderer for multi-guide support

Wave 2 (After Wave 1):
├── Task 3: Wire dual-query in SketchToolManager (depends: 1, 2)
└── Task 4: Add multi-guide proto tests (depends: 1, 2, 3)

Final:
└── Task 5: Build verification + integration smoke test (depends: all)
```

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|------|------------|--------|---------------------|
| 1 | None | 3, 4 | 2 |
| 2 | None | 3, 4 | 1 |
| 3 | 1, 2 | 4, 5 | None |
| 4 | 1, 2, 3 | 5 | None |
| 5 | All | None | None (final) |

### Agent Dispatch Summary

| Wave | Tasks | Recommended Agents |
|------|-------|-------------------|
| 1 | 1, 2 | 2 parallel tasks with category="quick" |
| 2 | 3, 4 | sequential (3 then 4) |
| Final | 5 | category="quick" — build + test |

---

## TODOs

- [x] 1. Fix perpendicular/tangent guideOrigin to use cursor position

  **What to do**:
  - In `SnapManager::findPerpendicularSnaps()` (~L775-887):
    - Currently sets `guideOrigin = result.position` (the snap point itself) → zero-length guide
    - Change to: `guideOrigin = cursorPos` (the cursor position passed into `findAllSnaps`)
    - The guide line will then render FROM cursor TO perpendicular snap point
    - The `cursorPos` parameter is already available as the first argument to `findAllSnaps` / `findBestSnap`
  - In `SnapManager::findTangentSnaps()` (~L891+):
    - Same fix: `guideOrigin = cursorPos` instead of `guideOrigin = result.position`
  - Add epsilon guard: before pushing any guide result, verify `(guideOrigin - position).norm() > 1e-6`
    - If near-zero, set `hasGuide = false` to prevent invisible VBO geometry
  - Also scan ALL other finders that set `hasGuide = true` (H/V/Extension/Angular) with `ast_grep_search` to verify none have the same zero-length bug

  **Must NOT do**:
  - Do NOT change snap position calculation (where the cursor snaps TO)
  - Do NOT change snap scoring or priority
  - Do NOT modify which snaps are found — only fix guide origin metadata

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Targeted fix — changing 2-3 assignments in a single file + adding epsilon guard
  - **Skills**: []
    - No special skills needed; pure C++ field assignments
  - **Skills Evaluated but Omitted**:
    - `occt-safe`: No OCCT handles involved
    - `render-gl`: No GL code changes

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Task 2)
  - **Blocks**: Tasks 3, 4
  - **Blocked By**: None

  **References**:

  **Pattern References** (existing code to follow):
  - `src/core/sketch/SnapManager.cpp:985-990` — `findHorizontalSnaps` shows correct guide pattern: `guideOrigin` set to the REFERENCE POINT (the aligned sketch point), not the snap position. This is the pattern to follow.
  - `src/core/sketch/SnapManager.cpp:1109-1114` — `findVerticalSnaps` same correct pattern
  - `src/core/sketch/SnapManager.cpp:1163-1168` — `findGuideSnaps` / extension snap: `guideOrigin` = endpoint being extended from

  **API/Type References**:
  - `src/core/sketch/SnapManager.h:58-79` — `SnapResult` struct: `.guideOrigin` (Vec2d), `.hasGuide` (bool), `.position` (Vec2d)
  - `src/core/sketch/SnapManager.h:106` — `findBestSnap(cursorPos, ...)` — `cursorPos` is the cursor position to use as guideOrigin
  - `src/core/sketch/SnapManager.cpp` — search for all `guideOrigin =` assignments to audit

  **Code Locations** (exact locations to modify):
  - `findPerpendicularSnaps`: SnapManager.cpp search for `SnapType::Perpendicular` push_back — change `.guideOrigin = ...` to `cursorPos`
  - `findTangentSnaps`: SnapManager.cpp search for `SnapType::Tangent` push_back — change `.guideOrigin = ...` to `cursorPos`

  **Acceptance Criteria**:
  - [ ] `findPerpendicularSnaps` sets `guideOrigin = cursorPos` (not `position`)
  - [ ] `findTangentSnaps` sets `guideOrigin = cursorPos` (not `position`)
  - [ ] Epsilon guard prevents near-zero-length guides from having `hasGuide = true`
  - [ ] No other finders have zero-length guide bug (verified via search)
  - [ ] `cmake --build build` → 0 errors
  - [ ] Existing proto_sketch_snap tests still pass (38/38)

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: Build succeeds after guideOrigin fix
    Tool: Bash
    Steps:
      1. cmake --build build 2>&1
      2. Assert: exit code 0
      3. Assert: no new warnings in SnapManager.cpp
    Expected Result: Clean build
    Evidence: Build output captured

  Scenario: Existing snap tests still pass
    Tool: Bash
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: exit code 0
      4. Assert: output shows 38 tests passed
    Expected Result: All existing tests green, no regressions
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `fix(snap): set perp/tangent guideOrigin to cursor position for visible guide lines`
  - Files: `src/core/sketch/SnapManager.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 2. Expand SketchRenderer for multi-guide support

  **What to do**:
  - In `SketchRenderer.h`:
    - Add a `GuideLineInfo` struct (or reuse minimal fields):
      ```cpp
      struct GuideLineInfo {
          Vec2d origin;      // guide line start
          Vec2d target;      // guide line end (usually snap position)
      };
      ```
    - Replace or augment the single guide in `snapIndicator_`:
      - Keep existing `snapIndicator_` for winner dot/type/hintText (unchanged)
      - Add: `std::vector<GuideLineInfo> activeGuides_;` member
    - Add new public method: `void setActiveGuides(const std::vector<GuideLineInfo>& guides);`
    - Keep existing `showSnapIndicator(pos, type, guideOrigin, hasGuide, hintText)` for backward compat but mark the single-guide path as fallback
  - In `SketchRenderer.cpp`:
    - `setActiveGuides()`: store guides, set `vboDirty_ = true`
    - In `buildVBOs()` snap section (~L968-1010):
      - Replace single-guide `if (snapIndicator_.hasGuide)` block with iteration over `activeGuides_`
      - For each guide: skip if `(origin - target).norm() < 1e-6` (epsilon guard)
      - Cap at 4 guides max (take first 4 from vector)
      - Call `appendDashedPolyline(guideLineVerts, {guide.origin, guide.target}, ...)` for each
      - Use existing orange color `{0.9f, 0.5f, 0.1f}` and dash/gap = `2.0 * pixelScale`
    - Existing `guideLineVAO_`/`guideLineVBO_` already handle arbitrary line counts — no GL changes needed
    - In the clear/reset path: `activeGuides_.clear()` when snap indicator is hidden

  **Must NOT do**:
  - Do NOT create new OpenGL shaders or VAOs
  - Do NOT change the snap dot rendering (point VAO)
  - Do NOT change the QPainter hint text overlay (winner-only, unchanged)
  - Do NOT change existing `showSnapIndicator` behavior for callers that don't use multi-guide

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Adding a vector member, a setter, and modifying VBO build loop — follows existing patterns closely
  - **Skills**: [`render-gl`]
    - `render-gl`: Understanding VBO/VAO patterns for guide line rendering
  - **Skills Evaluated but Omitted**:
    - `qt-widget`: No Qt changes in this task

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Task 1)
  - **Blocks**: Tasks 3, 4
  - **Blocked By**: None

  **References**:

  **Pattern References** (existing code to follow):
  - `src/core/sketch/SketchRenderer.cpp:968-1010` — Current `buildVBOs()` snap section: single guide `if (snapIndicator_.hasGuide)` → `appendDashedPolyline`. **This is the loop to vectorize.**
  - `src/core/sketch/SketchRenderer.cpp:171-229` — `appendDashedPolyline(verts, polyline, dashLen, gapLen, color)` — the function to call per guide. Already handles arbitrary polylines.
  - `src/core/sketch/SketchRenderer.cpp:75-79` — Render pass for guide lines: `glBindVertexArray(guideLineVAO_)` → `glDrawArrays(GL_LINES, ...)`. Already renders all lines in the VBO — no changes needed here.
  - `src/core/sketch/SketchRenderer.cpp:1735-1746` — `showSnapIndicator()` current implementation: sets `snapIndicator_` fields, `vboDirty_ = true`

  **API/Type References**:
  - `src/core/sketch/SketchRenderer.h:503-511` — Current `snapIndicator_` struct (keep for winner dot/type/hintText)
  - `src/core/sketch/SketchRenderer.h:406-409` — `showSnapIndicator()` signature
  - `src/core/sketch/SketchRenderer.h:412` — `hideSnapIndicator()` — must also clear `activeGuides_`
  - `src/core/sketch/SketchTypes.h` — `Vec2d` type

  **Acceptance Criteria**:
  - [ ] `GuideLineInfo` struct defined in SketchRenderer.h
  - [ ] `activeGuides_` vector member added to SketchRenderer
  - [ ] `setActiveGuides()` public method stores guides and sets `vboDirty_`
  - [ ] `buildVBOs()` iterates `activeGuides_` calling `appendDashedPolyline` per guide
  - [ ] Epsilon guard skips near-zero-length guides in buildVBOs
  - [ ] Max 4 guides rendered (cap in buildVBOs)
  - [ ] `hideSnapIndicator()` clears `activeGuides_`
  - [ ] `cmake --build build` → 0 errors
  - [ ] `./build/tests/test_compile` → exit code 0

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: Build succeeds with multi-guide renderer
    Tool: Bash
    Steps:
      1. cmake --build build 2>&1
      2. Assert: exit code 0
      3. Assert: no new warnings in SketchRenderer.cpp
    Expected Result: Clean build
    Evidence: Build output captured

  Scenario: UI compile check passes
    Tool: Bash
    Steps:
      1. cmake --build build --target test_compile
      2. ./build/tests/test_compile
      3. Assert: exit code 0
    Expected Result: Renderer compiles with new API
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): add multi-guide support to SketchRenderer with vectorized VBO build`
  - Files: `src/core/sketch/SketchRenderer.h`, `src/core/sketch/SketchRenderer.cpp`
  - Pre-commit: `cmake --build build --target test_compile && ./build/tests/test_compile`

---

- [x] 3. Wire dual-query in SketchToolManager for guide extraction

  **What to do**:
  - In `SketchToolManager::handleMouseMove()` (~L147):
    - AFTER the existing `findBestSnap()` call (which sets `currentSnapResult_`):
    - Add: `auto allSnaps = snapManager_.findAllSnaps(pos, *sketch_, excludeFromSnap_, refPt);`
    - Filter: extract all entries where `hasGuide == true` into a `std::vector<GuideLineInfo>`
    - Dedupe: skip guides whose origin→target line is collinear and overlapping with an already-collected guide (within angular epsilon ~0.01 radians)
    - Cap at 4 guides
    - Pass to renderer: `sketchRenderer_.setActiveGuides(guides);`
    - Debug assert: `assert(allSnaps.empty() || allSnaps.front().type == currentSnapResult_.type)` to catch winner/guide mismatch (only in debug builds)
  - In `SketchToolManager::renderPreview()` (~L195-223):
    - The existing `showSnapIndicator()` call remains — it handles winner dot + hintText
    - The guide lines are now handled by `activeGuides_` set in `handleMouseMove`
    - When NO snap: call `sketchRenderer_.setActiveGuides({})` to clear stale guides
    - Also in the existing `hideSnapIndicator()` path (~L213): guides already cleared (Task 2 ensures `hideSnapIndicator` clears them)
  - When snap is NOT active (no results): ensure `setActiveGuides({})` is called to prevent stale guide artifacts

  **Must NOT do**:
  - Do NOT change `findBestSnap()` signature or return type
  - Do NOT change how `currentSnapResult_` is used by tools
  - Do NOT modify tool state machines
  - Do NOT change snap priority/scoring

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Adding ~15 lines of guide extraction logic in a single function
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `render-gl`: Not modifying GL code, just calling renderer API

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on Tasks 1 and 2)
  - **Parallel Group**: Wave 2 (sequential after Wave 1)
  - **Blocks**: Tasks 4, 5
  - **Blocked By**: Tasks 1, 2

  **References**:

  **Pattern References** (existing code to follow):
  - `src/core/sketch/tools/SketchToolManager.cpp:147` — Current `findBestSnap()` call site in `handleMouseMove`
  - `src/core/sketch/tools/SketchToolManager.cpp:195-223` — `renderPreview()` where snap indicator is shown/hidden
  - `src/core/sketch/tools/SketchToolManager.cpp:213` — `hideSnapIndicator()` path (no-snap case)

  **API/Type References**:
  - `src/core/sketch/SnapManager.h:122` — `findAllSnaps()` signature: returns `std::vector<SnapResult>`
  - `src/core/sketch/SnapManager.h:58-79` — `SnapResult` struct: `.hasGuide`, `.guideOrigin`, `.position`
  - `src/core/sketch/SketchRenderer.h` — `setActiveGuides()` from Task 2, `GuideLineInfo` struct
  - `src/core/sketch/SketchRenderer.h:412` — `hideSnapIndicator()` (already clears guides per Task 2)

  **Acceptance Criteria**:
  - [ ] `handleMouseMove` calls `findAllSnaps()` after `findBestSnap()` on same cursor position
  - [ ] Guide extraction filters for `hasGuide == true` entries
  - [ ] Collinear dedupe eliminates duplicate guides
  - [ ] Max 4 guides passed to renderer
  - [ ] `setActiveGuides({})` called when no snap is active (prevents stale artifacts)
  - [ ] Debug assert verifies winner consistency
  - [ ] `cmake --build build` → 0 errors
  - [ ] Existing proto_sketch_snap tests pass (38/38)

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: Build succeeds with dual-query wiring
    Tool: Bash
    Steps:
      1. cmake --build build 2>&1
      2. Assert: exit code 0
    Expected Result: Clean build
    Evidence: Build output captured

  Scenario: Snap tests pass (no behavioral regression)
    Tool: Bash
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: exit code 0
      4. Assert: 38 tests passed
    Expected Result: No regressions in snap behavior
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): wire dual-query in SketchToolManager for multi-guide extraction`
  - Files: `src/core/sketch/tools/SketchToolManager.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 4. Add multi-guide proto tests

  **What to do**:
  - In `tests/prototypes/proto_sketch_snap.cpp`, add new test cases:

    1. **preserves_guides_when_vertex_wins**: Create a sketch with a vertex at (5,5) and a horizontal line. Place cursor near the vertex but aligned horizontally with the line's midpoint. Call `findAllSnaps()` → the winner should be Vertex (highest priority), but the allSnaps vector should ALSO contain an H/V alignment snap with `hasGuide=true`. Verify the guide is present in the full results even though Vertex wins.

    2. **perpendicular_guide_has_nonzero_length**: Create a line and a point. Call `findPerpendicularSnaps` (via `findAllSnaps`) with cursor near the perpendicular foot. Verify the resulting snap has `hasGuide=true` AND `(guideOrigin - position).norm() > 1e-6`.

    3. **tangent_guide_has_nonzero_length**: Create a circle and a line. Call `findAllSnaps` near the tangent point. Verify tangent snap has `hasGuide=true` AND nonzero guide length.

    4. **clears_guides_when_no_snap**: Call `findAllSnaps` with cursor far from all geometry. Verify returned vector is empty OR contains no `hasGuide=true` entries.

    5. **guide_count_bounded**: Create a complex sketch with many potential guide candidates. Verify that even with many inference possibilities, the filtered guide count does not exceed 4 (this tests the extraction logic pattern, may need to simulate in the test).

    6. **dedupe_collinear_guides**: Create two collinear horizontal lines. Place cursor aligned with both. Verify only ONE H-alignment guide is extracted (not two overlapping guides).

  - Keep all existing 38 tests unchanged

  **Must NOT do**:
  - Do NOT remove or modify existing test cases
  - Do NOT change test infrastructure or CMake setup
  - Do NOT add tests that require visual/human verification

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
    - Reason: Adding test cases to existing file; follows established patterns
  - **Skills**: [`proto-test`]
    - `proto-test`: OneCAD prototype test patterns

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on Tasks 1, 2, 3)
  - **Parallel Group**: Wave 2 (after Task 3)
  - **Blocks**: Task 5
  - **Blocked By**: Tasks 1, 2, 3

  **References**:

  **Pattern References**:
  - `tests/prototypes/proto_sketch_snap.cpp` — existing 38 test cases; follow same pattern (sketch setup → snap call → assert result fields)

  **API/Type References**:
  - `src/core/sketch/SnapManager.h` — `findAllSnaps()` returns `std::vector<SnapResult>`, `findBestSnap()` returns single `SnapResult`
  - `src/core/sketch/SnapManager.h:58-79` — `SnapResult` struct fields to assert against
  - `src/core/sketch/Sketch.h` — Sketch class for test entity creation

  **Test References**:
  - `tests/prototypes/proto_sketch_snap.cpp` — ALL existing tests for patterns, assertion helpers, sketch setup utilities

  **Acceptance Criteria**:
  - [ ] At least 6 new test cases added
  - [ ] All new tests pass: `./build/tests/proto_sketch_snap` → exit code 0
  - [ ] All 38 existing tests still pass
  - [ ] Tests cover: guide preservation when vertex wins, nonzero perp/tangent guide length, guide clearing, guide count cap, collinear dedupe

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: All snap tests pass including new multi-guide cases
    Tool: Bash
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: exit code 0
      4. Assert: output shows 44+ tests (38 existing + 6 new)
    Expected Result: All tests green
    Evidence: Test output captured

  Scenario: No test compile errors
    Tool: Bash
    Steps:
      1. cmake --build build --target proto_sketch_snap 2>&1
      2. Assert: exit code 0
      3. Assert: no errors in output
    Expected Result: Test target compiles cleanly
    Evidence: Build output captured
  ```

  **Commit**: YES
  - Message: `test(snap): add multi-guide preservation and dedupe tests`
  - Files: `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 5. Final build verification and integration smoke test

  **What to do**:
  - Full clean build: `cmake --build build`
  - Run ALL relevant proto tests:
    - `./build/tests/proto_sketch_snap` — snap system (must include new multi-guide tests)
    - `./build/tests/proto_sketch_geometry` — geometry basics (regression check)
    - `./build/tests/proto_sketch_constraints` — constraints (regression check)
    - `./build/tests/test_compile` — UI compile check
  - Verify no new compiler warnings in modified files
  - If all pass → done

  **Must NOT do**:
  - Do NOT make code changes in this task — only verify

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Just running build + test commands
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO (final verification)
  - **Parallel Group**: Sequential (after all other tasks)
  - **Blocks**: None (final)
  - **Blocked By**: All tasks (1-4)

  **References**: None needed — just build/test commands.

  **Acceptance Criteria**:
  - [ ] `cmake --build build` → exit code 0, 0 errors
  - [ ] `./build/tests/proto_sketch_snap` → exit code 0, 44+ tests pass
  - [ ] `./build/tests/proto_sketch_geometry` → exit code 0
  - [ ] `./build/tests/proto_sketch_constraints` → exit code 0
  - [ ] `./build/tests/test_compile` → exit code 0

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: Full build and all tests pass
    Tool: Bash
    Steps:
      1. cmake --build build 2>&1
      2. Assert: exit code 0
      3. ./build/tests/proto_sketch_snap
      4. Assert: exit code 0
      5. ./build/tests/proto_sketch_geometry
      6. Assert: exit code 0
      7. ./build/tests/proto_sketch_constraints
      8. Assert: exit code 0
      9. ./build/tests/test_compile
      10. Assert: exit code 0
    Expected Result: All builds and tests green
    Evidence: All outputs captured
  ```

  **Commit**: NO (verification only — no code changes)

---

## Commit Strategy

| After Task | Message | Files | Verification |
|------------|---------|-------|--------------|
| 1 | `fix(snap): set perp/tangent guideOrigin to cursor position for visible guide lines` | SnapManager.cpp | proto_sketch_snap |
| 2 | `feat(snap): add multi-guide support to SketchRenderer with vectorized VBO build` | SketchRenderer.h, SketchRenderer.cpp | test_compile |
| 3 | `feat(snap): wire dual-query in SketchToolManager for multi-guide extraction` | SketchToolManager.cpp | proto_sketch_snap |
| 4 | `test(snap): add multi-guide preservation and dedupe tests` | proto_sketch_snap.cpp | proto_sketch_snap |
| 5 | (no commit — verification only) | — | all tests |

---

## Success Criteria

### Verification Commands
```bash
cmake --build build                              # Expected: exit code 0
./build/tests/proto_sketch_snap                   # Expected: 44+ tests pass
./build/tests/proto_sketch_geometry               # Expected: exit code 0
./build/tests/proto_sketch_constraints            # Expected: exit code 0
./build/tests/test_compile                        # Expected: exit code 0
```

### Final Checklist
- [x] Multiple guide lines render simultaneously (not just winner's)
- [x] Perpendicular guide lines have non-zero length (guideOrigin = cursor position)
- [x] Tangent guide lines have non-zero length (guideOrigin = cursor position)
- [x] Guide lines clear when cursor moves away from guide zones
- [x] Guide count capped at 4 per frame
- [x] Collinear duplicate guides are eliminated
- [x] Winner snap dot and hint text still work as before
- [x] All 38 existing tests pass + 6 new tests (44+ total)
- [x] No new compiler warnings
