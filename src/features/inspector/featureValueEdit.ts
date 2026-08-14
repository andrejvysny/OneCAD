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
import { classifyRegen, failureReason } from "@/ipc/regenOutcome";
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
  // The USER's value, whatever `distanceType` says it means (SCHEMA §7.3). The
  // row edits the number, never the type — swapping Offset⇄Diameter changes the
  // operative closure the record froze, which is a tool gesture, not a scalar patch.
  OffsetFace: "distance",
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
 * Whether a feature's primary dimension may be BOUND to a document variable
 * (WP-VE.2), as opposed to merely edited as a number.
 *
 * Everything in {@link WIRE_FIELD} except `Hole`. A hole's own re-edit does NOT
 * go through the merge-patch lane below: `ModelToolController.commitHole`
 * rebuilds every `HoleParams` scalar from its FSM and wholesale-replaces the op,
 * so re-editing a hole's DEPTH would silently discard a binding on its DIAMETER —
 * a field the user never touched. Offering an affordance that an ordinary
 * follow-up gesture throws away is worse than not offering it, so the hole row
 * stays numbers-only until its tool carries bindings (WP-VE.2b).
 */
export function canBindFeatureValue(opType: string | undefined): boolean {
  return featureValueField(opType) !== null && opType !== "Hole";
}

/**
 * Commit one new primary value for a past feature.
 *
 * `value` is the DOCUMENT domain (mm for a length/diameter, degrees for an angle) —
 * exactly what `FeatureMeta.primaryValue` carries and what `DimensionInput` emits,
 * so nothing on this path converts units.
 *
 * `expr` is the document VARIABLE that should drive the field from now on
 * (WP-VE.2). `null`/omitted CLEARS any existing binding, which is what typing a
 * plain number into a bound field means. When it is set, `value` is still sent —
 * it is the `Scalar`'s cached last-evaluated number, and regen overwrites it with
 * the resolved one.
 *
 * Resolves `true` when the edit landed. A failure surfaces through the status hint
 * and resolves `false`, leaving the row on its previous value.
 */
export async function commitFeatureValue(
  featureId: string,
  opType: string,
  value: number,
  expr?: string | null,
): Promise<boolean> {
  const field = featureValueField(opType);
  if (field === null) return false;
  if (!Number.isFinite(value)) return false;
  if (expr != null && !canBindFeatureValue(opType)) return false;
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
        // A fresh object EVERY time, never a spread of the stored scalar: the
        // absence of `expr` is what clears a binding, so carrying the old one
        // forward would make an unbind impossible.
        [field]: expr != null ? { value, expr } : { value },
      }),
    );
    /*
     * VERDICT FIRST, hydrate second — `errorMessage` alone is not the failure
     * signal. A regen that publishes the rest of the timeline while THIS record
     * fails comes back with `terminal: "failed"` + `failedSteps` naming it and no
     * `errorMessage` at all (the U1 correlation in `tauriClient.applyOperation`),
     * so the old check hydrated that and reported success — a `=name` binding
     * that never took would have rendered as a bound row. Same shape as
     * `treeActions.ts`, which closed this hole for the metadata lane.
     *
     * `needsRepair` and `noop` KEEP the record (`keepsRecord`), so they are
     * successes here: the edit landed, and a downstream step needing repair is a
     * separate condition the repair panel owns.
     */
    const outcome = classifyRegen(res);
    const reason = failureReason(outcome);
    if (reason !== null) {
      // No hydrate on the failure path: the projection in the store still holds
      // the value the backend actually kept, which is the pre-edit one.
      errorHint(`Edit failed: ${reason}`);
      return false;
    }
    applyEditResult(res);
    viewportStore
      .getState()
      .setStatusHint(
        outcome.kind === "needsRepair" ? "Feature updated — downstream needs repair" : "Feature updated",
      );
    return true;
  } catch (e) {
    errorHint(`Edit failed: ${e instanceof Error ? e.message : String(e)}`);
    return false;
  }
}
