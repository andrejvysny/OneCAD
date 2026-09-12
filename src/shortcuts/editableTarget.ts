/*
 * "Is this event aimed at a text field?" — the one answer every global key
 * handler has to agree on.
 *
 * It lives in its own module because two owners need it and neither may import
 * the other: `useShortcuts` bails on it before resolving a binding, and the
 * undo/redo router (`@/features/shell/undoActions`) re-checks it so a chord that
 * arrives from somewhere OTHER than the keydown listener — a native menu
 * accelerator, the command palette — still leaves native text undo alone.
 */

/** Whether `el` is a focused text-entry surface that owns its own keys. */
export function isEditableTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  const tag = el.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    el.isContentEditable
  );
}
