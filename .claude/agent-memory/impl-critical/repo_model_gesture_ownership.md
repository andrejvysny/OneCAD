---
name: repo-model-gesture-ownership
description: ModelToolController value-drag ownership (H1/H1b + H3/H5, 2026-09-17) — gesture record, release/cancel/abandon, stale-arm guard, gestureLive, grab-time handle mapping + freeze, jsdom traps
metadata:
  type: project
---

`ModelToolController` value drags (extrude, fillet/chamfer, shell, offsetFace, revolve, transform gizmo)
are owned by ONE `gesture` record (`src/tools/modelTools/modelGesture.ts` holds the pure rules).
`dragging` is now a GETTER over it — assigning it is a compile error, by design.

**Why:** a missed pointerup used to strand `toolStore.phase === "dragging"` (wheel nav blocked, Enter
dead, Escape dropped the tool, hover kept mutating the value).

**How to apply:**

- A new grab site calls `beginGesture(e, kind)` BEFORE the grab mutates the FSM (transform's grab step
  re-types mode/axis/copy); extend `GestureSnapshots` with every field the drag or its side effects write.
- Three endings: `releaseGesture` (keep value), `cancelGesture` (restore only if `armGen` unchanged AND
  FSM still dragging), `abandonGesture(kind?)` (teardown; writes no value, only takes "dragging" off the
  FSM and off `toolStore` when still set). Every teardown/cancel path must call abandon.
- A gesture whose `armGen` is stale is ABANDONED on its next move/up/cancel (`abandonedAsStale`), and every
  arm install of a gesture tool (`beginExtrudeArmed`, `beginRevolveArmed`, edge-op arm + re-edit, `armShell`,
  `armOffsetFace`, `editOffsetFaceFeature`, `armTransform`, `enterRegionPick`) abandons before replacing
  state. `setTool(sameTool)` does NOT fire `onToolChange`, so re-edits never get the sweep for free.
- `armLanding` refuses grabs while an arm's kernel round trip is in flight (extrude sessions opening,
  offset-face handshake) because the landing re-seats the value; it is gen-keyed, so it goes inert on supersede.
- Drag ACTIVITY is `toolStore.gestureLive` (written only by begin/end gesture); `phase` is rewritten to
  "armed" by any `setTool` on the active tool. `ViewportRoot.viewportDragActive` reads `gestureLive`.
- With the stale-armGen guard, swapping `abandonGesture()` for `cancelGesture()` in `onToolChange` (after
  `invalidateArm`) is behaviour-IDENTICAL; a mutation probe for "abandon, not cancel" must cancel BEFORE
  the arm is invalidated.
- New commit/confirm entries start with `if (this.gestureBlocksCommit()) return;`.
- `lostpointercapture` is handled one task LATE on purpose (CadOrbitControls releases capture inside the
  pointerup dispatch); a synchronous cancel reverts every normal release.
- The restore preview sets every session's `lastAppliedEpoch` to the flush epoch: SECONDARY sessions have
  no throttle guard, so without that floor a stale refusal re-latches `previewFailure`. Refusal state is put
  back to what it was AT THE GRAB (per session id), not nulled.
- jsdom 29 HAS `PointerEvent` (pointerId default 0, pointerType "", isPrimary false); `MouseEvent` has no
  pointerId (→ null = matches any pointer). The missed-release rule fires for `pointerType` "mouse" or "pen"
  with `buttons & 1 === 0`, so a typed synthetic move MUST carry `buttons: 1`. A new `isPrimary` contact of the
  same type under another id releases a stale gesture, so touch tests of "foreign pointer" must leave
  `isPrimary` false.
- `vi.getTimerCount()` under `vi.useFakeTimers()` (switched on AFTER a real-timer arm) is the way to prove a
  timer does not leak past `dispose`; use a no-lane kind (transform) so no trailing preview timer counts.
- `window.__extrudePreview` is NOT refreshed on fillet/shell/offset drag frames (no `updateDebug`); read the
  live value from `toolChipStore.getState().value`, or the last `updatePreview` params.
- `CadOrbitControls` gates wheel orbit/pan/zoom, WebKit gesture pinch AND two-pointer pan/pinch on
  `isDragActive` (spec §7.8); the gated two-pointer path still tracks `lastPinch` so ungating does not jump.

## Grab mapping (H3/H5, 2026-09-17)

- Every value grab reads `engine.valueHandleMapping()` ONCE (`grabHandleMapping`) and freezes it; `endGesture`
  releases the freeze. The extrude/offset axis branch keeps ray math; every proxy (end-on arrow, degraded
  fillet/shell/offset) maps screen travel at the grab's `worldPerPx`, never the per-move `planePixelWorld()`.
- `showValueHandle`/`showScreenValueHandle` call `DragHandle.reset()`, which CLEARS the freeze. Offset re-shows
  its arrow every frame, so `applyOffsetFaceState` re-freezes; extrude's `setExtrudeHandle` does not reset.
- Test doubles without `valueHandleMapping` fall back to axis (extrude/offset) or screen-up proxy (fillet/shell)
  at `planePixelWorld()`. `ModelToolController.handleMapping.test.ts` has a camera-backed engine double (real
  `DragHandle` + THREE camera + `worldPerPixel`) — reuse it for any mapping assertion.
- Extrude drag frames are no-ops until `moved` (>`DRAG_PX` per axis from `downX/downY`); `forceExtrudeGrab`
  counts as moved. `downX/downY` start at 0, so pinning the forced case needs a move within 4 px of (0,0).
- The fillet arm's own validation reads VALID after `showFillet` (it clears the pending set by
  `armEdgeOpRange`); only a TYPED value goes `pending` while the range check is in flight.
- Typed extrude values (`onExtrudeChip`) and drag frames do not refresh `__extrudePreview.depth`; read
  `toolChipStore.value`.
