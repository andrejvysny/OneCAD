import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FloatingToolbar } from "./FloatingToolbar";
import { ICON_MONO } from "@/icons/Icon";
import { resetStores } from "@/test/resetStores";
import { selectionStore } from "@/stores/selectionStore";

describe("FloatingToolbar", () => {
  beforeEach(() => resetStores());

  it("renders the model tool set with Select active by default", () => {
    render(<FloatingToolbar />);
    for (const name of ["Select", "New sketch", "Extrude", "Revolve", "Fillet / Chamfer", "Combine"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument();
    }
    // Sketch-only tools are absent in model mode.
    expect(screen.queryByRole("button", { name: "Line" })).toBeNull();
    expect(screen.getByRole("button", { name: "Select" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("collapses the icon to monochrome on the ACTIVE tool only", () => {
    // The active button paints its glyph `text-accent`. Without the override
    // the icon's own accent tone stays a second, near-identical blue and the
    // two-tone split turns to mush. Idle buttons are neutral ink and must keep
    // their accent, so the override must NOT be there.
    render(<FloatingToolbar />);
    expect(screen.getByRole("button", { name: "Select" }).className).toContain(
      ICON_MONO,
    );
    expect(screen.getByRole("button", { name: "Extrude" }).className).not.toContain(
      ICON_MONO,
    );
  });

  it("has no separate Chamfer button — the unified tool covers both (FILLET-CHAMFER-UNIFY W2)", () => {
    render(<FloatingToolbar />);
    expect(screen.queryByRole("button", { name: "Chamfer" })).toBeNull();
  });

  it("toggles the active tool on click", async () => {
    const user = userEvent.setup();
    render(<FloatingToolbar />);
    await user.click(screen.getByRole("button", { name: "Extrude" }));
    expect(screen.getByRole("button", { name: "Extrude" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Select" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("swaps to the sketch tool set when entering sketch mode", async () => {
    const user = userEvent.setup();
    render(<FloatingToolbar />);
    // The Model "New sketch" tool enters sketch mode.
    await user.click(screen.getByRole("button", { name: "New sketch" }));

    for (const name of ["Line", "Rectangle", "Circle", "Arc", "3-point arc", "Dimension", "Trim", "Extend", "Mirror"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument();
    }
    expect(screen.queryByRole("button", { name: "Extrude" })).toBeNull();
    // Sketch mode defaults to the Line tool.
    expect(screen.getByRole("button", { name: "Line" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    // Active tool toggles within sketch mode too.
    await user.click(screen.getByRole("button", { name: "Circle" }));
    expect(screen.getByRole("button", { name: "Circle" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Line" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("grays out gated tools by default (only sketch2 selected) but keeps Extrude/Revolve enabled via the sketch fallback", () => {
    render(<FloatingToolbar />);
    for (const name of ["Extrude", "Revolve", "Select", "New sketch", "Datum plane", "Hole", "Measure"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "false");
    }
    for (const name of ["Fillet / Chamfer", "Combine", "Shell", "Offset face", "Linear pattern", "Circular pattern", "Mirror", "Move"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "true");
    }
  });

  it("selecting a body enables the body-gated tools but not the edge/face-gated ones", () => {
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    render(<FloatingToolbar />);
    for (const name of ["Combine", "Linear pattern", "Circular pattern", "Mirror", "Move"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "false");
    }
    for (const name of ["Fillet / Chamfer", "Shell", "Offset face"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "true");
    }
  });

  it("selecting an edge enables only Fillet among the gated tools", () => {
    selectionStore.getState().set([{ kind: "edge", id: "e1", bodyId: "body1" }]);
    render(<FloatingToolbar />);
    expect(screen.getByRole("button", { name: "Fillet / Chamfer" })).toHaveAttribute("aria-disabled", "false");
    for (const name of ["Combine", "Shell", "Offset face", "Linear pattern", "Circular pattern", "Mirror", "Move"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "true");
    }
  });

  it("clicking a disabled button does not change the active tool", async () => {
    const user = userEvent.setup();
    render(<FloatingToolbar />);
    await user.click(screen.getByRole("button", { name: "Shell" }));
    expect(screen.getByRole("button", { name: "Select" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Shell" })).toHaveAttribute("aria-pressed", "false");
  });

  it("the ACTIVE tool is exempt from its own gray-out (must not disable mid-gesture)", async () => {
    const user = userEvent.setup();
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    render(<FloatingToolbar />);
    await user.click(screen.getByRole("button", { name: "Combine" }));
    expect(screen.getByRole("button", { name: "Combine" })).toHaveAttribute("aria-pressed", "true");
    // Clearing the selection mid-gesture would normally gray Combine out —
    // but it's the active tool, so it stays enabled.
    act(() => selectionStore.getState().clear());
    expect(screen.getByRole("button", { name: "Combine" })).toHaveAttribute("aria-disabled", "false");
  });
});
