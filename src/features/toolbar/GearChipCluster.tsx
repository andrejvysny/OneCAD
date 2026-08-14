/**
 * The armed GEAR floating chip (Gear Generator G1-h): `[Gear · 20T m2] [✓] [✕]`.
 *
 * Deliberately minimal. An earlier version crammed all 16 involute fields into
 * this one floating strip and it was unreadable (a single line stretching most
 * of the viewport width) — every field now lives in the right-sidebar
 * `GearPropertiesPanel` instead (FreeCAD's property-editor precedent), which has
 * the vertical room a form this size actually needs. This chip keeps only the
 * geometry-adjacent confirm/cancel affordance every other armed tool has;
 * `ModelToolChips.tsx` adds the shared `✓ ✕` pair around it.
 */

import { useToolChipStore } from "@/stores/toolChipStore";
import { gearValueText } from "@/tools/modelTools/gearMachine";

export function GearChipCluster() {
  const teeth = useToolChipStore((s) => s.count);
  const module = useToolChipStore((s) => s.value);

  return (
    <span data-testid="chip-gear-summary" className="px-2 py-1 text-[11.5px] font-medium text-ink-2">
      Gear · {gearValueText(teeth, module)}
    </span>
  );
}
