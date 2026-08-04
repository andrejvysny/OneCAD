/*
 * Inline history-row dimension editing (HISTORY-HARDEN H3) — click a past
 * feature's value, type a new one, downstream rebuilds.
 *
 * DELIBERATELY NOT a tool session: there is no arm, no preview, no gesture. A
 * commit is one `UpdateOperationParams` carrying ONE changed scalar, merged into
 * the op's stored params so every non-scalar input (a revolve axis, a shell's open
 * faces, a fillet's edges, the profile) survives verbatim — the same contract the
 * double-click editors use, minus the viewport round-trip.
 *
 * The `wireField` table below is the ONE place a projection `primaryValue` maps
 * back onto the params key that produced it (`dto.rs feature_value`); it must be
 * read together with that match, since a key added on one side and not the other
 * makes a row show a number it cannot commit.
 */
import { createClient } from "@/ipc/client";
import { updateScalarParamsCommand } from "@/ipc/tauriCommandMap";
import type { FeatureMeta } from "@/stores/documentStore";
import type { ModelTool } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { applyEditResult } from "./historyActions";

/** The `opType` `updateScalarParamsCommand` accepts (the wire op-type union). */
type OpTypeParam = Parameters<typeof updateScalarParamsCommand>[1];

/** The params key an op's primary dimension lives under (SCHEMA §7.3 wire names). */
const WIRE_FIELD: Record<string, string> = {
  Extrude: "distance",
  // DEGREES on the wire as well as in the UI — a revolve angle is not a sketch
  // dimension, so `ipc/angleUnits` (the deg↔rad seam) has no business here.
  Revolve: "angleDeg",
  Fillet: "radius",
  // A chamfer's row edits d1 only; `distance2` (when set) rides the stored params.
  Chamfer: "radius",
  Shell: "thickness",
  Hole: "diameter",
};

/** The wire params field a feature's primary value commits into, or null. */
export function featureValueField(opType: string | undefined): string | null {
  // Not `(opType && WIRE_FIELD[opType]) ?? null`: `??` does not catch the empty
  // string an `opType`-less projection row falls back to, which would then read as
  // a legal field name and patch a params key called "".
  if (!opType) return null;
  return WIRE_FIELD[opType] ?? null;
}

/**
 * Whether a row's value may be edited in place right now. All three gates:
 *
 *  1. the model tool must be `select` — an armed tool owns the viewport gesture,
 *     and committing a history edit underneath it would race its own preview;
 *  2. the row must be APPLIED (`index < appliedOps`) — a draft beyond the rollback
 *     bar has no regen result to rebuild from, so editing it would change a number
 *     nothing recomputes;
 *  3. a SUPPRESSED row is read-only — its op is skipped, so an edit would silently
 *     do nothing visible.
 *
 * Plus the two structural preconditions: an op type this module can address, and a
 * projection that actually carries the primary value to seed the field with.
 */
export function canEditFeatureValue(
  item: FeatureMeta,
  index: number,
  appliedOps: number,
  modelTool: ModelTool,
): boolean {
  if (modelTool !== "select") return false;
  if (index >= appliedOps) return false;
  if (item.suppressed) return false;
  if (item.primaryValue === undefined) return false;
  return featureValueField(item.opType) !== null;
}

/** Sticky error hint — stays until the next status change so it stays readable. */
function errorHint(text: string): void {
  viewportStore.getState().setStatusHint(text, { severity: "error", sticky: true });
}

/**
 * Commit one new primary value for a past feature.
 *
 * `value` is the DOCUMENT domain (mm for a length/diameter, degrees for an angle) —
 * exactly what `FeatureMeta.primaryValue` carries and what `DimensionInput` emits,
 * so nothing on this path converts units.
 *
 * Resolves `true` when the edit landed. A failure surfaces through the status hint
 * and resolves `false`, leaving the row on its previous value.
 */
export async function commitFeatureValue(
  featureId: string,
  opType: string,
  value: number,
): Promise<boolean> {
  const field = featureValueField(opType);
  if (field === null) return false;
  if (!Number.isFinite(value)) return false;
  const client = createClient();
  try {
    // The stored params are what makes this a PATCH: `updateScalarParamsCommand`
    // shallow-merges one top-level key over them, so everything else round-trips.
    const stored = await client.getOperationParams(featureId);
    const res = await client.applyEditCommand(
      // The wire op-type union is module-private to `tauriCommandMap`; borrowing the
      // parameter's own type keeps this cast honest without widening that module's
      // public surface.
      updateScalarParamsCommand(featureId, opType as OpTypeParam, stored, {
        [field]: { value },
      }),
    );
    applyEditResult(res);
    if (res.errorMessage) {
      errorHint(`Edit failed: ${res.errorMessage}`);
      return false;
    }
    viewportStore.getState().setStatusHint("Feature updated");
    return true;
  } catch (e) {
    errorHint(`Edit failed: ${e instanceof Error ? e.message : String(e)}`);
    return false;
  }
}
