---
name: tauri-agent-session
description: Session-lifecycle facts for tools/tauri-agent — detached spawn needs node:child_process, the configured hover probe does not exist on the start screen, dev launch is ~4 s warm, and the survivor sweep's owner filter
metadata:
  type: project
---

`tools/tauri-agent/src/session/` (orchestrator + launch/startup/calibrate/teardown/procs/types).

- **`Bun.spawn` has no `detached` option; `node:child_process.spawn({detached:true})` under bun
  does work** and gives `pgid === pid` (verified). The launcher must lead its own group or a
  SIGTERM to `tauri dev` leaves the app holding ports 4445/1420.
- **`calibration.probeTestId` defaults to `document-title`, which lives in `TitleBar.tsx` and is
  therefore ABSENT on the start screen** (only `EditorScreen` renders it). The hover probe falls
  back to any small hit-testable element and reports `usedFallback:true`; on the live smoke it
  picked a sidebar `button`. Do not "fix" a fallback report — the probe measures the mapping, not
  that element.
- `[data-testid="viewport-canvas"]` is also absent on the start screen, so a start-screen
  calibration legitimately yields `wheelProbe: {skipped:"no viewport"}`; `ensureWheelProbe()`
  re-runs it once a document is open.
- **A warm `bun run tauri:agent` reaches WebDriver `/status` in ~3-4 s** (cargo cached, vite
  ~150 ms). The 600 s `readyTimeoutMs` is for the first `--features tauri-e2e` build only; pre-warm
  with `cd src-tauri && cargo build --features tauri-e2e` before a live run.
- The survivor sweep kills a pid only when its command line contains BOTH a `processPatterns`
  entry AND the project root (or the bundled app path) — `onecad-worker-` alone would match another
  checkout. Keep that conjunction if the sweep is ever touched.
- `SessionOrchestrator` takes an optional `deps` bag (`runner`, `platform`, `bridgeFactory`,
  `fetch`, `spawn`, `kill`, `sleep`); `server.ts` passes none. `tests/orchestrator.test.ts` fakes
  all of them, so start/stop decisions are testable without an app.
- Live acceptance: `bun tools/tauri-agent/scripts/smoke.ts` (launch → calibrate → one window
  screenshot → stop → `pgrep`). It moves the real cursor once and fronts the app, by design.

See [[tauri-agent-harness]] and [[tauri-agent-macos-helper]] for the MCP and native halves.
