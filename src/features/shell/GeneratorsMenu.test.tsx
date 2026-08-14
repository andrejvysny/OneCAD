/*
 * `GeneratorsMenu` must be a projection of the tool registry's
 * `generatorsMenu`-flagged entries, not a hardcoded list — the whole point of
 * the design is that a future generator appears here with no edit to this
 * file. Boots the REAL modeling module (`renderWithPlatform`) so Gear's own
 * `generatorsMenu: true` flag is what proves it, not a test fixture.
 */
import { describe, it, expect, vi } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { addonId, contributionId, type ToolId } from "@/platform";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";
import { renderWithPlatform, bootTestPlatform } from "@/test/renderWithPlatform";
import { GeneratorsMenu } from "./GeneratorsMenu";

vi.mock("@/tools/activateTool", () => ({ activateTool: vi.fn(async () => {}) }));

describe("GeneratorsMenu", () => {
  it("lists Gear and nothing toolbar-only", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<GeneratorsMenu />);

    await user.click(screen.getByTestId("generators-menu-trigger"));
    expect(screen.getByRole("menuitem", { name: /Gear/ })).toBeInTheDocument();
    // Extrude has no `generatorsMenu` flag — it must not leak in here.
    expect(screen.queryByRole("menuitem", { name: /Extrude/ })).not.toBeInTheDocument();
  });

  it("activating a row calls the tool's activate() and closes the menu", async () => {
    const { activateTool } = await import("@/tools/activateTool");
    const user = userEvent.setup();
    renderWithPlatform(<GeneratorsMenu />);

    await user.click(screen.getByTestId("generators-menu-trigger"));
    await user.click(screen.getByTestId("generators-menu-onecad.modeling.tool.model.gear"));

    expect(activateTool).toHaveBeenCalledWith("gear");
    expect(screen.queryByRole("menu", { name: "Generators" })).not.toBeInTheDocument();
  });

  it("renders nothing when no tool has opted into the menu", () => {
    const platform = bootTestPlatform();
    // A real module's tools all carry the flag or don't; simulate "nobody has
    // opted in yet" the same way `WorkspaceSwitcher`'s empty-add-ons test does
    // — a fresh scope with no generator-flagged registration.
    const owner = addonId("com.example.empty");
    platform.createScope(owner).registerTool({
      id: contributionId<ToolId>(owner, "com.example.empty.tool.noop"),
      title: "Noop",
      activate: () => {},
      deactivate: () => {},
    });
    // Dispose modeling's own registration so its `gear` entry (which DOES carry
    // `generatorsMenu: true`) does not mask the empty-menu case under test.
    platform.scopeFor(MODELING_MODULE_ID).dispose();

    renderWithPlatform(<GeneratorsMenu />, { platform });
    expect(screen.queryByTestId("generators-menu-trigger")).not.toBeInTheDocument();
  });
});
