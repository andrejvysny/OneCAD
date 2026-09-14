import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { PlatformAdapter } from "../src/platform/adapter.ts";
import { parseConfig } from "../src/session/config.ts";
import type { LaunchHandle, LaunchSpec } from "../src/session/launch.ts";
import { SessionOrchestrator } from "../src/session/orchestrator.ts";
import type { Runner } from "../src/session/procs.ts";
import type { SessionBridge } from "../src/session/types.ts";
import { Journal } from "../src/trace/journal.ts";

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
    if (argv[0] === "/bin/ps") return ok(w.commands.get(Number(argv[4])) ?? "");
    if (argv[0] === "/usr/bin/pgrep") return ok((w.pgrep.get(argv[2] as string) ?? []).join("\n"));
    throw new Error(`unexpected command ${argv.join(" ")}`);
  };
}

/** Only the verbs the orchestrator's calibration actually sends. */
function bridgeFor(opts: { viewport?: boolean; noProbe?: boolean } = {}): SessionBridge {
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
  return {
    wedged: false,
    windowGeom: async () => geom,
    close: async () => undefined,
    invoke: async () => undefined as never,
    execute: (async (name: string) => {
      if (name === "calibrate.pickProbe") {
        if (opts.noProbe === true) return { found: false, usedFallback: true, description: "", rect: null };
        return { found: true, usedFallback: false, description: "span[data-testid=\"document-title\"]", rect };
      }
      if (name === "calibrate.hover") return { connected: true, hit: true, hover: true, rect };
      if (name === "calibrate.wheelArm") return { present: opts.viewport === true, rect: null };
      if (name === "calibrate.wheelRead") return null;
      throw new Error(`unexpected script ${name}`);
    }) as SessionBridge["execute"],
  };
}

function platformFor(): PlatformAdapter {
  const noop = async (): Promise<void> => undefined;
  return {
    name: "macos",
    input: {
      move: noop,
      down: noop,
      up: noop,
      click: noop,
      path: noop,
      scroll: noop,
      keyDown: noop,
      keyUp: noop,
      press: noop,
      type: noop,
      releaseAll: noop,
      cursor: async () => ({ x: 0, y: 0 }),
      permissions: async () => ({ accessibility: true, screenRecording: true }),
      dispose: noop,
    },
    windows: {
      list: async () => [
        { windowId: 7, layer: 0, bounds: { x: 100, y: 50, width: 1200, height: 800 }, onscreen: true, name: "OneCAD" },
      ],
      isFrontmost: async () => true,
      focus: async () => true,
    },
    capture: {
      window: async () => ({ width: 2400, height: 1600, pixelScale: 2 }),
      region: async () => ({ width: 10, height: 10, pixelScale: 2 }),
      screen: async () => ({ width: 10, height: 10, pixelScale: 2 }),
      preview: noop,
    },
  };
}

function makeSession(w: World, over: { viewport?: boolean; noProbe?: boolean } = {}): SessionOrchestrator {
  const journal = new Journal(join(w.root, "artifacts"));
  const config = { root: w.root, config: parseConfig({}, "<test>"), configPath: join(w.root, "cfg.json") };
  return new SessionOrchestrator({
    sessionId: "s-test",
    journal,
    config,
    deps: {
      runner: runnerFor(w),
      platform: platformFor,
      bridgeFactory: async () => bridgeFor(over),
      fetch: async () => ({ ok: w.listeners.has(4445) }),
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
  test("launch:dev reaches Ready and calibrates; the wheel probe is skipped without a viewport", async () => {
    const w = world(tempDir());
    w.commands.set(APP_PID, `${w.root}/src-tauri/target/debug/onecad`);
    const session = makeSession(w);

    const status = await session.start({ mode: "launch", launch: "dev" });

    expect(status.state).toBe("Ready");
    expect(status.pid).toBe(APP_PID);
    expect(status.windowId).toBe(7);
    expect(status.calibration?.ok).toBe(true);
    expect(status.calibration?.hoverProbe.hover).toBe(true);
    expect(status.calibration?.wheelProbe).toEqual({ skipped: "no viewport" });
    expect(w.spawned).toHaveLength(1);
    expect(w.spawned[0]?.argv).toEqual(["bun", "run", "tauri:agent"]);
    expect(w.spawned[0]?.env.TAURI_WEBDRIVER_PORT).toBe("4445");
    expect(w.spawned[0]?.env.ONECAD_LOG_DIR).toContain("app-logs");
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
