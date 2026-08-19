import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConstraintContextChips } from "./ConstraintContextChips";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { sketchSelectionStore, type SketchSel } from "@/stores/sketchSelectionStore";
import { planeFor } from "@/ipc/mockSketch";
import { resetStores } from "@/test/resetStores";
import type { SketchEntity } from "@/ipc/types";

const twoLines: SketchEntity[] = [
  { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
  { id: "e2", type: "Line", p0: [0, 10], p1: [40, 12] },
];

function enterSketch(selected: SketchSel[]) {
  act(() => {
    toolStore.getState().setMode("sketch");
    sketchStore.getState().setSession({
      sketchId: "sk",
      plane: planeFor("XY"),
      entities: twoLines,
      constraints: [],
      dof: 0,
      status: "UnderConstrained",
    });
    sketchSelectionStore.getState().set(selected);
  });
}

describe("ConstraintContextChips", () => {
  beforeEach(() => resetStores());

  it("renders nothing when the selection is empty", () => {
    render(<ConstraintContextChips />);
    enterSketch([]);
    expect(screen.queryByTestId("constraint-context-chips")).toBeNull();
  });

  it("shows one clickable chip per applicable for a two-line selection", () => {
    render(<ConstraintContextChips />);
    enterSketch([{ entityId: "e1" }, { entityId: "e2" }]);
    // line & line ⇒ Distance, Parallel, Perpendicular, Angle.
    expect(screen.getByTestId("constraint-context-chips")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Parallel" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Angle" })).toBeInTheDocument();
  });

  // Design item 4d — each icon-only chip carries the constraint's display name
  // as a native title tooltip (no layout change).
  it("each chip's title is the constraint's display name", () => {
    render(<ConstraintContextChips />);
    enterSketch([{ entityId: "e1" }, { entityId: "e2" }]);
    expect(screen.getByRole("button", { name: "Parallel" })).toHaveAttribute("title", "Parallel");
    expect(screen.getByRole("button", { name: "Angle" })).toHaveAttribute("title", "Angle");
  });

  it("clicking a chip applies the constraint", async () => {
    const user = userEvent.setup();
    render(<ConstraintContextChips />);
    enterSketch([{ entityId: "e1" }, { entityId: "e2" }]);

    await user.click(screen.getByRole("button", { name: "Perpendicular" }));
    await waitFor(() => {
      expect(sketchStore.getState().session!.constraints.some((c) => c.type === "Perpendicular")).toBe(true);
    });
  });
});
