import { MonoValue } from "@/ui/MonoValue";
import { Icon } from "@/icons/Icon";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { CONSTRAINT_PRESENTATION } from "@/features/sketch/constraintCatalog";
import { formatDimensionValue } from "@/features/sketch/dimensionFormat";
import type { SketchConstraint, SketchConstraintType } from "@/ipc/types";

/** Dimensional constraint kinds that carry a `value` column. */
const DIMENSIONAL: ReadonlySet<SketchConstraintType> = new Set([
  "Distance",
  "HorizontalDistance",
  "VerticalDistance",
  "Angle",
  "Radius",
  "Diameter",
]);

/** Compact entity reference summary: "e1, e2" (≤2) or "e1 +2" (more). */
function entitySummary(entities: string[]): string {
  if (entities.length === 0) return "";
  if (entities.length <= 2) return entities.join(", ");
  return `${entities[0]} +${entities.length - 1}`;
}

/** Dimensional value text — Angle is UI-domain degrees already (angleUnits.ts). */
function valueText(c: SketchConstraint): string | null {
  if (!DIMENSIONAL.has(c.type) || typeof c.value !== "number") return null;
  return c.type === "Angle" ? `${formatDimensionValue(c.value)}°` : formatDimensionValue(c.value);
}

/** Small icon-only delete affordance — same shape as HistoryList's RowIconButton. */
function DeleteButton({
  onClick,
  title,
  testid,
}: {
  onClick: () => void;
  title: string;
  testid: string;
}) {
  return (
    <button
      type="button"
      data-testid={testid}
      title={title}
      aria-label={title}
      onClick={onClick}
      className="flex h-5 w-5 shrink-0 items-center justify-center rounded-sm text-ink-5 hover:bg-hover-3 hover:text-traffic-close"
    >
      <Icon name="x" size={13} strokeWidth={1.8} />
    </button>
  );
}

/** One 30px constraint row: glyph · type · entity summary · value · delete. A
 *  `conflicting` row (solver reports it in conflict, SCHEMA §7.4) tints its glyph +
 *  type label with the traffic-close token so the offending constraint is legible. */
function ConstraintRow({
  constraint,
  conflicting,
  onDelete,
}: {
  constraint: SketchConstraint;
  conflicting: boolean;
  onDelete: (id: string) => void;
}) {
  const value = valueText(constraint);
  const glyphTone = conflicting ? "text-traffic-close" : "text-ink-5";
  const typeTone = conflicting ? "text-traffic-close" : "text-ink-2";
  return (
    <div
      data-testid={`constraint-row-${constraint.id}`}
      data-conflicting={conflicting || undefined}
      onMouseEnter={() => sketchSelectionStore.getState().setConstraintHover(constraint.id)}
      onMouseLeave={() => sketchSelectionStore.getState().setConstraintHover(null)}
      className="group mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5 hover:bg-hover-2"
    >
      <span className={`w-4 shrink-0 text-center text-[12px] ${glyphTone}`}>
        {CONSTRAINT_PRESENTATION[constraint.type].glyph}
      </span>
      <span className={`flex-1 truncate text-[12.5px] ${typeTone}`}>{constraint.type}</span>
      <MonoValue className="shrink-0 text-[11px] text-ink-6">
        {entitySummary(constraint.entities)}
      </MonoValue>
      {value !== null && (
        <MonoValue className="shrink-0 text-[11.5px] text-ink-5">{value}</MonoValue>
      )}
      <DeleteButton
        title="Delete constraint"
        testid={`constraint-delete-${constraint.id}`}
        onClick={() => onDelete(constraint.id)}
      />
    </div>
  );
}

/** Per-row constraint list for the inspector SKETCH state. `conflictingIds` (SCHEMA
 *  §7.4, frontend ids) tint the matching rows red; defaults to none. */
export function ConstraintList({
  constraints,
  onDelete,
  conflictingIds = [],
}: {
  constraints: SketchConstraint[];
  onDelete: (id: string) => void;
  conflictingIds?: string[];
}) {
  const conflicting = new Set(conflictingIds);
  return (
    <div>
      {constraints.map((c) => (
        <ConstraintRow key={c.id} constraint={c} conflicting={conflicting.has(c.id)} onDelete={onDelete} />
      ))}
    </div>
  );
}
