/*
 * The readout must say what the grid is actually DOING — a stale or unit-blind
 * number is worse than no legend at all, because it reads as a measurement.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import { GridScaleChip } from "./GridScaleChip";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import { resetStores } from "@/test/resetStores";

describe("GridScaleChip", () => {
  beforeEach(() => {
    resetStores();
  });

  it("shows one cell's size in the display unit", () => {
    act(() => viewportStore.getState().setGridStep(20));
    render(<GridScaleChip />);
    expect(screen.getByTestId("grid-scale")).toHaveTextContent("20 mm");
  });

  it("follows the step as the camera zooms", () => {
    render(<GridScaleChip />);
    act(() => viewportStore.getState().setGridStep(200));
    expect(screen.getByTestId("grid-scale")).toHaveTextContent("200 mm");
  });

  it("converts into the display unit", () => {
    act(() => viewportStore.getState().setGridStep(25.4));
    act(() => settingsStore.getState().setDisplayUnit("in"));
    render(<GridScaleChip />);
    expect(screen.getByTestId("grid-scale")).toHaveTextContent("1 in");
  });

  it("renders nothing while the grid is hidden", () => {
    act(() => viewportStore.getState().toggleGrid());
    render(<GridScaleChip />);
    expect(screen.queryByTestId("grid-scale")).toBeNull();
  });
});
