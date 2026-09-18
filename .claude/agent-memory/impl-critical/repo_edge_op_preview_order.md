---
name: repo-edge-op-preview-order
description: Edge-op preview publication order + H4b hardening (2026-09-17) — open token, ✓ waits and re-judging, committing freeze, lookup chain, lifecycle request base, re-edit fetch, deferred test harness
metadata:
  type: project
---

`ModelToolController.openEdgeOpPreview(gen, prepare?)` takes a token (`edgeOpOpenSeq`) on EVERY call;
only the newest open in the same `armGen`, with the fillet `armed|dragging` and the lane free or
`edgeOp`, may install (`edgeOpOpenMayPublish`). Replacement goes through `closePreviewSessions`.

**Why:** several opens can be in flight in one arm (type flip + same-turn chamfer sync, failed-commit
re-arm + [Flip reference]); `armGen` alone let an older draft overwrite and leak a newer session.

**How to apply:**

- Any pre-open async step goes in `prepare` (token taken at request). EVERY writer of `chamferPairs`
  runs on `enqueueChamferStep` (counted in `chamferSyncPending`); `resolveChamferReferenceFaces`
  loops until its `chamferLookupInputs()` snapshot is unchanged across the round trips.
- `armGen` is a getter/setter; `nextArmChange()` resolves at the next change — race long awaits with
  it (`settleEdgeOpPreviewOpens(armGen)`, the range await) so a left arm's ✓ lets go at once.
- `commitFillet` waits ONLY when `edgeOpPreviewInFlight()`; an unconditional await breaks
  `edgeShellPreview.test.ts` (`onConfirm(); answerPreview()` same turn). After every await it calls
  `edgeOpConfirmSurvives` (arm, phase, retained failure, CURRENT validation → "Cannot confirm …").
  A dropped arm is always logged; the hint posts only if the line is empty or `EDGE_OP_CONFIRM_WAIT_HINT`.
- Committing freeze: `edgeOpFrozenForCommit()` guards every edge-op entry (segment, d2, angle, flip,
  size, suggestion, revert, sync, `sendPreview`); `ActiveToolInspector` disables its fieldset on
  `presentation.phase === "applying"` too. The reducer ALSO refuses sub-minimum setRadius/setDistance2.
- NEVER call `this.throttle.reset()` directly: use `resetPreviewThrottle()`, which advances
  `lifecycleRequestBase` — `setPreviewLifecycle` drops a lower (arm, request), so a same-arm reopen's
  restarted epochs were silently ignored.
- History re-edits fetch through `fetchReeditParams` (no armGen bump; abandons a live drag) and call
  `invalidateArm()` only once they proceed — `gestureLifecycle.test.ts` pins the drag ending at request.
- The handle mapping no longer rejects the edge tangent at all (β = 0) — see [[repo-frozen-handle-mapping]].
- Tests: `ModelToolController.edgeOpPreviewOrder.test.ts` — deferred per `beginPreview` (+ `fail`),
  microtask auto-answers, parked promotions/ranges, `answerPromotesInOrder` (FIFO backend = microtask
  drain between answers), `buildsOnRealLane` via `buildPreviewOp`. Cast `throttle.setTrailingMs(0)`
  to make sends deterministic.
- Extrude depth 0 (or a two-direction re-edit's distance-driven depth2 0) is suppressed: no send, no
  drawn candidate, lifecycle `none`. Re-edit seeds the stored second direction (`seedStoredSecondDirection`).
