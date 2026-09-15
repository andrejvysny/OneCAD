import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Tooltip } from "@/ui/Tooltip";

// The 450 ms dwell plus a React commit must fit inside RTL's wait even on a
// loaded machine (three vitest runs in parallel made the default 1 s flake).

describe("Tooltip", () => {
  it("is hidden until hovered", () => {
    render(
      <Tooltip label="Extrude (E)">
        <button type="button">anchor</button>
      </Tooltip>,
    );
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("appears on hover (after the open delay) and disappears on unhover", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip label="Extrude (E)">
        <button type="button">anchor</button>
      </Tooltip>,
    );
    await user.hover(screen.getByText("anchor"));
    expect(await screen.findByRole("tooltip", {}, { timeout: 3000 })).toHaveTextContent("Extrude (E)");
    await user.unhover(screen.getByText("anchor"));
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("renders when forced open", () => {
    render(
      <Tooltip label="Always" open>
        <span>anchor</span>
      </Tooltip>,
    );
    expect(screen.getByRole("tooltip")).toHaveTextContent("Always");
  });

  // ── C3/C4/C5 — dwell delay, dismissal, stacking above popovers ─────────────

  it("C4: does not appear the instant the cursor lands — only after a dwell delay", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip label="Extrude (E)">
        <button type="button">anchor</button>
      </Tooltip>,
    );
    await user.hover(screen.getByText("anchor"));
    // No `findBy` wait here — the assertion is precisely that it is NOT yet
    // there right after hover, so crossing the toolbar doesn't spam tooltips.
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    expect(await screen.findByRole("tooltip", {}, { timeout: 3000 })).toHaveTextContent("Extrude (E)");
  });

  it("C5: hides on a click anywhere, even without leaving the anchor", async () => {
    render(
      <Tooltip label="Extrude (E)">
        <button type="button">anchor</button>
      </Tooltip>,
    );
    fireEvent.mouseEnter(screen.getByText("anchor"));
    expect(await screen.findByRole("tooltip", {}, { timeout: 3000 })).toBeInTheDocument();
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("hides on Escape", async () => {
    render(
      <Tooltip label="Extrude (E)">
        <button type="button">anchor</button>
      </Tooltip>,
    );
    fireEvent.mouseEnter(screen.getByText("anchor"));
    expect(await screen.findByRole("tooltip", {}, { timeout: 3000 })).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("C3: renders above a popover's z-[100] stacking context", async () => {
    render(
      <Tooltip label="Extrude (E)">
        <button type="button">anchor</button>
      </Tooltip>,
    );
    fireEvent.mouseEnter(screen.getByText("anchor"));
    const tooltip = await screen.findByRole("tooltip", {}, { timeout: 3000 });
    expect(tooltip.className).toContain("z-[110]");
  });
});
