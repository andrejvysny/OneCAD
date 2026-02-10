# Snap System Upgrade — Learnings

## Conventions
- Proto tests: main(), print PASS/FAIL, return 0/1. No test frameworks.
- SnapType priority: Vertex > Endpoint > Midpoint > Center > Quadrant > Intersection > OnCurve > Grid > Perp > Tan > H > V > Guide > ActiveLayer3D
- 2mm snap radius constant: constants::SNAP_RADIUS_MM
- EntityID = std::string (UUID)
- Non-standard axes: Sketch X -> World Y+, Sketch Y -> World X-

- proto_sketch_snap baseline uses deterministic geometry + per-snap enable gating to avoid priority masking in isolated tests.
- proto_sketch_snap supports --legacy name-token filtering and --benchmark p95 over 100 snap queries on 1000 deterministic random points.
- Spatial hash integration can stay non-invasive by prefiltering inside existing finder loops (candidate-set membership check) instead of refactoring finder APIs globally.
- Stale rebuild heuristic by entity count (`lastEntityCount_`) is enough for first-pass acceleration and preserves snap outputs when geometry/topology count is unchanged.
- Equivalence test is easiest via a `SnapManager` toggle (`setSpatialHashEnabled`) to compare fast path vs brute-force path under identical settings.
- Current benchmark after hash: p95 ~0.52 ms for 1000 entities (target <2.0 ms pass).
- Ellipse center snap integrates cleanly via `SketchEllipse::centerPointId()` in the same center finder branch used for circle/arc center-point lookup.
- Ellipse quadrant snap should use parametric extrema (`t = 0, pi/2, pi, 3pi/2`) with `SketchEllipse::pointAtParameter(center, t)` so rotation is naturally handled.
- Robust ellipse OnCurve snap can avoid fragile derivatives: sample 32 params, then local ternary refinement around best sample to get nearest param point.
- Line-ellipse intersection is stable with a transform pipeline: world -> ellipse local (translate + rotate -theta) -> unit circle scale -> quadratic solve -> inverse transform.
- `proto_sketch_snap --legacy` blocklist must not include `ellipse`, otherwise new ellipse tests are skipped and regression coverage is lost.
- Perpendicular snap finder works as inference-only foot projection: line uses unclamped dot projection with segment-range check; circle/arc use radial foot; arc requires `containsAngle` gate.
- Tangent snap finder for circle/arc is stable with closed-form tangent points from external cursor; skip when cursor is on/inside radius; choose nearest valid tangent in snap radius.
- Tangent snaps are typically far from cursor for external points, so targeted prototype tests should set larger snap radius to keep deterministic pass conditions.
- Extension/guide snap is line-only inference: use unclamped line projection parameter `t`, accept only `t < 0` or `t > 1`, then gate by snap radius to projected point.
- Extension distance cap can be enforced in param space (`t`) to avoid very long guide snaps; guide origin should be nearest endpoint (`start` for `t < 0`, `end` for `t > 1`).
- Guide snap payload for renderer/UI should set `type=SketchGuide`, `hasGuide=true`, `hintText="EXT"`, and `guideOrigin` endpoint.
- Legacy proto mode skip tokens should include both `extension` and `guide` to keep historical baseline behavior stable.
- Angular snap can be injected as additional `SnapType::SketchGuide` candidate right after existing guide-extension finder; this preserves priority/ordering while enabling polar tracking without enum changes.
- Backward-compatible API extension works by adding `std::optional<Vec2d> referencePoint = std::nullopt` as 4th arg on `findBestSnap` and `findAllSnaps`; all existing 2-3 arg call sites compile unchanged.

- Snap visual toggles can be wired at renderPreview callsite: gate hasGuide by SnapManager::showGuidePoints() and gate hintText by showSnappingHints() while keeping snap position/type intact.
- SketchRenderer dotted guide line can reuse existing dashed polyline path in VBO build with pixelScale-converted 2px dash/2px gap, avoiding new shader paths.
- Per-snap indicator color mapping is cleanest in renderer via a snapColorForType() helper and applied before PIMPL buildVBOs call.

- Snap settings wiring already exists in `Viewport::updateSnapSettings`: `settings.showGuidePoints` -> `SnapManager::setShowGuidePoints(...)` and `settings.showSnappingHints` -> `SnapManager::setShowSnappingHints(...)`.
- `SnapSettingsPanel::settings()` already exports both toggles (`showGuidePoints`, `showSnappingHints`), so panel -> viewport -> snap manager pipeline is complete.
- Integration coverage added in `proto_sketch_snap`: `test_toggle_suppression` validates type-gating (Grid-only suppresses Vertex), and `test_all_snap_types_combined` validates global priority (Vertex wins near origin).
- Legacy mode skip filter must include `toggle` and `combined` tokens so `--legacy` remains backward-compatible after adding the two new integration tests.
