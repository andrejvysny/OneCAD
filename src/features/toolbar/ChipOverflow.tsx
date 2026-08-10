/*
 * ChipOverflow — the generic `⋯`-button-plus-click-away-popover shell every
 * armed model-tool cluster's secondary options collapse into (extracted from
 * ExtrudeChipControls' ExtrudeOverflow, the first tool to use this pattern).
 *
 * DISMISSAL is an outside press or a second click on the button. Esc
 * deliberately still cancels the TOOL, not the popover: the controller owns
 * Escape from a window listener registered in capture phase at construction
 * time, so nothing mounted later can preempt it — see ExtrudeChipControls for
 * the fuller rationale. Every tool's overflow shares that contract by sharing
 * this shell rather than re-implementing click-away per tool.
 */
import { useEffect, useRef, useState } from "react";
import { cn } from "@/ui/cn";

export interface ChipOverflowProps {
  testId?: string;
  /** Overrides the readout span's testid (default `${testId}-readout`) — kept
   *  distinct because Extrude's readout predates this shell as `chip-mode-readout`
   *  and is a frozen e2e/vitest contract (see `src/test/contracts/`). */
  readoutTestId?: string;
  ariaLabel: string;
  title?: string;
  /** Content shown inside the button when closed — a readout or "⋯". */
  readout: React.ReactNode;
  /** Raises the non-default dot: "something in here isn't the default". */
  marked: boolean;
  buttonClassName?: string;
  children: React.ReactNode;
}

export function ChipOverflow(props: ChipOverflowProps): React.ReactElement {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLSpanElement>(null);
  const testId = props.testId ?? "chip-overflow";
  const readoutTestId = props.readoutTestId ?? `${testId}-readout`;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent): void => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    // Capture: the viewport container stops some presses before they bubble.
    window.addEventListener("pointerdown", onDown, true);
    return () => window.removeEventListener("pointerdown", onDown, true);
  }, [open]);

  return (
    <span ref={rootRef} className="relative inline-flex items-center">
      <button
        type="button"
        data-testid={testId}
        aria-label={props.ariaLabel}
        aria-expanded={open}
        aria-pressed={open}
        title={props.title}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "relative rounded-full px-2 py-1 text-[11.5px] font-medium",
          open ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
          !open && props.buttonClassName,
        )}
      >
        <span data-testid={readoutTestId}>{props.readout}</span>
        {props.marked && (
          <span
            data-testid={`${testId}-dot`}
            aria-hidden="true"
            className="absolute right-0.5 top-0.5 h-1 w-1 rounded-full bg-accent"
          />
        )}
      </button>
      {open && (
        <div
          data-testid={`${testId}-panel`}
          className="absolute bottom-full left-1/2 mb-1.5 flex -translate-x-1/2 flex-col items-start gap-1.5 rounded-2xl border border-border bg-surface px-2 py-2 shadow-popover"
        >
          {props.children}
        </div>
      )}
    </span>
  );
}
