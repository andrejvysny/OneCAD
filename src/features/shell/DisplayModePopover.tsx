import type { RefObject } from "react";
import { Popover } from "@/ui/Popover";
import { SectionLabel } from "@/ui/SectionLabel";
import { SegmentedToggle, type SegmentedOption } from "@/ui/SegmentedToggle";
import { Icon } from "@/icons/Icon";
import { useSettingsStore } from "@/stores/settingsStore";
import { THEMES, THEME_ORDER, type ThemePref } from "@/theme/themes";
import { RENDER_MODES, RENDER_MODE_ORDER, type RenderModeId } from "@/viewport/engine/renderModes";

/** Built from the registry so the control can never list a theme that isn't real. */
const THEME_OPTIONS: SegmentedOption<ThemePref>[] = THEME_ORDER.map((id) => ({
  value: id,
  label: THEMES[id].label,
}));

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
 *
 * Also carries Appearance. Both settings answer "how does the viewport look",
 * so they belong behind the same button; a fourth corner-cluster icon for a
 * once-a-month preference would not earn its place.
 */
export function DisplayModePopover({ open, onClose, anchorRef }: DisplayModePopoverProps) {
  const displayMode = useSettingsStore((s) => s.displayMode);
  const setDisplayMode = useSettingsStore((s) => s.setDisplayMode);
  const theme = useSettingsStore((s) => s.theme);
  const setTheme = useSettingsStore((s) => s.setTheme);

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

      <div className="mx-3.5 my-1.5 h-px bg-border-subtle" />

      <SectionLabel className="px-3.5 pb-0.5 pt-0.5">Appearance</SectionLabel>
      <div className="px-3.5 pb-1 pt-0.5">
        {/* Stays open on change, unlike the mode rows: the whole point is to
            see the theme land, and re-opening the popover to undo a mistake is
            worse than one extra click to dismiss. */}
        <SegmentedToggle
          options={THEME_OPTIONS}
          value={theme}
          onChange={setTheme}
          ariaLabel="Appearance"
          size="sm"
          className="w-full"
        />
      </div>
    </Popover>
  );
}
