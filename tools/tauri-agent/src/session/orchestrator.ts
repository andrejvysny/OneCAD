/**
 * Session lifecycle: launch or attach, prove the app is really there, calibrate the
 * coordinate mapping, and tear the whole process tree down again.
 *
 * Two rules shape the code. (1) Nothing is Ready until the mapping has been proven with
 * a real cursor move — every later tool trusts `geom` to turn a CSS rect into a screen
 * point, and a wrong `geom` clicks in another application. (2) `stop()` must leave no
 * survivors: the app intercepts its own close, so the only reliable teardown is signalling
 * the launched process group and then sweeping for processes that belong to THIS project.
 */
import { realpathSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve } from "node:path";
import { AgentError } from "../errors.ts";
import { log } from "../log.ts";
import type { Rect, WindowGeom } from "../geometry/types.ts";
import type {
  AxWindowInfo,
  AxWindowsResult,
  KeyboardLayoutInfo,
  NativeWindowInfo,
  NativeWindows,
  Permissions,
  PlatformAdapter,
} from "../platform/adapter.ts";
import { createPlatformAdapter } from "../platform/adapter.ts";
import { backgroundAdapter } from "../platform/refusing.ts";
import { Bridge } from "../semantic/webdriver.ts";
import type { Snapshot } from "../semantic/snapshot.ts";
import { ensureInstrumentation } from "../semantic/snapshot.ts";
import type { Journal } from "../trace/journal.ts";
import {
  buildGeom,
  buildReport,
  buildWindowTable,
  focusTauriWindow,
  geomChecks,
  geomDiffers,
  hoverProbe,
  pickLargestWindow,
  readTauriWindows,
  requireIdentity,
  sameWindow,
  selectTargetWindow,
  wheelProbe,
} from "./calibrate.ts";
import type { CalibrationReport, ProbeDeps } from "./calibrate.ts";
import type {
  AgentIdentity,
  CaptureCapability,
  InteractionPolicy,
  ProcessOwnership,
  SessionBridge,
  SessionDeps,
  SessionState,
  SessionStatus,
  TeardownReport,
  StartOptions,
  StopOptions,
  RefStore,
  WindowIdentity,
  WindowTable,
} from "./types.ts";
import type { ResolvedConfig } from "./config.ts";
import { resolveFromRoot } from "./config.ts";
import type { LaunchHandle } from "./launch.ts";
import { childProcessSpawner, httpFetch, launchSpec, waitForWebDriver } from "./launch.ts";
import { isAlive, listeningPids, signal, spawnRunner } from "./procs.ts";
import {
  captureCapabilityOf,
  checkDevServerPort,
  checkPermissions,
  launcherAbortCheck,
  portOwnerIsOurs,
  resolveListenerPid,
} from "./startup.ts";
import type { Survivor } from "./teardown.ts";
import { killLauncherGroup, sweepSurvivors } from "./teardown.ts";
import { findSurvivors } from "./procs.ts";

export type * from "./types.ts";

const ATTACH_READY_TIMEOUT_MS = 60_000;
const WINDOW_WAIT_MS = 30_000;
const WINDOW_POLL_MS = 500;
const RECONNECT_TIMEOUT_MS = 60_000;
const WATCHDOG_INTERVAL_MS = 2_000;
/**
 * How long one accessibility window-state read is reused by the input gate.
 *
 * The gate runs on every acting tool, and `ax.windows(pid)` is synchronous IPC into the target
 * app. Two things keep that affordable. It is a window-list read, not a tree walk — a handful of
 * windows at a few attributes each, one helper round trip — and it answers BOTH questions the
 * gate asks (which window is key, and what is modal), so `ax.modal()` is never called alongside
 * it. This window then coalesces the reads of a tool that gates twice in one action into one.
 *
 * It is deliberately far shorter than any action's own resolve-and-settle, so a cached reading
 * can never outlive the action that took it; and `focus()` drops it, because activating the app
 * is precisely the event that changes which window is key.
 */
/** Why a background session's hover probe did not run; carried into `session_status`. */
export const BACKGROUND_HOVER_SKIP =
  'interaction:"background" — the hover probe moves the real cursor, so the CSS→global mapping is computed but unverified. Nothing in this policy posts a global point.';

/** Why a background session's wheel probe did not run. */
export const BACKGROUND_WHEEL_SKIP =
  'interaction:"background" — the wheel probe posts real notches. pointer_scroll dispatches a WheelEvent in the page instead, so wheelLinesPerNotch is not consulted.';

const WINDOW_STATE_TTL_MS = 150;

/** Feature-gated handshake command; only a `tauri-e2e` build registers it. */
const IDENTITY_COMMAND = "agent_identity";
/**
 * Tauri rejects a command it does not know with "Command <name> not found"
 * (tauri 2.11.5 `src/webview/mod.rs:1905`) or "<cmd> not allowed. Command not found"
 * (`src/ipc/authority.rs:404`).
 *
 * Deliberately NOT matching a bare "unknown command": that is WebDriver's own standard error
 * for an unimplemented endpoint, so accepting it would let a DRIVER fault masquerade as an old
 * app binary and downgrade a hard refusal into a warning — after which the session drives an
 * app whose identity was never proven.
 */
const UNKNOWN_COMMAND = /command\b[\s\S]*\bnot found\b/i;

/**
 * The real path, or the lexically resolved one when the path does not exist here.
 *
 * `CARGO_MANIFEST_DIR` is baked in on the machine that BUILT the binary, so it may name a
 * directory this process cannot stat; falling back keeps the comparison total.
 */
/** True when the probe posted at least one real notch — including the skips that posted one. */
function wheelPosted(wheel: CalibrationReport["wheelProbe"]): boolean {
  return "skipped" in wheel ? wheel.posted : true;
}

/**
 * Re-stamps a failure that happened AFTER the app was activated with `details.raised`.
 *
 * `window_focus` raises a window before it recalibrates, so a failure in the second half is not
 * a "nothing happened" failure — and an envelope that claimed `retrySafe` for it would be wrong
 * about the one thing the delivery block exists to state.
 */
function afterRaise(cause: unknown): AgentError {
  const err = cause instanceof AgentError ? cause : new AgentError("INTERNAL", String(cause), { cause });
  return new AgentError(err.code, err.message, {
    remediation: err.remediation,
    details: { ...(err.details ?? {}), raised: true },
    cause,
  });
}

/** Enough of an AX window for a refusal message to be actionable: which one, and which id. */
function describeWindow(w: AxWindowInfo): string {
  const title = w.title === null ? "untitled" : JSON.stringify(w.title);
  return `${title}, CGWindowID ${w.window.known ? String(w.window.id) : `unknown (${w.window.source})`}`;
}

function realOrResolved(p: string): string {
  // Realpathing only the WHOLE path is not enough, and the asymmetry bites: on macOS `/var` is
  // a symlink to `/private/var`, so an existing root normalises to `/private/var/...` while a
  // `CARGO_MANIFEST_DIR` naming a directory this machine does not have falls back to `/var/...`
  // — and the two stop comparing equal even though one really is inside the other. Walking up
  // to the first ancestor that DOES exist normalises both sides the same way.
  let current = resolve(p);
  const tail: string[] = [];
  for (;;) {
    try {
      const real = realpathSync(current);
      return tail.length === 0 ? real : join(real, ...tail);
    } catch {
      const parent = dirname(current);
      if (parent === current) return resolve(p);
      tail.unshift(basename(current));
      current = parent;
    }
  }
}

/**
 * True when `child` is `root` or below it, compared through the REAL path of each.
 *
 * Symlinks and case are the trap. A checkout reached as `~/dev/OneCAD` where `~/dev` is a
 * symlink, or anything under `/tmp` (which is `/private/tmp`), makes the app's compile-time
 * `CARGO_MANIFEST_DIR` and the harness's own root spell the same directory two different ways
 * — and a purely lexical comparison then refuses every `session_start` on a perfectly good
 * checkout, with no way to override it. macOS filesystems are also case-insensitive by default,
 * so the two spellings can differ only in case and still be one directory; `realpathSync`
 * normalises that too. `/repo-other` is still correctly rejected against `/repo`.
 */
function isInside(root: string, child: unknown): boolean {
  if (typeof child !== "string" || child.length === 0) return false;
  const rel = relative(realOrResolved(root), realOrResolved(child));
  return rel === "" || (!rel.startsWith("..") && !isAbsolute(rel));
}

/**
 * null when the handshake agrees with this session; otherwise the reason it does not.
 * The reply is untrusted input, so each check is total over any JSON value.
 */
function identityMismatch(identity: AgentIdentity, pid: number, root: string): string | null {
  if (identity.agentTesting !== true) {
    return "agentTesting is not true — the app was not built with the tauri-e2e feature";
  }
  if (identity.pid !== pid) {
    return `the app reports pid ${String(identity.pid)}, but the WebDriver port is held by pid ${pid}`;
  }
  // CARGO_MANIFEST_DIR is baked in at COMPILE time, so it proves the running binary was
  // built from THIS source tree — which no runtime path inspection can. It is
  // `<root>/src-tauri`, so the test is "inside root", never equality.
  if (!isInside(root, identity.cargoManifestDir)) {
    return `the app was built from ${JSON.stringify(identity.cargoManifestDir)}, which is not inside ${root}`;
  }
  return null;
}

export class SessionOrchestrator {
  state: SessionState = "Idle";
  bridge?: SessionBridge;
  platform?: PlatformAdapter;
  geom?: WindowGeom;
  pid?: number;
  windowId?: number;
  /**
   * The Tauri label this session is calibrated to and gates every acting tool on. Starts at the
   * configured main window and moves only through `focusWindow`, which refuses a label it cannot
   * correlate rather than re-pointing the session at a window it cannot name.
   */
  targetLabel: string;
  /**
   * Set while the session is deliberately driving a modal of this app — a native save panel has
   * no Tauri label, so no table entry can name it. The gate then requires that a modal IS up and
   * key instead of refusing it as an obstruction; the coordinate mapping still belongs to
   * `targetLabel`, because a modal hosts no webview.
   */
  targetIsModal = false;
  /** Correlated label ↔ native window table; the identity every gate reads. */
  windowTable?: WindowTable;
  /** Why the table could not be built; the window gate then has nothing to enforce. */
  windowTableWarning?: string;
  sessionEpoch = 0;
  wheelLinesPerNotch: number;
  permissions?: Permissions;
  /** Set by `#preflight`, read once. Absent when the adapter does not offer `layout()`. */
  keyboardLayout?: KeyboardLayoutInfo;
  /** Set by `#preflight` when the layout is not ANSI/US; never a mismatch verdict, just a fact. */
  keyboardLayoutWarning?: string;
  /**
   * Set by `#preflight` from the grant this process holds. The default is the honest reading
   * of a session that has not started: nothing is running, so nothing can be photographed.
   */
  captureCapability: CaptureCapability = { available: false, authoritative: false };
  calibration?: CalibrationReport;
  /** Last snapshot taken by ui_snapshot; `mode:"diff"` compares against it. */
  lastSnapshot?: Snapshot;
  /** Verified `agent_identity` reply of the app being driven. */
  identity?: AgentIdentity;
  /** Why the identity handshake could not be completed; never a mismatch verdict. */
  identityWarning?: string;
  /** Set when the page could not be instrumented; settle and console counts are degraded. */
  instrumentWarning?: string;
  readonly sessionId: string;
  /**
   * How much of the desktop this session may touch. Fixed for the session's whole life at
   * `start()`; every gate and every envelope reads it rather than re-deciding.
   */
  interactionPolicy: InteractionPolicy = "foreground";
  /** A background session that launched: tao activated the app anyway. See `SessionStatus`. */
  foregroundStolenAtLaunch = false;
  readonly journal: Journal;
  readonly refs: RefStore = new Map();
  readonly config: ResolvedConfig;
  failure?: { code: string; message: string };
  teardown?: TeardownReport;

  readonly #deps: Required<SessionDeps>;
  #ownership?: ProcessOwnership;
  #launcher?: LaunchHandle;
  #port: number;
  #launchKind?: "dev" | "bundled";
  /** Processes of this checkout that were running before this session started; never killed. */
  #preexisting: ReadonlySet<number> = new Set<number>();
  #watchdog?: ReturnType<typeof setInterval>;
  #recovering = false;
  /** One AX window-state reading, reused for WINDOW_STATE_TTL_MS. */
  #windowState?: { at: number; result: AxWindowsResult };

  constructor(opts: { sessionId: string; journal: Journal; config: ResolvedConfig; deps?: SessionDeps }) {
    this.sessionId = opts.sessionId;
    this.journal = opts.journal;
    this.config = opts.config;
    this.targetLabel = opts.config.config.app.windowLabel;
    this.#port = opts.config.config.webdriver.port;
    this.wheelLinesPerNotch = opts.config.config.input.wheelLinesPerNotch;
    this.interactionPolicy = opts.config.config.interaction.default;
    const d = opts.deps ?? {};
    this.#deps = {
      runner: d.runner ?? spawnRunner,
      platform: d.platform ?? createPlatformAdapter,
      bridgeFactory: d.bridgeFactory ?? ((port) => Bridge.connect(port)),
      fetch: d.fetch ?? httpFetch,
      spawn: d.spawn ?? childProcessSpawner,
      kill: d.kill ?? ((pid, sig) => process.kill(pid, sig)),
      sleep: d.sleep ?? ((ms) => new Promise((r) => setTimeout(r, ms))),
    };
  }

  status(): SessionStatus {
    return {
      sessionId: this.sessionId,
      state: this.state,
      ready: this.state === "Ready",
      ...(this.pid === undefined ? {} : { pid: this.pid }),
      ...(this.windowId === undefined ? {} : { windowId: this.windowId }),
      targetLabel: this.targetLabel,
      ...(this.targetIsModal ? { targetIsModal: true } : {}),
      ...(this.windowTable === undefined ? {} : { windowTable: this.windowTable }),
      ...(this.windowTableWarning === undefined ? {} : { windowTableWarning: this.windowTableWarning }),
      webdriverPort: this.#port,
      devServerPort: this.config.config.devServer.port,
      bridgeConnected: this.bridge !== undefined,
      platformReady: this.platform !== undefined,
      calibrated: this.geom !== undefined && this.calibration?.ok === true,
      artifactsDir: this.journal.dir,
      sessionEpoch: this.sessionEpoch,
      launched: this.#launcher !== undefined,
      wheelLinesPerNotch: this.wheelLinesPerNotch,
      ...(this.#ownership === undefined ? {} : { ownership: this.#ownership }),
      ...(this.identity === undefined ? {} : { identity: this.identity }),
      ...(this.identityWarning === undefined ? {} : { identityWarning: this.identityWarning }),
      ...(this.instrumentWarning === undefined ? {} : { instrumentWarning: this.instrumentWarning }),
      ...(this.permissions === undefined ? {} : { permissions: this.permissions }),
      ...(this.keyboardLayout === undefined ? {} : { input: { keyboardLayout: this.keyboardLayout } }),
      ...(this.keyboardLayoutWarning === undefined ? {} : { keyboardLayoutWarning: this.keyboardLayoutWarning }),
      capture: this.captureCapability,
      interaction: this.interactionPolicy,
      ...(this.foregroundStolenAtLaunch ? { foregroundStolenAtLaunch: true } : {}),
      ...(this.calibration === undefined ? {} : { calibration: this.calibration }),
      ...(this.failure === undefined ? {} : { failure: this.failure }),
      ...(this.teardown === undefined ? {} : { teardown: this.teardown }),
    };
  }

  requireReady(): void {
    if (this.state !== "Ready") {
      throw new AgentError("APP_NOT_RUNNING", `Session is ${this.state}, not Ready`, {
        details: { state: this.state, sessionId: this.sessionId },
      });
    }
  }

  /**
   * Refuses unless the event about to be posted would reach THIS session's window.
   *
   * Two questions, not one. "Is this application in front" is what `isFrontmost` answers, and it
   * is necessary but not sufficient: an app is frontmost while a second window of its own is key,
   * and frontmost while a save panel covers the window the caller named. So the application check
   * runs first (it is the cheap one, and the one `focus()` can fix), and the window check follows.
   *
   * `isFrontmost` is a read-only check when the adapter offers one; otherwise `focus()`
   * doubles as the check, which also raises the window — acceptable, since input that is
   * about to be sent needs that window in front anyway.
   */
  async assertFrontmost(): Promise<void> {
    const windows = this.requirePlatform().windows as NativeWindows & {
      isFrontmost?: (pid: number) => Promise<boolean>;
    };
    const pid = this.#requirePid();
    const ok = windows.isFrontmost ? await windows.isFrontmost(pid) : await windows.focus(pid);
    if (!ok) {
      throw new AgentError("WINDOW_NOT_FOREGROUND", `the app (pid ${pid}) is not the frontmost application`, {
        details: { pid, reason: "app-not-frontmost", windowId: this.windowId },
      });
    }
    await this.#assertTargetWindow(pid);
  }

  /**
   * Focus the app if something else is in front; WINDOW_NOT_FOREGROUND only if that fails.
   *
   * "OrFocus" fixes what activating an application can fix, which is the application being
   * behind another app. It cannot fix the window-level conditions — activating raises whichever
   * window is already key, and dismissing a modal is the caller's decision, not ours — so those
   * stay refusals that name the fix.
   */
  async assertFrontmostOrFocus(): Promise<boolean> {
    const windows = this.requirePlatform().windows as NativeWindows & {
      isFrontmost?: (pid: number) => Promise<boolean>;
    };
    const pid = this.#requirePid();
    let activated = false;
    if (!(windows.isFrontmost && (await windows.isFrontmost(pid)))) {
      this.#windowState = undefined;
      // In a background session `windows.focus` is the refusing seam and throws here, which is
      // the intended outcome: the caller asked for something that needs the app in front.
      if (!(await windows.focus(pid))) {
        throw new AgentError("WINDOW_NOT_FOREGROUND", `the app (pid ${pid}) could not be brought to the front`, {
          details: { pid, reason: "app-not-frontmost", windowId: this.windowId },
        });
      }
      activated = true;
    }
    await this.#assertTargetWindow(pid);
    return activated;
  }

  /**
   * The window half of the gate: the target is the key window of this app, and nothing modal is
   * covering it.
   *
   * No table means no identity evidence — the session has not calibrated yet, or the table could
   * not be built and `windowTableWarning` says why — and the gate then degrades to the
   * application-level check it made before this table existed. That is a stated degradation, not
   * a silent one: it is in `session_status`. What it never does is GUESS, which is why an
   * unresolved label refuses here rather than falling back to the largest window.
   *
   * An accessibility read that fails is deliberately not swallowed. Accessibility is granted
   * (`checkPermissions` refuses to start a session without it), so a failure here means the
   * helper is broken — and posting input while the window state is unknown is the exact thing
   * this gate exists to prevent.
   */
  async #assertTargetWindow(pid: number): Promise<void> {
    const table = this.windowTable;
    if (table === undefined) return;
    const pool = (await this.#axWindows(pid)).windows.filter((w) => w.role === null || w.role === "AXWindow");
    const modals = pool.filter((w) => w.modal);
    const key = pool.find((w) => w.focused);

    if (this.targetIsModal) {
      if (modals.length === 0) {
        throw this.#notForeground("no-modal-up", "the session is targeting a modal of this app, but none is up", {
          pid,
          windows: pool.map(describeWindow),
        });
      }
      if (key !== undefined && !key.modal) {
        throw this.#notForeground("modal-not-key", `the session is targeting a modal of this app, but ${describeWindow(key)} is the key window`, {
          pid,
          key: describeWindow(key),
        });
      }
      return;
    }

    const entry = table.entries.find((e) => e.label === this.targetLabel);
    if (entry === undefined || entry.windowId === undefined) {
      throw this.#notForeground(
        "target-unresolved",
        `the target window "${this.targetLabel}" is not correlated to a native window (${entry?.unresolved ?? "no such label"}), so nothing here can prove which window this input would reach`,
        { pid, label: this.targetLabel, source: entry?.source },
      );
    }
    const isTarget = (w: AxWindowInfo): boolean => sameWindow(w, entry);
    // One window, and it is the target: nothing else of this app can be key or modal, whatever
    // `AXFocusedWindow` happens to say. This is the single-window app, which is most of them.
    if (pool.length === 1 && isTarget(pool[0] as AxWindowInfo)) return;

    const blocking = modals.filter((w) => !isTarget(w));
    if (blocking.length > 0) {
      throw this.#notForeground(
        "modal-covering-target",
        `a modal of this app (${blocking.map(describeWindow).join("; ")}) is covering the target window "${this.targetLabel}"`,
        { pid, label: this.targetLabel, blockers: blocking.map(describeWindow) },
      );
    }
    if (key === undefined) {
      throw this.#notForeground("no-key-window", `this app has no key window, so "${this.targetLabel}" is not the window this input would reach`, {
        pid,
        label: this.targetLabel,
        windows: pool.map(describeWindow),
      });
    }
    if (!isTarget(key)) {
      throw this.#notForeground(
        "other-window-key",
        `another window of this app (${describeWindow(key)}) is the key window, not the target "${this.targetLabel}"`,
        { pid, label: this.targetLabel, key: describeWindow(key) },
      );
    }
  }

  #notForeground(reason: string, message: string, details: Record<string, unknown>): AgentError {
    return new AgentError("WINDOW_NOT_FOREGROUND", message, { details: { reason, ...details } });
  }

  /** See WINDOW_STATE_TTL_MS: one read per gate, reused across a tool that gates twice. */
  async #axWindows(pid: number): Promise<AxWindowsResult> {
    const now = Date.now();
    const cached = this.#windowState;
    if (cached !== undefined && now - cached.at < WINDOW_STATE_TTL_MS) return cached.result;
    const result = await this.requirePlatform().ax.windows(pid);
    this.#windowState = { at: now, result };
    return result;
  }

  /** Every acting tool calls this: Ready, and the mapping still describes the window. */
  async ensureCalibrated(): Promise<void> {
    this.requireReady();
    if (!(await this.geometryChanged())) return;
    log.info("window geometry changed; recalibrating");
    // The hover probe posts real input and reads `:hover`, which WebKit only applies to the
    // active window; a query tool may run while another app is in front, so bring ours back.
    // A background recalibration runs none of that, and must not raise the app either — this
    // path is reached from `ui_screenshot`, which would otherwise become a focus steal.
    if (this.interactionPolicy === "foreground") await this.assertFrontmostOrFocus();
    await this.calibrate();
  }

  async start(opts: StartOptions): Promise<SessionStatus> {
    if (this.state !== "Idle" && this.state !== "Failed") {
      throw new AgentError("INTERNAL", `session is ${this.state}; call session_stop first`, {
        remediation: "Call session_stop, then session_start again.",
      });
    }
    this.failure = undefined;
    this.teardown = undefined;
    this.state = "Launching";
    try {
      const plan = await this.#preflight(opts);
      if (plan.spawn) this.#spawnApp(opts, plan.kind);
      await this.#awaitWebDriver(plan.timeoutMs);
      this.pid = await this.#resolveAppPid();
      // Ownership is decided HERE and nowhere else. `#launcher` exists only on the path
      // that spawned the app, so its absence means this session is a guest in a process
      // somebody else started — and `stop()` must not signal it.
      this.#ownership = { kind: this.#launcher === undefined ? "attached" : "launched", pid: this.pid };
      this.state = "WaitingForBridge";
      this.bridge = await this.#deps.bridgeFactory(this.#port);
      await this.#verifyIdentity(this.bridge, this.pid);
      await this.#instrument(this.bridge);
      this.state = "WaitingForWindow";
      const win = await this.#discoverWindow(this.pid);
      this.windowId = win.windowId;
      // Never calibrate against a background window: the hover and wheel probes post real
      // native input, which the frontmost application would receive instead. A background
      // session runs neither probe, so it neither needs nor is allowed to raise anything.
      if (this.interactionPolicy === "foreground") {
        if (!(await this.requirePlatform().windows.focus(this.pid))) {
          throw new AgentError("FOCUS_FAILED", `the app (pid ${this.pid}) did not become frontmost`, {
            details: { pid: this.pid, windowId: win.windowId },
          });
        }
      } else if (plan.spawn) {
        // Launching activates the app whatever we do — tao's `window_activation_hack` calls
        // `makeKeyAndOrderFront` on every visible window at applicationDidFinishLaunching, and
        // `activate_ignoring_other_apps` defaults to true. Say so rather than pretend.
        this.foregroundStolenAtLaunch = true;
        log.warn("background session launched the app, which activated it once", { pid: this.pid });
      }
      await this.calibrate();
      this.state = "Ready";
      this.#startWatchdog();
      log.info("session ready", { pid: this.pid, windowId: this.windowId, port: this.#port });
      return this.status();
    } catch (e) {
      await this.#failStart(e);
      throw e;
    }
  }

  /** Re-reads the window, re-checks the four numeric invariants, re-runs both probes. */
  async calibrate(): Promise<CalibrationReport> {
    const bridge = this.requireBridge();
    const platform = this.requirePlatform();
    const pid = this.#requirePid();
    const reading = await bridge.windowGeom();
    // Identity before geometry: the table decides WHICH window this session means, and
    // `selectTargetWindow` reaches `pickLargestWindow` only while no table exists.
    await this.#refreshWindowTable(pid);
    // `{all: true}` in background, and only there. A foreground calibration runs immediately after
    // the app was brought forward, so the on-screen list is the right and stricter source. A
    // background session never focuses, so its window may legitimately be on another Space or
    // behind a full-screen app — which is the ADVERTISED use case, the user working while the
    // agent drives — and an on-screen-only list would then be empty and every recalibration would
    // throw WINDOW_NOT_FOUND, dead-ending the session with no way out (`window_focus` also
    // refuses). Identity still comes from the label table; `all` only widens what it may match.
    const listed = await platform.windows.list(pid, this.interactionPolicy === "background" ? { all: true } : {});
    const win = selectTargetWindow(listed, this.windowTable, this.targetLabel, pid);
    const geom = buildGeom(reading, win);
    const checks = geomChecks(reading, geom);
    if (Object.values(checks).some((ok) => !ok)) {
      throw new AgentError("CALIBRATION_FAILED", "the window geometry checks disagree", {
        details: { checks, reading, nativeBounds: win.bounds },
      });
    }
    this.geom = geom;
    this.windowId = win.windowId;
    // A background session stops here. Everything above is passive — an in-page geometry read,
    // the accessibility window list, and CGWindowList — while both probes below post real native
    // input at the window, which is exactly what this policy promises never to do. The mapping
    // is therefore computed but UNVERIFIED, and `hoverProbe: {skipped}` is how a reader sees that.
    if (this.interactionPolicy === "background") {
      this.calibration = buildReport(reading, win, checks, { skipped: BACKGROUND_HOVER_SKIP }, {
        skipped: BACKGROUND_WHEEL_SKIP,
        posted: false,
      });
      return this.calibration;
    }
    const probes: ProbeDeps = {
      bridge,
      input: platform.input,
      geom,
      occlusion: this.#occlusion(),
      sleep: this.#deps.sleep,
      refocus: () => platform.windows.focus(pid).then(() => undefined),
    };
    const probeTestId = this.config.config.calibration.probeTestId;
    const hoverStartedAt = Date.now();
    const hover = await hoverProbe(probes, probeTestId);
    await this.#journalProbe("calibration.hoverProbe", { probeTestId }, hover, Date.now() - hoverStartedAt);
    const startLines = this.wheelLinesPerNotch;
    const wheelStartedAt = Date.now();
    const wheel = await wheelProbe(probes, startLines);
    if ("ok" in wheel && wheel.ok) this.wheelLinesPerNotch = wheel.linesPerNotch;
    // Journalled whenever a notch was POSTED, not merely when the probe succeeded. A skip that
    // posted a notch is exactly the case that most needs an entry: the sample never arrived,
    // which means that notch was not swallowed and reached whatever was under the cursor.
    if (wheelPosted(wheel)) {
      await this.#journalProbe("calibration.wheelProbe", { startLines }, wheel, Date.now() - wheelStartedAt);
    }
    this.calibration = buildReport(reading, win, checks, hover, wheel);
    return this.calibration;
  }

  /**
   * Rebuilds the label ↔ window table from the webview's own window list and the accessibility
   * window list, cross-checked against every window of the process.
   *
   * A failure DROPS the table rather than keeping the last one. A stale table is worse than no
   * table: it names CGWindowIDs that may no longer exist, and the gate would enforce them. With
   * no table the gate falls back to the application-level check and `windowTableWarning` carries
   * the reason into `session_status`.
   */
  async #refreshWindowTable(pid: number): Promise<void> {
    this.windowTableWarning = undefined;
    this.#windowState = undefined;
    const platform = this.requirePlatform();
    try {
      const readings = await readTauriWindows(this.requireBridge());
      const ax = await platform.ax.windows(pid);
      // `all: true` — a window on another Space is still the window its label names, and judging
      // its id "foreign to this process" for being off screen would unresolve a good entry.
      const natives = await platform.windows.list(pid, { all: true });
      this.windowTable = buildWindowTable(pid, readings, ax, natives);
      log.debug("window table built", {
        pid,
        privateWindowIdApi: this.windowTable.privateWindowIdApi,
        entries: this.windowTable.entries.map((e) => ({ label: e.label, windowId: e.windowId, source: e.source })),
      });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      this.windowTable = undefined;
      this.windowTableWarning =
        `the label-to-window table could not be built (${message}), so input is gated on the ` +
        "application being frontmost only — it cannot prove WHICH window of this app an event reaches";
      log.warn("window table unavailable", { pid, error: message });
    }
  }

  /**
   * Brings the window labelled `label` to the front and re-points this session at it.
   *
   * Ordering matters. The label is resolved against the table FIRST, so a label the harness
   * cannot name refuses before anything is raised; the app is activated, then the named window is
   * raised through Tauri (activating an application raises whichever of its windows is already
   * key, which is the wrong one exactly when a different label was asked for); and the retarget
   * is rolled back if the recalibration that follows refuses, because a session pointing at a
   * window it cannot map is worse than one that never moved.
   */
  async focusWindow(label: string): Promise<{ identity: WindowIdentity; recalibrated: boolean; warnings: string[] }> {
    this.requireReady();
    const pid = this.#requirePid();
    const platform = this.requirePlatform();
    if (this.windowTable === undefined) await this.#refreshWindowTable(pid);
    const table = this.windowTable;
    if (table === undefined) {
      throw new AgentError("WINDOW_NOT_FOUND", `no label-to-window table: ${this.windowTableWarning ?? "unknown reason"}`, {
        details: { label, pid },
      });
    }
    // Resolution first, and nothing is touched while it runs: a failure here really did post
    // nothing, which is what lets the tool report `not_started` honestly.
    const identity = requireIdentity(table, label);
    const warnings: string[] = [];
    const previous = this.targetLabel;
    const wasModal = this.targetIsModal;
    // Activation runs OUTSIDE the try that stamps `raised: true`, and the two ways it can fail are
    // deliberately NOT the same.
    //
    // A THROW means the call never got as far as trying — a background session's refusing seam
    // throws from here — so nothing was raised and the refusal must stay retry-safe. Stamping it
    // as raised made the envelope claim a delivered, non-retry-safe, side-effecting action for a
    // refusal that touched nothing, in the exact field a caller's retry logic reads; the repo's
    // own `smoke-actions.ts` then aborts the run instead of reporting a clean capability refusal.
    //
    // `false` is different: the adapter DID attempt the activation and the app did not come
    // forward, so the user's desktop may well have flickered. That one keeps `raised: true`.
    const activated = await platform.windows.focus(pid);
    try {
      if (!activated) {
        throw new AgentError("WINDOW_NOT_FOREGROUND", `the app (pid ${pid}) did not become frontmost`, {
          details: { pid, label, reason: "app-not-frontmost" },
        });
      }
      this.#windowState = undefined;
      try {
        if (!(await focusTauriWindow(this.requireBridge(), label))) {
          warnings.push(`the webview reports no window labelled "${label}", so only the application was raised`);
        }
      } catch (e) {
        // The app IS frontmost at this point; only the per-window raise failed, and the gate on
        // the next acting tool re-checks the outcome rather than trusting this call.
        warnings.push(`raising the "${label}" window through Tauri failed: ${e instanceof Error ? e.message : String(e)}`);
      }
      this.targetLabel = label;
      this.targetIsModal = false;
      const recalibrated = await this.geometryChanged();
      if (recalibrated) await this.calibrate();
      return { identity, recalibrated, warnings };
    } catch (e) {
      // Never leave the session pointing at a window it could not calibrate against: a target
      // whose mapping is unproven turns every later css point into a coordinate in the wrong window.
      this.targetLabel = previous;
      this.targetIsModal = wasModal;
      throw afterRaise(e);
    }
  }

  /** Re-runs only the wheel probe, for tools that reach a viewport after calibration skipped it. */
  async ensureWheelProbe(): Promise<CalibrationReport["wheelProbe"]> {
    const cal = this.calibration;
    if (cal && !("skipped" in cal.wheelProbe)) return cal.wheelProbe;
    // Background scrolling dispatches a WheelEvent in the page, which needs no notch
    // classification at all — and running the probe here would post the very CGEvents this
    // policy exists to prevent, from inside somebody else's pointer_scroll.
    if (this.interactionPolicy === "background") {
      return { skipped: BACKGROUND_WHEEL_SKIP, posted: false };
    }
    const probes: ProbeDeps = {
      bridge: this.requireBridge(),
      input: this.requirePlatform().input,
      geom: this.requireGeom(),
      occlusion: this.#occlusion(),
      sleep: this.#deps.sleep,
      refocus: () => this.requirePlatform().windows.focus(this.#requirePid()).then(() => undefined),
    };
    const startLines = this.wheelLinesPerNotch;
    const startedAt = Date.now();
    const wheel = await wheelProbe(probes, startLines);
    if ("ok" in wheel && wheel.ok) this.wheelLinesPerNotch = wheel.linesPerNotch;
    // This probe runs from INSIDE another tool's action (pointer_scroll), before that
    // action's own before/after capture, so without its own entry its notches would be
    // attributed to the scroll the caller asked for.
    if (wheelPosted(wheel)) {
      await this.#journalProbe("calibration.wheelProbe", { startLines, lazy: true }, wheel, Date.now() - startedAt);
    }
    if (cal) cal.wheelProbe = wheel;
    return wheel;
  }

  /**
   * Calibration posts real native input and is not a tool call, so nothing else records it.
   * An unattributed CGEvent is exactly what makes a later action's journal unreadable.
   */
  async #journalProbe(tool: string, input: unknown, result: unknown, durationMs: number): Promise<void> {
    await this.journal.append({
      ts: new Date().toISOString(),
      sessionId: this.sessionId,
      actionId: this.journal.nextActionId(),
      tool,
      input,
      result,
      durationMs,
    });
  }

  /** True when the window moved, resized or changed scale since the last calibration. */
  async geometryChanged(): Promise<boolean> {
    if (!this.geom || !this.bridge || !this.platform || this.pid === undefined) return true;
    // Same reasoning as `calibrate()`: a background session's window is routinely not on the
    // active Space, and that is normal rather than a reason to force a recalibration that would
    // then fail. See the comment there.
    const all = this.interactionPolicy === "background";
    const windows = await this.platform.windows.list(this.pid, all ? { all: true } : {});
    // A momentarily empty list is not proof the geometry changed, but it does mean we cannot
    // confirm it — treat as "recalibrate". In FOREGROUND that recalibration runs after the focus
    // step and will then see the window. In background there is no focus step, but the list above
    // already included off-screen windows, so an empty list there means the window really is gone.
    if (windows.length === 0) return true;
    let win: NativeWindowInfo;
    try {
      win = selectTargetWindow(windows, this.windowTable, this.targetLabel, this.pid);
    } catch {
      // The target cannot be named right now — a retarget, or a window that went away. This
      // method only answers "must we recalibrate", so it says yes and lets `calibrate()` be the
      // one place that refuses, with its own message.
      return true;
    }
    const reading = await this.bridge.windowGeom();
    return geomDiffers(reading, win, this.geom);
  }

  /** Rebuilds the WebDriver session after a BRIDGE_WEDGED; refs from before are void. */
  async reconnectBridge(): Promise<void> {
    const port = this.#port;
    await this.#dropBridge();
    this.bridge = await this.#deps.bridgeFactory(port);
    await this.#instrument(this.bridge);
    this.sessionEpoch += 1;
    this.refs.clear();
    this.lastSnapshot = undefined;
    if (this.state === "Ready") await this.calibrate();
  }

  async stop(opts: StopOptions = {}): Promise<SessionStatus> {
    if (this.state === "Idle" && this.#launcher === undefined && this.pid === undefined) {
      return this.status();
    }
    this.state = "Stopping";
    this.#stopWatchdog();
    const launched = this.#launcher !== undefined;
    // Only a session that LAUNCHED the app may terminate it. `killApp` governs that case
    // alone; an attached app is the developer's, and `forceKillAttached` is the single,
    // deliberate way to take it down. Nothing else ever signals that pid.
    const owned = this.#ownershipKind() === "launched";
    const mayKill = owned ? opts.killApp !== false : opts.forceKillAttached === true;
    // "Detached" is about whether anything was killed, NOT about who launched it. A launched
    // session stopped with `killApp:false` also leaves the app running — that is what the MCP
    // server's own SIGTERM path asks for — so its survivors are expected too.
    const detached = !mayKill;
    let survivors: Survivor[];
    try {
      await this.platform?.input.releaseAll().catch(() => undefined);
      await this.#dropBridge();
      // Gated on `mayKill`, not merely on having a launcher. The launcher is spawned detached
      // and the app lives IN its process group, so signalling the group kills the app — which
      // made `killApp:false` kill the very app it promised to leave alone, including on the MCP
      // server's SIGTERM shutdown path.
      if (mayKill && this.#launcher) await killLauncherGroup(this.#launcher, this.#deps.kill);
      if (mayKill && this.pid !== undefined) {
        signal(this.pid, "SIGTERM", this.#deps.kill);
      }
      // A detached sweep is REPORT-ONLY: it must observe the app, never kill it.
      survivors = await this.#sweep(mayKill);
      await this.platform?.input.dispose().catch(() => undefined);
    } finally {
      // A session that failed to tear down cleanly must still return to Idle, or every
      // later session_start would be refused by our own bookkeeping.
      this.#reset();
    }
    this.teardown = await this.#teardownReport(survivors, launched, detached);
    // After a detached stop the app is SUPPOSED to still be running: survivors are the
    // expected outcome and belong in the report, not in an error.
    // Only OUR leftovers make a teardown incomplete. A pre-existing process was spared on
    // purpose, so counting it here would turn correct behaviour into a reported failure.
    const leftovers = survivors.filter((s) => s.preexisting !== true);
    if (leftovers.length > 0 && !detached) {
      throw new AgentError("STOP_INCOMPLETE", `${leftovers.length} process(es) survived session_stop`, {
        details: { survivors: leftovers, teardown: this.teardown },
      });
    }
    return this.status();
  }

  /**
   * `#ownership` is only set once `start()` resolved the app pid. Before that the
   * launcher is the evidence: a start that spawned one and then failed still owns
   * whatever it produced, while one that failed earlier owns nothing at all.
   */
  #ownershipKind(): ProcessOwnership["kind"] {
    return this.#ownership?.kind ?? (this.#launcher === undefined ? "attached" : "launched");
  }

  /** The evidence a report cites for "nothing of ours is still running". */
  async #teardownReport(survivors: Survivor[], launched: boolean, detached: boolean): Promise<TeardownReport> {
    const cfg = this.config.config;
    const ports = [cfg.webdriver.port, cfg.devServer.port];
    const portsFree: Record<string, boolean> = {};
    for (const port of ports) {
      const pids = await listeningPids(port, this.#deps.runner).catch(() => [] as number[]);
      portsFree[String(port)] = pids.length === 0;
    }
    return { survivors, portsFree, launched, detached };
  }

  // --- start helpers -------------------------------------------------------

  async #preflight(opts: StartOptions): Promise<{ spawn: boolean; kind: "dev" | "bundled"; timeoutMs: number }> {
    const cfg = this.config.config;
    this.interactionPolicy = opts.interaction ?? cfg.interaction.default;
    const real = this.#deps.platform();
    // The background promise is kept by the SEAM, not by conditionals at the call sites: the
    // session simply never holds an adapter that can post an event or activate the app.
    this.platform = this.interactionPolicy === "background" ? backgroundAdapter(real) : real;
    // Permissions are read through the real adapter either way — `permissions` is a capability
    // read and the refusing wrapper passes it through, but being explicit here keeps the
    // preflight independent of which verbs the wrapper happens to allow.
    this.permissions = await checkPermissions(real.input, opts.allowDegradedCapture === true);
    // A session allowed to start without Screen Recording carries that fact for its whole life:
    // the tools read it instead of attempting a capture that cannot work.
    this.captureCapability = captureCapabilityOf(this.permissions);
    // Read once, best-effort: the key map is US/ANSI with no translation (main.swift `Keys.map`),
    // so a non-ANSI active layout is a standing fact worth surfacing, not a reason to refuse.
    this.keyboardLayout = await real.input.layout?.().catch((e: unknown) => {
      log.warn("keyboard layout read failed", { error: String(e) });
      return undefined;
    });
    this.keyboardLayoutWarning =
      this.keyboardLayout !== undefined && !this.keyboardLayout.isAnsiUs
        ? `active input source is "${this.keyboardLayout.inputSourceId}", not ANSI/US — keyboard_press may send the wrong character because the key map is positional, not layout-aware`
        : undefined;
    this.#port = opts.port ?? cfg.webdriver.port;
    const kind = opts.launch ?? "dev";
    const owners = this.#owners(kind);
    // BOTH paths validate the port owner. An explicit `mode:"attach"` used to short-circuit
    // this check entirely, so the session adopted whatever held the port — and then owned
    // it for teardown. `portOwnerIsOurs` throws PORT_IN_USE for any other owner (including
    // this same app built in another checkout) and returns false only when the port is free.
    const attach =
      opts.mode === "attach"
        ? await this.#attachToPortOwner(cfg.app.processPatterns, owners)
        : await portOwnerIsOurs(
            this.#port,
            cfg.app.processPatterns,
            owners,
            opts.reuseExisting === true,
            this.#deps.runner,
          );
    if (!attach && kind === "dev") await checkDevServerPort(cfg.devServer.port, this.#deps.runner);
    // Taken before anything is spawned, and only when we are about to spawn: an attached
    // session never kills, so it has nothing to spare anything from.
    if (!attach) await this.#snapshotPreexisting(kind);
    this.#launchKind = attach ? undefined : kind;
    return {
      spawn: !attach,
      kind,
      timeoutMs: attach || kind === "bundled" ? ATTACH_READY_TIMEOUT_MS : cfg.launch.dev.readyTimeoutMs,
    };
  }

  /**
   * `mode:"attach"` with nothing on the port is APP_NOT_RUNNING right here — otherwise the
   * start burns the full 60 s `waitForWebDriver` deadline before saying the same thing.
   * `reuseExisting` is implied: attaching IS the request to reuse.
   */
  async #attachToPortOwner(patterns: string[], owners: string[]): Promise<true> {
    const ours = await portOwnerIsOurs(this.#port, patterns, owners, true, this.#deps.runner);
    if (!ours) {
      throw new AgentError("APP_NOT_RUNNING", `nothing is serving WebDriver on port ${this.#port} to attach to`, {
        details: { port: this.#port, mode: "attach" },
      });
    }
    return true;
  }

  #spawnApp(opts: StartOptions, kind: "dev" | "bundled"): void {
    const spec = launchSpec(kind, this.config.config, this.config.root, this.#launcherLogPath(), {
      port: this.#port,
      appLogDir: join(this.journal.dir, "app-logs"),
      ...(opts.env === undefined ? {} : { extra: opts.env }),
    });
    log.info("launching app", { argv: spec.argv, cwd: spec.cwd, port: this.#port });
    this.#launcher = this.#deps.spawn(spec);
  }

  #awaitWebDriver(timeoutMs: number): Promise<void> {
    const launcher = this.#launcher;
    return waitForWebDriver({
      port: this.#port,
      timeoutMs,
      fetchFn: this.#deps.fetch,
      sleep: this.#deps.sleep,
      ...(launcher === undefined ? {} : { check: launcherAbortCheck(launcher, this.#launcherLogPath()) }),
    });
  }

  #resolveAppPid(): Promise<number> {
    return resolveListenerPid(this.#port, this.#deps.runner, this.#deps.sleep);
  }

  /** The window appears only after the webview loads, so this polls rather than reads once. */
  async #discoverWindow(pid: number): Promise<NativeWindowInfo> {
    const platform = this.requirePlatform();
    const deadline = Date.now() + WINDOW_WAIT_MS;
    for (;;) {
      const windows = await platform.windows.list(pid);
      if (windows.length > 0) return pickLargestWindow(windows, pid);
      if (Date.now() >= deadline) {
        throw new AgentError("WINDOW_NOT_FOUND", `pid ${pid} has no on-screen window after ${WINDOW_WAIT_MS}ms`, {
          details: { pid },
        });
      }
      await this.#deps.sleep(WINDOW_POLL_MS);
    }
  }

  async #failStart(cause: unknown): Promise<void> {
    const err = cause instanceof AgentError ? cause : undefined;
    const failure = {
      code: err?.code ?? "INTERNAL",
      message: cause instanceof Error ? cause.message : String(cause),
    };
    log.error("session start failed", failure);
    // A start that got as far as SPAWNING the app must not leave it running: the next
    // session_start would then refuse with PORT_IN_USE and the user would have to hunt it.
    // A failed ATTACH must do the opposite — the app was already there and is not ours to
    // kill, so a calibration failure never takes down the developer's window.
    // `forceKillAttached` is deliberately never passed here.
    try {
      await this.stop({ killApp: this.#launcher !== undefined });
    } catch (e) {
      log.warn("teardown after failed start was incomplete", { error: String(e) });
    }
    this.state = "Failed";
    this.failure = failure;
  }

  /**
   * Proves the process serving WebDriver is THIS checkout's app, built for agent testing.
   *
   * Two failure shapes, deliberately different:
   *
   *  - The handshake could not be COMPLETED — the command is not registered (an app built
   *    before this handshake existed), or nothing usable came back. That is a recorded
   *    WARNING, never a refusal: hard-failing here would brick every attach to an older
   *    binary, and the session is no worse identified than it was before the handshake.
   *  - The handshake ANSWERED and a field disagrees. That is a different app; nothing may
   *    be driven, so it is a hard APP_IDENTITY_MISMATCH.
   *
   * Any other invoke rejection is a genuine bridge fault and is rethrown untouched.
   */
  /**
   * Installs the mutation-revision and console probes, once per bridge.
   *
   * Every action's `settle()` and console-error accounting reads these, so they must exist
   * before the FIRST action rather than before the first `ui_snapshot` — nothing in the MCP
   * API requires a snapshot first, and without them `settle` compares -1 to -1 and returns
   * instantly having proved nothing.
   *
   * A failure here is recorded, not fatal: every native input path still works, and refusing
   * to start over a missing observer would trade a degraded session for no session at all.
   * `settle` warns on its own when it sees an uninstrumented page, so the degradation is
   * never silent.
   */
  async #instrument(bridge: SessionBridge): Promise<void> {
    this.instrumentWarning = undefined;
    try {
      const report = await ensureInstrumentation(bridge);
      log.debug("page instrumentation ready", { ...report });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      this.instrumentWarning =
        `page instrumentation could not be installed (${message}), so settle and console-error ` +
        "accounting are degraded for this session";
      log.warn("page instrumentation failed", { error: message });
    }
  }

  async #verifyIdentity(bridge: SessionBridge, pid: number): Promise<void> {
    this.identity = undefined;
    this.identityWarning = undefined;
    let reply: unknown;
    try {
      reply = await bridge.invoke<unknown>(IDENTITY_COMMAND, {}, { readOnly: true });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      if (!UNKNOWN_COMMAND.test(message)) throw e;
      this.identityWarning =
        `${IDENTITY_COMMAND} is not registered in the running app (${message}), ` +
        "so its identity is unverified — rebuild the app to gate this session on the handshake";
      log.warn("app identity handshake unavailable", { error: message });
      return;
    }
    if (reply === null || typeof reply !== "object") {
      // Indistinguishable from an absent command over this transport (a reply that was
      // never awaited reads as undefined), so it is a warning and never a verdict.
      this.identityWarning = `${IDENTITY_COMMAND} returned no object, so the app identity is unverified`;
      log.warn("app identity handshake returned nothing");
      return;
    }
    const identity = reply as AgentIdentity;
    const mismatch = identityMismatch(identity, pid, this.config.root);
    if (mismatch !== null) {
      throw new AgentError("APP_IDENTITY_MISMATCH", `the app serving WebDriver is not this checkout's: ${mismatch}`, {
        details: { reason: mismatch, identity, expectedPid: pid, root: this.config.root },
      });
    }
    this.identity = identity;
    log.info("app identity verified", { pid, bundleId: identity.bundleId, nonce: identity.sessionNonce });
  }

  // --- stop helpers --------------------------------------------------------

  async #dropBridge(): Promise<void> {
    const bridge = this.bridge;
    this.bridge = undefined;
    if (!bridge) return;
    await bridge.close().catch((e: unknown) => log.warn("bridge close failed", { error: String(e) }));
  }

  /**
   * Re-points ownership at the pid that now serves WebDriver, and DOWNGRADES to "attached"
   * when this session can no longer claim to have launched it.
   *
   * A `tauri dev` restart after a rebuild is still ours: the launcher is alive and the new app
   * is a child of its process group. But if the launcher has exited, whatever is serving the
   * port now was started by somebody else — typically the developer, by hand, inside the 60 s
   * reconnect window after quitting the app. Keeping `kind:"launched"` there would let
   * `session_stop` SIGTERM their app, which is the exact defect the ownership model exists to
   * prevent. Identity cannot catch it: the same checkout answers the handshake correctly.
   */
  #rebindOwnership(pid: number): void {
    if (this.#ownership === undefined) return;
    const stillOurs = this.#ownership.kind === "launched" && this.#launcher?.exited === false;
    if (this.#ownership.kind === "launched" && !stillOurs) {
      log.warn("the launcher is gone; the app that came back is no longer ours to kill", { pid });
    }
    this.#ownership = { kind: stillOurs ? "launched" : "attached", pid };
  }

  /** Checkout scoping for every ownership decision: the project root, plus the bundle when one is used. */
  #owners(kind?: "dev" | "bundled"): string[] {
    const owners = [this.config.root];
    if (kind === "bundled") {
      owners.push(resolveFromRoot(this.config.root, this.config.config.launch.bundled.appPath));
    }
    return owners;
  }

  async #sweep(killThem: boolean): Promise<Survivor[]> {
    return sweepSurvivors({
      patterns: this.config.config.app.processPatterns,
      owners: this.#owners(this.#launchKind),
      runner: this.#deps.runner,
      kill: this.#deps.kill,
      sleep: this.#deps.sleep,
      killThem,
      preexisting: this.#preexisting,
    });
  }

  /**
   * Snapshots the processes of this checkout that are ALREADY running, before anything is
   * spawned, so the teardown sweep can tell its own leftovers from the developer's work.
   *
   * The sweep matches on executable path plus checkout, never on who started a process, so a
   * hand-run `bun run tauri dev` from this same checkout satisfies every test it applies. Taken
   * at preflight because that is the last moment at which "already running" is unambiguous.
   */
  async #snapshotPreexisting(kind: "dev" | "bundled"): Promise<void> {
    try {
      const found = await findSurvivors(
        this.config.config.app.processPatterns,
        this.#owners(kind),
        this.#deps.runner,
      );
      this.#preexisting = new Set(found.map((s) => s.pid));
      if (this.#preexisting.size > 0) {
        log.info("processes of this checkout were already running; the sweep will spare them", {
          pids: [...this.#preexisting],
        });
      }
    } catch (e) {
      // A failed snapshot must not block the session — but it must also not license a kill, so
      // an empty set is NOT the fallback: keep whatever we had and say so.
      log.warn("could not snapshot pre-existing processes; the sweep stays conservative", {
        error: String(e),
      });
    }
  }

  #reset(): void {
    this.state = "Idle";
    // Cleared with the rest of the session: `session_status` after a stop must not report the
    // DEAD session's TCC grants as if they described a live one. The pre-start default is the
    // honest reading of a session that cannot photograph anything because it is not running.
    this.permissions = undefined;
    this.captureCapability = { available: false, authoritative: false };
    this.pid = undefined;
    this.windowId = undefined;
    // Window identity dies with the session: the CGWindowIDs in the table name windows of a
    // process that is going away, and the next session must re-correlate rather than inherit them.
    this.targetLabel = this.config.config.app.windowLabel;
    this.targetIsModal = false;
    this.windowTable = undefined;
    this.windowTableWarning = undefined;
    this.#windowState = undefined;
    this.geom = undefined;
    this.calibration = undefined;
    this.platform = undefined;
    this.lastSnapshot = undefined;
    this.identity = undefined;
    this.identityWarning = undefined;
    this.#ownership = undefined;
    this.#launcher = undefined;
    this.#launchKind = undefined;
    this.refs.clear();
  }

  // --- watchdog ------------------------------------------------------------

  #startWatchdog(): void {
    this.#stopWatchdog();
    const timer = setInterval(() => {
      if (this.state !== "Ready" || this.pid === undefined || this.#recovering) return;
      if (isAlive(this.pid, this.#deps.kill)) return;
      this.#recovering = true;
      void this.#recover().finally(() => {
        this.#recovering = false;
      });
    }, WATCHDOG_INTERVAL_MS);
    // An interval must never be the reason this process stays alive.
    timer.unref?.();
    this.#watchdog = timer;
  }

  #stopWatchdog(): void {
    if (this.#watchdog !== undefined) clearInterval(this.#watchdog);
    this.#watchdog = undefined;
  }

  /** The app died. It may be a dev-server rebuild restart, so give it 60 s to come back. */
  async #recover(): Promise<void> {
    log.warn("app process is gone; reconnecting", { pid: this.pid });
    this.state = "Reconnecting";
    await this.#dropBridge();
    try {
      await waitForWebDriver({
        port: this.#port,
        timeoutMs: RECONNECT_TIMEOUT_MS,
        fetchFn: this.#deps.fetch,
        sleep: this.#deps.sleep,
      });
      this.pid = await this.#resolveAppPid();
      this.#rebindOwnership(this.pid);
      this.bridge = await this.#deps.bridgeFactory(this.#port);
      // The app came back — possibly as a DIFFERENT binary (a rebuild from elsewhere, or
      // another process that grabbed the port). Re-prove it before driving it again.
      await this.#verifyIdentity(this.bridge, this.pid);
      await this.#instrument(this.bridge);
      this.windowId = (await this.#discoverWindow(this.pid)).windowId;
      this.sessionEpoch += 1;
      this.refs.clear();
      this.lastSnapshot = undefined;
      await this.calibrate();
      this.state = "Ready";
      log.info("session reconnected", { pid: this.pid, sessionEpoch: this.sessionEpoch });
    } catch (e) {
      this.state = "Failed";
      this.failure = {
        code: e instanceof AgentError ? e.code : "APP_NOT_RUNNING",
        message: e instanceof Error ? e.message : String(e),
      };
      log.error("reconnect failed", this.failure);
    }
  }

  // --- small accessors -----------------------------------------------------

  #launcherLogPath(): string {
    return this.journal.artifactPath("launcher.log");
  }

  #occlusion(): Rect[] {
    return this.config.config.nativeOcclusion.rects;
  }

  /** The live WebDriver bridge, or WEBDRIVER_UNAVAILABLE. */
  requireBridge(): SessionBridge {
    if (!this.bridge) throw new AgentError("WEBDRIVER_UNAVAILABLE", "no WebDriver bridge in this session");
    return this.bridge;
  }

  requirePlatform(): PlatformAdapter {
    if (!this.platform) throw new AgentError("INTERNAL", "no platform adapter in this session");
    return this.platform;
  }

  #requirePid(): number {
    if (this.pid === undefined) throw new AgentError("APP_NOT_RUNNING", "no app pid in this session");
    return this.pid;
  }

  /** The calibrated geometry every CSS→screen mapping depends on. */
  requireGeom(): WindowGeom {
    if (!this.geom) throw new AgentError("CALIBRATION_FAILED", "the session has no calibrated geometry");
    return this.geom;
  }
}
