---
name: repo-frozen-handle-mapping
description: H8 value-handle mapping (2026-09-17) — FrozenMapping replaces HandleMapping/freezeStrategy, β = 0, the exact perspective inverse, scale-relative guards, one representative edge, and the test doubles that break on it
metadata:
  type: project
---

`src/tools/preview/handleProjection.ts` owns the WHOLE value-handle mapping now: `LinearHandlePath`
(`H(q) = point0 + dPointDValue·(q − q0)`) → `classifyMapping` → `FrozenMapping`
(`world | proxy | disabled`) → `sampleMapping`. `HandleMapping`, `HandleStrategy`,
`classifyHandleMapping`, `axisConditioning`, `projectRejectedAxis`, and `filletRadius`'s
`signedValueFromDrag` / `SCREEN_UP_AXIS` / `ScreenAxis` are GONE.

**Why:** `docs/design/astra/modeling-handle-attachment.md`. A rejected tangent is a direction the
moving point cannot follow (full rejection at a 1° screen angle multiplies the gain by 57.3); a
constant `p/g` gain is wrong under perspective (50 px on the P60 case is 8.934710 mm, not 9.622504);
absolute `1e-6` / `1e-9` floors break similarity and unit invariance.

**How to apply:**

- `DragHandle.freeze(FrozenMapping | null)` replaces `freezeStrategy`, and `orient()` draws the
  frozen object VERBATIM — no per-frame re-projection while a gesture owns it. `setValuePath(path,
  value)` + `setValue(v)` move the arrow to `H(q)`; `setAxis` is the `q0 = 0` degenerate form the
  extrude/offset per-frame re-anchor still uses.
- `ModelToolController.grabHandleMapping(fallback)` MUST keep its `"world" | "proxy"` argument: many
  engine doubles have no `valueHandleMapping`, and an extrude grab that falls back to `proxy`
  silently leaves the ray branch and drags at ~10× (5 extrudeGesture + 1 depthRounding tests).
- A mapping the seam reports as `disabled` is KEPT (no orbit-metric substitute): the drag goes inert
  and the chip stays typeable. `sampleMapping` on it returns `q0` unchanged.
- `validDeltaMm` is in Δq and `sampleMapping` clamps in PIXELS via the forward map, so the pole end
  (`1 + kΔq → 0`, p → ∞) is unreachable and only the τ-envelope end ever saturates.
- Fillet/Chamfer attach to ONE representative edge (first user-picked prepared edge, else the first):
  `edgePolylinePoint` for E, `edgeOutwardAt` for b at that point. `averageOutward` is no longer used
  by the controller — a closed tangent chain's mean cancels. `filletAxisSource` is now that one
  edge's tier, so an `auto` type flip can arm where a mixed-tier mean previously blocked it.
- The witness (`edgeParameterWitness`) carries `meaning: "parameterConstruction"` and never claims a
  blend contact: the true convex midpoint moves `−(√2−1)·q·b`, the opposite way, and reverses when
  concave. `ViewportEngine.hideValueHandle()` hides the witness too — they share a lifetime.

**Test-harness traps:**

- `handleMapping.test.ts`'s mock `prepareEdgeOp` stamps ONE anchor on every prepared edge; with an
  anchor present `edgePolylinePoint` returns the closest point to it, so a curved-edge attachment
  test must null that anchor out (`client.prepareState.anchor = null`) to get the arc-length midpoint.
- The arm seats the handle at the DEFAULT size, so `worldAnchor()` right after an arm is `H(2)`, not
  `E`. An edge-op type flip reseeds the size and therefore MOVES the handle — re-read the mapping
  after it.
- A test double returning the old `{strategy, pxPerWorld, worldPerPx}` shape throws
  `undefined is not iterable` inside `sampleMapping` (destructuring `validDeltaMm`).
