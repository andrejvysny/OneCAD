import type { RefObject } from "react";
import { Popover } from "@/ui/Popover";
import { SectionLabel } from "@/ui/SectionLabel";
import { Icon } from "@/icons/Icon";
import { cn } from "@/ui/cn";
import { useViewportStore, type SectionPlaneId } from "@/stores/viewportStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { formatLengthWithUnit } from "@/units/format";
import { useViewportEngine } from "@/viewport/engineBridge";
import {
  LAYER_GROUPS,
  useLayersStore,
  type LayerKey,
  type SceneLayer,
} from "@/stores/layersStore";

const SECTION_PLANES: readonly SectionPlaneId[] = ["XY", "XZ", "YZ"];

/**
 * Section (cut-away) controls — the plane, its offset and which half survives.
 *
 * They live in the LAYERS menu because a section is a VIEW FILTER, the same
 * class of thing every other switch here is: transient, document-free, and
 * scoped to this session. The state itself is `viewportStore.section`, not
 * `layersStore` — the NavPill button and ⇧X drive the identical actions, so
 * there is one piece of state behind three entry points, never three copies.
 *
 * The sub-controls are DISABLED while the section is off rather than hidden, so
 * the menu shows what the cut will be before it is turned on.
 */
function SectionControls() {
  const section = useViewportStore((s) => s.section);
  const toggleSection = useViewportStore((s) => s.toggleSection);
  const setPlane = useViewportStore((s) => s.setSectionPlane);
  const setOffset = useViewportStore((s) => s.setSectionOffset);
  const flipSection = useViewportStore((s) => s.flipSection);
  const displayUnit = useSettingsStore((s) => s.displayUnit);
  const engine = useViewportEngine();
  // Subscribed for its EDGE, not its value: it flips false the moment every
  // visible body has landed in the scene, which is the one event that turns the
  // slider's range from "nothing to measure" into a real span while this
  // popover is already open.
  useViewportStore((s) => s.geometryPending);
  const on = section.enabled;

  // Read on EVERY render, not memoised on [engine, plane]: bodies stream in
  // asynchronously, so a popover opened before the first mesh landed would pin
  // `null` (a dead slider) until the user closed and reopened it. The read is a
  // Box3 over the body groups' cached bounding boxes — cheap, and this component
  // only renders on a section/settings change.
  const range = engine?.sectionOffsetRange(section.plane) ?? null;

  return (
    <div>
      <SectionLabel className="px-3.5 pb-0.5 pt-2">Section</SectionLabel>
      <div
        role="checkbox"
        aria-checked={on}
        aria-label="Section view"
        data-testid="section-toggle"
        onClick={toggleSection}
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
        <span className="text-[13px] text-ink-2">Section view</span>
        <span className="ml-auto text-[11px] text-ink-5">⇧X</span>
      </div>

      <div className="flex items-center gap-1 px-3.5 py-1">
        {SECTION_PLANES.map((id) => (
          <button
            key={id}
            type="button"
            aria-pressed={section.plane === id}
            disabled={!on}
            data-testid={`section-plane-${id.toLowerCase()}`}
            onClick={() => setPlane(id)}
            className={cn(
              "h-6 flex-1 rounded-sm border text-[11.5px] font-medium transition-colors",
              "focus-visible:shadow-focus-ring focus-visible:outline-none",
              !on
                ? "cursor-default border-border bg-surface text-ink-5 opacity-40"
                : section.plane === id
                  ? "cursor-pointer border-accent bg-sel-bg text-accent"
                  : "cursor-pointer border-border bg-surface text-ink-3 hover:bg-hover",
            )}
          >
            {id}
          </button>
        ))}
        <button
          type="button"
          aria-pressed={section.flip}
          disabled={!on}
          data-testid="section-flip"
          onClick={flipSection}
          className={cn(
            "h-6 flex-1 rounded-sm border text-[11.5px] font-medium transition-colors",
            "focus-visible:shadow-focus-ring focus-visible:outline-none",
            !on
              ? "cursor-default border-border bg-surface text-ink-5 opacity-40"
              : section.flip
                ? "cursor-pointer border-accent bg-sel-bg text-accent"
                : "cursor-pointer border-border bg-surface text-ink-3 hover:bg-hover",
          )}
        >
          Flip
        </button>
      </div>

      <div className="flex items-center gap-2 px-3.5 pb-2 pt-0.5">
        <input
          type="range"
          aria-label="Section offset"
          data-testid="section-offset"
          disabled={!on || !range}
          min={range?.min ?? -1}
          max={range?.max ?? 1}
          step={range ? Math.max((range.max - range.min) / 200, 0.001) : 0.001}
          value={section.offsetMm}
          onChange={(e) => setOffset(Number(e.target.value))}
          className="h-1 flex-1 accent-accent disabled:opacity-40"
        />
        <span className="w-[62px] shrink-0 text-right font-mono text-[11px] text-ink-4">
          {formatLengthWithUnit(section.offsetMm, displayUnit)}
        </span>
      </div>
    </div>
  );
}

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
        <SectionControls />
      </div>
    </Popover>
  );
}
