/*
 * Reset all F-WP3 chrome stores to their initial state. Stores are module
 * singletons, so tests call this in beforeEach to isolate. `setState` merges,
 * so store actions are preserved.
 */
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
import { settingsStore } from "@/stores/settingsStore";
import { sketchStore } from "@/stores/sketchStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { workerStore } from "@/stores/workerStore";
import { repairStore } from "@/stores/repairStore";
import { resetMockSketches, resetMockDocument } from "@/ipc/mockClient";

export function resetStores(): void {
  toolStore.setState({
    mode: "model",
    modelTool: "select",
    sketchTool: "line",
    phase: "idle",
  });
  selectionStore.setState({
    selected: [{ kind: "sketch", id: "sketch2" }],
    hover: null,
  });
  viewportStore.setState({
    projection: "persp",
    displayMode: "shaded",
    gridVisible: true,
    activeSketchId: null,
    cameraViewLabel: "TOP",
    fov: 76,
    cursor: { x: 273, y: 210, z: 0 },
    dofBadge: null,
    detectedInputDevice: "mouse",
    statusHint: null,
    pendingExtrudeSketch: null,
  });
  // Go through the action too, to cancel any pending auto-dismiss timer the
  // previous test armed (setState alone would leave the module timer running).
  viewportStore.getState().setStatusHint(null);
  documentStore.setState(seedMockDocument());
  settingsStore.setState({
    snapTo: {
      grid: true,
      sketchGuideLines: true,
      sketchGuidePoints: true,
      quadrant: true,
      intersection: true,
      onCurve: true,
      guidePoints3d: true,
      distantEdges: false,
    },
    show: { guidePoints: true, snappingHints: true },
    navigation: { inputDevice: "auto" },
  });
  sketchStore.getState().reset();
  toolChipStore.getState().clear();
  workerStore.getState().reset();
  repairStore.getState().reset();
  resetMockSketches();
  resetMockDocument();
}
