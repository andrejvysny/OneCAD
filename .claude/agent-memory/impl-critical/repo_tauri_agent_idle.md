---
name: repo-tauri-agent-idle
description: tools/tauri-agent settling after WP-E — the ui_idle tuple replaced ui_revision in settle(), which in-page signals exist and when, and why unregistering the ?vpdebug pill is not enough on its own
metadata:
  type: project
---

Facts about `tools/tauri-agent`'s CAD/WebGL-aware settling (WP-E).

- **`settle()` reads the script named `ui_idle`, NOT `ui_revision`.** `captureBefore`
  (`actionPipeline.ts`) still reads `ui_revision`, so a test fake that cans `ui_revision` to
  throw now exercises only the BEFORE state — the settle-failure cases must can `ui_idle`.
  `tests/fixtures/fakeEnv.ts`'s `FakeBridge` answers both, and has a mutable `idle` field
  plus `canned["invoke:<command>"]` for `bridge.invoke`.
- **A null idle signal means "unavailable in this build", never "idle", and the two lanes
  treat it differently on purpose.** `settle` warns and still returns (blocking would hang
  every action on a build that lacks the signal); an explicit `wait_for worker_idle` /
  `render_idle` / `camera_stable` refuses to resolve from an unknown and times out reporting
  which signal was missing — the caller asked for that fact specifically.
- **`window.__stores` is installed ASYNCHRONOUSLY** — a `Promise.all` of dynamic module
  loads under `import.meta.env.DEV` in `src/main.tsx` — so it is legitimately absent for the
  first reads after app start, and every store read must degrade field by field.
- **Unregistering the `?vpdebug` origin pill is NOT enough to hide it.**
  `HtmlOverlayDriver.unregister` only stops the driver writing to the element; the last
  frame's `transform` and `display` stay on it, so an attached node freezes on screen. The
  node must also be detached (`[data-vp-debug-label]`), which additionally neutralises
  `ViewportEngine.setDebugOriginVisible(true)` re-registering it when a sketch session closes.
- **That pill is the ONLY visible chrome `?vpdebug` adds.** Every other `this.debug` branch in
  `ViewportEngine.ts` and `deps.debug` in `ModelToolController.ts` writes page globals
  (`__vpEngine`, `__vpFrames`, `__vpFrameTimes`, `__extrudePreview`) or internal accounting.
  `?trace` only opens the FE log-ring gate, which a DEV build has open anyway.
- `ViewportEngine.debugSnapshot()` is the only public camera read and it walks the scene
  (`getSceneBounds()`), so it is not free at poll frequency; `frameCount` is a plain getter.

**Why:** each of these is invisible from the type signatures, and the first two decide whether
a delivered action is reported as verified.
**How to apply:** when touching `tools/tauri-agent/src/mcp/tools/settle.ts`,
`src/semantic/idleScript.ts`, `src/mcp/tools/wait.ts`, or any fake that drives them.
