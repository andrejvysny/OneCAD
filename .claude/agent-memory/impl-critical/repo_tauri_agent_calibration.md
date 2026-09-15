---
name: repo-tauri-agent-calibration
description: tools/tauri-agent calibration invariants — the read-only wheel probe, why the app's wheel listener is beatable, checkPoint as the only input gate, and the test fake that constrains script names
metadata:
  type: project
---

Facts about `tools/tauri-agent/src/session/calibrate.ts` (WP-B, side-effect-free calibration).

- **Calibration must not change app state.** `session_start`, recalibration and the lazy
  `ensureWheelProbe()` may move the cursor but must not move the camera, selection, document
  or history. The wheel probe therefore swallows its own notch in the page.
- **The app's wheel handler is beatable from `window` capture.** `data-testid="viewport-canvas"`
  is the CONTAINER div (`src/viewport/ViewportRoot.tsx`); `CadOrbitControls` binds to the
  `<canvas>` INSIDE it (`ViewportEngine.ts` passes `element: this.canvas`), bubble phase,
  `{passive:false}`. So a `{capture:true, passive:false}` listener on `window` runs first and
  `stopImmediatePropagation()` stops the notch reaching it. Measured in jsdom: old probe
  (capture on the container, `once+passive`, records only) ⇒ app handler fires; new one ⇒ 0.
  `passive:true` is what made the old `preventDefault()` a no-op.
- **A probe listener cannot be `once`.** It must survive a wheel that is not ours (the user's),
  which it identifies by `clientX/clientY` within a few px of the probe point. It therefore
  needs an explicit disarm plus an in-page `setTimeout` guard — a dropped bridge must never
  leave the page eating the user's wheel.
- **`Engine.scroll` in the Swift helper moves the cursor to `p` and sets `ev.location = p`**
  (`src/platform/macos/helper/main.swift`), so the page really does see the notch at the
  probe point. Positional discrimination depends on that; do not remove the move.
- **`checkPoint` (`src/geometry/mapping.ts`) is the only legal gate before native input** —
  `pointInWindow` alone skips the native occlusion rects (traffic lights). A fixed step-off
  offset walks the cursor into another app for a probe near the right/bottom edge; the
  step-off is a candidate search over `checkPoint`.
- **`tests/orchestrator.test.ts`'s bridge fake throws on any script name outside
  `{calibrate.pickProbe, calibrate.hover, calibrate.wheelArm, calibrate.wheelRead}`** and always
  answers `wheelArm` with `rect: null`. Adding a calibration script that the skip path reaches
  breaks it. `calibrate.wheelDisarm` only runs after a successful arm, which that fake never
  reaches — keep it that way or the fake needs updating.
- **`PipelineSession` (`src/mcp/tools/actionPipeline.ts`) does not expose `calibration`**, so
  `pointer_scroll` detects "the lazy wheel probe ran inside this action" by object IDENTITY
  against `session.calibration?.wheelProbe` behind a local cast. `WheelProbeLike` is narrower
  than `WheelProbe` (no `attempts`).
