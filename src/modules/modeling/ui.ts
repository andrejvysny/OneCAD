/*
 * Modeling's UI contributions.
 *
 * These used to be nineteen imports at the top of `EditorScreen.tsx`. They are
 * now registrations, so the shell no longer knows that Extrude, the model tree
 * or the sketch chrome exist (spec §195).
 *
 * ORDER IS LOAD-BEARING. These are absolutely-positioned siblings over the
 * viewport, so DOM order decides stacking inside a z-index band — a past defect
 * had tool chips render UNDER the side panels and become unclickable. Priorities
 * here reproduce the shipped mount order exactly, and
 * `src/test/contracts/shellContract.ts` is the gate.
 *
 * Registered when the EDITOR mounts, not at bootstrap: the editor tree is a
 * deliberate code-split chunk (see `App.tsx`), and pulling these imports into the
 * startup bundle to satisfy an architectural preference would make the start
 * screen pay for the editor.
 */
import { Slots, type ModuleScope } from "@/platform";
import { ModelingPanels } from "./panelIds";
import { contributeInspectorSections } from "./inspectorSections";
import { contributeModelingTree } from "./treeProvider";
import { createDatumViewportContribution } from "./datumViewport";

import { ConstraintBadgeLayer } from "@/features/sketch/ConstraintBadgeLayer";
import { SelectionDimensionLabels } from "@/features/sketch/SelectionDimensionLabels";
import { LiveDimChips } from "@/features/sketch/LiveDimChips";
import { ConstraintContextChips } from "@/features/sketch/ConstraintContextChips";
import { ModelToolChips } from "@/features/toolbar/ModelToolChips";
import { MeasureOverlay } from "@/features/measure/MeasureOverlay";
import { RepairMarkerOverlay } from "@/features/repair/RepairMarkerOverlay";
import { MeasurePanel } from "@/features/measure/MeasurePanel";
import { FloatingToolbar } from "@/features/toolbar/FloatingToolbar";
import { SketchChromeBar } from "@/features/sketch/SketchChromeBar";
import { ModelTreePanel } from "@/features/tree/ModelTreePanel";
import { VariablesPanel } from "@/features/inspector/VariablesPanel";
import { InspectorPanel } from "@/features/inspector/InspectorPanel";
import { RepairBanner } from "@/features/repair/RepairBanner";
import { TimelineStoppedBanner } from "@/features/repair/TimelineStoppedBanner";

export { ModelingPanels };

/** Registers modeling's editor UI into `scope`, in the shipped mount order. */
export function contributeModelingUi(scope: ModuleScope): void {
  const overlays = [
    { id: ModelingPanels.ConstraintBadgeLayer, component: ConstraintBadgeLayer },
    { id: ModelingPanels.SelectionDimensionLabels, component: SelectionDimensionLabels },
    { id: ModelingPanels.LiveDimChips, component: LiveDimChips },
    { id: ModelingPanels.ConstraintContextChips, component: ConstraintContextChips },
    { id: ModelingPanels.ModelToolChips, component: ModelToolChips },
    { id: ModelingPanels.MeasureOverlay, component: MeasureOverlay },
    { id: ModelingPanels.RepairMarkerOverlay, component: RepairMarkerOverlay },
    { id: ModelingPanels.MeasurePanel, component: MeasurePanel },
  ];
  overlays.forEach((p, i) =>
    scope.registerPanel({ ...p, slot: Slots.ViewportOverlay, priority: 100 + i * 10 }),
  );

  scope.registerPanel({
    id: ModelingPanels.FloatingToolbar,
    slot: Slots.ToolbarPrimary,
    priority: 100,
    component: FloatingToolbar,
  });

  scope.registerPanel({
    id: ModelingPanels.SketchChromeBar,
    slot: Slots.ToolbarContextual,
    priority: 100,
    component: SketchChromeBar,
  });

  scope.registerPanel({
    id: ModelingPanels.ModelTree,
    slot: Slots.ShellLeft,
    title: "Model",
    priority: 100,
    component: ModelTreePanel,
  });
  // Takes the sidebar-tab slot Library used to share (WP-1.4's original
  // pattern) — mounts AFTER ModelTree, per `shellContract.ts`'s pinned order.
  scope.registerPanel({
    id: ModelingPanels.VariablesPanel,
    slot: Slots.ShellLeft,
    title: "Variables",
    priority: 110,
    component: VariablesPanel,
  });
  scope.registerPanel({
    id: ModelingPanels.Inspector,
    slot: Slots.ShellRight,
    title: "Inspector",
    priority: 100,
    component: InspectorPanel,
  });

  // The inspector's own sections. Registered with the rest of the editor UI, so
  // they come and go with the panel that hosts them.
  contributeInspectorSections(scope);

  // Modeling's rows in the shared model tree. The panel renders providers, not
  // bodies/sketches/datums, so a second module can add its own section here.
  contributeModelingTree(scope);

  // Datum planes are an IN-SCENE layer, not a panel. Registered with the rest of
  // the editor UI so the engine's host attaches it as soon as both exist.
  scope.registerViewportContribution(createDatumViewportContribution());

  scope.registerPanel({
    id: ModelingPanels.RepairBanner,
    slot: Slots.ShellNotification,
    priority: 100,
    component: RepairBanner,
  });
  scope.registerPanel({
    id: ModelingPanels.TimelineStoppedBanner,
    slot: Slots.ShellNotification,
    priority: 110,
    component: TimelineStoppedBanner,
  });
}
