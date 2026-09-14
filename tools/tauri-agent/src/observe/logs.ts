/**
 * Cursor tail over the app's `dev.jsonl`.
 *
 * The file is TRUNCATED at every app start, so a cursor past EOF means a new session
 * rather than "nothing new" — reading from that offset would silently return an empty
 * delta for the whole run. Cursors are byte offsets rendered as strings so they survive
 * the JSON envelope unchanged.
 */
import { open } from "node:fs/promises";
import { join } from "node:path";
import { resolveFromRoot } from "../session/config.ts";

export type LogLevel = "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR";

const SEVERITY: Record<LogLevel, number> = { TRACE: 10, DEBUG: 20, INFO: 30, WARN: 40, ERROR: 50 };

export const DEFAULT_LIMIT = 200;
/** Never pull more than this much history in one call, however far back the cursor points. */
const MAX_READ_BYTES = 4 * 1024 * 1024;

export interface LogQuery {
  since?: string;
  level?: LogLevel;
  lane?: string;
  grep?: string;
  limit?: number;
}

export interface LogLine {
  level: string;
  target: string;
  raw: string;
  fields?: Record<string, unknown>;
}

export interface LogTail {
  path: string;
  lines: LogLine[];
  cursor: string;
  matched: number;
  dropped: number;
  missing: boolean;
}

/**
 * The launched app writes to the session's own artifact directory (ONECAD_LOG_DIR);
 * an attached one writes to the project's `logs/dev.jsonl`.
 */
export function devJsonlPath(opts: {
  root: string;
  devJsonl: string | null;
  journalDir: string;
  launched: boolean;
}): string | null {
  if (opts.launched) return join(opts.journalDir, "app-logs", "dev.jsonl");
  return opts.devJsonl === null ? null : resolveFromRoot(opts.root, opts.devJsonl);
}

export async function logCursor(path: string | null): Promise<string> {
  if (path === null) return "0";
  try {
    const fh = await open(path, "r");
    try {
      return String((await fh.stat()).size);
    } finally {
      await fh.close();
    }
  } catch {
    return "0";
  }
}

export async function tailLogs(path: string | null, query: LogQuery = {}): Promise<LogTail> {
  if (path === null) {
    return { path: "", lines: [], cursor: "0", matched: 0, dropped: 0, missing: true };
  }
  const read = await readFrom(path, toOffset(query.since));
  if (read === null) {
    return { path, lines: [], cursor: query.since ?? "0", matched: 0, dropped: 0, missing: true };
  }
  const limit = Math.max(1, Math.min(query.limit ?? DEFAULT_LIMIT, DEFAULT_LIMIT));
  const matches =
    read.text.length === 0
      ? []
      : read.text
          .split("\n")
          .filter((l) => l.length > 0)
          .map(parseLine)
          .filter((l) => keeps(l, query));
  const lines = matches.length > limit ? matches.slice(matches.length - limit) : matches;
  return {
    path,
    lines,
    cursor: String(read.cursor),
    matched: matches.length,
    dropped: matches.length - lines.length,
    missing: false,
  };
}

/** How many lines at or above `level` arrived since `since`; the effects probe. */
export async function countSince(path: string | null, since: string, level: LogLevel): Promise<number> {
  return (await tailLogs(path, { since, level, limit: 1 })).matched;
}

function toOffset(since: string | undefined): number {
  const n = Number(since ?? "0");
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
}

function keeps(line: LogLine, q: LogQuery): boolean {
  if (q.level !== undefined && severityOf(line.level) < SEVERITY[q.level]) return false;
  if (q.lane !== undefined && !line.target.startsWith(q.lane)) return false;
  if (q.grep !== undefined && !new RegExp(q.grep).test(line.raw)) return false;
  return true;
}

function severityOf(level: string): number {
  return SEVERITY[level.toUpperCase() as LogLevel] ?? SEVERITY.INFO;
}

function parseLine(raw: string): LogLine {
  try {
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    return {
      level: typeof parsed.level === "string" ? parsed.level : "INFO",
      target: typeof parsed.target === "string" ? parsed.target : "",
      raw,
      fields: parsed,
    };
  } catch {
    return { level: "INFO", target: "", raw };
  }
}

/** Returns null when the file does not exist; whole lines only, so a half-written tail is left for next time. */
async function readFrom(path: string, start: number): Promise<{ text: string; cursor: number } | null> {
  let fh;
  try {
    fh = await open(path, "r");
  } catch {
    return null;
  }
  try {
    const size = (await fh.stat()).size;
    let from = start > size ? 0 : start;
    if (size - from > MAX_READ_BYTES) from = size - MAX_READ_BYTES;
    if (size - from <= 0) return { text: "", cursor: size };
    const buf = Buffer.alloc(size - from);
    await fh.read(buf, 0, buf.length, from);
    const text = buf.toString("utf8");
    const lastNl = text.lastIndexOf("\n");
    if (lastNl < 0) return { text: "", cursor: from };
    const whole = text.slice(0, lastNl);
    return { text: whole, cursor: from + Buffer.byteLength(text.slice(0, lastNl + 1)) };
  } finally {
    await fh.close();
  }
}
