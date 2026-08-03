# Viewport engine (F-WP4)

Imperative Three.js core for the OneCAD viewport. **No react-three-fiber** — a
`ViewportEngine` class owns everything and a thin `ViewportRoot` React component
bridges it to stores and DOM. Rendering is on-demand.

## HARD INVARIANT — Z-UP, RIGHT-HANDED, verbatim buffers

The world is **Z-up, right-handed**. `camera.up = (0, 0, 1)` for both the
perspective and orthographic cameras (`CameraRig`).

- The ground/grid plane is world **XY at Z = 0**.
- Mesh vertex buffers from the worker (MESH1) are uploaded **verbatim**. The
  kernel already produces Z-up geometry.
- **Never** rotate `scene`, `bodiesRoot`, or any root group to "fix" a Y-up
  look, and never bake an axis swap into ingestion. If something appears
  rotated, the bug is upstream (camera/orientation), not the buffers. Rotating
  content to compensate corrupts picking, normals, and saved coordinates.

Turntable orbit yaws about **world Z**; pitch is clamped to ±(90° − ε) so the
Z-up `lookAt` never degenerates.

## Rendering model — on-demand, single rAF

`invalidate()` marks the frame dirty and schedules **one** `requestAnimationFrame`.
While idle, **no frame is scheduled and nothing renders** (verify under
`?vpdebug`: `window.__vpFrames` stops incrementing). Camera tweens (Home / Fit /
ViewCube snap) keep scheduling frames until they finish, then the loop goes
quiet. There is no continuous render loop.

Inputs that must repaint call `invalidate()` (or go through the controls'
`onChange`, which does). `ResizeObserver` is devicePixelRatio-aware (capped at
2×) for crisp output on HiDPI displays.

## Render order & depth contract — `renderOrder.ts`

`renderOrder.ts` holds the ONE painter's ladder (`RENDER_ORDER`) every layer
uses; no raw `renderOrder` numbers appear anywhere else. Two hard rules:

1. **The grid never occludes anything.** Both grids (ground + sketch-plane, the
   same `GridPlane` class) are opaque, painted first (`RENDER_ORDER.GRID`) and
   have `depthWrite: false`. Bodies simply overpaint them; sketch content blends
   over them. A depth-writing grid coplanar with sketch geometry punches
   stippled holes through fills and curves (line vs. triangle rasterization
   yields per-pixel depth deltas at Z=0) — the exact bug this contract exists
   to prevent.
2. **All in-plane sketch content renders in the transparent pass with
   `depthWrite: false`.** Tint, fills, curves, markers and ghosts are coplanar —
   the depth buffer cannot layer them; only the ladder can. Fully-opaque-looking
   curves/points still set `transparent: true` (alpha 1) so they live in the
   same render list as the fills below them — an opaque curve would be painted
   BEFORE every translucent fill and get tinted/stippled by it. Depth TEST stays
   on everywhere, so solid bodies (opaque pass, depth-written, `polygonOffset`
   pushes faces back) still occlude sketch content behind them.

Consequences to preserve when adding a layer: pick a slot in `renderOrder.ts`,
never write depth from coplanar-plane content, and never mix a `depthTest:
false` overlay material with an opaque one for the same object's states (the
object would hop between the opaque and transparent lists — see `DragHandle`).

## Lifecycle — StrictMode-safe

`init()` and `dispose()` are idempotent. React 19 StrictMode double-invokes mount
effects (mount → unmount → mount); a `dispose()` that races an in-flight async
`init()` still releases the GPU context (the renderer is disposed the moment the
awaited construction resolves after disposal).

## Files

| File                 | Role                                                            |
| -------------------- | --------------------------------------------------------------- |
| `renderer.ts`        | SOLE renderer construction; WebGL default, flag-gated WebGPU.   |
| `ViewportEngine.ts`  | Orchestrator: scene graph, render loop, resize, actions.        |
| `CameraRig.ts`       | Persp+ortho pair; switch preserves apparent size at the pivot.  |
| `CadOrbitControls.ts`| Turntable orbit / pan / zoom-to-cursor; Home/Fit/snap tweens.   |
| `GridPlane.ts`       | Adaptive XY grid (1/5/10 decade step), re-centered on target.   |
| `HtmlOverlayDriver.ts`| Projects world→screen and writes DOM transforms per frame.     |
| `palette.ts`         | Reads design tokens (tokens.css) via getComputedStyle; cached.  |
| `lightRig.ts`        | Pure camera-relative key/fill positions (floored key elevation).|
| `BodyObject.ts`      | Per-body face Mesh + edge LineSegments; shared body materials.  |
| `Picker.ts`          | rAF-coalesced raycast → face/edge PickHit; edge screen-bias.    |
| `HighlightLayer.ts`  | Hover/selected highlight via shared-attribute drawRange clones. |

Colors come from `palette.ts` (design tokens) — the engine never hard-codes hex.

## Mesh ingestion + picking (F-WP5)

MESH1 blobs are parsed zero-copy (`../mesh/parseMeshPayload.ts`) into typed-array
views, built into GPU geometry in `../mesh/meshRegistry.ts` (a module Map OUTSIDE
zustand — double-buffered swap, old geometry disposed one frame later via
`flushDisposals()` in the render loop; a leak tripwire asserts the registry is
empty on document close). `../mesh/meshSync.ts` (`MeshIngest`) is the app glue:
`document-changed` → fetch visible bodies → swap → BodyObject in `bodiesRoot`.

Picking raycasts `bodiesRoot`; a triangle/segment index is mapped to a face/edge
id by binary search over the MESH1 ranges (`../mesh/faceRangeIndex.ts`), decoding
the id string LAZILY on pick. Highlights are shallow-cloned geometries that SHARE
the body's BufferAttributes and only narrow `drawRange` — so they own no GPU
buffers and are never disposed (that would free the shared buffers). Orbit is
suppressed when an LMB drag starts on geometry (`CadOrbitControls` `hitTest` seam).

## Scene graph

```
scene
├── HemisphereLight + key/fill DirectionalLights (camera-relative rig)
├── GridPlane           (world XY, Z=0)
├── bodiesRoot          (body face Mesh + edge LineSegments — F-WP5)
├── sketchRoot          (sketch entities — later WP)
└── interactionRoot     (hover/selected highlight meshes — F-WP5)
```

## Environment & shading

Bodies are lit as a **studio setup**, not by a headlight: `NeutralToneMapping`
(exposure 1.0), an image-based environment, and a camera-relative key/fill rig.

**Environment (IBL).** `RoomEnvironment` is prefiltered through `PMREMGenerator`
into `scene.environment` **once at `init()`** and **once per context restore** —
never inside `renderFrame`, so the idle-zero-rAF contract is untouched. Details
that are load-bearing:

- **WebGL only, by construction.** `PMREMGenerator` takes a `WebGLRenderer`, so
  `createEnvironment` exists only on the WebGL `RendererHandle`; it is *absent*
  on the WebGPU handle and on the mocked handle in unit tests. Callers write
  `handle.createEnvironment?.(…)` — there is no `isWebGPU` branch in the engine.
  WebGPU therefore runs lights-only, at higher intensities to compensate.
- **Context restore is deferred by a microtask.** A PMREM render target has no
  CPU-side source, so a restored context returns a black environment unless it is
  rebuilt. The engine's `webglcontextrestored` listener is registered in `init()`
  *before* the renderer exists, so it fires **before** `WebGLRenderer`'s own
  handler re-initialises GL; rebuilding synchronously would prefilter into a
  context three still considers lost. `queueMicrotask` puts the rebuild after it.
- **The caller owns the render target.** `PMREMGenerator.dispose()` does not free
  what `fromScene` returned, so `EnvironmentHandle.dispose()` does — and
  `ViewportEngine.dispose()` calls it *before* disposing the renderer handle,
  while the GL context is still alive.
- **Undefined directions inherit the renderer clear color.** That is deliberate:
  the environment automatically matches the canvas background token.
- `scene.environmentRotation` maps the Y-up room to our Z-up world. This rotates
  **sampling only** — no scene root is touched, so the Z-up invariant holds.
- `scene.environment` only affects `MeshStandardMaterial`; the body face material
  is the sole instance.

**Light rig — `lightRig.ts` (pure, unit-tested).** A key light offset from the
view direction plus a weaker fill on the opposite side, both camera-relative and
repositioned every rendered frame, over a hemisphere ambient floor. The key's
absolute elevation has a **floor**: a purely camera-relative key swings below the
horizon when orbiting under the model, which lights bottom faces brightest and
inverts the shape cue. Flooring the key while leaving the fill unfloored keeps
under-views legible without ever out-shining a top face.

Intensity numbers look large because of r185 physics: a `DirectionalLight`
uploads `color × intensity` with no 1/π factor while `BRDF_Lambert` divides by π,
so ~π is what full white costs. The old 0.75 headlight delivered ≈0.24 × albedo —
the root cause of the flat look.

**Tone-mapping rule.** Overlay/annotation materials set `toneMapped: false`. Tone
mapping is for lit body faces only — everything else renders its design token
exactly.

## Theming (dark mode)

`palette.ts` memoizes a `THREE.Color` per token, and almost every layer builds
its materials once at construction. So the viewport cannot follow a theme change
the way CSS does — it has to be told.

**THE INVARIANT: every layer that reads `palette` MUST expose `refreshColors()`
(or `setColors()`, where the color is baked into geometry) and MUST be listed in
`ViewportEngine.applyTheme()`.** A layer missing from that list fails *silently*
— nothing throws, it just keeps the previous theme's colors until something
unrelated happens to rebuild it. `themeRefresh.test.ts` covers this per layer;
each of those tests has been negative-checked by neutering the implementation.

The sequence, driven from `ViewportRoot` on `subscribeResolvedTheme`:

```
resetPaletteCache()        // orphans every cached Color — must come FIRST
engine.applyTheme()        // clear color → light levels → environment → layers
meshIngest.refreshColors() // the COMMITTED bodies' material library
```

Ordering inside `applyTheme()` is load-bearing:

1. `setClearColor` — before the environment, because
2. `buildEnvironment()` prefilters `RoomEnvironment` through PMREM, and PMREM
   fills every direction the room does not cover with **the renderer's current
   clear color**. Rebuild the environment first and bodies stay lit by the old
   background.

Two shapes of layer need different treatment:

- **Material recolor** — copy fresh palette colors into existing materials.
  Watch for materials that are SWAPPED rather than recolored (`DragHandle`,
  `SketchObject`'s nine `LineMaterial`s): the *inactive* one must be refreshed
  too, or it appears stale the moment the user hovers. `SketchObject`'s
  materials are built from `color.getHex()` — a value snapshot, not a live
  reference.
- **Geometry rebuild** — `GridPlane` bakes the minor-line fade toward the clear
  color into a per-vertex buffer, and `OriginTriad` bakes axis colors the same
  way. Both need the attribute rewritten, not a material touched.

Tints are STATE, not theme: `BodyMaterialLibrary`, `PreviewMesh` and
`RevolvePreview` all preserve an active Cut tint across a theme change while
still re-reading everything else.

**Baked FACE_COLORS are a third shape.** An imported body with authored MESH1
face colors renders through the `shadedVertex` material kind, whose face colors
live in a per-vertex geometry attribute — and every face the file left *unset*
has `--color-body-fill` baked into that attribute. A material re-read cannot
reach them, so `MeshIngest.refreshColors()` also calls `refreshFaceColors()`
(mesh registry), which re-bakes each entry's attribute in place; authored colors
are DATA and are rewritten identically, only the unset faces move. Miss this and
an imported body keeps the old theme's neutral on half its faces while every
other body follows — silently, as always. The `shadedVertex` material itself
keeps a WHITE base in both themes (vertex colors multiply it), so it is the
attribute, not the material, that carries the theme here.

There are **two** `BodyMaterialLibrary` instances — `MeshIngest` owns the
committed bodies', the engine lazily makes its own for previews — and neither
owner can reach the other. That is why `ViewportRoot` drives the pair instead of
each subscribing independently; independent subscribers would also race the
"drop the cache before anything re-reads it" ordering above.

Light levels are a theme × backend table (`LIGHT_LEVELS`), not just a backend
split: the hemisphere light's ground half IS the canvas token, so in dark it
must come down or it muddies undersides.

`--color-body-edge` INVERTS in dark. That is not cosmetic — wireframe mode draws
edges with no faces behind them, so a near-black edge on a dark canvas would be
invisible.

## Testing note

jsdom has no real WebGL, so the engine's GPU path is only fully verifiable
in-browser (Playwright vs vite). Unit tests cover the **pure** math (camera
apparent-size, orbit yaw/pitch/zoom, grid step, overlay projection, ViewCube
transform) and a mocked-renderer init/dispose smoke test.
