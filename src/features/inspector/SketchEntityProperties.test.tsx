/*
 * UX review 2026-09-14 — S12: clicking an arc selected it and surfaced a
 * two-icon unlabeled chip, but the inspector did not change (A-104). No radius,
 * no length, no coordinates, and no sign of the constraints already on it.
 */
import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { SketchEntityProperties } from "./SketchEntityProperties";
import { sketchStore } from "@/stores/sketchStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { settingsStore } from "@/stores/settingsStore";
import { resetStores } from "@/test/resetStores";
import type { SketchSession } from "@/ipc/types";

function session(): SketchSession {
  return {
    sketchId: "sketch2",
    plane: { kind: "XY", origin: [0, 0, 0], xAxis: [1, 0, 0], yAxis: [0, 1, 0], normal: [0, 0, 1] },
    entities: [
      { id: "l1", type: "Line", p0: [0, 0], p1: [30, 40] },
      { id: "c1", type: "Circle", center: [10, 20], radius: 5 },
      { id: "a1", type: "Arc", center: [0, 0], radius: 8, start: [8, 0], end: [0, 8] },
    ],
    constraints: [
      { id: "k1", type: "Horizontal", entities: ["l1"] },
      { id: "k2", type: "Radius", entities: ["c1"], value: 5 },
    ],
    dof: 3,
    status: "UnderConstrained",
  };
}

describe("SketchEntityProperties", () => {
  beforeEach(() => {
    resetStores();
    sketchSelectionStore.getState().clear();
    act(() => sketchStore.getState().setSession(session()));
  });

  it("renders the section with an explicit empty state when nothing is picked", () => {
    render(<SketchEntityProperties />);
    expect(screen.getByText("Entity")).toBeInTheDocument();
    expect(screen.getByTestId("sketch-entity-empty")).toBeInTheDocument();
  });

  it("names a selected line and reports its endpoints and length", () => {
    render(<SketchEntityProperties />);
    act(() => sketchSelectionStore.getState().set([{ entityId: "l1" }]));

    expect(screen.getByTestId("sketch-entity-name")).toHaveTextContent("Line 1");
    expect(screen.getByTestId("sketch-entity-prop-start")).toHaveTextContent("0 mm, 0 mm");
    expect(screen.getByTestId("sketch-entity-prop-end")).toHaveTextContent("30 mm, 40 mm");
    expect(screen.getByTestId("sketch-entity-prop-length")).toHaveTextContent("50 mm");
  });

  it("reports a circle's centre and radius and lists the constraints on it", () => {
    render(<SketchEntityProperties />);
    act(() => sketchSelectionStore.getState().set([{ entityId: "c1" }]));

    expect(screen.getByTestId("sketch-entity-name")).toHaveTextContent("Circle 1");
    expect(screen.getByTestId("sketch-entity-prop-center")).toHaveTextContent("10 mm, 20 mm");
    expect(screen.getByTestId("sketch-entity-prop-radius")).toHaveTextContent("5 mm");
    // Reuses ConstraintList's rows — the Radius row is the one that names c1.
    expect(screen.getByTestId("entity-constraint-row-k2")).toBeInTheDocument();
    expect(screen.queryByTestId("entity-constraint-row-k1")).toBeNull();
  });

  it("reports an arc's centre, radius and endpoints", () => {
    render(<SketchEntityProperties />);
    act(() => sketchSelectionStore.getState().set([{ entityId: "a1" }]));

    expect(screen.getByTestId("sketch-entity-name")).toHaveTextContent("Arc 1");
    expect(screen.getByTestId("sketch-entity-prop-radius")).toHaveTextContent("8 mm");
    expect(screen.getByTestId("sketch-entity-prop-start")).toHaveTextContent("8 mm, 0 mm");
    expect(screen.getByTestId("sketch-entity-prop-end")).toHaveTextContent("0 mm, 8 mm");
  });

  it("renders every value in the display unit", () => {
    render(<SketchEntityProperties />);
    act(() => {
      settingsStore.getState().setDisplayUnit("in");
      sketchSelectionStore.getState().set([{ entityId: "c1" }]);
    });
    expect(screen.getByTestId("sketch-entity-prop-radius")).toHaveTextContent("0.1969 in");
  });

  it("says so rather than guessing when the pick is not in the session", () => {
    render(<SketchEntityProperties />);
    act(() => sketchSelectionStore.getState().set([{ entityId: "gone" }]));
    expect(screen.getByTestId("sketch-entity-empty")).toBeInTheDocument();
  });
});
