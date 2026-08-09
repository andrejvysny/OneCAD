/*
 * Modeling's selection, in the platform's currency.
 *
 * `EntityRef` is modeling's own shape and `SelectionRef` is what a contribution
 * sees; every host that hands a contribution a context needs this translation,
 * so it lives in one place rather than once per host. The inspector had it
 * first; the toolbar and the keyboard lane need the same thing, because a
 * contributed tool or command that gates on `ctx.selection`/`ctx.scopes` is
 * simply broken if the host hands it an empty context (Codex review, P2.5).
 *
 * Reading modeling's stores here is correct: this file IS modeling. What must
 * not happen is a generic host reaching for them, which is why the translation
 * is exported rather than inlined into `FloatingToolbar`.
 */
import { useMemo } from "react";
import { unsafeId, type EntityId, type SelectionRef, type ToolContext, type TypeId } from "@/platform";
import { selectionStore, useSelectionStore, type EntityRef } from "@/stores/selectionStore";
import { toolStore, useToolStore, type EditorMode } from "@/stores/toolStore";
import { MODELING_MODULE_ID, ModelingScopes } from "./manifest";

/**
 * Modeling `EntityRef` → platform `SelectionRef`.
 *
 * `subElement` carries the ELEMENT id only, never the topoKey: a topoKey is
 * snapshot-scoped evidence, and letting it stand in here would make an
 * unpromoted face look promoted to every consumer's `canRender`.
 */
export function toSelectionRef(ref: EntityRef): SelectionRef {
  // `unsafeId` rather than `contributionId`: these are RUNTIME ids minted by the
  // kernel and the selection store, not authored contribution ids, so there is no
  // reverse-DNS namespace to validate against.
  return {
    entityId: unsafeId<EntityId>(ref.id),
    owner: MODELING_MODULE_ID,
    typeId: unsafeId<TypeId>(ref.kind),
    ...(ref.elementId ? { subElement: ref.elementId } : {}),
  };
}

/** The scope token an editor mode presents as. */
export function scopeTokensFor(mode: EditorMode): readonly string[] {
  return [mode === "sketch" ? ModelingScopes.Sketch : ModelingScopes.Model];
}

/** The current context, read at call time — for non-React callers. */
export function modelingContext(): { selection: SelectionRef[]; scopes: readonly string[] } {
  return {
    selection: selectionStore.getState().selected.map(toSelectionRef),
    scopes: scopeTokensFor(toolStore.getState().mode),
  };
}

/** The same context, as a hook — re-derived whenever the selection or mode moves. */
export function useModelingToolContext(): ToolContext {
  const selected = useSelectionStore((s) => s.selected);
  const mode = useToolStore((s) => s.mode);
  return useMemo(
    () => ({ selection: selected.map(toSelectionRef), scopes: scopeTokensFor(mode) }),
    [selected, mode],
  );
}
