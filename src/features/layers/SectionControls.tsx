import { useEffect, useState, type RefObject } from "react";
import { Icon } from "@/icons/Icon";
import { Popover } from "@/ui/Popover";
import { SectionLabel } from "@/ui/SectionLabel";
import { cn } from "@/ui/cn";
import { useSettingsStore } from "@/stores/settingsStore";
import { useViewportStore, type SectionPlaneId } from "@/stores/viewportStore";
import { formatLength, lengthSuffix, parseLength } from "@/units/format";
import { useViewportEngine } from "@/viewport/engineBridge";

const SECTION_PLANES: readonly SectionPlaneId[] = ["XY", "XZ", "YZ"];

function SectionOffsetInput({ disabled }: { disabled: boolean }) {
  const offsetMm = useViewportStore((s) => s.section.offsetMm);
  const setOffset = useViewportStore((s) => s.setSectionOffset);
  const unit = useSettingsStore((s) => s.displayUnit);
  const [draft, setDraft] = useState(() => formatLength(offsetMm, unit));
  const [dirty, setDirty] = useState(false);
  const valid = parseLength(draft, unit) !== null;
  const rejected = dirty && !valid;

  useEffect(() => {
    setDraft(formatLength(offsetMm, unit));
    setDirty(false);
  }, [offsetMm, unit]);

  const applyDraft = (): void => {
    if (!dirty) return;
    const next = parseLength(draft, unit);
    if (next === null) return;
    setDirty(false);
    setOffset(next);
  };

  return (
    <div>
      <label className="flex items-center gap-1 rounded-sm border border-border bg-surface px-1.5 py-1 font-mono text-[11px] text-ink-2">
        <span className="sr-only">Section offset</span>
        <input
          type="text"
          inputMode="decimal"
          data-testid="section-offset-input"
          aria-label={`Section offset (${lengthSuffix(unit)})`}
          aria-invalid={rejected}
          disabled={disabled}
          value={draft}
          onChange={(event) => {
            setDraft(event.target.value);
            setDirty(true);
          }}
          onBlur={applyDraft}
          onKeyDown={(event) => {
            if (event.key !== "Enter") return;
            event.preventDefault();
            applyDraft();
          }}
          className="w-12 bg-transparent text-right outline-none disabled:opacity-40"
        />
        <span aria-hidden className="text-ink-5">{lengthSuffix(unit)}</span>
      </label>
      {rejected && <p role="alert" className="mt-0.5 text-[10px] text-warn">Enter a valid length.</p>}
    </div>
  );
}

function SectionToggle({ on }: { on: boolean }) {
  const toggleSection = useViewportStore((s) => s.toggleSection);
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={on}
      aria-label="Section view"
      data-testid="section-toggle"
      onClick={toggleSection}
      className="flex h-[30px] w-full items-center gap-2.5 border-0 bg-transparent px-3.5 text-left hover:bg-hover"
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
    </button>
  );
}

/** Shared transient section controls for the Layers menu and navigation pill. */
export function SectionControls({ includeToggle = true }: { includeToggle?: boolean }) {
  const section = useViewportStore((s) => s.section);
  const setPlane = useViewportStore((s) => s.setSectionPlane);
  const setOffset = useViewportStore((s) => s.setSectionOffset);
  const flipSection = useViewportStore((s) => s.flipSection);
  const engine = useViewportEngine();
  useViewportStore((s) => s.geometryPending);
  const range = engine?.sectionOffsetRange(section.plane) ?? null;
  const on = section.enabled;

  return (
    <div>
      <SectionLabel className="px-3.5 pb-0.5 pt-2">Section</SectionLabel>
      {includeToggle && <SectionToggle on={on} />}
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
          aria-label="Section offset slider"
          data-testid="section-offset"
          disabled={!on || !range}
          min={range?.min}
          max={range?.max}
          step={range ? Math.max((range.max - range.min) / 200, 0.001) : undefined}
          value={section.offsetMm}
          onChange={(event) => setOffset(Number(event.target.value))}
          className="h-1 flex-1 accent-accent disabled:opacity-40"
        />
        <SectionOffsetInput disabled={!on} />
      </div>
    </div>
  );
}

export function SectionControlsPopover({
  open,
  onClose,
  anchorRef,
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
}) {
  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      placement="top-start"
      width={232}
      ariaLabel="Section controls"
      autoFocus="first"
      className="py-1.5"
    >
      <div role="group" aria-label="Section controls" data-testid="section-controls-popover">
        <SectionControls />
      </div>
    </Popover>
  );
}
