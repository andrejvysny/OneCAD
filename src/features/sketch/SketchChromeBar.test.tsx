import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SketchChromeBar } from "./SketchChromeBar";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore } from "@/stores/documentStore";
import { resetStores } from "@/test/resetStores";

describe("SketchChromeBar", () => {
  beforeEach(() => resetStores());

  it("is hidden in model mode", () => {
    render(<SketchChromeBar />);
    expect(screen.queryByText(/Editing/)).toBeNull();
  });

  it("shows the plane-pick prompt on bare sketch entry (no active sketch)", () => {
    render(<SketchChromeBar />);
    act(() => toolStore.getState().setMode("sketch"));

    expect(screen.getByText("Select a sketch plane")).toBeInTheDocument();
    // Pick variant has no editing pill / Finish button.
    expect(screen.queryByText(/Editing/)).toBeNull();
    expect(screen.queryByRole("button", { name: /Finish sketch/ })).toBeNull();
  });

  it("Cancel in the pick phase exits to model", async () => {
    const user = userEvent.setup();
    render(<SketchChromeBar />);
    act(() => toolStore.getState().setMode("sketch"));

    await user.click(screen.getByRole("button", { name: /Cancel/ }));
    expect(toolStore.getState().mode).toBe("model");
  });

  it("shows the editing pill and Finish drains the queue before requesting profile selection", async () => {
    const user = userEvent.setup();
    render(<SketchChromeBar />);
    act(() => toolStore.getState().setMode("sketch", "sketch2"));

    expect(screen.getByText("Editing Sketch 2")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Finish sketch/ }));

    // finishSketch (runAction) awaits the sketch mutation queue before flipping
    // mode + arming extrude — assert post-drain, not synchronously.
    await waitFor(() => expect(toolStore.getState().mode).toBe("model"));
    expect(viewportStore.getState().pendingExtrudeSketch).toBe("sketch2");
  });

  it("Cancel in the editing phase exits to model without arming extrude", async () => {
    const user = userEvent.setup();
    render(<SketchChromeBar />);
    act(() => toolStore.getState().setMode("sketch", "sketch2"));

    await user.click(screen.getByRole("button", { name: /Cancel/ }));
    expect(toolStore.getState().mode).toBe("model");
    expect(viewportStore.getState().pendingExtrudeSketch).toBeNull();
  });

  it("does not expose fabricated degrees of freedom without a current evaluation", () => {
    render(<SketchChromeBar />);
    act(() => toolStore.getState().setMode("sketch", "sketch2"));

    const status = screen.getByTestId("sketch-dof");
    expect(status).toHaveTextContent("Not evaluated");
    expect(status).not.toHaveAttribute("data-dof");
  });

  it("exposes degrees of freedom only for matching solver and geometry tokens", () => {
    render(<SketchChromeBar />);
    act(() => {
      const sketches = documentStore.getState().sketches;
      documentStore.setState({
        sketches: {
          ...sketches,
          sketch2: {
            ...sketches.sketch2,
            dof: 2,
            status: "under",
            geometryToken: "evaluation:2",
            solveGeometryToken: "evaluation:2",
          },
        },
      });
      toolStore.getState().setMode("sketch", "sketch2");
    });

    expect(screen.getByTestId("sketch-dof")).toHaveTextContent("DOF: 2");
    expect(screen.getByTestId("sketch-dof")).toHaveAttribute("data-dof", "2");
  });
});
