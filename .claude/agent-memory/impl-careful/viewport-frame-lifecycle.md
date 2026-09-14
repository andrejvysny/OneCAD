---
name: viewport-frame-lifecycle
description: OneCAD viewport frame scheduling, submission records and renderer lifecycle — the WP02 contracts and the jsdom/test-harness traps in exercising them
metadata:
  type: project
---

Durable facts about `src/viewport/engine/{FrameScheduler,ViewportEngine,renderer}.ts`
established by VP-HARDENING WP02 (VP01/VP02, findings R06/R17).

**Why:** every one cost a test cycle to find, and each contradicts a plausible
assumption. Getting the frame order wrong is silent — the redraw is just missing.

**How to apply:** read before touching `invalidate()`, `renderFrame()`,
`onAfterRender`, context-loss handling, or the engine's mocked-renderer harness.

- `FrameScheduler.tick()` SNAPSHOTS AND CLEARS the dirty mask before calling the
  frame work. Never move the clear after the work — that was R06, and the old
  `renderFrame(); dirty = false` order silently dropped any redraw asked for from
  inside a frame.
- The scheduler needs the work function BEFORE the rAF fires, so it exposes
  `setWork()` in addition to the spec's `tick(now, work)`; the frame timestamp
  rides on the `ConsumedFrame` record because `controls.update(now)` needs it.
- `ViewportEngine` starts the scheduler SUSPENDED in its constructor and resumes
  it at the end of `init()`. That is what reproduces the old "scheduleFrame bails
  when not initialized" guard WITHOUT losing a pre-init `invalidate()`.
- `CadOrbitControls.update()` calls `commit()` → `onChange()` on EVERY tween
  tick including the last, so a tween re-dirties the mask from inside the frame.
  Two consequences the engine now handles explicitly: `advancingTransitions`
  drops camera invalidations raised during `controls.update()` (else every tween
  costs one extra frame — measured 7 frames for a 6-tick Home tween), and the
  submission-failure park is tested BEFORE `controls.update()` using an
  `externalWake` flag (`invalidate()` while `!inFrame`), because "is the mask
  dirty" can never be false while a tween runs — that read let a throwing
  renderer make 86 draw attempts across 5 tween rounds.
- `FrameSubmission.publication` is derived from `getEntry(bodyId).provenance`
  for the VISIBLE bodies, never from `getCurrentMeshPublication()`: that pointer
  is adopted at the top of `meshSync.onDocumentChanged`, before any mesh of the
  new publication lands, so it names what was asked for, not what is on screen.
  Disagreeing bodies ⇒ `publication: null` plus a `displayedProvenance` row each.
- The controls' tween start time is `performance.now()`, so a test that flushes
  rAF with a synthetic `t = 16` never finishes a tween. Flush with
  `performance.now() + k`.
- A fresh `CadOrbitControls` starts at the DEFAULT view — `retryRenderer()` has
  to carry the camera across by `getViewState()` / `setView(view, false)`.
  Replacing the canvas is mandatory: a force-lost GL context can never be revived
  on the same `<canvas>` element.
- `ViewportEngine.test.ts`'s rAF harness now stores `{id, cb}` and its
  `cancelAnimationFrame` really removes the entry. The old no-op cancel made a
  cancelled frame look like a phantom pending one.
- The engine is store-agnostic by convention (only TYPE imports from `@/stores`).
  User-facing state — the "graphics context lost" hint — is published by
  `ViewportRoot` off `engine.onLifecycleChanged`, never from the engine.
  `viewportStore`'s severities are `info | warn | error`; there is no `warning`.
- `renderer.ts` reads its `RendererCapabilities` ONCE at construction and must
  tolerate a stubbed renderer with no `getContext()` (the whole unit lane). The
  `EXT_clip_control` probe uses `getSupportedExtensions()`, not `getExtension()`,
  so nothing is enabled as a side effect.
- The WebGPU path is now reachable ONLY via `prefs.allowUnsupportedBackends`;
  `experimentalWebGpu` alone just sets `capabilities.backendNote =
  "webgpu-preference-ignored"` and trips a module-level once-per-session warn
  latch (which no test resets — order the assertions accordingly).
- `ViewportRoot` mounts fine in jsdom and its `createRenderer` genuinely rejects
  (no WebGL), so the renderer-`error` UI can be tested through the real
  production path — `render(<ViewportRoot />)` then `findByTestId("renderer-retry")`.
  Spy the action with `vi.spyOn(ViewportEngine.prototype, "retryRenderer")`.
- `buildScene()` is idempotent because `retryRenderer()` re-runs the whole
  `init()` path after a FAILED init; without the guard the second pass would add
  a second light rig and leave the first in the scene.
- Exception-ISOLATING `onAfterRender` listeners is not enough on its own: a
  swallowed listener error that invalidated first loops forever, because the
  frame then succeeds and the scheduler's throwing-tick bound never sees it.
  The engine therefore counts consecutive frames with a throwing listener and
  parks on `SUBMIT_FAILURE_LIMIT` via the same `submissionHalted` + `externalWake`
  path as a failed submission. Isolation and boundedness are two separate jobs.
- `FrameScheduler.invalidate()` during a tick (`inTick`) only writes the mask —
  the tail of `tick()` is the ONLY place a frame is taken, and a park there also
  `cancelPending()`s. Reverting either half restores PR-05's permanent loop
  (measured 12 attempts with a frame still queued against an expected 3/0).
