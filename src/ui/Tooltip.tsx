import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

type TooltipProps = {
  label: string;
  children: ReactNode;
  /** Force-open (mainly for showcase/testing). */
  open?: boolean;
};

type Pos = { x: number; y: number };

/** C3/C4/C5: crossing the toolbar must not spam a tooltip per control — only
 *  a cursor that DWELLS on one anchor for this long earns a tooltip. */
const OPEN_DELAY_MS = 450;

/**
 * Dark tooltip positioned centered below the anchor. Portals into document.body
 * with fixed positioning (no portal library). Shows on hover/focus after a
 * dwell delay; hides immediately on unhover/blur, a click anywhere, or Escape.
 */
export function Tooltip({ label, children, open }: TooltipProps) {
  const anchor = useRef<HTMLSpanElement>(null);
  const [pos, setPos] = useState<Pos | null>(null);
  const openTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cancelOpenTimer = () => {
    if (openTimer.current === null) return;
    clearTimeout(openTimer.current);
    openTimer.current = null;
  };

  const show = () => {
    cancelOpenTimer();
    openTimer.current = setTimeout(() => {
      openTimer.current = null;
      const el = anchor.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      setPos({ x: r.left + r.width / 2, y: r.bottom + 8 });
    }, OPEN_DELAY_MS);
  };
  const hide = () => {
    cancelOpenTimer();
    setPos(null);
  };

  const visible = open || pos !== null;

  // A click anywhere (the popover the button just opened included) or Escape
  // must retire a lingering tooltip — a mouse that doesn't move off the button
  // after the click leaves `pos` set with nothing left to clear it otherwise.
  useEffect(() => {
    if (pos === null) return;
    const onPointerDown = () => setPos(null);
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setPos(null);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [pos]);

  useEffect(() => cancelOpenTimer, []);

  return (
    <span
      ref={anchor}
      className="inline-flex"
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={show}
      onBlur={hide}
    >
      {children}
      {visible &&
        createPortal(
          <div
            role="tooltip"
            style={{
              position: "fixed",
              left: pos?.x ?? 0,
              top: pos?.y ?? 0,
              transform: "translateX(-50%)",
            }}
            // Above the popover's z-[100] (C3: a stale tooltip must not peek out
            // from behind a snap-settings-shaped popover).
            className="pointer-events-none z-[110] whitespace-nowrap rounded-[5px] bg-tooltip px-2 py-1 font-ui text-[11px] text-tooltip-text"
          >
            {label}
          </div>,
          document.body,
        )}
    </span>
  );
}
