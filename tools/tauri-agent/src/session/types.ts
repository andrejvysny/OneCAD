/**
 * Public session types. They live apart from `orchestrator.ts` so the calibration and
 * teardown modules can reference the session's bridge surface without importing the class.
 */
import type { Bridge } from "../semantic/webdriver.ts";
import type { SnapNode } from "../semantic/snapshot.ts";
import type { Pt, Rect } from "../geometry/types.ts";
import type { KeyboardLayoutInfo, PlatformAdapter, Permissions } from "../platform/adapter.ts";
import type { CalibrationReport } from "./calibrate.ts";
import type { FetchFn, Spawner } from "./launch.ts";
import type { KillFn, Runner } from "./procs.ts";

export type SessionState =
  | "Idle"
  | "Launching"
  | "WaitingForBridge"
  | "WaitingForWindow"
  | "Ready"
  | "Reconnecting"
  | "Stopping"
  | "Failed";

export type RefStore = Map<string, SnapNode>;

/**
 * How much of the user's desktop this session is allowed to touch.
 *
 * "foreground" is the original harness: the app is activated before every action and native
 * CGEvents are posted into the system event stream, so the session genuinely proves a human
 * could do the same thing — at the price of taking the pointer and the frontmost application.
 *
 * "background" promises the opposite: no cursor movement, no activation, no OS event. It is
 * enforced structurally by handing the session a `PlatformAdapter` whose input verbs and whose
 * `focus` refuse (`platform/refusing.ts`), so no code path can escalate by accident. What it
 * can still do is dispatch the event sequence inside the page, actuate native chrome through
 * accessibility, and photograph the window — all of which work while the window is behind
 * whatever the user is actually looking at.
 */
export type InteractionPolicy = "foreground" | "background";

/**
 * Who owns the app process this session drives. It is the ONLY thing that licenses a
 * signal: a session that attached is a guest in a process the developer started by
 * hand, and terminating it would take out their app.
 */
export type ProcessOwnership = { kind: "launched"; pid: number } | { kind: "attached"; pid: number };

/**
 * Reply of the `agent_identity` handshake (`src-tauri/src/tauri_e2e.rs`, compiled only
 * under the `tauri-e2e` feature). `cargoManifestDir` is baked in at COMPILE time, so it
 * proves the running binary was built from this source tree.
 */
export interface AgentIdentity {
  runtime: string;
  agentTesting: boolean;
  bundleId: string;
  pid: number;
  executable: string;
  cargoManifestDir: string;
  sessionNonce: string;
}

/**
 * What the webview reports about one Tauri window, in Tauri's own units: `outerPositionPx`
 * and `innerSizePx` are PHYSICAL pixels and `scaleFactor` is what turns them into the global
 * display points every native coordinate in this harness is measured in.
 */
export interface TauriWindowReading {
  label: string;
  /** null when the window has no title; an empty title must not match an empty AX title. */
  title: string | null;
  outerPositionPx: Pt;
  innerSizePx: { width: number; height: number };
  scaleFactor: number;
}

/**
 * One Tauri label, and the native window it was correlated to — or an explicit record that it
 * could NOT be.
 *
 * `windowId` is present only when the correlation resolved. An entry without one is never
 * silently approximated to "the largest window": every consumer refuses on it, because the
 * cost of being wrong here is a real click posted into a window the caller did not name.
 */
export interface WindowIdentity {
  label: string;
  /** The Tauri title when the table was built. */
  title: string | null;
  /** CGWindowID. Absent exactly when `unresolved` is set. */
  windowId?: number;
  /** The AX window bounds that matched, global points; the fallback identity when an id is not. */
  bounds?: Rect;
  /** How it was correlated ("sole", "bounds", "bounds+title", …), or the attempt that failed. */
  source: string;
  /**
   * The application's `AXFocusedWindow` and `AXModal` AT BUILD TIME. Both are live facts that
   * change without any geometry change, so the input gate re-reads them and never trusts these.
   */
  key: boolean;
  modal: boolean;
  /** Why the label could not be correlated. Present exactly when `windowId` is absent. */
  unresolved?: string;
}

/**
 * The session's label ↔ native window table. Built during calibration, and the ONLY authority
 * on which native window a Tauri label names once it exists.
 */
export interface WindowTable {
  at: string;
  pid: number;
  /** false when `_AXUIElementGetWindow` could not be resolved: every id came from bounds+title. */
  privateWindowIdApi: boolean;
  entries: WindowIdentity[];
}

/** The bridge surface the session and the tools use; a `Pick` so tests can supply a fake. */
export type SessionBridge = Pick<Bridge, "execute" | "invoke" | "windowGeom" | "close"> & {
  readonly wedged: boolean;
};

export interface StartOptions {
  mode: "launch" | "attach";
  launch?: "dev" | "bundled";
  port?: number;
  reuseExisting?: boolean;
  env?: Record<string, string>;
  /** Proceed without the Screen Recording grant; screenshots then show wallpaper only. */
  allowDegradedCapture?: boolean;
  /**
   * How much of the desktop this session may touch. Defaults to `interaction.default` in
   * `tauri-agent.config.json`, which itself defaults to "foreground" — the original behaviour.
   */
  interaction?: InteractionPolicy;
}

export interface StopOptions {
  /** Launched sessions only: terminate the app this session started. Default true. Ignored when the session attached. */
  killApp?: boolean;
  /** Attached sessions only: deliberately terminate an app this session did NOT launch. Default false. */
  forceKillAttached?: boolean;
}

/**
 * What this session can actually prove with a picture — read it BEFORE attempting a capture.
 *
 * A session started with `allowDegradedCapture` has no Screen Recording grant, so every
 * screenshot attempt fails or photographs the wallpaper. Attempting one anyway turns a
 * delivered click into a failure whose message ("capture failed") hides the real fact: in
 * this session a capture was never possible.
 */
export interface CaptureCapability {
  /** Screenshots can be attempted at all. */
  available: boolean;
  /** A captured image is trustworthy evidence of what was on screen. */
  authoritative: boolean;
  reason?: "SCREEN_RECORDING_PERMISSION_DENIED";
}

export interface SessionStatus {
  sessionId: string;
  state: SessionState;
  ready: boolean;
  pid?: number;
  windowId?: number;
  /** The Tauri label the coordinate mapping and the input gate are both about. */
  targetLabel: string;
  /** Set while the session is deliberately driving a modal of this app instead of `targetLabel`. */
  targetIsModal?: boolean;
  /** Correlated label ↔ native window table; absent when it could not be built at all. */
  windowTable?: WindowTable;
  /**
   * Why the table could not be built. The window-level input gate then has no identity to
   * enforce and degrades to the application-level frontmost check it made before this table
   * existed — which is why it is reported rather than logged and forgotten.
   */
  windowTableWarning?: string;
  webdriverPort: number;
  devServerPort: number;
  bridgeConnected: boolean;
  platformReady: boolean;
  calibrated: boolean;
  artifactsDir: string;
  sessionEpoch: number;
  launched: boolean;
  wheelLinesPerNotch: number;
  /** Who owns the app process — "attached" means this session must not signal it. */
  ownership?: ProcessOwnership;
  /** The verified `agent_identity` handshake; absent when it could not be completed. */
  identity?: AgentIdentity;
  /** Why the handshake could not be completed (e.g. an app built before it existed). */
  identityWarning?: string;
  /** Present when the page could not be instrumented; settle and console counts are degraded. */
  instrumentWarning?: string;
  permissions?: Permissions;
  /** Native input capability facts read once at session start (see `#preflight`). */
  input?: { keyboardLayout?: KeyboardLayoutInfo };
  /** Set when the active layout is not ANSI/US: `keyboard_press` may send the wrong character. */
  keyboardLayoutWarning?: string;
  /** Whether this session can take a screenshot at all, and whether one would be evidence. */
  capture: CaptureCapability;
  /** How much of the desktop this session may touch; "background" never activates or moves the cursor. */
  interaction: InteractionPolicy;
  /**
   * Set when a BACKGROUND session launched the app and the launch activated it anyway.
   *
   * It is not a harness choice and no flag avoids it: tao's `window_activation_hack` calls
   * `makeKeyAndOrderFront` on every visible window at `applicationDidFinishLaunching`, and
   * `activate_ignoring_other_apps` defaults to true. Use `mode:"attach"` against an app you
   * started yourself to avoid it entirely. Every action AFTER the launch still reports
   * `interaction.foregroundChanged: false`.
   */
  foregroundStolenAtLaunch?: boolean;
  calibration?: CalibrationReport;
  failure?: { code: string; message: string };
  /** Measured result of the last stop: what survived the sweep and whether the ports are free. */
  teardown?: TeardownReport;
}

export interface TeardownReport {
  survivors: Array<{ pid: number; command: string }>;
  portsFree: Record<string, boolean>;
  launched: boolean;
  /** True when the session attached and deliberately left the app running — survivors are expected. */
  detached: boolean;
}

export interface SessionDeps {
  runner?: Runner;
  platform?: () => PlatformAdapter;
  bridgeFactory?: (port: number) => Promise<SessionBridge>;
  fetch?: FetchFn;
  spawn?: Spawner;
  kill?: KillFn;
  sleep?: (ms: number) => Promise<void>;
}

