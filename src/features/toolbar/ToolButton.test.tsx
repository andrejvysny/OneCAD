import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ToolButton } from "./ToolButton";

describe("ToolButton", () => {
  it("enabled: shows the 'Label (Shortcut)' tooltip and fires onClick", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(<ToolButton icon="fillet" label="Fillet / Chamfer" shortcut="F" active={false} onClick={onClick} />);
    const button = screen.getByRole("button", { name: "Fillet / Chamfer" });
    await user.hover(button);
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Fillet / Chamfer (F)");
    await user.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("disabled: grays out, is inert on click, and stays Tab-reachable", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(
      <ToolButton
        icon="fillet"
        label="Fillet / Chamfer"
        shortcut="F"
        active={false}
        onClick={onClick}
        disabled
        disabledReason="Select edges, then Fillet"
      />,
    );
    const button = screen.getByRole("button", { name: "Fillet / Chamfer" });
    expect(button.className).toContain("opacity-40");
    expect(button.className).toContain("cursor-not-allowed");
    expect(button).toHaveAttribute("aria-disabled", "true");

    await user.click(button);
    expect(onClick).not.toHaveBeenCalled();

    // Native `disabled` would drop the element from the tab order (tabIndex
    // -1); `aria-disabled` must not — this is the whole reason it was chosen.
    expect(button.tabIndex).toBe(0);
  });

  it("disabled: tooltip shows disabledReason verbatim, not 'Label (Shortcut)'", async () => {
    const user = userEvent.setup();
    render(
      <ToolButton
        icon="shell"
        label="Shell"
        shortcut="K"
        active={false}
        onClick={vi.fn()}
        disabled
        disabledReason="Select faces to remove, then Shell"
      />,
    );
    const button = screen.getByRole("button", { name: "Shell" });
    await user.hover(button);
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Select faces to remove, then Shell");
    expect(screen.queryByText("Shell (K)")).not.toBeInTheDocument();
  });

  // C11: a tool registered without a shortcut (e.g. Library) must not render
  // "Label ()" — the empty-shortcut case falls back to the bare label.
  it("C11: an empty shortcut renders the bare label, not 'Label ()'", async () => {
    const user = userEvent.setup();
    render(<ToolButton icon="select" label="Library" shortcut="" active={false} onClick={vi.fn()} />);
    const button = screen.getByRole("button", { name: "Library" });
    await user.hover(button);
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Library");
    expect(screen.queryByText("Library ()")).not.toBeInTheDocument();
  });
});
