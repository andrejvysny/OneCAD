import type { RefObject } from "react";
import { Popover } from "@/ui/Popover";
import { SectionLabel } from "@/ui/SectionLabel";
import { Icon } from "@/icons/Icon";
import { cn } from "@/ui/cn";
import { useViewportStore } from "@/stores/viewportStore";
import {
  LAYER_GROUPS,
  useLayersStore,
  type LayerKey,
  type SceneLayer,
} from "@/stores/layersStore";

/**
 * Viewport-layer menu, anchored to the bottom-left cluster (prototype 2a).
 *
 * Grouped checkboxes rather than a list, because the groups are what tell you
 * WHOSE layers these are — the design's point that a workspace adds layers
 * without adding a panel. Only groups the build can actually draw appear.
 *
 * The grid switch is `viewportStore`'s existing one, surfaced here as well as
 * on the corner cluster: two entry points to one piece of state, never two
 * pieces of state.
 */
export function LayersMenu({ anchorRef }: { anchorRef: RefObject<HTMLElement | null> }) {
  const open = useLayersStore((s) => s.open);
  const setOpen = useLayersStore((s) => s.setOpen);
  const visible = useLayersStore((s) => s.visible);
  const setVisible = useLayersStore((s) => s.setVisible);
  const gridVisible = useViewportStore((s) => s.gridVisible);
  const toggleGrid = useViewportStore((s) => s.toggleGrid);

  const isOn = (key: LayerKey) => (key === "grid" ? gridVisible : visible[key]);
  const flip = (key: LayerKey) => {
    if (key === "grid") toggleGrid();
    else setVisible(key as SceneLayer, !visible[key as SceneLayer]);
  };

  return (
    <Popover
      open={open}
      onClose={() => setOpen(false)}
      anchorRef={anchorRef}
      placement="top-start"
      width={224}
      className="py-1.5"
    >
      <div role="group" aria-label="Viewport layers" data-testid="layers-menu">
        {LAYER_GROUPS.map((group) => (
          <div key={group.id}>
            <SectionLabel className="px-3.5 pb-0.5 pt-2">{group.title}</SectionLabel>
            {group.layers.map((layer) => {
              const on = isOn(layer.key);
              return (
                <div
                  key={layer.key}
                  role="checkbox"
                  aria-checked={on}
                  aria-label={layer.label}
                  data-testid={`layer-${layer.key}`}
                  onClick={() => flip(layer.key)}
                  className="flex h-[30px] cursor-pointer items-center gap-2.5 px-3.5 hover:bg-hover"
                >
                  <span
                    aria-hidden="true"
                    className={cn(
                      "flex h-4 w-4 flex-none items-center justify-center rounded-[4px] border",
                      on ? "border-accent bg-accent" : "border-border-strong bg-surface",
                    )}
                  >
                    <Icon
                      name="check"
                      size={10}
                      strokeWidth={3.2}
                      className={cn("text-on-accent", on ? "opacity-100" : "opacity-0")}
                    />
                  </span>
                  <span className="text-[13px] text-ink-2">{layer.label}</span>
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </Popover>
  );
}
