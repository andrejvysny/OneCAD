/**
 * UI chrome which must never be interpreted as a viewport gesture.
 *
 * Viewport controllers listen on a common ancestor so overlays can sit above the
 * canvas. `target.closest()` alone is not enough for portaled/shadow content;
 * inspect the composed path first and fall back to the target for synthetic
 * events. Keep this deliberately structural: controls own their pointer stream,
 * while the canvas owns everything else.
 */
const INTERACTIVE_BOUNDARY = [
  "[data-viewport-interactive]",
  "input",
  "textarea",
  "select",
  "[contenteditable]",
  "button",
  "[role=button]",
  "[role=toolbar]",
  "[role=menu]",
  "[role=menubar]",
  "[role=menuitem]",
  "[role=listbox]",
  "[role=dialog]",
  '[data-testid="model-tool-chip"]',
  '[data-testid*="inspector"]',
  '[data-testid*="menu"]',
].join(",");

function isBoundaryElement(value: EventTarget): boolean {
  return value instanceof Element && value.matches(INTERACTIVE_BOUNDARY);
}

/** Target-only form for call sites that intentionally do not have the event. */
export function isInteractiveBoundaryTarget(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest(INTERACTIVE_BOUNDARY);
}

/** True when a pointer event originated in interactive chrome rather than canvas content. */
export function isInteractiveBoundary(event: Event): boolean {
  const path = typeof event.composedPath === "function" ? event.composedPath() : [];
  if (path.some(isBoundaryElement)) return true;
  return isInteractiveBoundaryTarget(event.target);
}
