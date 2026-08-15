import { MonoValue } from "@/ui/MonoValue";
import { ICON_MONO, Icon } from "@/icons/Icon";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { CONSTRAINT_PRESENTATION } from "@/features/sketch/constraintCatalog";
import { useSettingsStore } from "@/stores/settingsStore";
import { formatLength, formatUnitless, lengthSuffix } from "@/units/format";
import type { LengthUnitId } from "@/units/lengthUnits";
import type { SketchConstraint, SketchConstraintType, SketchEntity } from "@/ipc/types";

/** Dimensional constraint kinds that carry a `value` column. Exported as
 *  {@link isDimensionalConstraint} for the inspector's sketch-dimension editor
 *  (H3b), which must offer a field for exactly this set. */
const DIMENSIONAL: ReadonlySet<SketchConstraintType> = new Set([
  "Distance",
  "HorizontalDistance",
  "VerticalDistance",
  "Angle",
  "Radius",
  "Diameter",
]);

/** Whether a constraint carries an editable dimensional value. */
export function isDimensionalConstraint(type: SketchConstraintType): boolean {
  return DIMENSIONAL.has(type);
}

/**
 * Dimensional value text. Angle is UI-domain degrees already (angleUnits.ts)
 * and never sees the length display unit; every other kind is a length stored
 * in mm, rendered in the display unit WITH its symbol — a naked number here
 * would silently change meaning when the unit does (WP-C2).
 */
function valueText(c: SketchConstraint, unit: LengthUnitId): string | null {
  if (!DIMENSIONAL.has(c.type) || typeof c.value !== "number") return null;
  if (c.type === "Angle") return `${formatUnitless(c.value)}°`;
  return `${formatLength(c.value, unit)} ${lengthSuffix(unit)}`;
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

/** One 30px constraint row: glyph · type · value · delete. A
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
  // Subscribed per row so a unit switch repaints the value column immediately;
  // the list is otherwise driven by the sketch session, which does not change.
  const value = valueText(constraint, useSettingsStore((s) => s.displayUnit));
  // Red primary + blue accent reads as neither, so a conflicting row collapses.
  const glyphTone = conflicting ? `text-traffic-close ${ICON_MONO}` : "text-ink-5";
  const typeTone = conflicting ? "text-traffic-close" : "text-ink-2";
  return (
    <div
      data-testid={`constraint-row-${constraint.id}`}
      data-conflicting={conflicting || undefined}
      onMouseEnter={() => sketchSelectionStore.getState().setConstraintHover(constraint.id)}
      onMouseLeave={() => sketchSelectionStore.getState().setConstraintHover(null)}
      className="group mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5 hover:bg-hover-2"
    >
      <span className={`flex w-4 shrink-0 justify-center ${glyphTone}`}>
        <Icon
          name={CONSTRAINT_PRESENTATION[constraint.type].icon}
          size={14}
          strokeWidth={1.7}
        />
      </span>
      <span className={`flex-1 truncate text-[12.5px] ${typeTone}`}>{constraint.type}</span>
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

/**
 * Machine `Fixed` rows minted by a host-face projection (SKETCH-ON-FACE W2).
 *
 * A sketch on a face opens with one `Fixed` per projected boundary point — four
 * rows for a rectangular face, dozens for a real one, none of them authored or
 * deletable by the user. They are hidden HERE, in the view only: `session.constraints`
 * keeps every one of them, because the marshaller's seeded-set == live-set invariant
 * (`sketchWireMap.ts` `seedIdMapFromWire`) reads a seeded constraint missing from the
 * live array as a deletion and emits a `removeConstraint` for it. Filtering upstream
 * would delete the pins on the first edit after entering the sketch.
 *
 * The DOF badge is likewise unaffected — it reports the solver's number, which
 * counts these constraints.
 */
function isMachineFixed(c: SketchConstraint, locked: ReadonlySet<string>): boolean {
  return c.type === "Fixed" && c.entities.length > 0 && c.entities.every((id) => locked.has(id));
}

/** The rows this list actually renders. Exported so the panel's "No constraints
 *  yet." empty state agrees with the list instead of rendering an empty box for a
 *  sketch whose only constraints are the hidden projection pins. */
export function visibleConstraints(
  constraints: SketchConstraint[],
  entities: SketchEntity[],
): SketchConstraint[] {
  const locked = new Set(entities.filter((e) => e.referenceLocked).map((e) => e.id));
  if (locked.size === 0) return constraints;
  return constraints.filter((c) => !isMachineFixed(c, locked));
}

/** Per-row constraint list for the inspector SKETCH state. `conflictingIds` (SCHEMA
 *  §7.4, frontend ids) tint the matching rows red; defaults to none. `entities` is
 *  the live session geometry, used only to recognise (and hide) the machine `Fixed`
 *  rows pinning locked reference geometry. */
export function ConstraintList({
  constraints,
  entities = [],
  onDelete,
  conflictingIds = [],
}: {
  constraints: SketchConstraint[];
  entities?: SketchEntity[];
  onDelete: (id: string) => void;
  conflictingIds?: string[];
}) {
  const conflicting = new Set(conflictingIds);
  const rows = visibleConstraints(constraints, entities);
  return (
    <div>
      {rows.map((c) => (
        <ConstraintRow key={c.id} constraint={c} conflicting={conflicting.has(c.id)} onDelete={onDelete} />
      ))}
    </div>
  );
}
