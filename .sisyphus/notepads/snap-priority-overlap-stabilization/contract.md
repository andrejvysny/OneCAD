# Snap Overlap Arbitration Contract

## Purpose
Define explicit, deterministic winner selection when multiple snap candidates exist at the same location.

## Scope
- Applies to `SnapManager::findBestSnap()` winner selection.
- Applies to all call sites that consume snap winner for preview and commit.
- Applies to identical input tuple: cursor position, sketch state, exclude set, and reference point.

## Definitions
- **Co-located candidates**: two candidates are co-located when both position coordinates differ by <= `SnapResult::kOverlapEps`.
- **Co-location epsilon**: `SnapResult::kOverlapEps = 1e-6` (sketch coordinates).

## Arbitration Rules
1. **Co-location epsilon rule**
   - Candidate overlap is determined with epsilon `1e-6` in sketch coordinates.

2. **Co-located ranking by snap type**
   - For co-located candidates, priority is strict and follows `SnapType` enum order (lower enum value wins):
   - `Vertex > Endpoint > Midpoint > Center > Quadrant > Intersection > OnCurve > Grid`.

3. **Same-type co-located tie-break chain**
   - If co-located candidates share the same `SnapType`, break ties in this exact order:
   - `pointId < entityId < secondEntityId < position.x < position.y < hasGuide(false wins) < guideOrigin < hintText`.

4. **Preview/commit parity rule**
   - Same `findBestSnap()` output must be used for both preview and commit.
   - `resolveSnapForInputEvent()` in `SketchToolManager` must resolve same winner for mouse-move (preview) and mouse-press (commit) when cursor position and sketch state are identical.

5. **Determinism guarantee**
   - For identical input, `findBestSnap()` must return same winner on every call.
   - Winner selection must not depend on container insertion order or hash-table randomness.

## Comparator Contract Mapping
Current comparator chain in `SnapResult::operator<` is:
- `type < distance < pointId < entityId < secondEntityId < position.x < position.y < hasGuide(false first) < guideOrigin.x < guideOrigin.y < hintText`.

This chain is the canonical deterministic tie-break behavior and must remain stable unless the arbitration contract is versioned and updated.
