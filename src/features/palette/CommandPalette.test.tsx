/*
 * The palette as a keyboard surface: ↑/↓ move, Enter runs, Esc closes, and a
 * disabled entry stays visible with its reason instead of vanishing.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { contributionId, moduleId, type CommandId } from "@/platform";
import { renderWithPlatform, bootTestPlatform } from "@/test/renderWithPlatform";
import { paletteStore } from "@/stores/paletteStore";
import { resetStores } from "@/test/resetStores";
import { CommandPalette } from "./CommandPalette";

const OWNER = moduleId("onecad.demo");
const RUNNABLE = contributionId<CommandId>(OWNER, "onecad.demo.command.runnable");
const BLOCKED = contributionId<CommandId>(OWNER, "onecad.demo.command.blocked");

function renderPalette(onRun = vi.fn()) {
  const platform = bootTestPlatform();
  const scope = platform.createScope(OWNER);
  scope.registerCommand({
    id: RUNNABLE,
    title: "Zoom to fit selection",
    execute: () => {
      onRun();
      return { status: "done" as const };
    },
  });
  scope.registerCommand({
    id: BLOCKED,
    title: "Fillet edges",
    canExecute: () => ({ enabled: false, reason: "Requires one or more edges" }),
    execute: () => ({ status: "done" as const }),
  });
  return { ...renderWithPlatform(<CommandPalette />, { platform }), onRun };
}

describe("CommandPalette", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStores();
    paletteStore.setState({ open: true, query: "", index: 0 });
  });

  it("renders nothing while closed", () => {
    paletteStore.setState({ open: false });
    renderPalette();
    expect(screen.queryByTestId("command-palette")).not.toBeInTheDocument();
  });

  it("finds a registered command with no opt-in", async () => {
    const user = userEvent.setup();
    renderPalette();
    await user.type(screen.getByTestId("command-palette-input"), "zoom to fit sel");
    expect(screen.getByTestId(`palette-item-${RUNNABLE}`)).toBeInTheDocument();
  });

  it("keeps a disabled command visible and shows why", async () => {
    const user = userEvent.setup();
    renderPalette();
    await user.type(screen.getByTestId("command-palette-input"), "fillet edges");
    const row = screen.getByTestId(`palette-item-${BLOCKED}`);
    expect(row).toHaveAttribute("aria-disabled", "true");
    expect(row).toHaveTextContent("Requires one or more edges");
  });

  it("Enter runs the highlighted command and closes", async () => {
    const user = userEvent.setup();
    const { onRun } = renderPalette();
    await user.type(screen.getByTestId("command-palette-input"), "zoom to fit sel");
    await user.keyboard("{Enter}");
    expect(onRun).toHaveBeenCalledTimes(1);
    expect(paletteStore.getState().open).toBe(false);
  });

  it("Enter on a disabled command does nothing and stays open", async () => {
    const user = userEvent.setup();
    renderPalette();
    await user.type(screen.getByTestId("command-palette-input"), "fillet edges");
    await user.keyboard("{Enter}");
    expect(paletteStore.getState().open).toBe(true);
  });

  it("Escape closes", async () => {
    const user = userEvent.setup();
    renderPalette();
    await user.type(screen.getByTestId("command-palette-input"), "{Escape}");
    expect(paletteStore.getState().open).toBe(false);
  });

  it("arrow keys move the highlight without leaving the field", async () => {
    const user = userEvent.setup();
    renderPalette();
    const input = screen.getByTestId("command-palette-input");
    await user.click(input);
    await user.keyboard("{ArrowDown}");
    expect(paletteStore.getState().index).toBe(1);
    await user.keyboard("{ArrowUp}");
    expect(paletteStore.getState().index).toBe(0);
    expect(input).toHaveFocus();
  });

  it("says so when nothing matches", async () => {
    const user = userEvent.setup();
    renderPalette();
    await user.type(screen.getByTestId("command-palette-input"), "qqqqqq");
    expect(screen.getByTestId("command-palette")).toHaveTextContent("Nothing matches");
  });
});
