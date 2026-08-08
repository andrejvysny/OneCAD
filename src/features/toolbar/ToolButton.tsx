import { cn } from "@/ui/cn";
import { Tooltip } from "@/ui/Tooltip";
import { ICON_MONO, Icon } from "@/icons/Icon";
import type { IconName } from "@/icons/paths";

type ToolButtonProps = {
  icon: IconName;
  label: string;
  shortcut: string;
  active: boolean;
  onClick: () => void;
  /** Tool isn't applicable to the current selection — grayed out, inert click. */
  disabled?: boolean;
  /** Reason shown verbatim in the tooltip when `disabled` — reused from the
   *  same guard text ModelToolController's arm functions show as a status hint. */
  disabledReason?: string;
};

/**
 * 34px floating-toolbar tool (prototype 1c). Active = selection-tint bg + accent
 * icon; hover surface otherwise. Tooltip shows "Label (Shortcut)" below, or the
 * disabled reason when inapplicable to the current selection.
 *
 * The active state paints the whole glyph accent-colored, which would leave a
 * two-tone icon showing two near-identical blues instead of a readable
 * primary/accent split. It collapses to monochrome there by pointing
 * `--color-icon-accent` at `currentColor` — the mono preset every icon in the
 * family is authored to survive (src/icons/DESIGN.md §3).
 *
 * Disabled uses `aria-disabled` + a JS no-op guard, NOT the native `disabled`
 * attribute (contra `ModelToolChips.tsx`'s segmented-control buttons): a
 * natively-disabled element drops out of the tab order and — in Firefox —
 * never dispatches the mouse events `Tooltip` listens for, both wrong for a
 * button whose whole job here is to explain itself on hover/focus.
 */
export function ToolButton({
  icon,
  label,
  shortcut,
  active,
  onClick,
  disabled,
  disabledReason,
}: ToolButtonProps) {
  return (
    <Tooltip label={disabled && disabledReason ? disabledReason : `${label} (${shortcut})`}>
      <button
        type="button"
        aria-label={label}
        aria-pressed={active}
        aria-disabled={disabled}
        onClick={disabled ? undefined : onClick}
        className={cn(
          "flex h-[34px] w-[34px] items-center justify-center rounded-sm border-none transition-colors",
          "focus-visible:shadow-focus-ring focus-visible:outline-none",
          disabled
            ? "cursor-not-allowed bg-transparent text-ink-4 opacity-40"
            : cn(
                "cursor-pointer hover:bg-hover-3",
                active ? cn("bg-sel-bg text-accent", ICON_MONO) : "bg-transparent text-ink-4",
              ),
        )}
      >
        <Icon name={icon} size={20} strokeWidth={1.7} />
      </button>
    </Tooltip>
  );
}
