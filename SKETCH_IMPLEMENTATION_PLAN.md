# OneCAD Sketch System Implementation Plan

Status: **Phase 1 Complete** - Architecture & Interface Definition

---

## Implementation Status Overview

### COMPLETED - Phase 1: Architecture Foundation

| Component | File | Status |
|-----------|------|--------|
| Type Definitions | `src/core/sketch/SketchTypes.h` | Complete |
| Entity Base Class | `src/core/sketch/SketchEntity.h` | Complete |
| Point Entity | `src/core/sketch/SketchPoint.h` | Complete |
| Line Entity | `src/core/sketch/SketchLine.h` | Complete |
| Arc Entity | `src/core/sketch/SketchArc.h` | Complete |
| Circle Entity | `src/core/sketch/SketchCircle.h` | Complete |
| Constraint Base | `src/core/sketch/SketchConstraint.h` | Complete |
| Concrete Constraints | `src/core/sketch/constraints/Constraints.h` | Complete |
| Sketch Manager | `src/core/sketch/Sketch.h` | Complete |
| Solver Interface | `src/core/sketch/solver/ConstraintSolver.h` | Interface Only |
| Loop Detector | `src/core/loop/LoopDetector.h` | Interface Only |
| Renderer Interface | `src/core/sketch/SketchRenderer.h` | Interface Only |
| CMake Configuration | `src/core/CMakeLists.txt` | Complete |

### AWAITING IMPLEMENTATION

| Component | Phase | Complexity |
|-----------|-------|------------|
| PlaneGCS Integration | 2 | High |
| Sketch.cpp Implementation | 2 | Medium |
| Constraint Solver Implementation | 2 | High |
| Loop Detection Algorithms | 3 | High |
| Rendering System | 4 | High |
| Snap & Auto-Constrain | 5 | Medium |
| Sketch Tools (Line, Arc, etc.) | 6 | Medium |
| UI Integration | 7 | Medium |

---

## Detailed Implementation Phases

### Phase 2: PlaneGCS Integration & Core Implementation

**Priority: Critical**
**Estimated Complexity: High**
**Dependencies: Phase 1**

#### 2.1 PlaneGCS Library Setup
```
Tasks:
- Download/build PlaneGCS from FreeCAD source
- Configure CMake to find and link PlaneGCS
- Create wrapper for LGPL compliance (separate module)
- Write basic integration tests
```

Files to Create/Modify:
- `external/planegcs/` - Library sources
- `cmake/FindPlaneGCS.cmake` - CMake find module
- `src/core/sketch/solver/PlaneGCSWrapper.h/.cpp`

#### 2.2 ConstraintSolver Implementation
```cpp
// Key implementation tasks in ConstraintSolver.cpp:
void ConstraintSolver::addPoint(SketchPoint* point) {
    // TODO: Create GCS::Point with direct parameter binding
    // gcs_->addPoint(&point->x_, &point->y_);
}

SolverResult ConstraintSolver::solve() {
    // TODO: Implement full solve sequence
    // 1. backupParameters()
    // 2. gcs_->solve(algorithm_)
    // 3. Check result, apply or revert
}
```

Algorithm Implementation Notes:
- Use LevenbergMarquardt as default (most stable)
- 1e-4mm tolerance matches spec
- Direct parameter binding (no copying)
- Backup/restore for failed solves

#### 2.3 Sketch.cpp Implementation
```cpp
// Key implementation tasks:
EntityID Sketch::addPoint(double x, double y, bool construction) {
    // Create SketchPoint
    // Add to entities_ vector
    // Register with solver
    // Return ID
}

SolveResult Sketch::solve() {
    // Rebuild solver if dirty
    // Call solver_->solve()
    // Update cached DOF
    // Return result
}
```

#### 2.4 Constraint Registration
```
Constraint Type → PlaneGCS Mapping:
- Coincident → GCS::ConstraintPointOnPoint
- Horizontal → GCS::ConstraintHorizontal
- Vertical → GCS::ConstraintVertical
- Parallel → GCS::ConstraintParallel
- Perpendicular → GCS::ConstraintPerpendicular
- Distance → GCS::ConstraintP2PDistance
- Radius → GCS::ConstraintRadius
- Angle → GCS::ConstraintL2LAngle
- Fixed → GCS::ConstraintFix
- Tangent → GCS::ConstraintTangent
- Concentric → GCS::ConstraintPointOnPoint (centers)
- Equal → GCS::ConstraintEqual
- Midpoint → GCS::ConstraintMidpoint
- Symmetric → GCS::ConstraintSymmetric
```

---

### Phase 3: Loop Detection Algorithms

**Priority: High**
**Estimated Complexity: High**
**Dependencies: Phase 2**

#### 3.1 Graph Construction
```cpp
// Build adjacency graph from sketch
struct GraphNode {
    EntityID pointId;
    std::vector<GraphEdge> edges;
};

struct GraphEdge {
    EntityID entityId;
    EntityID otherNode;
    bool isArc;
};

// Algorithm: O(n) pass through all entities
// Lines contribute 2 edges
// Arcs contribute 2 edges (endpoints)
```

#### 3.2 Cycle Detection (DFS)
```cpp
// Johnson's algorithm for finding all simple cycles
// Complexity: O((V+E)(C+1)) where C = number of cycles

std::vector<Wire> findCycles(const AdjacencyGraph& graph) {
    // 1. Find all SCCs (Tarjan's algorithm)
    // 2. For each SCC, enumerate simple cycles
    // 3. Build Wire from each cycle
}
```

#### 3.3 Area & Orientation Calculation
```cpp
// Shoelace formula for polygon area
double computeSignedArea(const std::vector<Vec2d>& polygon) {
    double area = 0.0;
    for (size_t i = 0; i < polygon.size(); ++i) {
        size_t j = (i + 1) % polygon.size();
        area += polygon[i].x * polygon[j].y;
        area -= polygon[j].x * polygon[i].y;
    }
    return area * 0.5;
}
// Positive = CCW, Negative = CW
```

#### 3.4 Hole Detection & Face Hierarchy
```cpp
// Point-in-polygon for containment
bool isPointInPolygon(const Vec2d& p, const Loop& loop) {
    // Ray casting algorithm
    // Count intersections with edges
}

// Build hierarchy: outer loops contain inner loops (holes)
std::vector<Face> buildFaceHierarchy(std::vector<Loop>& loops) {
    // 1. Sort by area (largest first)
    // 2. For each loop, find smallest containing loop
    // 3. Build parent-child relationships
}
```

---

### Phase 4: Rendering System

**Priority: High**
**Estimated Complexity: High**
**Dependencies: Phase 2, existing OpenGL infrastructure**

#### 4.1 Shader Programs
```glsl
// sketch_line.vert
uniform mat4 u_mvp;
uniform float u_lineWidth;
attribute vec2 a_position;
attribute vec4 a_color;
// Expand line to quad in geometry shader or vertex shader

// sketch_line.frag
uniform bool u_dashed;
uniform float u_dashLength;
varying float v_linePos;  // For dash pattern
```

#### 4.2 VBO Management
```cpp
// Batch geometry by type for efficient rendering
struct RenderBatch {
    GLuint vao;
    GLuint vbo;
    size_t vertexCount;
    GLenum primitiveType;
};

void SketchRenderer::buildVBOs() {
    // Collect all line vertices
    // Tessellate all arcs
    // Upload to GPU
}
```

#### 4.3 Constraint Icon Atlas
```
Icon Layout (16x16 per icon):
[Coincident][Horizontal][Vertical][Parallel]
[Perpendicular][Tangent][Concentric][Equal]
[Distance][Angle][Radius][Diameter]
[Fixed][Midpoint][Symmetric][Error]
```

#### 4.4 Adaptive Arc Tessellation
```cpp
int calculateArcSegments(double radius, double arcAngle, double zoom) {
    // More segments at higher zoom
    // Minimum 8, maximum 128
    double pixelArc = radius * arcAngle * zoom;
    int segments = std::clamp(
        static_cast<int>(pixelArc / TESSELLATION_ANGLE),
        MIN_SEGMENTS, MAX_SEGMENTS
    );
    return segments;
}
```

---

### Phase 5: Snap & Auto-Constrain

**Priority: Medium**
**Estimated Complexity: Medium**
**Dependencies: Phase 4**

#### 5.1 Snap Point Finding
```cpp
// Find all snap candidates within radius
std::vector<SnapResult> findAllSnaps(const Vec2d& cursor, const Sketch& sketch) {
    std::vector<SnapResult> results;

    // Check points (highest priority)
    for (auto* pt : sketch.getPoints()) {
        double dist = distance(cursor, pt->pos());
        if (dist < snapRadius_)
            results.push_back({SnapType::Vertex, pt->pos(), pt->id(), dist});
    }

    // Check endpoints, midpoints, centers
    // Check intersections
    // Check grid

    // Sort by priority then distance
    std::sort(results.begin(), results.end());
    return results;
}
```

#### 5.2 Auto-Constraint Detection
```cpp
// Per SPECIFICATION.md §5.14
struct AutoConstraint {
    ConstraintType type;
    EntityID entity1;
    EntityID entity2;
    double confidence;  // For UI preview
};

std::vector<AutoConstraint> detectAutoConstraints(
    const Vec2d& newPoint,
    const Sketch& sketch,
    EntityID drawingEntity
) {
    // Detect potential coincident
    // Detect horizontal/vertical alignments
    // Detect tangent/perpendicular to nearby
}
```

#### 5.3 Ghost Icon Preview
```
During drawing:
- Show semi-transparent constraint icons
- User can accept (click) or reject (move away)
- Icons show on all detected auto-constraints
```

---

### Phase 6: Sketch Tools

**Priority: Medium**
**Estimated Complexity: Medium**
**Dependencies: Phases 4, 5**

#### 6.1 Tool Interface
```cpp
class SketchTool {
public:
    virtual ~SketchTool() = default;
    virtual void onMousePress(const Vec2d& pos, Qt::MouseButton btn) = 0;
    virtual void onMouseMove(const Vec2d& pos) = 0;
    virtual void onMouseRelease(const Vec2d& pos, Qt::MouseButton btn) = 0;
    virtual void onKeyPress(Qt::Key key) = 0;
    virtual void render(SketchRenderer& renderer) = 0;
    virtual void activate() {}
    virtual void deactivate() {}
};
```

#### 6.2 Line Tool Implementation
```cpp
class LineTool : public SketchTool {
    enum State { Idle, FirstPoint, Drawing };
    State state_ = Idle;
    Vec2d startPoint_;

    void onMousePress(const Vec2d& pos, Qt::MouseButton btn) override {
        if (btn == Qt::LeftButton) {
            if (state_ == Idle) {
                startPoint_ = snap(pos);
                state_ = FirstPoint;
            } else {
                // Complete line
                sketch_->addLine(startPoint_, snap(pos));
                startPoint_ = snap(pos);  // Chain mode
            }
        }
    }
};
```

#### 6.3 Arc Tool (Three-Point)
```cpp
class ArcTool : public SketchTool {
    enum State { Idle, Start, End, Bulge };
    Vec2d start_, end_;

    // Three-point arc: Start, End, then drag to set curvature
    // Calculate center from three points using circle math
};
```

#### 6.4 Rectangle Tool
```cpp
// Constructs 4 lines with coincident and perpendicular constraints
```

---

### Phase 7: UI Integration

**Priority: Medium**
**Estimated Complexity: Medium**
**Dependencies: Phases 4, 5, 6**

#### 7.1 Sketch Mode Panel
```
UI Elements:
- Tool buttons (Line, Arc, Circle, Rectangle)
- Constraint buttons (when entities selected)
- DOF indicator
- Constraint list with edit/delete
- Expression editor for dimensions
```

#### 7.2 Dimension Editor
```cpp
// Click-to-edit dimensional constraints
// Per SPECIFICATION.md §5.15
class DimensionEditor : public QLineEdit {
    // Popup at constraint position
    // Parse expression: "10", "10+5", "width/2"
    // Support basic math: +, -, *, /, ()
};
```

#### 7.3 Constraint Conflict Dialog
```
When over-constrained:
- Show conflicting constraints list
- Suggest which to remove
- "Remove" button per constraint
- "Remove All Conflicts" button
```

---

## Algorithm Implementation Notes

### Critical Algorithms Requiring Careful Implementation

1. **PlaneGCS Direct Parameter Binding**
   - Must not copy values, use pointers
   - Thread safety considerations
   - Backup/restore mechanism

2. **Johnson's Cycle Enumeration**
   - Complex graph algorithm
   - Memory management for large sketches
   - Early termination for max loops

3. **Rubber-Band Dragging with Spring Resistance**
   - Per §5.13: Progressive resistance as constraints fight
   - Smooth visual feedback
   - Partial solve application

4. **Redundancy Analysis**
   - PlaneGCS provides some support
   - Need custom handling for user feedback
   - Conflict identification algorithm

### Performance Targets (from SPECIFICATION.md)

| Metric | Target |
|--------|--------|
| Solve time (≤100 entities) | <33ms (30 FPS) |
| Background threshold | >100 entities |
| Arc tessellation | 8-128 segments |
| Snap radius | 2mm |
| Solver tolerance | 1e-4mm |

---

## Testing Strategy

### Unit Tests
```cpp
// Per-component testing
TEST(SketchPoint, BasicOperations)
TEST(SketchLine, EndpointAccess)
TEST(ConstraintSolver, SimpleCoincident)
TEST(LoopDetector, SingleTriangle)
```

### Integration Tests
Cross-phase contracts and end-to-end checks (Phase 2 -> Phase 3):

- Contract: Solver output provides solved 2D geometry (points/arcs/lines) that LoopDetector consumes.
- Input: closed rectangle (4 lines, 4 points, fully constrained) -> Output: 1 outer loop, 0 holes.
- Input: rectangle with inner circle (construction = false) -> Output: 1 outer loop + 1 inner loop (hole).
- Input: open polyline (missing edge) -> Output: 0 loops.
- Input: arc + line chain forming closed profile -> Output: 1 loop with mixed edges.

Planned automated tests:
- `tests/integration/sketch_solver_loop.cpp`:
  - `TEST(SketchSolverLoop, RectangleOneLoop)`
  - `TEST(SketchSolverLoop, RectangleWithHole)`
  - `TEST(SketchSolverLoop, OpenPolylineNoLoop)`
  - `TEST(SketchSolverLoop, ArcLineClosedLoop)`
- `tests/integration/sketch_renderer_contract.cpp`:
  - `TEST(SketchRendererContract, BoundsMatchSketch)`

### Visual Tests
Automated snapshots (Phase 4):
- Framework: QtTest + QOpenGLWidget capture to QImage.
- Baselines stored in `tests/visual/baselines/`.
- Diff tool: pixel diff with threshold (e.g., 1% pixels > 2/255 delta fails).

Manual verification (acceptance criteria):
- Constraint icons render at expected positions for each constraint type.
- Dragging a constrained point shows spring resistance when constraints conflict.
- DOF indicator color transitions: blue (under), green (fully), orange (over), red (conflict).

Repeatable scenes:
- `tests/visual/scenes/rectangle_constrained.json` -> Expect green state, icons visible.
- `tests/visual/scenes/overconstrained.json` -> Expect orange/red highlight.

Performance regression checks:
- `tests/bench/bench_sketch_solver.cpp`:
  - 100 entities: solve < 33ms (mean over 200 iterations).
  - 500 entities: solve < 200ms (mean over 50 iterations).
- CI alerts: fail build if thresholds exceeded; emit timing log.

---

## Design Decisions (ADR)

Open questions are tracked in `docs/adr/ADR-001-sketch-phase2.md`. Phase 2 should
not start until the ADR is signed off.

---

## File Structure Summary

```
src/core/
├── sketch/
│   ├── SketchTypes.h           [COMPLETE]
│   ├── SketchEntity.h          [COMPLETE]
│   ├── SketchPoint.h           [COMPLETE]
│   ├── SketchLine.h            [COMPLETE]
│   ├── SketchArc.h             [COMPLETE]
│   ├── SketchCircle.h          [COMPLETE]
│   ├── SketchConstraint.h      [COMPLETE]
│   ├── Sketch.h                [COMPLETE]
│   ├── Sketch.cpp              [PHASE 2]
│   ├── SketchRenderer.h        [COMPLETE]
│   ├── SketchRenderer.cpp      [PHASE 4]
│   ├── SketchTool.h            [PHASE 6]
│   ├── tools/
│   │   ├── LineTool.h/.cpp     [PHASE 6]
│   │   ├── ArcTool.h/.cpp      [PHASE 6]
│   │   └── ...
│   ├── constraints/
│   │   └── Constraints.h       [COMPLETE]
│   └── solver/
│       ├── ConstraintSolver.h  [COMPLETE - interface]
│       ├── ConstraintSolver.cpp[PHASE 2]
│       └── PlaneGCSWrapper.h   [PHASE 2]
├── loop/
│   ├── LoopDetector.h          [COMPLETE - interface]
│   └── LoopDetector.cpp        [PHASE 3]
└── CMakeLists.txt              [COMPLETE]
```

---

## Next Steps

| Item | Estimate | Timeline | Roles |
|------|----------|----------|-------|
| Immediate: architecture review | S (2-3 days) | Sprint P2-0 (week 0) | Lead engineer x1, PM x1 |
| Phase 2 Start: PlaneGCS setup | M (2 weeks) | Sprint P2-1 to P2-2 | Senior engineer x1 |
| Parallel: sketch UI mockups | S (1 week) | Sprint P2-1 | Designer x1, PM x1 |
| Risk: PlaneGCS API adaptation | M (1-2 weeks) | Sprint P2-2 (buffer) | Senior engineer x1 |

Risk mitigation for PlaneGCS API adaptation:
1. Feasibility spike: build a minimal solver prototype against the target PlaneGCS version.
2. API compatibility assessment: map required constraints to PlaneGCS types and document gaps.
3. Fallback plan: define adapter/shim layer if PlaneGCS APIs diverge.
4. Effort estimate and rollback criteria: decide go/no-go if gaps exceed 1 sprint.
5. Ownership: assign spike owner and target completion before Phase 2 kickoff.

---

*Document Version: 1.0*
*Last Updated: Phase 1 Complete*
