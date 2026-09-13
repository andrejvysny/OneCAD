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

## Rendering model — dirty reasons, one rAF, submitted ≠ presented

`FrameScheduler.ts` owns scheduling. `invalidate(reason)` ORs a **dirty reason**
(`camera`, `geometry`, `appearance`, `overlay`, `resize`, `quality`, `recovery`)
into a bitset, bumps a monotonic `requestedRevision`, and ensures **exactly one**
`requestAnimationFrame` is outstanding. While idle, **no frame is scheduled and
nothing renders** (verify under `?vpdebug`: `window.__vpFrames` stops
incrementing). There is no continuous render loop and no second timer — a
permanent rAF would hide exactly the bug below.

**The mask is consumed BEFORE the frame work runs.** `tick()` snapshots and
clears the reasons, then calls the work; anything invalidated *during* the frame
— a contribution's `frame` hook, an `onAfterRender` listener, an overlay
layout — belongs to the **next** frame and is answered by it. The old loop
cleared the flag *after* `renderFrame()`, so such a redraw was silently lost
(VP-HARDENING finding R06). Never reintroduce that order.

The work reports two **distinct** flags. `changedThisTick` says the tick drew;
`stillActive` says a transition (a Home / Fit / ViewCube tween) needs another
frame. Collapsing them into one produces an extra idle frame when a tween ends.
A frame is rescheduled when the mask is dirty again **or** `stillActive`.

`invalidate()` with no argument means `appearance`, so every legacy caller still
works; pass the specific reason where it is known. Camera motion goes through
the controls' `onChange` (`camera`), `resize()` / `syncDpr()` pass `resize`, and
context recovery passes `recovery`.

**Submitted is not presented.** There are three milestones — *scheduled*,
*submitted*, *presented*. `renderer.render()` returning proves only that the
frame was **submitted**; a compositor may still not have shown it. Only native
capture is evidence of presentation. `onAfterRender(listener)` therefore hands
the listener a `FrameSubmission` — `{ submission, requestedRevision, publication,
displayedBodyIds, displayedProvenance }` — built immediately before the draw call.

**And `publication` is the DISPLAYED one, not the adopted one.** It is derived
from `getEntry(bodyId).provenance` for every visible body, never from the
registry's `getCurrentMeshPublication()` pointer: that pointer is adopted at the
top of a document change, *before a single mesh of the new publication has
landed*, so a frame drawn mid-ingest would otherwise claim the new snapshot while
showing the previous one's triangles. When the displayed bodies disagree —
a mixed frame, mid-ingest — `publication` is `null` and `displayedProvenance`
names each body's own stamp.

A `render()` that throws (or whose promise rejects) notifies **nobody**, counts a
failure, logs at most once per distinct message per 5 s (oldest key evicted, so a
recurring message keeps its window), and after three consecutive failures **parks**
the engine: `submissionHalted` is checked *before* `controls.update()`, because a
tween commits the camera and re-invalidates on every tick and would otherwise
keep the mask permanently dirty. A parked engine freezes its transitions and
draws again only on an `invalidate()` raised **outside** the frame work — one
attempt per request, re-parking on each failure — or after `retryRenderer()`.
For the same reason a camera invalidation raised *during* `controls.update()`
does not dirty the mask at all: that tick already draws the post-update camera,
so a tween of N advancing ticks costs exactly N frames.

`RendererLifecycle` is `constructing → active → lost → restoring → active`, with
terminal `disposed` and the bounded `error`. Context loss suspends the scheduler
(nothing may be submitted into a dead context) and keeps CPU state; restoration
rebuilds the PMREM environment on a microtask (after three's own handler),
re-applies the renderer size, resumes, and invalidates once with `recovery`.
A `webglcontextrestored` that arrives while the engine is **not** `lost` is
ignored, and a loss during the awaited construction is never overwritten with
`active` by `init()`'s tail. `captureThumbnail()` returns `null` outside `active`
rather than forcing a frame into a dead context.

`retryRenderer()` is the way out of `error`: it builds a **fresh canvas** —
a force-lost context can never be revived on the same element — plus a fresh
renderer, controls and picker, re-measures the CSS box before sizing, and returns
`false` without claiming success if that fails. After a *failed init* it re-runs
the whole `init()` path instead, because that engine has no grid, triad, section,
picker or observers and a `true` for that shell would be a false success claim
(`buildScene()` is idempotent, so the light rig is not duplicated).

The engine stays store-agnostic — it imports stores only as TYPES — so
`ViewportRoot` subscribes to `onLifecycleChanged` and owns the user-facing half:
a sticky warning while `lost`/`restoring`, a sticky error plus the **Retry
renderer** action in the viewport while `error`. Its init-failure catch routes
into that same state, so "WebGL never came up" and "recovery failed" look and
behave identically.

WebGPU is not selectable in production: `experimentalWebGpu` only produces the
once-per-session fallback diagnostic and `capabilities.backendNote`, and the
Settings toggle is rendered disabled with the reason. `RendererPrefs.allowUnsupportedBackends`
is the only route to `createWebGpu`, and **nothing in the app sets it** — it is
there for the capability lane WP15/WP16 may build.

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
   BEFORE every translucent fill and get tinted/stippled by it.
3. **Depth TEST is on for STATIC sketch content and OFF for the ACTIVE session**
   (audit item #1). Static sketches keep the old rule: solid bodies (opaque
   pass, depth-written, `polygonOffset` pushes faces back) occlude them. The
   live `SketchObject` reverses it — in sketch mode the camera looks down the
   plane normal, so a body between the eye and a coplanar plane hid the user's
   very first stroke entirely. `depthTest: false` plus the transparent pass
   (which runs after ALL opaque objects) paints it over bodies unconditionally.
   **Accepted consequence: orbiting mid-session x-rays the active sketch through
   solids** — standard CAD behaviour, and the price of the stroke existing on
   screen at all. Picking is untouched: sketch hit-testing is plane math
   (`sketchHitTest.ts`), not a raycast, and body picking reads neither
   `renderOrder` nor `depthTest`. The plane TINT and the plane grid are
   deliberately excluded — they are the surface, not the sketch.

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
| `renderer.ts`        | SOLE renderer construction; WebGL2 only + the one capability record. |
| `FrameScheduler.ts`  | Dirty-reason bitset, one rAF, consume-before-work (VP02).       |
| `ViewportEngine.ts`  | Orchestrator: scene graph, render loop, resize, actions.        |
| `CameraRig.ts`       | Persp+ortho pair; switch preserves apparent size at the pivot.  |
| `CadOrbitControls.ts`| Turntable orbit / pan / zoom-to-cursor; Home/Fit/snap tweens.   |
| `GridPlane.ts`       | Adaptive XY grid (1/2/5/10 decade step), re-centered on target. |
| `HtmlOverlayDriver.ts`| Projects world→screen and writes DOM transforms per frame.     |
| `palette.ts`         | Reads design tokens (tokens.css) via getComputedStyle; cached.  |
| `lightRig.ts`        | Pure camera-relative key/fill positions (floored key elevation).|
| `BodyObject.ts`      | Per-body face Mesh + fat edge `LineSegments2`; shared materials.|
| `Picker.ts`          | rAF-coalesced raycast → face/edge PickHit; edge screen-bias.    |
| `HighlightLayer.ts`  | Hover/selected highlight: owned face slices (cached), leased body, owned edges. |
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
`traverseVisible`, so a hidden pick proxy would be gathered too).

**Every screen-space fat line is authored in CSS (logical) pixels** — width,
`resolution`, and pick radius alike, never multiplied by the device pixel ratio
(VP03, finding R01). That is forced by the installed three 0.185.1, not chosen:
`examples/jsm/lines/LineSegments2.js:419-428` has `onBeforeRender`
UNCONDITIONALLY overwrite the uniform with `renderer.getViewport()`, and
`src/renderers/WebGLRenderer.js:781-785` returns the viewport `setSize` stored
UNSCALED (only `getDrawingBufferSize` applies the ratio). Since
`LineMaterial.js:239-243` computes `offset *= linewidth; offset /= resolution.y`,
a device-pixel width over a logical-pixel resolution draws `dpr` times too wide.
`screenLineStyle.ts` owns that contract and the `LINE_WIDTHS_CSS` table (spec
§7.3); `ViewportMetrics.ts` is the single snapshot the sizes come from, written
only by `resize()` and `syncDpr()`. `THREE.Points` markers are a SEPARATE
adapter — `WebGLMaterials.js:303-308` multiplies `material.size` by the pixel
ratio itself, so a point size is *also* CSS px but by a different route.

Three consequences the Picker owns, none of which apply to plain lines:

- an intersection carries **`faceIndex` = the segment ordinal directly** (the
  geometry is instanced, one instance per segment) and **no `index`**. There is
  no `>> 1`; adding one binds every edge pick to the wrong edge.
- the anchor is **`pointOnLine`** (on the segment), not `point` (on the ray).
- the raycast is **screen-space**: the hit radius is
  `(material.linewidth + raycaster.params.Line2.threshold) / 2` in the CSS px
  `material.resolution` is expressed in. `line2PickThresholdCss` cancels the
  drawn width out so the tolerance stays `EDGE_PICK_CSS_PX` at any weight, with
  no dpr term on either side, and the Picker **flushes `material.resolution`
  itself before every raycast** — at (0,0), which is where a body sits until its
  first rendered frame, `LineSegments2.raycast` returns silently with no hits.
  `params.Line.threshold` (world units) is still driven, because the
  face-vs-edge preference bias arbitrates on the world-space `distance` both hit
  kinds report.

Highlights follow the ownership contract of `docs/viewport-hardening/01-SPECIFICATION.md`
§8 (VP-HARDENING WP03, decision D05); the old shared-attribute wrappers that
were never disposed are gone (finding R02). Three shapes:

- **BODY** overlays borrow the body's EXACT `entry.geometry` object through a
  registry lease (`acquireLease(entry, "highlight:body")`) — no wrapper, no
  copy; the registry will not dispose a leased entry until the lease is
  released, one flush after the last holder lets go.
- **FACE** overlays are compact OWNED geometry (`mesh/faceSliceGeometry.ts`):
  the face's triangles are copied into their own position/index buffers with a
  local vertex remap (no normals — the overlay is unlit), one combined buffer
  per (body, role) for multi-face selection, capacity reused across selection
  changes. They live in `mesh/highlightCache.ts`, a bounded LRU (64 MiB / 256
  entries) pinned while displayed and dropped when their source entry retires.
  Disposing one frees only its own buffers.
- **EDGE** overlays build a `LineSegmentsGeometry` over a `subarray` VIEW of the
  entry's `edgeSegmentPositions` (CPU zero-copy, but their own
  `InstancedInterleavedBuffer`, so their own GL buffer) and are cached and
  disposed the same way.

Body objects and section stencils borrow the body geometry through leases too
(`"body"`, `"section"`); the registry is the unique disposer of installed
geometry (`meshRegistry.ts`, ordered frame-end retirement). Never call
`dispose()` on any geometry that shares the body's `BufferAttribute`s.

`SketchStaticLayer` (model-mode, non-editable presence of every sketch) picks
DIFFERENTLY: its `hitTest` raycasts an explicit object list it maintains itself,
not `traverseVisible`. Each sketch's curves are therefore two objects sharing one
geometry — a fat, VISIBLE `LineSegments2` draw pass (CSS-px width, see
`screenLineStyle.ts`) and a plain, INVISIBLE `LineSegments` pick proxy
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
├── sketchRoot          (all sketch presence)
│   ├── staticSketchRoot  (SketchStaticLayer — every document sketch)
│   └── activeSketchRoot  (the live SketchObject of the open session)
└── interactionRoot     (hover/selected highlight meshes — F-WP5)
```

**The sketch split is load-bearing** (audit item #9, the recorded §10.5
deferral). `setLayerVisible("sketches")` targets `staticSketchRoot`, so the
Layers filter hides the OTHER sketches and never the one being drawn, and
`SketchStaticLayer.setSessionActive()` drops static region fills (hidden AND
removed from its pick set — `intersectObjects` does not consult `visible`) and
dims static ink while a session is open. `sketchRoot` survives as their common
parent so "all sketch content" stays addressable in one place.

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

**A display change is an EVENT, not a poll.** `dpr.ts` owns `MAX_DPR` and
nothing else; the ratio it caps is the DRAWING-BUFFER ratio, never a width
multiplier. A window dragged between a 1x and a 2x display changes the ratio
with no CSS box change, so `ResizeObserver` never fires and render-on-demand
means there is no next frame to notice in. `dprWatcher.ts` arms a
`(resolution: Ndppx)` media query and RE-ARMS it at the new ratio on every
change (a query pinned to the old value reports only the first transition);
`ViewportEngine.syncDpr()` then recomputes the metrics snapshot, re-sizes the
drawing buffer, and schedules exactly one repaint without touching the camera.
The same `syncDpr()` still runs once per rendered frame as a safety net. Line
widths need no adjustment at all: they are CSS.

**Curve tessellation is adaptive** (`curveTessellation.ts`): segment counts come
from a 0.35 CSS px sagitta budget with a 25% rebuild band, floors of 8 (24 for an
ellipse) and a 2048 cap. This is a RENDERING budget only — snap candidates,
hit-testing and the solver all work against the exact analytic curve, and
changing the segment count must never move a snap point.

**Selected sketch points are held as REFS**, not coordinates
(`setSketchSelectedPointRefs`), and re-resolved on every `setSession`. The
coordinate form froze the ring where the point was at selection time, so a solve
that moved the point left the ring behind.

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
