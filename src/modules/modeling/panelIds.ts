/*
 * Modeling panel ids, split out from `ui.ts` on purpose.
 *
 * `ui.ts` imports every editor component, so anything that only needs an ID —
 * a workspace definition, a test, a future command palette — must import from
 * HERE. Importing `ui.ts` for an id would drag the whole editor chunk into the
 * startup bundle and undo the code split `App.tsx` documents.
 */
import { contributionId, type PanelId } from "@/platform";
import { MODELING_MODULE_ID } from "./manifest";

const panelId = (name: string) =>
  contributionId<PanelId>(MODELING_MODULE_ID, `onecad.modeling.panel.${name}`);

export const ModelingPanels = {
  ConstraintBadgeLayer: panelId("constraintBadgeLayer"),
  LiveDimChips: panelId("liveDimChips"),
  ConstraintContextChips: panelId("constraintContextChips"),
  ModelToolChips: panelId("modelToolChips"),
  MeasureOverlay: panelId("measureOverlay"),
  RepairMarkerOverlay: panelId("repairMarkerOverlay"),
  MeasurePanel: panelId("measurePanel"),
  FloatingToolbar: panelId("floatingToolbar"),
  SketchChromeBar: panelId("sketchChromeBar"),
  SketchConstraintToolbar: panelId("sketchConstraintToolbar"),
  ModelTree: panelId("modelTree"),
  Inspector: panelId("inspector"),
  RepairBanner: panelId("repairBanner"),
  TimelineStoppedBanner: panelId("timelineStoppedBanner"),
} as const;
