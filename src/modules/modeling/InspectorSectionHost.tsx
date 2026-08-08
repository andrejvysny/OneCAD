/*
 * Host for `InspectorContribution`s.
 *
 * `SlotHost` cannot be reused: it is bound to `platform.panels`, and an inspector
 * section is a different registry with a different admission rule (`canRender`
 * over a context, not a slot id).
 *
 * The host lives in `modules/modeling` rather than `platform/react` because
 * building the context means knowing what modeling's SELECTION and MODE are.
 * The platform only sees the result: opaque `SelectionRef`s and opaque scope
 * tokens.
 *
 * It subscribes to the stores that feed the context, not just to the registry.
 * `canRender` is a plain function — nothing re-evaluates it on its own — so a
 * host that only watched the registry would keep rendering last selection's
 * sections.
 */
import { useMemo } from "react";
import {
  ContributionBoundary,
  unsafeId,
  usePlatform,
  useRegistryEntries,
  type EntityId,
  type InspectorContext,
  type SelectionRef,
  type TypeId,
} from "@/platform";
import { useSelectionStore, type EntityRef } from "@/stores/selectionStore";
import { useToolStore } from "@/stores/toolStore";
import { MODELING_MODULE_ID, ModelingScopes } from "./manifest";

/**
 * Modeling `EntityRef` → platform `SelectionRef`.
 *
 * `subElement` carries the ELEMENT id only, never the topoKey: a topoKey is
 * snapshot-scoped evidence, and letting it stand in here would make an
 * unpromoted face look promoted to every consumer's `canRender`.
 */
function toSelectionRef(ref: EntityRef): SelectionRef {
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

export function useInspectorContext(): InspectorContext {
  const selected = useSelectionStore((s) => s.selected);
  const mode = useToolStore((s) => s.mode);
  return useMemo(
    () => ({
      selection: selected.map(toSelectionRef),
      scopes: [mode === "sketch" ? ModelingScopes.Sketch : ModelingScopes.Model],
    }),
    [selected, mode],
  );
}

/** Every registered section that admits the current context, in registry order. */
export function InspectorSectionHost() {
  const platform = usePlatform();
  const sections = useRegistryEntries(platform.inspector);
  const ctx = useInspectorContext();

  return (
    <>
      {sections
        .filter((s) => s.canRender(ctx))
        .map((s) => (
          <ContributionBoundary key={s.id} contributionId={s.id}>
            <s.component />
          </ContributionBoundary>
        ))}
    </>
  );
}
