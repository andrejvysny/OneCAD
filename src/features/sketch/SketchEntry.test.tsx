import { describe, it, expect, beforeEach } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FloatingToolbar } from "@/features/toolbar/FloatingToolbar";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { selectionStore } from "@/stores/selectionStore";
import { resetStores } from "@/test/resetStores";
import { renderWithPlatform } from "@/test/renderWithPlatform";

/**
 * F-WP6 sketch-entry-from-toolbar flow: clicking the model "New sketch" tool
 * enters sketch mode with NO target (bare intent) — activeSketchId stays null so
 * the controller shows the plane picker — and swaps the toolbar to the sketch
 * tool set with Line armed. The SketchController then picks this up (browser
 * only) to run the plane-pick → open-session flow.
 */
describe("sketch entry from the toolbar", () => {
  beforeEach(() => resetStores());

  it("enters sketch mode with no target (plane-pick phase)", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<FloatingToolbar />);
    expect(toolStore.getState().mode).toBe("model");
    const before = selectionStore.getState().selected;

    await user.click(screen.getByRole("button", { name: "New sketch" }));

    expect(toolStore.getState().mode).toBe("sketch");
    // Bare entry → no active sketch yet (the controller will show the picker).
    expect(viewportStore.getState().activeSketchId).toBeNull();
    // Selection is untouched by a bare entry (only an explicit id selects).
    expect(selectionStore.getState().selected).toBe(before);
    // Sketch tool set is shown, Line armed by default.
    expect(screen.getByRole("button", { name: "Line" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("button", { name: "Extrude" })).toBeNull();
  });
});
