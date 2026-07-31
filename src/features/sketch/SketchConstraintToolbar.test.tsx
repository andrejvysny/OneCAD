import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SketchConstraintToolbar } from "./SketchConstraintToolbar";
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

function enterSketch(entities: SketchEntity[], selected: SketchSel[]) {
  act(() => {
    toolStore.getState().setMode("sketch");
    sketchStore.getState().setSession({
      sketchId: "sk",
      plane: planeFor("XY"),
      entities,
      constraints: [],
      dof: 0,
      status: "UnderConstrained",
    });
    sketchSelectionStore.getState().set(selected);
  });
}

describe("SketchConstraintToolbar", () => {
  beforeEach(() => resetStores());

  it("is hidden in model mode", () => {
    render(<SketchConstraintToolbar />);
    expect(screen.queryByRole("toolbar", { name: "Constraints" })).toBeNull();
  });

  it("is hidden in sketch mode with no active session", () => {
    render(<SketchConstraintToolbar />);
    act(() => toolStore.getState().setMode("sketch"));
    expect(screen.queryByRole("toolbar", { name: "Constraints" })).toBeNull();
  });

  it("renders all 16 constraint buttons, all disabled with no selection", () => {
    // 12 geometric (GEOMETRIC_TYPES — includes the Tangent/Equal/Midpoint
    // frontend-extension additions) + 4 dimensional (DIMENSIONAL_TYPES).
    render(<SketchConstraintToolbar />);
    enterSketch(twoLines, []);
    const bar = screen.getByRole("toolbar", { name: "Constraints" });
    const buttons = bar.querySelectorAll("button");
    expect(buttons).toHaveLength(16);
    buttons.forEach((b) => expect(b).toBeDisabled());
  });

  it("enables exactly the applicable kinds for a two-line selection", () => {
    render(<SketchConstraintToolbar />);
    enterSketch(twoLines, [{ entityId: "e1" }, { entityId: "e2" }]);
    // line & line ⇒ Distance, Parallel, Perpendicular, Angle.
    expect(screen.getByRole("button", { name: "Parallel" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Perpendicular" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Distance" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Angle" })).toBeEnabled();
    // Not offered for two lines.
    expect(screen.getByRole("button", { name: "Coincident" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Radius" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Fixed" })).toBeDisabled();
  });

  it("clicking an enabled geometric button applies the constraint (lands in the session)", async () => {
    const user = userEvent.setup();
    render(<SketchConstraintToolbar />);
    enterSketch(twoLines, [{ entityId: "e1" }, { entityId: "e2" }]);

    await user.click(screen.getByRole("button", { name: "Parallel" }));

    await waitFor(() => {
      const s = sketchStore.getState().session!;
      expect(s.constraints.some((c) => c.type === "Parallel")).toBe(true);
    });
  });
});
