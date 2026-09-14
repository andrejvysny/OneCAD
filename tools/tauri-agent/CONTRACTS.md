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
  | "UNSUPPORTED_PLATFORM_CAPABILITY" | "INVALID_TARGET" | "HELPER_FAILED" | "INTERNAL";

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
  settle: { frameMs: number; quietMs: number; timeoutMs: number };
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
export interface CaptureResult { width: number; height: number; pixelScale: number; warning?: string }
export interface NativeCapture {
  window(windowId: number, outPath: string, boundsPt?: Rect): Promise<CaptureResult>;
  region(rect: Rect /* global pts */, outPath: string): Promise<CaptureResult>;
  screen(outPath: string, boundsPt?: Rect): Promise<CaptureResult>;
  preview(srcPath: string, outPath: string, maxPx: number): Promise<void>;   // sips -Z
}
export interface PlatformAdapter { input: NativeInput; windows: NativeWindows; capture: NativeCapture; name: "macos" | "windows" | "linux" }
export function createPlatformAdapter(): PlatformAdapter;   // by process.platform; non-mac throws UNSUPPORTED_PLATFORM_CAPABILITY
```
All coordinates handed to the adapter are **global display points** (top-left origin, y down).

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
still returned — with `warning` set, which callers surface in `ActionResult.warnings`.

### Native input bounds
Every duration, interval, hold, dwell and per-character delay is bounded to **[0, 60000] ms**,
`steps` to **[1, 2000]**, `count`/`clickState` to **[1, 3]**, and scroll `dy`/`dx` to
**[-100, 100]** lines. The adapter rejects out-of-range values with INVALID_TARGET; the helper
clamps independently, because a trapping numeric conversion would kill it mid-drag (exit 133).

### Helper protocol — `src/platform/macos/helper/main.swift` (version 1.1.0, protocol 2)
One JSON request per stdin line, one JSON reply per stdout line, ids correlate them; stderr carries
diagnostics only. Verbs: `move`, `down`, `up`, `click`, `path`, `scroll`, `keydown`, `keyup`,
`press`, `type`, `release_all`, `cursor`, `permissions`, `windows`, `focus`, `frontmost`, `version`.
Failure replies carry `code`: `INVALID_ARGS`/`INVALID_VERB` map to INVALID_TARGET,
`NATIVE_INPUT_PERMISSION_DENIED`, `WINDOW_NOT_FOUND` and `FOCUS_FAILED` map to themselves, anything
else to INTERNAL.

`release_all` takes `{"osState": bool}`, default **false**: it then releases only what the helper
itself is holding. `osState: true` additionally releases what the OS reports held — that report
includes the USER's own hand on the mouse and their own held modifiers, so it is reserved for the
client recovering from a CRASHED helper, whose tracked state died with it.

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
export type Mode = "real_user" | "webview" | "diagnostic";
export interface ActionResult {
  actionId: string; status: "ok" | "warning" | "error"; mode: Mode;
  backend: "cgevent" | "webdriver" | "js" | "none"; windowId?: number;
  target?: { ref?: string; role?: string; name?: string; testId?: string };
  resolvedPoint?: { global: Pt; css: Pt };
  state?: { beforeRevision: number; afterRevision: number; settled: boolean };
  effects?: { consoleErrors: number; logErrors: number; newWindows: number; windowMoved: boolean };
  screenshot?: { path: string; previewPath: string; width: number; height: number; pixelScale: number; captureMode: string };
  warnings: string[];
  error?: { code: ErrorCode; message: string; remediation: string; details?: Record<string, unknown> };
  timingsMs: { resolve: number; input: number; settle: number; capture: number };
  data?: unknown;   // tool-specific payload (snapshot text, log lines, ...)
}
```

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

## Teardown report — `SessionStatus.teardown`
`session_stop` measures `{survivors, portsFree: {"4445": bool, "1420": bool}, launched}` after the sweep and
returns it in the status (and in `STOP_INCOMPLETE.details`). Reports cite this, never an assumption.
