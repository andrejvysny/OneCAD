# Sequential Thinking Log

**Purpose**: Track reasoning and decisions for each milestone of the OneCAD test suite implementation.

---

## Milestone 0 — Inventory (2026-01-04)

### Thinking Summary

**Goal**: Understand the OneCAD sketcher system, identify testable APIs, and produce a comprehensive test plan.

**Current Facts Gathered**:

1. **Constraint System** (18 types):
   - Positional: Coincident, Horizontal, Vertical, Fixed, Midpoint, OnCurve (6)
   - Relational: Parallel, Perpendicular, Tangent, Concentric, Equal (5)
   - Dimensional: Distance, HorizontalDistance, VerticalDistance, Angle, Radius, Diameter (6)
   - Symmetry: Symmetric (1)
   - Status: ~3 are TODO (Diameter, Concentric, Symmetric)

2. **Snap System** (12 types total):
   - Core snap types: Vertex, Endpoint, Midpoint, Center, Quadrant, Intersection, OnCurve, Grid (8)
   - Inference types: Perpendicular, Tangent, Horizontal, Vertical (4)
   - Radius: **2.0mm constant** in sketch coordinates (zoom-independent)
   - Priority: Enum value (lower = higher priority)

3. **Auto-Constrainer** (7+ rules):
   - Inference rules: Horizontal (±5°), Vertical (±5°), Coincident (2mm), Tangent, Perpendicular (90±5°), Parallel (±5°)
   - Configuration: Per-type enable/disable, confidence values
   - Behavior: Ghost icons at 50% opacity, applied on commit

4. **Solver Configuration**:
   - Tolerance: 1e-4 mm
   - Algorithm: DogLeg (default) with LM/BFGS fallback
   - Redundancy detection: On by default
   - Max iterations: 100
   - Timeout: 1000ms

5. **Entity Types**: Point, Line, Arc, Circle, Ellipse, Spline (future)

6. **Existing Prototypes**:
   - `proto_sketch_geometry.cpp` — geometry primitives
   - `proto_sketch_constraints.cpp` — constraint satisfaction
   - `proto_sketch_solver.cpp` — solver/DOF
   - Plus: `proto_tnaming`, `proto_custom_map`, `proto_elementmap_rigorous`, etc.

**Risks/Unknowns**:

1. **PlaneGCS Integration Status** — Skeleton exists; actual solve() capability unclear
   - Mitigation: Start with DOF calculation + constraint satisfaction tests
   
2. **TODO Constraint Types** — 3 types marked TODO; unknown priority
   - Mitigation: Test only implemented types; skip TODOs
   
3. **Auto-Constrainer Implementation** — 7 rules stated; actual implementation unknown
   - Mitigation: Grep code for inference implementations
   
4. **Snap Type Completeness** — Are all 8 snap types fully implemented?
   - Mitigation: Grep for snap type handlers

5. **Determinism** — Random elements in solver?
   - Mitigation: Use fixed seeds; verify reproducibility in tests

**Assumptions Made**:

1. APIs documented in headers are the source of truth (assume implementations match)
2. Solver is minimally functional (can at least calculate DOF)
3. Constraint satisfaction checks are complete (can test without solve)
4. Snap radius is truly constant at 2mm (no zoom scaling)

**Smallest Next Steps**:

1. **Milestone 1**: Implement test harness (3-4 hours)
   - Create `tests/test_harness/TestHarness.h`
   - Assertion macros + runner
   - Migrate existing prototypes

2. **Then**: Constraint tests → Solver tests → SnapManager → AutoConstrainer → ctest integration

**Validation Plan for Milestone 0**:

✅ SKETCH_TEST_PLAN.md created with:
- All APIs documented (Sketch, Entities, Constraints, Solver, SnapManager, AutoConstrainer)
- Complete constraint type table with DOF/satisfaction status
- Test matrix for all features (coverage: 5-60% depending on subsystem)
- Known tolerances with file:line references
- Coverage gaps identified (110+ tests needed)
- Risk analysis + unknowns documented

---

## Milestone 1 — Test Harness (TBD)

### Thinking Summary (2026-01-04)

**Goal**: Provide a shared, deterministic test harness for prototype executables (headless, fast), then migrate existing sketch prototypes to it.

**Facts used**:
- Prototypes lived under `tests/prototypes/` and used raw `assert` + `std::cout`.
- Targets: `proto_sketch_geometry`, `proto_sketch_constraints`, `proto_sketch_solver` (linked against `onecad_core`).
- Need to avoid adding heavy deps; prefer header-only minimal harness.

**Decision**: Implement a minimal header-only harness (`tests/test_harness/TestHarness.h`) with:
- TEST_CASE registration via static Registrar
- EXPECT_TRUE/FALSE/EQ/NE/NEAR/VEC2_NEAR
- runAllTests() summary and non-zero exit on failures
- Stream-safety: fallback "<unprintable>" for non-streamable types (e.g., enums)

**Steps taken**:
1) Added include path for prototypes to `tests/CMakeLists.txt` (tests directory)
2) Created harness header with registry, assertions, and summary runner
3) Migrated three sketch prototypes to harness (geometry, constraints, solver)
4) Built and ran: all three prototypes pass

**Validation**:
- Build: `cmake --build . --target proto_sketch_geometry proto_sketch_constraints proto_sketch_solver`
- Run: `./tests/proto_sketch_geometry && ./tests/proto_sketch_constraints && ./tests/proto_sketch_solver`
- Output: All tests passed (harness prints concise summaries)

**Risks/Unknowns**:
- None observed for harness; future tests may need richer matchers (consider extending if needed)

**Next**: Milestone 2 — Constraint unit tests expansion (all constraint types, edge cases, serialization)

