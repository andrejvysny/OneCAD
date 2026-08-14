/*
 * Model-tree edit actions (TRUST wave) — the imperative glue from the tree's eye
 * toggle + rename affordance to the raw `EditCommand` surface.
 *
 * Before this module the eye was a LOCAL zustand flip (`documentStore.setVisibility`)
 * and there was no rename at all, so both were lies: any rehydration
 * (`projection-updated`, reopen, a from-0 regen) silently restored the backend's
 * `visible: true` / generated `"Body N"`. The backend now owns both facts durably
 * (`src-tauri/tests/body_metadata.rs`), so every tree mutation goes through
 * `SetVisibility` / `RenameBody` / `RenameSketch`.
 *
 * Shape mirrors `features/inspector/historyActions.ts`: optimistic store write →
 * one `client.applyEditCommand(...)` → hydrate from the returned features, or
 * revert + a sticky error hint on rejection.
 */
import { createClient } from "@/ipc/client";
import {
  deleteDatumCommand,
  renameBodyCommand,
  renameSketchCommand,
  setBodyColorCommand,
  setFaceColorCommand,
  setBodyVisibilityCommand,
  setSketchVisibilityCommand,
  type WireEditCommand,
} from "@/ipc/tauriCommandMap";
import type { Rgba } from "@/ipc/types";
import { toFeatureMeta } from "@/ipc/projectionHydration";
import { classifyRegen, failureReason } from "@/ipc/regenOutcome";
import type { ApplyOperationResult } from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";

/** Hydrate the feature timeline from a metadata command's projection. */
function applyEditResult(res: ApplyOperationResult): void {
  documentStore.getState().applyChange({
    revision: res.revision,
    features: res.features.map(toFeatureMeta),
    dirty: true,
  });
}

function errorHint(text: string): void {
  viewportStore.getState().setStatusHint(text, { severity: "error", sticky: true });
}

function errMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** The current name of a tree row, or undefined when the id is unknown. */
function currentName(kind: "body" | "sketch", id: string): string | undefined {
  const s = documentStore.getState();
  return kind === "body" ? s.bodies[id]?.name : s.sketches[id]?.name;
}

/** Local name write (the optimistic half + the revert half of a rename). */
function writeName(kind: "body" | "sketch", id: string, name: string): void {
  const s = documentStore.getState();
  if (kind === "body") {
    const row = s.bodies[id];
    if (row) s.applyChange({ bodies: { ...s.bodies, [id]: { ...row, name } } });
  } else {
    const row = s.sketches[id];
    if (row) s.applyChange({ sketches: { ...s.sketches, [id]: { ...row, name } } });
  }
}

/** Local body-color write (the optimistic half + the revert half). */
function writeColor(id: string, color?: Rgba): void {
  const s = documentStore.getState();
  const row = s.bodies[id];
  if (!row) return;
  const next: typeof row = { ...row };
  if (color === undefined) delete next.color;
  else next.color = color;
  s.applyChange({ bodies: { ...s.bodies, [id]: next } });
}

/** Local face-color write (the optimistic half + the revert half). */
function writeFaceColor(id: string, elementId: string, color?: Rgba): void {
  const s = documentStore.getState();
  const row = s.bodies[id];
  if (!row) return;
  const next: typeof row = { ...row };
  const faces = { ...(next.faceColors ?? {}) };
  if (color === undefined) delete faces[elementId];
  else faces[elementId] = color;
  if (Object.keys(faces).length === 0) delete next.faceColors;
  else next.faceColors = faces;
  s.applyChange({ bodies: { ...s.bodies, [id]: next } });
}

/**
 * Dispatch one metadata command with an optimistic local write. `revert` restores
 * the prior local state when the backend rejects — a silent revert would be worse
 * than the flip, so the failure is also surfaced as a sticky hint.
 *
 * A REJECTION is not the only failure shape (U1): a regen can resolve carrying
 * `terminal:"failed"` + `errorMessage`, and this family used to read that as
 * success — the optimistic flip stood, no hint appeared, and the caller was told
 * `true`. `classifyRegen` is the one authority for that verdict.
 */
async function dispatch(
  command: WireEditCommand,
  revert: () => void,
  failLabel: string,
): Promise<boolean> {
  try {
    const res = await createClient().applyEditCommand(command);
    // These commands are `RegenHint::None` on the core: they publish a projection
    // and fire NO regen, so a result with no terminal and no bodies is their
    // normal success, not the body-count inference's "failure".
    const outcome = classifyRegen(res, { noTerminal: "published" });
    const reason = failureReason(outcome);
    if (reason !== null) {
      revert();
      errorHint(`${failLabel} failed: ${reason}`);
      return false;
    }
    // `published`, `noop` and `needsRepair` all keep the record; these commands are
    // metadata-only (`RegenHint::None`), so `noop` is their NORMAL success.
    applyEditResult(res);
    return true;
  } catch (e) {
    revert();
    errorHint(`${failLabel} failed: ${errMessage(e)}`);
    return false;
  }
}

// ── Visibility ──────────────────────────────────────────────────────────────

/** Show/hide a body (`SetVisibility{Body}`). Optimistic; reverts on rejection. */
export async function setBodyVisible(bodyId: string, visible: boolean): Promise<boolean> {
  const before = documentStore.getState().bodies[bodyId]?.visible;
  documentStore.getState().setVisibility(bodyId, visible); // meshSync reacts to the store
  return dispatch(
    setBodyVisibilityCommand(bodyId, visible),
    () => documentStore.getState().setVisibility(bodyId, before ?? true),
    visible ? "Show body" : "Hide body",
  );
}

/** Show/hide a sketch (`SetVisibility{Sketch}`). Optimistic; reverts on rejection. */
export async function setSketchVisible(sketchId: string, visible: boolean): Promise<boolean> {
  const before = documentStore.getState().sketches[sketchId]?.visible;
  documentStore.getState().setVisibility(sketchId, visible); // sketchStaticSync reacts
  return dispatch(
    setSketchVisibilityCommand(sketchId, visible),
    () => documentStore.getState().setVisibility(sketchId, before ?? true),
    visible ? "Show sketch" : "Hide sketch",
  );
}

/** Set or clear a body's authored color (`SetBodyColor`). Optimistic; reverts on rejection. */
export async function setBodyColor(bodyId: string, color: Rgba | null): Promise<boolean> {
  const before = documentStore.getState().bodies[bodyId]?.color;
  writeColor(bodyId, color ?? undefined);
  return dispatch(
    setBodyColorCommand(bodyId, color),
    () => writeColor(bodyId, before),
    color ? "Set body color" : "Reset body color",
  );
}

/** Set or clear a face color (`SetFaceColor`). Optimistic; reverts on rejection. */
export async function setFaceColor(
  bodyId: string,
  elementId: string,
  color: Rgba | null,
): Promise<boolean> {
  const before = documentStore.getState().bodies[bodyId]?.faceColors?.[elementId];
  writeFaceColor(bodyId, elementId, color ?? undefined);
  return dispatch(
    setFaceColorCommand(bodyId, elementId, color),
    () => writeFaceColor(bodyId, elementId, before),
    color ? "Set face color" : "Reset face color",
  );
}

// ── Rename ──────────────────────────────────────────────────────────────────

/**
 * A rename's accepted name, or null when it must be refused. An empty/whitespace
 * name would leave an unclickable blank row, and the backend has no notion of a
 * "default" to fall back to — refuse loudly rather than write it.
 */
function validName(raw: string): string | null {
  const name = raw.trim();
  return name.length > 0 ? name : null;
}

/** Rename a body (`RenameBody`). Refuses an empty/whitespace name. */
export async function renameBody(bodyId: string, raw: string): Promise<boolean> {
  const name = validName(raw);
  if (name === null) {
    errorHint("Rename failed: name cannot be empty");
    return false;
  }
  const before = currentName("body", bodyId);
  if (before === undefined) return false; // unknown row — nothing to rename
  if (before === name) return true; // no-op: never spend an undo step on it
  writeName("body", bodyId, name);
  return dispatch(renameBodyCommand(bodyId, name), () => writeName("body", bodyId, before), "Rename");
}

/** Rename a sketch (`RenameSketch`). Refuses an empty/whitespace name. */
export async function renameSketch(sketchId: string, raw: string): Promise<boolean> {
  const name = validName(raw);
  if (name === null) {
    errorHint("Rename failed: name cannot be empty");
    return false;
  }
  const before = currentName("sketch", sketchId);
  if (before === undefined) return false;
  if (before === name) return true;
  writeName("sketch", sketchId, name);
  return dispatch(
    renameSketchCommand(sketchId, name),
    () => writeName("sketch", sketchId, before),
    "Rename",
  );
}

// ── Delete ──────────────────────────────────────────────────────────────────

/**
 * Delete a sketch from the tree. This is NOT a metadata command: `DeleteSketch`
 * dirties the timeline (`RegenHint::ToEnd`) and keeps its own transport on the
 * `CadClient` — the same compensation path `SketchController` uses to clean up an
 * orphaned sketch. Refuses the sketch currently open for editing.
 */
export async function deleteSketch(sketchId: string): Promise<boolean> {
  if (viewportStore.getState().activeSketchId === sketchId) {
    errorHint("Cannot delete the sketch you are editing — exit the sketch first");
    return false;
  }
  try {
    await createClient().deleteSketch(sketchId);
    documentStore.getState().removeSketch(sketchId); // idempotent (real lane: projection wins)
    viewportStore.getState().setStatusHint("Sketch deleted");
    return true;
  } catch (e) {
    errorHint(`Delete sketch failed: ${errMessage(e)}`);
    return false;
  }
}

/**
 * Delete a datum plane (`DeleteDatum`, metadata-only — `RegenHint::None`).
 *
 * The backend REFUSES while any sketch is hosted on the datum, and the rejection
 * NAMES the blocking sketches — that message is surfaced verbatim rather than
 * paraphrased, because it is the only thing that tells the user what to delete
 * first.
 *
 * The local removal happens AFTER the round-trip, not optimistically before it:
 * both lanes resolve the datum by id at command time (the mock rejects an id it
 * cannot find), so removing first would turn every delete into a spurious
 * "unknown datum" and make the referenced-guard unreachable. Same order
 * `deleteSketch` above uses, for the same reason.
 */
export async function deleteDatum(datumId: string): Promise<boolean> {
  try {
    applyEditResult(await createClient().applyEditCommand(deleteDatumCommand(datumId)));
    documentStore.getState().removeDatum(datumId); // idempotent (real lane: projection wins)
    viewportStore.getState().setStatusHint("Datum deleted");
    return true;
  } catch (e) {
    errorHint(`Delete datum failed: ${errMessage(e)}`);
    return false;
  }
}
