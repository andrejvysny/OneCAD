/**
 * Everything this process says goes to stderr, because stdout is the OCAK1
 * frame stream (`docs/assistant/wire-protocol.md` §8). The supervisor reads
 * stderr line by line and forwards it into `tracing` under the target
 * `assistant`, which is why a line is a single JSON object: the Rust side gets
 * structured fields without parsing prose.
 *
 * The geometry worker splits its streams the same way for the same reason. One
 * `console.log` on stdout desynchronises a length-prefixed frame reader, and a
 * desynchronised reader cannot resync — it can only tear the connection down.
 */
import type { Logger } from "agentkit/host";

export const LOG_LEVELS = ["error", "warn", "info", "debug"] as const;
export type LogLevel = (typeof LOG_LEVELS)[number];

const LEVEL_RANK: Readonly<Record<LogLevel, number>> = Object.freeze({
  error: 0,
  warn: 1,
  info: 2,
  debug: 3,
});

export function isLogLevel(value: string): value is LogLevel {
  return (LOG_LEVELS as readonly string[]).includes(value);
}

/** Where a logger writes. Injected so a test can read back what was emitted. */
export type LogSink = (line: string) => void;

export const stderrSink: LogSink = (line) => {
  process.stderr.write(`${line}\n`);
};

/**
 * `fields` is serialised defensively: a tool result or a provider error can
 * carry a cycle or a `BigInt`, and a logger that throws while reporting a
 * failure replaces the failure with its own.
 */
function safeFields(fields: Record<string, unknown> | undefined): string {
  if (fields === undefined) return "";
  try {
    return JSON.stringify(fields);
  } catch {
    return '"<unserialisable>"';
  }
}

export function createLogger(
  level: LogLevel = "info",
  sink: LogSink = stderrSink,
): Logger {
  const threshold = LEVEL_RANK[level];
  const emit = (at: LogLevel, message: string, fields?: Record<string, unknown>): void => {
    if (LEVEL_RANK[at] > threshold) return;
    const serialised = safeFields(fields);
    sink(
      `{"ts":${JSON.stringify(new Date().toISOString())},"level":${JSON.stringify(at)},` +
        `"target":"assistant","message":${JSON.stringify(message)}` +
        (serialised === "" ? "" : `,"fields":${serialised}`) +
        "}",
    );
  };
  return {
    debug: (message, fields) => emit("debug", message, fields),
    info: (message, fields) => emit("info", message, fields),
    warn: (message, fields) => emit("warn", message, fields),
    error: (message, fields) => emit("error", message, fields),
  };
}

/**
 * Take exclusive ownership of stdout for the bridge, then make it impossible
 * for anything else in this process to reach it.
 *
 * Structural, not a convention: the real writer is captured and handed back to
 * exactly one caller (the frame writer in `main.ts`), and `process.stdout.write`
 * plus the five `console` methods are then rebound to stderr. A dependency deep
 * in AgentKit — or in a transitive package nobody here reviewed — that prints a
 * deprecation notice can no longer corrupt the frame stream; it just shows up in
 * the supervisor's log, which is where it belonged.
 *
 * Call this BEFORE importing anything that might print at module scope.
 * Idempotent, so a test may call it in several files.
 */
export type StdoutWriter = (bytes: Uint8Array) => Promise<void>;

interface ClaimedStdout {
  claimed: true;
  write: StdoutWriter;
}

const CLAIM_KEY = "__onecadAssistantStdoutClaim";

export function claimStdout(): StdoutWriter {
  const globals = globalThis as unknown as Record<string, ClaimedStdout | undefined>;
  const existing = globals[CLAIM_KEY];
  if (existing) return existing.write;

  const stdout = process.stdout;
  const realWrite = stdout.write.bind(stdout);

  /**
   * Resolves when the bytes have left this process's buffer, which is what
   * gives the frame writer real backpressure: without it a stalled host would
   * be answered by an unbounded kernel-side queue in our own heap.
   */
  const write: StdoutWriter = (bytes) =>
    new Promise((resolve, reject) => {
      realWrite(bytes, (err) => (err ? reject(err) : resolve()));
    });

  const toStderr = (...args: unknown[]): void => {
    process.stderr.write(
      `${args
        .map((arg) => (typeof arg === "string" ? arg : safeFields({ v: arg })))
        .join(" ")}\n`,
    );
  };

  // `process.stdout.write` catches a direct writer; the console methods are
  // patched separately because Bun's console does not route through the stream.
  stdout.write = ((chunk: unknown, ...rest: unknown[]): boolean => {
    process.stderr.write(typeof chunk === "string" ? chunk : (chunk as Uint8Array));
    const callback = rest.find((arg) => typeof arg === "function");
    if (callback) (callback as (err?: Error | null) => void)(null);
    return true;
  }) as typeof stdout.write;

  console.log = toStderr;
  console.info = toStderr;
  console.debug = toStderr;
  console.warn = toStderr;
  console.error = toStderr;

  globals[CLAIM_KEY] = { claimed: true, write };
  return write;
}
