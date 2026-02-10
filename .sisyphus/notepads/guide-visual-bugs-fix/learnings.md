## Guide Visual Bugs Fix — Learnings

## Guide Visual Bugs Fix
- Fixed guide line dash pattern to be more visible (8px dash, 6px gap).
- Implemented crosshair rendering at intersections of multiple active guides.
- Increased crosshair size to 5px for better visibility.
- Ensured crosshairs are clipped to viewport bounds.
### Guide Crossing Snap Priority
- In `applyGuideFirstSnapPolicy`, individual Horizontal/Vertical guide snaps often win over the Intersection snap because the cursor is physically closer to one of the lines than the crossing point.
- Explicitly prioritizing `SnapType::Intersection` with `hasGuide=true` ensures that when two guides cross, the intersection point is preferred regardless of distance (within the snap radius).
- This fix was verified with `proto_sketch_snap` (test case `test_guide_crossing_snaps_to_intersection`).

## Snap Logic Validation (Task 4)
- H/V guide crossing produces an Intersection candidate with `hasGuide = true`.
- `findBestSnap` (via `applyGuideFirstSnapPolicy`) correctly prefers this Intersection over individual H/V snaps.
- Vertex snaps still maintain priority over guide intersections.
- Near-parallel guides are correctly rejected by the cross-product threshold in `infiniteLineIntersection`, preventing spurious snaps.
