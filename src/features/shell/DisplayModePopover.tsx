import type { RefObject } from "react";
import { Popover } from "@/ui/Popover";
import { SectionLabel } from "@/ui/SectionLabel";
import { Icon } from "@/icons/Icon";
import { useSettingsStore } from "@/stores/settingsStore";
import { RENDER_MODES, RENDER_MODE_ORDER, type RenderModeId } from "@/viewport/engine/renderModes";

function DisplayModeRow({
  label,
  checked,
  onClick,
}: {
  label: string;
  checked: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitemradio"
      aria-checked={checked}
      onClick={onClick}
      className="flex h-8 w-full cursor-pointer items-center gap-2 px-3.5 text-left text-[13px] text-ink-2 hover:bg-hover"
    >
      <span className="flex-1">{label}</span>
      {checked && <Icon name="check" size={14} strokeWidth={2} className="text-accent" />}
    </button>
  );
}

type DisplayModePopoverProps = {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLButtonElement | null>;
};

/**
 * Display-mode popover (mirrors SnapPopover): every render mode as one
 * menuitemradio row, anchored to the corner-cluster display button, opening
 * to its left with a right-pointing caret. Replaces the old cycling button —
 * a tri-state control that only ever showed "the next click's target" gave no
 * way to jump straight to a mode or see all of them at once.
 */
export function DisplayModePopover({ open, onClose, anchorRef }: DisplayModePopoverProps) {
  const displayMode = useSettingsStore((s) => s.displayMode);
  const setDisplayMode = useSettingsStore((s) => s.setDisplayMode);

  const choose = (mode: RenderModeId) => {
    setDisplayMode(mode);
    onClose();
  };

  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      width={200}
      caret
      placement="left-start"
      className="py-1.5"
    >
      <SectionLabel className="px-3.5 pb-0.5 pt-0.5">Display</SectionLabel>
      <div role="menu" aria-label="Display mode">
        {RENDER_MODE_ORDER.map((id) => (
          <DisplayModeRow
            key={id}
            label={RENDER_MODES[id].label}
            checked={id === displayMode}
            onClick={() => choose(id)}
          />
        ))}
      </div>
    </Popover>
  );
}
