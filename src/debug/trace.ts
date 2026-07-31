/*
 * Commit-path tracing (EXTRUDE-COMMIT investigation). Always-on console logging
 * for the RARE lifecycle events of an op commit — arm, confirm, apply, regen
 * outcome, teardown. Never call this from drag-frequency paths (per-frame
 * preview updates), which would flood the console.
 *
 * Read it in the webview devtools console (`bun run tauri dev` → right-click →
 * Inspect); the matching Rust-side `tracing::` lines land on the dev terminal's
 * stderr, so the two streams correlate by wall-clock + revision.
 */

/** Seconds since page load, fixed to ms — cheap correlation key. */
function stamp(): string {
  return (performance.now() / 1000).toFixed(3);
}

export function trace(tag: string, message: string, data?: unknown): void {
  if (data === undefined) console.info(`[${stamp()}] [${tag}] ${message}`);
  else console.info(`[${stamp()}] [${tag}] ${message}`, data);
}

export function traceWarn(tag: string, message: string, data?: unknown): void {
  if (data === undefined) console.warn(`[${stamp()}] [${tag}] ${message}`);
  else console.warn(`[${stamp()}] [${tag}] ${message}`, data);
}
