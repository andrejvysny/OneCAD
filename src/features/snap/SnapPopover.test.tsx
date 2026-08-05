import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
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
      // SP-5.5: read by SketchController.snapAt → computeSnap's polar tier.
      "Polar tracking",
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

  // The snap radius is NUMERIC, so it is a tablist and not a switch — asserted
  // separately so the switch-row pin above stays a statement about booleans only.
  it("renders the snap radius as a segmented control (not a switch)", () => {
    render(<SnapPopover open onClose={() => {}} anchorRef={anchorRef} />);
    const tablist = screen.getByRole("tablist", { name: "Snap radius" });
    expect(tablist).toBeInTheDocument();
    expect(
      screen.queryByRole("switch", { name: "Snap radius" }),
    ).not.toBeInTheDocument();
    expect(
      within(tablist)
        .getAllByRole("tab")
        .map((t) => t.textContent),
    ).toEqual(["S", "M", "L"]);
    expect(within(tablist).getByRole("tab", { name: "M" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("persists the snap radius to the store and localStorage", async () => {
    const user = userEvent.setup();
    render(<SnapPopover open onClose={() => {}} anchorRef={anchorRef} />);

    await user.click(screen.getByRole("tab", { name: "L" }));
    expect(settingsStore.getState().snapRadius).toBe("l");

    const raw = localStorage.getItem("onecad.settings");
    expect(JSON.parse(raw!).state.snapRadius).toBe("l");
  });

  it("persists the polar-tracking toggle", async () => {
    const user = userEvent.setup();
    // The APP default is on; the test lane resets it off (see resetStores), so
    // start from the app's state to assert the row actually turns it back off.
    settingsStore.getState().setSnap("polarTracking", true);
    render(<SnapPopover open onClose={() => {}} anchorRef={anchorRef} />);

    await user.click(screen.getByRole("switch", { name: "Polar tracking" }));
    expect(settingsStore.getState().snapTo.polarTracking).toBe(false);

    const raw = localStorage.getItem("onecad.settings");
    expect(JSON.parse(raw!).state.snapTo.polarTracking).toBe(false);
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
