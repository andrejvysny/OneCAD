# tauri-agent — internal contracts (binding for every module)

Runtime: **bun** executes `src/**/*.ts` directly. Deps resolve from the repo-root `node_modules`
(`@modelcontextprotocol/sdk` 1.30, `zod` 3.25, `webdriverio` 9.29, `@wdio/tauri-service` 1.3).
No build step. ESM, `import ... from "./x.ts"` with explicit `.ts` extensions (bun resolves them).
`tsconfig.json`: strict, `moduleResolution: "bundler"`, `allowImportingTsExtensions`, `noEmit`,
`types: ["bun-types"]`, `lib: ["ES2023","DOM"]` (DOM needed for snapshot script typing).

## Stdout rule
The MCP stdio transport owns `process.stdout`. **Nothing else writes to stdout.** All logging via
`src/log.ts` (stderr). Every child process is spawned with `stdio: ["pipe"|"ignore", "pipe", "pipe"]`,
never `"inherit"`. `server.ts` installs a guard that rebinds `console.log` to stderr.

## Errors — `src/errors.ts`
```ts
export type ErrorCode =
  | "APP_NOT_RUNNING" | "WEBDRIVER_UNAVAILABLE" | "BRIDGE_WEDGED" | "WINDOW_NOT_FOUND"
  | "WINDOW_NOT_FOREGROUND" | "FOCUS_FAILED" | "ELEMENT_NOT_FOUND" | "ELEMENT_STALE"
  | "ELEMENT_MOVING" | "ELEMENT_OCCLUDED" | "POINT_OUTSIDE_WINDOW" | "CALIBRATION_FAILED"
  | "NATIVE_INPUT_PERMISSION_DENIED" | "SCREEN_CAPTURE_PERMISSION_DENIED" | "PORT_IN_USE"
  | "DEV_SERVER_PORT_IN_USE" | "ACTION_TIMEOUT" | "SETTLE_TIMEOUT" | "STOP_INCOMPLETE"
  | "UNSUPPORTED_PLATFORM_CAPABILITY" | "INVALID_TARGET" | "HELPER_FAILED"
  | "APP_IDENTITY_MISMATCH" | "INTERNAL";

export class AgentError extends Error {
  readonly code: ErrorCode;
  readonly remediation: string;
  readonly details?: Record<string, unknown>;
  constructor(code: ErrorCode, message: string, opts?: { remediation?: string; details?: Record<string, unknown>; cause?: unknown });
}
export function isAgentError(e: unknown): e is AgentError;
export const REMEDIATION: Record<ErrorCode, string>; // default remediation text per code
```

## Log — `src/log.ts`
```ts
export const log: { debug(msg: string, data?: object): void; info(...): void; warn(...): void; error(...): void };
// level from env TAURI_AGENT_LOG (debug|info|warn|error, default info). Writes JSON lines to stderr.
```

## Config — `src/session/config.ts`
```ts
export interface AgentConfig {
  app: { bundleId: string; windowLabel: string; title: string; processPatterns: string[] };
  launch: { dev: { command: string[]; readyTimeoutMs: number }; bundled: { appPath: string } };
  webdriver: { port: number };
  devServer: { port: number };
  artifacts: { dir: string };
  screenshots: { policy: "never" | "on_failure" | "state_changing" | "always"; previewMaxPx: number };
  settle: {
    frameMs: number; quietMs: number; timeoutMs: number;
    // Which members of the idle tuple take part in the quiet window. `rev` is deliberately NOT
    // switchable: the -1 guard that catches an uninstrumented page hangs off it.
    signals: { regenBusy: boolean; geometryPending: boolean; documentRevision: boolean; frames: boolean };
  };
  calibration: { probeTestId: string };
  input: { wheelLinesPerNotch: number; dwellMs: number; clickIntervalMs: number };
  logs: { devJsonl: string | null };
  nativeOcclusion: { rects: Rect[] };   // geometry/types Rect — {x,y,width,height}, css px
}
export interface ResolvedConfig { root: string; config: AgentConfig; configPath: string }
export function loadConfig(): ResolvedConfig; // TAURI_AGENT_ROOT → walk up cwd for tauri-agent.config.json → walk up from import.meta.dir to .git
```
All paths inside `AgentConfig` are relative to `root`.

## Geometry types — `src/geometry/types.ts`
```ts
export interface Pt { x: number; y: number }
export interface Rect { x: number; y: number; width: number; height: number }   // css px, viewport-relative
export interface WindowGeom {
  innerPositionPx: Pt;   // tauri innerPosition() (physical px)
  innerSizePx: { width: number; height: number };
  scaleFactor: number;
  nativeBoundsPt: Rect;  // from CGWindowList (global points, top-left origin)
  windowId: number;      // CGWindowID
}
export type Space = "webview" | "window" | "global";
```

## Platform adapter — `src/platform/adapter.ts`
```ts
export type MouseButton = "left" | "right" | "middle";
export type ModKey = "Command" | "Control" | "Option" | "Shift" | "Fn";
export interface NativeWindowInfo { windowId: number; layer: number /* CGWindowLevel; 0 = normal */; bounds: Rect /* global pts */; name?: string; onscreen: boolean }
export interface Permissions { accessibility: boolean; screenRecording: boolean }

export interface NativeInput {
  move(p: Pt, opts?: { durationMs?: number; steps?: number; mods?: ModKey[] }): Promise<void>;
  down(button: MouseButton, p: Pt, opts?: { clickState?: 1|2|3; mods?: ModKey[] }): Promise<void>;
  up(button: MouseButton, p: Pt, opts?: { clickState?: 1|2|3; mods?: ModKey[] }): Promise<void>;
  click(button: MouseButton, p: Pt, opts?: { count?: 1|2|3; intervalMs?: number; mods?: ModKey[] }): Promise<void>;
  path(button: MouseButton, points: Pt[], opts?: { durationMs?: number; holdMs?: number; dwellMs?: number; mods?: ModKey[] }): Promise<void>;
  scroll(p: Pt, delta: { dy: number; dx?: number }, opts?: { mods?: ModKey[] }): Promise<void>;   // line units; positive dy = wheel up
  keyDown(key: string, opts?: { mods?: ModKey[] }): Promise<void>;   // key: "a".."z","0".."9","Enter","Escape","Tab","Space","Backspace","Delete","ArrowUp"..., "F1".., or a ModKey
  keyUp(key: string, opts?: { mods?: ModKey[] }): Promise<void>;
  press(key: string, mods?: ModKey[]): Promise<void>;
  type(text: string, opts?: { perCharMs?: number }): Promise<void>;
  releaseAll(): Promise<void>;   // never throws
  cursor(): Promise<Pt>;
  permissions(opts?: { prompt?: boolean }): Promise<Permissions>;
  dispose(): Promise<void>;
}
export interface NativeWindows {
  list(pid: number): Promise<NativeWindowInfo[]>;   // every on-screen window of pid, each with its layer
  isFrontmost(pid: number): Promise<boolean>;       // read-only; activates nothing
  focus(pid: number): Promise<boolean>;             // activates, then reports what is ACTUALLY frontmost
}
export interface CaptureResult {
  width: number; height: number; pixelScale: number; warning?: string;
  authoritative?: boolean; reason?: string;   // false + reason: produced, but not trustworthy evidence
}
export interface NativeCapture {
  window(windowId: number, outPath: string, boundsPt?: Rect): Promise<CaptureResult>;
  region(rect: Rect /* global pts */, outPath: string): Promise<CaptureResult>;
  screen(outPath: string, boundsPt?: Rect): Promise<CaptureResult>;
  preview(srcPath: string, outPath: string, maxPx: number): Promise<void>;   // sips -Z
}
export interface PlatformAdapter {
  input: NativeInput; windows: NativeWindows; capture: NativeCapture;
  ax: NativeAccessibility;               // macOS accessibility READS; actuation stays CGEvent
  name: "macos" | "windows" | "linux";
}
export function createPlatformAdapter(): PlatformAdapter;   // by process.platform; non-mac throws UNSUPPORTED_PLATFORM_CAPABILITY
```
All coordinates handed to the adapter are **global display points** (top-left origin, y down).

### Two point gates, and why there must be two
`checkPoint(css, geom, occlusion)` maps a CSS point through the calibrated MAIN window and refuses
anything outside it. That is right for a webview target and wrong for a native one: an `NSSavePanel`
legitimately extends past the main window, so under that gate it is unreachable by construction.

`checkGlobalPoint` takes a point already in global points and admits it when it falls inside ANY
on-screen window of this app's pid — ownership being the OS's answer via CGWindowList-by-pid, never
inferred from bounds or titles. The windows are re-read on every resolve, because a panel that opened
since the last snapshot is exactly the window a native target is most likely to be in. List order is
used here as ORDER — the first containing window is the one that would receive the click — and never as
identity, so the rule above still holds.

Its occlusion parameter is an **anchored** `GlobalOcclusion = {geom, rects}`, applied only when the
point landed in the window whose `windowId` matches `geom.windowId`. `nativeOcclusion.rects` are CSS
pixels relative to the main window's content box, so evaluating them against a point in a different
window would silently reinterpret the traffic-light rect as the top-left of the display. The AX
resolver passes no occlusion by default — those native controls are a legitimate target, and refusing
them as "occluders" would make the close button unreachable.

`checkPoint` is unchanged; the webview path keeps the strict gate.

**Calibration posts no state-changing input.** The wheel probe's page listener is registered on
`window` in the CAPTURE phase with `passive:false` and swallows its own notch with `preventDefault()` +
`stopImmediatePropagation()` before the viewport's own bubble-phase handler can see it, so the probe
measures the app's wheel classification without moving the camera. Three independent mechanisms disarm
it (the matching notch, an explicit disarm in a `finally`, and an in-page timeout that fires even if
the bridge dies), and a wheel event more than a few pixels from the probe point passes through
untouched. **No calibration path reaches `input.move` or `input.scroll` without passing
`checkPoint`** — the hover step-off searches candidate offsets for one that does. A probe that still
posted a notch reports `posted: true` and is journalled, because an unattributed CGEvent is what makes
a later action's journal unreadable.

**Window list order is not identity.** `list()` returns CGWindowList order — front to back at the
instant of the call — and it changes when a panel opens or the window is raised. Pick a window by
area, `name` and `layer` (0 is a normal window; higher layers are panels, menus and tooltips), never
by index, and re-resolve rather than remembering a position. An empty list is WINDOW_NOT_FOUND.

**Frontmost before input.** Native input goes to whatever application is in front, so every verb
that posts an event is gated on `isFrontmost(pid)` (WINDOW_NOT_FOREGROUND when false). `focus()` is
the action that fixes that; `isFrontmost()` is the check, and it never activates anything.

**Capture `warning`.** One image of a known point-size area has ONE scale, so a width ratio that
disagrees with the height ratio by more than 5% means a different window or display was
photographed (`screencapture -l` has answered with another window of the same pid). The capture is
still returned — with `warning` set, which callers surface in `ActionResult.warnings`, and the image is
then NOT `authoritative`.

**Capture capability.** `SessionStatus.capture` is `{available, authoritative, reason?}`, set at
preflight. When capture is unavailable an **automatic** (policy-driven) screenshot is skipped with a
mandatory warning rather than attempted, while an **explicit** `ui_screenshot` or `screenshot:true`
still refuses with `SCREEN_CAPTURE_PERMISSION_DENIED` — raised BEFORE any input is posted, so the call
stays retry-safe. That asymmetry is deliberate: a caller who asked for a picture must be told it cannot
have one; policy evidence is best-effort.

`screenshot.authoritative` is true only when the session capability AND the individual capture both
hold up. `mode:"screen"` is never authoritative — it photographs the whole display with no bounds to
check against, so the ratio test that backs the flag everywhere else simply does not run. A capture
that looks degraded re-reads the LIVE Screen Recording grant rather than trusting a preflight reading,
and reports three distinct outcomes: genuinely denied, granted-but-something-else-broke (which must not
send the user to System Settings), and unanswerable.

### Native input bounds
Every duration, interval, hold, dwell and per-character delay is bounded to **[0, 60000] ms**,
`steps` to **[1, 2000]**, `count`/`clickState` to **[1, 3]**, and scroll `dy`/`dx` to
**[-100, 100]** lines. The adapter rejects out-of-range values with INVALID_TARGET; the helper
clamps independently, because a trapping numeric conversion would kill it mid-drag (exit 133).

### Helper protocol — `src/platform/macos/helper/main.swift` (version 1.3.0, protocol 4)
One JSON request per stdin line, one JSON reply per stdout line, ids correlate them; stderr carries
diagnostics only.

Input and system verbs: `move`, `down`, `up`, `click`, `path`, `scroll`, `keydown`, `keyup`,
`press`, `type`, `release_all`, `cursor`, `permissions`, `layout`, `windows`, `focus`, `frontmost`,
`version`.

Accessibility verbs, all gated on `AXIsProcessTrusted()`. Seven READ: `ax_windows`, `ax_snapshot`,
`ax_find`, `ax_point`, `ax_focused_window`, `ax_modal`, `ax_menu`. Three ACT: `ax_press`,
`ax_set_value`, `ax_menu_press`. Every reply **on the helper wire** carries `actuated: boolean`, so
anything reading the protocol directly never has to infer read-versus-act from the verb name. At the
MCP boundary the distinction is carried by `mode` instead (`"accessibility"` exactly for the three),
and the TypeScript result types do not re-expose the flag. They exist because the WebDriver bridge is blind to everything
the webview does not own — `NSOpenPanel`/`NSSavePanel`, the app menu, native context menus, sheets,
title-bar buttons — and OneCAD's Save and Open really do go through native panels.

**CGEvent is the only acceptance-grade actuator.** This supersedes the older absolute
("AX reads, never AX acts"), which the three actuation verbs amend. An AX action asks the control to
perform itself: no cursor moves, no key is pressed, and the application need not be in front. That is
what makes it the only actuation an `interaction:"background"` session has, and it is why every one
of them is labelled `mode:"accessibility"` / `backend:"ax"` and **never closes a real-user acceptance
claim** — exactly the standing `mode:"webview"` already has. In a foreground session,
`pointer_click {axRef}` with a real mouse remains the stronger evidence and the default.

Each actuation verb refuses rather than reporting a success the application did not give it:
`ax_press` refuses an action the element does not advertise (`AXUIElementCopyActionNames`);
`ax_set_value` refuses when `AXUIElementIsAttributeSettable` says no, and reports the value read
BACK so a field that normalised the write is visible; `ax_menu_press` refuses a title path that
matches no item or more than one, listing the paths that do exist.

AX refs are generation-scoped `@a<gen>e<n>`, mirroring the webview snapshot, so a ref from an earlier
walk is absent from the map rather than aliased onto whichever element now occupies that slot.
`ax_find` bumps the generation too. `ax_point` re-validates and has five distinguishable refusals
(stale / not found / outside window / moving / invalid args).

AX coordinates are **already global display points, top-left origin, y down** — measured
byte-identical to `kCGWindowBounds`, not assumed — so an AX rect goes straight to `click`.

A window's `CGWindowID` comes from the private `_AXUIElementGetWindow`, resolved at runtime with
`dlsym` so a future macOS that drops it degrades instead of failing to link. The bounds+title fallback
genuinely cannot disambiguate two same-size windows, so it reports `"ambiguous"` and the id surfaces as
a discriminated union whose unknown case has **no `id` member at all** — a caller cannot read an id
without narrowing.
Failure replies carry `code`: `INVALID_ARGS`/`INVALID_VERB` map to INVALID_TARGET,
`NATIVE_INPUT_PERMISSION_DENIED`, `WINDOW_NOT_FOUND` and `FOCUS_FAILED` map to themselves, anything
else to INTERNAL.

`release_all` takes `{"osState": bool, "force": [String]}`, both optional. By default it releases only
what the helper itself is holding.

`force` is the crash path and the ONLY one used automatically: the client names exactly the buttons,
modifiers and ordinary keys **it** may have pressed, and the helper releases a named input only when
`CGEventSource` confirms it is actually down — so an over-wide guess is a no-op rather than an event
fired at the user. The reply reports `releasedButtons` / `releasedMods` / `releasedKeys`.

`osState: true` releases everything the OS reports held, the user's own hand and modifiers included.
**No automatic caller uses it any more**; it is a manual escape hatch for a human unwedging a machine.

The host keeps a shadow `potentialHeld` set covering buttons, modifiers **and ordinary keys** — a
`keydown "a"` that never gets its `keyup` leaves that key down and auto-repeating, and it is nameable
only from there. A name is latched BEFORE the write that could press it and cleared only by a reply
that proves the release landed. A drain that fails re-arms the obligation rather than discarding it: a
single failed respawn must not become the last attempt ever made.

`HelperClient` serialises verbs in a FIFO and starts a verb's timeout when it reaches the head (the
helper answers one at a time, so queue time is not the verb's fault). A timeout or an unexpected
exit restarts the helper and drains with `osState: true` before anything else runs; `dispose()`
releases tracked input, then closes stdin and waits briefly so the helper's own EOF release runs
before SIGTERM.

## Journal — `src/trace/journal.ts`
```ts
export interface JournalEntry { ts: string; sessionId: string; actionId: string; tool: string; input: unknown; result: unknown; durationMs: number }
export class Journal {
  constructor(dir: string /* artifacts/<session> */);
  nextActionId(): string;                    // "A-001", "A-002", ...
  append(entry: JournalEntry): Promise<void>; // journal.jsonl
  since(actionId?: string, limit?: number): Promise<JournalEntry[]>;
  artifactPath(name: string): string;        // dir/name
}
```

## Action envelope — `src/mcp/envelope.ts`
```ts
export type Mode = "real_user" | "webview" | "accessibility" | "diagnostic";
export type InteractionPolicy = "foreground" | "background";
export interface ActionResult {
  actionId: string; status: "ok" | "warning" | "error"; mode: Mode;
  backend: "cgevent" | "webdriver" | "ax" | "js" | "none"; windowId?: number;
  target?: { ref?: string; role?: string; name?: string; testId?: string };
  resolvedPoint?: { global: Pt; css: Pt };
  state?: { beforeRevision: number; afterRevision: number; settled: boolean };
  effects?: { consoleErrors: number; logErrors: number; newWindows: number; windowMoved: boolean };
  screenshot?: { path: string; previewPath: string; width: number; height: number; pixelScale: number;
                 captureMode: string; authoritative: boolean };
  warnings: string[];
  error?: { code: ErrorCode; message: string; remediation: string; details?: Record<string, unknown> };
  delivery: Delivery;                            // REQUIRED — read this, never `status`, before re-sending
  interaction: Interaction;                      // REQUIRED — what this call did to the user's desktop
  fidelity?: "text_entry" | "physical_key";      // keyboard tools only
  timingsMs: { resolve: number; input: number; settle: number; capture: number };
  data?: unknown;   // tool-specific payload (snapshot text, log lines, ...)
}

export type DeliveryPhase = "not_started" | "input_started" | "input_completed" | "postconditions_completed";
export interface Delivery {
  phase: DeliveryPhase;
  inputStarted: boolean;
  inputCompleted: boolean;
  mayHaveSideEffects: boolean;
  retrySafe: boolean;          // true EXACTLY at phase "not_started", never otherwise
  evidenceIncomplete?: boolean;
}

export interface Interaction {
  policy: InteractionPolicy;
  foregroundChanged: boolean;  // this call activated the app or raised a window
  cursorMoved: boolean;        // this call moved the global cursor
}
```

### Retry is decided by `delivery`, never by `status`
`status` alone cannot separate "the click was never sent" from "the click landed and the screenshot
failed", and re-sending the second double-applies it. So:

- A failure **before or during** the input is `status:"error"` with an `error` field.
- A failure **after** the input completed is `status:"warning"` with **no `error` field** — the cause
  goes to `warnings` and `data.postconditionError`, and `delivery.evidenceIncomplete` is set. The
  `error` field means "the action did not happen"; populating it after a delivered input is exactly
  what makes a caller re-send a Save.
- `deliveryFor()` in `envelope.ts` is the single derivation site. `okResult` defaults to
  `not_started`/`retrySafe:true`, which is correct for a query tool — so any tool that posts input or
  changes process state MUST pass its own `delivery` (`session_start`, `session_stop` and
  `window_focus` do).
- MCP `isError` is `false` for a delivered-but-evidence-failed action. A client keying retry off
  `isError` rather than `delivery` will behave wrongly.

### `fidelity` — two honest levels, not one
`keyboard_type_text` is `"text_entry"`: it posts `virtualKey: 0` plus a Unicode string, so a consumer
reading `event.code` sees `"KeyA"` whatever character was typed. Correct for filling a field, wrong for
a `code`-sensitive handler. `keyboard_press` / `keyboard_shortcut` / `keyboard_down` / `keyboard_up` are
`"physical_key"`: real modifier down/up events around a real key code, which native menu key
equivalents need.

**The key map is positional US/ANSI (`kVK_ANSI_*`) with no layout translation.** On a non-US layout
`keyboard_press` sends whatever key sits at that ANSI position. `session_status.input.keyboardLayout`
reports the active input source and `session_start` warns when it is not ANSI/US. Measured on the
development machine: under `com.apple.keylayout.Slovak`, `kVK_ANSI_Z` produces `y` and `kVK_ANSI_Y`
produces `z` — and OneCAD binds ⌘Z to undo and ⌘Y to redo, so those two are swapped there.

## Tool registration — `src/mcp/defineTool.ts`
```ts
export interface ToolCtx { session: SessionOrchestrator; journal: Journal; config: ResolvedConfig }
export function defineTool<S extends z.ZodTypeAny>(server: McpServer, def: {
  name: string; description: string; input: S; kind: "action" | "query" | "diagnostic";
  handler: (args: z.infer<S>, ctx: ToolCtx, actionId: string) => Promise<ActionResult>;
}): void;
// wraps: actionId alloc → handler → journal → MCP content [{type:"text", text: JSON.stringify(result)}] + image content when result.screenshot?.previewPath exists and inline requested. Catches AgentError → status "error" envelope; other errors → INTERNAL.
```

## Semantic layer — `src/semantic/*`
```ts
// webdriver.ts
export class Bridge {
  static connect(port: number): Promise<Bridge>;              // webdriverio remote(); sets the session script timeout to SNAPSHOT_BUDGET_MS, then waits window.wdioTauri?.waitForInit if present ("absent" returns at once)
  execute<T>(name: string, fn: (...a: any[]) => T, args: unknown[], opts: { readOnly: boolean; budgetMs?: number }): Promise<T>; // named budget; readOnly retries once; 2 consecutive budget failures → BRIDGE_WEDGED
  invoke<T>(command: string, args?: Record<string, unknown>, opts?: { readOnly?: boolean }): Promise<T>; // window.__TAURI__.core.invoke
  windowGeom(): Promise<{ innerPositionPx: Pt; innerSizePx: {width:number;height:number}; scaleFactor: number; focused: boolean; dpr: number; vvScale: number; innerWidth: number; innerHeight: number }>;
  // validateGeom (exported): blanks ⇒ WEBDRIVER_UNAVAILABLE; vvScale !== 1, |dpr - scaleFactor| > 0.01,
  // or |innerSizePx/scaleFactor - inner*| > 1 px ⇒ CALIBRATION_FAILED. The embedded driver DOES await an
  // async script (tauri-plugin-wdio-webdriver executor.rs wraps it in an async IIFE), so a blank reading
  // is the window API answering with nothing, never a dropped promise.
  // pollPluginInit(deps) (exported): "ready" | "absent" | "timeout"; injected probe/sleep/now for tests.
  close(): Promise<void>;
  readonly wedged: boolean;
}
// snapshot.ts
export type SnapshotMode = "interactive" | "accessibility" | "dom" | "diff";
export interface SnapNode { ref: string; fp: string; role: string; name: string; testId?: string; rect: Rect; depth: number; state: Record<string, string | boolean | number>; css: string; dragRegion: boolean; disabled: boolean }
export interface Snapshot { generation: number; revision: number; title: string; viewport: { width: number; height: number }; nodes: SnapNode[]; text: string; truncated: boolean }
// `ref` is "@s<generation>e<n>": every snapshot bumps the page's generation counter, so a ref from an
// older snapshot is absent from the current ref map ⇒ ELEMENT_STALE, never a collision with a new node.
// A LANDMARK role (toolbar/main/dialog/region/navigation/group/status) is never named from its own
// textContent, in the name ladder or the fingerprint: a live readout child must not re-fingerprint its
// container. aria-label / aria-labelledby / label / title still name one.
export function takeSnapshot(bridge: Bridge, opts: { mode: SnapshotMode; rootCss?: string; maxNodes?: number; maxChars?: number }): Promise<Snapshot>;
export function diffSnapshots(a: Snapshot, b: Snapshot): { added: SnapNode[]; removed: SnapNode[]; changed: Array<{ before: SnapNode; after: SnapNode }> };
export function readRevision(bridge: Bridge): Promise<{ rev: number; lastMutationAt: number; now: number }>;
// resolve.ts
export type Target = { ref: string } | { testId: string } | { role: string; name?: string } | { text: string } | { css: string } | { point: Pt & { space: Space } };
export interface Resolved { node?: SnapNode; css: Pt; rect?: Rect; source: "ref"|"role"|"testId"|"text"|"css"|"point"; dragRegion?: boolean }
export class Resolver {
  constructor(bridge: Bridge, refs: RefStore /* last snapshot nodes by ref */);
  resolve(target: Target, opts?: { offset?: Pt; forInput?: boolean; forDrag?: boolean; geom?: WindowGeom }): Promise<Resolved>;
  // forInput OR forDrag runs the checks (checkScript.ts): re-find + fingerprint + 2 rect samples 50 ms
  // apart + elementFromPoint + dragRegion. Ladder: ELEMENT_STALE → ELEMENT_MOVING → ELEMENT_OCCLUDED.
  // A `point` target has no element, so it runs ONE probe instead — elementFromPoint(x,y).closest(
  // "[data-tauri-drag-region]") — which refuses a drag (ELEMENT_OCCLUDED, reason "tauri-drag-region")
  // and flags a click (`dragRegion: true`).
  // A held `ref` NEVER falls back to a selector: detached ⇒ ELEMENT_STALE. `node.css` is positional, so
  // re-finding by it addresses whichever sibling slid into the slot and the positional fingerprint agrees.
  // role/testId/text may fall back only through a nominal identity ([data-testid=…] or tag#id); css uses
  // its own selector. Non-finite rect components refuse before any point is computed.
}
```
## Page instrumentation and settling
`window.__tauriAgentRev` and `window.__tauriAgentConsole` are installed by
`installInstrumentation()` (`semantic/instrumentScript.ts`) at **every bridge construction** — session
start, bridge rebuild, app reconnect — not as a side effect of the first `ui_snapshot`. Nothing in the
MCP API requires a snapshot first, and without the probes `settle` compared `-1` to `-1` and reported a
clean quiet having watched nothing. `settle` now warns rather than settling when it sees `-1`.

The installers are duplicated between `instrumentScript.ts` and `snapshotInPage` on purpose: page
scripts are serialized with `Function.prototype.toString` and cannot share a helper. A test asserts
both produce identical globals.

**Settling is CAD- and WebGL-aware, not DOM-only.** A viewport orbit changes camera matrices and
repaints WebGL with ZERO DOM mutations; an Extrude runs frontend → Rust → OCCT worker → mesh → Three.js
upload. So the quiet window is over the whole tuple `(rev, regenBusy, geometryPending,
documentRevision, frames)` AND requires `regenBusy === 0` AND `geometryPending === false`. Every member
but `rev` is nullable; **a null means the signal is unavailable in this build and always produces a
warning** — silent degradation to DOM-only is the defect being fixed, not a fallback. A settle timeout
remains a warning, never fatal, and names which member was still moving.

Signals come from `window.__stores` (DEV), `window.__vpEngine` (`?vpdebug`, supplied by the agent
lane's `devUrl`) and the feature-gated `agent_status` command. `installInstrumentation` removes the
`?vpdebug` origin pill through public API so debug chrome never reaches a screenshot.

Snapshot script runtime globals (installed idempotently by the snapshot script):
`window.__tauriAgentRev = { rev, lastMutationAt }`, `window.__tauriAgentConsole = { entries: [...], errors: n }`,
`window.__tauriAgentRefs: Map<string, Element>` (current generation only, keyed by the full ref),
`window.__tauriAgentSnapGen: number` (bumped by every snapshot), `window.__tauriAgentFp(el)`.
`takeSnapshot`'s script is marked `readOnly: true` even though it writes those globals: read-only means
"safe to re-send" — it touches no application state, and a retry just issues a new ref generation.

## Geometry — `src/geometry/mapping.ts`
```ts
export function cssToGlobal(css: Pt, g: WindowGeom): Pt;      // inner/sf + css
export function globalToCss(global: Pt, g: WindowGeom): Pt;
export function windowToGlobal(win: Pt, g: WindowGeom): Pt;   // same as css for Overlay titlebar (content == window)
export function pointInWindow(global: Pt, g: WindowGeom): boolean;
export function pointInNativeOcclusion(css: Pt, rects: Rect[]): boolean;
export function checkGlobalPoint(global: Pt, owned: OwnedWindow[], occlusion?: GlobalOcclusion): Pt;
export function rectIsFinite(r: Rect | null | undefined): r is Rect;   // JSON carries NaN/Infinity as null
export function rectStable(a: Rect, b: Rect, tolPx?: number): boolean;  // false unless all 8 numbers are finite
export function rectCenter(r: Rect, offset?: Pt): Pt;
// wheelClass.ts — mirror of src/viewport/engine/navInput.ts evidence rules
export function classifyWheel(sample: { deltaMode: number; deltaX: number; deltaY: number }, prior: "mouse"|"trackpad"): "mouse"|"trackpad"|"unknown";
// A probe notch always OPENS a navInput segment, which navInput weighs with an infinite
// gap: `WheelSample.gapMs` is recorded by the page probe and deliberately ignored here.
// tests/wheelClass.test.ts imports navInput.ts and compares verdicts over a grid.
```

## Target pool refresh — `src/mcp/tools/targetPool.ts`
`session.refs` is the pool from the last `ui_snapshot`. `resolveWithRefresh(session, bridge, target, opts)` is the
one entry point every tool uses: a miss on a role/name, testId or text target takes a fresh FULL interactive
snapshot, replaces the pool and retries once, adding `POOL_REFRESHED_WARNING` to the envelope. Refs, css and
points are never retried. A scoped `ui_snapshot {root}` merges its nodes into the pool instead of replacing it.

## Window identity — which window an event actually reaches
```ts
export interface WindowIdentity {
  label: string; title: string | null;
  windowId?: number;        // absent EXACTLY when `unresolved` is set
  bounds?: Rect; source: string; key: boolean; modal: boolean;
  unresolved?: string;
}
export interface WindowTable { at: string; pid: number; privateWindowIdApi: boolean; entries: WindowIdentity[] }
```
Built during calibration by correlating each Tauri window's label, title, `outerPosition`,
`innerSize` and `scaleFactor` against the AX window list, and it is the ONLY authority on which
native window a label names once it exists.

**The ordering is the point.** `pickLargestWindow` is bootstrap, used only while no table exists;
the table is identity; and an unresolved label falls through to neither — it refuses. The cost of
being wrong is a real CGEvent posted into a window the caller did not name.

A label is unresolved when no AX window matches its origin, when several do and neither title nor
size separates them, when the helper cannot name a `CGWindowID`, or when two labels claim one id —
in which case **both** become unresolved, because nothing can say which correlation was wrong. The
one deliberate exception is a single Tauri window and a single layer-0 native window: counting
resolves that even when AX cannot, since there is nothing to confuse it with.

**The input gate is about the window, not the app.** `assertFrontmost` / `assertFrontmostOrFocus`
verify the application is frontmost AND that the target window is key AND that no modal of this app
covers it — unless the target IS the modal, which is exactly what a native Save panel presents.
`WINDOW_NOT_FOREGROUND` names which condition failed (`app-not-frontmost`, `target-unresolved`,
`modal-covering-target`, `modal-not-key`, `no-modal-up`), because "another window is key" and "a
modal is covering you" are different problems with different fixes. `key` and `modal` in the table
are build-time facts; the gate re-reads them live and never trusts the stored ones.

## Interaction policy — how much of the desktop a session may touch

```ts
export type InteractionPolicy = "foreground" | "background";
// session_start { interaction?: InteractionPolicy }   default: config `interaction.default`, itself "foreground"
```

`"foreground"` is the original harness and is unchanged in every respect: the app is activated
before each action, calibration runs both probes, and native CGEvents are posted. It is the only
policy that produces real-user evidence, and it takes the pointer and the frontmost application to
do it.

`"background"` promises three things: **no cursor movement, no activation, no OS event.**

### The promise is structural, not a conditional
A background session is handed a `PlatformAdapter` whose input verbs and whose `windows.focus`
refuse — `src/platform/refusing.ts`. No code path, however new, can post a `CGEvent` or activate the
app through it. What passes through untouched: `permissions`, `layout`, `cursor` (reads the pointer,
never moves it), `dispose`, `windows.list`, `windows.isFrontmost`, every AX verb, and the whole
capture path — `screencapture -x -o -l <windowId>` photographs a specific window whether or not it is
in front. `releaseAll` is the one input verb that passes through: it releases only what the helper
itself pressed, and can therefore only REDUCE held state. It is also the single exception to "no OS
event" — the helper process is shared across sessions, so an earlier foreground session can have
left a key latched and clearing it posts a real key-up. `keyboard_release_all` says so in a warning
when the session is background, rather than leaving a reader to discover it.

### No silent escalation
A verb that cannot be performed in background returns `BACKGROUND_CAPABILITY_UNAVAILABLE` naming
`details.supportedInBackground`. It never falls back to `cgevent`. An explicit `mode:"real_user"` in
a background session is refused before any input, so `delivery.retrySafe` is `true`.

### What the background lane covers
`pointer_move`, `pointer_hover`, `pointer_click`, `pointer_scroll`, `keyboard_press`,
`keyboard_shortcut`, `keyboard_down`, `keyboard_up`, `keyboard_type_text` — dispatched in the page
(`mode:"webview"`, `backend:"webdriver"`) — plus `native_press`, `native_set_value` and
`native_menu_invoke` (`mode:"accessibility"`, `backend:"ax"`) for native chrome.

It does **not** use the embedded driver's W3C `/actions` endpoint, which is implemented by injecting
`new MouseEvent(...)` (`tauri-plugin-wdio-webdriver` `executor.rs:1391`). OneCAD's viewport, picker
and sketch controller all bind POINTER events, so a MouseEvent reaches none of them; the harness's
own `PointerEvent` dispatch does.

### What it refuses, and exactly why
`pointer_down`, `pointer_up`, `pointer_drag`, `pointer_drag_path`. `CadOrbitControls.onPointerDown`
calls `el.setPointerCapture(e.pointerId)` with no guard; a synthetic pointer id has no active pointer
to capture, so that call throws inside the app's own handler and it aborts before recording the drag.
Orbit, pan and every sketch drag would report delivered and move nothing. No page patching is done to
hide this.

### Stated fidelity limits, carried in the envelope rather than in a doc
- **Hover**: JavaScript hover handlers run; CSS `:hover` does not, because the engine applies it from
  the real pointer position. A screenshot will not show a hover-styled control.
- **Wheel**: the app's wheel-versus-trackpad device scoring and the calibrated lines-per-notch are
  both bypassed. Direction and magnitude are exercised; wheel fidelity is not.
- **Keyboard**: reaches the app's JavaScript keydown lane, not the macOS `NSMenu` accelerator, which
  claims a real chord first. For Save the two converge on the same action; they are different code.
- **Calibration**: the hover probe is what verifies the CSS→global mapping, and a background session
  skips it, so `calibration.hoverProbe` is `{skipped}` and any global point it reports is computed
  but unproven. Nothing in this policy posts a global point — the page lane dispatches at
  `clientX/clientY`, and AX actuation addresses an `AXUIElement`.

### Launching still activates the app, once
tao's `window_activation_hack` calls `makeKeyAndOrderFront` on every visible window at
`applicationDidFinishLaunching`, and `activate_ignoring_other_apps` defaults to true. No Tauri config
flag avoids it: `focus:false` only changes the initial `makeKeyAndOrderFront` to `orderFront`, which
the hack then undoes. A background launch therefore reports
`session_status.foregroundStolenAtLaunch: true`, and every action after it still reports
`interaction.foregroundChanged: false`. `mode:"attach"` avoids it entirely.

## Native (accessibility) tools
`native_snapshot`, `native_find`, `native_inspect`, `native_focused_window`, `native_modal`,
`native_menu_snapshot` are **queries** over the AX tree, and `{axRef}` is a target form the pointer
and keyboard verbs accept. For an `{axRef}` target `mode` stays `"real_user"` and `backend` stays
`"cgevent"` — only the way the point was found differs, which `ActionResult.surface` records as
`"webview"` or `"native"`.

`native_press`, `native_set_value` and `native_menu_invoke` ACT through accessibility instead, and
are labelled `mode:"accessibility"` / `backend:"ax"`. They are available in both policies:
`native_menu_invoke ["Edit","Undo"]` presses the menu item rather than sending a chord, which makes
it the layout-proof way to trigger a menu command on a machine where the positional key map sends the
wrong character. They do not run through `beginAction`/`finishAction`, because that pipeline's first
step is the frontmost gate and not needing the window in front is the whole point.

**Each refuses rather than reporting a press that did nothing.** Both gates on `ax_press` are load
bearing and the second is the one that bites: a DISABLED control still advertises `AXPress` (a
disabled Finder menu item advertises `AXCancel, AXPress, AXPick`), `AXUIElementPerformAction` still
answers `.success`, and nothing happens. The motivating case is an `NSSavePanel` whose Save button
stays disabled until the filename is non-empty — and `native_set_value` does not fire the
text-did-change notification, so that is exactly the state an agent can put it in. `ax_press` also refuses **window chrome** — the
full-screen, zoom, minimise and close buttons — unless the caller passes `acceptDisruption:"yes"`,
because pressing one moves the user's Space, resizes the window or takes it away, none of which a
background session may do silently. `ax_menu_press` requires at least two path segments, since a
lone menu-bar title is not a command and `AXPress` on one opens the menu over whatever the user is
looking at. It refuses a disabled item the same way, narrows AppKit alternate items by `AXEnabled`
before deciding a path is ambiguous (Finder's menu bar has 13 duplicate two-segment paths, normally with one enabled
alternate), ranks its candidate list by shared prefix with the request rather than alphabetically,
and tells a truncated walk apart from a genuinely missing item.

**What they do NOT produce: `state`, `effects`, `screenshot`, `windowId`, or non-zero `timingsMs`.**
They deliver input and say so in `delivery`, but nothing observes the consequence — so a caller
cannot read the result and conclude the app changed. Follow one with `native_modal`,
`native_snapshot`, `wait_for` or `observe_logs` for that. The AX-subtree settle described below
belongs to `{axRef}` targets in the pointer/keyboard pipeline, not to these three.

A native action **taken through the pointer or keyboard pipeline with an `{axRef}` target** settles
on an AX-subtree hash going quiet, not on the DOM revision: clicking Save in an `NSSavePanel` changes
nothing in the webview, so the idle tuple would report a settle having watched the wrong surface
entirely. The three accessibility actuation tools bypass that pipeline and therefore settle on
nothing — see above.

## Process ownership and teardown
```ts
export type ProcessOwnership = { kind: "launched"; pid: number } | { kind: "attached"; pid: number };
export interface StopOptions { killApp?: boolean; forceKillAttached?: boolean }
```
Ownership is decided once, where the pid is first resolved, from whether this session spawned the
launcher. **Only a LAUNCHED session may terminate the app.** `killApp` (default true) governs that case
alone; an attached app is the developer's and `forceKillAttached` is the single deliberate way to take
it down. A failed start signals nothing it did not launch.

`killApp:false` really detaches: the launcher process group is signalled only when the session may
kill, because the app lives IN that group and signalling it regardless killed the very app the flag
promised to spare. `#recover()` re-points ownership at whatever pid now serves the port and
**downgrades to "attached" when the launcher has exited** — otherwise an app the developer restarted
by hand inside the reconnect window would be adopted as ours and later killed.

The sweep identifies a process by executable path plus checkout, never by who started it, so the
session snapshots the checkout's already-running pids at preflight and never signals those. They are
still reported, flagged `preexisting: true`, and **do not count toward `STOP_INCOMPLETE`** — sparing
them is correct behaviour, not an incomplete teardown.

`session_stop` measures `{survivors, portsFree: {"4445": bool, "1420": bool}, launched, detached}` after
the sweep and returns it in the status (and in `STOP_INCOMPLETE.details`). Reports cite this, never an
assumption.

## App identity handshake
`session_start` invokes the feature-gated `agent_identity` command and refuses with
`APP_IDENTITY_MISMATCH` when `agentTesting !== true`, the reported pid disagrees, or
`cargoManifestDir` is not inside the project root. `cargoManifestDir` is the load-bearing field: `env!`
bakes it in at COMPILE time, so it proves which source tree produced the binary — something no runtime
path inspection can. Paths are compared through the real path of their deepest existing ancestor,
because `/var` is a symlink to `/private/var` on macOS and a lexical comparison refused perfectly good
symlinked checkouts.

An app built WITHOUT the command makes `invoke` reject with Tauri's "Command … not found"; that is a
recorded warning, not a refusal, so an older binary is not bricked. Every other rejection — including
WebDriver's own "unknown command", which is a DRIVER fault — is rethrown.
