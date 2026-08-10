import { useRef, useState, type Ref } from "react";
import { cn } from "@/ui/cn";
import { Tooltip } from "@/ui/Tooltip";
import { Icon } from "@/icons/Icon";
import type { IconName } from "@/icons/paths";
import { useViewportStore } from "@/stores/viewportStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { RENDER_MODES } from "@/viewport/engine/renderModes";
import { SnapPopover } from "@/features/snap/SnapPopover";
import { DisplayModePopover } from "@/features/shell/DisplayModePopover";
import { ViewCube } from "@/features/viewcube/ViewCube";

function ClusterButton({
  icon,
  label,
  strokeWidth = 1.6,
  active = false,
  expanded,
  onClick,
  ref,
}: {
  icon: IconName;
  label: string;
  strokeWidth?: number;
  active?: boolean;
  /**
   * Popover-trigger semantics: when set, the button emits aria-expanded
   * instead of aria-pressed (it opens a popover, it isn't a plain toggle).
   * Visual `active` styling is unaffected either way.
   */
  expanded?: boolean;
  onClick: () => void;
  ref?: Ref<HTMLButtonElement>;
}) {
  const stateProps =
    expanded === undefined ? { "aria-pressed": active } : { "aria-expanded": expanded };
  return (
    <Tooltip label={label}>
      <button
        ref={ref}
        type="button"
        aria-label={label}
        {...stateProps}
        onClick={onClick}
        className={cn(
          "flex h-9 w-9 cursor-pointer items-center justify-center rounded-md border border-border shadow-ctrl transition-colors",
          "focus-visible:shadow-focus-ring focus-visible:outline-none",
          active ? "bg-sel-bg text-accent" : "bg-surface text-ink-4 hover:bg-hover",
        )}
      >
        <Icon name={icon} size={17} strokeWidth={strokeWidth} />
      </button>
    </Tooltip>
  );
}

/**
 * Top-right reserved corner (prototype 1c): ViewCube placeholder + a column of
 * display-mode / grid / snap buttons. Grid shows a pressed (accent) state;
 * display + snap are popover triggers that open their settings panel to the
 * left. The two popovers share one `openPopover` slot so they can never both
 * be open in this narrow column.
 */
export function CornerCluster() {
  const gridVisible = useViewportStore((s) => s.gridVisible);
  const toggleGrid = useViewportStore((s) => s.toggleGrid);
  const displayMode = useSettingsStore((s) => s.displayMode);

  // Two popovers share this column; only one may be open at a time.
  const [openPopover, setOpenPopover] = useState<"none" | "display" | "snap">("none");
  const displayBtnRef = useRef<HTMLButtonElement | null>(null);
  const snapBtnRef = useRef<HTMLButtonElement | null>(null);

  return (
    <div className="absolute right-[264px] top-3 z-[25] flex flex-col items-center gap-2">
      <div className="-translate-x-4">
        <ViewCube />
      </div>

      {/* The label carries the current mode — a bare "Display mode" on a
          tri-state control says nothing about which mode is active. */}
      <ClusterButton
        ref={displayBtnRef}
        icon="display"
        label={`Display mode: ${RENDER_MODES[displayMode].label}`}
        active={openPopover === "display"}
        expanded={openPopover === "display"}
        onClick={() => setOpenPopover((v) => (v === "display" ? "none" : "display"))}
      />
      <ClusterButton
        icon="grid"
        label="Toggle grid"
        strokeWidth={1.5}
        active={gridVisible}
        onClick={toggleGrid}
      />
      <ClusterButton
        ref={snapBtnRef}
        icon="snap"
        label="Snap settings"
        active={openPopover === "snap"}
        expanded={openPopover === "snap"}
        onClick={() => setOpenPopover((v) => (v === "snap" ? "none" : "snap"))}
      />

      <DisplayModePopover
        open={openPopover === "display"}
        onClose={() => setOpenPopover("none")}
        anchorRef={displayBtnRef}
      />
      <SnapPopover
        open={openPopover === "snap"}
        onClose={() => setOpenPopover("none")}
        anchorRef={snapBtnRef}
      />
    </div>
  );
}
