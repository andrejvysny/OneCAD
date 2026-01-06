# OneCAD Detailed Implementation Roadmap

This document outlines the phased implementation strategy for OneCAD, ensuring a robust foundation before adding complexity.

**Last Updated:** 2026-01-06

---

## Current Status Summary

| Phase | Status | Progress |
|-------|--------|----------|
| Phase 1.1 Project & Rendering | In Progress | ~95% |
| Phase 1.2 OCCT Kernel | Partial | ~40% |
| Phase 1.3 Topological Naming | Substantial | ~90% |
| Phase 1.4 Command & Document | In Progress | ~75% |
| Phase 2 Sketching Engine | **Complete** | **100%** |
| Phase 3 Solid Modeling | In Progress | ~25% |
| Phase 4 Advanced Modeling | Not Started | 0% |
| Phase 5 Optimization & Release | Not Started | 0% |

**Key Achievements:**
- ✅ PlaneGCS solver fully integrated (1450 LOC, all 15 constraint types working)
- ✅ Loop detection complete (1985 LOC with DFS, shoelace area, hole detection)
- ✅ FaceBuilder OCCT integration complete (524 LOC cpp + 195 LOC header)
- ✅ All 7 sketch tools production-ready (2618 LOC total)
- ✅ SketchRenderer complete (2472 LOC with VBO, adaptive tessellation)
- ✅ SnapManager complete (1166 LOC, 8 snap types)
- ✅ AutoConstrainer complete (1091 LOC, 7 inference rules)
- ✅ UI integration complete (ContextToolbar, ConstraintPanel, DimensionEditor, ViewCube)
- ✅ Adaptive Grid3D spacing (pixel-targeted minor/major grid)
- ✅ Selection system with deep select + click cycling
- ✅ Mesh-based picking + 3D selection overlays (face/edge/vertex/body)
- ✅ SceneMeshStore + BodyRenderer (Shaded+Edges + preview meshes)
- ✅ Extrude v1a complete (SketchRegion → new body, preview, draft angle working)
- ✅ CommandProcessor with full transaction support (undo/redo/group commands)

---

## Phase 1: Core Foundation & Topological Infrastructure
**Focus:** establishing the critical architectural bedrock—specifically the geometry kernel integration and the topological naming system to prevent future refactoring.

### 1.1 Project & Rendering Setup
- [x] **Build System**: CMake with Qt6, OCCT, Eigen3, PlaneGCS. **Complete** (all deps linked).
- [x] **App Shell**: Main window layout (MainWindow, Viewport, Navigator, Inspector). **Complete**.
- [x] **OpenGL Viewport**: OpenGLWidgets-based rendering pipeline. **Complete** (1427 LOC).
- [ ] **Qt RHI Viewport**: Metal-based rendering pipeline (deferred to optimization phase).
- [~] **Camera Controls**: Camera3D with Orbit, Pan, Zoom, standard views (270 LOC). *Missing: inertia physics, sticky pivot.*
- [x] **Grid System**: Grid3D adaptive spacing implemented (pixel-targeted minor/major tiers).
- [x] **Body Renderer**: Shaded + Edges renderer (SceneMeshStore-backed) with preview mesh support.

### 1.2 OCCT Kernel Integration
- [~] **Shape Wrappers**: Basic ElementMap structure exists. *Missing: full `onecad::kernel::Shape` decoupled wrapper.*
- [~] **Tessellation Cache**: OCCT triangulation → SceneMeshStore for rendering/picking (implemented; LOD and progressive refinement pending).
- [ ] **Geometry Factory**: Utilities for creating primitives (Box, Cylinder, Plane).
- [ ] **BREP Utilities**: Explorer for traversing Faces, Edges, Vertices.

### 1.3 Topological Naming System (ElementMap)
*Critical Path Item: Must be rigorous.*
- [x] **ElementMap Architecture**: **Complete** (955 LOC header-only).
    - ✅ Deterministic descriptor-based IDs (FNV-1a hash, quantized geometry)
    - ✅ 14-field enhanced descriptor (surface/curve types, normals, adjacency)
    - ✅ Split handling with sibling IDs
- [x] **Evolution Tracking**: **Complete**.
    - ✅ Full OCCT history integration (`BRepAlgoAPI` Modified/Generated/Deleted)
    - ✅ Parent→child source tracking
- [x] **Serialization**: **Complete** (text format, versioned, round-trip tested).
- [~] **Testing Suite**: Core validated, advanced scenarios pending.
    - ✅ 5/5 prototype tests passing (basic cut, split, determinism, serialization)
    - ⚠️ Missing: chain ops, merge scenarios, pattern operations
- [x] **Document Integration**: ElementMap wired into Document (body registration + tessellation IDs).

### 1.4 Command & Document Infrastructure
- [~] **Document Model**: Owns sketches + bodies + operations list; persistence/history replay still pending.
- [x] **Selection Manager**: Ray-casting picking, deep select, click cycling, hover/selection highlights.
- [x] **Command Processor**: Execute/undo/redo with full transaction support (begin/end/cancel, command grouping). 197 LOC.
- [x] **AddBodyCommand**: Working command for adding bodies to document.
- [~] **Additional Commands**: Only AddBodyCommand implemented; other modeling commands pending.

---

## Phase 2: Sketching Engine
**Focus:** Creating a robust 2D constraint-based sketcher.
**Status:** ✅ **COMPLETE** (100%) — PlaneGCS + Loop Detection + All Tools + UI Integration

### 2.1 Sketch Infrastructure ✅ COMPLETE
- [x] **Sketch Entity**: Complete data model (Sketch.cpp 894 lines, Sketch.h 476 lines).
- [x] **Workplane System**: Coordinate transforms (3D World <-> 2D Local) via `SketchPlane`. **Complete**.
- [x] **Snap System**: SnapManager **complete** (1166 LOC).
    - ✅ All 8 snap types: Vertex, Endpoint, Midpoint, Center, Quadrant, Intersection, OnCurve, Grid
    - ✅ 2mm snap radius (zoom-independent)
    - ✅ Priority-based selection

### 2.2 Sketch Tools ✅ COMPLETE
- [x] **All 7 Tools**: Line (322 LOC), Arc (369 LOC), Circle (226 LOC), Rectangle (206 LOC), Ellipse (268 LOC), Mirror (444 LOC), Trim (219 LOC).
    - ✅ Total: 2618 LOC production code
- [x] **Construction Geometry**: Toggling construction/normal mode via `isConstruction` flag. **Complete**.
- [x] **Visual Feedback**: SketchRenderer **complete** (1955 LOC) with VBO rendering, adaptive tessellation, viewport culling.

### 2.3 Constraint Solver (PlaneGCS) ✅ COMPLETE
**STATUS: PRODUCTION-READY** (1450 LOC total)
- [x] **PlaneGCS Library**: Linked as static lib (42MB libplanegcs.a), CMake configured, runtime verified.
- [x] **ConstraintSolver.cpp**: Production implementation (1014 LOC + 436 header):
    - ✅ Direct parameter binding (zero-copy solving)
    - ✅ Full GCS::System wrapper with all primitives (Point, Line, Arc, Circle)
    - ✅ 15 constraint types fully translated
    - ✅ DogLeg algorithm (default) with LM/BFGS fallback
    - ✅ Backup/restore for failed solves
    - ✅ Redundancy and conflict detection
    - ✅ DOF calculation
- [x] **Constraint Types**: All 15 implemented in Constraints.h (1875 LOC):
    - Positional: Fixed, Midpoint, Coincident, PointOnCurve
    - Geometric: Horizontal, Vertical, Parallel, Perpendicular, Tangent, Concentric, Equal
    - Dimensional: Distance, Angle, Radius, Diameter
- [x] **SolverAdapter**: Sketch → PlaneGCS translation (85 LOC).
- [x] **Interactive Solver**: Drag solving with spring resistance. **Complete**.
- [x] **AutoConstrainer**: Inference engine **complete** (1091 LOC, 7 rules, ±5° tolerance).
- [~] **DOF Visualization**: DOF calculation works. *UI color-coding partial (SketchRenderer has logic, needs full UI wiring).*

### 2.4 Loop Detection & Face Creation ✅ COMPLETE
**STATUS: PRODUCTION-READY** (2704 LOC total implementation)
- [x] **LoopDetector**: Full implementation (1985 LOC: 381 header + 1506 cpp + 98 AdjacencyGraph).
    - ✅ DFS cycle detection algorithm (947-1015)
    - ✅ Planar graph face extraction with half-edge structure
    - ✅ Segment intersection planarization
    - ✅ Arc/circle adaptive tessellation
- [x] **Wire, Loop, Face Structures**: Complete data model with all methods.
- [x] **Algorithms**: All production-ready:
    - ✅ Graph-based adjacency analysis
    - ✅ Shoelace signed area calculation
    - ✅ Ray casting point-in-polygon
    - ✅ Face hierarchy with hole detection
    - ✅ Orientation correction (CCW outer, CW holes)
- [x] **FaceBuilder**: OCCT bridge **complete** (719 LOC).
    - ✅ Converts Loop → `TopoDS_Face` with holes
    - ✅ Wire gap repair via `ShapeFix_Wire`
    - ✅ Plane transformations (2D → 3D)
- [x] **Testing**: 2 prototype tests passing (proto_loop_detector, proto_face_builder).
- [x] **Production Integration**: Active use in SketchRenderer for region visualization.

---

## Phase 3: Solid Modeling Operations
**Focus:** Enabling 3D geometry creation and manipulation.
**Status:** In Progress

### 3.1 Feature Management
- [ ] **Feature History Tree**: Parametric history management.
- [ ] **Dependency Graph**: Tracking relationships between sketches and features.
- [ ] **Regeneration Engine**: Replaying history on modification.

### 3.2 Core Operations
- [x] **Extrude v1a** (282 LOC):
    - SketchRegion → new body with preview + auto-commit on drag release.
    - Draft angle fully working (BRepOffsetAPI_DraftAngle integration).
    - Distance indicator with direction flip on negative drag.
    - Uses FaceBuilder + BRepPrimAPI_MakePrism.
    - Creates AddBodyCommand + OperationRecord.
- [~] **Extrude v1b** (pending):
    - Face input (not just SketchRegion).
    - Smart boolean detection (Add/Cut based on direction).
    - Boolean override UI badge.
- [ ] **Revolve**: Axis selection, angle parameters.
- [ ] **Boolean Operations**: Union, Subtract, Intersect (Multi-body).

### 3.3 Direct Modeling (Hybrid)
- [ ] **Push/Pull**:
    - Face detection under cursor.
    - Context-aware extrude/offset.
- [ ] **Direct Face Manipulation**: Moving faces with valid topology.

---

## Phase 4: Advanced Modeling & Refinement
**Focus:** Adding depth to modeling capabilities and UI polish.
**Status:** Not Started

### 4.1 Modification Tools
- [ ] **Fillet/Chamfer**: Variable radius, edge chain selection.
- [ ] **Shell**: Hollow solids with thickness.
- [ ] **Patterns**: Linear and Circular duplication features.

### 4.2 UI/UX Polish
- [ ] **Contextual Toolbar**: Floating tool palette near selection.
- [ ] **Property Inspector**: Fully functional attribute editor.
- [ ] **Visual Styles**: Wireframe, Shaded, Shaded with Edges.
- [ ] **Dark/Light Themes**: Full UI consistency.

### 4.3 I/O & Persistence
- [ ] **Native File Format**: Full serialization of Document, ElementMap, and History.
- [ ] **STEP Support**: Import/Export with interoperability.

---

## Phase 5: Optimization & Release
**Focus:** Performance tuning and stability.
**Status:** Not Started

### 5.1 Performance
- [ ] **Tessellation Control**: Adjustable LOD (Coarse/Fine).
- [ ] **Background Processing**: Threading complex boolean ops.
- [ ] **Rendering Optimization**: Instance rendering for patterns.

### 5.2 Stability & Release
- [ ] **Crash Recovery**: Autosave and backup systems.
- [ ] **Memory Management**: Cleanup of large OCCT structures.
- [ ] **Packaging**: macOS Bundle creation and signing.

---

## Prototype Tests

| Test | Status | Description |
|------|--------|-------------|
| `proto_tnaming.cpp` | Exists | Basic topological naming |
| `proto_elementmap_rigorous.cpp` | Exists | ElementMap validation |
| `proto_custom_map.cpp` | Exists | Custom mapping tests |
| `proto_sketch_constraints.cpp` | Exists | Constraint system tests |
| `proto_sketch_geometry.cpp` | Exists | Geometry entity tests |
| `proto_sketch_solver.cpp` | Exists | Solver integration |
| `proto_planegcs_integration.cpp` | Exists | PlaneGCS direct test |
| `proto_loop_detector.cpp` | Exists | Loop detection tests |

---

## Legend

- [x] Complete
- [~] Partial / In Progress
- [ ] Not Started

---

## Priority Recommendations

### ✅ COMPLETED SINCE LAST UPDATE
1. ~~Implement SketchRenderer~~ → **DONE** (1955 LOC production)
2. ~~Create MakeFace wrapper~~ → **DONE** (FaceBuilder 719 LOC)
3. ~~Add Rectangle/Ellipse tools~~ → **DONE** (all 7 tools complete)
4. ~~Implement SnapManager~~ → **DONE** (1166 LOC, 8 snap types)
5. ~~Fix Grid3D adaptive spacing~~ → **DONE** (pixel-targeted minor/major tiers)
6. ~~Integrate ElementMap into Document~~ → **DONE** (body registration + tessellation IDs)
7. ~~Implement Selection Manager~~ → **DONE** (deep select, click cycling, 3D picking)
8. ~~Add Shaded + Edges renderer~~ → **DONE** (SceneMeshStore-backed)
9. ~~Implement Extrude v1a~~ → **DONE** (SketchRegion → new body, preview, auto-commit)
10. ~~Add CommandProcessor~~ → **DONE** (execute/undo/redo + AddBodyCommand)

### Immediate Next Steps (High Priority for Phase 3)
1. **Extrude v1b smart boolean** — Add/Cut detection based on extrude direction + override enum plumbed to UI.
2. **Face extrude input** — Extend ExtrudeTool to accept face selection (not just SketchRegion).
3. **Revolve operation** — Axis selection + angle parameter + preview.
4. **Boolean operations** — Union/Subtract/Intersect with multi-body selection.
5. **Native `.onecad` save/load v0** — BREP + ElementMap + operation list serialization.

### Short-term (Medium Priority - UX Polish)
1. Add Camera inertia physics and sticky pivot (270 LOC existing base).
2. Complete DOF color-coding UI wiring (logic exists in SketchRenderer).
3. Wire PropertyInspector into UI (88 LOC stub exists, not integrated).
4. Add contextual tool badges (boolean override, direction flip).

### Blocking Items for Phase 3
- ✅ ~~MakeFace wrapper~~ → COMPLETE (FaceBuilder 524 LOC)
- ✅ ~~SketchRenderer~~ → COMPLETE (2472 LOC)
- ✅ ~~ElementMap Document integration~~ → COMPLETE
- ✅ ~~Grid3D adaptive spacing~~ → COMPLETE
- ✅ ~~Extrude v1a~~ → COMPLETE (282 LOC, draft angle working)
- ✅ ~~CommandProcessor~~ → COMPLETE (197 LOC, transactions)
- ⚠️ **History replay + persistence** → REQUIRED for robust parametric workflows
- ⚠️ **I/O layer** → Not implemented (src/io/ empty)

---

*Document Version: 4.2*
*Last Updated: 2026-01-06*
*Major Update: Verified Extrude v1a with draft angle, CommandProcessor with transactions, updated LOC counts*
