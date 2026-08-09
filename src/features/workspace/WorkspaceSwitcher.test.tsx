/*
 * The switcher must be a projection of the workspace REGISTRY, not a list.
 *
 * That is the whole claim of the modular shell: a module or add-on registers a
 * workspace and it appears, with no edit to this component. A test that seeded
 * the registry with fakes would prove the rendering; these boot the REAL shell
 * module so the shipped workspaces are what shows up.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  addonId,
  contributionId,
  createPlatform,
  Slots,
  type Platform,
  type WorkspaceId,
} from "@/platform";
import { registerShellModule } from "@/modules/shell/module";
import { ShellWorkspaces } from "@/modules/shell/workspaceIds";
import { renderWithPlatform } from "@/test/renderWithPlatform";
import { workspaceStore } from "@/stores/workspaceStore";
import { extensionsStore } from "@/stores/extensionsStore";
import { WorkspaceSwitcher } from "./WorkspaceSwitcher";

function bootShell(): Platform {
  const platform = createPlatform();
  registerShellModule(platform);
  platform.initializeSync();
  return platform;
}

describe("WorkspaceSwitcher", () => {
  beforeEach(() => {
    workspaceStore.setState({
      activeId: ShellWorkspaces.Design,
      menuOpen: false,
      customizeOpen: false,
      hiddenPanels: {},
      hiddenToolGroups: {},
    });
    extensionsStore.setState({ managerOpen: false });
  });

  it("shows the active workspace's title", () => {
    renderWithPlatform(<WorkspaceSwitcher />, { platform: bootShell() });
    expect(screen.getByTestId("workspace-switcher")).toHaveTextContent("Design");
  });

  it("lists every registered workspace and switches to the picked one", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<WorkspaceSwitcher />, { platform: bootShell() });

    await user.click(screen.getByTestId("workspace-switcher"));
    for (const id of Object.values(ShellWorkspaces)) {
      expect(screen.getByTestId(`workspace-option-${id}`)).toBeInTheDocument();
    }

    await user.click(screen.getByTestId(`workspace-option-${ShellWorkspaces.Drawing}`));
    expect(workspaceStore.getState().activeId).toBe(ShellWorkspaces.Drawing);
    // Picking closes the menu — a switcher that stayed open would invite a
    // second, accidental switch.
    expect(workspaceStore.getState().menuOpen).toBe(false);
  });

  it("puts an ADD-ON's workspace in its own group, not among the built-ins", async () => {
    const platform = bootShell();
    const owner = addonId("com.example.robot");
    platform.createScope(owner).registerWorkspace({
      id: contributionId<WorkspaceId>(owner, "com.example.robot.workspace.design"),
      title: "Robot Design",
      panels: [{ panelId: ShellWorkspaces.Design as never, slot: Slots.ShellTop }],
    });

    const user = userEvent.setup();
    renderWithPlatform(<WorkspaceSwitcher />, { platform });
    await user.click(screen.getByTestId("workspace-switcher"));

    expect(screen.getByText("Add-ons")).toBeInTheDocument();
    expect(screen.getByText("Robot Design")).toBeInTheDocument();
  });

  it("has no ADD-ONS group when no add-on registered one", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<WorkspaceSwitcher />, { platform: bootShell() });
    await user.click(screen.getByTestId("workspace-switcher"));
    // An empty group would advertise a catalog that does not exist.
    expect(screen.queryByText("Add-ons")).not.toBeInTheDocument();
  });

  it("routes 'Manage extensions…' to the manager", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<WorkspaceSwitcher />, { platform: bootShell() });
    await user.click(screen.getByTestId("workspace-switcher"));
    await user.click(screen.getByTestId("workspace-manage-extensions"));
    expect(extensionsStore.getState().managerOpen).toBe(true);
  });
});
