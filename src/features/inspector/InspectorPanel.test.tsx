import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import { InspectorPanel } from "./InspectorPanel";
import { selectionStore } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { resetStores } from "@/test/resetStores";
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

const flush = () => new Promise((r) => setTimeout(r, 0));

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
      await flush();
    });

    expect(sketchStore.getState().session!.constraints.map((c) => c.id)).toEqual(["c1"]);
    expect(screen.queryByTestId("constraint-row-c2")).toBeNull();
  });
});
