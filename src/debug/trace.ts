/*
 * Commit-path tracing (EXTRUDE-COMMIT investigation). Console logging for the
 * RARE lifecycle events of an op commit — arm, confirm, apply, regen outcome,
 * teardown. Never call this from drag-frequency paths (per-frame preview
 * updates), which would flood the console.
 *
 * GATED (TRUST wave): enabled iff this is a DEV build (`import.meta.env.DEV`) OR
 * the URL carries `?trace` — a shipped build stays silent unless the user asks
 * for it, mirroring how the viewport reads `?vpdebug` (ViewportRoot `hasFlag`).
 * The gate is evaluated ONCE at module load and both entry points early-return
 * on it, so a disabled trace costs one boolean test and never builds a message.
 *
 * Read it in the webview devtools console (`bun run tauri dev` → right-click →
 * Inspect); the matching Rust-side `tracing::` lines land on the dev terminal's
 * stderr, so the two streams correlate by wall-clock + revision.
 */

/** Whether tracing is on for this page load (DEV build, or an explicit `?trace`). */
const ENABLED: boolean = (() => {
  if (import.meta.env.DEV) return true;
  // No `window` in a non-browser host (SSR/node) — nothing to opt in with.
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).has("trace");
})();

/** Seconds since page load, fixed to ms — cheap correlation key. */
function stamp(): string {
  return (performance.now() / 1000).toFixed(3);
}

export function trace(tag: string, message: string, data?: unknown): void {
  if (!ENABLED) return;
  if (data === undefined) console.info(`[${stamp()}] [${tag}] ${message}`);
  else console.info(`[${stamp()}] [${tag}] ${message}`, data);
}

export function traceWarn(tag: string, message: string, data?: unknown): void {
  if (!ENABLED) return;
  if (data === undefined) console.warn(`[${stamp()}] [${tag}] ${message}`);
  else console.warn(`[${stamp()}] [${tag}] ${message}`, data);
}
