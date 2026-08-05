import type { Ref } from "react";
import { cn } from "./cn";
import { Icon } from "@/icons/Icon";

type EyeToggleProps = {
  /** true = visible (open eye, 0.85 opacity), false = hidden (struck-through eye, 0.3). */
  on: boolean;
  onChange: (on: boolean) => void;
  ariaLabel?: string;
  className?: string;
  ref?: Ref<HTMLButtonElement>;
};

/**
 * Visibility toggle. State is carried by the GLYPH (open eye vs struck-through
 * eye) and reinforced by opacity (prototype tree rows: 0.85 on ↔ 0.3 off,
 * hover → 1).
 *
 * The prototype had no eye-off glyph, so state used to be opacity ALONE — which
 * meant a hidden row and a dimmed-but-visible row looked the same, and the
 * control read as "faint" rather than "off". The CAD icon family ships both
 * halves of the pair, so the distinction is now in the shape.
 */
export function EyeToggle({
  on,
  onChange,
  ariaLabel,
  className,
  ref,
}: EyeToggleProps) {
  return (
    <button
      ref={ref}
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={ariaLabel ?? (on ? "Hide" : "Show")}
      onClick={() => onChange(!on)}
      className={cn(
        "inline-flex cursor-pointer items-center border-none bg-transparent text-ink-4 transition-opacity",
        "hover:opacity-100 focus-visible:opacity-100 focus-visible:outline-none",
        on ? "opacity-[0.85]" : "opacity-30",
        className,
      )}
    >
      <Icon name={on ? "eyeOn" : "eyeOff"} size={14} strokeWidth={1.6} />
    </button>
  );
}
