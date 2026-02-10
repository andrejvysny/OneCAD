## Snap Multi-Guide Notepad — Learnings
## Multi-guide Rendering Learnings

- Expanded `SketchRenderer` to support multiple guide lines via `activeGuides_` vector.
- Updated `SketchRendererImpl::buildVBOs` to iterate and render up to 4 guides.
- Maintained backward compatibility with `snapIndicator_.hasGuide` for single-guide callers.
- `setActiveGuides` triggers `vboDirty_` to ensure immediate visual update.
- `hideSnapIndicator` now clears both the single snap guide and the multi-guide vector.
- Used `SketchRenderer::GuideLineInfo` scope in `SketchRendererImpl` method signatures to resolve type visibility issues.
## SnapManager Guide Line Fixes
- Fixed perpendicular and tangent guide lines by setting `guideOrigin = cursorPos` instead of `guideOrigin = position`.
- Added epsilon guard to all snap finders that set `hasGuide = true` to prevent zero-length guide lines.
- Fixed a build error in `SketchRenderer.cpp` where `activeGuides` was being used but not passed to `buildVBOs`.
- Updated `proto_sketch_snap` test to expect the new `guideOrigin` for perpendicular snaps.
- Successfully wired dual-query in SketchToolManager::handleMouseMove() to support multiple active inference guides.
- Implemented collinear deduplication for guides using cross product (epsilon 0.01 radians) to prevent overlapping visual clutter.
- Capped active guides at 4 to maintain performance and visual clarity.
- Verified that setActiveGuides({}) is called when no guides are present or sketch is null, ensuring stale guides are cleared.
- Confirmed that Vec2d is a simple struct and requires manual math for vector operations in SketchToolManager.cpp.

- Added 6 multi-guide test cases to proto_sketch_snap.
- Verified that findAllSnaps() returns multiple results when multiple entities are within snap radius.
- Noted that findHorizontalSnaps and findVerticalSnaps currently only return the single best result, while findVertexSnaps, findEndpointSnaps, findPerpendicularSnaps, findTangentSnaps, and findGuideSnaps return all matches.
- Used SketchGuide (Extension) to verify multi-guide behavior for collinear guides in test_dedupe_collinear_guides.
