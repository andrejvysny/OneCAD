/*
 * SketchErrorPulse — the NEAR-THE-ACTION half of refusal feedback (audit item
 * #10's recorded residual; SKETCH_UX_AUDIT.md "Landed record").
 *
 * A refused sketch edit (a conflicting dimension, a locked-geometry edit, a
 * failed upsert) already lands in `StatusBar`, which pulses it — but that is the
 * far bottom-left corner while the user's eyes are on the cursor. This mirrors
 * the SAME hint, once, next to where the action happened, then gets out of the
 * way after {@link PULSE_DISMISS_MS}.
 *
 * It is NOT a second hint bus: the only source is `viewportStore.statusHint`
 * (+ `statusHintSeq`, so a repeated identical error re-pulses exactly the way
 * the status bar's own re-key does). Deliberately narrow:
 *   - `severity === "error"` only — an info/success hint never appears here,
 *   - sketch mode only (this component only mounts with the sketch chrome),
 *   - `pointer-events-none`, always: it floats over the canvas where the next
 *     click has to go, so it must never intercept one.
 *
 * ANCHOR: the last pointer position seen while sketching, which for every
 * pointer-driven refusal IS the action point (the click that was refused). A
 * keyboard-driven refusal (a constraint chord) falls back to wherever the
 * cursor was left, and a session with no pointer event yet anchors at the
 * window centre rather than at (0,0). Tracking writes to a ref and NEVER to
 * state — no re-render, no log, nothing on the pointer path.
 *
 * Rendered through a portal to `document.body` because its host lives inside
 * the toolbar stack, which is `-translate-x-1/2` — a transformed ancestor is a
 * containing block for `position: fixed`, so an in-place bubble would be
 * offset by half the stack's width.
 */
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useToolStore } from "@/stores/toolStore";
import { useViewportStore } from "@/stores/viewportStore";

/** How long the near-action bubble stays up. Deliberately shorter than the
 *  status bar's own `AUTO_DISMISS_MS` (4s): this one sits over the drawing, and
 *  the corner copy remains readable after it goes. */
export const PULSE_DISMISS_MS = 2500;

/** Down-right of the anchor, clear of the cursor itself. */
const OFFSET_X = 14;
const OFFSET_Y = 16;
/** Matches the `max-w-[240px]` below — used to keep the bubble on screen. */
const MAX_W = 240;
/** Enough for two wrapped lines; only used for the bottom-edge clamp. */
const EST_H = 44;
const MARGIN = 8;

interface Bubble {
  message: string;
  seq: number;
  x: number;
  y: number;
}

/** Anchor → an on-screen top-left, clamped so a refusal at the window edge is
 *  still fully readable. No pointer seen yet ⇒ the window centre. */
function place(at: { x: number; y: number } | null): { x: number; y: number } {
  const px = at?.x ?? window.innerWidth / 2;
  const py = at?.y ?? window.innerHeight / 2;
  const maxX = Math.max(MARGIN, window.innerWidth - MAX_W - MARGIN);
  const maxY = Math.max(MARGIN, window.innerHeight - EST_H - MARGIN);
  return {
    x: Math.min(Math.max(px + OFFSET_X, MARGIN), maxX),
    y: Math.min(Math.max(py + OFFSET_Y, MARGIN), maxY),
  };
}

export function SketchErrorPulse() {
  const mode = useToolStore((s) => s.mode);
  const hint = useViewportStore((s) => s.statusHint);
  const seq = useViewportStore((s) => s.statusHintSeq);
  const pointer = useRef<{ x: number; y: number } | null>(null);
  const [bubble, setBubble] = useState<Bubble | null>(null);

  const sketching = mode === "sketch";

  useEffect(() => {
    if (!sketching) return;
    const track = (e: PointerEvent) => {
      pointer.current = { x: e.clientX, y: e.clientY };
    };
    window.addEventListener("pointermove", track, { passive: true });
    window.addEventListener("pointerdown", track, { passive: true });
    return () => {
      window.removeEventListener("pointermove", track);
      window.removeEventListener("pointerdown", track);
    };
  }, [sketching]);

  const message = sketching && hint?.severity === "error" ? hint.message : null;

  // `seq` is in the deps on purpose: a REPEATED identical error keeps the same
  // message, and re-running on the bumped sequence is what re-arms the timer and
  // re-keys the node so the one-shot animation plays again.
  useEffect(() => {
    if (message === null) {
      setBubble(null);
      return;
    }
    setBubble({ message, seq, ...place(pointer.current) });
    const timer = setTimeout(() => setBubble(null), PULSE_DISMISS_MS);
    return () => clearTimeout(timer);
  }, [message, seq]);

  if (!bubble) return null;

  return createPortal(
    <div
      key={bubble.seq}
      data-testid="sketch-error-pulse"
      role="status"
      style={{ left: bubble.x, top: bubble.y }}
      className="hint-error-pulse pointer-events-none fixed z-[60] max-w-[240px] rounded-md border border-danger-border bg-danger-surface px-2 py-1 text-[11.5px] font-medium leading-snug text-danger-strong shadow-panel"
    >
      {bubble.message}
    </div>,
    document.body,
  );
}
