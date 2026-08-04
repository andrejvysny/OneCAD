import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { RefObject } from "react";
import { SnapPopover } from "./SnapPopover";
import { settingsStore } from "@/stores/settingsStore";
import { resetStores } from "@/test/resetStores";

const anchorRef = { current: null } as RefObject<HTMLButtonElement | null>;

describe("SnapPopover", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStores();
  });

  it("renders SNAP TO + SHOW sections bound to settings", () => {
    render(<SnapPopover open onClose={() => {}} anchorRef={anchorRef} />);
    expect(screen.getByText("Snap to")).toBeInTheDocument();
    expect(screen.getByText("Show")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Grid" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  // Adding a row here without wiring it through SketchController.snapAt
  // re-creates the VF dead-toggle defect (a switch that lies about what it
  // does). `guidePoints3d`/`distantEdges` stay RESERVED in the store only.
  it("renders exactly the snap rows the snap path actually reads", () => {
    render(<SnapPopover open onClose={() => {}} anchorRef={anchorRef} />);
    const live = [
      "Grid",
      "Sketch guide lines",
      "Sketch guide points",
      "Quadrant points",
      "Intersections",
      "On-curve points",
      // SP-1: read by SketchController.dimQuantumNow / snapAt (rounding tier)
      // and by the live-dimension chip overlay.
      "Dimension rounding",
      "Live dimensions",
    ];
    for (const label of live) {
      expect(screen.getByRole("switch", { name: label })).toBeInTheDocument();
    }
    expect(
      screen.queryByRole("switch", { name: "3D guide points" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("switch", { name: "Distant edges" }),
    ).not.toBeInTheDocument();
  });

  it("persists a toggle to the store and localStorage", async () => {
    const user = userEvent.setup();
    render(<SnapPopover open onClose={() => {}} anchorRef={anchorRef} />);

    await user.click(screen.getByRole("switch", { name: "Grid" }));
    expect(settingsStore.getState().snapTo.grid).toBe(false);

    const raw = localStorage.getItem("onecad.settings");
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!).state.snapTo.grid).toBe(false);
  });

  it("persists a SHOW toggle", async () => {
    const user = userEvent.setup();
    render(<SnapPopover open onClose={() => {}} anchorRef={anchorRef} />);

    await user.click(screen.getByRole("switch", { name: "Snapping hints" }));
    expect(settingsStore.getState().show.snappingHints).toBe(false);
  });
});
