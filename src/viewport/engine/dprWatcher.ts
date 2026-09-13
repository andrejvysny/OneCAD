/*
 * dprWatcher — event-driven notification that the DISPLAY changed, with no CSS
 * box change (VP-HARDENING VP03, spec §7.2).
 *
 * `ResizeObserver` never fires for a stationary window dragged from a 1× to a
 * 2× display: the element's CSS size is identical, only `devicePixelRatio`
 * moved. Until now the only detector was an in-frame recheck, which is useless
 * on an idle viewport — render-on-demand means there IS no next frame.
 *
 * The detector is a resolution media query armed at the CURRENT ratio. It stops
 * matching the instant the ratio changes, which fires `change`; the query is
 * then thrown away and a new one armed at the NEW ratio, because a query pinned
 * to the old value can only report the first transition. `visualViewport`'s
 * `resize` is a secondary trigger (some browsers report a scale change there
 * first); the consumer is expected to no-op on an unchanged ratio, so a
 * duplicate notification is free.
 *
 * NO POLLING. There is no interval and no rAF here — an idle viewport must
 * schedule zero work (TEST-LIFE-04).
 */

/**
 * Call `onChange` with the new RAW `window.devicePixelRatio` whenever the
 * display scale changes. Returns a disposer that removes every listener.
 *
 * In an environment without `matchMedia` (jsdom) this is a no-op returning a
 * no-op disposer — the in-frame `syncDpr()` recheck remains the safety net.
 */
export function watchDevicePixelRatio(onChange: (dpr: number) => void): () => void {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return () => {};
  }

  let query: MediaQueryList | null = null;
  let disposed = false;

  const rawDpr = (): number => window.devicePixelRatio || 1;

  const handleQueryChange = (): void => {
    if (disposed) return;
    // Re-arm FIRST: `onChange` may synchronously resize and re-render, and a
    // window that lands on a third display mid-frame must still be caught.
    disarm();
    arm();
    onChange(rawDpr());
  };

  const handleViewportResize = (): void => {
    if (disposed) return;
    onChange(rawDpr());
  };

  function arm(): void {
    query = window.matchMedia(`(resolution: ${rawDpr()}dppx)`);
    query.addEventListener("change", handleQueryChange);
  }

  function disarm(): void {
    query?.removeEventListener("change", handleQueryChange);
    query = null;
  }

  arm();
  window.visualViewport?.addEventListener("resize", handleViewportResize);

  return () => {
    if (disposed) return;
    disposed = true;
    disarm();
    window.visualViewport?.removeEventListener("resize", handleViewportResize);
  };
}
