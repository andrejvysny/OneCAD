/*
 * Split-button flyout for tool variants (`ToolDefinition.flyout`).
 *
 * The visible half is the member the toolbar shows by default — the family's
 * last-picked member, else its lowest-priority one — and a plain click arms
 * THAT member immediately. The chevron half opens a menu listing every member,
 * so a variant is one extra click away instead of owning real-estate of its own.
 *
 * The chip follows the ACTIVE member when one is armed (so the glyph always says
 * what a gesture is about to draw), and falls back to the sticky default
 * otherwise. The pressed state covers the whole family: any member active
 * lights the button.
 *
 * Availability is per member, read fresh on every render the way `ToolEntry`
 * reads it: the main half carries the chip's own `canActivate` verdict, and a
 * disabled member renders inert in the menu with its reason kept off the row
 * (the row is too small to carry it — the disabled Tooltip convention belongs
 * to the 34px buttons, not 30px menu rows).
 */
import { useRef, useState } from "react";
import { cn } from "@/ui/cn";
import { Icon } from "@/icons/Icon";
import { MonoValue } from "@/ui/MonoValue";
import { Popover } from "@/ui/Popover";
import { Tooltip } from "@/ui/Tooltip";
import { toolbarIcon, ToolButton } from "./ToolButton";
import type { ToolAvailability, ToolContext, ToolDefinition, ToolId } from "@/platform";

const ALWAYS_ENABLED: ToolAvailability = { enabled: true };

type ToolFlyoutProps = {
  /** The family's members in registry order (length ≥ 2). */
  members: readonly ToolDefinition[];
  /** The member the main half defaults to (last-picked, else first). */
  sticky: ToolDefinition;
  /** `useActiveToolId`'s currency: the platform's `ToolId` or `null` when idle. */
  activeId: ToolId | null | undefined;
  context: ToolContext;
  /** Arm a member — the host's own `activate`, plus remembering the default. */
  onPick: (def: ToolDefinition) => void;
};

export function ToolFlyout({ members, sticky, activeId, context, onPick }: ToolFlyoutProps) {
  const [open, setOpen] = useState(false);
  const chevron = useRef<HTMLButtonElement | null>(null);

  const active = members.find((m) => m.id === activeId);
  // The chip shows the ACTIVE member when one is armed, else the sticky default.
  const shown = active ?? sticky;
  const verdict = active ? ALWAYS_ENABLED : (shown.canActivate?.(context) ?? ALWAYS_ENABLED);

  return (
    <div className="flex items-center">
      <ToolButton
        icon={toolbarIcon(shown.icon)}
        label={shown.title}
        shortcut={shown.shortcutLabel ?? ""}
        active={active !== undefined}
        onClick={() => onPick(shown)}
        disabled={!verdict.enabled}
        disabledReason={verdict.reason}
      />
      <Tooltip label="More options">
        <button
          ref={chevron}
          type="button"
          aria-label={`${sticky.title} options`}
          aria-haspopup="menu"
          aria-expanded={open}
          onClick={() => setOpen((v) => !v)}
          className={cn(
            "-ml-0.5 flex h-[34px] w-[14px] items-center justify-center rounded-sm border-none transition-colors",
            "cursor-pointer text-ink-4 hover:bg-hover-3 hover:text-ink-2",
            "focus-visible:shadow-focus-ring focus-visible:outline-none",
            open && "bg-hover-3 text-ink-2",
          )}
        >
          <Icon name="chevronDown" size={12} strokeWidth={2} />
        </button>
      </Tooltip>

      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={chevron}
        placement="bottom-start"
        width={190}
        className="p-1"
      >
        {members.map((m) => (
          <FlyoutRow
            key={m.id}
            def={m}
            isActive={m.id === activeId}
            context={context}
            onClick={() => {
              setOpen(false);
              onPick(m);
            }}
          />
        ))}
      </Popover>
    </div>
  );
}

function FlyoutRow({
  def,
  isActive,
  context,
  onClick,
}: {
  def: ToolDefinition;
  isActive: boolean;
  context: ToolContext;
  onClick: () => void;
}) {
  const verdict = isActive ? ALWAYS_ENABLED : (def.canActivate?.(context) ?? ALWAYS_ENABLED);
  return (
    <div
      role="menuitem"
      aria-disabled={!verdict.enabled}
      onClick={verdict.enabled ? onClick : undefined}
      className={cn(
        "flex h-[30px] cursor-pointer items-center gap-2 rounded-[5px] px-2.5 text-[12.5px] hover:bg-well",
        verdict.enabled ? "text-ink-2" : "cursor-not-allowed text-ink-4 opacity-60",
      )}
    >
      <Icon name={toolbarIcon(def.icon)} size={16} strokeWidth={1.7} className="shrink-0" />
      <span className="flex-1 truncate">{def.title}</span>
      {def.shortcutLabel && (
        <MonoValue className="text-[11px] text-ink-6">{def.shortcutLabel}</MonoValue>
      )}
      {isActive && <Icon name="check" size={13} strokeWidth={2} className="shrink-0 text-accent" />}
    </div>
  );
}
