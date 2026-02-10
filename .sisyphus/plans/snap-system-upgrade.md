# Snap System Upgrade — Guide Lines, Visual Feedback & Missing Snap Types

## TL;DR

> **Quick Summary**: Upgrade OneCAD's snap system from basic point-only snapping to professional-grade smart guides with visual feedback. Implement 5 missing snap types (Perpendicular, Tangent, H/V Alignment, Extension/Guide), add angular snap at 15° increments, fill Ellipse gaps across all finders, add spatial hash grid for performance, and upgrade rendering from a single colored dot to orange dotted guide lines with cursor hint text.
> 
> **Deliverables**:
> - 5 new snap type implementations (Perpendicular, Tangent, Horizontal, Vertical, SketchGuide/Extension)
> - Angular snap (polar tracking) at 15° increments with absolute reference
> - Ellipse support across all 8 existing snap finders
> - Spatial hash grid for O(1) nearest-entity queries
> - Visual upgrade: orange dotted guide lines + snap type text hints + color-coded indicators
> - `proto_sketch_snap` test target with deterministic coverage
> 
> **Estimated Effort**: Large
> **Parallel Execution**: YES — 3 waves
> **Critical Path**: Task 1 (proto test) → Task 2 (spatial hash) → Task 3 (ellipse) → Tasks 4-8 (snap types + visual) → Task 9 (integration)

---

## Context

### Original Request
Improve and extend the OneCAD Sketcher Engine's Snap System — implement new snap options, improve existing ones, and bring visual feedback to professional CAD standards.

### Interview Summary
**Key Discussions**:
- **Scope focus**: "Guide lines + visual upgrade" — biggest UX impact, aligns with Shapr3D/Onshape/Fusion patterns
- **Angular snap**: 15° increments, absolute reference (sketch X-axis = 0°)
- **Visual style**: Orange dotted guide lines (Onshape-inspired), snap type text hints near cursor
- **Performance**: Spatial hash grid (simple, effective for 2D ~100-1000 entities)
- **Testing**: Extend existing proto test infrastructure, new `proto_sketch_snap` target
- **Guide lifetime**: Transient (hover-only) — appear when cursor nears alignment, disappear on move-away

**Research Findings**:
- **LibreCAD**: Sequential priority with squared-distance tiebreak; candidate filtering via nearest-entity before pair testing; extension lines via tangent direction × 1.5× bounding box
- **FreeCAD Sketcher**: Ctrl-based angular snaps at fixed multiples, axis locking, snap-to-midpoint on curves
- **Onshape**: Rich inference layer — dotted/orange guide lines, cursor text shows constraint type before commit, color-coded feedback
- **Algorithms**: Perpendicular = dot-product projection; tangent = rotated vector `(-vy, vx) * r*sqrt(c²-r²)/c²`; angular = `std::remainder(angle, increment)`; spatial hash = Mapbox GridIndex pattern

### Metis Review
**Identified Gaps** (addressed):
- Existing toggles (`showGuidePoints_`, `showSnappingHints_`) are state-only with no rendering behavior — plan includes wiring these
- Priority contract must be preserved exactly per AGENTS.md constraint
- Degenerate geometry (near-zero length entities) needs epsilon guards in perp/tangent finders
- Renderer coupling risk — keep inference math in SnapManager, renderer draw-only
- No snap proto tests exist — must add deterministic coverage first (Task 1)

---

## Work Objectives

### Core Objective
Transform OneCAD's snap system from basic point snapping into a professional smart guide system with visual feedback, implementing all declared-but-empty snap types and adding angular snap capability.

### Concrete Deliverables
- `src/core/sketch/SnapManager.h/.cpp` — Extended with 5 new finders + ellipse support + angular snap + spatial hash
- `src/core/sketch/SpatialHashGrid.h/.cpp` — New spatial hash grid class
- `src/core/sketch/SketchRenderer.h/.cpp` — Extended with guide line rendering + snap hint text
- `src/core/sketch/tools/SketchToolManager.h/.cpp` — Extended to pass guide data to renderer
- `tests/prototypes/proto_sketch_snap.cpp` — New deterministic test target
- `tests/CMakeLists.txt` — Updated with new test target

### Definition of Done
- [x] All 13 snap types in SnapType enum have working implementations
- [x] Angular snap locks at 15° increments during line drawing
- [x] Ellipse handled by all snap finders (endpoint/center/quadrant/OnCurve/intersection)
- [x] Orange dotted guide lines render for H/V alignment and extensions
- [x] Snap type text label appears near cursor ("MID", "TAN", "PERP", etc.)
- [x] `proto_sketch_snap` passes all tests (exit code 0)
- [x] `cmake --build build` succeeds with 0 errors
- [x] `test_compile` still passes (no UI regression)

### Must Have
- Preserved priority order: Vertex > Endpoint > Midpoint > Center > Quadrant > Intersection > OnCurve > Grid > Perpendicular > Tangent > Horizontal > Vertical > SketchGuide
- 2mm snap radius preserved (constants::SNAP_RADIUS_MM)
- Existing snap behavior unchanged (regression-free)
- Ellipse support in all relevant finders
- Spatial hash grid with equivalent results to brute-force
- Construction geometry participates in inference snaps

### Must NOT Have (Guardrails)
- **No R-tree or advanced spatial indexing** — grid hash only
- **No snap distance display** (spec §5.26 deferred)
- **No per-snap-type UI toggles** in SnapSettingsPanel
- **No shift-to-suppress modifier** (deferred)
- **No 3D snap (ActiveLayer3D) improvements**
- **No intersection extension beyond segment bounds**
- **No changes to non-standard sketch/world axis mapping**
- **No modification to AutoConstrainer logic** (only consume new snap data)
- **No new SnapType enum values** — implement behind existing declared entries
- **No changes to existing priority ordering contract**

---

## Verification Strategy

> **UNIVERSAL RULE: ZERO HUMAN INTERVENTION**
>
> ALL tasks verified by agent-executable commands only.

### Test Decision
- **Infrastructure exists**: YES (prototype executables in `tests/prototypes/`)
- **Automated tests**: Proto tests (existing infra pattern)
- **Framework**: Custom prototype executables linked to `onecad_core`

### Proto Test Pattern
Each new snap type gets deterministic test cases in `proto_sketch_snap`:
1. Create a Sketch with known geometry
2. Call `SnapManager::findBestSnap()` at known cursor positions
3. Assert snap type, position, and entity ID match expected values
4. Assert priority ordering preserved across all types

### Agent-Executed QA Scenarios (MANDATORY — ALL tasks)

**Primary Verification:**
```bash
cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap
# Assert: exit code 0
# Assert stdout contains all PASS lines (per-task)
```

**Regression Verification:**
```bash
cmake --build build && ./build/tests/test_compile
# Assert: exit code 0 for both
```

**Performance Verification:**
```bash
./build/tests/proto_sketch_snap --benchmark
# Assert: query time < 2ms p95 for 1000 entities
```

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Start Immediately):
├── Task 1: Proto test infrastructure (proto_sketch_snap)
└── (sequential dependency: all other tasks need this)

Wave 2 (After Task 1):
├── Task 2: Spatial hash grid implementation
├── Task 3: Ellipse support across all finders
├── Task 4: Perpendicular snap
├── Task 5: Tangent snap
├── Task 6: H/V alignment snap
├── Task 7: Extension/Guide snap
└── Task 8: Angular snap (polar tracking)

Wave 3 (After Wave 2):
└── Task 9: Visual upgrade (guide lines + hints + color-coding)
    └── Task 10: Integration, wiring & final regression

Critical Path: Task 1 → Tasks 2-8 (parallel) → Task 9 → Task 10
```

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|------|------------|--------|---------------------|
| 1 | None | 2,3,4,5,6,7,8 | None (foundation) |
| 2 | 1 | 9,10 | 3,4,5,6,7,8 |
| 3 | 1 | 9,10 | 2,4,5,6,7,8 |
| 4 | 1 | 9,10 | 2,3,5,6,7,8 |
| 5 | 1 | 9,10 | 2,3,4,6,7,8 |
| 6 | 1 | 9,10 | 2,3,4,5,7,8 |
| 7 | 1 | 9,10 | 2,3,4,5,6,8 |
| 8 | 1 | 9,10 | 2,3,4,5,6,7 |
| 9 | 2,3,4,5,6,7,8 | 10 | None |
| 10 | 9 | None | None (final) |

### Agent Dispatch Summary

| Wave | Tasks | Recommended |
|------|-------|-------------|
| 1 | 1 | `task(category="unspecified-high", load_skills=["proto-test"], ...)` |
| 2 | 2-8 | dispatch parallel, each `task(category="deep", load_skills=[], ...)` |
| 3 | 9,10 | sequential: `task(category="deep", load_skills=["render-gl"], ...)` |

---

## TODOs

- [x] 1. Create `proto_sketch_snap` Test Infrastructure

  **What to do**:
  - Create `tests/prototypes/proto_sketch_snap.cpp` — deterministic snap test executable
  - Register in `tests/CMakeLists.txt` (link to `onecad_core`)
  - Test structure: create Sketch with known geometry → call findBestSnap at known positions → assert results
  - Include baseline tests for ALL 8 existing snap types (regression coverage)
  - Include test helpers: `createTestSketch()` with a line, arc, circle, point at known positions
  - Include `--benchmark` flag support: run 1000-entity sketch, measure p95 query time
  - Include `--legacy` flag: run only existing snap type tests
  - Test output format: `PASS: test_name` or `FAIL: test_name (expected X, got Y)`

  **Must NOT do**:
  - Do not use any testing framework (Google Test, Catch2, etc.) — follow existing proto pattern
  - Do not test visual rendering (that's integration, not unit)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: Test infrastructure requiring understanding of existing proto patterns and snap system API
  - **Skills**: [`proto-test`]
    - `proto-test`: Directly matches creating prototype test executables for OneCAD

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 1 (solo)
  - **Blocks**: Tasks 2, 3, 4, 5, 6, 7, 8
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `tests/prototypes/proto_sketch_geometry.cpp` — Existing proto test pattern (create sketch, test geometry, assert results)
  - `tests/prototypes/proto_sketch_constraints.cpp` — Proto test with constraint validation pattern
  - `tests/CMakeLists.txt:28-34` — How to register proto target linked to `onecad_core`

  **API/Type References**:
  - `src/core/sketch/SnapManager.h:33-49` — SnapType enum (all 14 values to test)
  - `src/core/sketch/SnapManager.h:54-72` — SnapResult struct (fields to assert: snapped, type, position, entityId, distance)
  - `src/core/sketch/SnapManager.h:99-101` — `findBestSnap()` signature (main API under test)
  - `src/core/sketch/SnapManager.h:114-116` — `findAllSnaps()` signature (for priority ordering tests)
  - `src/core/sketch/Sketch.h` — Sketch class API for creating test geometry
  - `src/core/sketch/SketchPoint.h`, `SketchLine.h`, `SketchArc.h`, `SketchCircle.h`, `SketchEllipse.h` — Entity constructors

  **WHY Each Reference Matters**:
  - `proto_sketch_geometry.cpp`: Follow exact same test structure (main(), print PASS/FAIL, return 0/1)
  - `CMakeLists.txt:28-34`: Copy this pattern to add `proto_sketch_snap` target
  - `SnapManager.h:33-49`: Know which snap types to test (all 14 enum values)
  - `SnapManager.h:54-72`: Know which SnapResult fields to assert
  - `Sketch.h`: Understand how to create test sketches with entities at known positions

  **Acceptance Criteria**:
  - [x] `tests/prototypes/proto_sketch_snap.cpp` exists
  - [x] `tests/CMakeLists.txt` includes `proto_sketch_snap` target linked to `onecad_core`
  - [x] `cmake --build build --target proto_sketch_snap` → builds successfully
  - [x] `./build/tests/proto_sketch_snap` → exit code 0
  - [x] stdout contains: `PASS: vertex_snap`, `PASS: endpoint_snap`, `PASS: midpoint_snap`, `PASS: center_snap`, `PASS: quadrant_snap`, `PASS: intersection_snap`, `PASS: oncurve_snap`, `PASS: grid_snap`
  - [x] `./build/tests/proto_sketch_snap --legacy` → exit code 0, runs only existing type tests
  - [x] `./build/tests/proto_sketch_snap --benchmark` → prints timing info (p95)

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Proto test builds and passes baseline
    Tool: Bash
    Preconditions: Build directory exists with CMake configured
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. Assert: build succeeds (exit code 0)
      3. ./build/tests/proto_sketch_snap
      4. Assert: exit code 0
      5. Assert: stdout contains "PASS: vertex_snap"
      6. Assert: stdout contains "PASS: endpoint_snap"
      7. Assert: stdout contains "PASS: midpoint_snap"
      8. Assert: stdout contains "PASS: center_snap"
      9. Assert: stdout contains "PASS: grid_snap"
      10. Assert: stdout does NOT contain "FAIL:"
    Expected Result: All baseline snap tests pass
    Evidence: Terminal output captured

  Scenario: Legacy mode runs only existing tests
    Tool: Bash
    Preconditions: proto_sketch_snap built
    Steps:
      1. ./build/tests/proto_sketch_snap --legacy
      2. Assert: exit code 0
      3. Assert: stdout contains 8 PASS lines (one per existing type)
      4. Assert: stdout does NOT contain "perpendicular" or "tangent" or "angular"
    Expected Result: Legacy tests isolated from new functionality
    Evidence: Terminal output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): add proto_sketch_snap test infrastructure with baseline coverage`
  - Files: `tests/prototypes/proto_sketch_snap.cpp`, `tests/CMakeLists.txt`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 2. Implement Spatial Hash Grid

  **What to do**:
  - Create `src/core/sketch/SpatialHashGrid.h/.cpp` — standalone spatial index class
  - API: `insert(EntityID, Vec2d center, BoundingBox2d bounds)`, `query(Vec2d center, double radius) → vector<EntityID>`, `clear()`, `rebuild(const Sketch&)`
  - Cell size = snap radius (2mm) for optimal locality
  - Use Mapbox GridIndex pattern: fixed cells, scatter entity UIDs across overlapping cells, query only cells within radius
  - Integrate into `SnapManager`: rebuild on sketch mutation, use for candidate filtering in all finders
  - `SnapManager::findAllSnaps` should query spatial hash first, then test only nearby entities
  - Rebuild policy: call `rebuild()` when sketch mutation events occur (not every mouse move)
  - Add benchmark test to `proto_sketch_snap` verifying hash results match brute-force

  **Must NOT do**:
  - Do not use R-tree, k-d tree, or boost dependencies
  - Do not change public SnapManager API signature
  - Do not break existing snap behavior (hash is acceleration only)

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Algorithm implementation requiring careful correctness verification against brute-force
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `render-gl`: No rendering involved, pure data structure

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 3-8)
  - **Blocks**: Tasks 9, 10
  - **Blocked By**: Task 1

  **References**:

  **Pattern References**:
  - `src/core/sketch/SnapManager.cpp:82-124` — Current `findAllSnaps` loop where spatial hash integration happens
  - `src/core/sketch/SnapManager.cpp:128-153` — `findVertexSnaps` — example of linear entity scan to replace with hash query
  - `src/core/sketch/SnapManager.h:168-180` — Private members where spatial hash will be stored

  **API/Type References**:
  - `src/core/sketch/SketchTypes.h` — Vec2d, EntityID, BoundingBox2d definitions
  - `src/core/sketch/Sketch.h` — `getAllEntities()` API for rebuild
  - `src/core/sketch/SketchEntity.h` — `bounds()` method for bounding box extraction

  **External References**:
  - Mapbox GridIndex: `github.com/mapbox/mapbox-gl-native/src/mbgl/util/grid_index.cpp` — Reference implementation (fixed-cell scatter + box query)

  **WHY Each Reference Matters**:
  - `SnapManager.cpp:82-124`: This is the integration point — spatial hash filters candidates BEFORE individual finders run
  - `SnapManager.cpp:128-153`: Each finder currently does `for (const auto& entity : sketch.getAllEntities())` — replace with hash query
  - Mapbox GridIndex: Proven pattern for 2D spatial hashing in production code

  **Acceptance Criteria**:
  - [x] `src/core/sketch/SpatialHashGrid.h` and `.cpp` exist
  - [x] `SnapManager` uses spatial hash for candidate filtering
  - [x] `proto_sketch_snap` includes: `PASS: spatial_hash_equivalent_to_bruteforce`
  - [x] `proto_sketch_snap --benchmark` includes: `PASS: benchmark_query_under_2ms_p95`
  - [x] Full build succeeds: `cmake --build build` → 0 errors

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Spatial hash produces same results as brute-force
    Tool: Bash
    Preconditions: proto_sketch_snap built with Task 2 changes
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: stdout contains "PASS: spatial_hash_equivalent_to_bruteforce"
      4. Assert: exit code 0
    Expected Result: Hash acceleration doesn't change snap results
    Evidence: Terminal output captured

  Scenario: Performance benchmark passes
    Tool: Bash
    Preconditions: proto_sketch_snap built
    Steps:
      1. ./build/tests/proto_sketch_snap --benchmark
      2. Assert: stdout contains "PASS: benchmark_query_under_2ms_p95"
      3. Assert: stdout contains "p95:" with a value < 2.0
    Expected Result: Sub-2ms p95 for 1000 entities
    Evidence: Terminal output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): add spatial hash grid for O(1) nearest-entity queries`
  - Files: `src/core/sketch/SpatialHashGrid.h`, `src/core/sketch/SpatialHashGrid.cpp`, `src/core/sketch/SnapManager.h`, `src/core/sketch/SnapManager.cpp`, `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 3. Add Ellipse Support Across All Snap Finders

  **What to do**:
  - `findEndpointSnaps`: Ellipse has no traditional endpoints — skip (ellipse is a closed curve). Confirm no-op is correct.
  - `findMidpointSnaps`: Skip for ellipse (no meaningful midpoint on closed curve). Confirm no-op is correct.
  - `findCenterSnaps`: Add ellipse center snap via `centerPointId()` → look up SketchPoint position
  - `findQuadrantSnaps`: Add ellipse axis-extrema points. For rotated ellipse: compute 4 points at parametric t=0, π/2, π, 3π/2 using `SketchEllipse::pointAtParameter()`
  - `findOnCurveSnaps`: Add nearest-point-on-ellipse. Use iterative Newton-Raphson on parametric form (or bisection on angular parameter)
  - `findIntersectionSnaps`: Add line-ellipse and circle-ellipse intersections. For line-ellipse: transform to ellipse-local coords (unrotate + scale to unit circle), solve quadratic. For circle-ellipse: numerical approach (sample + refine)
  - Include `#include "SketchEllipse.h"` in SnapManager.cpp
  - Add proto_sketch_snap tests for each ellipse snap case

  **Must NOT do**:
  - Do not add endpoint/midpoint snap for ellipse (they don't apply to closed curves)
  - Do not modify SketchEllipse class itself
  - Do not add ellipse-ellipse intersection (complex, out of scope)

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Geometric algorithms (nearest point on ellipse, line-ellipse intersection) require mathematical precision
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `constraint`: Not adding constraints, just snap geometry

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 2, 4-8)
  - **Blocks**: Tasks 9, 10
  - **Blocked By**: Task 1

  **References**:

  **Pattern References**:
  - `src/core/sketch/SnapManager.cpp:277-315` — `findCenterSnaps` for Arc/Circle — follow this pattern for Ellipse center
  - `src/core/sketch/SnapManager.cpp:317-374` — `findQuadrantSnaps` for Circle/Arc — adapt for Ellipse axis-extrema
  - `src/core/sketch/SnapManager.cpp:460-530` — `findOnCurveSnaps` for Line/Circle/Arc — extend with Ellipse
  - `src/core/sketch/SnapManager.cpp:376-458` — `findIntersectionSnaps` — add Ellipse pair types

  **API/Type References**:
  - `src/core/sketch/SketchEllipse.h:59-69` — `centerPointId()`, `majorRadius()`, `minorRadius()`, `rotation()` accessors
  - `src/core/sketch/SketchEllipse.h:93` — `pointAtParameter(centerPos, t)` — computes point at parametric angle (for quadrants)
  - `src/core/sketch/SketchEllipse.h:100` — `tangentAtParameter(t)` — for potential iterative nearest-point
  - `src/core/sketch/SketchEllipse.h:122` — `type()` returns `EntityType::Ellipse`

  **WHY Each Reference Matters**:
  - `SnapManager.cpp:277-315`: Shows exact pattern for center snap (entity type check → get center point → distance check → emit result)
  - `SketchEllipse.h:93`: `pointAtParameter()` is the key API for computing quadrant points and iterating for nearest-point
  - `SketchEllipse.h:100`: `tangentAtParameter()` enables Newton-Raphson for nearest point (minimize distance function)

  **Acceptance Criteria**:
  - [x] Ellipse center snap works: cursor near ellipse center → SnapType::Center
  - [x] Ellipse quadrant snap works: cursor near axis extrema → SnapType::Quadrant (4 points per ellipse)
  - [x] Ellipse OnCurve snap works: cursor near ellipse edge → SnapType::OnCurve
  - [x] Line-ellipse intersection works: SnapType::Intersection at line-ellipse crossings
  - [x] `proto_sketch_snap` includes: `PASS: ellipse_center_snap`, `PASS: ellipse_quadrant_snap`, `PASS: ellipse_oncurve_snap`, `PASS: ellipse_line_intersection`
  - [x] Full build succeeds

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Ellipse snap types pass in proto test
    Tool: Bash
    Preconditions: proto_sketch_snap built with ellipse changes
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: stdout contains "PASS: ellipse_center_snap"
      4. Assert: stdout contains "PASS: ellipse_quadrant_snap"
      5. Assert: stdout contains "PASS: ellipse_oncurve_snap"
      6. Assert: stdout contains "PASS: ellipse_line_intersection"
      7. Assert: exit code 0
    Expected Result: All ellipse snap tests pass
    Evidence: Terminal output captured

  Scenario: Rotated ellipse quadrant points correct
    Tool: Bash
    Preconditions: proto_sketch_snap built
    Steps:
      1. ./build/tests/proto_sketch_snap
      2. Assert: stdout contains "PASS: ellipse_quadrant_rotated"
      3. (Test creates ellipse at (10,10) with major=5, minor=3, rotation=45°, verifies 4 quadrant points)
    Expected Result: Quadrant points account for rotation
    Evidence: Terminal output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): add ellipse support to center/quadrant/OnCurve/intersection finders`
  - Files: `src/core/sketch/SnapManager.cpp`, `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 4. Implement Perpendicular Snap

  **What to do**:
  - Add `findPerpendicularSnaps()` private method to SnapManager
  - Wire into `findAllSnaps()` after Grid snap, before ActiveLayer3D (respects priority ordering)
  - **For lines**: Project cursor onto infinite line using dot-product formula: `foot = lineStart + ae * dot(ae, ap) / |ae|²`. Only emit if foot is within snap radius of cursor AND foot is within line segment bounds (or within a small extension tolerance).
  - **For circles/arcs**: Perpendicular foot = center + normalize(cursor - center) * radius. For arcs, check that the foot angle is within arc angle range.
  - **For ellipses**: Use `pointAtParameter` with iterative refinement (minimize distance from cursor to ellipse point where radial direction is perpendicular to tangent).
  - **Epsilon guard**: Skip entities with length < 1e-6 mm (degenerate geometry protection per Metis directive)
  - **Context awareness**: Perpendicular snap is meaningful only when drawing FROM a reference point — snap should indicate "if you click here, the result will be perpendicular to entity X". The reference point is the active tool's start point (if available).
  - Add tests to proto_sketch_snap

  **Must NOT do**:
  - Do not add new SnapType enum values — use existing `SnapType::Perpendicular`
  - Do not emit perpendicular snap when no active drawing reference point exists
  - Do not modify AutoConstrainer (it already has perpendicular inference)

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Geometric math with edge cases (degenerate geometry, arc angle bounds)
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 2,3,5,6,7,8)
  - **Blocks**: Tasks 9, 10
  - **Blocked By**: Task 1

  **References**:

  **Pattern References**:
  - `src/core/sketch/SnapManager.cpp:460-530` — `findOnCurveSnaps` — similar pattern (iterate entities, compute nearest point, check distance)
  - `src/core/sketch/SnapManager.h:271-273` — `nearestPointOnLine()` static helper — reuse for projection

  **API/Type References**:
  - `src/core/sketch/SnapManager.h:43` — `SnapType::Perpendicular` (existing enum entry)
  - `src/core/sketch/SketchLine.h` — `startPointId()`, `endPointId()` for line endpoints
  - `src/core/sketch/SketchArc.h` — `centerPointId()`, `radius()`, `startAngle()`, `endAngle()`

  **External References**:
  - LibreCAD perpendicular: `LC_LineMath::getNearestPointOnInfiniteLine()` — dot-product projection formula

  **WHY Each Reference Matters**:
  - `SnapManager.cpp:460-530`: Exact pattern to follow (entity iteration, type dispatch, distance check, result push)
  - `nearestPointOnLine()`: Already implements projection — can be extended or called directly
  - LibreCAD formula: Proven algorithm `lineStart + ae * dot(ae, ap) / (magnitude * magnitude)`

  **Acceptance Criteria**:
  - [x] `findPerpendicularSnaps()` implemented for Line, Arc, Circle, Ellipse
  - [x] Wired into `findAllSnaps()` between Grid and ActiveLayer3D
  - [x] Degenerate geometry (< 1e-6mm) silently skipped
  - [x] `proto_sketch_snap` includes: `PASS: perpendicular_snap_line`, `PASS: perpendicular_snap_circle`, `PASS: perpendicular_snap_arc`
  - [x] Full build succeeds

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Perpendicular snap tests pass
    Tool: Bash
    Preconditions: proto_sketch_snap built with perpendicular changes
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: stdout contains "PASS: perpendicular_snap_line"
      4. Assert: stdout contains "PASS: perpendicular_snap_circle"
      5. Assert: exit code 0
    Expected Result: Perpendicular snap works for lines and circles
    Evidence: Terminal output captured
  ```

  **Commit**: YES (groups with Task 5)
  - Message: `feat(snap): implement perpendicular and tangent snap finders`
  - Files: `src/core/sketch/SnapManager.h`, `src/core/sketch/SnapManager.cpp`, `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 5. Implement Tangent Snap

  **What to do**:
  - Add `findTangentSnaps()` private method to SnapManager
  - Wire into `findAllSnaps()` after Perpendicular
  - **For circles**: Given cursor (external point) and circle (center, radius): compute tangent contact points using rotated vector method: `vp = cursor - center`, `c² = vp.squared()`, `r² = radius²`, tangent offsets = `(-vp.y, vp.x) * radius * sqrt(c² - r²) / c²`, tangent points = `center + vp * r² / c² ± offsets`. Only if cursor is OUTSIDE circle (c² > r²).
  - **For arcs**: Same as circle but check that tangent point falls within arc angle range
  - **For ellipses**: Numerical tangent (iterative: find parameter t where line from cursor to ellipse point is tangent, i.e., dot(cursor - point(t), tangent(t)) = 0)
  - **For lines**: Tangent to a line doesn't have a meaningful snap (it's collinear) — skip
  - **Epsilon guard**: Skip when cursor is on or very near the curve (c² ≈ r²)
  - Add tests to proto_sketch_snap

  **Must NOT do**:
  - Do not emit tangent snap when cursor is inside the circle/arc
  - Do not add new SnapType enum values — use existing `SnapType::Tangent`

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Geometric math (tangent point construction, numerical iteration for ellipse)
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 2-4,6-8)
  - **Blocks**: Tasks 9, 10
  - **Blocked By**: Task 1

  **References**:

  **Pattern References**:
  - `src/core/sketch/SnapManager.cpp:460-530` — `findOnCurveSnaps` pattern (entity iteration + type dispatch)

  **API/Type References**:
  - `src/core/sketch/SnapManager.h:44` — `SnapType::Tangent` (existing enum entry)
  - `src/core/sketch/SketchCircle.h` — `centerPointId()`, `radius()` for circle tangent computation
  - `src/core/sketch/SketchArc.h` — `startAngle()`, `endAngle()` for arc bounds check
  - `src/core/sketch/SketchEllipse.h:100` — `tangentAtParameter(t)` for ellipse tangent direction

  **External References**:
  - LibreCAD tangent: `RS_Circle::getTangentPoint()` — rotated vector method with `(-vp.y, vp.x) * radius * sqrt(c² - r²) / c²`

  **WHY Each Reference Matters**:
  - LibreCAD formula: Proven production algorithm for circle tangent from external point
  - `SketchEllipse::tangentAtParameter()`: Needed for iterative ellipse tangent (dot product = 0 condition)

  **Acceptance Criteria**:
  - [x] `findTangentSnaps()` implemented for Circle, Arc, Ellipse
  - [x] Wired into `findAllSnaps()` after Perpendicular
  - [x] Cursor inside circle → no tangent snap emitted
  - [x] `proto_sketch_snap` includes: `PASS: tangent_snap_circle`, `PASS: tangent_snap_arc`
  - [x] Full build succeeds

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Tangent snap tests pass
    Tool: Bash
    Preconditions: proto_sketch_snap built with tangent changes
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: stdout contains "PASS: tangent_snap_circle"
      4. Assert: stdout contains "PASS: tangent_snap_arc"
      5. Assert: stdout does NOT contain "FAIL:"
      6. Assert: exit code 0
    Expected Result: Tangent snap works for circles and arcs
    Evidence: Terminal output captured
  ```

  **Commit**: YES (grouped with Task 4)
  - Message: `feat(snap): implement perpendicular and tangent snap finders`
  - Files: `src/core/sketch/SnapManager.h`, `src/core/sketch/SnapManager.cpp`, `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 6. Implement Horizontal/Vertical Alignment Snap

  **What to do**:
  - Add `findHorizontalSnaps()` and `findVerticalSnaps()` private methods to SnapManager
  - Wire into `findAllSnaps()` after Tangent
  - **Algorithm**: For each visible sketch point (including endpoints, centers, midpoints of all entities):
    - Compute `|cursor.y - point.y|` (horizontal alignment) and `|cursor.x - point.x|` (vertical alignment)
    - If delta < snap radius → emit alignment snap at projected position `(cursor.x, point.y)` for horizontal or `(point.x, cursor.y)` for vertical
  - **Anchor set**: All visible sketch points (including entity endpoints looked up from Sketch)
  - **Guide data**: Store the reference point in SnapResult (use entityId + new field or encode in secondEntityId) so renderer can draw guide line FROM reference point TO snap position
  - **Multiple alignments**: Can emit multiple H/V alignment snaps — best one wins by distance
  - Add new data to SnapResult if needed: `Vec2d guideOrigin` for guide line rendering (or extend struct minimally)
  - Add tests to proto_sketch_snap

  **Must NOT do**:
  - Do not check alignment against entities in `excludeEntities` set
  - Do not add per-snap-type UI toggles

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Needs to gather all sketch points efficiently and manage guide line data flow
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 2-5,7,8)
  - **Blocks**: Tasks 9, 10
  - **Blocked By**: Task 1

  **References**:

  **Pattern References**:
  - `src/core/sketch/SnapManager.cpp:128-153` — `findVertexSnaps` — iterates all entities, checks points, similar scanning pattern
  - `src/core/sketch/SnapManager.cpp:156-243` — `findEndpointSnaps` — shows how to extract line/arc endpoint positions

  **API/Type References**:
  - `src/core/sketch/SnapManager.h:45-46` — `SnapType::Horizontal`, `SnapType::Vertical` (existing enum entries)
  - `src/core/sketch/SnapManager.h:54-72` — SnapResult struct (may need `guideOrigin` field for guide line rendering)
  - `src/core/sketch/Sketch.h` — `getAllEntities()`, `getEntity()`, `getPoint()` for extracting point positions

  **Documentation References**:
  - `docs/SPECIFICATION.md` §5.22.3 — Horizontal/Vertical Guide Lines spec requirement

  **External References**:
  - LibreCAD H/V: `rs_snapper.cpp:44-82` — project cursor to H/V lines through reference points, compare distances

  **WHY Each Reference Matters**:
  - `SnapManager.cpp:128-153`: Same entity iteration pattern, adapt to extract ALL point positions (not just SketchPoint entities)
  - Spec §5.22.3: This is the spec requirement we're implementing — must match expected behavior
  - LibreCAD code: Shows projection formula and distance comparison approach

  **Acceptance Criteria**:
  - [x] `findHorizontalSnaps()` and `findVerticalSnaps()` implemented
  - [x] Cursor horizontally aligned with existing point → SnapType::Horizontal emitted
  - [x] Cursor vertically aligned with existing point → SnapType::Vertical emitted
  - [x] SnapResult contains guide origin data for renderer
  - [x] `proto_sketch_snap` includes: `PASS: horizontal_alignment_snap`, `PASS: vertical_alignment_snap`
  - [x] Full build succeeds

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: H/V alignment snap tests pass
    Tool: Bash
    Preconditions: proto_sketch_snap built with H/V changes
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: stdout contains "PASS: horizontal_alignment_snap"
      4. Assert: stdout contains "PASS: vertical_alignment_snap"
      5. Assert: exit code 0
    Expected Result: H/V alignment snaps detect alignment with existing points
    Evidence: Terminal output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): implement horizontal/vertical alignment inference snaps`
  - Files: `src/core/sketch/SnapManager.h`, `src/core/sketch/SnapManager.cpp`, `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 7. Implement Extension/Guide Line Snap (SketchGuide)

  **What to do**:
  - Add `findGuideSnaps()` private method to SnapManager
  - Wire into `findAllSnaps()` after Vertical (SketchGuide position in enum)
  - **Extension snap for lines**: Extend line beyond its endpoints along tangent direction. Project cursor onto infinite line; if projection falls OUTSIDE segment bounds but within snap radius of cursor → emit SnapType::SketchGuide with position = projected point
  - **Direction**: Extend toward cursor side only (not both directions)
  - **Max extension**: Limit to 3× entity length or viewport bounds, whichever is smaller (prevent infinite guides)
  - **Guide data**: Store both the entity's endpoint AND the snapped position so renderer can draw a dotted line between them
  - **For arcs/circles**: No extension snap (they're bounded or closed). Skip.
  - **For ellipses**: No extension snap. Skip.
  - Add tests to proto_sketch_snap

  **Must NOT do**:
  - Do not extend arcs, circles, or ellipses (out of scope)
  - Do not persist guide lines (transient hover-only)
  - Do not extend in both directions simultaneously

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Geometric projection with bounds checking and guide data packaging
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 2-6,8)
  - **Blocks**: Tasks 9, 10
  - **Blocked By**: Task 1

  **References**:

  **Pattern References**:
  - `src/core/sketch/SnapManager.h:271-273` — `nearestPointOnLine()` — already computes projection; extend to handle points outside [0,1] parameter range
  - `src/core/sketch/SnapManager.cpp:460-530` — `findOnCurveSnaps` — similar cursor-to-entity projection pattern

  **External References**:
  - LibreCAD extension: `lc_looputils.cpp:75-86` — `extendLineToBBox()` extending line via normalized tangent × 1.5× bounding box

  **WHY Each Reference Matters**:
  - `nearestPointOnLine()`: The existing function clamps parameter t to [0,1] — need to allow t outside this range for extension
  - LibreCAD extension: Shows production pattern for computing extension direction and limiting length

  **Acceptance Criteria**:
  - [x] `findGuideSnaps()` implemented for lines
  - [x] Cursor beyond line endpoint along tangent → SnapType::SketchGuide emitted
  - [x] Extension limited to 3× entity length or reasonable bounds
  - [x] Guide origin (endpoint) stored for renderer
  - [x] `proto_sketch_snap` includes: `PASS: extension_snap_line`
  - [x] Full build succeeds

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Extension snap tests pass
    Tool: Bash
    Preconditions: proto_sketch_snap built with extension changes
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: stdout contains "PASS: extension_snap_line"
      4. Assert: exit code 0
    Expected Result: Extension snap works for lines beyond endpoints
    Evidence: Terminal output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): implement extension/guide line snap for lines`
  - Files: `src/core/sketch/SnapManager.h`, `src/core/sketch/SnapManager.cpp`, `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 8. Implement Angular Snap (Polar Tracking)

  **What to do**:
  - Add angular snap to SnapManager — new method `findAngularSnap(cursorPos, referencePoint, sketch, excludeEntities, radiusSq, results)`
  - **Reference frame**: Absolute — 0° = sketch X-axis positive direction, counterclockwise
  - **Increment**: 15° (π/12 radians) — snap angles: 0°, 15°, 30°, 45°, 60°, 75°, 90°, etc.
  - **Algorithm**: 
    1. Compute angle from reference point to cursor: `θ = atan2(cursor.y - ref.y, cursor.x - ref.x)`
    2. Snap to nearest increment: `θ_snapped = θ - remainder(θ, π/12)` (using `std::remainder`)
    3. Reconstruct position: `snapped_pos = ref + polar(distance, θ_snapped)` where distance = cursor distance from ref
    4. Check if snapped position is within snap radius of cursor
  - **Scope**: Active during LineTool only (when there's a reference/start point)
  - **Integration point**: `SketchToolManager::handleMouseMove()` or `findBestSnap()` — need to pass reference point context
  - **New SnapManager API**: Add optional `referencePoint` parameter to `findBestSnap()` / `findAllSnaps()` (default nullopt for backward compat)
  - **SnapType**: Use `SnapType::SketchGuide` (angular guides are a form of sketch guide) OR add angular as a sub-behavior within the guide finder. Decision: emit as `SnapType::SketchGuide` with angular info in result.
  - Add tests to proto_sketch_snap

  **Must NOT do**:
  - Do not make angular snap relative to last segment direction
  - Do not apply angular snap to Arc/Circle/Ellipse tools (line tool only)
  - Do not add new SnapType enum value for angular

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Requires careful API design (reference point passing) and mathematical correctness
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 2-7)
  - **Blocks**: Tasks 9, 10
  - **Blocked By**: Task 1

  **References**:

  **Pattern References**:
  - `src/core/sketch/SnapManager.h:63-66` — `findBestSnap()` signature — extend with optional referencePoint
  - `src/core/sketch/tools/SketchToolManager.cpp:155-183` — `handleMouseMove` — where referencePoint can be extracted from active tool
  - `src/core/sketch/tools/LineTool.h` — LineTool likely stores start point (reference for angular snap)

  **API/Type References**:
  - `src/core/sketch/SnapManager.h:47` — `SnapType::SketchGuide` — reuse for angular guide snap

  **External References**:
  - LibreCAD angular: `rs_snapper.cpp:10059-10091` — `std::remainder(ucsAngle, angularResolution)` to nearest multiple, reconstruct polar position
  - FreeCAD: `SnapManager.cpp:177-395` — Ctrl-based angular snap with axis locking

  **WHY Each Reference Matters**:
  - `findBestSnap()` signature: Must add optional parameter without breaking existing callers
  - `handleMouseMove`: Integration point where active tool's reference point is available
  - LibreCAD angular code: Proven `std::remainder` approach for angle snapping

  **Acceptance Criteria**:
  - [x] `findAngularSnap()` implemented with 15° increment snapping
  - [x] `findBestSnap()` accepts optional referencePoint (backward compatible)
  - [x] Angular snap only activates when referencePoint provided (LineTool context)
  - [x] `proto_sketch_snap` includes: `PASS: angular_snap_15deg_rounding`, `PASS: angular_snap_45deg_exact`, `PASS: angular_snap_no_reference`
  - [x] Full build succeeds

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Angular snap tests pass
    Tool: Bash
    Preconditions: proto_sketch_snap built with angular changes
    Steps:
      1. cmake --build build --target proto_sketch_snap
      2. ./build/tests/proto_sketch_snap
      3. Assert: stdout contains "PASS: angular_snap_15deg_rounding"
      4. Assert: stdout contains "PASS: angular_snap_45deg_exact"
      5. Assert: stdout contains "PASS: angular_snap_no_reference"
      6. Assert: exit code 0
    Expected Result: Angular snap rounds to 15° increments correctly
    Evidence: Terminal output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): implement angular snap (polar tracking) at 15° increments`
  - Files: `src/core/sketch/SnapManager.h`, `src/core/sketch/SnapManager.cpp`, `src/core/sketch/tools/SketchToolManager.cpp`, `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build --target proto_sketch_snap && ./build/tests/proto_sketch_snap`

---

- [x] 9. Visual Upgrade — Guide Lines, Snap Hints & Color-Coding

  **What to do**:
  - **Extend SnapResult/SnapIndicator**: Add fields for guide line data:
    - `Vec2d guideOrigin` — start point of guide line (reference point for H/V alignment, line endpoint for extension)
    - `bool hasGuide` — whether to render guide line
    - `std::string hintText` — snap type label ("MID", "END", "TAN", "PERP", "H", "V", "CTR", "QUAD", "INT", "GRID", "EXT", "15°")
  - **Extend SketchRenderer snap indicator**:
    - Add to `snapIndicator_` struct: `Vec2d guideOrigin`, `bool hasGuide`, `std::string hintText`
    - `showSnapIndicator()` signature: add guide data parameters
  - **Render guide lines**: In `buildVBOs()` / render pass:
    - Draw orange dotted line from `guideOrigin` to `snapPos` when `hasGuide` is true
    - Line style: orange (0.9, 0.5, 0.1), dotted (stipple pattern via GL dash or geometry subdivision), 1px width
    - Dotted pattern: alternate filled/gap segments, each ~2px long
  - **Render hint text**: 
    - Draw snap type text label offset from snap indicator position (e.g., +8px right, +8px up)
    - Use existing Qt text rendering or simple OpenGL text (if available in renderer)
    - Font: small (10-12pt), same orange color as guide line
    - If OpenGL text is too complex: emit text data for Viewport to render as QLabel overlay
  - **Color-code snap indicator per type**:
    - Vertex/Endpoint/Midpoint/Center: Green (0.2, 0.8, 0.3) — geometry snaps
    - Quadrant/Intersection/OnCurve: Cyan (0.2, 0.7, 0.9) — derived snaps
    - Perpendicular/Tangent: Orange (0.9, 0.5, 0.1) — inference snaps
    - Horizontal/Vertical: Blue (0.3, 0.5, 0.9) — alignment snaps
    - SketchGuide/Angular: Orange (0.9, 0.5, 0.1) — guide snaps
    - Grid: Gray (0.6, 0.6, 0.6) — grid snap
  - **Wire existing toggles**: Connect `showGuidePoints_` and `showSnappingHints_` flags to actual rendering behavior
  - **Update SketchToolManager::renderPreview()**: Pass guide data through to renderer

  **Must NOT do**:
  - Do not change snap math logic (visual layer only)
  - Do not add new shader programs (use existing line/point rendering infrastructure)
  - Do not add per-snap-type UI toggles in SnapSettingsPanel

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: OpenGL rendering integration requiring understanding of existing VBO pipeline
  - **Skills**: [`render-gl`]
    - `render-gl`: OpenGL 4.1 Core rendering patterns for viewport/sketch rendering modifications

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (solo, after all snap logic)
  - **Blocks**: Task 10
  - **Blocked By**: Tasks 2, 3, 4, 5, 6, 7, 8

  **References**:

  **Pattern References**:
  - `src/core/sketch/SketchRenderer.cpp:1657-1667` — Current `showSnapIndicator()`/`hideSnapIndicator()` — extend these
  - `src/core/sketch/SketchRenderer.cpp:1884-1908` — `buildVBOs()` where snap indicator is passed to impl — extend with guide data
  - `src/core/sketch/tools/SketchToolManager.cpp:195-214` — `renderPreview()` — where snap result flows to renderer

  **API/Type References**:
  - `src/core/sketch/SketchRenderer.h:496-500` — `snapIndicator_` struct to extend
  - `src/core/sketch/SketchRenderer.h:401-406` — `showSnapIndicator()`/`hideSnapIndicator()` signatures to extend
  - `src/core/sketch/SketchRenderer.h:55` — `constraintIcon` color (current orange reference)
  - `src/core/sketch/SketchRenderer.h:85` — `snapPointSize = 16.0f`
  - `src/core/sketch/SnapManager.h:54-72` — SnapResult struct to extend

  **Documentation References**:
  - `docs/SPECIFICATION.md` §5.22.3 — Horizontal/Vertical Guide Lines visual spec

  **WHY Each Reference Matters**:
  - `SketchRenderer.cpp:1657-1667`: This is the exact code to extend — add guide line and hint text to the indicator
  - `SketchRenderer.cpp:1884-1908`: `buildVBOs()` is where the snap data flows into the OpenGL pipeline
  - `SketchRenderer.h:496-500`: The struct that stores snap state between frames — needs guide fields
  - `SketchToolManager.cpp:195-214`: The bridge between snap logic and rendering — must pass new data through

  **Acceptance Criteria**:
  - [x] SnapResult has `guideOrigin`, `hasGuide`, `hintText` fields
  - [x] `showSnapIndicator()` accepts guide data
  - [x] Orange dotted guide lines render for H/V alignment and extension snaps
  - [x] Snap type text appears near indicator ("MID", "TAN", "PERP", etc.)
  - [x] Snap indicator color varies by type (green/cyan/orange/blue/gray)
  - [x] `showGuidePoints_` toggle controls guide line rendering
  - [x] `showSnappingHints_` toggle controls hint text rendering
  - [x] `cmake --build build` succeeds
  - [x] `test_compile` passes

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Build succeeds with visual changes
    Tool: Bash
    Preconditions: All snap logic tasks complete
    Steps:
      1. cmake --build build
      2. Assert: exit code 0 (no build errors)
      3. cmake --build build --target test_compile
      4. ./build/tests/test_compile
      5. Assert: exit code 0
    Expected Result: Visual changes compile and link correctly
    Evidence: Build output captured

  Scenario: Snap indicator renders with guide data (compile verification)
    Tool: Bash
    Preconditions: Build succeeds
    Steps:
      1. grep -c "guideOrigin" src/core/sketch/SketchRenderer.h
      2. Assert: count > 0 (field exists)
      3. grep -c "hintText" src/core/sketch/SketchRenderer.h
      4. Assert: count > 0 (field exists)
      5. grep -c "hasGuide" src/core/sketch/SnapManager.h
      6. Assert: count > 0 (field exists in SnapResult)
    Expected Result: All new fields present in headers
    Evidence: grep output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): add orange guide lines, snap hint text, and color-coded indicators`
  - Files: `src/core/sketch/SnapManager.h`, `src/core/sketch/SketchRenderer.h`, `src/core/sketch/SketchRenderer.cpp`, `src/core/sketch/tools/SketchToolManager.cpp`
  - Pre-commit: `cmake --build build && ./build/tests/test_compile`

---

- [x] 10. Integration, Wiring & Final Regression

  **What to do**:
  - **Verify all snap types work together**: Run complete `proto_sketch_snap` with all tasks integrated
  - **Verify priority ordering**: Create test with multiple snap types at similar distances, confirm correct winner
  - **Verify toggle behavior**: Test that disabling specific snap types (via `setSnapEnabled`) correctly suppresses them
  - **Wire `showGuidePoints_` / `showSnappingHints_`**: Ensure SnapSettingsPanel toggles control the new guide/hint rendering
  - **Run full build**: `cmake --build build` — all targets
  - **Run all proto tests**: Ensure no regressions in `proto_sketch_geometry`, `proto_sketch_constraints`, `proto_sketch_solver`
  - **Run test_compile**: Ensure UI still compiles
  - **Performance check**: Run `proto_sketch_snap --benchmark` to confirm sub-2ms p95

  **Must NOT do**:
  - Do not add new features in this task
  - Do not refactor existing code
  - This is verification + minor wiring only

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: Integration verification requiring broad project understanding
  - **Skills**: [`proto-test`]
    - `proto-test`: Running and validating prototype test outputs

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (sequential after Task 9)
  - **Blocks**: None (final task)
  - **Blocked By**: Task 9

  **References**:

  **Pattern References**:
  - `tests/prototypes/proto_sketch_snap.cpp` — The test file built across all tasks
  - `src/ui/viewport/SnapSettingsPanel.cpp` — Toggle wiring to SnapManager
  - `src/ui/viewport/Viewport.cpp:3431` — `updateSnapSettings()` where panel state flows to SnapManager

  **Acceptance Criteria**:
  - [x] `cmake --build build` → 0 errors
  - [x] `./build/tests/proto_sketch_snap` → exit code 0, all PASS, no FAIL
  - [x] `./build/tests/proto_sketch_snap --legacy` → exit code 0
  - [x] `./build/tests/proto_sketch_snap --benchmark` → p95 < 2ms
  - [x] `./build/tests/test_compile` → exit code 0
  - [x] `./build/tests/proto_sketch_geometry` → exit code 0 (no regression)
  - [x] `./build/tests/proto_sketch_constraints` → exit code 0 (no regression)
  - [x] `./build/tests/proto_sketch_solver` → exit code 0 (no regression)
  - [x] SnapSettingsPanel guide toggle controls guide line rendering
  - [x] SnapSettingsPanel hint toggle controls hint text rendering

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Full integration passes all tests
    Tool: Bash
    Preconditions: All tasks 1-9 committed
    Steps:
      1. cmake --build build
      2. Assert: exit code 0
      3. ./build/tests/proto_sketch_snap
      4. Assert: exit code 0, stdout contains all expected PASS lines
      5. ./build/tests/proto_sketch_snap --legacy
      6. Assert: exit code 0
      7. ./build/tests/proto_sketch_snap --benchmark
      8. Assert: stdout contains "PASS: benchmark_query_under_2ms_p95"
      9. ./build/tests/test_compile
      10. Assert: exit code 0
      11. ./build/tests/proto_sketch_geometry
      12. Assert: exit code 0
      13. ./build/tests/proto_sketch_constraints
      14. Assert: exit code 0
      15. ./build/tests/proto_sketch_solver
      16. Assert: exit code 0
    Expected Result: Zero regressions, all new functionality verified
    Evidence: Terminal output captured for all test runs

  Scenario: Priority ordering preserved
    Tool: Bash
    Preconditions: proto_sketch_snap built with all tasks
    Steps:
      1. ./build/tests/proto_sketch_snap
      2. Assert: stdout contains "PASS: priority_order_preserved"
      3. (Test creates cursor position equidistant from vertex + endpoint + midpoint, verifies Vertex wins)
    Expected Result: Existing priority contract honored
    Evidence: Terminal output captured
  ```

  **Commit**: YES
  - Message: `feat(snap): complete snap system upgrade integration and wiring`
  - Files: `src/ui/viewport/SnapSettingsPanel.cpp`, `src/ui/viewport/Viewport.cpp`, `tests/prototypes/proto_sketch_snap.cpp`
  - Pre-commit: `cmake --build build && ./build/tests/proto_sketch_snap && ./build/tests/test_compile`

---

## Commit Strategy

| After Task | Message | Key Files | Verification |
|------------|---------|-----------|--------------|
| 1 | `feat(snap): add proto_sketch_snap test infrastructure` | proto_sketch_snap.cpp, CMakeLists.txt | proto_sketch_snap |
| 2 | `feat(snap): add spatial hash grid for O(1) queries` | SpatialHashGrid.h/.cpp, SnapManager.h/.cpp | proto_sketch_snap |
| 3 | `feat(snap): add ellipse support to all snap finders` | SnapManager.cpp, proto_sketch_snap.cpp | proto_sketch_snap |
| 4+5 | `feat(snap): implement perpendicular and tangent snap` | SnapManager.h/.cpp, proto_sketch_snap.cpp | proto_sketch_snap |
| 6 | `feat(snap): implement H/V alignment inference snaps` | SnapManager.h/.cpp, proto_sketch_snap.cpp | proto_sketch_snap |
| 7 | `feat(snap): implement extension/guide line snap` | SnapManager.h/.cpp, proto_sketch_snap.cpp | proto_sketch_snap |
| 8 | `feat(snap): implement angular snap (polar tracking)` | SnapManager.h/.cpp, SketchToolManager.cpp | proto_sketch_snap |
| 9 | `feat(snap): add guide lines, hints, color-coded indicators` | SketchRenderer.h/.cpp, SnapManager.h | build + test_compile |
| 10 | `feat(snap): complete integration and toggle wiring` | SnapSettingsPanel.cpp, Viewport.cpp | all tests |

---

## Success Criteria

### Verification Commands
```bash
# Full build
cmake --build build
# Expected: 0 errors

# All snap tests
./build/tests/proto_sketch_snap
# Expected: exit code 0, all PASS lines

# Legacy regression
./build/tests/proto_sketch_snap --legacy
# Expected: exit code 0

# Performance
./build/tests/proto_sketch_snap --benchmark
# Expected: p95 < 2ms for 1000 entities

# UI compile
./build/tests/test_compile
# Expected: exit code 0

# Existing test regression
./build/tests/proto_sketch_geometry && ./build/tests/proto_sketch_constraints && ./build/tests/proto_sketch_solver
# Expected: all exit code 0
```

### Final Checklist
- [x] All 13 snap types functional (Vertex through SketchGuide)
- [x] Angular snap at 15° increments working
- [x] Ellipse handled in center/quadrant/OnCurve/intersection finders
- [x] Orange dotted guide lines for H/V alignment and extensions
- [x] Snap type hint text near cursor
- [x] Color-coded indicator by snap type category
- [x] Spatial hash grid accelerating queries
- [x] All proto tests passing
- [x] No regressions in existing tests
- [x] Priority order preserved exactly per AGENTS.md contract
