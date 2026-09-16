import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ModelOperationBar } from "./ModelOperationBar";
import { inspectorLayoutStore } from "@/stores/inspectorLayoutStore";
import { resetStores } from "@/test/resetStores";
import { toolChipStore } from "@/stores/toolChipStore";
import { documentStore } from "@/stores/documentStore";

describe("ModelOperationBar", () => {
  beforeEach(() => resetStores());

  it("keeps completion in stable chrome and opens inspector settings", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    act(() => toolChipStore.getState().showExtrude(20, [0, 0, 0], {
      onValue: () => undefined,
      onSymmetric: () => undefined,
      onConfirm,
      onCancel,
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
    inspectorLayoutStore.getState().setOpen(false);
    render(<ModelOperationBar />);

    expect(screen.getByTestId("model-operation-bar")).toHaveTextContent("Extrude");
    fireEvent.click(screen.getByTestId("model-operation-more"));
    expect(inspectorLayoutStore.getState().open).toBe(true);
    fireEvent.click(screen.getByTestId("model-operation-done"));
    fireEvent.click(screen.getByTestId("model-operation-cancel"));
    expect(onConfirm).toHaveBeenCalledOnce();
    expect(onCancel).toHaveBeenCalledOnce();
  });
});
