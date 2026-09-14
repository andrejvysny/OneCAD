import { afterEach, describe, expect, test } from "bun:test";
import { symlinkSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { AgentError } from "../src/errors.ts";
import type { KeyboardLayoutInfo, Permissions, PlatformAdapter } from "../src/platform/adapter.ts";
import { hoverRan } from "../src/session/calibrate.ts";
import { parseConfig } from "../src/session/config.ts";
import type { LaunchHandle, LaunchSpec } from "../src/session/launch.ts";
import { SessionOrchestrator } from "../src/session/orchestrator.ts";
import type { Runner } from "../src/session/procs.ts";
import type { AgentIdentity, SessionBridge } from "../src/session/types.ts";
import { Journal } from "../src/trace/journal.ts";
import { FakeAx, axWindow } from "./fixtures/fakeAx.ts";

const APP_PID = 4242;
const OTHER_PID = 999;
const dirs: string[] = [];

afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

function tempDir(): string {
  const d = mkdtempSync(join(tmpdir(), "tauri-agent-session-"));
  dirs.push(d);
  return d;
}

interface World {
  root: string;
  listeners: Map<number, number[]>;
  commands: Map<number, string>;
  pgrep: Map<string, number[]>;
  dead: Set<number>;
  /** pids that ignore every signal, so the sweep must report them */
  stubborn: Set<number>;
  signals: Array<{ pid: number; sig: NodeJS.Signals | number }>;
  spawned: LaunchSpec[];
  launcherExited: boolean;
  /** `/status` polls, so a test can prove a refusal happened BEFORE the readiness wait */
  fetches: number;
  /** false models the app's window sitting on another Space: CGWindowList omits it on-screen. */
  onscreenWindows: boolean;
}

function world(root: string): World {
  return {
    root,
    listeners: new Map(),
    commands: new Map(),
    pgrep: new Map(),
    dead: new Set(),
    stubborn: new Set(),
    signals: [],
    spawned: [],
    launcherExited: false,
    fetches: 0,
    onscreenWindows: true,
  };
}

function runnerFor(w: World): Runner {
  return async (argv) => {
    const ok = (stdout: string): { code: number; stdout: string; stderr: string } => ({
      code: stdout.length > 0 ? 0 : 1,
      stdout,
      stderr: "",
    });
    if (argv[0] === "/usr/sbin/lsof") {
      const port = Number((argv[2] as string).replace("-iTCP:", ""));
      return ok((w.listeners.get(port) ?? []).join("\n"));
    }
    if (argv[0] === "/bin/ps") {
      const cmd = w.commands.get(Number(argv[4])) ?? "";
      // `-o comm=` is the EXECUTABLE path; `-o command=` is the full command line.
      return ok(argv[2] === "comm=" ? (cmd.trim().split(/\s+/)[0] ?? "") : cmd);
    }
    if (argv[0] === "/usr/bin/pgrep") return ok((w.pgrep.get(argv[2] as string) ?? []).join("\n"));
    throw new Error(`unexpected command ${argv.join(" ")}`);
  };
}

interface BridgeOpts {
  viewport?: boolean;
  noProbe?: boolean;
  /** Replaces the `agent_identity` reply; present-but-undefined models "returned nothing". */
  identity?: unknown;
  /** Makes `invoke` reject with this message, as a real bridge fault would. */
  invokeError?: string;
  /** Collects the names of page scripts the session ran, so a test can assert what was installed. */
  scripts?: string[];
  /** Makes the instrumentation install script reject, as a wedged bridge would. */
  instrumentError?: string;
}

/** What a `tauri-e2e` build of THIS checkout answers `agent_identity` with. */
function identityFor(root: string): AgentIdentity {
  return {
    runtime: "tauri",
    agentTesting: true,
    bundleId: "com.andrejvysny.onecad",
    pid: APP_PID,
    executable: `${root}/src-tauri/target/debug/onecad`,
    cargoManifestDir: `${root}/src-tauri`,
    sessionNonce: "0123456789abcdef",
  };
}

/** Only the verbs the orchestrator's calibration and handshake actually send. */
function bridgeFor(opts: BridgeOpts = {}, root = ""): SessionBridge {
  const geom = {
    innerPositionPx: { x: 200, y: 100 },
    innerSizePx: { width: 2400, height: 1600 },
    scaleFactor: 2,
    focused: true,
    dpr: 2,
    vvScale: 1,
    innerWidth: 1200,
    innerHeight: 800,
  };
  const rect = { x: 100, y: 40, width: 80, height: 24 };
  const scripts = opts.scripts ?? [];
  return {
    wedged: false,
    windowGeom: async () => geom,
    close: async () => undefined,
    invoke: (async () => {
      if (opts.invokeError !== undefined) throw new AgentError("INTERNAL", `invoke:agent_identity: ${opts.invokeError}`);
      return "identity" in opts ? opts.identity : identityFor(root);
    }) as SessionBridge["invoke"],
    execute: (async (name: string) => {
      if (name === "calibrate.pickProbe") {
        if (opts.noProbe === true) return { found: false, usedFallback: true, description: "", rect: null };
        return { found: true, usedFallback: false, description: "span[data-testid=\"document-title\"]", rect };
      }
      if (name === "calibrate.hover") return { connected: true, hit: true, hover: true, rect };
      // The webview half of the window-identity table: one window, whose outer position and
      // inner size in PHYSICAL px divide by the scale factor into the native bounds below.
      if (name === "window.table") {
        return [
          {
            label: "main",
            title: "OneCAD",
            outerPositionPx: { x: 200, y: 100 },
            innerSizePx: { width: 2400, height: 1600 },
            scaleFactor: 2,
          },
        ];
      }
      if (name === "window.focusLabel") return true;
      if (name === "calibrate.wheelArm") return { present: opts.viewport === true, rect: null };
      if (name === "calibrate.wheelRead") return null;
      if (name === "instrument.install") {
        scripts.push(name);
        if (opts.instrumentError !== undefined) throw new AgentError("INTERNAL", opts.instrumentError);
        return { revInstalled: true, consoleInstalled: true, rev: 0, consoleErrors: 0 };
      }
      throw new Error(`unexpected script ${name}`);
    }) as SessionBridge["execute"],
  };
}

const GRANTED: Permissions = { accessibility: true, screenRecording: true };
const ANSI_US_LAYOUT: KeyboardLayoutInfo = { inputSourceId: "com.apple.keylayout.US", isAnsiUs: true };

/**
 * Records the verbs that would touch the user's desktop — the two calibration probes post
 * `move` and `scroll`, and the start sequence calls `focus`. A background session must produce
 * an empty log, and the default session must NOT.
 */
function platformFor(
  perms: Permissions = GRANTED,
  layout: KeyboardLayoutInfo = ANSI_US_LAYOUT,
  nativeLog: string[] = [],
  onscreen: () => boolean = () => true,
): PlatformAdapter {
  const noop = async (): Promise<void> => undefined;
  // Accessibility agrees with both the native window below and the webview's `window.table`
  // reading, so calibration correlates label "main" to CGWindowID 7 exactly as it must live.
  const ax = new FakeAx();
  ax.windowList = [axWindow({ bounds: { x: 100, y: 50, width: 1200, height: 800 } })];
  return {
    name: "macos",
    input: {
      move: async () => {
        nativeLog.push("move");
      },
      down: noop,
      up: noop,
      click: noop,
      path: noop,
      scroll: async () => {
        nativeLog.push("scroll");
      },
      keyDown: noop,
      keyUp: noop,
      press: noop,
      type: noop,
      releaseAll: noop,
      cursor: async () => ({ x: 0, y: 0 }),
      permissions: async () => perms,
      layout: async () => layout,
      dispose: noop,
    },
    ax,
    windows: {
      // `all` is what a background session passes: it asks for windows whether or not they are on
      // the active Space. With `onscreen` false and no `all`, the list is empty — exactly what
      // CGWindowList reports for a window on another Space.
      list: async (_pid: number, opts?: { all?: boolean }) =>
        onscreen() || opts?.all === true
          ? [{ windowId: 7, layer: 0, bounds: { x: 100, y: 50, width: 1200, height: 800 }, onscreen: onscreen(), name: "OneCAD" }]
          : [],
      isFrontmost: async () => true,
      focus: async () => {
        nativeLog.push("focus");
        return true;
      },
    },
    capture: {
      window: async () => ({ width: 2400, height: 1600, pixelScale: 2 }),
      region: async () => ({ width: 10, height: 10, pixelScale: 2 }),
      screen: async () => ({ width: 10, height: 10, pixelScale: 2 }),
      preview: noop,
    },
  };
}

function makeSession(
  w: World,
  over: BridgeOpts = {},
  perms: Permissions = GRANTED,
  layout: KeyboardLayoutInfo = ANSI_US_LAYOUT,
  nativeLog: string[] = [],
  configOverrides: Record<string, unknown> = {},
): SessionOrchestrator {
  const journal = new Journal(join(w.root, "artifacts"));
  const config = {
    root: w.root,
    config: parseConfig(configOverrides, "<test>"),
    configPath: join(w.root, "cfg.json"),
  };
  return new SessionOrchestrator({
    sessionId: "s-test",
    journal,
    config,
    deps: {
      runner: runnerFor(w),
      platform: () => platformFor(perms, layout, nativeLog, () => w.onscreenWindows),
      bridgeFactory: async () => bridgeFor(over, w.root),
      fetch: async () => {
        w.fetches += 1;
        return { ok: w.listeners.has(4445) };
      },
      spawn: (spec: LaunchSpec): LaunchHandle => {
        w.spawned.push(spec);
        // The app binds the WebDriver port shortly after the launcher starts.
        if (!w.launcherExited) w.listeners.set(4445, [APP_PID]);
        const handle: LaunchHandle = {
          pid: 5000,
          pgid: 5000,
          get exited() {
            return w.launcherExited || w.dead.has(5000);
          },
          exitCode: w.launcherExited ? 1 : null,
          waitExit: async () => w.launcherExited || w.dead.has(5000),
        };
        return handle;
      },
      kill: (pid, sig) => {
        w.signals.push({ pid, sig });
        if (sig === 0) {
          if (w.dead.has(pid)) throw new Error("ESRCH");
          return;
        }
        if (!w.stubborn.has(Math.abs(pid))) w.dead.add(Math.abs(pid));
      },
      sleep: async () => undefined,
    },
  });
}

describe("SessionOrchestrator.start", () => {
  /**
   * The probes `settle()` and the console-error effect read used to be installed only as a side
   * effect of `ui_snapshot`. Nothing in the MCP API forces a snapshot first, so `session_start`
   * followed straight by a keyboard shortcut settled on `-1 === -1` and counted no console errors.
   */
  /**
   * Adversarial-review finding. `isInside` compared lexically resolved paths, so a checkout
   * reached through a symlink — `~/dev -> /Volumes/Data/dev`, or anything under `/tmp`, which
   * IS `/private/tmp` — made the app's compile-time `CARGO_MANIFEST_DIR` and the harness's own
   * root spell one directory two ways. Every `session_start` then hard-failed with
   * APP_IDENTITY_MISMATCH on a perfectly good checkout, with no override available.
   */
  test("a checkout reached through a symlink still passes the identity handshake", async () => {
    const real = tempDir();
    const link = join(tempDir(), "link-to-checkout");
    symlinkSync(real, link);
    const w = world(link);
    w.commands.set(APP_PID, `${link}/src-tauri/target/debug/onecad`);
    // The app reports the path it was BUILT at — the real one, not the symlinked spelling.
    const session = makeSession(w, { identity: { ...identityFor(real), pid: APP_PID } });

    const status = await session.start({ mode: "launch", launch: "dev" });

    expect(status.state).toBe("Ready");
    expect(status.identityWarning).toBeUndefined();
    expect(status.identity?.cargoManifestDir).toBe(`${real}/src-tauri`);
  });

  /**
   * Adversarial-review finding, verified by probe: `killLauncherGroup` ran unconditionally, and
   * the app lives in the launcher's process group because the launcher is spawned detached — so
   * `killApp:false` killed the very app it promised to leave alone. `src/mcp/server.ts` calls
   * exactly this on SIGTERM, believing it detaches.
   */
  /**
   * Adversarial-review finding. The sweep identifies a process by executable path plus checkout,
   * never by who started it, so a developer's own `bun run tauri dev` from this same checkout
   * matched every test it applies and was SIGKILLed — along with its worker — by an agent
   * `session_stop` that was only cleaning up after itself.
   */
  test("session_stop never kills a process of this checkout that predates the session", async () => {
    const w = world(tempDir());
    const DEV_PID = 7777;
    // The developer's own dev app, already running from this same checkout.
    w.commands.set(DEV_PID, `${w.root}/src-tauri/target/debug/onecad`);
    w.pgrep.set("target/debug/onecad", [DEV_PID]);
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w);

    await session.start({ mode: "launch", launch: "dev" });
    // Both now match the sweep's patterns and owners.
    w.pgrep.set("target/debug/onecad", [DEV_PID, APP_PID]);
    const status = await session.stop();

    // `sig: 0` is `kill(pid, 0)` — a liveness PROBE, not a signal — so it is not a kill.
    const realSignals = w.signals.filter((s) => s.sig !== 0);
    expect(realSignals.some((s) => s.pid === DEV_PID)).toBe(false);
    // Still reported, and flagged as pre-existing: "something of this checkout is running" is
    // true and worth saying, but it is not a failed teardown and must not raise STOP_INCOMPLETE.
    expect(status.teardown?.survivors).toEqual([
      expect.objectContaining({ pid: DEV_PID, preexisting: true }),
    ]);
  });

  test("stop({killApp:false}) on a LAUNCHED session really detaches: nothing is signalled", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w);
    await session.start({ mode: "launch", launch: "dev" });

    const status = await session.stop({ killApp: false });

    expect(w.signals).toEqual([]);
    expect(status.teardown?.detached).toBe(true);
  });

  test("stop() on a launched session still kills it, so the normal path is unchanged", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w);
    await session.start({ mode: "launch", launch: "dev" });

    await session.stop();

    expect(w.signals.length).toBeGreaterThan(0);
  });

  test("session_start instruments the page, so an action before any ui_snapshot is observable", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const scripts: string[] = [];
    const session = makeSession(w, { scripts });

    const status = await session.start({ mode: "launch", launch: "dev" });

    expect(status.state).toBe("Ready");
    expect(scripts).toContain("instrument.install");
    expect(status.instrumentWarning).toBeUndefined();
  });

  test("a failed install degrades the session with a warning instead of refusing to start", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w, { instrumentError: "bridge said no" });

    const status = await session.start({ mode: "launch", launch: "dev" });

    // Native input still works without the observer, so no session is strictly worse than a
    // degraded one — but the caller has to be told, and `settle` warns again on its own.
    expect(status.state).toBe("Ready");
    expect(status.instrumentWarning).toMatch(/bridge said no/);
  });

  test("launch:dev reaches Ready and calibrates; the wheel probe is skipped without a viewport", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w);

    const status = await session.start({ mode: "launch", launch: "dev" });

    expect(status.state).toBe("Ready");
    expect(status.pid).toBe(APP_PID);
    expect(status.windowId).toBe(7);
    expect(status.calibration?.ok).toBe(true);
    const hover = status.calibration?.hoverProbe;
    // The default (foreground) session still RUNS the hover probe — the skip variant here
    // would mean the background branch leaked into the unchanged path.
    expect(hover !== undefined && hoverRan(hover) && hover.hover).toBe(true);
    expect(status.calibration?.wheelProbe).toEqual({ skipped: "no viewport", posted: false });
    expect(w.spawned).toHaveLength(1);
    expect(w.spawned[0]?.argv).toEqual(["bun", "run", "tauri:agent"]);
    expect(w.spawned[0]?.env.TAURI_WEBDRIVER_PORT).toBe("4445");
    expect(w.spawned[0]?.env.ONECAD_LOG_DIR).toContain("app-logs");
    expect(status.input?.keyboardLayout).toEqual(ANSI_US_LAYOUT);
    expect(status.keyboardLayoutWarning).toBeUndefined();
    await session.stop();
  });

  test("a non-ANSI/US layout produces the session-start warning; an ANSI one does not", async () => {
    const nonAnsi: KeyboardLayoutInfo = { inputSourceId: "com.apple.keylayout.German", isAnsiUs: false };
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w, {}, GRANTED, nonAnsi);

    const status = await session.start({ mode: "launch", launch: "dev" });

    expect(status.input?.keyboardLayout).toEqual(nonAnsi);
    expect(status.keyboardLayoutWarning).toContain("com.apple.keylayout.German");
    expect(status.keyboardLayoutWarning).toContain("keyboard_press");
    await session.stop();
  });

  test("an occupied WebDriver port owned by our app attaches when reuseExisting is set", async () => {
    const w = world(tempDir());
    w.listeners.set(4445, [APP_PID]);
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w);

    const status = await session.start({ mode: "launch", launch: "dev", reuseExisting: true });

    expect(status.state).toBe("Ready");
    expect(status.pid).toBe(APP_PID);
    // Attaching must never spawn a second app on top of the running one.
    expect(w.spawned).toHaveLength(0);
    await session.stop();
  });

  test("an occupied WebDriver port without reuseExisting is PORT_IN_USE, never a kill", async () => {
    const w = world(tempDir());
    w.listeners.set(4445, [OTHER_PID]);
    w.commands.set(OTHER_PID, "/usr/bin/python3 -m http.server 4445");
    const session = makeSession(w);

    await expect(session.start({ mode: "launch", launch: "dev" })).rejects.toMatchObject({
      code: "PORT_IN_USE",
    });
    expect(session.status().state).toBe("Failed");
    expect(w.signals).toEqual([]);
  });

  test("a busy Vite dev port is DEV_SERVER_PORT_IN_USE before anything is spawned", async () => {
    const w = world(tempDir());
    w.listeners.set(1420, [OTHER_PID]);
    w.commands.set(OTHER_PID, "node vite");
    const session = makeSession(w);

    await expect(session.start({ mode: "launch", launch: "dev" })).rejects.toMatchObject({
      code: "DEV_SERVER_PORT_IN_USE",
    });
    expect(w.spawned).toHaveLength(0);
  });

  test("a launcher that exits early fails as APP_NOT_RUNNING with the log tail", async () => {
    const w = world(tempDir());
    w.launcherExited = true;
    const session = makeSession(w);
    const logPath = join(w.root, "artifacts", "launcher.log");
    const lines = Array.from({ length: 50 }, (_, i) => `line ${i + 1}`);
    writeFileSync(logPath, `${lines.join("\n")}\n`, "utf8");

    const err = (await session.start({ mode: "launch", launch: "dev" }).catch((e: unknown) => e)) as {
      code: string;
      details: { tail: string[] };
    };
    expect(err.code).toBe("APP_NOT_RUNNING");
    expect(err.details.tail).toHaveLength(40);
    expect(err.details.tail[39]).toBe("line 50");
  });
});

/**
 * A session may start without Screen Recording, but it may not pretend otherwise: every later
 * decision about whether to attempt or trust a picture reads `status().capture`.
 */
describe("SessionOrchestrator capture capability", () => {
  test("the grant present makes captures available and authoritative", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w);

    const status = await session.start({ mode: "launch", launch: "dev" });

    expect(status.capture).toEqual({ available: true, authoritative: true });
    await session.stop();
  });

  test("allowDegradedCapture starts the session and records that it cannot photograph", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w, {}, { accessibility: true, screenRecording: false });

    const status = await session.start({ mode: "launch", launch: "dev", allowDegradedCapture: true });

    expect(status.state).toBe("Ready");
    expect(status.capture).toEqual({
      available: false,
      authoritative: false,
      reason: "SCREEN_RECORDING_PERMISSION_DENIED",
    });
    expect(session.status().capture.available).toBe(false);
    await session.stop();
  });

  test("without the grant and without the flag the session refuses to start at all", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w, {}, { accessibility: true, screenRecording: false });

    await expect(session.start({ mode: "launch", launch: "dev" })).rejects.toMatchObject({
      code: "SCREEN_CAPTURE_PERMISSION_DENIED",
    });
    expect(w.spawned).toEqual([]);
  });
});

/**
 * Ownership is the P0 invariant: a session that ATTACHED to an app somebody else started
 * must never signal it. Every test here asserts on `w.signals`, which records every call
 * the orchestrator makes to `kill` — an empty array is the proof.
 */
describe("SessionOrchestrator attach ownership", () => {
  function attachWorld(): World {
    const w = world(tempDir());
    w.listeners.set(4445, [APP_PID]);
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    return w;
  }

  test("mode:attach records attached ownership and stop() signals nothing", async () => {
    const w = attachWorld();
    const session = makeSession(w);

    const started = await session.start({ mode: "attach" });
    expect(started.state).toBe("Ready");
    expect(started.ownership).toEqual({ kind: "attached", pid: APP_PID });
    expect(w.spawned).toHaveLength(0);

    const status = await session.stop();

    expect(status.state).toBe("Idle");
    expect(w.signals).toEqual([]);
    expect(w.dead.has(APP_PID)).toBe(false);
    expect(status.teardown?.detached).toBe(true);
    expect(status.teardown?.launched).toBe(false);
  });

  test("a failed attach never kills the app it attached to", async () => {
    const w = attachWorld();
    // The sweep would find the app; on an attached session it must only REPORT it.
    w.pgrep.set("target/debug/onecad", [APP_PID]);
    const session = makeSession(w, { noProbe: true });

    const err = (await session.start({ mode: "attach" }).catch((e: unknown) => e)) as { code: string };

    expect(err.code).toBe("CALIBRATION_FAILED");
    expect(session.status().state).toBe("Failed");
    // #failStart ran a full teardown — and signalled nothing, because we did not launch it.
    expect(w.signals).toEqual([]);
    expect(w.dead.has(APP_PID)).toBe(false);
  });

  test("stop({forceKillAttached:true}) is the one deliberate way to terminate an attached app", async () => {
    const w = attachWorld();
    const session = makeSession(w);
    await session.start({ mode: "attach" });

    await session.stop({ forceKillAttached: true });

    expect(w.signals).toEqual([{ pid: APP_PID, sig: "SIGTERM" }]);
  });

  test("attaching to the same app from ANOTHER checkout is PORT_IN_USE, never adopted", async () => {
    const w = world(tempDir());
    w.listeners.set(4445, [OTHER_PID]);
    // Same binary, same pattern — a different project root. Pattern-only ownership said yes.
    w.commands.set(OTHER_PID, "/Users/someone-else/OneCAD/src-tauri/target/debug/onecad");
    const session = makeSession(w);

    const err = (await session.start({ mode: "attach" }).catch((e: unknown) => e)) as {
      code: string;
      details: { matchesApp: boolean };
    };

    expect(err.code).toBe("PORT_IN_USE");
    expect(err.details.matchesApp).toBe(false);
    expect(w.signals).toEqual([]);
    expect(w.spawned).toHaveLength(0);
  });

  test("attaching with nothing on the port fails fast, before the readiness wait", async () => {
    const w = world(tempDir());
    const session = makeSession(w);

    const err = (await session.start({ mode: "attach" }).catch((e: unknown) => e)) as { code: string };

    expect(err.code).toBe("APP_NOT_RUNNING");
    // Nothing was spawned and /status was never polled: the refusal beat the 60 s wait.
    expect(w.spawned).toHaveLength(0);
    expect(w.fetches).toBe(0);
  });
});

describe("SessionOrchestrator identity handshake", () => {
  /** An attachable world plus a session whose bridge answers `agent_identity` per `over(root)`. */
  function attachSession(over: (root: string) => BridgeOpts = () => ({})): {
    w: World;
    session: SessionOrchestrator;
  } {
    const w = world(tempDir());
    w.listeners.set(4445, [APP_PID]);
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    return { w, session: makeSession(w, over(w.root)) };
  }

  test("a verified handshake travels in the status", async () => {
    const { w, session } = attachSession();
    const status = await session.start({ mode: "attach" });
    expect(status.identity?.cargoManifestDir).toBe(`${w.root}/src-tauri`);
    expect(status.identityWarning).toBeUndefined();
  });

  test("agentTesting:false is a hard APP_IDENTITY_MISMATCH", async () => {
    const { w, session } = attachSession((root) => ({ identity: { ...identityFor(root), agentTesting: false } }));

    const err = (await session.start({ mode: "attach" }).catch((e: unknown) => e)) as {
      code: string;
      details: { reason: string };
    };
    expect(err.code).toBe("APP_IDENTITY_MISMATCH");
    expect(err.details.reason).toContain("tauri-e2e");
    expect(w.signals).toEqual([]);
  });

  test("a foreign cargoManifestDir is a hard APP_IDENTITY_MISMATCH", async () => {
    // A SIBLING of the root: `<root>-other` passes a naive string-prefix test, and must
    // not pass a path-resolved one.
    const { session } = attachSession((root) => ({
      identity: { ...identityFor(root), cargoManifestDir: `${root}-other/src-tauri` },
    }));

    const err = (await session.start({ mode: "attach" }).catch((e: unknown) => e)) as {
      code: string;
      details: { reason: string };
    };
    expect(err.code).toBe("APP_IDENTITY_MISMATCH");
    expect(err.details.reason).toContain("not inside");
  });

  test("a pid that is not the port owner is a hard APP_IDENTITY_MISMATCH", async () => {
    const { session } = attachSession((root) => ({ identity: { ...identityFor(root), pid: OTHER_PID } }));
    await expect(session.start({ mode: "attach" })).rejects.toMatchObject({ code: "APP_IDENTITY_MISMATCH" });
  });

  test("a reply that is not an object is unverified, not a mismatch", async () => {
    const { session } = attachSession(() => ({ identity: null }));

    const status = await session.start({ mode: "attach" });

    expect(status.state).toBe("Ready");
    expect(status.identityWarning).toContain("no object");
  });

  test("an app built before the handshake existed still starts, with a warning", async () => {
    const { session } = attachSession(() => ({ invokeError: "Command agent_identity not found" }));

    const status = await session.start({ mode: "attach" });

    expect(status.state).toBe("Ready");
    expect(status.identity).toBeUndefined();
    expect(status.identityWarning).toContain("agent_identity");
  });

  test("any other invoke failure is a real bridge fault and is not swallowed", async () => {
    const { session } = attachSession(() => ({ invokeError: "invalid session id" }));
    await expect(session.start({ mode: "attach" })).rejects.toMatchObject({ code: "INTERNAL" });
  });
});

describe("SessionOrchestrator start failure", () => {
  test("a calibration failure after launch tears the launched app down and ends Failed", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    w.pgrep.set("target/debug/onecad", [APP_PID]);
    const session = makeSession(w, { noProbe: true });

    const err = (await session.start({ mode: "launch", launch: "dev" }).catch((e: unknown) => e)) as { code: string };

    expect(err.code).toBe("CALIBRATION_FAILED");
    expect(session.status().state).toBe("Failed");
    // The launched process group and the app pid were both signalled — nothing is left behind.
    expect(w.signals).toContainEqual({ pid: -5000, sig: "SIGTERM" });
    expect(w.signals.some((s) => s.pid === APP_PID && s.sig !== 0)).toBe(true);
  });
});

describe("SessionOrchestrator.stop", () => {
  test("signals the launcher group, then kills only survivors owned by this project", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    w.commands.set(OTHER_PID, "/Users/someone-else/OneCAD/src-tauri/target/debug/onecad");
    const session = makeSession(w);
    await session.start({ mode: "launch", launch: "dev" });
    w.pgrep.set("target/debug/onecad", [APP_PID, OTHER_PID]);

    const status = await session.stop();

    expect(status.state).toBe("Idle");
    // The measured teardown travels with the status so a report can cite it.
    expect(status.teardown?.survivors).toEqual([]);
    expect(status.teardown?.launched).toBe(true);
    expect(Object.keys(status.teardown?.portsFree ?? {}).sort()).toEqual(["1420", "4445"]);
    expect(w.signals).toContainEqual({ pid: -5000, sig: "SIGTERM" });
    expect(w.signals).toContainEqual({ pid: APP_PID, sig: "SIGKILL" });
    // The other checkout's app matches the pattern but is never signalled.
    expect(w.signals.some((s) => s.pid === OTHER_PID)).toBe(false);
  });

  test("a launched session still SIGTERMs the app it started", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w);
    const started = await session.start({ mode: "launch", launch: "dev" });
    expect(started.ownership).toEqual({ kind: "launched", pid: APP_PID });

    const status = await session.stop();

    expect(w.signals).toContainEqual({ pid: APP_PID, sig: "SIGTERM" });
    expect(status.teardown?.detached).toBe(false);
    expect(status.teardown?.launched).toBe(true);
  });

  test("reports STOP_INCOMPLETE when a matching process refuses to die", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    w.stubborn.add(APP_PID);
    const session = makeSession(w);
    await session.start({ mode: "launch", launch: "dev" });
    w.pgrep.set("target/debug/onecad", [APP_PID]);

    const err = (await session.stop().catch((e: unknown) => e)) as {
      code: string;
      details: { survivors: Array<{ pid: number; command: string }> };
    };
    expect(err.code).toBe("STOP_INCOMPLETE");
    expect(err.details.survivors).toEqual([{ pid: APP_PID, command: `${w.root}/src-tauri/target/debug/onecad` }]);
    // The session still resets, so a later session_start is not blocked by our own state.
    expect(session.status().state).toBe("Idle");
  });

  test("stopping an untouched session is a no-op", async () => {
    const w = world(tempDir());
    const session = makeSession(w);
    expect((await session.stop()).state).toBe("Idle");
    expect(w.signals).toEqual([]);
  });
});

/**
 * The background interaction policy at the session level.
 *
 * The other background tests drive the tools; these drive `start()` itself, which is where the
 * two calibration probes and the one `windows.focus` live. Both groups matter: the tools are
 * where a caller meets the policy, and `start()` is where it would be broken silently, because
 * a probe that posts a stray cursor move produces no error at all — just a pointer that jumped
 * out of the user's hand.
 */
describe("interaction policy", () => {
  test("the DEFAULT session is unchanged: it focuses and runs both probes", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const log: string[] = [];
    const session = makeSession(w, {}, GRANTED, ANSI_US_LAYOUT, log);

    const status = await session.start({ mode: "launch", launch: "dev" });

    expect(status.interaction).toBe("foreground");
    expect(status.foregroundStolenAtLaunch).toBeUndefined();
    expect(log).toContain("focus");
    // The hover probe really moved the cursor. This is the assertion that fails first if the
    // background branch ever leaks into the default path.
    expect(log).toContain("move");
    const entries = await journalTools(session);
    expect(entries).toContain("calibration.hoverProbe");
    await session.stop();
  });

  test("a background session starts Ready without focusing or posting a single event", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const log: string[] = [];
    const session = makeSession(w, {}, GRANTED, ANSI_US_LAYOUT, log);

    const status = await session.start({ mode: "launch", launch: "dev", interaction: "background" });

    expect(status.state).toBe("Ready");
    expect(status.interaction).toBe("background");
    // Nothing was focused, moved or scrolled — the whole promise, measured.
    expect(log).toEqual([]);
    // Calibration still happened; it is the PASSIVE half, and says so rather than going quiet.
    expect(status.calibration?.ok).toBe(true);
    expect(status.windowId).toBe(7);
    const hover = status.calibration?.hoverProbe;
    expect(hover !== undefined && !hoverRan(hover)).toBe(true);
    expect((hover as { skipped: string }).skipped).toContain("background");
    expect((status.calibration?.wheelProbe as { skipped: string }).skipped).toContain("background");
    // No probe journal entries, because no probe ran.
    expect(await journalTools(session)).not.toContain("calibration.hoverProbe");
    await session.stop();
  });

  test("a background LAUNCH reports the one activation it could not avoid", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w);
    const status = await session.start({ mode: "launch", launch: "dev", interaction: "background" });
    // Not a harness choice: tao's window_activation_hack makes every visible window key at
    // applicationDidFinishLaunching. Saying so is the only honest option.
    expect(status.foregroundStolenAtLaunch).toBe(true);
    await session.stop();
  });

  test("a background ATTACH steals nothing at all, because nothing is launched", async () => {
    const w = world(tempDir());
    w.listeners.set(4445, [APP_PID]);
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const log: string[] = [];
    const session = makeSession(w, {}, GRANTED, ANSI_US_LAYOUT, log);

    const status = await session.start({ mode: "attach", interaction: "background" });

    expect(status.foregroundStolenAtLaunch).toBeUndefined();
    expect(log).toEqual([]);
    expect(w.spawned).toHaveLength(0);
    await session.stop();
  });

  test("config's interaction.default decides when session_start does not", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const log: string[] = [];
    const session = makeSession(w, {}, GRANTED, ANSI_US_LAYOUT, log, {
      interaction: { default: "background" },
    });

    const status = await session.start({ mode: "launch", launch: "dev" });

    expect(status.interaction).toBe("background");
    expect(log).toEqual([]);
    await session.stop();
  });

  test("an explicit session_start argument beats the config default, in both directions", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const log: string[] = [];
    const session = makeSession(w, {}, GRANTED, ANSI_US_LAYOUT, log, {
      interaction: { default: "background" },
    });

    const status = await session.start({ mode: "launch", launch: "dev", interaction: "foreground" });

    expect(status.interaction).toBe("foreground");
    expect(log).toContain("focus");
    await session.stop();
  });
});

/** Tool names recorded in the session journal, in order. */
async function journalTools(session: SessionOrchestrator): Promise<string[]> {
  const path = join(session.journal.dir, "journal.jsonl");
  const text = await Bun.file(path)
    .text()
    .catch(() => "");
  return text
    .split("\n")
    .filter((l) => l.trim() !== "")
    .map((l) => (JSON.parse(l) as { tool?: string }).tool ?? "");
}

/**
 * The seam itself, asserted on the ADAPTER a real session ends up holding.
 *
 * `background.test.ts` proves the tools route away from native input, and the refusing wrappers are
 * unit-tested in isolation — but neither covers the JOIN, which is the single line in `#preflight`
 * that installs the wrapper. A mutation run proved the gap: replacing that line with
 * `this.platform = real` left 129 policy tests passing, because the tool tests use a FakeSession
 * whose `requirePlatform()` returns the raw fake, and the start test's probes are skipped by the
 * policy branches regardless of which adapter is installed.
 *
 * So these reach past the tools and ask the session's own adapter directly. They are the tests that
 * fail if anyone ever removes the wrapping.
 */
describe("the refusing adapter is actually installed", () => {
  async function started(policy: "foreground" | "background"): Promise<{
    session: SessionOrchestrator;
    log: string[];
  }> {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const log: string[] = [];
    const session = makeSession(w, {}, GRANTED, ANSI_US_LAYOUT, log);
    await session.start({ mode: "launch", launch: "dev", interaction: policy });
    return { session, log };
  }

  test("a background session's own adapter refuses input and focus", async () => {
    const { session, log } = await started("background");
    const platform = session.requirePlatform();

    const move = await platform.input.move({ x: 1, y: 1 }).then(
      () => "(did not throw)",
      (e: unknown) => (e instanceof AgentError ? e.code : String(e)),
    );
    const focus = await platform.windows.focus(APP_PID).then(
      () => "(did not throw)",
      (e: unknown) => (e instanceof AgentError ? e.code : String(e)),
    );

    expect(move).toBe("BACKGROUND_CAPABILITY_UNAVAILABLE");
    expect(focus).toBe("BACKGROUND_CAPABILITY_UNAVAILABLE");
    // And the refusals reached no real verb: the world records `move`/`scroll`/`focus`.
    expect(log).toEqual([]);
    await session.stop();
  });

  test("the reads a background session still needs are NOT refused", async () => {
    const { session } = await started("background");
    const platform = session.requirePlatform();
    // Capture, window enumeration and the frontmost read all have to keep working — a background
    // session still screenshots the window and still reports what it can see.
    expect(await platform.windows.list(APP_PID)).toHaveLength(1);
    expect(await platform.input.cursor()).toEqual({ x: 0, y: 0 });
    expect((await platform.input.permissions()).accessibility).toBe(true);
    await session.stop();
  });

  /**
   * `window_focus`'s refusal must stay retry-safe.
   *
   * `focusWindow` wraps its body in a catch that stamps `details.raised = true`, and the tool turns
   * that flag into `delivery.inputCompleted` / `mayHaveSideEffects` / `retrySafe:false`. The
   * activation call used to sit INSIDE that try, so a background session's seam — which throws
   * before raising anything — produced an envelope claiming a delivered, side-effecting action for
   * a refusal that touched nothing, in the exact field a caller's retry logic reads. The repo's own
   * `smoke-actions.ts` aborts a run on that.
   */
  test("window_focus in background refuses without claiming it raised anything", async () => {
    const { session, log } = await started("background");

    const err = await session.focusWindow("main").then(
      () => null,
      (e: unknown) => e,
    );

    expect(err).toBeInstanceOf(AgentError);
    const agentError = err as AgentError;
    expect(agentError.code).toBe("BACKGROUND_CAPABILITY_UNAVAILABLE");
    // The flag the delivery block is built from must be absent: nothing was raised.
    expect((agentError.details as { raised?: boolean } | undefined)?.raised).toBeUndefined();
    expect(log).toEqual([]);
    await session.stop();
  });

  /**
   * The advertised use case, which used to dead-end.
   *
   * A background session never focuses, so its window is routinely NOT on the active Space — the
   * user is working in another one, which is the whole point. `windows.list(pid)` defaults to
   * on-screen only, so every recalibration saw an empty list and threw WINDOW_NOT_FOUND, taking
   * out `ui_snapshot`, `ui_screenshot`, `window_list` and every webview action at once. The session
   * could not dig itself out either, because `window_focus` also refuses. Background now lists
   * `{all: true}`; foreground keeps the stricter on-screen list, which is correct there because
   * calibration runs immediately after the app was brought forward.
   */
  test("a background session survives its window leaving the active Space", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const log: string[] = [];
    const session = makeSession(w, {}, GRANTED, ANSI_US_LAYOUT, log);
    await session.start({ mode: "launch", launch: "dev", interaction: "background" });

    // Model the Space switch: the window still exists, it is simply not on screen.
    w.onscreenWindows = false;

    // The read every query tool funnels through.
    await session.ensureCalibrated();
    expect(session.status().state).toBe("Ready");
    expect(session.windowId).toBe(7);
    // And it got there without touching anything.
    expect(log).toEqual([]);
    await session.stop();
  });

  test("a foreground session's adapter is the real one, with nothing wrapped away", async () => {
    const { session, log } = await started("foreground");
    const platform = session.requirePlatform();
    await platform.input.move({ x: 1, y: 1 });
    expect(await platform.windows.focus(APP_PID)).toBe(true);
    // The real verbs ran. `focus` also appears from the start sequence itself.
    expect(log).toContain("move");
    expect(log).toContain("focus");
    await session.stop();
  });
});
