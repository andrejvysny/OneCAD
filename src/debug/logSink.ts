/*
 * logSink — batched forwarding of frontend log events to the Rust `log_event`
 * command, which re-emits them at target `fe` into `logs/dev.jsonl`.
 *
 * That single file is the whole point of the exercise: without this forwarder
 * the webview console and the Rust/worker stderr are two streams nobody can
 * correlate without a stopwatch.
 *
 * ── Invariants ───────────────────────────────────────────────────────────────
 *  • `invoke` is imported DIRECTLY from `@tauri-apps/api/core`, never through
 *    `tauriClient.call<T>()` — `call` logs, and logging the forwarding of a log
 *    would recurse. For the same reason `log_event` is in `IPC_LOG_EXEMPT`.
 *  • Nothing happens outside a Tauri webview (mock lane / browser dev / e2e):
 *    there is no Rust side to receive it, and the ring + console already carry
 *    everything.
 *  • Events tagged `sink` are NEVER forwarded — that tag is this module talking
 *    about itself, and forwarding it is the second recursion path.
 *  • Three consecutive failures disable forwarding for the session with exactly
 *    ONE console.warn. A missing/unregistered `log_event` command lands here, so
 *    an older backend degrades to console+ring instead of warning per event.
 */
import { invoke } from "@tauri-apps/api/core";
import { logEnabled, onLogEvent, type LogEvent } from "./log";

/** Batch size that forces an immediate flush. */
const FLUSH_AT = 50;
/** Lazy timer: armed by the first buffered event, cleared by every flush. */
const FLUSH_MS = 250;
/** Hard cap on the pending buffer — a stalled backend drops the OLDEST events. */
const MAX_BUFFER = 500;
const MAX_FAILURES = 3;

const NOOP = (): void => {};

let installed = false;

/**
 * Start forwarding. Returns the teardown (idempotent, safe to call once per app
 * boot). A no-op — and a no-op teardown — when the gate is closed or there is no
 * Tauri bridge.
 */
export function installLogForwarder(): () => void {
  if (installed) return NOOP;
  if (!logEnabled()) return NOOP;
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return NOOP;
  installed = true;

  const buffer: LogEvent[] = [];
  let timer: ReturnType<typeof setTimeout> | null = null;
  let failures = 0;
  let disabled = false;
  let inFlight = false;

  function flush(): void {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    if (disabled || inFlight || buffer.length === 0) return;
    const events = buffer.splice(0, buffer.length);
    inFlight = true;
    void invoke("log_event", { events })
      .then(() => {
        failures = 0;
      })
      .catch(() => {
        // The batch is gone either way — this is a best-effort dev lane, and
        // retrying a rejected batch would grow the buffer under a dead backend.
        failures += 1;
        if (failures >= MAX_FAILURES) {
          disabled = true;
          buffer.length = 0;
          // eslint-disable-next-line no-console
          console.warn(
            `[sink] log forwarding disabled after ${MAX_FAILURES} consecutive log_event failures`,
          );
        }
      })
      .finally(() => {
        inFlight = false;
        if (!disabled && buffer.length >= FLUSH_AT) flush();
      });
  }

  const off = onLogEvent((ev) => {
    if (disabled) return;
    if (ev.tag === "sink") return;
    buffer.push(ev);
    if (buffer.length > MAX_BUFFER) buffer.splice(0, buffer.length - MAX_BUFFER);
    if (buffer.length >= FLUSH_AT) flush();
    else if (timer === null) timer = setTimeout(flush, FLUSH_MS);
  });

  // `pagehide` (not `beforeunload`) — the only teardown hook a WKWebView fires
  // reliably. Best-effort: an in-flight invoke may not complete.
  const onPageHide = (): void => flush();
  window.addEventListener("pagehide", onPageHide);

  return () => {
    off();
    window.removeEventListener("pagehide", onPageHide);
    if (timer !== null) clearTimeout(timer);
    timer = null;
    buffer.length = 0;
    installed = false;
  };
}
