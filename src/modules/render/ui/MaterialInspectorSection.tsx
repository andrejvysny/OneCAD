/*
 * The inspector's "Material" section — one for a selected body, one for a
 * selected face.
 *
 * WHERE THE SELECTION COMES FROM. A `ContributionComponent` is mounted with NO
 * props (`platform/contributions.ts`), so a section cannot be HANDED the
 * context its own `canRender` was asked about — it has to re-read it.
 * `useInspectorContext()` is the host's own builder for that value, and what it
 * returns is platform currency: opaque `SelectionRef`s and opaque scope tokens.
 * Render therefore reads the selection WITHOUT importing modeling's
 * `selectionStore` — the store belongs to modeling and a second module reaching
 * into it is the coupling ADR-0002 forbids. The seam left open is that the
 * import is still a modeling FILE rather than a platform service; closing it
 * means either props on inspector components or a platform selection service,
 * and this one line is where that change would land.
 *
 * WHAT EACH SECTION OFFERS, and why they are not one component:
 *   BODY — the body's own material, plus a count of the face overrides that
 *          currently win over it. Changing it goes through
 *          `assignBodyWithOverridePolicy`, so it raises the same keep/replace
 *          prompt a drop on the viewport does.
 *   FACE — the EFFECTIVE material (its own override, else the body's, else
 *          none) and a way to set or clear the override. Assigning here never
 *          prompts: a face override is exactly the thing the prompt is about,
 *          so authoring one is the user already having answered.
 *
 * A face's owning body is recovered from the composite selection id
 * (`${bodyId}#${topoKey}`, `stores/selectionStore.ts`) because the platform's
 * generic `SelectionRef` does not carry it. It is used for the INHERITED
 * lookup only — a face override is keyed by its globally-unique ElementId — so
 * the worst a failed parse can do is show "None" where it should have shown the
 * body's material. Nothing is ever written against a guessed body id.
 */
import { SectionLabel } from "@/ui/SectionLabel";
import { useInspectorContext } from "@/modules/modeling/InspectorSectionHost";

import type { MaterialId } from "../model/material";
import { renderStore } from "../store/renderStore";
import { colorToCss } from "./materialColor";
import { assignBodyWithOverridePolicy } from "./overridePolicy";
import { materialQuery, useRenderStore } from "./state";

/**
 * A stable empty array for the "no overrides" case.
 *
 * `useSyncExternalStore` compares snapshots by identity, so a selector that
 * builds `[]` on every call reports a change on every store read and React
 * re-renders until it gives up ("Maximum update depth exceeded" — this file hit
 * it exactly once, on the first run).
 */
const NO_OVERRIDES: readonly string[] = [];

/** `"body1#f:22"` → `"body1"`. `null` when the id is not a composite face ref. */
export function bodyIdFromFaceEntity(entityId: string): string | null {
  const hash = entityId.lastIndexOf("#");
  return hash > 0 ? entityId.slice(0, hash) : null;
}

function MaterialSelect({
  label,
  value,
  onChange,
  disabled,
}: {
  label: string;
  value: MaterialId | null;
  onChange: (next: MaterialId | null) => void;
  disabled: boolean;
}) {
  const library = useRenderStore((s) => s.state.library);
  const materials = Object.values(library);
  return (
    <div className="flex items-center gap-2">
      <span
        aria-hidden="true"
        className="h-3.5 w-3.5 flex-none rounded-[3px] border border-border-strong"
        style={{ background: colorToCss(value === null ? undefined : library[value]?.base.base_color) }}
      />
      <select
        aria-label={label}
        data-testid={`material-select-${label.toLowerCase().replace(/\s+/g, "-")}`}
        value={value ?? ""}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value === "" ? null : e.target.value)}
        className="h-[26px] min-w-0 flex-1 rounded-sm border border-border-strong bg-surface px-1 text-[12px] text-ink-2 outline-none focus:border-accent disabled:opacity-50"
      >
        <option value="">None</option>
        {materials.map((m) => (
          <option key={m.id} value={m.id}>
            {m.name}
          </option>
        ))}
      </select>
    </div>
  );
}

export function MaterialBodySection() {
  const ctx = useInspectorContext();
  const bodyId = ctx.selection[0]?.entityId ?? null;
  const readOnly = useRenderStore((s) => s.readOnly);
  const assigned = useRenderStore((s) =>
    bodyId === null ? null : (s.state.assignments.bodies[bodyId] ?? null),
  );
  const overrides = useRenderStore((s) =>
    bodyId === null ? NO_OVERRIDES : (s.boundFaceOverrides[bodyId] ?? NO_OVERRIDES),
  );

  if (bodyId === null) return null;

  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">Material</SectionLabel>
      <MaterialSelect
        label="Body material"
        value={assigned}
        disabled={readOnly}
        onChange={(next) => void assignBodyWithOverridePolicy(bodyId, next)}
      />
      {overrides.length > 0 && (
        <div className="mt-2 flex items-center gap-2 text-[11.5px] text-ink-5">
          <span data-testid="material-override-count" className="flex-1">
            {overrides.length} face override{overrides.length === 1 ? "" : "s"}
          </span>
          <button
            type="button"
            data-testid="material-override-clear"
            disabled={readOnly}
            onClick={() => void renderStore.getState().clearFaceOverrides(overrides)}
            className="cursor-pointer text-ink-5 hover:text-ink disabled:opacity-50"
          >
            Clear
          </button>
        </div>
      )}
    </>
  );
}

export function MaterialFaceSection() {
  const ctx = useInspectorContext();
  const ref = ctx.selection[0];
  const elementId = ref?.subElement ?? null;
  const bodyId = ref ? bodyIdFromFaceEntity(ref.entityId) : null;
  const readOnly = useRenderStore((s) => s.readOnly);
  const override = useRenderStore((s) =>
    elementId === null ? null : (s.state.assignments.faces[elementId] ?? null),
  );
  // Subscribed to the state object, so a body-level change re-renders the
  // inherited line too — `materialQuery` itself is a plain read.
  const inherited = useRenderStore((s) =>
    bodyId === null ? null : (s.state.assignments.bodies[bodyId] ?? null),
  );

  if (elementId === null) return null;
  const effectiveName =
    override !== null
      ? `Override: ${materialQuery.getMaterial(override)?.name ?? "missing material"}`
      : inherited !== null
        ? `From body: ${materialQuery.getMaterial(inherited)?.name ?? "missing material"}`
        : "None";

  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">Material</SectionLabel>
      <div data-testid="material-face-effective" className="mb-2 text-[11.5px] text-ink-5">
        {effectiveName}
      </div>
      <MaterialSelect
        label="Face override"
        value={override}
        disabled={readOnly}
        onChange={(next) => void renderStore.getState().assignFace(elementId, next)}
      />
    </>
  );
}
