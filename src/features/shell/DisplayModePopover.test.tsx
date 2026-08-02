import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { RefObject } from "react";
import { DisplayModePopover } from "./DisplayModePopover";
import { settingsStore } from "@/stores/settingsStore";
import { resetStores } from "@/test/resetStores";

const anchorRef = { current: null } as RefObject<HTMLButtonElement | null>;

describe("DisplayModePopover", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStores();
  });

  it("renders one menuitemradio row per render mode, current mode checked", () => {
    render(<DisplayModePopover open onClose={() => {}} anchorRef={anchorRef} />);

    expect(screen.getByText("Display")).toBeInTheDocument();

    const shaded = screen.getByRole("menuitemradio", { name: "Shaded" });
    const shadedEdges = screen.getByRole("menuitemradio", { name: "Shaded + edges" });
    const wireframe = screen.getByRole("menuitemradio", { name: "Wireframe" });

    // resetStores() leaves displayMode at the default (shadedEdges).
    expect(shadedEdges).toHaveAttribute("aria-checked", "true");
    expect(shaded).toHaveAttribute("aria-checked", "false");
    expect(wireframe).toHaveAttribute("aria-checked", "false");
  });

  it("clicking a row sets displayMode and closes the popover", async () => {
    const user = userEvent.setup();
    let closed = false;
    render(
      <DisplayModePopover
        open
        onClose={() => {
          closed = true;
        }}
        anchorRef={anchorRef}
      />,
    );

    await user.click(screen.getByRole("menuitemradio", { name: "Wireframe" }));

    expect(settingsStore.getState().displayMode).toBe("wireframe");
    expect(closed).toBe(true);
  });

  it("re-renders with the new mode checked after a selection", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <DisplayModePopover open onClose={() => {}} anchorRef={anchorRef} />,
    );

    await user.click(screen.getByRole("menuitemradio", { name: "Shaded" }));
    rerender(<DisplayModePopover open onClose={() => {}} anchorRef={anchorRef} />);

    expect(screen.getByRole("menuitemradio", { name: "Shaded" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByRole("menuitemradio", { name: "Wireframe" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });
});
