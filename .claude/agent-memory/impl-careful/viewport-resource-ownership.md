---
name: viewport-resource-ownership
description: Mesh-registry leases, ordered frame-end retirement, owned highlight overlays, and the three.js buffer-release facts that constrain them
metadata:
  type: project
---

The mesh registry is the unique disposer of installed face/edge geometry (WP03, spec §8.2). Borrowers hold `acquireLease(entry, ownerTag)`; `swap`/`remove` mark the previous entry `retired` and `flushDisposals()` (one per rendered frame, it advances the frame counter itself) frees it only when `leaseCount === 0 && retiredAtFrame < currentFrame`.

**Why:** three 0.185.1's `WebGLAttributes` has NO listener on a BufferAttribute's own `dispose` event — verified in `node_modules/three/src/renderers/webgl/WebGLAttributes.js`. A GL buffer is deleted only by `onGeometryDispose`, which iterates the geometry's CURRENT attributes. So (a) `attribute.dispose()` frees nothing, (b) replacing an attribute on a live geometry orphans its buffer until context loss, and (c) an undisposed shared-attribute wrapper leaks renderer bookkeeping forever. That is finding R02.

**How to apply:**
- Never `dispose()` a geometry that shares attributes with a body — free the body's buffers and the body goes black.
- To resize an owned buffer, dispose the whole geometry and build a new one (`OwnedFaceGeometry.grow` in `faceSliceGeometry.ts`); never swap the attribute in place. Same fix applies to the four `setAttribute("position", new Float32BufferAttribute(...))` marker sites in `SketchObject.ts` (WP11).
- `disposeAll()` must empty the registry BEFORE notifying `onEntryRetired` listeners, or a listener that rebuilds (HighlightLayer) takes fresh leases on the bodies being closed.
- Every borrower is an OWNER or a LESSEE — no third kind. Lease tags in use: `body` (BodyObject, committed and preview), `section`, `highlight:body` (whole-body + degraded outline), `ghost` (unranged ghosts only). Ranged ghosts and highlight overlays OWN compact copies.
- `buildBodyObject` leases and stamps `faceMesh.userData.meshEntry`; `SectionLayer.createPair` leases THAT object and refuses (one `logWarn`, pair stored as `null` so it never re-warns) when `entry.geometry !== mesh.geometry`. `stencilCount` counts non-null pairs.
- `ViewportEngine` disposes every preview handle at its four removal sites. The old `BodyObject` detached-borrower sweep is GONE — it only ran on a retirement event, so it was never equivalent to ownership.
- Owned overlays live in `HighlightCache` (64 MiB / 256 entries, pinned while displayed). Multi-face selection keeps ONE buffer per (body, role) by `cache.take()`-ing the previous ordinal key and rewriting it.
- `reserve(bytes, reason)` returns a RECEIPT or null and `put(key, value, receipt)` refuses a value larger than it. Price with `planFaceSetCapacity(entry, ordinals, reuse)` — `estimateFaceSetBytes` is a lower bound that the ×1.5 growth rule beats on every growth step (4→5 triangles: estimate 240, actual 288), and `peakBytes` adds the outgoing buffer. `OwnedFaceGeometry.update` calls the same planner, so plan and allocation cannot drift.
- Degraded highlight = leased `entry.edgeGeometry` outline + a count chip via `HighlightDeps.onDegraded` → `ViewportEngine.syncDegradedChips`; re-attempted on every rebuild, and `HighlightLayer.degraded` reads the DISPLAY state. `HighlightDeps.budget` exists so a test can exhaust it.
- `LineSegments2.computeLineDistances()` writes two new attributes onto the geometry it is called on, so a DASHED style is unavailable on a leased registry geometry without mutating a borrowed resource — the degraded outline is solid by weight and colour instead.

Related: [[viewport-frame-lifecycle]], [[viewport-screen-units]].
