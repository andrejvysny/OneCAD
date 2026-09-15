/*
 * The inspector's ENTITY section for sketch mode (UX review 2026-09-14, S12).
 *
 * Clicking an arc selected it and surfaced a two-icon unlabeled chip, and the
 * inspector did not change at all (A-104): no radius, no length, no coordinates,
 * and no sign of the constraints already on it. Selection is the moment a user
 * asks "what is this?", and the panel had no answer.
 *
 * READ-ONLY on purpose. Editing a sketch dimension already has one authority —
 * the dimensional constraint rows (`SketchDimensionsSection` /
 * `setSketchDimension`) — and a second editable surface over the same numbers
 * would be a second way to move geometry without a constraint to hold it.
 *
 * UNCONDITIONAL, like the Constraints section beside it: the label renders with
 * an explicit empty state rather than appearing and disappearing with the pick,
 * so the panel does not reflow on every click (`inspectorContract.ts`, D14).
 */
import { SectionLabel } from "@/ui/SectionLabel";
import { MonoValue } from "@/ui/MonoValue";
import { useSketchStore } from "@/stores/sketchStore";
import { useSketchSelectionStore } from "@/stores/sketchSelectionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { entityDisplayName } from "@/features/sketch/entityNames";
import { formatLength, lengthSuffix } from "@/units/format";
import { createClient } from "@/ipc/client";
import { deleteConstraints } from "@/tools/sketch/sketchService";
import { ConstraintList } from "./ConstraintList";
import type { LengthUnitId } from "@/units/lengthUnits";
import type { SketchEntity } from "@/ipc/types";

/** Display word per entity kind — the same nouns `entityNames` ordinals use. */
const ENTITY_KIND_WORD: Readonly<Record<SketchEntity["type"], string>> = {
  Point: "Point",
  Line: "Line",
  Arc: "Arc",
  Circle: "Circle",
  Ellipse: "Ellipse",
};

/** One sketch length (document mm) in the display unit, with its symbol. */
function length(mm: number, unit: LengthUnitId): string {
  return `${formatLength(mm, unit)} ${lengthSuffix(unit)}`;
}

/** A plane point as two display-unit lengths — never a bare pair of numbers,
 *  which would silently change meaning when the display unit does. */
function point(p: readonly [number, number], unit: LengthUnitId): string {
  return `${length(p[0], unit)}, ${length(p[1], unit)}`;
}

/** The read-only rows for one entity, in the order a user reads them. */
function entityRows(entity: SketchEntity, unit: LengthUnitId): { id: string; label: string; value: string }[] {
  const rows: { id: string; label: string; value: string }[] = [];
  const push = (id: string, label: string, value: string) => rows.push({ id, label, value });
  if (entity.p0) push("start", "Start", point(entity.p0, unit));
  if (entity.p1) push("end", "End", point(entity.p1, unit));
  if (entity.p0 && entity.p1) {
    push("length", "Length", length(Math.hypot(entity.p1[0] - entity.p0[0], entity.p1[1] - entity.p0[1]), unit));
  }
  if (entity.center) push("center", entity.type === "Point" ? "Position" : "Centre", point(entity.center, unit));
  if (entity.radius !== undefined) push("radius", "Radius", length(entity.radius, unit));
  if (entity.majorR !== undefined) push("majorRadius", "Semi-major", length(entity.majorR, unit));
  if (entity.minorR !== undefined) push("minorRadius", "Semi-minor", length(entity.minorR, unit));
  if (entity.start) push("start", "Start", point(entity.start, unit));
  if (entity.end) push("end", "End", point(entity.end, unit));
  return rows;
}

/** Same fire-and-forget delete the Constraints section uses — the service
 *  re-solves and writes the session back, so the list re-renders off the store. */
function deleteConstraint(id: string): void {
  void deleteConstraints(createClient(), [id]);
}

export function SketchEntityProperties() {
  const session = useSketchStore((s) => s.session);
  const selected = useSketchSelectionStore((s) => s.selected);
  const conflictingIds = useSketchStore((s) => s.conflictingIds);
  const unit = useSettingsStore((s) => s.displayUnit);

  const entities = session?.entities ?? [];
  // The PRIMARY pick, matching `primarySelection` in model mode. A multi-pick
  // has no single set of properties to report, so it reports the first.
  const pick = selected[0];
  const entity = pick ? entities.find((e) => e.id === pick.entityId) : undefined;
  const label = <SectionLabel className="pb-1.5 pt-3.5">Entity</SectionLabel>;

  if (!entity) {
    return (
      <>
        {label}
        <div data-testid="sketch-entity-empty" className="text-[12px] leading-normal text-ink-6">
          {selected.length > 0
            ? "That selection is no longer in the sketch."
            : "Select geometry to see its properties."}
        </div>
      </>
    );
  }

  const referencing = (session?.constraints ?? []).filter((c) => c.entities.includes(entity.id));
  return (
    <>
      {label}
      <div data-testid="sketch-entity-properties" className="mb-1 rounded-sm bg-chip px-2.5 py-2">
        <div className="flex items-baseline gap-2">
          <span data-testid="sketch-entity-name" className="text-[12.5px] font-medium text-ink-2">
            {entityDisplayName(entities, entity.id) ?? ENTITY_KIND_WORD[entity.type]}
          </span>
          <span data-testid="sketch-entity-type" className="text-[11.5px] text-ink-5">
            {entity.referenceLocked ? "Projected reference" : entity.construction ? "Construction" : ENTITY_KIND_WORD[entity.type]}
          </span>
        </div>
        {entityRows(entity, unit).map((row) => (
          <div
            key={row.id}
            data-testid={`sketch-entity-prop-${row.id}`}
            className="mt-1 flex items-baseline justify-between gap-2 text-[12px]"
          >
            <span className="shrink-0 text-ink-5">{row.label}</span>
            <MonoValue className="truncate text-[11.5px] text-ink-2">{row.value}</MonoValue>
          </div>
        ))}
      </div>
      {referencing.length > 0 && (
        <ConstraintList
          constraints={referencing}
          entities={entities}
          onDelete={deleteConstraint}
          conflictingIds={conflictingIds}
          testIdPrefix="entity-"
        />
      )}
    </>
  );
}
