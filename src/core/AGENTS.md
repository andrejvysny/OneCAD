# Core Guidelines

## Scope
- Sketch entities/constraints/tools, SnapManager, AutoConstrainer, SketchRenderer.
- Solver adapter to PlaneGCS, DOF tracking, JSON serialization.
- LoopDetector + FaceBuilder bridge to OCCT; modeling helpers (EdgeChainer, BooleanOperation).

## Sketch fundamentals
- EntityID/ConstraintID are UUID strings; never reuse; keep entities non-copyable, movable.
- SketchPlane::XY() uses intentional non-standard axes (Sketch X→World Y+, Sketch Y→World X-, Normal→World Z+); do not “fix”.
- Construction flag travels through rendering, snapping, and loop detection; preserve it on copies/moves.

## Solver integration (ConstraintSolver)
- PlaneGCS binding is by raw pointer to entity coordinates; lifetime must outlive solver; add entities before constraints.
- Default config: DogLeg, tolerance 1e-4 mm, maxIterations 100; detectRedundant on. Respect applyPartialSolution=false unless UI explicitly wants partials.
- Maintain ordering stability when rebuilding the system; bindings rely on consistent pointer layout.
- On solve failure, coordinates must stay unchanged; rollback/backup before solve if mutating outside PlaneGCS.

## Constraints
- Implementations in constraints/ remove specific DOF as documented; keep degreesRemoved accurate for DOF reporting.
- Constraints reference entities by ID; validate existence before binding to solver; guard against null handles.
- Keep getIconPosition lightweight; used every frame for rendering.

## SnapManager
- Default snap radius 2mm in sketch coordinates; priority order must remain: Vertex > Endpoint > Midpoint > Center > Quadrant > Intersection > OnCurve > Grid (then inference hints).
- Supports external 3D-projected geometry via setExternalGeometry; keep cleared when no longer valid.
- Respect per-type enable flags and grid toggle; avoid emitting snaps when disabled or excluded.

## AutoConstrainer
- Default tolerances: ±5° for horizontal/vertical/parallel/perpendicular/tangent; 2mm coincidence; auto-apply threshold 0.5; enabled by default.
- Inference APIs return ghost constraints with confidence; filterForAutoApply drives automatic commit. Preserve confidence semantics for UI opacity.
- Watch excludeEntity in coincident inference to avoid self-snap during drawing.

## SketchRenderer
- OpenGL 4.1 core: VBO-based geometry, adaptive arc tessellation (8–128 segments; angle ~5°/seg), region fill, constraint icon atlas, DOF state colors.
- Call initialize() after GL context is ready; updateGeometry when entities change; updateConstraints when only constraint state changes.
- Viewport culling is applied; keep bounds accurate in EntityRenderData.

## LoopDetector / FaceBuilder
- LoopDetector uses graph traversal + sampling; tolerances: coincidence 1e-4 mm; tessellation tol 0.05 mm (relative 0.02); arc seg 8–256, circle seg 32–512.
- Faces: outer loop CCW, holes CW; validate hole containment and non-overlap. If planarizeIntersections is off, expect unusedEdges/isolatedPoints outputs.
- FaceBuilder builds OCCT wires/faces on sketch plane; default edgeTolerance 1e-4 mm; can repair gaps up to 0.1 mm when enabled.
- Respect SketchPlane mapping when converting 2D to gp_Pln; holes become inner wires.

## Modeling helpers
- EdgeChainer: builds tangent-continuous edge chains; default tangentTolerance cosine 0.9999 (~0.8°). Use when creating fillet-like sweeps.
- BooleanOperation: perform(tool, target, mode) and detectMode(tool, targets, extrudeDir); assumes single target for v1 cut/join/intersect.

## Testing
- Sketch/solver/constraints: proto_sketch_geometry, proto_sketch_constraints, proto_sketch_solver.
- Loop/face bridge: proto_loop_detector, proto_face_builder.
- Rendering compile guard: test_compile. Run relevant prototypes after changing tolerances, ordering, or binding logic.

## Logging & Debugging
- Core categories currently in use:
- `onecad.core.sketch`
- `onecad.core.snap`
- `onecad.core.constraintsolver`
- `onecad.core.autoconstraint`
- `onecad.core.sketchtool`
- `onecad.core.sketchtool.line`
- Preserve start/decision/done logging shape for operations (for example `addConstraint:start` -> validation reason -> `addConstraint:done`).
- Include stable IDs and counts in logs (`EntityID`, `ConstraintID`, entity/constraint totals, candidate counts) to support deterministic replay analysis.
- Keep hot-path logs bounded: avoid per-frame/per-point spam unless explicitly gated by debug-only categories.
- For constraint/snap regressions, inspect in this order:
- `onecad.core.sketch` for entity/constraint creation and solver invalidation
- `onecad.core.constraintsolver` for translation/solve fallback paths
- `onecad.core.snap` for candidate generation and ambiguity resolution
- `onecad.core.autoconstraint` and `onecad.core.sketchtool*` for inference + tool state transitions
- Suggested triage command:
- `rg -n "onecad\\.core\\.(sketch|snap|constraintsolver|autoconstraint|sketchtool)" logs/<latest>.log`
