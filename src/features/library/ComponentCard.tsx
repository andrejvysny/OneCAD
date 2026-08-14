/*
 * One library component as a card (LGU-1 canvas D1-A).
 *
 * ONE implementation for both grids. `LibraryModal` and `StartLibraryPanel`
 * carried character-for-character identical card markup — thumbnail, name,
 * version — which is how the two drifted into showing a bare semver where the
 * design asks for a standard and a size range.
 *
 * The old card also set `title={c.id}`, a native browser tooltip. That is the
 * detached `mine.body-2` label floating beside no card that the screenshot
 * audit caught: the platform renders it on its own schedule, at its own
 * position, outside the slot system. An id is provenance, so it moved to the
 * detail panel where provenance lives.
 */
import { ComponentThumbnail } from "./ComponentThumbnail";
import { SourceBadge } from "@/ui/SourceBadge";
import { badgeKindOf, cardSubtitleOf } from "@/modules/library/componentMeta";
import type { LibraryComponent } from "@/ipc/types";
import { cn } from "@/ui/cn";

export interface ComponentCardProps {
  component: LibraryComponent;
  selected?: boolean;
  onClick: () => void;
  /** Distinguishes the two grids for the e2e specs that address them. */
  testId: string;
}

export function ComponentCard({ component, selected, onClick, testId }: ComponentCardProps) {
  return (
    <button
      type="button"
      data-testid={testId}
      data-component-id={component.id}
      data-selected={selected || undefined}
      onClick={onClick}
      className={cn(
        "flex flex-col gap-1 rounded-lg border p-2 text-left transition-[box-shadow,border-color] duration-150 hover:border-card-hover-border hover:shadow-card-hover",
        selected ? "border-accent ring-1 ring-accent" : "border-border bg-surface",
      )}
    >
      <div className="relative">
        <ComponentThumbnail
          componentId={component.id}
          componentVersion={component.version}
          size={192}
          className="aspect-square w-full rounded bg-well object-contain"
        />
        <SourceBadge kind={badgeKindOf(component)} className="absolute right-[7px] top-[7px]" />
      </div>
      <div className="truncate text-[11.5px] font-semibold text-ink">{component.name}</div>
      <div className="truncate font-mono text-[10px] text-ink-3">{cardSubtitleOf(component)}</div>
    </button>
  );
}
