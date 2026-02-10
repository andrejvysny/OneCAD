# Learnings

## 2026-02-09 Plan Start
- SketchToolManager::handleMouseMove has 2 decoupled paths: findBestSnap (tool position) and findAllSnaps (guide rendering only)
- Guides are visual-only; they never influence snappedPos
- Grid snaps (1mm spacing within 2mm radius) frequently win over guide candidates due to static enum priority
- SnapResult has hasGuide/guideOrigin fields already populated by finders
- Viewport struct in SketchRenderer.h has getMin()/getMax() for clipping
- Guide VBO uses appendDashedPolyline with {guideOrigin, snapPos} as short segment
- new helper mirrors SketchToolManager guide-first policy for standalone proto tests
- three tests cover guide override, fallback, and closest-guide tiebreak without Qt

## 2026-02-09 Completion
- applyGuideFirstSnapPolicy() helper works cleanly as static local function in SketchToolManager.cpp
- Liang-Barsky parametric AABB clipping is ideal for infinite guide projection - handles all orientations including vertical/horizontal
- Guide-first policy applied consistently across handleMouseMove, handleMousePress, handleMouseRelease
- selectEffectiveSnap standalone helper in proto tests mirrors the production policy for testability without Qt dependency
- Subagents proactively added hintText to ALL snap finders (Vertex=PT, Endpoint=END, etc.) which was a useful bonus
- getReferencePoint() overrides added to LineTool, ArcTool, RectangleTool improve guide inference accuracy
- QPainter snap hint overlay added in Viewport.cpp for visual feedback
- Total test count: 47/47 (44 original + 3 new guide-first policy tests)
