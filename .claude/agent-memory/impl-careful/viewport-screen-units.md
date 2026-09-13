---
name: viewport-screen-units
description: OneCAD viewport CSS-pixel contract for fat lines, points and picking — what the installed three 0.185.1 actually does, and the jsdom traps in testing it
metadata:
  type: project
---

Durable facts about the screen-space unit contract in `src/viewport/engine`,
established by VP-HARDENING WP01 (VP03 / finding R01).

**Why:** every one of these was read off `node_modules/three` at 0.185.1 rather
than inferred, and each contradicts a plausible-sounding assumption that was in
the code before. Getting the unit wrong is silent: nothing throws, the line just
draws at the wrong weight or the pick radius quietly moves.

**How to apply:** read before touching any `LineMaterial`, `THREE.Points`,
pick radius, or the engine's size/DPR plumbing.

- `LineSegments2.onBeforeRender` (`examples/jsm/lines/LineSegments2.js:419-428`)
  UNCONDITIONALLY overwrites `material.resolution` from `renderer.getViewport()`.
  `WebGLRenderer.js:781-785` returns the viewport `setSize()` stored UNSCALED
  (only `getDrawingBufferSize` applies the ratio), so the draw-time resolution is
  LOGICAL/CSS pixels. Therefore `linewidth` must be CSS too — the shader does
  `offset *= linewidth; offset /= resolution.y` (`LineMaterial.js:239-243`), so a
  device-px width over a logical-px resolution draws `dpr`× too wide.
- An application write to `material.resolution` only ever affects a raycast
  BEFORE the first rendered frame (`LineSegments2.raycast` early-outs at (0,0)).
  Nothing else. Write CSS there or the pick disagrees with the draw.
- `LineSegments2.raycast` maps the pointer's NDC offset into *resolution* units,
  so the acquisition radius measured in CSS pointer travel is
  `(linewidth + Line2.threshold) / 2 * (cssSize / resolution)`. Feeding a
  device-px resolution HALVES the effective CSS radius (6 → 3), it does not
  double it — the intuition runs the wrong way.
- `THREE.Points` is a different route to the same unit: `WebGLMaterials.js:303-308`
  sets `uniforms.size = material.size * pixelRatio` and `points.glsl.js:34` does
  `gl_PointSize = size` in DEVICE px, so with `sizeAttenuation:false` the authored
  `material.size` is already CSS px. Never fold points into the line adapter.
- The one size/DPR truth is `engine/ViewportMetrics.ts` (immutable, identity-
  compared: `nextMetrics` returns `prev` verbatim on a no-op). Only
  `ViewportEngine.resize()` and `syncDpr()` write it.
- `src/test/setup.ts` stubs `window.matchMedia` globally with `vi.fn()` listeners
  that can never be fired. A test that needs to fire a media-query `change` must
  `vi.stubGlobal("matchMedia", …)` with its own capturing fake.
- jsdom lays nothing out, so an engine container's `clientWidth` is 0 and
  `ViewportEngine.resize()` bails before touching metrics. `Object.defineProperty`
  `clientWidth`/`clientHeight` on the container to exercise the metrics path.
