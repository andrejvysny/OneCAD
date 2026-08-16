/*
 * ONE code path for "assign a material to a body".
 *
 * Four surfaces do it — the library panel, the inspector section, the viewport
 * drop target and the tree's context menu — and every one of them has to answer
 * the same question the same way: this body already carries face overrides, so
 * does the new body material go UNDER them or does it replace them?
 *
 * Getting that wrong is silent data loss in one direction (replacing overrides
 * the user authored deliberately) and a confusing no-op in the other (assigning
 * a material to a body whose visible faces all override it, so nothing on
 * screen changes). Four copies of the decision would eventually answer it four
 * ways, which is why the flow is here and the callers only supply the gesture.
 *
 * BOUND overrides, not every override in the document. `boundFaceOverrides` is
 * what the viewport actually resolved onto a face of THIS body — the persisted
 * map is face-keyed and global, so it is the only honest answer to "how many
 * overrides does this body have" (see `store/renderStore.ts`). Prompting on
 * unbound ones would ask the user about faces that are not on screen.
 *
 * Pure w.r.t. its dependencies: `deps` is injectable so a test drives both
 * branches without a dialog, a store or a document.
 */
import type { MaterialId } from "../model/material";
import { renderStore } from "../store/renderStore";
import { renderDialogStore, type OverrideChoice, type OverridePromptRequest } from "./dialogStore";
import { materialQuery } from "./state";

export interface OverridePolicyDeps {
  /** Face overrides the viewport bound to a face of `bodyId`. */
  boundOverrides(bodyId: string): readonly string[];
  /** Display name for a material id — `"None"` when the id is null/unknown. */
  materialName(id: MaterialId | null): string;
  assignBody(bodyId: string, materialId: MaterialId | null): Promise<void>;
  clearFaceOverrides(faceElementIds: readonly string[]): Promise<void>;
  /** Ask the user. Only ever called when there is something to ask about. */
  prompt(request: OverridePromptRequest): Promise<OverrideChoice>;
}

export type AssignOutcome =
  /** No overrides to ask about — assigned straight through. */
  | "assigned"
  /** Assigned; the face overrides were left standing. */
  | "kept-overrides"
  /** Assigned; the bound face overrides were cleared in the same gesture. */
  | "replaced-overrides"
  /** The user dismissed the prompt — nothing was written. */
  | "cancelled";

/** The live wiring: render's own store, its own query, its own dialog host. */
export function defaultOverridePolicyDeps(): OverridePolicyDeps {
  return {
    boundOverrides: (bodyId) => renderStore.getState().boundFaceOverrides[bodyId] ?? [],
    materialName: (id) => (id === null ? "None" : (materialQuery.getMaterial(id)?.name ?? "None")),
    assignBody: (bodyId, materialId) => renderStore.getState().assignBody(bodyId, materialId),
    clearFaceOverrides: (ids) => renderStore.getState().clearFaceOverrides(ids),
    prompt: (request) => renderDialogStore.getState().requestOverrideChoice(request),
  };
}

/**
 * Assign `materialId` (or `null` to unassign) to `bodyId`, asking about face
 * overrides first when there are any.
 *
 * "Replace" clears the BOUND ids only, in a second write. One write would be
 * better, but `renderStore` deliberately exposes no combined action and this
 * module must not grow a private mutation lane that skips the store's own
 * optimistic-then-persist path (ADR-0009). The window between the two is a
 * document that briefly still carries its overrides — visible, undoable, and
 * never a lost material.
 */
export async function assignBodyWithOverridePolicy(
  bodyId: string,
  materialId: MaterialId | null,
  deps: OverridePolicyDeps = defaultOverridePolicyDeps(),
  bodyLabel?: string,
): Promise<AssignOutcome> {
  const bound = deps.boundOverrides(bodyId);
  if (bound.length === 0) {
    await deps.assignBody(bodyId, materialId);
    return "assigned";
  }

  const choice = await deps.prompt({
    bodyId,
    bodyLabel,
    materialId,
    materialName: deps.materialName(materialId),
    overrideElementIds: [...bound],
  });
  if (choice === "cancel") return "cancelled";

  await deps.assignBody(bodyId, materialId);
  if (choice === "keep") return "kept-overrides";

  // Exactly the ids the prompt NAMED, not a fresh read: "Replace all" is an
  // answer to the question that was asked, and the set it was asked about is
  // the set the user agreed to lose. `clearFaceOverrides` no-ops on an id the
  // document no longer carries, so a stale member is harmless.
  await deps.clearFaceOverrides(bound);
  return "replaced-overrides";
}
