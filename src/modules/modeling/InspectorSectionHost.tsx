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
import { logError } from "@/debug/log";
import {
  ContributionBoundary,
  usePlatform,
  useRegistryEntries,
  type InspectorContext,
  type InspectorContribution,
} from "@/platform";
import { useSelectionStore } from "@/stores/selectionStore";
import { useToolStore } from "@/stores/toolStore";
import { scopeTokensFor, toSelectionRef } from "./selectionContext";

export function useInspectorContext(): InspectorContext {
  const selected = useSelectionStore((s) => s.selected);
  const mode = useToolStore((s) => s.mode);
  return useMemo(
    () => ({ selection: selected.map(toSelectionRef), scopes: scopeTokensFor(mode) }),
    [selected, mode],
  );
}

/**
 * `canRender` runs OUTSIDE the section's own `ContributionBoundary` — the
 * boundary can only catch what its children throw, and admission is decided
 * before there are any children. An unguarded predicate that throws would
 * therefore escape past the section and take out the whole inspector panel
 * through the shell's boundary. A throwing predicate is treated as "does not
 * admit", which is the safe reading: a section that cannot decide is a section
 * that does not render.
 */
function admits(section: InspectorContribution, ctx: InspectorContext): boolean {
  try {
    return section.canRender(ctx);
  } catch (err) {
    logError("err", `inspector canRender failed: ${section.id}`, {
      contributionId: section.id,
      message: err instanceof Error ? err.message : String(err),
    });
    return false;
  }
}

/** Every registered section that admits the current context, in registry order. */
export function InspectorSectionHost() {
  const platform = usePlatform();
  const sections = useRegistryEntries(platform.inspector);
  const ctx = useInspectorContext();

  return (
    <>
      {sections
        .filter((s) => admits(s, ctx))
        .map((s) => (
          <ContributionBoundary key={s.id} contributionId={s.id}>
            <s.component />
          </ContributionBoundary>
        ))}
    </>
  );
}
