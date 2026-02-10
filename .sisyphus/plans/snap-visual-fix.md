# Snap Visual Feedback Fix

## TL;DR

> **Quick Summary**: Fix 5 root causes preventing snap guide lines, hint text labels, and angular/polar tracking from appearing during sketch drawing. The snap math works and a colored dot IS drawn, but guide lines only appear for H/V/Extension snaps, hint text is stored but never rendered, angular snaps are dead because no tool passes its reference point, and the spatial hash goes stale on geometry edits.
> 
> **Deliverables**:
> - Guide metadata (`hasGuide`, `guideOrigin`, `hintText`) populated in all relevant snap finders
> - `referencePoint` wired from active tools through SketchToolManager into SnapManager
> - Qt QPainter overlay rendering snap hint text labels ("H", "V", "45°", "MID", "PERP", etc.)
> - Spatial hash invalidation on geometry mutation (not just entity count change)
> - Updated proto_sketch_snap tests covering all fixes
> 
> **Estimated Effort**: Medium
> **Parallel Execution**: YES - 2 waves
> **Critical Path**: Task 1 → Task 3 → Task 5 → Task 6

---

## Context

### Original Request
User reported "no visual guides or snaps" when drawing sketches despite snap system implementation existing in the codebase. After thorough diagnostic analysis tracing the entire data flow pipeline (Mouse → Viewport → SketchToolManager → SnapManager → SnapResult → SketchRenderer), we identified 5 root causes and 2 minor issues.

### Interview Summary
**Key Discussions**:
- The 16px colored snap dot IS being drawn but is subtle; user's core complaint is missing guide lines and text labels
- Angular/polar tracking (15° increments) should be fixed — requires passing tool start point
- Hint text should be rendered via Qt QPainter overlay (same pattern as preview dimensions)
- Spatial hash stale-on-edit bug should be fixed

**Research Findings**:
- Render call order is CORRECT (`renderPreview()` before `render()` in Viewport.cpp:652→655)
- All SnapManager defaults ON (enabled=true, showGuidePoints=true, showSnappingHints=true)
- SnapSettingsPanel defaults all ON (every toggle `setChecked(true)`)
- Only 4 of 13+ finders set `hasGuide=true`: Horizontal, Vertical, Guide/Extension, Angular
- `hintText` is stored in `snapIndicator_` struct but NEVER read during draw
- `referencePoint` parameter in `findBestSnap()` is never supplied by any caller
- Spatial hash `rebuildSpatialHash()` only fires when entity count changes (line 163)
- Cache file changes are benign: only `.opencode/*.jsonc` config files

### Metis Review
**Identified Gaps** (addressed):
- Guide metadata added inconsistently → centralize approach via helper or standardized pattern per finder
- `referencePoint` semantics differ by tool → define virtual `getReferencePoint()` on base `SketchTool`
- QPainter overlay coordinate mismatch risk → follow existing preview dimensions pattern exactly
- Spatial hash rebuild perf risk → use dirty flag approach, not rebuild-on-every-mutation
- SnapResult lifecycle stale carryover → enforce `hideSnapIndicator()` on every no-snap frame (already done in renderPreview line 213)

---

## Work Objectives

### Core Objective
Make snap visual feedback (guide lines, hint text labels, angular tracking) appear correctly during sketch drawing by fixing metadata propagation, reference point wiring, overlay rendering, and spatial hash invalidation.

### Concrete Deliverables
- `src/core/sketch/SnapManager.cpp` — guide metadata in perpendicular/tangent finders; hintText in basic finders
- `src/core/sketch/tools/SketchTool.h` — virtual `getReferencePoint()` method
- `src/core/sketch/tools/LineTool.h/.cpp` — override `getReferencePoint()`
- `src/core/sketch/tools/ArcTool.h/.cpp` — override `getReferencePoint()`
- `src/core/sketch/tools/RectangleTool.h/.cpp` — override `getReferencePoint()`
- `src/core/sketch/tools/SketchToolManager.cpp` — pass `referencePoint` to `findBestSnap()`
- `src/core/sketch/SketchRenderer.h` — expose snap indicator state for overlay
- `src/ui/viewport/Viewport.cpp` — QPainter overlay for snap hint text
- `src/core/sketch/SpatialHashGrid.h/.cpp` — dirty flag invalidation
- `src/core/sketch/SnapManager.cpp` — spatial hash mutation-aware rebuild
- `tests/prototypes/proto_sketch_snap.cpp` — new test cases

### Definition of Done
- [x] `cmake --build build` → 0 errors, 0 new warnings
- [x] `./build/tests/proto_sketch_snap` → all tests pass including new cases (38/38)
- [x] `./build/tests/test_compile` → exit code 0
- [x] All existing snap tests remain green (29/29 + 9 new = 38/38)

### Must Have
- Guide dashed lines visible for H/V/Extension/Angular snaps (already partially working, must remain)
- Hint text labels rendered near snap point via QPainter
- Angular snap guide lines when drawing lines from a start point
- Perpendicular/tangent finders produce guide metadata
- Spatial hash rebuilds when geometry is edited/moved
- All tools that track a start point expose it for angular snap

### Must NOT Have (Guardrails)
- DO NOT change snap scoring/ranking heuristics — only add metadata to existing results
- DO NOT add per-snap-type UI toggles in SnapSettingsPanel
- DO NOT implement shift-to-suppress modifier key behavior
- DO NOT add OpenGL text rendering — use Qt QPainter overlay only
- DO NOT touch 3D snap (ActiveLayer3D) logic
- DO NOT change snap radius, priority order, or distance calculations
- DO NOT add new snap types — only fix metadata on existing ones

---

## Verification Strategy

> **UNIVERSAL RULE: ZERO HUMAN INTERVENTION**
>
> ALL tasks are verified by running commands or proto tests. No manual visual testing required for plan completion. Visual verification is bonus.

### Test Decision
- **Infrastructure exists**: YES (proto_sketch_snap, test_compile)
- **Automated tests**: Tests-after (extend existing proto_sketch_snap)
- **Framework**: Proto executables (CMake targets)

### Agent-Executed QA Scenarios (MANDATORY — ALL tasks)

**Verification Tool by Deliverable Type:**

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
├── Task 1: Add guide metadata to all snap finders (SnapManager.cpp)
├── Task 2: Add virtual getReferencePoint() to SketchTool + overrides
└── Task 4: Fix spatial hash invalidation (SpatialHashGrid + SnapManager)

Wave 2 (After Wave 1):
├── Task 3: Wire referencePoint in SketchToolManager (depends: 2)
├── Task 5: Add QPainter hint text overlay in Viewport (depends: 1)
└── Task 6: Add/extend proto_sketch_snap tests (depends: 1, 3, 4)

Final:
└── Task 7: Build verification + integration smoke test (depends: all)
```

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|------|------------|--------|---------------------|
| 1 | None | 5, 6 | 2, 4 |
| 2 | None | 3 | 1, 4 |
| 3 | 2 | 6 | 5 |
| 4 | None | 6 | 1, 2 |
| 5 | 1 | 7 | 3 |
| 6 | 1, 3, 4 | 7 | 5 |
| 7 | All | None | None (final) |

### Agent Dispatch Summary

| Wave | Tasks | Recommended Agents |
|------|-------|-------------------|
| 1 | 1, 2, 4 | 3 parallel tasks with category="quick" |
| 2 | 3, 5, 6 | 3 parallel tasks after Wave 1 |
| Final | 7 | category="quick" — build + test |

---

## TODOs

- [x] 1. Add guide metadata to all snap finders in SnapManager.cpp

  **What to do**:
  - In `findPerpendicularSnaps()`: Set `hasGuide = true`, `guideOrigin` = foot of perpendicular on the target entity, `hintText = "PERP"` in the SnapResult being pushed
  - In `findTangentSnaps()`: Set `hasGuide = true`, `guideOrigin` = tangent point on curve, `hintText = "TAN"` in the SnapResult being pushed
  - In basic finders that should show hint text but NOT guide lines, add `hintText` only:
    - `findVertexSnaps()`: `.hintText = "PT"` 
    - `findEndpointSnaps()`: `.hintText = "END"`
    - `findMidpointSnaps()`: `.hintText = "MID"`
    - `findCenterSnaps()`: `.hintText = "CEN"`
    - `findQuadrantSnaps()`: `.hintText = "QUAD"`
    - `findIntersectionSnaps()`: `.hintText = "INT"`
    - `findOnCurveSnaps()`: `.hintText = "ON"`
    - `findGridSnaps()`: `.hintText = "GRID"`
  - Keep `hasGuide = false` (default) for basic point snaps — they don't need guide lines, just hint labels
  - Verify Horizontal/Vertical/Extension/Angular finders already set metadata correctly (they do — just confirm no regression)

  **Must NOT do**:
  - Do NOT change snap scoring/distance/priority logic
  - Do NOT modify which snaps are found — only add metadata to existing results
  - Do NOT add `hasGuide = true` to basic point snaps (Vertex/Endpoint/etc.) — they don't need dashed guide lines

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single-file edits adding fields to existing push_back calls; straightforward pattern
  - **Skills**: []
    - No special skills needed; pure C++ field additions

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 4)
  - **Blocks**: Tasks 5, 6
  - **Blocked By**: None

  **References**:

  **Pattern References** (existing code to follow):
  - `src/core/sketch/SnapManager.cpp:985-990` — `findHorizontalSnaps` shows correct pattern: sets `.hasGuide = true`, `.guideOrigin = ...`, `.hintText = "H"` in SnapResult push
  - `src/core/sketch/SnapManager.cpp:1109-1114` — `findVerticalSnaps` same pattern with `.hintText = "V"`
  - `src/core/sketch/SnapManager.cpp:1163-1168` — `findGuideSnaps` same pattern with `.hintText = "EXT"`

  **API/Type References**:
  - `src/core/sketch/SnapManager.h` — `SnapResult` struct definition with `.hasGuide`, `.guideOrigin`, `.hintText` fields
  - `src/core/sketch/SnapManager.cpp:199-208` — `findVertexSnaps` push_back pattern (currently NO metadata — add `.hintText = "PT"` here)
  - `src/core/sketch/SnapManager.cpp:233-241` — `findEndpointSnaps` push_back pattern (add `.hintText = "END"`)

  **Code Locations for Each Finder** (exact lines to modify):
  - `findVertexSnaps`: SnapManager.cpp ~line 200 — add `.hintText = "PT"` to results.push_back
  - `findEndpointSnaps`: SnapManager.cpp ~lines 233, 250 (start/end variants) — add `.hintText = "END"`
  - `findMidpointSnaps`: search for `SnapType::Midpoint` push_back — add `.hintText = "MID"`
  - `findCenterSnaps`: search for `SnapType::Center` push_back — add `.hintText = "CEN"`
  - `findQuadrantSnaps`: search for `SnapType::Quadrant` push_back — add `.hintText = "QUAD"`
  - `findIntersectionSnaps`: search for `SnapType::Intersection` push_back — add `.hintText = "INT"`
  - `findOnCurveSnaps`: search for `SnapType::OnCurve` push_back — add `.hintText = "ON"`
  - `findGridSnaps`: search for `SnapType::Grid` push_back — add `.hintText = "GRID"`
  - `findPerpendicularSnaps`: search for `SnapType::Perpendicular` push_back — add `.hasGuide = true`, `.guideOrigin = <foot_of_perp>`, `.hintText = "PERP"`
  - `findTangentSnaps`: search for `SnapType::Tangent` push_back — add `.hasGuide = true`, `.guideOrigin = <tangent_point>`, `.hintText = "TAN"`

  **Acceptance Criteria**:
  - [ ] All SnapResult push_back calls include `.hintText` field
  - [ ] Perpendicular and tangent finders also set `.hasGuide = true` and `.guideOrigin`
  - [ ] `cmake --build build` → 0 errors
  - [ ] Existing proto_sketch_snap tests still pass (29/29)

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: Build succeeds after metadata additions
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
      4. Assert: output contains "29" or more tests passed
    Expected Result: All existing tests green
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `fix(snap): add guide metadata and hint text to all snap finders`
  - Files: `src/core/sketch/SnapManager.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 2. Add virtual getReferencePoint() to SketchTool base class + overrides in tools

  **What to do**:
  - In `SketchTool` base class (`src/core/sketch/tools/SketchTool.h`):
    - Add virtual method: `virtual std::optional<Vec2d> getReferencePoint() const { return std::nullopt; }`
    - Include `<optional>` header
  - In `LineTool` (`src/core/sketch/tools/LineTool.h`):
    - Override: `std::optional<Vec2d> getReferencePoint() const override`
    - Implementation: return `startPoint_` when `state_ == State::FirstClick` (i.e., when drawing from a start point), else `std::nullopt`
  - In `ArcTool` (`src/core/sketch/tools/ArcTool.h`):
    - Override similarly: return center or start point when in Drawing state
  - In `RectangleTool` (`src/core/sketch/tools/RectangleTool.h`):
    - Override: return first corner point when in FirstClick state
  - Other tools (CircleTool, EllipseTool, TrimTool, MirrorTool): leave default `std::nullopt` — they don't have meaningful reference points for angular snapping

  **Must NOT do**:
  - Do NOT change tool state machines or drawing behavior
  - Do NOT modify onMousePress/Move/Release logic
  - Do NOT change how tools use snapResult_

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Adding a virtual method + 3 one-line overrides; simple header changes
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 4)
  - **Blocks**: Task 3
  - **Blocked By**: None

  **References**:

  **Pattern References**:
  - `src/core/sketch/tools/SketchTool.h:97` — existing `isActive()` shows inline method pattern
  - `src/core/sketch/tools/SketchTool.h:104` — `setSnapResult()`/`snapResult()` shows data accessor pattern

  **API/Type References**:
  - `src/core/sketch/tools/LineTool.h:54` — `Vec2d startPoint_{0, 0}` is the reference point to expose
  - `src/core/sketch/tools/LineTool.h:32-36` — `State` enum (Idle, FirstClick, Drawing)
  - `src/core/sketch/tools/ArcTool.h` — check for center/start point member
  - `src/core/sketch/tools/RectangleTool.h` — check for first corner member
  - `src/core/sketch/SketchTypes.h` — `Vec2d` type definition

  **Acceptance Criteria**:
  - [ ] `SketchTool` has `virtual std::optional<Vec2d> getReferencePoint() const` returning `std::nullopt`
  - [ ] `LineTool` overrides: returns `startPoint_` when `state_ == FirstClick`, else `std::nullopt`
  - [ ] `ArcTool` and `RectangleTool` override similarly
  - [ ] `cmake --build build` → 0 errors
  - [ ] `./build/tests/test_compile` → exit code 0

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: Build and compile check pass
    Tool: Bash
    Steps:
      1. cmake --build build 2>&1
      2. Assert: exit code 0
      3. cmake --build build --target test_compile
      4. ./build/tests/test_compile
      5. Assert: exit code 0
    Expected Result: All tools compile with new virtual method
    Evidence: Build output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): add getReferencePoint() virtual to SketchTool for angular snap support`
  - Files: `src/core/sketch/tools/SketchTool.h`, `src/core/sketch/tools/LineTool.h`, `src/core/sketch/tools/LineTool.cpp`, `src/core/sketch/tools/ArcTool.h`, `src/core/sketch/tools/ArcTool.cpp`, `src/core/sketch/tools/RectangleTool.h`, `src/core/sketch/tools/RectangleTool.cpp`
  - Pre-commit: `cmake --build build --target test_compile && ./build/tests/test_compile`

---

- [x] 3. Wire referencePoint from active tool through SketchToolManager into findBestSnap

  **What to do**:
  - In `SketchToolManager::handleMouseMove()` (line 147):
    - Get reference point from active tool: `auto refPt = activeTool_->getReferencePoint();`
    - Pass it to findBestSnap: `currentSnapResult_ = snapManager_.findBestSnap(pos, *sketch_, excludeFromSnap_, refPt);`
  - In `SketchToolManager::handleMousePress()` (line 77) and `handleMouseRelease()` (line 171):
    - Same change: get `refPt` from active tool, pass as 4th arg to `findBestSnap`
  - Verify `findBestSnap` signature already accepts `std::optional<Vec2d> referencePoint` as 4th parameter (it does — SnapManager.h line 65-69)

  **Must NOT do**:
  - Do NOT change SnapManager::findBestSnap signature
  - Do NOT change tool lifecycle or state machine
  - Do NOT change snap priority ordering

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 3 one-line changes in a single file
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on Task 2 for getReferencePoint())
  - **Parallel Group**: Wave 2
  - **Blocks**: Task 6
  - **Blocked By**: Task 2

  **References**:

  **Pattern References**:
  - `src/core/sketch/tools/SketchToolManager.cpp:147` — current `findBestSnap(pos, *sketch_, excludeFromSnap_)` call (add 4th arg)
  - `src/core/sketch/tools/SketchToolManager.cpp:77` — same in handleMousePress
  - `src/core/sketch/tools/SketchToolManager.cpp:171` — same in handleMouseRelease

  **API/Type References**:
  - `src/core/sketch/SnapManager.h:65-69` — `findBestSnap(cursorPos, sketch, excludeEntities, referencePoint)` signature
  - `src/core/sketch/tools/SketchTool.h` — `getReferencePoint()` from Task 2
  - `src/core/sketch/SnapManager.cpp:148-153` — `findAngularSnap` only runs when `referencePoint.has_value()` — this change enables it

  **Acceptance Criteria**:
  - [ ] All 3 `findBestSnap` calls in SketchToolManager pass `activeTool_->getReferencePoint()` as 4th arg
  - [ ] `cmake --build build` → 0 errors
  - [ ] When LineTool is in FirstClick state, `findBestSnap` receives the start point → angular snaps can activate

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: Build succeeds with referencePoint wiring
    Tool: Bash
    Steps:
      1. cmake --build build 2>&1
      2. Assert: exit code 0
    Expected Result: Clean build
    Evidence: Build output captured

  Scenario: Snap tests still pass
    Tool: Bash
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: exit code 0
    Expected Result: No regressions
    Evidence: Test output captured
  ```

  **Commit**: YES (group with Task 2)
  - Message: `feat(snap): wire tool referencePoint into findBestSnap for angular/polar tracking`
  - Files: `src/core/sketch/tools/SketchToolManager.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 4. Fix spatial hash invalidation on geometry mutation

  **What to do**:
  - In `SpatialHashGrid` (`src/core/sketch/SpatialHashGrid.h`):
    - Add `void invalidate()` method that forces next `query()` to rebuild
    - Simplest: add `bool dirty_ = true` flag; `rebuild()` clears it; `invalidate()` sets it
  - In `SnapManager::rebuildSpatialHash()` (`src/core/sketch/SnapManager.cpp:161-168`):
    - Change the rebuild condition from `entityCount == lastEntityCount_` to also check a dirty flag
    - Or better: track a sketch mutation counter/hash instead of just entity count
  - In `SnapManager` header:
    - Add `void invalidateSpatialHash()` public method
    - This should be called when geometry changes (move/edit)
  - Wire invalidation: In `SketchToolManager` or at the Viewport level, call `snapManager_.invalidateSpatialHash()` after geometry mutations
    - Check `SketchToolManager` signals: `geometryCreated` signal exists — also invalidate on sketch entity modifications
    - The simplest approach: in `rebuildSpatialHash()`, also compare a version counter from the sketch, or just always rebuild (the hash rebuild is cheap for <1000 entities)

  **SIMPLEST FIX** (recommended): Just remove the `entityCount == lastEntityCount_` early-return guard in `rebuildSpatialHash()`. The spatial hash rebuild is O(n) and runs once per mouse move — for typical sketches (<100 entities) this is negligible. This avoids complex invalidation wiring.

  **Must NOT do**:
  - Do NOT introduce complex observer/notification patterns
  - Do NOT change spatial hash algorithm or cell size
  - Do NOT remove spatial hashing entirely

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Removing one early-return guard or adding a dirty flag; minimal change
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2)
  - **Blocks**: Task 6
  - **Blocked By**: None

  **References**:

  **Pattern References**:
  - `src/core/sketch/SnapManager.cpp:161-168` — current `rebuildSpatialHash()` with entity-count-only check

  **API/Type References**:
  - `src/core/sketch/SpatialHashGrid.h` — `SpatialHashGrid` class with `rebuild()` and `query()` methods
  - `src/core/sketch/SpatialHashGrid.cpp` — rebuild implementation
  - `src/core/sketch/SnapManager.h` — `mutable SpatialHashGrid spatialHash_` and `mutable size_t lastEntityCount_`

  **Acceptance Criteria**:
  - [ ] Spatial hash rebuilds when geometry is moved/edited (not just when entity count changes)
  - [ ] `cmake --build build` → 0 errors
  - [ ] Existing snap tests pass

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: Build succeeds
    Tool: Bash
    Steps:
      1. cmake --build build 2>&1
      2. Assert: exit code 0
    Expected Result: Clean build
    Evidence: Build output captured

  Scenario: Snap tests pass
    Tool: Bash
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: exit code 0
    Expected Result: No regressions
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `fix(snap): rebuild spatial hash on geometry mutation, not just entity count change`
  - Files: `src/core/sketch/SnapManager.cpp`, possibly `src/core/sketch/SpatialHashGrid.h`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 5. Add QPainter overlay for snap hint text labels in Viewport

  **What to do**:
  - In `SketchRenderer` (`src/core/sketch/SketchRenderer.h`):
    - Add public getter for snap indicator state: `const auto& getSnapIndicator() const { return snapIndicator_; }` (or a struct with position/hintText/active)
    - Alternatively, add specific getters: `bool hasActiveSnap() const`, `Vec2d getSnapPosition() const`, `std::string getSnapHintText() const`
  - In `Viewport::paintGL()` (`src/ui/viewport/Viewport.cpp`), AFTER the existing preview dimensions overlay (line ~700+):
    - Read snap indicator state from `m_sketchRenderer->getSnapIndicator()`
    - If active and hintText is non-empty:
      - Convert snap position from sketch 2D → world 3D → screen 2D (same transform as preview dimensions: use `buildPlaneAxes`, `projectToScreen`)
      - Draw text label offset ~20px above the snap point
      - Use QPainter with same font/style pattern as preview dimensions (bold, size 10, with background pill)
      - Color: use a snap-type-appropriate color or a fixed orange/green
  - Follow the EXACT pattern of lines 657-700 in Viewport.cpp (preview dimensions overlay) for coordinate conversion and QPainter usage

  **Must NOT do**:
  - Do NOT use OpenGL text rendering
  - Do NOT create a new shader for text
  - Do NOT modify the existing snap indicator point rendering
  - Do NOT render hint text if `showSnappingHints()` is false (respect the toggle — already gated in SketchToolManager::renderPreview which clears hintText when flag is off)

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Following existing QPainter overlay pattern verbatim; copy-paste + adapt
  - **Skills**: [`qt-widget`]
    - `qt-widget`: QPainter overlay in QOpenGLWidget context

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 1 provides hintText in all finders)
  - **Parallel Group**: Wave 2 (with Tasks 3, 6)
  - **Blocks**: Task 7
  - **Blocked By**: Task 1

  **References**:

  **Pattern References** (CRITICAL — follow this exact pattern):
  - `src/ui/viewport/Viewport.cpp:657-700` — Existing preview dimensions QPainter overlay: gets dimensions from renderer, converts sketch→world→screen, draws text with background. **COPY THIS PATTERN EXACTLY for snap hints.**
  - `src/ui/viewport/Viewport.cpp:674` — `buildPlaneAxes(plane)` for coordinate conversion
  - `src/ui/viewport/Viewport.cpp:684-688` — `projectToScreen(viewProjection, worldPos, width, height, &screenPos)` for world→screen

  **API/Type References**:
  - `src/core/sketch/SketchRenderer.h:498-506` — `snapIndicator_` struct with `.active`, `.position`, `.type`, `.hintText`
  - `src/core/sketch/SketchRenderer.h:386` — `getPreviewDimensions()` pattern to follow for snap getter
  - `src/ui/viewport/Viewport.h` — `projectToScreen()` method signature
  - `src/ui/theme/ThemeManager.h` — theme colors for text/background

  **Acceptance Criteria**:
  - [ ] `SketchRenderer` exposes snap indicator state via public getter
  - [ ] Viewport renders hint text as QPainter overlay when snap is active and hintText non-empty
  - [ ] Text appears ~20px above snap point in screen space
  - [ ] Text has background pill (same style as preview dimensions)
  - [ ] `cmake --build build` → 0 errors
  - [ ] `./build/tests/test_compile` → exit code 0

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: Build succeeds with overlay code
    Tool: Bash
    Steps:
      1. cmake --build build 2>&1
      2. Assert: exit code 0
    Expected Result: Clean build
    Evidence: Build output captured

  Scenario: UI compile check passes
    Tool: Bash
    Steps:
      1. cmake --build build --target test_compile
      2. ./build/tests/test_compile
      3. Assert: exit code 0
    Expected Result: Qt overlay compiles cleanly
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): add QPainter overlay for snap hint text labels`
  - Files: `src/core/sketch/SketchRenderer.h`, `src/ui/viewport/Viewport.cpp`
  - Pre-commit: `cmake --build build --target test_compile && ./build/tests/test_compile`

---

- [x] 6. Add/extend proto_sketch_snap tests for all fixes

  **What to do**:
  - In `tests/prototypes/proto_sketch_snap.cpp`, add test cases:
    1. **Guide metadata test**: Create a sketch with lines, call `findAllSnaps()` near a midpoint → verify result has `.hintText = "MID"`, `.snapped = true`
    2. **Perpendicular guide test**: Create perpendicular scenario, verify result has `.hasGuide = true`, `.hintText = "PERP"`
    3. **Tangent guide test**: Similar for tangent snap
    4. **Angular snap test**: Call `findBestSnap()` WITH `referencePoint` at known angle → verify angular snap activates with `.hasGuide = true`, `.hintText` containing degree symbol
    5. **Angular snap without referencePoint test**: Call WITHOUT referencePoint → verify NO angular snap result (confirms gating works)
    6. **Spatial hash staleness test**: Create entities, query snaps, move an entity, query again → verify snap finds moved entity at new position
    7. **hintText population test**: For each snap type, verify hintText is non-empty in the result
  - Keep all existing 29 tests unchanged

  **Must NOT do**:
  - Do NOT remove or modify existing test cases
  - Do NOT change test infrastructure or CMake setup

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
    - Reason: Adding test cases to existing test file; follows existing patterns
  - **Skills**: [`proto-test`]
    - `proto-test`: OneCAD prototype test patterns

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on Tasks 1, 3, 4 being complete so tests can verify them)
  - **Parallel Group**: Wave 2 (after all logic fixes)
  - **Blocks**: Task 7
  - **Blocked By**: Tasks 1, 3, 4

  **References**:

  **Pattern References**:
  - `tests/prototypes/proto_sketch_snap.cpp` — existing test file with 29 test cases; follow same pattern for new cases
  - Check existing test helper functions for creating test sketches with points/lines/arcs

  **API/Type References**:
  - `src/core/sketch/SnapManager.h` — `SnapResult` struct, `findBestSnap()` and `findAllSnaps()` signatures
  - `src/core/sketch/Sketch.h` — Sketch class for creating test entities
  - `src/core/sketch/SpatialHashGrid.h` — for understanding hash invalidation

  **Test References**:
  - `tests/prototypes/proto_sketch_snap.cpp` — ALL existing tests to understand patterns, assertions, sketch setup

  **Acceptance Criteria**:
  - [ ] At least 7 new test cases added
  - [ ] All new tests pass: `./build/tests/proto_sketch_snap` → exit code 0
  - [ ] All 29 existing tests still pass
  - [ ] Tests cover: hintText population, guide metadata, angular snap activation, spatial hash staleness

  **Agent-Executed QA Scenarios:**

  ```
  Scenario: All snap tests pass including new ones
    Tool: Bash
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: exit code 0
      4. Assert: output shows 36+ tests (29 existing + 7 new)
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
  - Message: `test(snap): add tests for guide metadata, angular snap, and spatial hash invalidation`
  - Files: `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 7. Final build verification and integration smoke test

  **What to do**:
  - Full clean build: `cmake --build build`
  - Run ALL relevant proto tests:
    - `./build/tests/proto_sketch_snap` — snap system (must include new tests)
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
  - **Blocked By**: All tasks (1-6)

  **References**: None needed — just build/test commands.

  **Acceptance Criteria**:
  - [ ] `cmake --build build` → exit code 0, 0 errors
  - [ ] `./build/tests/proto_sketch_snap` → exit code 0, 36+ tests pass
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
| 1 | `fix(snap): add guide metadata and hint text to all snap finders` | SnapManager.cpp | proto_sketch_snap |
| 2 | `feat(snap): add getReferencePoint() virtual to SketchTool for angular snap support` | SketchTool.h, LineTool.h/.cpp, ArcTool.h/.cpp, RectangleTool.h/.cpp | test_compile |
| 3 | `feat(snap): wire tool referencePoint into findBestSnap for angular/polar tracking` | SketchToolManager.cpp | proto_sketch_snap |
| 4 | `fix(snap): rebuild spatial hash on geometry mutation, not just entity count change` | SnapManager.cpp, SpatialHashGrid.h | proto_sketch_snap |
| 5 | `feat(snap): add QPainter overlay for snap hint text labels` | SketchRenderer.h, Viewport.cpp | test_compile |
| 6 | `test(snap): add tests for guide metadata, angular snap, and spatial hash invalidation` | proto_sketch_snap.cpp | proto_sketch_snap |
| 7 | (no commit — verification only) | — | all tests |

---

## Success Criteria

### Verification Commands
```bash
cmake --build build                              # Expected: exit code 0
./build/tests/proto_sketch_snap                   # Expected: 36+ tests pass
./build/tests/proto_sketch_geometry               # Expected: exit code 0
./build/tests/proto_sketch_constraints            # Expected: exit code 0
./build/tests/test_compile                        # Expected: exit code 0
```

### Final Checklist
- [x] All snap finders populate `hintText` in their SnapResult
- [x] Perpendicular and tangent finders set `hasGuide = true` with guide geometry
- [x] Active tools expose `getReferencePoint()` for angular snap support
- [x] `SketchToolManager` passes reference point to `findBestSnap()`
- [x] QPainter overlay renders snap hint text labels in Viewport
- [x] Spatial hash rebuilds on geometry edits
- [x] All existing 29 tests pass + 9 new tests (38/38)
- [x] No new compiler warnings
