import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConstraintMenu } from "./ConstraintMenu";
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

describe("ConstraintMenu", () => {
  beforeEach(() => resetStores());

  it("is hidden in model mode", () => {
    render(<ConstraintMenu />);
    expect(screen.queryByRole("button", { name: "Add constraint" })).toBeNull();
  });

  it("is hidden in sketch mode with no active session", () => {
    render(<ConstraintMenu />);
    act(() => toolStore.getState().setMode("sketch"));
    expect(screen.queryByRole("button", { name: "Add constraint" })).toBeNull();
  });

  it("renders a closed trigger; opening it shows all 20 constraint rows (icon + label), all disabled with no selection", async () => {
    const user = userEvent.setup();
    render(<ConstraintMenu />);
    enterSketch(twoLines, []);

    expect(screen.queryByRole("toolbar", { name: "Constraints" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Add constraint" }));

    const bar = screen.getByRole("toolbar", { name: "Constraints" });
    const buttons = bar.querySelectorAll("button");
    // 14 geometric + 6 dimensional (A11g adds H-/V-Distance).
    expect(buttons).toHaveLength(20);
    buttons.forEach((b) => expect(b).toBeDisabled());
    // Section headers (design item 4a) and a visible text label per row, not
    // just an icon-only glyph.
    expect(screen.getByText("Geometric")).toBeInTheDocument();
    expect(screen.getByText("Dimensional")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Parallel" })).toHaveTextContent("Parallel");
  });

  it("A11g: Horizontal/Vertical Distance are reachable from the Add-constraint menu", async () => {
    const user = userEvent.setup();
    render(<ConstraintMenu />);
    enterSketch(twoLines, []);
    await user.click(screen.getByRole("button", { name: "Add constraint" }));

    expect(screen.getByRole("button", { name: "Horizontal distance" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Vertical distance" })).toBeInTheDocument();
  });

  it("a disabled row's title explains what to select; an enabled row's title is just its label", async () => {
    const user = userEvent.setup();
    render(<ConstraintMenu />);
    enterSketch(twoLines, [{ entityId: "e1" }, { entityId: "e2" }]);
    await user.click(screen.getByRole("button", { name: "Add constraint" }));

    // Coincident is not applicable to a two-line selection — disabled, reason shown.
    expect(screen.getByRole("button", { name: "Coincident" })).toHaveAttribute(
      "title",
      "Select two points",
    );
    // Parallel IS applicable here — title is just the label.
    expect(screen.getByRole("button", { name: "Parallel" })).toHaveAttribute("title", "Parallel");
  });

  it("enables exactly the applicable kinds for a two-line selection", async () => {
    const user = userEvent.setup();
    render(<ConstraintMenu />);
    enterSketch(twoLines, [{ entityId: "e1" }, { entityId: "e2" }]);
    await user.click(screen.getByRole("button", { name: "Add constraint" }));

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
    render(<ConstraintMenu />);
    enterSketch(twoLines, [{ entityId: "e1" }, { entityId: "e2" }]);
    await user.click(screen.getByRole("button", { name: "Add constraint" }));

    await user.click(screen.getByRole("button", { name: "Parallel" }));

    await waitFor(() => {
      const s = sketchStore.getState().session!;
      expect(s.constraints.some((c) => c.type === "Parallel")).toBe(true);
    });
  });
});
