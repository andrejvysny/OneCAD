# Render Guidelines

## Scope & Modules
- OpenGL 4.1 Core Profile pipeline: Camera3D, Grid3D, BodyRenderer; SketchRenderer lives under core/sketch.
- SceneMeshStore holds renderable mesh + topology; TessellationCache builds it from OCCT shapes + ElementMap.
- GL usage is immediate-mode style via QOpenGLFunctions; shaders are inlined GLSL 410.

## Camera3D
- Orbit camera with position/target/up; cameraAngle 0 deg = orthographic, >0 = perspective (FOV = angle). setCameraAngle preserves apparent scale by adjusting distance/orthoScale.
- Standard views respect project mapping (Front = Geom +X, Right = Geom +Y, Top = +Z; see nonstandard axis mapping note in repo instructions).
- Clamps: distance [1, 50000], pitch [-89, 89], FOV [5, 120]. Near/far defaults 0.1/100000. No inertia/sticky pivot yet (known gap).

## Grid3D
- XY grid rendered in screen fade; adaptive spacing derived from pixelScale targeting ~50 px minor lines (calculateSpacing). Major = 5x minor. Clamps to 600 lines/axis.
- Axis coloring matches project mapping: Geom Y+ draws red (user X), Geom X- draws green (user Y), Z axis blue. Depth writes disabled so grid never occludes bodies.
- Grid rebuilds when spacing or extents change; shader handles radial fade from fadeOrigin. Keep spacing logic stable; spec previously assumed fixed 10mm—changing behavior impacts UX expectations.

## BodyRenderer
- Expects SceneMeshStore meshes; uses precomputed vertex normals when present, else computes flat normals. Model matrix applied to vertices and normals.
- Renders triangle pass with polygon offset to reduce z-fighting; optional preview batch uses previewAlpha. Style controls lights, rim, hemi ambient, debug normals/depth, matcap blend, glow/edge toggles; near/far/isOrtho must match camera.
- Edge pass draws only OCCT topology edges from SceneMeshStore::FaceTopology (no tessellation edges). Glow pass is additive with widened lines; wireframeOnly skips triangle pass.
- Uses VAOs/VBOs; initialize() must be called in a valid GL context. Avoid per-frame buffer rebuilds unless dirty flags set.

## SceneMeshStore
- Not thread-safe; mutate on UI/render thread. Mesh stores vertices, normals (optional, same size as vertices), triangles with faceId, topologyByFace (edge polylines + vertex samples), faceGroupByFaceId for smoothing groups. bodyId is required and set in setBodyMesh.

## TessellationCache
- Settings defaults: linearDeflection 0.05, angularDeflection 0.2, adaptive deflection on (scales with bbox diagonal, min 0.001), parallel meshing on.
- Uses BRepMesh_IncrementalMesh; builds visible edge set (sharp/open) via face angle threshold 30 deg and continuity < G1; only those edges become rendered polylines.
- Integrates ElementMap: face/edge/vertex IDs fetched via findIdsByShape; generates fallback ids bodyId/face|edge|vertex/unknown_N when missing. Face groups merged across non-visible edges to smooth normals; computeSmoothNormals splits vertices per smooth group.

## GL State & Safety
- Always bind/unbind VAO/VBO pairs; shaders are created at initialize() and deleted in cleanup(). Depth test enabled for bodies; grid disables depth writes. Polygon offset enabled for triangle pass to avoid edge z-fighting.
- QOpenGLFunctions must be initialized in the active context; do not call render() before initialize().

## Testing
- After render/tessellation changes: build with test_compile. Run a quick viewport manual check (grid fade, edge overlay, preview opacity) to catch GL state regressions.

## Logging & Debugging
- Render stack currently relies mostly on startup and Qt warnings plus caller-side logs; when adding render diagnostics, use categories `onecad.render.<component>` (for example `onecad.render.grid`, `onecad.render.body`).
- Log initialization/state transitions, not per-frame draw spam:
- good: renderer init success/failure, mesh cache invalidation, topology cache rebuild reason
- avoid: per-frame triangle/edge counts every paint call
- In CI/headless/offscreen runs (`ONECAD_HEADLESS=1` or `CODEX_CI=1`), `QOpenGLWidget` context warnings are expected and should not be treated as renderer logic regressions by default.
- When debugging rendering regressions, correlate:
- `onecad.main` startup markers and GL format setup
- Qt warnings about OpenGL context support
- any `onecad.render.*` state transitions you add
- Suggested triage command:
- `rg -n "onecad\\.(main|render\\.)|QOpenGLWidget|createPlatformOpenGLContext" logs/<latest>.log`
