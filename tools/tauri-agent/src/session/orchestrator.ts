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
import { join } from "node:path";
import { AgentError } from "../errors.ts";
import { log } from "../log.ts";
import type { Rect, WindowGeom } from "../geometry/types.ts";
import type { NativeWindowInfo, NativeWindows, Permissions, PlatformAdapter } from "../platform/adapter.ts";
import { createPlatformAdapter } from "../platform/adapter.ts";
import { Bridge } from "../semantic/webdriver.ts";
import type { Snapshot } from "../semantic/snapshot.ts";
import type { Journal } from "../trace/journal.ts";
import {
  buildGeom,
  buildReport,
  geomChecks,
  geomDiffers,
  hoverProbe,
  pickLargestWindow,
  wheelProbe,
} from "./calibrate.ts";
import type { CalibrationReport, ProbeDeps } from "./calibrate.ts";
import type {
  SessionBridge,
  SessionDeps,
  SessionState,
  SessionStatus,
  TeardownReport,
  StartOptions,
  StopOptions,
  RefStore,
} from "./types.ts";
import type { ResolvedConfig } from "./config.ts";
import { resolveFromRoot } from "./config.ts";
import type { LaunchHandle } from "./launch.ts";
import { childProcessSpawner, httpFetch, launchSpec, waitForWebDriver } from "./launch.ts";
import { isAlive, listeningPids, signal, spawnRunner } from "./procs.ts";
import {
  checkDevServerPort,
  checkPermissions,
  launcherAbortCheck,
  portOwnerIsOurs,
  resolveListenerPid,
} from "./startup.ts";
import type { Survivor } from "./teardown.ts";
import { killLauncherGroup, sweepSurvivors } from "./teardown.ts";

export type * from "./types.ts";

const ATTACH_READY_TIMEOUT_MS = 60_000;
const WINDOW_WAIT_MS = 30_000;
const WINDOW_POLL_MS = 500;
const RECONNECT_TIMEOUT_MS = 60_000;
const WATCHDOG_INTERVAL_MS = 2_000;

export class SessionOrchestrator {
  state: SessionState = "Idle";
  bridge?: SessionBridge;
  platform?: PlatformAdapter;
  geom?: WindowGeom;
  pid?: number;
  windowId?: number;
  sessionEpoch = 0;
  wheelLinesPerNotch: number;
  permissions?: Permissions;
  calibration?: CalibrationReport;
  /** Last snapshot taken by ui_snapshot; `mode:"diff"` compares against it. */
  lastSnapshot?: Snapshot;
  readonly sessionId: string;
  readonly journal: Journal;
  readonly refs: RefStore = new Map();
  readonly config: ResolvedConfig;
  failure?: { code: string; message: string };
  teardown?: TeardownReport;

  readonly #deps: Required<SessionDeps>;
  #launcher?: LaunchHandle;
  #port: number;
  #launchKind?: "dev" | "bundled";
  #watchdog?: ReturnType<typeof setInterval>;
  #recovering = false;

  constructor(opts: { sessionId: string; journal: Journal; config: ResolvedConfig; deps?: SessionDeps }) {
    this.sessionId = opts.sessionId;
    this.journal = opts.journal;
    this.config = opts.config;
    this.#port = opts.config.config.webdriver.port;
    this.wheelLinesPerNotch = opts.config.config.input.wheelLinesPerNotch;
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
      webdriverPort: this.#port,
      devServerPort: this.config.config.devServer.port,
      bridgeConnected: this.bridge !== undefined,
      platformReady: this.platform !== undefined,
      calibrated: this.geom !== undefined && this.calibration?.ok === true,
      artifactsDir: this.journal.dir,
      sessionEpoch: this.sessionEpoch,
      launched: this.#launcher !== undefined,
      wheelLinesPerNotch: this.wheelLinesPerNotch,
      ...(this.permissions === undefined ? {} : { permissions: this.permissions }),
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
   * Refuses unless the app window is frontmost. Native input goes to whatever is in front,
   * so every verb that posts an event calls this first.
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
        details: { pid, windowId: this.windowId },
      });
    }
  }

  /** Focus the app if something else is in front; WINDOW_NOT_FOREGROUND only if that fails. */
  async assertFrontmostOrFocus(): Promise<void> {
    const windows = this.requirePlatform().windows as NativeWindows & {
      isFrontmost?: (pid: number) => Promise<boolean>;
    };
    const pid = this.#requirePid();
    if (windows.isFrontmost && (await windows.isFrontmost(pid))) return;
    if (await windows.focus(pid)) return;
    throw new AgentError("WINDOW_NOT_FOREGROUND", `the app (pid ${pid}) could not be brought to the front`, {
      details: { pid, windowId: this.windowId },
    });
  }

  /** Every acting tool calls this: Ready, and the mapping still describes the window. */
  async ensureCalibrated(): Promise<void> {
    this.requireReady();
    if (await this.geometryChanged()) {
      log.info("window geometry changed; recalibrating");
      // The hover probe posts real input and reads `:hover`, which WebKit only applies to the
      // active window; a query tool may run while another app is in front, so bring ours back.
      await this.assertFrontmostOrFocus();
      await this.calibrate();
    }
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
      this.state = "WaitingForBridge";
      this.bridge = await this.#deps.bridgeFactory(this.#port);
      this.state = "WaitingForWindow";
      const win = await this.#discoverWindow(this.pid);
      this.windowId = win.windowId;
      // Never calibrate against a background window: the hover and wheel probes post real
      // native input, which the frontmost application would receive instead.
      if (!(await this.requirePlatform().windows.focus(this.pid))) {
        throw new AgentError("FOCUS_FAILED", `the app (pid ${this.pid}) did not become frontmost`, {
          details: { pid: this.pid, windowId: win.windowId },
        });
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
    const win = await pickLargestWindow(await platform.windows.list(pid), pid);
    const geom = buildGeom(reading, win);
    const checks = geomChecks(reading, geom);
    if (Object.values(checks).some((ok) => !ok)) {
      throw new AgentError("CALIBRATION_FAILED", "the window geometry checks disagree", {
        details: { checks, reading, nativeBounds: win.bounds },
      });
    }
    this.geom = geom;
    this.windowId = win.windowId;
    const probes: ProbeDeps = {
      bridge,
      input: platform.input,
      geom,
      occlusion: this.#occlusion(),
      sleep: this.#deps.sleep,
      refocus: () => platform.windows.focus(pid).then(() => undefined),
    };
    const hover = await hoverProbe(probes, this.config.config.calibration.probeTestId);
    const wheel = await wheelProbe(probes, this.wheelLinesPerNotch);
    if ("ok" in wheel && wheel.ok) this.wheelLinesPerNotch = wheel.linesPerNotch;
    this.calibration = buildReport(reading, win, checks, hover, wheel);
    return this.calibration;
  }

  /** Re-runs only the wheel probe, for tools that reach a viewport after calibration skipped it. */
  async ensureWheelProbe(): Promise<CalibrationReport["wheelProbe"]> {
    const cal = this.calibration;
    if (cal && !("skipped" in cal.wheelProbe)) return cal.wheelProbe;
    const probes: ProbeDeps = {
      bridge: this.requireBridge(),
      input: this.requirePlatform().input,
      geom: this.requireGeom(),
      occlusion: this.#occlusion(),
      sleep: this.#deps.sleep,
      refocus: () => this.requirePlatform().windows.focus(this.#requirePid()).then(() => undefined),
    };
    const wheel = await wheelProbe(probes, this.wheelLinesPerNotch);
    if ("ok" in wheel && wheel.ok) this.wheelLinesPerNotch = wheel.linesPerNotch;
    if (cal) cal.wheelProbe = wheel;
    return wheel;
  }

  /** True when the window moved, resized or changed scale since the last calibration. */
  async geometryChanged(): Promise<boolean> {
    if (!this.geom || !this.bridge || !this.platform || this.pid === undefined) return true;
    const windows = await this.platform.windows.list(this.pid);
    // A momentarily empty on-screen list (the app's Space is not active) is not proof the geometry
    // changed, but it does mean we cannot confirm it — treat as "recalibrate", which runs after the
    // focus step and will then see the window, rather than throwing WINDOW_NOT_FOUND here.
    if (windows.length === 0) return true;
    const reading = await this.bridge.windowGeom();
    return geomDiffers(reading, pickLargestWindow(windows, this.pid), this.geom);
  }

  /** Rebuilds the WebDriver session after a BRIDGE_WEDGED; refs from before are void. */
  async reconnectBridge(): Promise<void> {
    const port = this.#port;
    await this.#dropBridge();
    this.bridge = await this.#deps.bridgeFactory(port);
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
    let survivors: Survivor[];
    try {
      await this.platform?.input.releaseAll().catch(() => undefined);
      await this.#dropBridge();
      if (this.#launcher) await killLauncherGroup(this.#launcher, this.#deps.kill);
      if (opts.killApp !== false && this.pid !== undefined) {
        signal(this.pid, "SIGTERM", this.#deps.kill);
      }
      survivors = await this.#sweep(opts.killApp !== false);
      await this.platform?.input.dispose().catch(() => undefined);
    } finally {
      // A session that failed to tear down cleanly must still return to Idle, or every
      // later session_start would be refused by our own bookkeeping.
      this.#reset();
    }
    this.teardown = await this.#teardownReport(survivors, launched);
    if (survivors.length > 0) {
      throw new AgentError("STOP_INCOMPLETE", `${survivors.length} process(es) survived session_stop`, {
        details: { survivors, teardown: this.teardown },
      });
    }
    return this.status();
  }

  /** The evidence a report cites for "nothing of ours is still running". */
  async #teardownReport(survivors: Survivor[], launched: boolean): Promise<TeardownReport> {
    const cfg = this.config.config;
    const ports = [cfg.webdriver.port, cfg.devServer.port];
    const portsFree: Record<string, boolean> = {};
    for (const port of ports) {
      const pids = await listeningPids(port, this.#deps.runner).catch(() => [] as number[]);
      portsFree[String(port)] = pids.length === 0;
    }
    return { survivors, portsFree, launched };
  }

  // --- start helpers -------------------------------------------------------

  async #preflight(opts: StartOptions): Promise<{ spawn: boolean; kind: "dev" | "bundled"; timeoutMs: number }> {
    const cfg = this.config.config;
    this.platform = this.#deps.platform();
    this.permissions = await checkPermissions(this.platform.input, opts.allowDegradedCapture === true);
    this.#port = opts.port ?? cfg.webdriver.port;
    const kind = opts.launch ?? "dev";
    const attach =
      opts.mode === "attach" ||
      (await portOwnerIsOurs(this.#port, cfg.app.processPatterns, opts.reuseExisting === true, this.#deps.runner));
    if (!attach && kind === "dev") await checkDevServerPort(cfg.devServer.port, this.#deps.runner);
    this.#launchKind = attach ? undefined : kind;
    return {
      spawn: !attach,
      kind,
      timeoutMs: attach || kind === "bundled" ? ATTACH_READY_TIMEOUT_MS : cfg.launch.dev.readyTimeoutMs,
    };
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
    // A start that got as far as spawning the app must not leave it running: the next
    // session_start would then refuse with PORT_IN_USE and the user would have to hunt it.
    try {
      await this.stop({ killApp: true });
    } catch (e) {
      log.warn("teardown after failed start was incomplete", { error: String(e) });
    }
    this.state = "Failed";
    this.failure = failure;
  }

  // --- stop helpers --------------------------------------------------------

  async #dropBridge(): Promise<void> {
    const bridge = this.bridge;
    this.bridge = undefined;
    if (!bridge) return;
    await bridge.close().catch((e: unknown) => log.warn("bridge close failed", { error: String(e) }));
  }

  async #sweep(killThem: boolean): Promise<Survivor[]> {
    const owners = [this.config.root];
    if (this.#launchKind === "bundled") {
      owners.push(resolveFromRoot(this.config.root, this.config.config.launch.bundled.appPath));
    }
    return sweepSurvivors({
      patterns: this.config.config.app.processPatterns,
      owners,
      runner: this.#deps.runner,
      kill: this.#deps.kill,
      sleep: this.#deps.sleep,
      killThem,
    });
  }

  #reset(): void {
    this.state = "Idle";
    this.pid = undefined;
    this.windowId = undefined;
    this.geom = undefined;
    this.calibration = undefined;
    this.platform = undefined;
    this.lastSnapshot = undefined;
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
      this.bridge = await this.#deps.bridgeFactory(this.#port);
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
