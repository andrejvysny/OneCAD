/*
 * SlotHost — renders every panel contribution placed in one slot.
 *
 * This is the inversion that removes `Platform → Modeling` imports: the shell
 * renders slots, modules register into them, and the shell never names a feature
 * component (docs/ARCHITECTURE.md §6).
 *
 * Order is the registry's deterministic order, not registration order
 * (ADR-0007), and each contribution is isolated by its own error boundary.
 *
 * Nothing is wrapped in a DOM node: these contributions are absolutely
 * positioned over the viewport and a wrapper element would change the stacking
 * they depend on. The mount-order contract is therefore probed through the
 * registry, not through the DOM.
 */
import { usePlatform, useRegistryEntries } from "./PlatformProvider";
import { ContributionBoundary } from "./ContributionBoundary";
import type { SlotId } from "../slots";
import type { PanelContribution } from "../contributions";

/** The contributions placed in `slot`, in deterministic order. */
export function useSlotContributions(slot: SlotId): readonly PanelContribution[] {
  const platform = usePlatform();
  const panels = useRegistryEntries(platform.panels);
  return panels.filter((p) => p.slot === slot);
}

export function SlotHost({ slot }: { slot: SlotId }) {
  const contributions = useSlotContributions(slot);
  return (
    <>
      {contributions.map((c) => (
        <ContributionBoundary key={c.id} contributionId={c.id}>
          <c.component />
        </ContributionBoundary>
      ))}
    </>
  );
}
