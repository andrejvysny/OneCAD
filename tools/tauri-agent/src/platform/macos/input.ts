/**
 * macOS native input: a long-lived Swift helper driven over JSON lines.
 *
 * One helper process per agent process (`getHelperClient()`), shared with `MacWindows`.
 * Replies are correlated by request id, and verbs run one at a time in a FIFO because the
 * helper answers one at a time — a verb's timeout starts when it reaches the head, so queueing
 * behind a 10s drag is not reported as that verb timing out.
 *
 * Every verb has a timeout; a timeout or an unexpected helper exit restarts the helper and
 * immediately drains held input, so a wedged drag can never leave a button or modifier stuck down
 * at the OS level. The drain names what THIS process may have pressed (`release_all` with
 * `force`), never what the OS reports held — that report includes the user's own hand on the
 * mouse and their own modifiers, and synthesising ups for those fights the human. On the normal
 * path a release is TRACKED-only, which the live helper already knows.
 *
 * `mods` are physical ModKeys. The schema-level `Primary` alias is translated to
 * `Command` by the caller (the MCP tool layer), never here.
 */
import type { ErrorCode } from "../../errors.ts";
import { AgentError } from "../../errors.ts";
import type { Pt } from "../../geometry/types.ts";
import { log } from "../../log.ts";
import type { KeyboardLayoutInfo, ModKey, MouseButton, NativeInput, Permissions } from "../adapter.ts";
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
  // `ax_point`'s refusal ladder. These name the SAME conditions the webview resolver names, so
  // they map to the same codes: a caller must be able to tell "re-snapshot" from "it is gone"
  // from "it is still animating" without reading `details.helperCode`. Left unmapped they all
  // surfaced as INTERNAL, which reads as an agent bug rather than the refusal it is.
  ELEMENT_STALE: "ELEMENT_STALE",
  ELEMENT_NOT_FOUND: "ELEMENT_NOT_FOUND",
  ELEMENT_MOVING: "ELEMENT_MOVING",
  POINT_OUTSIDE_WINDOW: "POINT_OUTSIDE_WINDOW",
  // The helper's own refusal code, which it uses for every "you asked for something this element
  // cannot do": an unadvertised AX action, a disabled control, a non-settable AXValue, an
  // ambiguous menu path. Unmapped it fell through to INTERNAL, so a perfectly clear refusal
  // ("that button is disabled") reached the caller looking like an agent crash.
  INVALID_TARGET: "INVALID_TARGET",
};

/** Verb groups, for an accurate failure message. `ax_*` is a READ or an AX action, never input. */
function verbKind(verb: string): string {
  if (verb.startsWith("ax_")) return "accessibility verb";
  if (verb === "windows" || verb === "focus" || verb === "frontmost") return "window verb";
  if (verb === "permissions" || verb === "version" || verb === "layout") return "helper verb";
  return "native input verb";
}

/** Verbs that can leave a button or modifier held if they do not complete. */
const LATCHES = new Set(["down", "keydown", "click", "path", "press"]);
/** Verbs whose success proves what they name is no longer held at the OS level. */
const RELEASES = new Set(["up", "keyup", "click", "path", "press"]);

/**
 * What a verb could still be holding once it is over. Buttons and modifiers only — those are the
 * names `release_all` can act on, exactly as the helper's own tracked release can.
 */
interface Held {
  buttons: string[];
  mods: string[];
  /** Ordinary (non-modifier) keys, which `keydown`/`keyup` can also leave down. */
  keys: string[];
}

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
  /** set when a helper died on its own; the replacement must drain held input first */
  private needsDrain = false;
  /**
   * Everything THIS process may have left held at the OS level. It is the only record that
   * survives a helper crash, and it exists so recovery never has to ask the OS what is held —
   * that answer includes the user's own hand and their own modifiers.
   *
   * Covers mouse buttons, modifiers AND ordinary keys: `keyboard_down("a")` followed by a crash
   * leaves that key down and auto-repeating, and it is nameable only from here.
   *
   * Deliberately a superset: a name that turns out not to be held costs one ignored request,
   * because the helper releases a `force` name only when the OS confirms it down. A name MISSING
   * here is the expensive direction — an input left stuck.
   */
  private readonly potentialHeld: { buttons: Set<string>; mods: Set<string>; keys: Set<string> } = {
    buttons: new Set<string>(),
    mods: new Set<string>(),
    keys: new Set<string>(),
  };
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
        const out = await this.send("release_all", {}, DEFAULT_TIMEOUT_MS);
        // A tracked-only release clears the shadow set ONLY when no drain is still owed. This
        // helper reports what IT is holding, and a replacement that never pressed anything holds
        // nothing — so clearing here after a failed drain would answer "released" for a button
        // the OS still has down, and destroy the only record of it.
        if (!this.needsDrain) {
          this.potentialHeld.buttons.clear();
          this.potentialHeld.mods.clear();
          this.potentialHeld.keys.clear();
        }
        return out;
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
    // Recorded BEFORE the write: if the helper dies mid-verb there is no reply to learn from,
    // and this set is all that is left to say what the crash left held.
    const latched = LATCHES.has(verb) ? this.latch(heldBy(verb, args)) : null;
    try {
      const out = await this.send(verb, args, timeoutMs ?? verbTimeout(verb, args));
      // `up`/`keyup` name what they just released. A compound verb releases exactly what IT
      // pressed (its own Swift `defer`), which is what `latch` reported — so a modifier an
      // earlier `keydown` is still holding is not dropped by a `press` that rode on top of it.
      if (RELEASES.has(verb)) this.unlatch(latched ?? heldBy(verb, args));
      return out;
    } catch (e) {
      if (isRecoverable(e)) await this.recover();
      throw e;
    }
  }

  /** Records what a verb may leave held, and reports back only the names it actually added. */
  private latch(held: Held): Held {
    return {
      buttons: addNew(this.potentialHeld.buttons, held.buttons),
      mods: addNew(this.potentialHeld.mods, held.mods),
      keys: addNew(this.potentialHeld.keys, held.keys),
    };
  }

  private unlatch(held: Held): void {
    for (const b of held.buttons) this.potentialHeld.buttons.delete(b);
    for (const m of held.mods) this.potentialHeld.mods.delete(m);
    for (const k of held.keys) this.potentialHeld.keys.delete(k);
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

  /** After an unplanned helper death, the replacement releases what this process may have held. */
  private async drainIfNeeded(): Promise<void> {
    if (!this.needsDrain) return;
    // Cleared only by a drain that actually LANDED. Clearing it here would mean a drain that
    // threw — a respawn that failed, a replacement that timed out — silently became the last
    // attempt ever made, leaving a button down at the OS level with no path left to release it.
    this.needsDrain = !(await this.drainPotentialHeld());
  }

  /**
   * Asks the replacement helper to release exactly the buttons and modifiers this process may
   * have pressed. Never `osState`: the tracked state died with the old helper, but the host
   * already knows what it might have caused, and the OS's own answer would include the user's
   * hand on the mouse and their own held modifiers.
   *
   * The helper drops any of these the OS does not actually report held, so an over-wide guess
   * costs nothing. Never throws — it runs ahead of someone else's verb.
   */
  private async drainPotentialHeld(): Promise<boolean> {
    const force = [...this.potentialHeld.buttons, ...this.potentialHeld.mods, ...this.potentialHeld.keys];
    try {
      await this.send("release_all", { osState: false, force }, DEFAULT_TIMEOUT_MS);
      this.potentialHeld.buttons.clear();
      this.potentialHeld.mods.clear();
      this.potentialHeld.keys.clear();
      return true;
    } catch (e) {
      // Both the record AND the obligation outlive a failed drain: the caller re-arms
      // `needsDrain`, so the next verb through the queue tries again against a fresh helper.
      log.warn("post-restart release_all failed", { error: String(e), force });
      // An empty force list owed nothing, so a failed send leaves no obligation behind; a
      // non-empty one must be retried or the named input stays down.
      return force.length === 0;
    }
  }

  /** Restart, then release what this process may have left held before any further verb runs. */
  private async recover(): Promise<void> {
    if (this.disposed) return;
    log.warn("restarting native input helper");
    this.kill(new AgentError("HELPER_FAILED", "native input helper restarted"));
    try {
      await this.ensureStarted();
      // Re-armed rather than assumed done: if this respawn or its drain fails, the obligation
      // has to survive to the next attempt or the held button is lost for the session.
      this.needsDrain = !(await this.drainPotentialHeld());
    } catch (e) {
      this.needsDrain = true;
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

/** Adds `names` to `set`, returning only those that were not already in it. */
function addNew(set: Set<string>, names: string[]): string[] {
  const added: string[] = [];
  for (const name of names) {
    if (set.has(name)) continue;
    set.add(name);
    added.push(name);
  }
  return added;
}

/**
 * Mirror of the helper's `canonicalButton`. Aliases MUST collapse the same way on both sides: a
 * button pressed as "center" that a release named "middle" could not find would stay down.
 */
function canonicalButton(raw: unknown): string | null {
  switch (String(raw).toLowerCase()) {
    case "left":
      return "left";
    case "right":
      return "right";
    case "middle":
    case "center":
    case "other":
      return "middle";
    default:
      return null;
  }
}

/** Mirror of the helper's `Mods.canonical`; a name that is not a modifier is simply not one. */
function canonicalMod(raw: unknown): string | null {
  switch (String(raw).toLowerCase()) {
    case "command":
    case "cmd":
      return "Command";
    case "control":
    case "ctrl":
      return "Control";
    case "option":
    case "alt":
      return "Option";
    case "shift":
      return "Shift";
    case "fn":
    case "function":
      return "Fn";
    default:
      return null;
  }
}

function present(names: Array<string | null>): string[] {
  return names.filter((n): n is string => n !== null);
}

/**
 * The buttons and modifiers a verb holds down while it runs — the names the helper's own `defer`
 * would release, and therefore the names left held if the helper never gets that far.
 *
 * A verb's `mods` option only sets event FLAGS (`flagsNow`) except in `press`, which physically
 * presses the modifier keys; so only `press` reports them.
 */
function heldBy(verb: string, args: Record<string, unknown>): Held {
  switch (verb) {
    case "down":
    case "up":
    case "click":
    case "path":
      return { buttons: present([canonicalButton(args.button)]), mods: [], keys: [] };
    case "keydown":
    case "keyup": {
      // A modifier goes in `mods`; anything else is an ordinary key, which `keydown` can leave
      // down and auto-repeating just as easily — and which nothing could release before.
      const mod = canonicalMod(args.key);
      if (mod !== null) return { buttons: [], mods: [mod], keys: [] };
      const key = typeof args.key === "string" && args.key.length > 0 ? args.key.toLowerCase() : null;
      return { buttons: [], mods: [], keys: present([key]) };
    }
    case "press": {
      // `press` holds its modifiers around the key and unwinds them in a `defer`; a throw in
      // between latches them. `key` counts too — pressing a modifier BY NAME latches it the same.
      const list: unknown[] = Array.isArray(args.mods) ? args.mods : [];
      // `press` releases its own key in the same verb, so only the modifiers it holds AROUND
      // that key can be left latched by a throw in between.
      return { buttons: [], mods: present([...list, args.key].map(canonicalMod)), keys: [] };
    }
    default:
      return { buttons: [], mods: [], keys: [] };
  }
}

export function mapHelperError(verb: string, code: string | undefined, message: string | undefined): AgentError {
  const mapped: ErrorCode = (code === undefined ? undefined : CODE_MAP[code]) ?? "INTERNAL";
  // The prefix names what actually failed. Calling an `ax_press` refusal a "native input verb"
  // failure sent a reader looking for a CGEvent problem that does not exist.
  return new AgentError(mapped, `${verbKind(verb)} '${verb}' failed: ${message ?? code ?? "unknown error"}`, {
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

  async layout(): Promise<KeyboardLayoutInfo> {
    const r = await this.client.request("layout");
    return { inputSourceId: String(r.inputSourceId ?? ""), isAnsiUs: Boolean(r.isAnsiUs) };
  }

  /** Disposes this input's helper, so `MacWindows` stops working until a new adapter is made. */
  async dispose(): Promise<void> {
    if (shared === this.client) shared = null;
    await this.client.dispose();
  }
}
