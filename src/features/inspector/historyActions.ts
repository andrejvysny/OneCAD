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
import {
  edgeElementRef,
  filletEdgeRebindCommand,
  removeOperationCommand,
  rollbackToCursorCommand,
  suppressOperationCommand,
} from "@/ipc/tauriCommandMap";
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
    // An `ApplyOperationResult` carries no timeline cursor — see `nextAppliedOps`.
    appliedOps: nextAppliedOps(doc.appliedOps, doc.features.length, features.length),
  });
}

/** Transient success confirmation (auto-dismisses). */
function hint(text: string): void {
  viewportStore.getState().setStatusHint(text);
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
    applyEditResult(res);
    hint(suppressed ? "Feature suppressed" : "Feature unsuppressed");
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
    applyEditResult(res);
    hint("Rolled timeline");
  } catch (e) {
    errorHint(`Rollback failed: ${errMessage(e)}`);
  }
}

/** Delete a feature permanently (`RemoveOperation`). */
export async function deleteFeature(opId: string): Promise<void> {
  try {
    const res = await createClient().applyEditCommand(removeOperationCommand(opId));
    applyEditResult(res);
    hint("Feature deleted");
  } catch (e) {
    errorHint(`Delete failed: ${errMessage(e)}`);
  }
}

// ── Click-to-rebind (repair) ────────────────────────────────────────────────

/**
 * Derive the body a repair item's feature operated on. SEAM: the projection has
 * no feature→body linkage. A single-body document is unambiguous; with several
 * bodies the operated one is genuinely ambiguous UNLESS the user has explicitly
 * narrowed it by selecting exactly one body — a silent guess (e.g. "the first")
 * risks repairing the wrong feature's reference, which is worse than refusing. A
 * follow-up needs the needs-repair item to carry its op's target body.
 */
function deriveOperatedBody(): string | null {
  const ids = Object.keys(documentStore.getState().bodies);
  if (ids.length === 0) return null;
  if (ids.length === 1) return ids[0];
  const selectedBodies = selectionStore.getState().selected.filter((r) => r.kind === "body");
  return selectedBodies.length === 1 ? selectedBodies[0].id : null;
}

/**
 * Rebind a NeedsRepair fillet edge to a chosen candidate:
 *   (a) promote the candidate TopoKey → a minted ElementId (anchor = worldPos),
 *   (b) build the typed edge ElementRef (primary {bodyId, elementId, kind:"edge"}
 *       + anchor.worldPoint), and
 *   (c) send `EditOperationInput{FilletEdges{index}}` (the backend-designated
 *       fillet-edge rebind — it rewrites BOTH `edge_ids[index]` and `edges[index]`
 *       in lockstep server-side; command.rs). `index` comes from the refId
 *       (`"<opId>.input<k>"`).
 * Returns false when it could not proceed (no body / already in flight upstream).
 */
export async function rebindCandidate(
  item: NeedsRepairItem,
  candidate: ResolveCandidate,
): Promise<boolean> {
  const bodyId = deriveOperatedBody();
  if (!bodyId) {
    const bodyCount = Object.keys(documentStore.getState().bodies).length;
    errorHint(
      bodyCount > 1
        ? `Cannot repair: operated body is ambiguous (${bodyCount} bodies). Select the body this feature operated on first.`
        : "Cannot repair: no body to bind against",
    );
    return false;
  }
  const index = parseRefId(item.refId)?.index ?? 0;
  const client = createClient();
  try {
    const [promoted] = await client.promoteSelection(bodyId, [
      { topoKey: candidate.topoKey, anchor: { worldPoint: candidate.worldPos } },
    ]);
    if (!promoted) {
      errorHint("Repair failed: could not promote candidate");
      return false;
    }
    const ref = edgeElementRef(promoted.bodyId, promoted.elementId, candidate.worldPos);
    const res = await client.applyEditCommand(filletEdgeRebindCommand(item.opId, index, ref));
    applyEditResult(res);
    hint("Reference repaired");
    return true;
  } catch (e) {
    errorHint(`Repair failed: ${errMessage(e)}`);
    return false;
  }
}
