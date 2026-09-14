/**
 * macOS native input: a long-lived Swift helper driven over JSON lines.
 *
 * One helper process per agent process (`getHelperClient()`), shared with `MacWindows`.
 * Replies are correlated by request id, and verbs run one at a time in a FIFO because the
 * helper answers one at a time — a verb's timeout starts when it reaches the head, so queueing
 * behind a 10s drag is not reported as that verb timing out.
 *
 * Every verb has a timeout; a timeout or an unexpected helper exit restarts the helper and
 * immediately drains held input (`release_all` with `osState`), so a wedged drag can never leave
 * a button or modifier stuck down at the OS level. On the normal path a release is TRACKED-only:
 * the OS report of held buttons includes the user's own hand on the mouse.
 *
 * `mods` are physical ModKeys. The schema-level `Primary` alias is translated to
 * `Command` by the caller (the MCP tool layer), never here.
 */
import type { ErrorCode } from "../../errors.ts";
import { AgentError } from "../../errors.ts";
import type { Pt } from "../../geometry/types.ts";
import { log } from "../../log.ts";
import type { ModKey, MouseButton, NativeInput, Permissions } from "../adapter.ts";
import { ensureHelper } from "./build.ts";

const DEFAULT_TIMEOUT_MS = 5_000;
/** How long a disposed helper gets to act on EOF (release what it holds) before SIGTERM. */
const SHUTDOWN_GRACE_MS = 300;
/** setTimeout truncates past this, turning a huge budget into an instant bogus timeout. */
const MAX_TIMER_MS = 2_147_483_647;

/** Mirrors of the helper's own clamps (`main.swift`). Out of range is a caller bug. */
const MAX_DURATION_MS = 60_000;
const MAX_STEPS = 2_000;
const MAX_SCROLL_LINES = 100;

/** Helper error codes that carry meaning on the wire; anything else is an agent bug. */
const CODE_MAP: Record<string, ErrorCode> = {
  NATIVE_INPUT_PERMISSION_DENIED: "NATIVE_INPUT_PERMISSION_DENIED",
  WINDOW_NOT_FOUND: "WINDOW_NOT_FOUND",
  FOCUS_FAILED: "FOCUS_FAILED",
  HELPER_FAILED: "HELPER_FAILED",
  // The helper refused the request; that is the caller's argument, not an agent fault.
  INVALID_ARGS: "INVALID_TARGET",
  INVALID_VERB: "INVALID_TARGET",
};

export type HelperResult = Record<string, unknown>;

interface Pending {
  resolve: (v: HelperResult) => void;
  reject: (e: unknown) => void;
  timer: ReturnType<typeof setTimeout>;
  verb: string;
}

type HelperProc = Bun.Subprocess<"pipe", "pipe", "pipe">;

/** Argv of the helper process; injectable so tests can drive a fake over the same protocol. */
export type HelperArgv = () => Promise<string[]>;

export class HelperClient {
  private readonly argv: HelperArgv;
  private proc: HelperProc | null = null;
  private starting: Promise<void> | null = null;
  private pending = new Map<number, Pending>();
  private nextId = 1;
  private disposed = false;
  /** set when a helper died on its own; the replacement must drain OS-held input first */
  private needsDrain = false;
  /**
   * The helper answers one verb at a time, so a verb behind a 10s drag would otherwise spend
   * its whole budget queueing and report ACTION_TIMEOUT for someone else's slowness. The
   * timer starts when the verb reaches the head of this chain.
   */
  private queue: Promise<unknown> = Promise.resolve();

  constructor(argv: HelperArgv = defaultArgv) {
    this.argv = argv;
  }

  /** Sends one verb, restarting + draining held input if the helper wedges or dies. */
  request(verb: string, args: Record<string, unknown> = {}, timeoutMs?: number): Promise<HelperResult> {
    return this.enqueue(() => this.run(verb, args, timeoutMs));
  }

  /**
   * Releases what the helper itself is holding. Best-effort: never throws, so it is safe in a
   * `finally`. It deliberately does NOT release OS-reported state — that includes the user's
   * own hand on the mouse.
   */
  async releaseAll(): Promise<HelperResult> {
    try {
      return await this.enqueue(async () => {
        await this.ensureStarted();
        await this.drainIfNeeded();
        return this.send("release_all", {}, DEFAULT_TIMEOUT_MS);
      });
    } catch (e) {
      log.warn("release_all failed", { error: String(e) });
      return {};
    }
  }

  /** Releases tracked input, then lets the helper see EOF before it is killed. */
  async dispose(): Promise<void> {
    if (this.disposed) return;
    // A verb still in flight must not hold the shutdown hostage: the helper's own EOF release
    // covers whatever this one misses.
    if (this.proc) await Promise.race([this.releaseAll(), Bun.sleep(DEFAULT_TIMEOUT_MS)]);
    this.disposed = true;
    await this.shutdown(new AgentError("HELPER_FAILED", "helper disposed"));
  }

  private enqueue<T>(job: () => Promise<T>): Promise<T> {
    const run = this.queue.then(job, job);
    this.queue = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  private async run(verb: string, args: Record<string, unknown>, timeoutMs?: number): Promise<HelperResult> {
    await this.ensureStarted();
    await this.drainIfNeeded();
    try {
      return await this.send(verb, args, timeoutMs ?? verbTimeout(verb, args));
    } catch (e) {
      if (isRecoverable(e)) await this.recover();
      throw e;
    }
  }

  private async ensureStarted(): Promise<void> {
    if (this.disposed) throw new AgentError("HELPER_FAILED", "helper client was disposed");
    if (this.proc) return;
    this.starting ??= this.start().finally(() => {
      this.starting = null;
    });
    await this.starting;
  }

  private async start(): Promise<void> {
    const argv = await this.argv();
    const proc = Bun.spawn(argv, { stdin: "pipe", stdout: "pipe", stderr: "pipe" });
    const bin = argv[0];
    this.proc = proc;
    void this.pumpStdout(proc);
    void this.pumpStderr(proc);
    if (this.disposed) {
      // dispose() ran while this spawn was in flight; nothing will ever read this process.
      this.proc = null;
      proc.kill();
      return;
    }
    void proc.exited.then((code) => {
      if (this.proc !== proc) return;
      this.proc = null;
      this.needsDrain = true;
      this.rejectAll(new AgentError("HELPER_FAILED", `native input helper exited (code ${code})`));
    });
    log.debug("native input helper started", { bin, pid: proc.pid });
  }

  private send(verb: string, args: Record<string, unknown>, timeoutMs: number): Promise<HelperResult> {
    const proc = this.proc;
    if (!proc) return Promise.reject(new AgentError("HELPER_FAILED", "native input helper is not running"));
    const id = this.nextId++;
    // setTimeout truncates a non-finite or oversized delay to ~1ms, which would report a
    // timeout for a verb that was never given any time at all.
    const budget = Number.isFinite(timeoutMs) ? Math.min(Math.max(timeoutMs, 0), MAX_TIMER_MS) : DEFAULT_TIMEOUT_MS;
    return new Promise<HelperResult>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(
          new AgentError("ACTION_TIMEOUT", `native input verb '${verb}' did not answer within ${budget}ms`, {
            details: { verb, timeoutMs: budget },
          }),
        );
      }, budget);
      this.pending.set(id, { resolve, reject, timer, verb });
      try {
        proc.stdin.write(`${JSON.stringify({ id, verb, ...args })}\n`);
        proc.stdin.flush();
      } catch (cause) {
        // A closed pipe must settle this request here; nothing will ever answer it.
        this.pending.delete(id);
        clearTimeout(timer);
        reject(
          new AgentError("HELPER_FAILED", `native input verb '${verb}' could not be written to the helper`, {
            details: { verb },
            cause,
          }),
        );
      }
    });
  }

  /** After an unplanned helper death, the replacement releases whatever the OS still holds. */
  private async drainIfNeeded(): Promise<void> {
    if (!this.needsDrain) return;
    this.needsDrain = false;
    try {
      await this.send("release_all", { osState: true }, DEFAULT_TIMEOUT_MS);
    } catch (e) {
      log.warn("post-restart release_all failed", { error: String(e) });
    }
  }

  /**
   * Restart, then drain OS-level held buttons/modifiers before any further verb runs.
   * This is the one path allowed to release OS state: the tracked state died with the helper,
   * so the OS report is all that is left to say what a wedged drag left held.
   */
  private async recover(): Promise<void> {
    if (this.disposed) return;
    log.warn("restarting native input helper");
    this.kill(new AgentError("HELPER_FAILED", "native input helper restarted"));
    this.needsDrain = false;
    try {
      await this.ensureStarted();
      await this.send("release_all", { osState: true }, DEFAULT_TIMEOUT_MS);
    } catch (e) {
      log.error("native input helper restart failed", { error: String(e) });
    }
  }

  /** Immediate: for a helper that is wedged or already gone, where EOF would never be read. */
  private kill(reason: AgentError): void {
    const proc = this.proc;
    this.proc = null;
    this.rejectAll(reason);
    if (proc) {
      try {
        proc.kill();
      } catch {
        /* already gone */
      }
    }
  }

  /** Graceful: EOF first, so a helper that still holds something runs its own release. */
  private async shutdown(reason: AgentError): Promise<void> {
    const proc = this.proc;
    this.proc = null;
    this.rejectAll(reason);
    if (!proc) return;
    try {
      proc.stdin.end();
    } catch {
      /* already closed */
    }
    const exited = await Promise.race([proc.exited, Bun.sleep(SHUTDOWN_GRACE_MS).then(() => null)]);
    if (exited === null) {
      try {
        proc.kill();
      } catch {
        /* already gone */
      }
    }
  }

  private rejectAll(reason: AgentError): void {
    for (const [, p] of this.pending) {
      clearTimeout(p.timer);
      p.reject(reason);
    }
    this.pending.clear();
  }

  private async pumpStdout(proc: HelperProc): Promise<void> {
    const reader = proc.stdout.getReader();
    const decoder = new TextDecoder();
    let buf = "";
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        let nl: number;
        while ((nl = buf.indexOf("\n")) >= 0) {
          const line = buf.slice(0, nl);
          buf = buf.slice(nl + 1);
          if (line.trim()) this.onLine(line);
        }
      }
    } catch (e) {
      log.debug("helper stdout pump ended", { error: String(e) });
    }
  }

  private async pumpStderr(proc: HelperProc): Promise<void> {
    const reader = proc.stderr.getReader();
    const decoder = new TextDecoder();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        const text = decoder.decode(value, { stream: true }).trimEnd();
        if (text) log.debug("helper", { stderr: text });
      }
    } catch {
      /* helper gone */
    }
  }

  private onLine(line: string): void {
    let reply: { id?: number; ok?: boolean; result?: HelperResult; code?: string; message?: string };
    try {
      reply = JSON.parse(line) as typeof reply;
    } catch {
      log.warn("unparseable helper reply", { line: line.slice(0, 200) });
      return;
    }
    const entry = typeof reply.id === "number" ? this.pending.get(reply.id) : undefined;
    if (!entry || typeof reply.id !== "number") {
      log.warn("helper reply with no pending request", { id: reply.id });
      return;
    }
    this.pending.delete(reply.id);
    clearTimeout(entry.timer);
    if (reply.ok) entry.resolve(reply.result ?? {});
    else entry.reject(mapHelperError(entry.verb, reply.code, reply.message));
  }
}

async function defaultArgv(): Promise<string[]> {
  return [await ensureHelper()];
}

/** The budget a verb needs on top of the base timeout is the motion it was asked to perform. */
export function verbTimeout(verb: string, args: Record<string, unknown>): number {
  if (verb !== "move" && verb !== "path") return DEFAULT_TIMEOUT_MS;
  const motion = Number(args.durationMs ?? 0) + Number(args.holdMs ?? 0) + Number(args.dwellMs ?? 0);
  return DEFAULT_TIMEOUT_MS + (Number.isFinite(motion) ? Math.max(0, motion) : 0);
}

function isRecoverable(e: unknown): boolean {
  return e instanceof AgentError && (e.code === "ACTION_TIMEOUT" || e.code === "HELPER_FAILED");
}

export function mapHelperError(verb: string, code: string | undefined, message: string | undefined): AgentError {
  const mapped: ErrorCode = (code === undefined ? undefined : CODE_MAP[code]) ?? "INTERNAL";
  return new AgentError(mapped, `native input verb '${verb}' failed: ${message ?? code ?? "unknown error"}`, {
    details: { verb, helperCode: code },
  });
}

let shared: HelperClient | null = null;

/** The single helper process for this agent process, shared by MacInput and MacWindows. */
export function getHelperClient(): HelperClient {
  shared ??= new HelperClient();
  return shared;
}

/** Kills the shared helper and clears the singleton, so a later adapter starts a fresh one. */
export async function disposeHelperClient(): Promise<void> {
  const client = shared;
  shared = null;
  if (client) await client.dispose();
}

function mods(m?: ModKey[]): Record<string, unknown> {
  return m && m.length > 0 ? { mods: m } : {};
}

/**
 * Mirror of the helper's clamps. The helper would clamp these itself, but a caller asking for
 * 1e20ms of easing has a bug the agent should report rather than silently reinterpret.
 */
function bounded(value: number, lo: number, hi: number, field: string): number {
  if (!Number.isFinite(value) || value < lo || value > hi) {
    throw new AgentError("INVALID_TARGET", `'${field}' must be a finite number in [${lo}, ${hi}], got ${value}`, {
      details: { field, value, min: lo, max: hi },
    });
  }
  return value;
}

function durationMs(value: number | undefined, fallback: number, field: string): number {
  return bounded(value ?? fallback, 0, MAX_DURATION_MS, field);
}

export class MacInput implements NativeInput {
  private readonly client: HelperClient;

  constructor(client: HelperClient = getHelperClient()) {
    this.client = client;
  }

  /**
   * Any verb that can leave a button down must not reject without releasing it: the helper's
   * own `defer` covers a throw inside the verb, this covers a timeout, a crash, or a dead pipe
   * where no reply ever comes back.
   */
  private async holding<T>(run: () => Promise<T>): Promise<T> {
    try {
      return await run();
    } catch (e) {
      await this.client.releaseAll();
      throw e;
    }
  }

  /** A move is a DRAG while a button is held, so it carries the same release obligation. */
  async move(p: Pt, opts?: { durationMs?: number; steps?: number; mods?: ModKey[] }): Promise<void> {
    const args = {
      x: p.x,
      y: p.y,
      durationMs: durationMs(opts?.durationMs, 0, "durationMs"),
      ...(opts?.steps === undefined ? {} : { steps: bounded(opts.steps, 1, MAX_STEPS, "steps") }),
      ...mods(opts?.mods),
    };
    await this.holding(() => this.client.request("move", args));
  }

  async down(button: MouseButton, p: Pt, opts?: { clickState?: 1 | 2 | 3; mods?: ModKey[] }): Promise<void> {
    await this.holding(() =>
      this.client.request("down", {
        button,
        x: p.x,
        y: p.y,
        clickState: bounded(opts?.clickState ?? 1, 1, 3, "clickState"),
        ...mods(opts?.mods),
      }),
    );
  }

  /** An up that failed left the button down; releasing tracked state is the second attempt. */
  async up(button: MouseButton, p: Pt, opts?: { clickState?: 1 | 2 | 3; mods?: ModKey[] }): Promise<void> {
    const args = {
      button,
      x: p.x,
      y: p.y,
      clickState: bounded(opts?.clickState ?? 1, 1, 3, "clickState"),
      ...mods(opts?.mods),
    };
    await this.holding(() => this.client.request("up", args));
  }

  async click(
    button: MouseButton,
    p: Pt,
    opts?: { count?: 1 | 2 | 3; intervalMs?: number; mods?: ModKey[] },
  ): Promise<void> {
    const count = bounded(opts?.count ?? 1, 1, 3, "count");
    const intervalMs = durationMs(opts?.intervalMs, 80, "intervalMs");
    await this.holding(() =>
      this.client.request(
        "click",
        { button, x: p.x, y: p.y, count, intervalMs, ...mods(opts?.mods) },
        DEFAULT_TIMEOUT_MS + count * intervalMs,
      ),
    );
  }

  async path(
    button: MouseButton,
    points: Pt[],
    opts?: { durationMs?: number; holdMs?: number; dwellMs?: number; mods?: ModKey[] },
  ): Promise<void> {
    const duration = durationMs(opts?.durationMs, 0, "durationMs");
    const holdMs = durationMs(opts?.holdMs, 0, "holdMs");
    const dwellMs = durationMs(opts?.dwellMs, 0, "dwellMs");
    await this.holding(() =>
      this.client.request(
        "path",
        {
          button,
          points: points.map((q) => ({ x: q.x, y: q.y })),
          durationMs: duration,
          holdMs,
          dwellMs,
          ...mods(opts?.mods),
        },
        DEFAULT_TIMEOUT_MS + duration + holdMs + dwellMs,
      ),
    );
  }

  async scroll(p: Pt, delta: { dy: number; dx?: number }, opts?: { mods?: ModKey[] }): Promise<void> {
    await this.client.request("scroll", {
      x: p.x,
      y: p.y,
      dy: bounded(Math.trunc(delta.dy), -MAX_SCROLL_LINES, MAX_SCROLL_LINES, "dy"),
      dx: bounded(Math.trunc(delta.dx ?? 0), -MAX_SCROLL_LINES, MAX_SCROLL_LINES, "dx"),
      ...mods(opts?.mods),
    });
  }

  async keyDown(key: string, opts?: { mods?: ModKey[] }): Promise<void> {
    await this.client.request("keydown", { key, ...mods(opts?.mods) });
  }

  async keyUp(key: string, opts?: { mods?: ModKey[] }): Promise<void> {
    await this.client.request("keyup", { key, ...mods(opts?.mods) });
  }

  async press(key: string, keyMods?: ModKey[]): Promise<void> {
    // press holds modifiers around the key, so a rejection can latch them just like a drag.
    await this.holding(() => this.client.request("press", { key, ...mods(keyMods) }));
  }

  async type(text: string, opts?: { perCharMs?: number }): Promise<void> {
    const perCharMs = durationMs(opts?.perCharMs, 8, "perCharMs");
    await this.client.request("type", { text, perCharMs }, DEFAULT_TIMEOUT_MS + text.length * (perCharMs + 4));
  }

  async releaseAll(): Promise<void> {
    await this.client.releaseAll();
  }

  async cursor(): Promise<Pt> {
    const r = await this.client.request("cursor");
    return { x: Number(r.x), y: Number(r.y) };
  }

  async permissions(opts?: { prompt?: boolean }): Promise<Permissions> {
    const r = await this.client.request("permissions", { prompt: opts?.prompt ?? false });
    return { accessibility: Boolean(r.accessibility), screenRecording: Boolean(r.screenRecording) };
  }

  /** Disposes this input's helper, so `MacWindows` stops working until a new adapter is made. */
  async dispose(): Promise<void> {
    if (shared === this.client) shared = null;
    await this.client.dispose();
  }
}
