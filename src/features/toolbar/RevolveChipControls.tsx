/*
 * RevolveChipControls — the armed-revolve cluster's `⋯` overflow, holding the
 * New Body / Add / Cut segments (UNIFY-UX Phase 2). Mirrors ExtrudeChipControls'
 * `ExtrudeOverflow` / EdgeOpChipControls' `EdgeOpOverflow`: the collapsed chip
 * carries the angle and the Axis reset (both primary — Axis stays inline, only
 * the boolean mode collapses), everything else behind the button.
 */
import { ChipOverflow } from "@/features/toolbar/ChipOverflow";
import { BooleanModeSegments } from "@/features/toolbar/ExtrudeChipControls";
import type { BooleanMode } from "@/tools/modelTools/modelToolMachine";

export interface RevolveOverflowProps {
  booleanMode: BooleanMode;
  canBoolean: boolean;
  onBooleanMode: (mode: BooleanMode) => void;
}

/** Short label for the resolved boolean mode — the collapsed chip's readout. */
const MODE_LABEL: Record<BooleanMode, string> = {
  NewBody: "New",
  Add: "Add",
  Cut: "Cut",
  Intersect: "Intersect",
};

export function RevolveOverflow(props: RevolveOverflowProps): React.ReactElement {
  return (
    <ChipOverflow
      testId="chip-revolve-overflow"
      ariaLabel="Revolve options"
      title="Boolean mode"
      readout={MODE_LABEL[props.booleanMode]}
      marked={props.booleanMode !== "NewBody"}
    >
      <BooleanModeSegments
        active={props.booleanMode}
        canBoolean={props.canBoolean}
        onPick={props.onBooleanMode}
      />
    </ChipOverflow>
  );
}
