import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SketchChromeBar } from "./SketchChromeBar";
import { toolStore } from "@/stores/toolStore";
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

  it("shows the editing pill for a targeted sketch and Finish exits to model", async () => {
    const user = userEvent.setup();
    render(<SketchChromeBar />);
    act(() => toolStore.getState().setMode("sketch", "sketch2"));

    expect(screen.getByText("Editing Sketch 2")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Finish sketch/ }));

    expect(toolStore.getState().mode).toBe("model");
    expect(screen.queryByText(/Editing/)).toBeNull();
  });

  it("Cancel in the editing phase exits to model", async () => {
    const user = userEvent.setup();
    render(<SketchChromeBar />);
    act(() => toolStore.getState().setMode("sketch", "sketch2"));

    await user.click(screen.getByRole("button", { name: /Cancel/ }));
    expect(toolStore.getState().mode).toBe("model");
  });
});
