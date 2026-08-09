import { useRef, type Ref } from "react";
import { cn } from "@/ui/cn";
import { Tooltip } from "@/ui/Tooltip";
import { LayersMenu } from "@/features/layers/LayersMenu";
import { useLayersStore } from "@/stores/layersStore";
import { Icon } from "@/icons/Icon";
import type { IconName } from "@/icons/paths";
import { useViewportStore } from "@/stores/viewportStore";
import { selectedBodyIds, useSelectionStore } from "@/stores/selectionStore";

function NavButton({
  icon,
  label,
  strokeWidth = 1.7,
  active = false,
  disabled = false,
  onClick,
  ref,
}: {
  icon: IconName;
  label: string;
  strokeWidth?: number;
  /** Pressed (accent) treatment — the mode this button toggles is ON. */
  active?: boolean;
  disabled?: boolean;
  onClick: () => void;
  ref?: Ref<HTMLButtonElement>;
}) {
  return (
    <Tooltip label={label}>
      <button
        ref={ref}
        type="button"
        aria-label={label}
        aria-pressed={active}
        disabled={disabled}
        onClick={onClick}
        className={cn(
          "flex h-8 w-8 items-center justify-center rounded-sm border-none transition-colors",
          "focus-visible:shadow-focus-ring focus-visible:outline-none",
          disabled
            ? "cursor-default bg-transparent text-ink-4 opacity-40"
            : active
              ? "cursor-pointer bg-sel-bg text-accent"
              : "cursor-pointer bg-transparent text-ink-4 hover:bg-hover-2",
        )}
      >
        <Icon name={icon} size={17} strokeWidth={strokeWidth} />
      </button>
    </Tooltip>
  );
}

/**
 * Bottom-left navigation pill (prototype 1c): home, zoom-to-fit, isolate, view
 * presets. Zoom-to-fit is Shift+F because plain F is the Fillet tool (see keymap
 * note), so the tooltip reads "(⇧F)".
 *
 * Isolate is a TRANSIENT view state (viewportStore), never a document write — it
 * shows the pressed treatment while on, and is disabled when there is nothing to
 * isolate (no body-shaped selection) and nothing to leave.
 */
export function NavPill() {
  const zoomFit = useViewportStore((s) => s.zoomFit);
  const homeView = useViewportStore((s) => s.homeView);
  const toggleIsolate = useViewportStore((s) => s.toggleIsolate);
  const isolated = useViewportStore((s) => s.isolatedBodyIds !== null);
  const hasBodySelection = useSelectionStore((s) => selectedBodyIds(s.selected).length > 0);
  const layersOpen = useLayersStore((s) => s.open);
  const toggleLayers = useLayersStore((s) => s.toggleOpen);
  const layersBtn = useRef<HTMLButtonElement | null>(null);

  return (
    <div className="absolute bottom-[46px] left-[228px] z-[25] flex gap-0.5 rounded-md border border-border bg-surface p-[3px] shadow-ctrl">
      <NavButton icon="home" label="Home view (H)" onClick={homeView} />
      <NavButton icon="fit" label="Zoom to fit (⇧F)" onClick={zoomFit} />
      <NavButton
        icon="isolate"
        label={isolated ? "Exit isolation (⇧I)" : "Isolate selection (⇧I)"}
        strokeWidth={1.6}
        active={isolated}
        disabled={!isolated && !hasBodySelection}
        onClick={toggleIsolate}
      />
      <NavButton
        ref={layersBtn}
        icon="layers"
        label="Viewport layers"
        strokeWidth={1.6}
        active={layersOpen}
        onClick={toggleLayers}
      />
      <LayersMenu anchorRef={layersBtn} />
    </div>
  );
}
