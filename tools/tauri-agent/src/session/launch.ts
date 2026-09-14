/**
 * Launching the app under test and waiting for its WebDriver endpoint.
 *
 * The child is spawned DETACHED so it leads its own process group: `stop()` signals
 * `-pgid`, which reaches `tauri dev`, the cargo/vite children it spawned and the app
 * itself. Without the new group a SIGTERM to the launcher leaves the real app running
 * and holding both ports.
 */
import { createWriteStream, existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { spawn as nodeSpawn } from "node:child_process";
import { basename, join } from "node:path";
import { AgentError } from "../errors.ts";
import { log } from "../log.ts";
import type { AgentConfig } from "./config.ts";
import { resolveFromRoot } from "./config.ts";

export interface LaunchSpec {
  argv: string[];
  cwd: string;
  env: Record<string, string>;
  logPath: string;
}

export interface LaunchHandle {
  readonly pid: number;
  /** process group id; equals `pid` because the child is detached */
  readonly pgid: number;
  readonly exited: boolean;
  readonly exitCode: number | null;
  waitExit(ms: number): Promise<boolean>;
}

export type Spawner = (spec: LaunchSpec) => LaunchHandle;

class ChildLaunchHandle implements LaunchHandle {
  readonly pid: number;
  #exited = false;
  #exitCode: number | null = null;
  #done: Promise<void>;

  constructor(spec: LaunchSpec) {
    const child = nodeSpawn(spec.argv[0] as string, spec.argv.slice(1), {
      cwd: spec.cwd,
      env: spec.env,
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    if (child.pid === undefined) {
      throw new AgentError("APP_NOT_RUNNING", `could not spawn ${spec.argv.join(" ")}`, {
        details: { argv: spec.argv, cwd: spec.cwd },
      });
    }
    this.pid = child.pid;
    const logStream = createWriteStream(spec.logPath, { flags: "a" });
    child.stdout?.pipe(logStream, { end: false });
    child.stderr?.pipe(logStream, { end: false });
    this.#done = new Promise<void>((resolve) => {
      child.on("exit", (code) => {
        this.#exited = true;
        this.#exitCode = code;
        logStream.end();
        resolve();
      });
      child.on("error", (err) => {
        log.warn("launcher process error", { error: String(err) });
        this.#exited = true;
        resolve();
      });
    });
  }

  get pgid(): number {
    return this.pid;
  }

  get exited(): boolean {
    return this.#exited;
  }

  get exitCode(): number | null {
    return this.#exitCode;
  }

  async waitExit(ms: number): Promise<boolean> {
    if (this.#exited) return true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<void>((resolve) => {
      timer = setTimeout(resolve, ms);
    });
    try {
      await Promise.race([this.#done, timeout]);
    } finally {
      clearTimeout(timer);
    }
    return this.#exited;
  }
}

export const childProcessSpawner: Spawner = (spec) => new ChildLaunchHandle(spec);

function stringEnv(extra: Record<string, string>): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [k, v] of Object.entries(process.env)) if (v !== undefined) env[k] = v;
  return { ...env, ...extra };
}

/**
 * `<name>.app` is a bundle directory; the executable inside it is what must be spawned
 * (matching e2e-tauri/wdio.conf.ts). A plain path is used as-is.
 */
export function bundledExecutable(appPath: string): string {
  if (!appPath.endsWith(".app")) return appPath;
  return join(appPath, "Contents", "MacOS", basename(appPath, ".app"));
}

export interface LaunchEnvOpts {
  port: number;
  appLogDir: string;
  extra?: Record<string, string>;
}

export function launchSpec(
  kind: "dev" | "bundled",
  cfg: AgentConfig,
  root: string,
  logPath: string,
  envOpts: LaunchEnvOpts,
): LaunchSpec {
  const env = stringEnv({
    TAURI_WEBDRIVER_PORT: String(envOpts.port),
    ONECAD_LOG_DIR: envOpts.appLogDir,
    ...(envOpts.extra ?? {}),
  });
  if (kind === "dev") {
    return { argv: cfg.launch.dev.command, cwd: root, env, logPath };
  }
  const appPath = resolveFromRoot(root, cfg.launch.bundled.appPath);
  const exe = bundledExecutable(appPath);
  if (!existsSync(exe)) {
    throw new AgentError("APP_NOT_RUNNING", `bundled app executable not found at ${exe}`, {
      remediation:
        "Build the bundle first: `bun run tauri build --features tauri-e2e --config src-tauri/tauri.e2e.conf.json --bundles app`, or point launch.bundled.appPath at an existing .app.",
      details: { appPath, executable: exe },
    });
  }
  return { argv: [exe], cwd: root, env, logPath };
}

export type FetchFn = (url: string) => Promise<{ ok: boolean }>;

export const httpFetch: FetchFn = async (url) => {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(2_000) });
    return { ok: res.ok };
  } catch {
    return { ok: false };
  }
};

/** Last `lines` lines of a file; empty when it does not exist yet. */
export async function tailFile(path: string, lines: number): Promise<string[]> {
  try {
    const text = await readFile(path, "utf8");
    const all = text.split("\n").filter((l) => l.length > 0);
    return all.slice(Math.max(0, all.length - lines));
  } catch {
    return [];
  }
}

export interface ReadinessOpts {
  port: number;
  timeoutMs: number;
  fetchFn: FetchFn;
  sleep: (ms: number) => Promise<void>;
  intervalMs?: number;
  /** Called each tick; throw to abort early (used when the launcher dies). */
  check?: () => Promise<void>;
}

/** Polls `/status` until the app answers, the deadline passes, or `check` throws. */
export async function waitForWebDriver(opts: ReadinessOpts): Promise<void> {
  const url = `http://127.0.0.1:${opts.port}/status`;
  const interval = opts.intervalMs ?? 500;
  const deadline = Date.now() + opts.timeoutMs;
  for (;;) {
    if (opts.check) await opts.check();
    const res = await opts.fetchFn(url);
    if (res.ok) return;
    if (Date.now() >= deadline) {
      throw new AgentError(
        "WEBDRIVER_UNAVAILABLE",
        `no WebDriver /status on 127.0.0.1:${opts.port} within ${opts.timeoutMs}ms`,
        { details: { url, timeoutMs: opts.timeoutMs } },
      );
    }
    await opts.sleep(interval);
  }
}
