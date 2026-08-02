/*
 * Commit-path tracing — now a THIN SHIM over the structured logger (`log.ts`).
 *
 * The 40-odd call sites predate the logger and keep their signature verbatim:
 * `trace(tag, message, data?)` → `logInfo(tag, message, {data})`, likewise
 * `traceWarn` → `logWarn`. Everything the old module owned (the DEV/`?trace`
 * gate, the monotonic stamp, the console mirror) now lives in `log.ts`, so this
 * file adds no behaviour of its own beyond wrapping the loose `data` argument
 * into a named `ctx` key.
 *
 * NEW call sites should use `logDebug`/`logInfo`/`logWarn`/`logError` directly
 * and pass real structured context; this shim exists so the migration did not
 * have to touch 40 lines of unrelated logic.
 *
 * Still never call it from a drag-frequency path — see the policy note in log.ts.
 */
import { logInfo, logWarn } from "./log";

export function trace(tag: string, message: string, data?: unknown): void {
  logInfo(tag, message, data === undefined ? undefined : { data });
}

export function traceWarn(tag: string, message: string, data?: unknown): void {
  logWarn(tag, message, data === undefined ? undefined : { data });
}
