/*
 * log.ts — the FRONTEND structured logger (DEV-OBSERVABILITY Wave F).
 *
 * One event type, one ring buffer, one console mirror, one subscription seam.
 * `logSink.ts` rides the subscription to batch-forward events to Rust, which
 * re-emits them into `logs/dev.jsonl` at target `fe` — that is how a webview
 * console line ends up next to the command span and the worker stderr that
 * produced it.
 *
 * ── HARD RULE: this module imports NOTHING from `src/**` ──────────────────────
 * Every layer logs, including the stores and the IPC client, so any import here
 * is a latent cycle. Keep it dependency-free (the `@tauri-apps` invoke lives in
 * logSink.ts, which is imported by main.tsx only).
 *
 * ── Gate ─────────────────────────────────────────────────────────────────────
 * Evaluated ONCE at module load: `(DEV build || ?trace) && !vitest`. A disabled
 * logger costs one boolean test per call and never builds a message, so call
 * sites need no gate of their own. The vitest exclusion is load-bearing: ~30
 * test files drive the instrumented code paths, and a console mirror there would
 * bury the real test output. The logger's OWN tests re-enable via
 * `__resetLogForTests()`.
 *
 * ── Policy ───────────────────────────────────────────────────────────────────
 * NEVER log from a drag-frequency path (per-frame preview updates, pointer
 * moves, solver ticks). This lane is for the RARE lifecycle events — commits,
 * failures, phase transitions, IPC round-trips. `ctx` is capped at entry (see
 * the constants below), so a caller may pass a fat object without thinking, but
 * it must not pass a raw geometry payload.
 *
 * ── White-box surfaces (only when enabled) ───────────────────────────────────
 *   window.__logs        — chronological snapshot of the ring (getter)
 *   window.__logsDump()  — the same, as a pretty JSON string
 *   window.__logsClear() — empty the ring
 */

export type LogLevel = "debug" | "info" | "warn" | "error";

/**
 * A tag names the SUBSYSTEM, not the file. Current taxonomy:
 *   boolean · extrude · revolve · sketch · mesh · measure   (op/tool lanes)
 *   ipc · fsm · vp · hint · err · worker · repair · sink     (infrastructure)
 * Add a tag rather than overloading one; `docs/DEBUGGING.md` carries the list.
 */
export type LogTag = string;

/** One logged event. This is ALSO the wire DTO for the `log_event` command. */
export interface LogEvent {
  /** Wall clock, epoch ms — correlates with the Rust `timestamp` field. */
  ts: number;
  /** `performance.now()`, ms — monotonic, survives a clock step. */
  tMono: number;
  /** Per-session counter; makes same-millisecond ordering unambiguous. */
  seq: number;
  lane: "fe";
  level: LogLevel;
  tag: LogTag;
  msg: string;
  /** Capped structured context (see `capCtx`). Absent when the caller passed none. */
  ctx?: Record<string, unknown>;
}

// ── Capping limits (applied AT ENTRY, so the ring itself is always bounded) ───
const MAX_STRING = 500;
const MAX_ARRAY = 20;
/** Nesting levels kept below `ctx`; deeper values collapse to a placeholder. */
const MAX_DEPTH = 2;
const RING_CAP = 2000;

// ── State ────────────────────────────────────────────────────────────────────

function underVitest(): boolean {
  if (import.meta.env.MODE === "test") return true;
  // `process` is absent in a browser; vitest's node host defines VITEST.
  const proc = (globalThis as { process?: { env?: Record<string, string | undefined> } }).process;
  return Boolean(proc?.env?.VITEST);
}

function gateOpen(): boolean {
  if (underVitest()) return false;
  if (import.meta.env.DEV) return true;
  // No `window` in a non-browser host (SSR/node) — nothing to opt in with.
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).has("trace");
}

let enabled = gateOpen();
let mirrorConsole = true;
let seq = 0;

/** Head-index ring — a write is one store + one modulo, never an O(n) shift. */
const ring = new Array<LogEvent>(RING_CAP);
let head = 0;
let filled = 0;

const subscribers = new Set<(e: LogEvent) => void>();

// ── Capping ──────────────────────────────────────────────────────────────────

function capString(s: string): string {
  return s.length <= MAX_STRING ? s : `${s.slice(0, MAX_STRING)}…(+${s.length - MAX_STRING})`;
}

function capValue(v: unknown, depth: number): unknown {
  if (v === null || v === undefined) return v;
  switch (typeof v) {
    case "string":
      return capString(v);
    case "number":
    case "boolean":
      return v;
    case "bigint":
      return `${v.toString()}n`;
    case "function":
      return "[Function]";
    case "symbol":
      return v.toString();
    default:
      break;
  }
  // A mesh buffer must never reach the sink as 200k numbers.
  if (ArrayBuffer.isView(v)) return { type: v.constructor.name, byteLength: v.byteLength };
  if (v instanceof ArrayBuffer) return { type: "ArrayBuffer", byteLength: v.byteLength };
  // An Error JSON-serializes to `{}` — the one shape that MUST survive, since
  // most warn/error call sites pass the caught value straight through.
  if (v instanceof Error) {
    return {
      name: v.name,
      message: capString(v.message),
      ...(v.stack === undefined ? {} : { stack: capString(v.stack) }),
    };
  }
  if (Array.isArray(v)) {
    if (depth > MAX_DEPTH) return `[Array(${v.length})]`;
    const out: unknown[] = v.slice(0, MAX_ARRAY).map((x) => capValue(x, depth + 1));
    if (v.length > MAX_ARRAY) out.push(`…+${v.length - MAX_ARRAY} more`);
    return out;
  }
  if (depth > MAX_DEPTH) return "[Object]";
  const src = v as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const k of Object.keys(src)) out[k] = capValue(src[k], depth + 1);
  return out;
}

function capCtx(ctx: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const k of Object.keys(ctx)) out[k] = capValue(ctx[k], 1);
  return out;
}

// ── Ring ─────────────────────────────────────────────────────────────────────

function push(ev: LogEvent): void {
  ring[head] = ev;
  head = (head + 1) % RING_CAP;
  if (filled < RING_CAP) filled++;
}

function snapshot(): LogEvent[] {
  if (filled < RING_CAP) return ring.slice(0, filled);
  return [...ring.slice(head), ...ring.slice(0, head)];
}

function clearRing(): void {
  ring.length = 0;
  head = 0;
  filled = 0;
}

// ── Emit ─────────────────────────────────────────────────────────────────────

function mirror(ev: LogEvent): void {
  const line = `[${(ev.tMono / 1000).toFixed(3)}] [${ev.tag}] ${ev.msg}`;
  /* eslint-disable no-console */
  const sink =
    ev.level === "error"
      ? console.error
      : ev.level === "warn"
        ? console.warn
        : ev.level === "debug"
          ? console.debug
          : console.info;
  /* eslint-enable no-console */
  if (ev.ctx === undefined) sink(line);
  else sink(line, ev.ctx);
}

function emit(level: LogLevel, tag: LogTag, msg: string, ctx?: Record<string, unknown>): void {
  if (!enabled) return;
  const ev: LogEvent = {
    ts: Date.now(),
    tMono: Math.round(performance.now() * 1000) / 1000,
    seq: seq++,
    lane: "fe",
    level,
    tag,
    msg,
    ...(ctx === undefined ? {} : { ctx: capCtx(ctx) }),
  };
  push(ev);
  if (mirrorConsole) mirror(ev);
  for (const cb of subscribers) {
    try {
      cb(ev);
    } catch {
      // A broken subscriber must never break the code it is observing.
    }
  }
}

// ── Public API ───────────────────────────────────────────────────────────────

/** Log at a level chosen at runtime (severity-driven call sites). */
export function log(level: LogLevel, tag: LogTag, msg: string, ctx?: Record<string, unknown>): void {
  emit(level, tag, msg, ctx);
}

export function logDebug(tag: LogTag, msg: string, ctx?: Record<string, unknown>): void {
  emit("debug", tag, msg, ctx);
}

export function logInfo(tag: LogTag, msg: string, ctx?: Record<string, unknown>): void {
  emit("info", tag, msg, ctx);
}

export function logWarn(tag: LogTag, msg: string, ctx?: Record<string, unknown>): void {
  emit("warn", tag, msg, ctx);
}

export function logError(tag: LogTag, msg: string, ctx?: Record<string, unknown>): void {
  emit("error", tag, msg, ctx);
}

/** Whether the gate is open for this page load. `logSink` reads it to stay off. */
export function logEnabled(): boolean {
  return enabled;
}

/** Chronological copy of the ring (oldest first). */
export function logSnapshot(): LogEvent[] {
  return snapshot();
}

/** Subscribe to every emitted event. Returns the unsubscribe. */
export function onLogEvent(cb: (e: LogEvent) => void): () => void {
  subscribers.add(cb);
  return () => {
    subscribers.delete(cb);
  };
}

// ── Window surface ───────────────────────────────────────────────────────────

function installWindowApi(): void {
  if (typeof window === "undefined") return;
  const w = window as unknown as Record<string, unknown>;
  // A getter (not a snapshot array) so devtools always shows the CURRENT ring.
  Object.defineProperty(w, "__logs", { configurable: true, get: () => snapshot() });
  w.__logsDump = (): string => JSON.stringify(snapshot(), null, 2);
  w.__logsClear = (): void => clearRing();
}

function removeWindowApi(): void {
  if (typeof window === "undefined") return;
  const w = window as unknown as Record<string, unknown>;
  delete w.__logs;
  delete w.__logsDump;
  delete w.__logsClear;
}

if (enabled) installWindowApi();

/**
 * TEST SEAM — re-open the gate that vitest closes.
 *
 * `console` defaults to OFF so a test can assert on emitted events without
 * polluting the runner output; the logger's own mirror test opts back in.
 * Resets the ring, the sequence counter and every subscriber, so each test
 * starts from a known state. `src/test/setup.ts` closes the gate again after
 * every test, so an enabling test can never leak into its neighbours.
 */
export function __resetLogForTests(opts?: { enabled?: boolean; console?: boolean }): void {
  enabled = opts?.enabled ?? true;
  mirrorConsole = opts?.console ?? false;
  seq = 0;
  clearRing();
  subscribers.clear();
  if (enabled) installWindowApi();
  else removeWindowApi();
}
