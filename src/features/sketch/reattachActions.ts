/*
 * Sketch REATTACH actions (H9) — the imperative glue behind the "Reattach…"
 * affordance on a sketch row (model tree) and a Sketch history row (inspector).
 *
 * One `client.reattachSketch(...)` per pick; the result hydrates the document
 * store through the SAME `applyEditResult` the other raw-edit lanes use, so a
 * reattach and a rebind can never drift in how they publish a regen. Errors
 * surface through the StatusBar hint.
 */
import { createClient } from "@/ipc/client";
import { applyEditResult } from "@/features/inspector/historyActions";
import { classifyRegen, failureReason } from "@/ipc/regenOutcome";
import type { SketchAttachTarget } from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";

/** Human label for a reattach target (status hints + menu rows). */
export function attachTargetLabel(target: SketchAttachTarget): string {
  if (target.kind === "world") return `${target.plane} plane`;
  return documentStore.getState().datums[target.datumId]?.name ?? "datum";
}

/**
 * Reattach `sketchId` onto `target` and hydrate the result.
 *
 * **The silent-drop hint.** The core's `UpdateSketchAttachment` carries an
 * anti-time-travel guard: when the sketch's timeline record sits AHEAD of the
 * body it is glued to (a legacy front-inserted record —
 * `backfill_missing_sketch_records`), the host-face stamp is dropped rather than
 * authoring an edge the timeline cannot honour. That drop is silent on the wire
 * — the command still succeeds and still publishes — so there is no flag to read
 * back. What IS observable is that nothing regenerated: a reattach that moved a
 * frame dirties from the sketch's producing step to the end, so a result with no
 * changed bodies AND no revision movement means the document did not move.
 * That is reported as a plain "nothing changed" hint rather than a false success.
 */
export const REATTACH_HOST_FACE_REFUSAL =
  "A sketch on a model face cannot be reattached — its geometry is projected in that face's frame";

export async function reattachSketch(
  sketchId: string,
  target: SketchAttachTarget,
): Promise<boolean> {
  // V1 refusal, enforced at the ACTION so BOTH entry points (tree row + history
  // row) are covered even though only one of them can see the attachment.
  if (documentStore.getState().sketches[sketchId]?.hostFace) {
    viewportStore
      .getState()
      .setStatusHint(REATTACH_HOST_FACE_REFUSAL, { severity: "error", sticky: true });
    return false;
  }
  const before = documentStore.getState().revision;
  try {
    const res = await createClient().reattachSketch(sketchId, target);
    // A resolved result is not a success (U1). This family decided the verdict
    // with its own `revision > before` heuristic, so a regen that bumped the
    // revision AND reported `failed` read as a successful reattach. Classify
    // first; the revision/body heuristic below then only distinguishes the
    // SILENT-DROP no-op described above, which is what it was written for.
    // No terminal ⇒ do not claim a failure: the silent-drop case below is exactly
    // a legitimate result with no changed bodies, and inference cannot tell the
    // two apart. The revision/body heuristic then reports it as the no-op it is.
    const outcome = classifyRegen(res, { noTerminal: "published" });
    const reason = failureReason(outcome);
    if (reason !== null) {
      viewportStore
        .getState()
        .setStatusHint(`Reattach failed: ${reason}`, { severity: "error", sticky: true });
      return false;
    }
    applyEditResult(res);
    const moved = res.revision > before || (res.changedBodies?.length ?? 0) > 0;
    if (moved) {
      viewportStore.getState().setStatusHint(`Sketch reattached to ${attachTargetLabel(target)}`);
    } else {
      viewportStore
        .getState()
        .setStatusHint("Reattach did not change the model — the sketch may precede its host in the timeline", {
          severity: "error",
          sticky: true,
        });
    }
    return moved;
  } catch (e) {
    viewportStore
      .getState()
      .setStatusHint(`Reattach failed: ${e instanceof Error ? e.message : String(e)}`, {
        severity: "error",
        sticky: true,
      });
    return false;
  }
}

/**
 * The sketch id behind a Sketch HISTORY row. The projection's `FeatureDto` carries
 * no sketch id (it is a record id), so the op's stored params are the only place
 * the link exists — `SketchOpParams::sketch` serializes as `sketchId`. Fetched on
 * demand (menu click), never per render.
 */
export async function sketchIdOfFeature(recordId: string): Promise<string | null> {
  try {
    const params = await createClient().getOperationParams(recordId);
    const id = params.sketchId;
    return typeof id === "string" && id.length > 0 ? id : null;
  } catch {
    return null;
  }
}
