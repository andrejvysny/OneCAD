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
- `buildBodyObject` leases; `ViewportEngine`'s exact-preview handles never call `handle.dispose()`, so `BodyObject.ts` carries a detached-borrower sweep (entry not `installed` AND `group.parent === null` ⇒ release) that runs on retirement only.
- Owned overlays live in `HighlightCache` (64 MiB / 256 entries, pinned while displayed). Multi-face selection keeps ONE buffer per (body, role) by `cache.take()`-ing the previous ordinal key and rewriting it.

Related: [[viewport-frame-lifecycle]], [[viewport-screen-units]].
