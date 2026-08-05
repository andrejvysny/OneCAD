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
import { DEFAULT_THEME } from "@/theme/themes";
import { DEFAULT_SNAP_RADIUS } from "@/tools/sketch/snapRadius";
import { DEFAULT_LENGTH_UNIT } from "@/units/lengthUnits";
import { DEFAULT_RENDER_MODE } from "@/viewport/engine/renderModes";
import { sketchStore } from "@/stores/sketchStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { liveDimStore } from "@/stores/liveDimStore";
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
    gridVisible: true,
    activeSketchId: null,
    cameraViewLabel: "TOP",
    fov: 76,
    cursor: { x: 273, y: 210, z: 0 },
    dofBadge: null,
    detectedInputDevice: "mouse",
    statusHint: null,
    pendingExtrudeSketch: null,
    isolatedBodyIds: null,
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
      // OFF here, ON in the app: the exact-coordinate pointer specs pin RAW
      // click coords, which a zoom-adaptive quantum would move. The live-dim
      // lane opts back in explicitly (SketchController.liveDim.test.ts).
      dimensionRound: false,
      // OFF here, ON in the app, for exactly the reason above — and one more:
      // the pointer specs' own `disableSnapping()` helpers enumerate the snap
      // keys they knew about when they were written, so a tier added later
      // would silently switch itself back on inside them. Polar re-creates the
      // H/V alignment those helpers deliberately disabled (its 0°/90° rays run
      // through the chain anchor), which moves pinned click coords. The polar
      // lane opts back in explicitly (SketchController.polar.test.ts).
      polarTracking: false,
      guidePoints3d: true,
      distantEdges: false,
    },
    show: { guidePoints: true, snappingHints: true, liveDimensions: true },
    navigation: { inputDevice: "auto" },
    displayMode: DEFAULT_RENDER_MODE,
    theme: DEFAULT_THEME,
    displayUnit: DEFAULT_LENGTH_UNIT,
    snapRadius: DEFAULT_SNAP_RADIUS,
  });
  sketchStore.getState().reset();
  toolChipStore.getState().clear();
  liveDimStore.getState().clear();
  workerStore.getState().reset();
  repairStore.getState().reset();
  resetMockSketches();
  resetMockDocument();
}
