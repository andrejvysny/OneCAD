/*
 * Split-button flyout behavior (`ToolDefinition.flyout`).
 *
 * The visual is one button; the contract is three: the folded variants are NOT
 * direct buttons, the chip highlights when ANY family member is armed, and the
 * chip's plain click arms the family's default (last-picked, else first).
 */
import { describe, it, expect, beforeEach } from "vitest";
import { screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FloatingToolbar } from "./FloatingToolbar";
import { resetStores } from "@/test/resetStores";
import { renderWithPlatform } from "@/test/renderWithPlatform";
import { selectionStore } from "@/stores/selectionStore";

describe("tool flyouts", () => {
  beforeEach(() => resetStores());

  async function enterSketch(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByRole("button", { name: "New sketch" }));
  }

  it("a plain click on the chip arms the family default", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<FloatingToolbar />);
    await enterSketch(user);

    await user.click(screen.getByRole("button", { name: "Rectangle" }));
    expect(screen.getByRole("button", { name: "Rectangle" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("a variant is one click behind the chevron and activates from the menu", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<FloatingToolbar />);
    await enterSketch(user);

    await user.click(screen.getByRole("button", { name: "Rectangle options" }));
    const center = screen.getByRole("menuitem", { name: /Center rectangle/ });
    await user.click(center);

    // The chip follows the armed member and highlights the whole family.
    expect(screen.getByRole("button", { name: "Center rectangle" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.queryByRole("menuitem", { name: /Center rectangle/ })).toBeNull();
  });

  it("the picked member becomes the sticky default for later plain clicks", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<FloatingToolbar />);
    await enterSketch(user);

    await user.click(screen.getByRole("button", { name: "Circle options" }));
    await user.click(screen.getByRole("menuitem", { name: /Ellipse/ }));

    // Disarm the flyout family by arming another tool, then chip must again
    // be the Ellipse glyph — the remembered default, not Rectangle.
    await user.click(screen.getByRole("button", { name: "Line" }));
    expect(screen.getByRole("button", { name: "Ellipse" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(screen.queryByRole("button", { name: "Circle" })).toBeNull();
  });

  it("menu rows carry the member's shortcut glyph and gating", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<FloatingToolbar />);
    await enterSketch(user);

    await user.click(screen.getByRole("button", { name: "Arc options" }));
    expect(screen.getByRole("menuitem", { name: /3-point arc/ })).toHaveAttribute(
      "aria-disabled",
      "false",
    );
  });

  it("a disabled family member is inert in the menu until its gate opens", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<FloatingToolbar />);

    await user.click(screen.getByRole("button", { name: "Linear pattern options" }));
    expect(screen.getByRole("menuitem", { name: /Circular pattern/ })).toHaveAttribute(
      "aria-disabled",
      "true",
    );

    act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));
    expect(screen.getByRole("menuitem", { name: /Circular pattern/ })).toHaveAttribute(
      "aria-disabled",
      "false",
    );
  });
});