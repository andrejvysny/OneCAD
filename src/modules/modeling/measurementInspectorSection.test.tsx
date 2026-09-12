import { describe, expect, it, beforeEach } from "vitest";
import { act, fireEvent, screen } from "@testing-library/react";
import { InspectorPanel } from "@/features/inspector/InspectorPanel";
import { measureStore } from "@/stores/measureStore";
import { toolStore } from "@/stores/toolStore";
import { inspectorLayoutStore } from "@/stores/inspectorLayoutStore";
import { resetStores } from "@/test/resetStores";
import { renderWithPlatform, bootTestPlatform } from "@/test/renderWithPlatform";
import { measureAdd, measureInit, measureSummary, type MeasurePick } from "@/tools/modelTools/measureTool";
import { ModelingInspectorPriorities, ModelingInspectorSections } from "./inspectorSectionIds";
import { contributeInspectorSections } from "./inspectorSections";
import { contributeModelingUi } from "./ui";
import { ModelingPanels } from "./panelIds";
import { Slots } from "@/platform";

function facePick(id: string, center: [number, number, number]): MeasurePick {
  return {
    bodyId: "body1",
    elementId: id,
    kind: "face",
    magnitude: 4800,
    center,
    curveType: -1,
    surfaceType: 0,
    normal: [0, 0, 1],
    hasNormal: true,
    radius: null,
  };
}

function pairSummary(a: MeasurePick, b: MeasurePick) {
  return measureSummary(measureAdd(measureAdd(measureInit(), a), b));
}

function renderMeasurementInspector() {
  act(() => toolStore.getState().setTool("measure"));
  return renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
}

describe("measurement inspector contribution", () => {
  beforeEach(() => {
    resetStores();
    measureStore.getState().clear();
  });

  it("registers ahead of selection sections and leaves MeasurePanel out of the viewport overlays", () => {
    const platform = bootTestPlatform(contributeModelingUi);
    expect(platform.inspector.get(ModelingInspectorSections.Measurement)).toMatchObject({
      priority: ModelingInspectorPriorities.Measurement,
    });
    expect(platform.panels.get(ModelingPanels.MeasurePanel)).toBeUndefined();
    expect(
      platform.panels
        .entries()
        .filter((panel) => panel.slot === Slots.ViewportOverlay)
        .map((panel) => [panel.id, panel.priority]),
    ).toEqual([
      [ModelingPanels.ConstraintBadgeLayer, 100],
      [ModelingPanels.SelectionDimensionLabels, 110],
      [ModelingPanels.LiveDimChips, 120],
      [ModelingPanels.ConstraintContextChips, 130],
      [ModelingPanels.ModelToolChips, 140],
      [ModelingPanels.MeasureOverlay, 150],
      [ModelingPanels.RepairMarkerOverlay, 160],
    ]);
  });

  it("is absent for an armed Measure tool until a pick exists", () => {
    renderMeasurementInspector();
    expect(screen.queryByTestId("measure-panel")).toBeNull();
  });

  it("shows one factual pick only while the Measure tool remains active", () => {
    measureStore.getState().set([facePick("face-a", [0, 0, 0])], null);
    renderMeasurementInspector();

    expect(screen.getByTestId("measure-pick-0")).toHaveTextContent("face");
    act(() => toolStore.getState().setTool("select"));
    expect(screen.queryByTestId("measure-panel")).toBeNull();
  });

  it("shows a pair reading in the inspector and remains mounted through collapse and a 280px width", () => {
    const a = facePick("face-a", [0, 0, 0]);
    const b = facePick("face-b", [0, 0, 30]);
    measureStore.getState().set([a, b], pairSummary(a, b));
    renderMeasurementInspector();

    const reading = screen.getByTestId("measure-panel");
    expect(screen.getByTestId("measure-panel-distance")).toBeInTheDocument();
    expect(reading).toHaveClass("w-full", "min-w-0", "max-w-full");
    expect(reading).not.toHaveClass("absolute", "pointer-events-none");

    act(() => inspectorLayoutStore.getState().setWidth(280));
    expect(screen.getByTestId("inspector-panel")).toHaveStyle({ width: "280px" });
    fireEvent.click(screen.getByTestId("inspector-drawer-toggle"));

    expect(screen.getByTestId("inspector-drawer-content")).toHaveAttribute("hidden");
    expect(screen.getByTestId("measure-panel")).toBe(reading);
    fireEvent.click(screen.getByTestId("inspector-drawer-toggle"));
    expect(screen.getByTestId("measure-panel-distance")).toBeInTheDocument();
  });
});
