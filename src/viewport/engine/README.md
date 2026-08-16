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
| `BodyObject.ts`      | Per-body face Mesh + fat edge `LineSegments2`; shared materials.|
| `pbrParams.ts`       | Viewport-owned PBR param set + the `BodyMaterialSource` seam.   |
| `openPbrToThree.ts`  | Pure OpenPBR→three mapping + the standard/physical tier rule.   |
| `bodyMaterialPool.ts`| One shared material per ASSIGNED document material (data-colored).|
| `Picker.ts`          | rAF-coalesced raycast → face/edge PickHit; edge screen-bias.    |
| `HighlightLayer.ts`  | Hover/selected highlight: shared-attribute faces, sliced edges. |
| `DragHandle.ts`      | Extrude depth handle: screen-scaled arrow + fat pick cylinder.  |
| `TransformGizmo.ts`  | Placement gizmo: 3 arrows / 3 plane quads / 3 rings (WP-B W2).  |

Both grab gizmos follow the same wiring, and a new one should too: created lazily
by `ViewportEngine`, rescaled to `planePixelWorld()` every frame, hit-tested on
LMB-down so a press that lands on a handle grabs it instead of falling through to
selection, refreshed in `applyTheme()`, and disposed with the engine. Orbit itself
never collides with this — it is RMB+Shift, a different button entirely.
`TransformGizmo` is a
deliberate SIBLING of `DragHandle` rather than a generalisation of it — the
extrude handle is a one-axis depth grab on a hot, e2e-covered path, and the two
share nothing beyond "screen-scaled overlay mesh".

Colors come from `palette.ts` (design tokens) — the engine never hard-codes hex.

## Mesh ingestion + picking (F-WP5)

MESH1 blobs are parsed zero-copy (`../mesh/parseMeshPayload.ts`) into typed-array
views, built into GPU geometry in `../mesh/meshRegistry.ts` (a module Map OUTSIDE
zustand — double-buffered swap, old geometry disposed one frame later via
`flushDisposals()` in the render loop; a leak tripwire asserts the registry is
empty on document close). `../mesh/meshSync.ts` (`MeshIngest`) is the app glue:
`document-changed` → fetch visible bodies → swap → BodyObject in `bodiesRoot`.

Picking raycasts `bodiesRoot`; a triangle/segment ordinal is mapped to a
face/edge id by binary search over the MESH1 ranges (`../mesh/faceRangeIndex.ts`),
decoding the id string LAZILY on pick. Orbit is suppressed when an LMB drag
starts on geometry (`CadOrbitControls` `hitTest` seam).

**Body edges are fat lines (`LineSegments2`)** — WebGL clamps
`LineBasicMaterial.linewidth` to one device pixel, which on a HiDPI display is a
half-CSS-pixel hairline. `Picker.ts` raycasts them NATIVELY (it gathers via
`traverseVisible`, so a hidden pick proxy would be gathered too). Three
consequences the Picker owns, none of which apply to plain lines:

- an intersection carries **`faceIndex` = the segment ordinal directly** (the
  geometry is instanced, one instance per segment) and **no `index`**. There is
  no `>> 1`; adding one binds every edge pick to the wrong edge.
- the anchor is **`pointOnLine`** (on the segment), not `point` (on the ray).
- the raycast is **screen-space**: the hit radius is
  `(material.linewidth + raycaster.params.Line2.threshold) / 2` in the device px
  `material.resolution` is expressed in. `line2PickThreshold` cancels the drawn
  width out so the tolerance stays `EDGE_PICK_PX` CSS px at any weight or dpr,
  and the Picker **flushes `material.resolution` itself before every raycast** —
  at (0,0), which is where a body sits until its first rendered frame,
  `LineSegments2.raycast` returns silently with no hits. `params.Line.threshold`
  (world units) is still driven, because the face-vs-edge preference bias
  arbitrates on the world-space `distance` both hit kinds report.

Highlights are two shapes. FACE and BODY overlays are shallow-cloned geometries
that SHARE the body's BufferAttributes and only narrow `drawRange` — they own no
GPU buffers and are never disposed (that would free the shared buffers). EDGE
overlays cannot do that: `drawRange` is meaningless on an instanced geometry, so
each one builds a `LineSegmentsGeometry` over a `subarray` VIEW of the entry's
`edgeSegmentPositions` (CPU-side still zero-copy, but its own GL buffer) and IS
disposed on rebuild/teardown. `HighlightLayer.clearObjects` discriminates on
`isLineSegmentsGeometry`.

`SketchStaticLayer` (model-mode, non-editable presence of every sketch) picks
DIFFERENTLY: its `hitTest` raycasts an explicit object list it maintains itself,
not `traverseVisible`. Each sketch's curves are therefore two objects sharing one
geometry — a fat, VISIBLE `LineSegments2` draw pass (dpr-normalized CSS-px width,
`SketchObject.cssLineWidth`) and a plain, INVISIBLE `LineSegments` pick proxy
that `hitTest` raycasts instead. An invisible object costs no GPU (the renderer
skips it before buffer upload), and the proxy keeps world-unit `Line.threshold`
semantics and bare-`Raycaster` (no camera/resolution) test compatibility. The
explicit-list gather is what makes a proxy possible there and impossible for
body edges.

**WebGPU limitation.** `three/examples/jsm/lines/*` is WebGLRenderer-only (the
WebGPU build lives under `lines/webgpu/`). `OriginTriad`, `SketchObject`,
`SketchStaticLayer` and now **body edges** all depend on it, so the flag-gated
WebGPU backend would lose the triad, sketch curves and every body edge — the
same class of gap as the missing PMREM environment. WebGL is the default
(F-WP4); a WebGPU line path is a later concern for all four together.

## Scene graph

```
scene
├── HemisphereLight + key/fill DirectionalLights (camera-relative rig)
├── GridPlane           (world XY, Z=0)
├── bodiesRoot          (body face Mesh + fat edge LineSegments2 — F-WP5)
├── sketchRoot          (sketch entities — later WP)
└── interactionRoot     (hover/selected highlight meshes — F-WP5)
```

## HTML chips must stay off the value arrow (MC-R9)

`DragHandle` is a 3D object; the model-tool chip is a DOM element the
`HtmlOverlayDriver` positions. They share one world anchor, so left alone the chip
lands ON the arrow — measured, with an armed Fillet: the arrow's own grab pixel
(888, 398) resolved to the chip's Cancel button, and `isExcludedClickAwayTarget`
refused that press, which made the arrow ungrabbable rather than merely awkward.

The contract, for any future overlay that shares an anchor with a grab handle:

- `ViewportEngine.getInteractionOverlayBounds("valueHandle")` is the arrow's screen
  box, and the render loop passes it to `HtmlOverlayDriver.update()` **per frame**,
  right after `dragHandle.orient()` so it is current for that frame.
- An overlay opts in with `ChipPlacement.avoidValueHandle`
  (→ `OverlayPlacement.avoidKeepOut`). Opting in is the caller's choice; the driver
  never displaces an item that did not ask.
- Displacement is **vertical only** and its side is **sticky** while the overlap
  lasts (`keepOutShiftY`, pure and separately tested). A side re-chosen per frame
  flips as the chip drifts across the box's centre — the chip jumping over the arrow
  mid-drag.
- The item's size comes from a `ResizeObserver`, never a per-frame
  `getBoundingClientRect()`: the driver writes transforms every frame, so reading a
  rect back would force a synchronous reflow on the drag path. An item with no
  measured size is left alone rather than displaced by a guess.

## Screen metrics, grid ownership, guides and DPR (SNAP P0/P4)

**`planeScreenMetric(at)` is the snap distance authority.** It returns the local
2×2 plane→screen Jacobian at a plane point, in CSS pixels per plane unit
(`planeMetric.ts` holds the pure math so it is testable with no renderer).
`planePixelWorld()` remains for callers that genuinely want one scalar (drag
handles, degeneracy floors), but snap arbitration must not use it: under an
oblique or perspective view a plane unit projects to a different pixel length
along u than along v, so a scalar threshold turns the snap radius into an
invisible ellipse. Null means the point is behind the camera, non-finite, or the
view is edge-on — treat that as "no snap this frame", never as "reuse the last
one".

**One effective grid.** In model mode the world XY grid follows
`viewportStore.gridVisible`. Entering sketch mode hands that preference to the
sketch-plane grid and hides the world one **without touching the store**;
exiting restores it. `applyGridVisibility()` is the single owner, called from
`setGridVisible`, `enterSketch` (before the first frame) and `exitSketch`.
Snap-to-grid is `settingsStore.snapTo.grid` and is deliberately independent —
hiding the drawing must never silently stop the snapping.

**Guides are `Line2`, and their dash cadence is CSS-relative.**
`LineDashedMaterial` measures `dashSize`/`gapSize` in WORLD units, so a constant
there is a solid line at one zoom and a handful of enormous strokes at another.
`SnapIndicator` gives each guide its own material and recomputes dash/gap per
frame from the plane→screen scale the engine passes it, holding 6px on / 4px off
at every zoom. Marker glyphs are one per snap family — the marker is the only
feedback when snapping hints are off.

**DPR is not a load-time constant.** `dpr.ts` owns `MAX_DPR` and the CSS→device
conversion. Fat-line `linewidth` is in DEVICE pixels, so a width baked at import
time renders wrong the moment a window moves to a different-DPR display — and
`ResizeObserver` never fires for that, because the CSS box did not change.
`ViewportEngine.syncDpr()` checks the ratio each frame and re-sizes the drawing
buffer; `SketchObject.update` re-converts every authored CSS width when the
capped ratio moves.

**Curve tessellation is adaptive** (`curveTessellation.ts`): segment counts come
from a 0.35 CSS px sagitta budget with a 25% rebuild band, floors of 8 (24 for an
ellipse) and a 2048 cap. This is a RENDERING budget only — snap candidates,
hit-testing and the solver all work against the exact analytic curve, and
changing the segment count must never move a snap point.

**Selected sketch points are held as REFS**, not coordinates
(`setSketchSelectedPointRefs`), and re-resolved on every `setSession`. The
coordinate form froze the ring where the point was at selection time, so a solve
that moved the point left the ring behind.

## Per-material PBR (assigned materials)

A body can wear a document **material** as well as the shared token look. Two
material owners therefore coexist, and which one a body draws with is decided
per body, never per mode:

| owner                | colors come from | follows the theme | who owns it |
| -------------------- | ---------------- | ----------------- | ----------- |
| `BodyMaterialLibrary`| design tokens    | yes               | `MeshIngest` (committed), engine (previews) |
| `BodyMaterialPool`   | document data    | no                | `MeshIngest` |

Both obey the same shared-material discipline: `pool.acquire(key, …)` returns the
SAME instance for a key, so N bodies wearing one material cost one program. **The
key is the caller's and must already encode the resolved parameter hash**, plus a
`:vc` suffix for the vertex-colored twin — the pool never derives one, because
that would re-implement a hash the material's owner already computed. Lifetime is
a **sweep**, not a refcount (`sweep(liveKeys)` after each sync): a refcount would
need every caller to pair acquire/release across async mesh loads.

**`src/viewport/**` must not import `src/modules/**`.** So the viewport declares
what it consumes (`PbrMaterialParams` / `BodyMaterialSource`, `pbrParams.ts`) and
the owning module *registers* an implementation through `materialSourceBridge.ts`
— a module singleton, like `engineBridge.ts`, because the two lifetimes do not
nest (a module activates once at bootstrap; `MeshIngest` attaches with the
engine, twice under StrictMode). `ResolvedMaterial` satisfies `PbrMaterialParams`
**structurally**; the check lives in `modules/render/viewportBridge.test.ts`,
which is the only side allowed to see both types.

**Two tiers.** `materialTier` promotes to `MeshPhysicalMaterial` only when a lobe
a standard material cannot express is actually in play (coat, transmission, fuzz,
thin film, anisotropy, or non-default specular). Parameters that are *stored but
not rendered* — subsurface, `base_diffuse_roughness`, the coat's ior/color/
darkening, `geometry_thin_walled` — never force a promotion, because paying for
the physical shader to render nothing would be a pure loss. Transmission stays
LIVE in the modeling view; its scene-render-target cost is only paid once a
transmissive material is actually assigned.

**THE TWO-TRACK RULE.** A body can carry FUNCTIONAL color (a user-painted body/
face color, or an imported STEP file's `FACE_COLORS`) *and* a material. In the
modeling view functional color WINS, per face:

```
face     = authored face color ?? mesh FACE_COLORS ?? material override ?? fallback
fallback = body color ?? material base_color ?? the body-fill TOKEN
```

That ladder has exactly ONE implementation, `bakeFaceColors` — a per-face
material override extends the existing bake rather than adding a second one. Only
the last rung moves with the theme, which is what keeps `rebakeFaceColors` a
theme concern. A body-level material needs no bake at all (its base color is
uniform, so it lives on the pooled material and the geometry stays indexed and
zero-copy); a per-face override does, and adding or removing the *first* one is a
de-index/re-index — a layout change, not an attribute rewrite. `MeshEntry.
setMaterialColors` returns `false` there and `MeshIngest` goes back through the
normal load path, because live highlight clones share those BufferAttributes by
reference.

**Face overrides report their binding.** The persisted override map is
face-keyed and global, so *which body a face belongs to* is tessellation evidence
only `MeshIngest` holds. It reports `bound` (override ids the current mesh has a
face for) and `unbound` (ids it previously bound to that body and no longer can)
back through the source. That second set is the H5-B failure mode in the
appearance lane: a regen renumbers the topology and an override stops naming a
real face. Reporting it beats deleting the assignment or silently binding it to
whatever now carries that id.

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

**The same invariant binds a `ViewportContribution`**, one indirection out: a
contributed layer is not in `applyTheme()`'s list at all, so it must subscribe
through `ViewportContext.onThemeChange`. The engine cannot tell that it did not
— there is nothing to omit and nothing to throw — which makes a forgotten
subscription strictly harder to spot than a missing `applyTheme()` line. The
datum layer (the first contribution) is covered by a negative-checked case in
`themeRefresh.test.ts`; any layer added the same way needs one too.

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
  `SketchObject`'s nine `LineMaterial`s, `BodyMaterialLibrary`'s edge pair): the
  *inactive* one must be refreshed too, or it appears stale the moment the user
  hovers — or, for the edge pair, the moment they press the display-mode button.
  Materials built from `color.getHex()` (`SketchObject`, both edge materials)
  hold a value snapshot, not a live reference.
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

`BodyMaterialPool.refreshColors()` is the **one deliberate no-op** in the
invariant: its materials are colored from document data, never from a token, so
there is nothing to re-read. The method exists anyway, because an absent method
is indistinguishable from a forgotten one. The only token-colored part of a
pooled body — the fallback baked into a vertex-colored twin's unset faces — is
reached by `refreshFaceColors()` like any other baked attribute.

Light levels are a theme × backend table (`LIGHT_LEVELS`), not just a backend
split: the hemisphere light's ground half IS the canvas token, so in dark it
must come down or it muddies undersides.

**Body edges are TWO tokens, not one**, and `BodyMaterialSet` carries a material
for each. Which one a body draws with is decided by the render mode's
`edgeStyle` (`renderModes.ts`), never by a branch in `BodyObject`:

| `edgeStyle`  | modes                | token                    | why                                                                                            |
| ------------ | -------------------- | ------------------------ | ---------------------------------------------------------------------------------------------- |
| `onFaces`    | shaded, shaded+edges | `--color-body-edge`      | there is a lit face behind every edge, so it is an OUTLINE — near-black in BOTH themes (Shapr3D) |
| `standalone` | wireframe            | `--color-body-edge-wire` | the edges ARE the drawing, so this one INVERTS or a dark canvas swallows them                   |

`applyPaletteColors` refreshes both materials unconditionally — see the
swapped-material trap above. A theme flip while in shaded+edges must still leave
the wireframe material current, because the swap happens on the next button
press with no palette read in between.

## Testing note

jsdom has no real WebGL, so the engine's GPU path is only fully verifiable
in-browser (Playwright vs vite). Unit tests cover the **pure** math (camera
apparent-size, orbit yaw/pitch/zoom, grid step, overlay projection, ViewCube
transform) and a mocked-renderer init/dispose smoke test.
