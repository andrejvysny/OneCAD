/*
 * History + repair edit actions (M4b) — the imperative glue from the inspector's
 * history rows + repair panel to the raw `EditCommand` surface.
 *
 * Each action dispatches ONE `client.applyEditCommand(...)` (or, for a rebind, a
 * promote_selection followed by an EditOperationInput) and hydrates the document
 * store from the correlated regen result (mirrors ModelToolController.applyResult).
 * Errors surface through the StatusBar hint (viewportStore.setStatusHint).
 */
import { createClient } from "@/ipc/client";
import { promoteOne } from "@/ipc/promote";
import {
  elementKindOfTopoKey,
  elementRefOfKind,
  inputPathFor,
  rebindInputCommand,
  removeOperationCommand,
  rollbackToCursorCommand,
  suppressOperationCommand,
} from "@/ipc/tauriCommandMap";
import { classifyRegen, failureReason, keepsRecord } from "@/ipc/regenOutcome";
import { toFeatureMeta } from "@/ipc/projectionHydration";
import type { ApplyOperationResult, NeedsRepairItem, ResolveCandidate } from "@/ipc/types";
import { parseRefId } from "@/ipc/tauriCommandMap";
import { documentStore, nextAppliedOps } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";

/**
 * Hydrate the document store from a regen result (bodies + feature timeline).
 *
 * Exported for the inline value editor (H3), which commits through the same raw
 * `applyEditCommand` lane and must hydrate identically — a second hand-rolled copy
 * is exactly how `statusMessage` silently dropped out of one path once already.
 */
export function applyEditResult(res: ApplyOperationResult): void {
  const doc = documentStore.getState();
  const bodies = { ...doc.bodies };
  let n = Object.keys(bodies).length;
  for (const ref of res.changedBodies ?? []) {
    if (!bodies[ref.bodyId]) bodies[ref.bodyId] = { id: ref.bodyId, name: `Body ${++n}`, visible: true };
  }
  for (const id of res.removedBodies ?? []) delete bodies[id];
  const features = res.features.map(toFeatureMeta);
  doc.applyChange({
    revision: res.revision,
    features,
    bodies,
    dirty: true,
    // H7b: the result CARRIES the cursor (both lanes), so a rollback lands here
    // immediately; `nextAppliedOps` is the fallback for a result without one.
    appliedOps:
      res.appliedOps ?? nextAppliedOps(doc.appliedOps, doc.features.length, features.length),
  });
}

/** Transient success confirmation (auto-dismisses). */
function hint(text: string): void {
  viewportStore.getState().setStatusHint(text);
}

/**
 * Hydrate, then report what the backend ACTUALLY did (WP0.3).
 *
 * Every history affordance used to hydrate and then announce success, catching
 * only a rejected promise — so a resolved `errorMessage` produced a success hint
 * on a document the regen had refused to change. The result is classified once,
 * here, and the hint follows the verdict.
 */
function settle(res: ApplyOperationResult, success: string, failurePrefix: string): void {
  const outcome = classifyRegen(res);
  if (keepsRecord(outcome)) applyEditResult(res);
  const reason = failureReason(outcome);
  if (reason !== null) {
    errorHint(`${failurePrefix}: ${reason}`);
  } else if (outcome.kind === "needsRepair") {
    // Not a failure: the record stands and the repair panel acts on it next.
    viewportStore.getState().setStatusHint(`${success} — needs repair`, { sticky: true });
  } else {
    hint(success);
  }
}

/** Sticky error hint — stays until the next status change so it stays readable. */
function errorHint(text: string): void {
  viewportStore.getState().setStatusHint(text, { severity: "error", sticky: true });
}

function errMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

// ── History-row affordances ────────────────────────────────────────────────

/**
 * Suppress / un-suppress a feature (`SetOperationSuppression`).
 *
 * NO optimistic flip: `FeatureDto.suppressed` is authoritative and rides every
 * projection, so the returned features array (hydrated by `applyEditResult`) IS
 * the dim state — including the CASCADE the backend applied, which a frontend
 * overlay could never have known about. The retired overlay also broke on reopen
 * (it started empty, so a persisted suppression could never be undone).
 */
export async function suppressFeature(opId: string, suppressed: boolean): Promise<void> {
  try {
    // Cascade is ONE-DIRECTIONAL: suppressing a step also suppresses everything
    // downstream that depends on it, but UN-suppressing must not cascade — that
    // would resurrect dependents the user deliberately suppressed on their own.
    const res = await createClient().applyEditCommand(suppressOperationCommand(opId, suppressed, suppressed));
    settle(res, suppressed ? "Feature suppressed" : "Feature unsuppressed", "Suppress failed");
  } catch (e) {
    errorHint(`Suppress failed: ${errMessage(e)}`);
  }
}

/**
 * Roll the timeline to a row (`SetRollback`). Cursor = applied op count, so
 * "roll to here" = `index + 1` (timeline.rs); rolling to the LAST row therefore
 * restores full history (cursor == length).
 */
export async function rollToIndex(index: number): Promise<void> {
  try {
    const res = await createClient().applyEditCommand(rollbackToCursorCommand(index + 1));
    settle(res, "Rolled timeline", "Rollback failed");
  } catch (e) {
    errorHint(`Rollback failed: ${errMessage(e)}`);
  }
}

/** Delete a feature permanently (`RemoveOperation`). */
export async function deleteFeature(opId: string): Promise<void> {
  try {
    const res = await createClient().applyEditCommand(removeOperationCommand(opId));
    settle(res, "Feature deleted", "Delete failed");
  } catch (e) {
    errorHint(`Delete failed: ${errMessage(e)}`);
  }
}

// ── Click-to-rebind (repair) ────────────────────────────────────────────────

/**
 * The body a repair item's feature operated on.
 *
 * H9: the backend now DERIVES this from the failing record's own inputs and ships
 * it on the `needs-repair` item (`NeedsRepairItemDto.body_id`), so this is the
 * answer whenever it is present. The frontend derivation below is the fallback
 * for an older backend / a step with no derivable body: a single-body document is
 * unambiguous, and with several bodies the operated one is genuinely ambiguous
 * UNLESS the user narrowed it by selecting exactly one body. A silent guess ("the
 * first") risks repairing against the wrong body, which is worse than refusing.
 */
function deriveOperatedBody(item: NeedsRepairItem): string | null {
  if (item.bodyId) return item.bodyId;
  const ids = Object.keys(documentStore.getState().bodies);
  if (ids.length === 0) return null;
  if (ids.length === 1) return ids[0];
  const selectedBodies = selectionStore.getState().selected.filter((r) => r.kind === "body");
  return selectedBodies.length === 1 ? selectedBodies[0].id : null;
}

/**
 * The `InputPath` that writes the slot a repair item names, or `null` when that
 * slot is not element-addressable. Fetches the op's stored params only for the
 * opTypes that need them, so the common repair path stays a single round trip:
 *
 *  - **Extrude** — its ToFace slots are conditional, so slot 0 could be either
 *    direction's target face.
 *  - **Chamfer** — SCHEMA §7.3 (WP-F) puts one reference-FACE slot per
 *    `referenceFaces` pair AFTER the `edgeIds.length` edge slots, and only
 *    `edgeIds` says where that boundary is. `seedEdgeId` (set only on a
 *    `legacyReferenceFace` item) is echoed back verbatim: it is the contour edge
 *    the CREATED pair is keyed by, and core refuses a create without it.
 *
 * See `inputPathFor` for both slot tables.
 */
export async function repairInputPath(
  item: NeedsRepairItem,
): Promise<ReturnType<typeof inputPathFor>> {
  const opType = documentStore.getState().features.find((f) => f.id === item.opId)?.opType;
  const index = parseRefId(item.refId)?.index ?? 0;
  if (opType !== "Extrude" && opType !== "Chamfer") return inputPathFor(opType, index);
  try {
    const params = await createClient().getOperationParams(item.opId);
    return inputPathFor(opType, index, params, item.seedEdgeId);
  } catch {
    // Without the params neither slot can be identified: an Extrude ToFace slot
    // cannot be told apart from its twin, and a Chamfer's trailing face slot cannot
    // be told apart from an edge slot. Guessing rebinds the wrong input.
    return null;
  }
}

/** The message shown when a repair item's slot has no `EditOperationInput` path. */
export const REPAIR_NOT_REBINDABLE =
  "This reference must be re-picked in the feature editor";

/**
 * Rebind a NeedsRepair input slot to a chosen candidate:
 *   (a) resolve the slot's `InputPath` from the op type + slot index
 *       (`inputPathFor` — mirrors the Rust wire slot table),
 *   (b) promote the candidate TopoKey → a minted ElementId (anchor = worldPos),
 *   (c) build the typed ElementRef, and
 *   (d) send `EditOperationInput{path}`.
 *
 * Returns false when it could not proceed (slot not rebindable / no body /
 * promotion refused). Never sends a doomed command: a slot with no path is
 * reported to the user instead.
 */
export async function rebindCandidate(
  item: NeedsRepairItem,
  candidate: ResolveCandidate,
  snapshotId?: number,
): Promise<boolean> {
  const path = await repairInputPath(item);
  if (!path) {
    errorHint(REPAIR_NOT_REBINDABLE);
    return false;
  }
  // The candidate's own authoritative body (denormalized from `ResolveRefResult.
  // bodyId` — see `ResolveCandidate.bodyId`) always wins over the item's stored
  // `bodyId`, which names the body a feature HISTORICALLY operated on and can be
  // stale by the time a candidate is resolved and picked.
  const bodyId = candidate.bodyId ?? deriveOperatedBody(item);
  if (!bodyId) {
    const bodyCount = Object.keys(documentStore.getState().bodies).length;
    errorHint(
      bodyCount > 1
        ? `Cannot repair: operated body is ambiguous (${bodyCount} bodies). Select the body this feature operated on first.`
        : "Cannot repair: no body to bind against",
    );
    return false;
  }
  const client = createClient();
  try {
    // `promoteOne` already hints when the candidate cannot be promoted (a stale
    // snapshot / unresolvable pick), so this arm only has to stop the repair.
    const promoted = await promoteOne(client, bodyId, {
      topoKey: candidate.topoKey,
      anchor: { worldPoint: candidate.worldPos },
    }, snapshotId);
    if (!promoted) return false;
    // A CHAMFER reference-face pair (SCHEMA §7.3/§9, WP-F) needs an anchor that is
    // an INTERIOR point of the chosen face. A candidate's `worldPos` is DISPLAY
    // evidence (the worker item and the Rust re-derivation put the face centre there
    // today, but an older item carried the seed EDGE's anchor, which lies on BOTH
    // adjacent faces and would tie the resolver's anchor rung between them on every
    // replay — exactly the ambiguity §7.3 makes the anchor non-optional to prevent).
    // Read the face centre live instead, and only fall back to `worldPos` when the query cannot
    // answer (an anchorless ref is refused by core, so a wrong-but-present anchor
    // is still the better failure than none).
    const worldPoint =
      path.path === "chamferReferenceFace"
        ? (await client
            .elementInfo(promoted.bodyId, promoted.elementId, candidate.topoKey)
            .catch(() => null))?.center ?? candidate.worldPos
        : candidate.worldPos;
    // The candidate's TopoKey decides `primary.kind` — a hole/shell slot binds a
    // FACE, and mislabelling it as an edge would poison the descriptor rung.
    const ref = elementRefOfKind(
      promoted.bodyId,
      promoted.elementId,
      elementKindOfTopoKey(candidate.topoKey),
      worldPoint,
    );
    const res = await client.applyEditCommand(rebindInputCommand(item.opId, path, ref));
    // A rebind that resolved to a failure must not report the reference repaired,
    // and must return false so the panel keeps the row.
    const outcome = classifyRegen(res);
    if (keepsRecord(outcome)) applyEditResult(res);
    const reason = failureReason(outcome);
    if (reason !== null) {
      errorHint(`Repair failed: ${reason}`);
      return false;
    }
    // Still NeedsRepair after a rebind means this slot took, another did not.
    hint(outcome.kind === "needsRepair" ? "Reference repaired — more remain" : "Reference repaired");
    return true;
  } catch (e) {
    errorHint(`Repair failed: ${errMessage(e)}`);
    return false;
  }
}
