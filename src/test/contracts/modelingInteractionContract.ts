/*
 * FROZEN modeling interaction contract — the TARGET interaction grammar every
 * model tool must conform to after the UX hardening program (spec b022edf7).
 *
 * This is NOT the current state. It is the normative target. Probe tests
 * compare production against it; they are RED until the corresponding WP
 * closes the gap. Once a WP lands, its rows go green. The contract itself
 * must not be edited to make a refactor pass — see ./README.md.
 *
 * Per tool, the contract declares:
 *   - primaryParameter: the main numeric value (if any)
 *   - previewFidelity: "kernelExact" | "localExact" | "ghost" | "none"
 *   - enterSupport: whether Enter commits
 *   - visibleCancel: whether a visible ✕ / Cancel control exists
 *   - clickAwayPolicy: "cancel" (click-away cancels focus, never commits)
 *   - requiredSelections: what must be selected before the tool arms
 *   - successSelection: which bodies are selected after a successful commit
 *   - reeditCapabilities: which stored fields survive a re-edit
 */

export type PreviewFidelity = "kernelExact" | "localExact" | "ghost" | "none";

export type SuccessSelectionPolicy =
  | "affectedExisting"
  | "newBodies"
  | "source"
  | "copies"
  | "allChildOutputs";

export interface ModelingInteractionRow {
  tool: string;
  primaryParameter: string | null;
  previewFidelity: PreviewFidelity;
  enterSupport: boolean;
  visibleCancel: boolean;
  clickAwayPolicy: "cancel";
  requiredSelections: string;
  successSelection: SuccessSelectionPolicy;
  reeditCapabilities: string;
}

export const MODELING_INTERACTION_CONTRACT: readonly ModelingInteractionRow[] = [
  {
    tool: "extrude",
    primaryParameter: "depth",
    previewFidelity: "kernelExact",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "sketch region",
    successSelection: "affectedExisting",
    reeditCapabilities: "all stored fields preserved",
  },
  {
    tool: "revolve",
    primaryParameter: "angle",
    previewFidelity: "kernelExact",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "sketch region + axis line",
    successSelection: "affectedExisting",
    reeditCapabilities: "all stored fields preserved",
  },
  {
    tool: "fillet",
    primaryParameter: "radius",
    previewFidelity: "kernelExact",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "edge(s)",
    successSelection: "affectedExisting",
    reeditCapabilities: "all stored fields preserved",
  },
  {
    tool: "chamfer",
    primaryParameter: "distance",
    previewFidelity: "kernelExact",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "edge(s)",
    successSelection: "affectedExisting",
    reeditCapabilities: "all stored fields preserved",
  },
  {
    tool: "shell",
    primaryParameter: "thickness",
    previewFidelity: "kernelExact",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "face(s)",
    successSelection: "affectedExisting",
    reeditCapabilities: "all stored fields preserved",
  },
  {
    tool: "boolean",
    primaryParameter: null,
    previewFidelity: "kernelExact",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "target body + tool body",
    successSelection: "allChildOutputs",
    reeditCapabilities: "operation swap only; target/tool preserved",
  },
  {
    tool: "offsetFace",
    primaryParameter: "distance",
    previewFidelity: "kernelExact",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "face(s)",
    successSelection: "affectedExisting",
    reeditCapabilities: "all stored fields preserved",
  },
  {
    tool: "hole",
    primaryParameter: "diameter",
    previewFidelity: "kernelExact",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "host face + point",
    successSelection: "affectedExisting",
    reeditCapabilities: "all stored fields preserved",
  },
  {
    tool: "linearPattern",
    primaryParameter: "spacing",
    previewFidelity: "ghost",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "source body",
    successSelection: "newBodies",
    reeditCapabilities: "axis + count + spacing + resultPolicyVersion + fuseResult preserved",
  },
  {
    tool: "circularPattern",
    primaryParameter: "angle",
    previewFidelity: "ghost",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "source body",
    successSelection: "newBodies",
    reeditCapabilities: "axis + axisOrigin + count + angle + resultPolicyVersion + fuseResult preserved",
  },
  {
    tool: "mirror",
    primaryParameter: null,
    previewFidelity: "ghost",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "source body",
    successSelection: "newBodies",
    reeditCapabilities: "plane + planePoint + fuseWithOriginal preserved",
  },
  {
    tool: "transform",
    primaryParameter: "value",
    previewFidelity: "localExact",
    enterSupport: true,
    visibleCancel: true,
    clickAwayPolicy: "cancel",
    requiredSelections: "body/bodies",
    successSelection: "copies",
    reeditCapabilities: "full placement (translate + rotate + center + copy) preserved",
  },
];
