# tauri-agent

An MCP server that lets Claude Code operate the **real OneCAD Tauri window** the way a user does:
semantic snapshots find targets, OS-native input (a Swift CGEvent helper) acts on them, native
window capture shows the result, and the app's logs explain it. This is spec Phase 0 + Phase 1 of
`tauri-agent-real-user-testing-specification.md`, macOS only; Windows and Linux adapters are seams
that throw `UNSUPPORTED_PLATFORM_CAPABILITY`.

## Run

- Registered for Claude Code by the repo's `.mcp.json` (`bun tools/tauri-agent/src/mcp/server.ts`,
  cwd = repo root). Project config: `tauri-agent.config.json`.
- The app must be built with the `tauri-e2e` Cargo feature; `session_start {mode:"launch",
  launch:"dev"}` runs `bun run tauri:agent` (= `tauri dev --features tauri-e2e --config
  src-tauri/tauri.agent.conf.json`) with `TAURI_WEBDRIVER_PORT=4445`, waits for the embedded
  WebDriver `/status`, connects, finds the native window, focuses it, and calibrates the CSS→screen
  mapping with a real hover probe before any input is allowed.
- Requires Accessibility and Screen Recording for the terminal app running Claude Code.
- Skills: global `~/.claude/skills/tauri-agent-test` (operating policy, tool guide) and the project
  overlay `.claude/skills/onecad-agent-test` (selectors, keymap, gestures, logs).

## Layout

`src/mcp` (server, tool registration, envelope, tool groups) · `src/session` (orchestrator, launch,
startup checks, calibration, teardown, process helpers) · `src/semantic` (WebDriver bridge, snapshot
script, resolver, in-page checks) · `src/geometry` (mapping, wheel classification mirror) ·
`src/platform/macos` (Swift helper, input client, windows, capture, build) · `src/observe` (dev.jsonl
tail) · `src/trace` (journal). Binding interfaces: `CONTRACTS.md`.

## Gates

```
bunx tsc --noEmit -p tools/tauri-agent
cd tools/tauri-agent && TAURI_AGENT_REQUIRE_HELPER=1 bun test     # helper tests must run, not skip
bun tools/tauri-agent/scripts/smoke.ts          # live: launch, calibrate, screenshot, stop
bun tools/tauri-agent/scripts/smoke-actions.ts  # live: click, hover, key, scroll, orbit drag
bun tools/tauri-agent/scripts/phase0.ts         # live, through the MCP stdio boundary (spec §37 exit)
```
Live scripts move the real mouse and keyboard and need ports 1420/4445 free. Artifacts land in
`.tauri-agent/artifacts/<session>/` (journal.jsonl, launcher.log, app-logs/dev.jsonl, PNGs).

## Evidence rules

Only `mode:"real_user"` + `backend:"cgevent"` journal entries prove a user step. `debug_*` tools are
diagnostic and never evidence. `session_stop` verifies no `onecad`/`onecad-worker-` process from this
checkout survived.
