---
name: repo-shell-wall-attachment
description: H9 (2026-09-17) — Shell's retained-wall handle, the rim-adjacency refusal, axisDepthFromRay returning null, and the chip anchor that rides H(q)
metadata:
  type: project
---

Shell no longer shows a proxy at `faces[0]?.anchor ?? [0,0,0]`. `armShell` resolves a
RETAINED WALL once, before it publishes the handle or the chip, and the resolution is
`await`ed inside the arm (it may issue one `classifyElement`).

**Why:** `docs/design/astra/modeling-handle-attachment.md` §5 "Shell". The removed lid's
own normal describes cavity depth, not wall thickness, and a re-edit supplies no picks at
all — which is how the handle used to land on the world origin (review R07).

**How to apply:**

- `src/viewport/mesh/faceEdges.ts` owns the adjacency: `retainedRimNeighbours(view,
  removedFaceOrdinals)` returns `{faceOrdinal, edgeOrdinal}` sorted by face then edge, and
  keeps ONLY edges exactly one retained face bounds. Two matches is not a choice — a 40 mm
  cube at fine LOD has a `2δ = 0.069282 mm` tolerance, so a face 0.05 mm away passes the
  same test. `faceBoundaryEdgeOrdinals` is now a thin wrapper over the same
  `faceSurface`/`edgeOnSurface` pair; keep it that way or the rim search costs O(E·P·T)
  instead of O(B·P·T).
- `src/tools/preview/shellThickness.ts` owns the paths: `planarWallPath` (`H = E − t·n`)
  and `cylindricalWallPath` (`H = C + (R − σt)·r̂`, σ = +1 pin / −1 hole), plus
  `cylindricalWallRadiusAt` and `shellThicknessWitness` (`targetConstruction`, label
  `"Thickness target t — construction"`). The cylinder path seats `point0` at the
  CLASSIFIED radius — the rim point only supplies r̂, because a faceted vertex sits inside.
- The controller ladder is `wall → rim → pick → body → none` (`debug().shellAttachment`).
  `none` (no mesh AND no pick) HIDES the handle rather than drawing one at the origin.
  `classifyElement` is called UNFENCED there on purpose — it positions a glyph, the way
  the placement hover reads do, and `armGen` plus the document-change arm drop cover it.
  Absent `sidedness` means NOT MEASURED, so σ is never assumed; a non-positive predicted
  radius drops the path at arm and keeps the labelled chip.
- **`axisDepthFromRay` now returns `number | null`.** The parallel branch used to project
  the ray ORIGIN onto the axis — a camera-tracking quantity under a pointer-tracking name.
  The guard is scale-relative (`γ` on `a·e − b²`), so a short direction vector no longer
  trips it. Callers decide the lane ONCE at the grab via `gestureAxisRay`; a mid-drag
  refusal HOLDS the frame. `forceExtrudeGrab` must set `gestureAxisRay = true` or every
  gate-lane extrude drags at the proxy gain.
- The CHIP label is anchored at `H(q)` for both the edge op and Shell, pushed through
  `engine.moveChip(MODEL_TOOL_CHIP_ID, …)` and never through `toolChipStore.worldPos`
  (that field is the mount effect's key). The edge op also passes `path.point0Mm` as the
  leader-line `axisFrom`; Shell has no `anchorAxisFrom` because `showShell` takes no opts.

**Test-harness traps:**

- A test fake whose `screenRay` runs PARALLEL to the extrude axis now yields no depth. The
  perpendicular fake (`dir: [1,0,0]` against a +Z axis) gives the IDENTICAL number — both
  branches reduce to `−c` when `a = e = 1, b = 0` — so that is the drop-in replacement.
- `edgePolylinePoint` on a faceted circle returns a CHORD INTERIOR for any anchor not
  exactly on the polyline: an anchor at the cap centre lands at `R·cos(π/segments)`. Put
  the pick anchor ON the rim to get the analytic vertex.
- `showScreenValueHandle` now takes `(anchor, thicknessMm)`; a `toHaveBeenCalledWith([x,y,z])`
  assertion fails on arity alone.
