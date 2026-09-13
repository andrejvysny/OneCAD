---
name: repo-map-viewport
description: File-level map of camera/orbit/picker/sketch-tessellation code under src/viewport, for viewport-hardening or camera-refactor tasks
metadata:
  type: project
---

Layout of `src/viewport/engine/` relevant to camera, picking, and sketch rendering (as of HEAD 65b4c60, docs/viewport-hardening/ in progress untracked):

- `CameraRig.ts` — persp+ortho pair sharing `{target, offset, distance}`; `apply()` derives ortho half-height from `orthoHalfHeight(distance, fovDeg)` to preserve apparent size across projection switch; `orient()` is a custom turntable basis (NOT `THREE.lookAt`) built in `writeCameraBasis` to avoid pole-roll snapping. FOV default 76 (`constructor(fovDeg = 76)`). `setAspect()` only updates `persp.aspect` + `updateProjectionMatrix()` — ortho bounds are recomputed lazily inside `apply()`, not in `setAspect`.
- `CadOrbitControls.ts` — owns the actual mutable camera state `{target, yaw, pitch, distance}` (CameraRig itself is stateless target/offset consumer). `isDragActive` gates ONLY wheel/gesture-driven orbit (`dispatchNav`'s `case "orbit"`), not RMB+Shift drag orbit (gated instead by sticky `lmbOrbitSuppressed`). `getViewState()`/`setView()` snapshot the full `{yaw,pitch,distance,target}` — used by `ViewportEngine.enterSketch/exitSketch` to save/restore the pre-sketch view.
- `navInput.ts` — pure reducer for wheel/WebKit-gesture device disambiguation (mouse vs trackpad); `CadOrbitControls` is a thin DOM adapter over it.
- `cameraFit.ts` — `computeCameraFit(request): CameraFitTarget | null`, pure NDC-rect-based fit math; `CAMERA_MIN_DISTANCE=0.5`, `CAMERA_MAX_DISTANCE=50000`, `CAMERA_FIT_MARGIN=1.15`.
- `sketchBasis.ts` — pure plane<->world math (`planeBasisMatrix`, `planePointToWorld`, `worldToPlanePoint`, `planeGeometry`); no camera coupling.
- `curveTessellation.ts` — adaptive sagitta-budget tessellation: `MAX_SAGITTA_PX=0.35`, `MIN_SEGMENTS=8`, `MIN_ELLIPSE_SEGMENTS=24`, `MAX_SEGMENTS=2048`, `SEGMENT_REBUILD_RATIO=0.25` (hysteresis band before `SketchObject` retessellates). Only `SketchObject`'s COMMITTED entity strokes/fills use this; `entityPolyline()`'s own default (`ARC_SEGMENTS=64` in `SketchObject.ts`) is used verbatim (no adaptive segment count passed) by `setPreview`/`setTrimGhost` (rubber-band + trim ghost) and by `SketchStaticLayer.ts` (model-mode always-visible sketch presence).
- `Picker.ts` — rAF-coalesced raycast; `EDGE_PICK_PX=6`; `choosePreferredHit` prefers edge when `edgeHit.distance <= faceHit.distance + bias`; `linePickThreshold` converts px->world at `focusDistance` for both projections; `raycastAll` returns `{faceHits, edgeHits, threshold}` filtered by active section clip planes via `firstUnclippedHit`-style `visible()` filter.
- `mesh/rebindPick.ts` — post-regen selection reconciliation (D-5 policy: "selection never guesses" — a ref survives a regen only via an authoritative ElementId re-resolution, never nearest-match search).
- `GridPlane.ts` — step from `chooseGridStep(cameraDistance) = snapToDecade(distance/25)`, ladder is `[1,2,5,10]`; grid is a FINITE `LineSegments` sized `HALF_CELLS=100 * minor`, re-rebuilt only when the snapped step changes.
- `planeMetric.ts` — `computePlaneScreenMetric` returns a 2x2 Jacobian (`m00,m01,m10,m11`) from 3 projected sample points (not an SVD); consumed by `ViewportEngine.planeScreenMetric`. `pxPerUnit` fed to `SketchObject.update` is `Math.max(hypot(m00,m10), hypot(m01,m11))` (larger of the two basis-column lengths — conservative/over-tessellate choice), computed in `ViewportEngine.ts` around line 877-880.
- `screenScale.ts` — `worldPerPixel(camera, anchor, height)` for constant-on-screen-size gizmos; perspective uses ray depth at anchor (not orbit distance), ortho uses frustum height / zoom.
- No dedicated vitest file for `planeMetric.ts` or `screenScale.ts` (checked `find src/viewport -iname '*planeMetric*' -o -iname '*screenScale*'` — only the source files exist).

Camera state persistence: **none**. `src/stores/viewportStore.ts` stores only a derived display string `cameraViewLabel` (no `persist` middleware on this store — only `settingsStore` persists, per CLAUDE.md). No `camera` fields found in `src-tauri/src/dto.rs`, `onecad-core`, or `protocol/SCHEMA.md` — camera/orbit state lives only in the live `CadOrbitControls` instance and is never part of document save/load or settings. A camera-persistence feature would need new document/DTO/schema plumbing from scratch, not a migration of an existing field.

Sketch-entry view save/restore: `ViewportEngine.enterSketch` (~line 1393-1452) does `this.savedView ??= this.controls?.getViewState() ?? null` then animates to `orientationAlongNormal(normal, xAxis)`; `exitSketch` (~line 1699) restores via `this.controls.setView(this.savedView, true)` unless `opts.restoreView === false`.
