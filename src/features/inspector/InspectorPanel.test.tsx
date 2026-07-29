import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import { InspectorPanel } from "./InspectorPanel";
import { selectionStore } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { documentStore } from "@/stores/documentStore";
import { setModelToolController } from "@/tools/modelTools/modelToolBridge";
import type { ModelToolController } from "@/tools/modelTools/ModelToolController";
import { resetStores } from "@/test/resetStores";
import { flushSketchMutations } from "@/tools/sketch/sketchService";
import type { SketchConstraint, SketchSession } from "@/ipc/types";

/** A live sketch session carrying the constraints the inspector summarizes. */
function sessionWithConstraints(constraints: SketchConstraint[]): SketchSession {
  return {
    sketchId: "sketch2",
    plane: { kind: "XY", origin: [0, 0, 0], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
    entities: [],
    constraints,
    dof: 3,
    status: "UnderConstrained",
  };
}

describe("InspectorPanel", () => {
  beforeEach(() => resetStores());

  it("shows the SELECTION state for the default sketch selection", () => {
    render(<InspectorPanel />);
    expect(screen.getByText("Sketch 2")).toBeInTheDocument();
    expect(screen.getByText("Sketch")).toBeInTheDocument();
    expect(screen.getByText("Under-constrained · DOF 3")).toBeInTheDocument();
    expect(screen.getByText("History")).toBeInTheDocument();
    expect(screen.getByText("83.3 mm")).toBeInTheDocument();
  });

  it("shows body status + full history when a body is selected", () => {
    render(<InspectorPanel />);
    act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));

    expect(screen.getByText("Body 1")).toBeInTheDocument();
    expect(screen.getByText("Solid body")).toBeInTheDocument();
    expect(screen.getByText("Sketch 1")).toBeInTheDocument();
    expect(screen.getByText("Fillet")).toBeInTheDocument();
    expect(screen.getByText("2.0 mm")).toBeInTheDocument();
    // A body is fully defined — no DOF line.
    expect(screen.queryByText("Under-constrained · DOF 3")).toBeNull();
  });

  it("shows the owning sketch when a filled region is selected", () => {
    render(<InspectorPanel />);
    act(() =>
      selectionStore.getState().set([
        {
          kind: "sketchRegion",
          id: '["sketch2","r_profile"]',
          sketchId: "sketch2",
          regionId: "r_profile",
        },
      ]),
    );

    expect(screen.getByText("Sketch 2")).toBeInTheDocument();
    expect(screen.getByText("Sketch profile")).toBeInTheDocument();
    expect(screen.getByText("Under-constrained · DOF 3")).toBeInTheDocument();
  });

  it("shows the EMPTY state when nothing is selected", () => {
    render(<InspectorPanel />);
    act(() => selectionStore.getState().clear());
    expect(screen.getByText("Nothing selected")).toBeInTheDocument();
  });

  it("shows the SKETCH state (DOF card + one row per live constraint) in sketch mode", () => {
    render(<InspectorPanel />);
    act(() => {
      toolStore.getState().setMode("sketch", "sketch2");
      // Live sketch session drives the CONSTRAINTS panel (no hardcoded demo data).
      sketchStore.getState().setSession(
        sessionWithConstraints([
          { id: "c1", type: "Coincident", entities: ["p1", "p2"] },
          { id: "c2", type: "Coincident", entities: ["p3", "p4"] },
          { id: "c3", type: "Coincident", entities: ["p5", "p6"] },
          { id: "c4", type: "Coincident", entities: ["p7", "p8"] },
          { id: "c5", type: "Horizontal", entities: ["l1"] },
          { id: "c6", type: "Distance", entities: ["p1", "p2"], value: 90 },
          { id: "c7", type: "Angle", entities: ["l1", "l2"], value: 30 },
        ]),
      );
    });

    expect(screen.getByText("Sketch 2")).toBeInTheDocument();
    expect(screen.getByText("Under-constrained · DOF 3")).toBeInTheDocument();
    expect(screen.getByText("Constraints")).toBeInTheDocument();
    // One row per constraint now (grouping/counting is gone) — 4 Coincident rows
    // plus Horizontal/Distance/Angle, each its own row with its own value column.
    expect(screen.getAllByTestId(/^constraint-row-/)).toHaveLength(7);
    expect(screen.getAllByText("Coincident")).toHaveLength(4);
    expect(screen.getByText("Horizontal")).toBeInTheDocument();
    expect(screen.getByText("Distance")).toBeInTheDocument();
    expect(screen.getByText("90")).toBeInTheDocument();
    expect(screen.getByText("Angle")).toBeInTheDocument();
    expect(screen.getByText("30°")).toBeInTheDocument();
    expect(
      screen.getByText(/degrees of freedom remain/),
    ).toBeInTheDocument();
  });

  it("shows the empty-constraints hint when the sketch has none", () => {
    render(<InspectorPanel />);
    act(() => {
      toolStore.getState().setMode("sketch", "sketch2");
      sketchStore.getState().setSession(sessionWithConstraints([]));
    });
    expect(screen.getByText("No constraints yet.")).toBeInTheDocument();
    expect(screen.queryByText("Coincident")).toBeNull();
  });

  /**
   * Re-edit routing runs off the authored `opType`, not `kind` — the backend folds
   * Chamfer into kind "fillet", so routing on `kind` alone opened a Chamfer in the
   * FILLET editor. Both rows below share kind "fillet" and must diverge here.
   */
  it("routes a history double-click on opType, so a Chamfer opens the chamfer editor", () => {
    const editEdgeOpFeature = vi.fn(() => Promise.resolve());
    setModelToolController({ editEdgeOpFeature } as unknown as ModelToolController);
    try {
      documentStore.setState({
        features: [
          { id: "f-ch", kind: "fillet", opType: "Chamfer", label: "Chamfer", valueText: "1.0 mm", status: "ok" },
          { id: "f-fi", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" },
        ],
      });
      render(<InspectorPanel />);
      act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));

      fireEvent.doubleClick(screen.getByTestId("history-row-f-ch"));
      expect(editEdgeOpFeature).toHaveBeenCalledWith("f-ch", "Chamfer");

      fireEvent.doubleClick(screen.getByTestId("history-row-f-fi"));
      expect(editEdgeOpFeature).toHaveBeenLastCalledWith("f-fi", "Fillet");
    } finally {
      setModelToolController(null);
    }
  });

  it("wires each row's delete button to deleteConstraints (re-solves + drops the row)", async () => {
    render(<InspectorPanel />);
    act(() => {
      toolStore.getState().setMode("sketch", "sketch2");
      sketchStore.getState().setSession(
        sessionWithConstraints([
          { id: "c1", type: "Coincident", entities: ["p1", "p2"] },
          { id: "c2", type: "Horizontal", entities: ["l1"] },
        ]),
      );
    });
    expect(screen.getByTestId("constraint-row-c2")).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByTestId("constraint-delete-c2"));
      // Drain the ACTUAL mutation queue rather than racing a fixed macrotask
      // (flaky under parallel worker-pool load — see useShortcuts.test.tsx).
      await flushSketchMutations();
    });

    expect(sketchStore.getState().session!.constraints.map((c) => c.id)).toEqual(["c1"]);
    expect(screen.queryByTestId("constraint-row-c2")).toBeNull();
  });
});
