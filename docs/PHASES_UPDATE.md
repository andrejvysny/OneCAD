# OneCAD Phase 3 Implementation Plan (Updated)

**Current Date:** 2026-01-06
**Status:** Phase 3 (Solid Modeling) - ~45% Complete

## Recent Accomplishments
- ✅ **Boolean Logic:** Implemented `BooleanOperation` core (Union, Cut, NewBody) and detection heuristics.
- ✅ **Extrude Tool Upgrade:** Added support for Boolean Modes and Face target detection.
- ✅ **Revolve Tool:** Full implementation with Profile + Axis selection, Drag interaction, and Preview.
- ✅ **Commands:** Implemented `ModifyBodyCommand` for updating existing bodies with boolean results.
- ✅ **Infrastructure:** `OperationRecord` updated with `RevolveParams` and `OperationParams` variant.

## Upcoming Implementation Steps

### Step 1: Boolean UI & Interaction (Priority: High)
- [ ] **Boolean Badge UI:** Add a floating UI element near the extrusion/revolve handle to show/change the boolean mode (New Body vs Join vs Cut).
- [ ] **Cursor Feedback:** Update the cursor or tooltip to indicate the current detected mode (e.g., "Cut (-25mm)").

### Step 2: Fillet & Chamfer (Priority: Medium)
Essential engineering features.
- [ ] **Selection Filter:** Enhance `SelectionManager` to support Edge chaining (tangent propagation).
- [ ] **Fillet Tool:**
    - Input: Edges.
    - Interaction: Drag arrow to set radius.
    - Kernel: `BRepFilletAPI_MakeFillet`.
- [ ] **Chamfer Tool:** Similar to Fillet but using `BRepFilletAPI_MakeChamfer`.

### Step 3: History & Parametric Engine (Priority: Critical)
Currently, operations are recorded but cannot be replayed reliably to edit parameters.
- [ ] **Dependency Graph:** Track which sketches drive which bodies.
- [ ] **Recompute Engine:** Logic to roll back document state and re-execute commands when a dependency changes (e.g., editing a sketch).
- [ ] **Topological Stability:** Ensure `ElementMap` correctly remaps IDs after recompute so downstream operations (like Fillets) don't break.

### Step 4: Native File I/O (Priority: Medium)
- [ ] **Serializer:** JSON-based format for Document structure (Sketches, Operations).
- [ ] **Geometry Storage:** Store BREP data (likely via OCCT `BRepTools::Write`) or rebuild from history.
- [ ] **Integration:** "Save" and "Open" menu actions.

## Next Immediate Action
Refine the **Boolean UI** to give users control over the operation type, or start **Fillet/Chamfer** implementation.