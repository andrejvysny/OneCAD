# OneCAD Sketch System Implementation Plan

Status: **Phase 3 Complete** - Loop Detection Implemented

**Last Updated:** 2026-01-03 *(Corrected based on actual codebase analysis)*

---

## Implementation Status Overview

### ✅ COMPLETED - Phase 1: Architecture Foundation

| Component | File | Status |
|-----------|------|--------|
| Type Definitions | `src/core/sketch/SketchTypes.h` | ✅ Complete |
| Entity Base Class | `src/core/sketch/SketchEntity.h/cpp` | ✅ Complete |
| Point Entity | `src/core/sketch/SketchPoint.h/cpp` | ✅ Complete |
| Line Entity | `src/core/sketch/SketchLine.h/cpp` | ✅ Complete |
| Arc Entity | `src/core/sketch/SketchArc.h/cpp` | ✅ Complete |
| Circle Entity | `src/core/sketch/SketchCircle.h/cpp` | ✅ Complete |
| Ellipse Entity | *Declared in API* | ❌ **NOT IMPLEMENTED** |
| Constraint Base | `src/core/sketch/SketchConstraint.h/cpp` | ✅ Complete |
| Concrete Constraints | `src/core/sketch/constraints/Constraints.h/cpp` | ✅ Complete (454 lines) |
| Sketch Manager | `src/core/sketch/Sketch.h/cpp` | ✅ Complete |
| Solver Interface | `src/core/sketch/solver/ConstraintSolver.h` | ✅ Complete |
| Loop Detector | `src/core/loop/LoopDetector.h` | ✅ Complete (Interface + Impl) |
| **Renderer FULL IMPLEMENTATION** | `src/core/sketch/SketchRenderer.h/cpp` | ✅ **COMPLETE (1200 lines)** |
| CMake Configuration | `src/core/CMakeLists.txt` | ✅ Complete |

### ✅ COMPLETED - Phase 2: PlaneGCS Integration & Core Implementation

| Component | File | Status |
|-----------|------|--------|
| PlaneGCS Library | `third_party/planegcs/` | ✅ Complete |
| Constraint Solver | `src/core/sketch/solver/ConstraintSolver.cpp` | ✅ Complete (1015 lines) |
| Solver Adapter | `src/core/sketch/solver/SolverAdapter.h/cpp` | ✅ Complete |
| Sketch.cpp | `src/core/sketch/Sketch.cpp` | ✅ Complete (903 lines) |
| Solve & Drag | `Sketch::solve()`, `Sketch::solveWithDrag()` | ✅ Complete |
| DOF Calculation | `Sketch::getDegreesOfFreedom()` | ✅ Complete |
| Conflict Detection | `ConstraintSolver::findRedundantConstraints()` | ✅ Complete |
| Serialization | `Sketch::toJson()`, `Sketch::fromJson()` | ✅ Complete |

### PlaneGCS-Mapped Constraints (9 types integrated)

| OneCAD Constraint | PlaneGCS Constraint | Status |
|-------------------|---------------------|--------|
| Coincident | `GCS::addConstraintP2PCoincident` | ✅ |
| Horizontal | `GCS::addConstraintHorizontal` | ✅ |
| Vertical | `GCS::addConstraintVertical` | ✅ |
| Parallel | `GCS::addConstraintParallel` | ✅ |
| Perpendicular | `GCS::addConstraintPerpendicular` | ✅ |
| Distance | `GCS::addConstraintP2PDistance` | ✅ |
| Angle | `GCS::addConstraintL2LAngle` | ✅ |
| Radius | `GCS::addConstraintCircleRadius/ArcRadius` | ✅ |
| Tangent | `GCS::addConstraintTangent` (all combinations) | ✅ |
| Equal | `GCS::addConstraintEqualLength/EqualRadius` | ✅ |

**Additional Constraints (Not PlaneGCS-mapped):**
- Fixed (2 DOF removed) - ✅ Implemented in Constraints.cpp
- Midpoint (2 DOF removed) - ✅ Implemented in Constraints.cpp

### ✅ COMPLETED - Phase 3: Loop Detection Algorithms

| Component | File | Status |
|-----------|------|--------|
| Loop Detector | `src/core/loop/LoopDetector.h/cpp` | ✅ Complete (990 lines) |
| Adjacency Graph | `src/core/loop/AdjacencyGraph.h/cpp` | ✅ Complete |
| Face Builder | `src/core/loop/FaceBuilder.h/cpp` | ✅ Complete |
| DFS Cycle Detection | `LoopDetector::findCycles()` | ✅ Complete |
| Area Calculation | `computeSignedArea()` (Shoelace) | ✅ Complete |
| Point-in-Polygon | `isPointInPolygon()` (Ray casting) | ✅ Complete |
| Face Hierarchy | `buildFaceHierarchy()` | ✅ Complete |
| Wire Building | `buildWire()` | ✅ Complete |
| Loop Validation | `validateLoop()` | ✅ Complete |

### ✅ COMPLETED - Phase 4: Rendering System (Previously Undocumented)

**Actual Status:** **FULLY IMPLEMENTED** ✅

| Component | File | Lines | Status |
|-----------|------|-------|--------|
| **SketchRenderer Implementation** | `src/core/sketch/SketchRenderer.cpp` | **1200** | ✅ Complete |
| Inline GLSL Shaders | Embedded in SketchRenderer.cpp | ~120 | ✅ Complete |
| VBO Batching System | `buildVBOs()`, `render()` methods | — | ✅ Complete |
| Adaptive Arc Tessellation | Implemented (8+ segments/π) | — | ✅ Complete |
| Selection State Colors | Blue/Green/Orange feedback | — | ✅ Complete |
| Preview Rendering | Line/Circle/Rectangle preview | — | ✅ Complete |

**Key Implementation Details:**
- **GLSL Shaders**: Inline vertex & fragment shaders (OpenGL 4.1 Core)
- **Geometry Batching**: Separate VBOs for lines, points, construction geometry
- **State-based Coloring**: Hover, selected, construction modes
- **Constraint Icons**: Positioned via `getIconPosition()` (texture rendering pending)

### ✅ PARTIAL - Phase 5: Sketch Tools

| Component | File | Lines | Status |
|-----------|------|-------|--------|
| Tool Base & Manager | `SketchTool.h`, `SketchToolManager.h/cpp` | — | ✅ Complete |
| Line Tool | `tools/LineTool.h/cpp` | 94 | ✅ Complete (polyline mode) |
| Circle Tool | `tools/CircleTool.h/cpp` | 95 | ✅ Complete (center-radius) |
| Rectangle Tool | `tools/RectangleTool.h/cpp` | 125 | ✅ Complete (auto-constrained) |
| **Arc Tool** | — | — | ❌ **NOT IMPLEMENTED** |
| **Ellipse Tool** | — | — | ❌ **NOT IMPLEMENTED** |

### AWAITING IMPLEMENTATION

| Component | Phase | Complexity | Priority |
|-----------|-------|------------|----------|
| Snap & Auto-Constrain | 6 | Medium | High |
| Arc & Ellipse Tools | 5 (cont'd) | Medium | Medium |
| UI Integration | 7 | Medium | Medium |
| DOF Color Visualization | 7 | Low | Low |



## Next Implementation Priorities

### Immediate (Phase 6 - Snap & Auto-Constrain)
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

Files to Create:
- `src/core/sketch/SnapManager.h/cpp`
- `src/core/sketch/AutoConstrainer.h/cpp`

---

### Phase 6: Sketch Tools

**Priority: High**
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

Files to Create:
- `src/core/sketch/tools/SketchTool.h`
- `src/core/sketch/tools/LineTool.h/cpp`
- `src/core/sketch/tools/RectangleTool.h/cpp`
- `src/core/sketch/tools/CircleTool.h/cpp`
- `src/core/sketch/tools/ArcTool.h/cpp`

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

#### 6.3 Rectangle Tool
```cpp
// Constructs 4 lines + 4 points
// Auto-applies horizontal/vertical constraints
// Auto-applies coincident constraints at corners
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
- DOF indicator with color coding
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

1. **PlaneGCS Direct Parameter Binding** ✅ DONE
   - Uses pointers directly to sketch coordinates
   - Backup/restore mechanism implemented
   - Thread safety via atomic flags

2. **Johnson's Cycle Enumeration** (Phase 3)
   - Complex graph algorithm
   - Memory management for large sketches
   - Early termination for max loops

3. **Rubber-Band Dragging with Spring Resistance**
   - Per §5.13: Progressive resistance as constraints fight
   - Basic implementation in `solveWithDrag()`
   - Needs damping factor refinement

4. **Redundancy Analysis** ✅ DONE
   - PlaneGCS `getRedundant()` integrated
   - Conflict identification working

### Performance Targets (from SPECIFICATION.md)

| Metric | Target | Current |
|--------|--------|---------|
| Solve time (≤100 entities) | <33ms (30 FPS) | ✅ Achievable |
| Background threshold | >100 entities | Implemented (basic) |
| Arc tessellation | 8-128 segments | Not implemented |
| Snap radius | 2mm | Not implemented |
| Solver tolerance | 1e-4mm | ✅ Configured |

---

## Testing Strategy

### Unit Tests (Existing Prototypes)
```
tests/prototypes/
├── proto_sketch_geometry.cpp    # Entity creation tests
├── proto_sketch_constraints.cpp # Constraint validation
├── proto_sketch_solver.cpp      # Solver integration
└── proto_planegcs_integration.cpp # Direct PlaneGCS test
```

### Integration Tests (Needed)
Cross-phase contracts (Phase 2 → Phase 3):

- Contract: Solver output provides solved 2D geometry that LoopDetector consumes.
- Input: closed rectangle (4 lines, 4 points) → Output: 1 outer loop.
- Input: rectangle with inner circle → Output: 1 outer + 1 inner loop (hole).
- Input: open polyline → Output: 0 loops.
- Input: arc + line chain forming closed profile → Output: 1 loop with mixed edges.

Planned tests:
- `tests/integration/sketch_solver_loop.cpp`
- `tests/integration/sketch_renderer_contract.cpp`

### Performance Tests
- `tests/bench/bench_sketch_solver.cpp`:
  - 100 entities: solve < 33ms
  - 500 entities: solve < 200ms

---

## File Structure Summary

```
src/core/
├── sketch/
│   ├── SketchTypes.h           [✅ COMPLETE]
│   ├── SketchEntity.h/cpp      [✅ COMPLETE]
│   ├── SketchPoint.h/cpp       [✅ COMPLETE]
│   ├── SketchLine.h/cpp        [✅ COMPLETE]
│   ├── SketchArc.h/cpp         [✅ COMPLETE]
│   ├── SketchCircle.h/cpp      [✅ COMPLETE]
│   ├── SketchEllipse.h/cpp     [❌ NOT IMPLEMENTED - Declared in API only]
│   ├── SketchConstraint.h/cpp  [✅ COMPLETE]
│   ├── Sketch.h/cpp            [✅ COMPLETE] (903 lines)
│   ├── SketchRenderer.h        [✅ COMPLETE]
│   ├── SketchRenderer.cpp      [✅ COMPLETE] (1200 lines with GLSL)
│   ├── SketchTool.h            [✅ COMPLETE]
│   ├── SnapManager.h/cpp       [❌ NOT IMPLEMENTED]
│   ├── AutoConstrainer.h/cpp   [❌ NOT IMPLEMENTED]
│   ├── tools/
│   │   ├── SketchToolManager.h/cpp [✅ COMPLETE]
│   │   ├── LineTool.h/cpp      [✅ COMPLETE]
│   │   ├── RectangleTool.h/cpp [✅ COMPLETE]
│   │   ├── CircleTool.h/cpp    [✅ COMPLETE]
│   │   ├── ArcTool.h/cpp       [❌ NOT IMPLEMENTED]
│   │   └── EllipseTool.h/cpp   [❌ NOT IMPLEMENTED]
│   ├── constraints/
│   │   └── Constraints.h/cpp   [✅ COMPLETE] (1033 lines)
│   └── solver/
│       ├── ConstraintSolver.h  [✅ COMPLETE]
│       ├── ConstraintSolver.cpp[✅ COMPLETE] (1015 lines)
│       ├── SolverAdapter.h     [✅ COMPLETE]
│       └── SolverAdapter.cpp   [✅ COMPLETE]
├── loop/
│   ├── LoopDetector.h          [✅ COMPLETE]
│   ├── LoopDetector.cpp        [✅ COMPLETE] (990 lines)
│   ├── AdjacencyGraph.h/cpp    [✅ COMPLETE]
│   └── FaceBuilder.h/cpp       [✅ COMPLETE]
└── CMakeLists.txt              [✅ COMPLETE]

third_party/
└── planegcs/                   [✅ COMPLETE]
```
│   ├── SketchArc.h/cpp         [✅ COMPLETE]
│   ├── SketchCircle.h/cpp      [✅ COMPLETE]
│   ├── SketchConstraint.h/cpp  [✅ COMPLETE]
│   ├── Sketch.h/cpp            [✅ COMPLETE] (894 lines)
│   ├── SketchRenderer.h        [✅ INTERFACE]
│   ├── SketchRenderer.cpp      [PHASE 4]
│   ├── SketchTool.h            [PHASE 6]
│   ├── SnapManager.h/cpp       [PHASE 5]
│   ├── AutoConstrainer.h/cpp   [PHASE 5]
│   ├── tools/
│   │   ├── LineTool.h/cpp      [PHASE 6]
│   │   ├── RectangleTool.h/cpp [PHASE 6]
│   │   ├── ArcTool.h/cpp       [PHASE 6]
│   │   └── CircleTool.h/cpp    [PHASE 6]
│   ├── constraints/
│   │   └── Constraints.h/cpp   [✅ COMPLETE] (884 lines)
│   └── solver/
│       ├── ConstraintSolver.h  [✅ COMPLETE]
│       ├── ConstraintSolver.cpp[✅ COMPLETE] (981 lines)
│       ├── SolverAdapter.h     [✅ COMPLETE]
│       └── SolverAdapter.cpp   [✅ COMPLETE]
├── loop/
│   ├── LoopDetector.h          [✅ COMPLETE]
│   ├── LoopDetector.cpp        [✅ COMPLETE] (969 lines)
│   └── AdjacencyGraph.h/cpp    [✅ COMPLETE]
└── CMakeLists.txt              [✅ COMPLETE]

third_party/
└── planegcs/                   [✅ COMPLETE]
```

---

## Next Steps

### Immediate Priority (Blocking for UI)
1. **Implement SketchRenderer.cpp** — Visual feedback for sketch editing.
2. **Create LineTool with snapping** — Basic interactive drawing.
3. **Add DOF color visualization** — Green/Blue/Orange feedback.

### Short-term
4. Add Rectangle, Circle, Arc tools.
5. Implement SnapManager for object snapping.
6. Add auto-constraint detection.

### Medium-term
7. Create dimension editor widget.
8. Implement constraint icons (texture atlas).
9. Add 30 FPS throttling to interactive solver.

---

## Unresolved Questions

1. **Arc tessellation during loop detection** — Currently using 8+ segments per π radians. Sufficient?
2. **Performance threshold for background solve** — Keep 100 entities or adjust based on profiling?
3. **Constraint icon rendering** — Texture atlas or individual billboards?
4. **Snap priority for overlapping candidates** — Distance-only or include geometric priority?
5. **Missing constraint types** — Fixed, Midpoint, OnCurve, Concentric, Symmetric, Diameter not wired to PlaneGCS. Defer to v1.1?
6. **MakeFace integration** — LoopDetector returns Face structs. Need OCCT `BRepBuilderAPI_MakeFace` wrapper.

---

*Document Version: 3.0*
*Last Updated: 2026-01-03*
