# OneCAD Sketch System Test Plan

**Document Status**: Phase 0 Inventory (Milestone 0)  
**Last Updated**: 2026-01-04  
**Test Engineer**: GPT-5.1-Codex-Max (GitHub Copilot)

---

## 1. Overview

This document provides a comprehensive test plan for the OneCAD **Sketch Engine + Constraint System**, focusing on regression-driven, deterministic, headless testing of core sketch functionality.

### 1.1 Scope
- Sketch geometry primitives (Point, Line, Arc, Circle, Ellipse)
- All constraint types (18 implemented/TODOs)
- Constraint solver (PlaneGCS integration, DOF calculation)
- Snap system (8 snap types + 4 inference types)
- Auto-constrainer (7+ inference rules, ±5° tolerance)
- Integration scenarios (realistic multi-entity sketches)

### 1.2 Out of Scope
- UI rendering, OpenGL details, Qt event loops
- File I/O, serialization (except constraint JSON round-trip)
- 3D modeling operations, topological naming (ElementMap covered separately)
- Performance benchmarks (focus on correctness first)

---

## 2. API Surfaces Available for Testing

### 2.1 Sketch Core (`src/core/sketch/`)

#### **Sketch Container** (`Sketch.h/cpp`)
```cpp
// Entity creation
EntityID addPoint(double x, double y);
EntityID addLine(const EntityID& startPtId, const EntityID& endPtId);
EntityID addArc(const EntityID& centerId, double radius, 
                double startAngle, double sweepAngle);
EntityID addCircle(const EntityID& centerId, double radius);
EntityID addEllipse(const EntityID& centerId, double majorAxis, 
                    double minorAxis, double rotation);

// Entity retrieval
template<typename T> T* getEntityAs(const EntityID& id);
SketchEntity* getEntity(const EntityID& id);
const std::vector<std::shared_ptr<SketchEntity>>& entities() const;

// Constraint management
ConstraintID addConstraint(std::unique_ptr<SketchConstraint> constraint);
SketchConstraint* getConstraint(const ConstraintID& id);
const std::vector<std::shared_ptr<SketchConstraint>>& constraints() const;

// Solver integration
const ConstraintSolver& solver() const;
DOFResult calculateDOF();
```

**Testing Pattern**: Create minimal sketches, add entities, verify state invariants.

#### **Geometry Entities** (`SketchPoint.h`, `SketchLine.h`, `SketchArc.h`, `SketchCircle.h`, `SketchEllipse.h`)

**SketchPoint**:
- `position()` / `setPosition(x, y)`
- `bounds()` → BoundingBox2d
- `isNear(point, tol)` → bool

**SketchLine**:
- `startPoint()` / `endPoint()`
- `static length(p1, p2)` / `direction()` / `midpoint()`
- `static isHorizontal()` / `isVertical()`
- `bounds()`

**SketchArc**:
- `radius()` / `setRadius(r)`
- `startAngle()` / `sweepAngle()`
- `startPoint(center)` / `endPoint(center)` / `midpoint(center)`
- `arcLength()`
- `boundsWithCenter(center)`
- `containsAngle(angle)`

**SketchCircle**:
- `radius()` / `setRadius(r)`
- `pointAtAngle(center, angle)`
- `circumference()`
- `boundsWithCenter(center)`

**SketchEllipse**:
- `majorAxis()` / `minorAxis()` / `rotation()`
- `pointAtParameter(center, param)` (0-1)
- `bounds(center)`

**Testing Pattern**: Static helpers + entity mutations + state validation.

---

### 2.2 Constraints (`src/core/sketch/constraints/Constraints.h`)

#### **18 Constraint Types** (with DOF + satisfaction checks)

| Type | Removed | Entity Refs | Key Methods | Status |
|------|---------|------------|-------------|--------|
| **Positional** |
| Coincident | 2 | 2 Points | `isSatisfied()`, `getError()` | ✅ |
| Horizontal | 1 | 1 Line | `isSatisfied()` | ✅ |
| Vertical | 1 | 1 Line | `isSatisfied()` | ✅ |
| Fixed | 2 | 1 Point | `isSatisfied()`, fixedX/Y refs | ✅ |
| Midpoint | 2 | 1 Point + 1 Line | `isSatisfied()` | ✅ |
| OnCurve | 1 | 1 Point + 1 Curve | `isSatisfied()`, position | ✅ |
| **Relational** |
| Parallel | 1 | 2 Lines | `isSatisfied()` | ✅ |
| Perpendicular | 1 | 2 Lines | `isSatisfied()` | ✅ |
| Tangent | 1 | 1 Line/Arc/Circle + 1 Curve | `isSatisfied()` | ✅ |
| Concentric | 1 | 2 Circles/Arcs | `isSatisfied()` | 🚧 TODO |
| Equal | 1 | 2 Lines/Circles | `isSatisfied()` | ✅ |
| **Dimensional** |
| Distance | 1 | 2 Points OR (1 Point + 1 Line) | `distance()`, `getError()` | ✅ |
| HorizontalDistance | 1 | 2 Points | `distance()` | ✅ |
| VerticalDistance | 1 | 2 Points | `distance()` | ✅ |
| Angle | 1 | 2 Lines OR (1 Line + 1 Vector) | `angle()`, `getError()` | ✅ |
| Radius | 1 | 1 Circle/Arc | `radius()`, `getError()` | ✅ |
| Diameter | 1 | 1 Circle/Arc | `diameter()` | 🚧 TODO |
| **Symmetry** |
| Symmetric | 2 | 2 Points + 1 Line | `isSatisfied()` | 🚧 TODO |

**Common Methods** (all constraints inherit from `SketchConstraint`):
- `id()` → ConstraintID
- `type()` / `typeName()` → ConstraintType + string
- `toString()` → display string (e.g., "Distance: 25.0 mm")
- `referencedEntities()` → std::vector<EntityID>
- `degreesRemoved()` → int (1 or 2)
- `isSatisfied(sketch, tolerance)` → bool
- `getError(sketch)` → double
- `serialize(json)` / `deserialize(json)` → JSON round-trip
- `getIconPosition(sketch)` → gp_Pnt2d (for rendering)
- `references(entityId)` → bool

**Testing Pattern**: Create constraint → check references → verify satisfaction → test JSON serialize/deserialize.

---

### 2.3 Constraint Solver (`src/core/sketch/solver/ConstraintSolver.h`)

#### **Solver Configuration & Execution**
```cpp
struct SolverConfig {
    double tolerance = 1e-4;      // Convergence tolerance (mm)
    int maxIterations = 100;
    enum Algorithm { LevenbergMarquardt, DogLeg, BFGS };
    Algorithm algorithm = Algorithm::DogLeg;
    bool detectRedundant = true;
    int timeoutMs = 1000;
};

class ConstraintSolver {
    SolverResult solve();
    DOFResult calculateDOF();
    void setConfig(const SolverConfig& cfg);
    const SolverConfig& config() const;
};

struct SolverResult {
    bool success;
    int iterations;
    double residual;
    enum Status { Success, PartialSuccess, MaxIterations, Timeout, Diverged, ... };
};

struct DOFResult {
    int total;           // Total DOF of sketch
    int constrained;     // DOF removed by constraints
    int redundant;       // Redundant constraint count
};
```

**SolverAdapter** (`SolverAdapter.h/cpp`) — Wiring sketch entities to solver:
```cpp
static void populateSolver(const Sketch& sketch, ConstraintSolver& solver);
```

**Testing Pattern**: Build sketch → populate solver → call `solve()` / `calculateDOF()` → validate result.

---

### 2.4 Snap System (`src/core/sketch/SnapManager.h`)

#### **Snap Types** (12 total, priority-ordered by enum value)

```cpp
enum class SnapType {
    None = 0,
    Vertex,              // ① Existing point (highest priority)
    Endpoint,            // ② Line/arc endpoint
    Midpoint,            // ③ Line midpoint
    Center,              // ④ Arc/circle center
    Quadrant,            // ⑤ Circle quadrant (0°, 90°, 180°, 270°)
    Intersection,        // ⑥ Two curves intersection
    OnCurve,             // ⑦ Nearest point on curve
    Grid,                // ⑧ Grid point (lowest priority)
    Perpendicular,       // ⑨ Perpendicular inference
    Tangent,             // ⑩ Tangent inference
    Horizontal,          // ⑪ Horizontal alignment inference
    Vertical             // ⑫ Vertical alignment inference
};

struct SnapResult {
    bool snapped;           // Whether snap was found
    SnapType type;
    Vec2d position;         // Snapped position
    EntityID entityId;      // Entity snapped to
    EntityID secondEntityId;// For intersections
    EntityID pointId;       // If snap maps to existing point
    double distance;        // Distance from cursor to snap
    bool operator<(const SnapResult& other) const;  // Priority comparison
};

class SnapManager {
    SnapResult findBestSnap(const Vec2d& cursorPos,
                           const Sketch& sketch,
                           const std::unordered_set<EntityID>& excludeEntities = {}) const;
    
    // Configuration (per spec)
    double snapRadius() const { return 2.0; }  // 2mm constant
    
    // Enable/disable snap types
    void setSnapTypeEnabled(SnapType type, bool enabled);
    bool isSnapTypeEnabled(SnapType type) const;
};
```

**Key Invariants**:
- **2mm snap radius** in sketch coordinates (zoom-independent constant)
- **Priority ordering** by enum value (lower = higher priority)
- **Distance sorting** on ties (closer snap wins)
- **Always check `snapped` flag** before using position

**Testing Pattern**: Build sketch with snappable entities → query at various positions → verify snap type, position, priority.

---

### 2.5 Auto-Constrainer (`src/core/sketch/AutoConstrainer.h`)

#### **Configuration & Inference**
```cpp
struct AutoConstrainerConfig {
    // Angular tolerances (radians, defaults are ±5°)
    double horizontalTolerance = 5.0 * π / 180.0;     // ±5°
    double verticalTolerance = 5.0 * π / 180.0;       // ±5°
    double perpendicularTolerance = 5.0 * π / 180.0;  // 90±5°
    double parallelTolerance = 5.0 * π / 180.0;       // ±5°
    double tangentTolerance = 5.0 * π / 180.0;        // ±5°
    
    // Distance tolerance (mm, same as snap radius per spec)
    double coincidenceTolerance = 2.0;  // 2mm
    
    double autoApplyThreshold = 0.5;   // Confidence threshold
    bool enabled = true;
};

struct InferredConstraint {
    ConstraintType type;
    EntityID entity1;
    std::optional<EntityID> entity2;
    double confidence = 1.0;           // 0.0-1.0 for opacity
    std::optional<double> value;       // For dimensional constraints
    std::optional<Vec2d> position;     // For positional constraints
};

struct DrawingContext {
    EntityID activeEntity;             // Currently drawing
    std::optional<EntityID> previousEntity;
    Vec2d startPoint, currentPoint;
    bool isFirstPoint, isPolylineMode;
};

class AutoConstrainer {
    std::vector<InferredConstraint> infer(const DrawingContext& context,
                                          const Sketch& sketch);
    
    void setTypeEnabled(ConstraintType type, bool enabled);
    bool isTypeEnabled(ConstraintType type) const;
    void enableAll();
    void disableAll();
    
    void setConfig(const AutoConstrainerConfig& config);
    const AutoConstrainerConfig& config() const;
};
```

**Inference Rules** (7+, per SPECIFICATION.md §5.14 and SKETCH_IMPLEMENTATION_PLAN.md):
1. **Horizontal** — Line within ±5° of horizontal
2. **Vertical** — Line within ±5° of vertical
3. **Coincident** — Endpoint within 2mm of existing point
4. **Tangent** — Arc start at line endpoint in tangent direction
5. **Perpendicular** — Lines meet at 90±5°
6. **Parallel** — Drawing parallel to existing line (within ±5°)
7. (Additional rules to be discovered via code inspection)

**UI Behavior** (headless testing only covers logic):
- Ghost icons shown at 50% opacity during preview
- Applied on geometry commit (separate undo from geometry)
- Can be disabled per constraint type

**Testing Pattern**: Build context → call `infer()` → verify proposed constraints → disable/enable types.

---

## 3. Known Tolerances & Configuration

### 3.1 Solver Tolerances

| Property | Value | Source | Notes |
|----------|-------|--------|-------|
| Convergence tolerance | 1e-4 mm | `ConstraintSolver.h:53` | SolverConfig::tolerance |
| Max iterations | 100 | `ConstraintSolver.h:56` | SolverConfig::maxIterations |
| Algorithm | DogLeg | `ConstraintSolver.h:62` | Default; LM/BFGS fallback |
| Redundancy detection | On | `ConstraintSolver.h:71` | SolverConfig::detectRedundant |
| Solve timeout | 1000ms | `ConstraintSolver.h:76` | SolverConfig::timeoutMs |

### 3.2 Snap System Tolerances

| Property | Value | Source | Notes |
|----------|-------|--------|-------|
| Snap radius | 2.0 mm | `SnapManager.h:270` (snapRadius()) | Sketch coords, zoom-independent |
| Priority order | Enum value | `SnapManager.h:26-40` | Lower enum = higher priority |

### 3.3 Auto-Constrainer Tolerances

| Constraint | Tolerance | Source | Notes |
|-----------|-----------|--------|-------|
| Horizontal | ±5° | `AutoConstrainer.h:67` | horizontalTolerance |
| Vertical | ±5° | `AutoConstrainer.h:68` | verticalTolerance |
| Perpendicular | 90±5° | `AutoConstrainer.h:69` | perpendicularTolerance |
| Parallel | ±5° | `AutoConstrainer.h:70` | parallelTolerance |
| Tangent | ±5° | `AutoConstrainer.h:71` | tangentTolerance |
| Coincidence | 2.0 mm | `AutoConstrainer.h:74` | coincidenceTolerance (same as snap radius) |
| Auto-apply threshold | 0.5 | `AutoConstrainer.h:76` | autoApplyThreshold |

### 3.4 Floating-Point Comparison

**Recommended tolerance for tests**:
- Entity positions: `1e-6` (relative to scale)
- Constraint satisfaction: Use `constraint.getError()` and compare against solver tolerance
- Angle comparisons: `1e-6` radians (≈ 0.00006°)

---

## 4. Test Matrix

### 4.1 Sketch Geometry Primitives

| Feature | Test Cases | Status |
|---------|-----------|--------|
| **SketchPoint** |
| Creation + position | Create, read, validate bounds | ✅ Partial (proto_sketch_geometry) |
| Mutation | Set position, verify state | ✅ Partial |
| Distance to point | isNear() with tolerance | ✅ Partial |
| **SketchLine** |
| Creation | Start + end points | ✅ Partial |
| Geometry helpers | length, direction, midpoint, isHorizontal, isVertical | ✅ Partial |
| Bounds | BoundingBox2d calculation | ✅ Partial |
| **SketchArc** |
| Creation | Center, radius, start angle, sweep angle | ⚠️ Minimal |
| Properties | sweepAngle, arcLength, containsAngle | ✅ Partial |
| Points on arc | startPoint, endPoint, midpoint | ✅ Partial |
| **SketchCircle** |
| Creation | Center, radius | ⚠️ Minimal |
| Properties | radius, circumference | ✅ Partial |
| Points on circle | pointAtAngle, quadrants | ⚠️ Minimal |
| **SketchEllipse** |
| Creation | Center, major/minor axes, rotation | ⚠️ Minimal |
| Properties | axes, rotation | ⚠️ Missing |
| Points on ellipse | pointAtParameter | ⚠️ Missing |

### 4.2 Constraints

| Constraint | Creation | Satisfaction | DOF | Serialization | Status |
|-----------|----------|--------------|-----|---------------|--------|
| **Positional** |
| Coincident | ✅ | ✅ | ✅ | ✅ | ✅ Covered |
| Horizontal | ✅ | ✅ | ✅ | ✅ | ✅ Covered |
| Vertical | ✅ | ✅ | ✅ | ✅ | ✅ Covered |
| Fixed | ✅ | ✅ | ✅ | ⚠️ | ⚠️ Partial |
| Midpoint | ✅ | ✅ | ✅ | ⚠️ | ⚠️ Partial |
| OnCurve | ⚠️ | ⚠️ | ✅ | ⚠️ | ⚠️ Minimal |
| **Relational** |
| Parallel | ✅ | ✅ | ✅ | ✅ | ✅ Covered |
| Perpendicular | ✅ | ✅ | ✅ | ✅ | ✅ Covered |
| Tangent | ✅ | ✅ | ✅ | ⚠️ | ⚠️ Partial |
| Concentric | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ❌ TODO |
| Equal | ✅ | ✅ | ✅ | ⚠️ | ⚠️ Partial |
| **Dimensional** |
| Distance | ✅ | ✅ | ✅ | ✅ | ✅ Covered |
| HorizontalDistance | ✅ | ✅ | ✅ | ⚠️ | ⚠️ Partial |
| VerticalDistance | ✅ | ✅ | ✅ | ⚠️ | ⚠️ Partial |
| Angle | ✅ | ✅ | ✅ | ⚠️ | ⚠️ Partial |
| Radius | ✅ | ✅ | ✅ | ✅ | ✅ Covered |
| Diameter | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ❌ TODO |
| **Symmetry** |
| Symmetric | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ❌ TODO |

### 4.3 Constraint Solver

| Feature | Test Cases | Status |
|---------|-----------|--------|
| **DOF Calculation** |
| Empty sketch | 0 entities = 0 DOF | ⚠️ Minimal |
| Single point | 2 DOF (X, Y free) | ⚠️ Minimal |
| Fixed point | 0 DOF | ⚠️ Minimal |
| Line + constraints | Verify DOF reduction | ✅ Partial |
| **Convergence** |
| Simple constraints | 1-2 constraints, should converge | ⚠️ Minimal |
| Realistic sketch | 5-10 entities + 8 constraints | ⚠️ Minimal |
| Over-constrained | Detect redundancy | ⚠️ Missing |
| Conflicting | Fail gracefully, no crash | ⚠️ Missing |
| **Failure Modes** |
| Backup/restore on failure | Verify state unchanged | ⚠️ Missing |
| Divergence detection | Halt on divergence | ⚠️ Missing |
| Timeout | Graceful timeout | ⚠️ Missing |

### 4.4 Snap Manager

| Feature | Test Cases | Status |
|---------|-----------|--------|
| **Snap Types** |
| Vertex snap | Snap to existing point | ⚠️ Missing |
| Endpoint snap | Snap to line/arc endpoint | ⚠️ Missing |
| Midpoint snap | Snap to line midpoint | ⚠️ Missing |
| Center snap | Snap to circle/arc center | ⚠️ Missing |
| Quadrant snap | Snap to 0°/90°/180°/270° points on circle | ⚠️ Missing |
| Intersection snap | Snap to curve intersection | ⚠️ Missing |
| OnCurve snap | Snap to nearest point on curve | ⚠️ Missing |
| Grid snap | Snap to grid point | ⚠️ Missing |
| **Priority** |
| Correct type chosen | Verify priority ordering | ⚠️ Missing |
| Distance fallback | On tie, closer snap wins | ⚠️ Missing |
| **Radius Behavior** |
| 2mm radius | Snap within 2mm radius | ⚠️ Missing |
| Beyond radius | No snap outside radius | ⚠️ Missing |
| Zoom-independence | Radius constant regardless of view scale | ⚠️ Missing |
| **Exclusion** |
| Exclude entities | Entities not snappable if excluded | ⚠️ Missing |

### 4.5 Auto-Constrainer

| Rule | Test Cases | Status |
|------|-----------|--------|
| **Horizontal** | Line within ±5° of horizontal → infer Horizontal | ⚠️ Missing |
| **Vertical** | Line within ±5° of vertical → infer Vertical | ⚠️ Missing |
| **Coincident** | Endpoint within 2mm of existing point | ⚠️ Missing |
| **Tangent** | Arc start at line endpoint, tangent direction | ⚠️ Missing |
| **Perpendicular** | Lines meet at 90±5° → infer Perpendicular | ⚠️ Missing |
| **Parallel** | Drawing parallel to existing line (±5°) | ⚠️ Missing |
| **Type enable/disable** | Enable/disable per-constraint-type inference | ⚠️ Missing |
| **Confidence** | Inferred constraints include confidence value | ⚠️ Missing |
| **Ghost opacity** | UI opacity (not testable headless) | ❌ Out of Scope |

### 4.6 Integration Scenarios

| Scenario | Description | Status |
|----------|-------------|--------|
| **Rectangle** | 4 lines + 4 corners + horizontal/vertical constraints | ⚠️ Missing |
| **Rectangle with Circle** | Rectangle + circle tangent to top edge | ⚠️ Missing |
| **Dimensioned Rectangle** | Rectangle + 4 distance constraints | ⚠️ Missing |
| **Triangle** | 3 lines forming triangle + angle constraints | ⚠️ Missing |
| **Regular Polygon** | N lines + equal length/angle constraints | ⚠️ Missing |

---

## 5. Coverage Summary

### Current State
- ✅ **Geometry Primitives**: ~50% (basic tests exist; ellipse + advanced features missing)
- ✅ **Constraints**: ~60% (most basic types tested; advanced types + comprehensive edge cases missing)
- ⚠️ **Solver**: ~20% (skeleton only; convergence + failure modes missing)
- ⚠️ **SnapManager**: ~5% (no tests found)
- ⚠️ **AutoConstrainer**: ~5% (no tests found)

### Required New Tests
- 40+ unit tests for constraints (edge cases, all types)
- 20+ solver regression tests (convergence, DOF, failure modes)
- 30+ snap tests (all types, priority, radius behavior)
- 15+ auto-constrainer tests (inference rules, tolerance, enable/disable)
- 5+ integration tests (realistic sketches)

**Total Estimated New Tests**: 110+

---

## 6. Test Harness Requirements

### 6.1 Minimal Features Needed
```cpp
// Assertion macros
EXPECT_TRUE(condition)           // Assert true, continue on failure
EXPECT_FALSE(condition)
EXPECT_EQ(a, b)                  // Equality
EXPECT_NE(a, b)                  // Not equal
EXPECT_NEAR(a, b, tolerance)     // Float comparison
EXPECT_VEC2_NEAR(v1, v2, tol)   // 2D vector comparison (custom)

// Test registration
TEST_CASE("description")         // Declare test

// Summary + exit
int main() {
    // ... run all tests
    return 0;  // or non-zero on failure
}
```

### 6.2 Recommended Options
1. **Minimal internal header** (`tests/test_harness/TestHarness.h`)
   - Pros: No external dependencies, tight control
   - Cons: Reinvent wheel, limited tooling

2. **doctest** (single-header, lightweight)
   - Pros: Feature-rich, fast, single header
   - Cons: Small external dependency

3. **Catch2** (single/multi-header versions)
   - Pros: Industry standard, very flexible
   - Cons: Heavier, more complex

**Recommendation**: Start with **minimal internal header** (fastest to implement), upgrade to **doctest** if needed.

---

## 7. Implementation Roadmap

### Milestone 1: Test Harness (1-2 hours)
- Create `tests/test_harness/TestHarness.h`
- Implement assertion macros + runner
- Migrate existing prototypes to use harness

### Milestone 2: Constraint Tests (4-6 hours)
- Comprehensive tests for all constraint types
- Edge cases (zero-length lines, tiny radii, etc.)
- JSON serialization round-trip for each type

### Milestone 3: Solver Regression (3-4 hours)
- DOF calculation tests
- Convergence scenarios
- Failure mode handling (conflict, timeout, divergence)

### Milestone 4: SnapManager Tests (2-3 hours)
- All 8 snap types
- Priority ordering
- 2mm radius behavior

### Milestone 5: AutoConstrainer Tests (2-3 hours)
- All 7+ inference rules
- Tolerance behavior
- Type enable/disable

### Milestone 6: ctest Integration (1 hour)
- Wire prototypes into CMake's `add_test()`
- Document usage in `docs/testing/TESTING.md`

**Total Estimated Time**: 13-19 hours

---

## 8. Known Unknowns & Risks

1. **PlaneGCS Integration Status** — How much solver integration is complete?
   - Risk: May not be able to test `solve()` if solver is incomplete
   - Mitigation: Focus on `calculateDOF()` and constraint satisfaction checks first

2. **Auto-Constrainer Implementation** — 7 rules stated; actual implementation coverage unclear
   - Risk: May discover fewer rules than expected
   - Mitigation: Grep code for all inference points

3. **Ellipse & Advanced Curves** — Implementation status unclear
   - Risk: Missing test coverage if not fully implemented
   - Mitigation: Verify implementation before writing tests

4. **Snap Type Implementation** — Are all 8 snap types fully implemented?
   - Risk: Some snap types may be stubs
   - Mitigation: Grep for snap type implementations

5. **Determinism** — Are there any random elements in solver / snap / auto-constrainer?
   - Risk: Flaky tests if nondeterministic
   - Mitigation: Use fixed seeds; verify reproducibility

---

## 9. Reference Documentation

- [SPECIFICATION.md](../SPECIFICATION.md) — §5 (Sketch System), §23 (PlaneGCS)
- [PHASES.md](../PHASES.md) — Phase 2 (Sketch Engine) status
- [SKETCH_IMPLEMENTATION_PLAN.md](../SKETCH_IMPLEMENTATION_PLAN.md) — Sketch detail spec
- [DEVELOPMENT.md](../../DEVELOPMENT.md) — Build & test commands
- [CLAUDE.md](../../CLAUDE.md) — Architecture overview

---

## 10. Sequential Thinking Log

See `SEQUENTIAL_THINKING_LOG.md` for detailed thinking processes per milestone.

