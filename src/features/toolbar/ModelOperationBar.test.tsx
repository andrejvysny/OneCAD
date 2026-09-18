import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ModelOperationBar } from "./ModelOperationBar";
import { inspectorLayoutStore } from "@/stores/inspectorLayoutStore";
import { resetStores } from "@/test/resetStores";
import { toolChipStore } from "@/stores/toolChipStore";
import { toolChipPlacementStore } from "@/stores/toolChipPlacementStore";
import { documentStore } from "@/stores/documentStore";

function armExtrude(handlers: { onConfirm?: () => void; onCancel?: () => void } = {}): void {
  act(() => toolChipStore.getState().showExtrude(20, [0, 0, 0], {
    onValue: () => undefined,
    onSymmetric: () => undefined,
    onConfirm: handlers.onConfirm ?? (() => undefined),
    onCancel: handlers.onCancel ?? (() => undefined),
  }));
  documentStore.setState({ sketches: { sketch1: { id: "sketch1", name: "Sketch 1", visible: true, geometryToken: "g1" } } });
  toolChipStore.getState().setContext("extrudeDepth", {
    tool: "extrudeDepth",
    kind: "profile",
    sketch: { sketchId: "sketch1" },
    regionIds: ["region1"],
    hostBodies: [],
    direction: { kind: "normal", vector: [0, 0, 1] },
  });
}

describe("ModelOperationBar", () => {
  beforeEach(() => resetStores());

  it("keeps completion in stable chrome and opens inspector settings", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    armExtrude({ onConfirm, onCancel });
    inspectorLayoutStore.getState().setOpen(false);
    render(<ModelOperationBar />);

    expect(screen.getByTestId("model-operation-bar")).toHaveTextContent("Extrude");
    fireEvent.click(screen.getByTestId("model-operation-more"));
    fireEvent.click(screen.getByTestId("model-operation-more-settings"));
    expect(inspectorLayoutStore.getState().open).toBe(true);
    fireEvent.click(screen.getByTestId("model-operation-done"));
    fireEvent.click(screen.getByTestId("model-operation-cancel"));
    expect(onConfirm).toHaveBeenCalledOnce();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("carries the operation modes and the result summary (spec §4.2)", () => {
    armExtrude();
    act(() => toolChipStore.getState().setResultSummary("Extrude · 1 new body"));
    render(<ModelOperationBar />);

    expect(screen.getByTestId("chip-mode-badge")).toHaveTextContent("New");
    expect(screen.getByTestId("chip-end-badge")).toHaveTextContent("Blind");
    expect(screen.getByTestId("chip-result-summary")).toHaveTextContent("Extrude · 1 new body");
  });

  it("states an invalid value briefly and blocks Done", () => {
    armExtrude();
    act(() =>
      toolChipStore.getState().setRangeValidation({ status: "invalid", draft: 9, message: "Too deep" }),
    );
    render(<ModelOperationBar />);

    expect(screen.getByTestId("tool-validation")).toHaveTextContent("Too deep");
    expect(screen.getByTestId("model-operation-done")).toBeDisabled();
  });

  /*
   * Spec §4.4: placement lives behind More, not in a header row on every label,
   * and "Show value in inspector" opens the drawer first — the dock host is
   * `hidden`/`inert` while it is closed, so docking into it would hide the only
   * primary field.
   */
  it("exposes placement through More and opens the drawer before docking", () => {
    armExtrude();
    inspectorLayoutStore.getState().setOpen(false);
    render(<ModelOperationBar />);

    fireEvent.click(screen.getByTestId("model-operation-more"));
    fireEvent.click(screen.getByTestId("model-operation-place-manual"));
    expect(toolChipPlacementStore.getState().placing).toBe(true);

    fireEvent.click(screen.getByTestId("model-operation-more"));
    fireEvent.click(screen.getByTestId("model-operation-place-inspector"));
    expect(inspectorLayoutStore.getState().open).toBe(true);
    expect(toolChipPlacementStore.getState().placement).toEqual({ mode: "docked" });
    expect(toolChipPlacementStore.getState().placing).toBe(false);

    fireEvent.click(screen.getByTestId("model-operation-more"));
    fireEvent.click(screen.getByTestId("model-operation-place-follow"));
    expect(toolChipPlacementStore.getState().placement).toEqual({ mode: "anchored" });
  });
});
