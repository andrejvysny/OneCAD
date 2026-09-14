# tauri-agent

An MCP server that lets Claude Code operate the **real OneCAD Tauri window** the way a user does:
semantic snapshots find targets, OS-native input (a Swift CGEvent helper) acts on them, native
window capture shows the result, and the app's logs explain it. macOS only; Windows and Linux
adapters are seams that throw `UNSUPPORTED_PLATFORM_CAPABILITY`.

Targets are found two ways. **WebDriver** reads the WebView; **accessibility (AX)** reads what the
WebView cannot — `NSOpenPanel`/`NSSavePanel`, the app menu, native context menus, sheets, title-bar
buttons — which matters because OneCAD's Save and Open really are native panels. Every *acceptance*
interaction is a real CGEvent.

## Two interaction policies

`session_start {interaction: "foreground" | "background"}`, default `"foreground"`.

**foreground** is the harness as described above and is unchanged: the app is activated before each
action, calibration runs a real hover probe and real wheel notches, and input is a real CGEvent. It
is the only policy that proves a user could do the thing — and it takes the pointer and the
frontmost application while it runs.

**background** never moves the cursor, never activates the app and never posts an OS event, so you
can keep working while it drives OneCAD behind your windows. It dispatches pointer, wheel and key
events inside the page, actuates native chrome through accessibility (`native_press`,
`native_set_value`, `native_menu_invoke`), and still screenshots the window, which
`screencapture -l <windowId>` can do while the window is occluded.

The promise is enforced at the seam, not by conditionals: a background session holds a
`PlatformAdapter` whose input verbs and `focus` refuse (`src/platform/refusing.ts`). It never
escalates silently — a verb it cannot perform returns `BACKGROUND_CAPABILITY_UNAVAILABLE` naming the
ones it can.

What background cannot do, and refuses rather than fakes: **press-and-hold**, so no drag, orbit, pan
or sketch gesture. OneCAD captures the pointer on `pointerdown` and a synthetic pointer id cannot be
captured, so the gesture would report delivered and move nothing. It also cannot prove CSS `:hover`,
wheel device classification, or that a keyboard chord reaches the native menu — each of those limits
is stated in the envelope of the call that hits it. Launching still activates the app once (a
Tauri/tao behaviour no flag avoids); `mode:"attach"` avoids even that.

## Run

- Registered for Claude Code by the repo's `.mcp.json` (`bun tools/tauri-agent/src/mcp/server.ts`,
  cwd = repo root). Project config: `tauri-agent.config.json`.
- The app must be built with the `tauri-e2e` Cargo feature; `session_start {mode:"launch",
  launch:"dev"}` runs `bun run tauri:agent` (= `tauri dev --features tauri-e2e --config
  src-tauri/tauri.agent.conf.json`) with `TAURI_WEBDRIVER_PORT=4445`, waits for the embedded
  WebDriver `/status`, connects, finds the native window, focuses it, and calibrates the CSS→screen
  mapping with a real hover probe before any input is allowed. A background session stops after the
  passive half of that: geometry, the window table, no focus and no probes.
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

**Enforced by CI** (the `tauri-agent` job, macos-14):
```
bunx tsc --noEmit -p tools/tauri-agent
cd tools/tauri-agent && TAURI_AGENT_REQUIRE_HELPER=1 bun test     # helper tests must run, not skip
```
`TAURI_AGENT_REQUIRE_HELPER=1` is load-bearing: without it the helper tests skip themselves when
`swiftc` is missing and the job goes green having proved nothing. A GitHub runner has no Accessibility
or Screen Recording grant, which is fine — only the input-posting helper verbs gate on
`AXIsProcessTrusted`, and every test that reaches one wraps it in the suite's own `tolerate()`.

**User-run, not in CI** — these launch the real app and move the real cursor, so they need both TCC
grants and a window server. Record them as owed in `TODO.md`:
```
bun tools/tauri-agent/scripts/smoke.ts          # live: launch, calibrate, screenshot, stop
bun tools/tauri-agent/scripts/smoke-actions.ts  # live: click, hover, key, scroll, orbit drag
bun tools/tauri-agent/scripts/phase0.ts         # live, through the MCP stdio boundary
```
Live scripts need ports 1420/4445 free. Artifacts land in `.tauri-agent/artifacts/<session>/`
(journal.jsonl, launcher.log, app-logs/dev.jsonl, PNGs).

## Evidence rules

Only `mode:"real_user"` + `backend:"cgevent"` journal entries prove a user step. `mode:"webview"`
(page-dispatched) and `mode:"accessibility"` (an AX action) are real interactions with the real app
and real Rust backend, but they are not physical input and must be reported as what they are.
`debug_*` tools are diagnostic and never evidence. `session_stop` verifies no `onecad`/`onecad-worker-` process this
session started survived — a process of this checkout that PREDATES the session is reported with
`preexisting: true` and is never killed.

Read `delivery`, not `status`, before re-sending anything: `retrySafe` is true only when no input was
posted, and a delivered action whose evidence failed comes back as `warning` with no `error` field.
A screenshot counts as evidence only when `screenshot.authoritative` is true.
