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
  renameBodyCommand,
  renameSketchCommand,
  setBodyVisibilityCommand,
  setSketchVisibilityCommand,
  type WireEditCommand,
} from "@/ipc/tauriCommandMap";
import { toFeatureMeta } from "@/ipc/projectionHydration";
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

/**
 * Dispatch one metadata command with an optimistic local write. `revert` restores
 * the prior local state when the backend rejects — a silent revert would be worse
 * than the flip, so the failure is also surfaced as a sticky hint.
 */
async function dispatch(
  command: WireEditCommand,
  revert: () => void,
  failLabel: string,
): Promise<boolean> {
  try {
    applyEditResult(await createClient().applyEditCommand(command));
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
