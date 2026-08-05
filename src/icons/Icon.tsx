import type { Ref, SVGProps } from "react";
import { ICONS, type IconName } from "./paths";

/**
 * Collapse a two-tone icon to monochrome, for containers that paint the glyph a
 * CHROMATIC color of their own — an active tool (accent), a failed history row
 * (red), a needs-repair row (amber), a selected row (selection blue).
 *
 * Two-tone assumes the primary tone is neutral. When it isn't, the accent stops
 * being a second tone and becomes a second HUE fighting the first: a red
 * "this op failed" icon with a blue arrow inside it reads as neither. Worse for
 * the accent-colored case, the two blues are close enough to just look like a
 * rendering artifact. Every icon in the family is authored to survive this
 * collapse (DESIGN.md §3), so it is always the safe answer.
 *
 * Neutral tones (`text-ink-*`) need nothing — they are what two-tone is for.
 */
export const ICON_MONO = "[--color-icon-accent:currentColor]";

type IconProps = {
  name: IconName;
  /** Rendered px width/height. Prototype default is 15. */
  size?: number;
  /** Base stroke width. Per-element `sw` ratios scale off this. */
  strokeWidth?: number;
  ref?: Ref<SVGSVGElement>;
} & Omit<SVGProps<SVGSVGElement>, "ref">;

/**
 * Two-tone stroked icon on the shared 24x24 grid: fill none, round caps/joins,
 * PRIMARY tone inheriting `currentColor` and ACCENT tone reading
 * `--color-icon-accent`.
 *
 * The accent is emitted as a `var()` rather than a resolved color so a
 * container can collapse the icon to monochrome by redefining that one custom
 * property (see ToolButton's active state) — the icons are authored to stay
 * readable when it does.
 */
export function Icon({
  name,
  size = 15,
  strokeWidth = 1.7,
  ref,
  ...rest
}: IconProps) {
  return (
    <svg
      ref={ref}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      {ICONS[name].map((el, i) => {
        const color =
          el.tone === "accent" ? "var(--color-icon-accent)" : "currentColor";
        return (
          <path
            // Icon definitions are static, so index is a stable identity here.
            key={i}
            d={el.d}
            fill={el.filled ? color : "none"}
            stroke={el.filled ? "none" : color}
            strokeWidth={el.filled ? undefined : strokeWidth * (el.sw ?? 1)}
            strokeDasharray={el.dash}
            transform={el.transform}
          />
        );
      })}
    </svg>
  );
}
