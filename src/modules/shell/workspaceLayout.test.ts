/*
 * The conservative visibility rule, pinned.
 *
 * "Unlisted ⇒ hidden" is the reading that looks tidier and quietly swallows
 * every overlay a new tool grows. This file exists so that reading cannot come
 * back by accident.
 */
import { describe, it, expect } from "vitest";
import {
  contributionId,
  moduleId,
  Slots,
  type PanelContribution,
  type PanelId,
  type WorkspaceDefinition,
  type WorkspaceId,
} from "@/platform";
import { activeWorkspace, customizablePanels, resolvePanelVisibility } from "./workspaceLayout";

const OWNER = moduleId("onecad.demo");
const pid = (name: string) => contributionId<PanelId>(OWNER, `onecad.demo.panel.${name}`);
const wid = (name: string) => contributionId<WorkspaceId>(OWNER, `onecad.demo.workspace.${name}`);

const panel = (id: PanelId, title?: string): PanelContribution => ({
  id,
  slot: Slots.ShellLeft,
  ...(title === undefined ? {} : { title }),
  component: () => null,
});

const TREE = pid("tree");
const CHIPS = pid("chips");
const TOOLBAR = pid("toolbar");

const WORKSPACE: WorkspaceDefinition = {
  id: wid("main"),
  title: "Main",
  panels: [
    { panelId: TREE, slot: Slots.ShellLeft },
    { panelId: TOOLBAR, slot: Slots.ToolbarPrimary, visible: false },
  ],
};

describe("panel visibility", () => {
  it("shows a panel the workspace never mentions", () => {
    // The overlay case: chips belong to whichever tool is running, and no
    // workspace should have to enumerate them to keep them on screen.
    const { isVisible } = resolvePanelVisibility(WORKSPACE, []);
    expect(isVisible(panel(CHIPS))).toBe(true);
  });

  it("hides a panel the workspace explicitly placed as invisible", () => {
    const { isVisible } = resolvePanelVisibility(WORKSPACE, []);
    expect(isVisible(panel(TOOLBAR))).toBe(false);
  });

  it("hides a panel the user switched off, even when the workspace shows it", () => {
    const { isVisible } = resolvePanelVisibility(WORKSPACE, [TREE]);
    expect(isVisible(panel(TREE))).toBe(false);
  });

  it("shows everything when there is no workspace at all", () => {
    const { isVisible } = resolvePanelVisibility(undefined, []);
    expect(isVisible(panel(TOOLBAR))).toBe(true);
  });
});

describe("customizable panels", () => {
  it("offers only panels the workspace placed AND that have a name", () => {
    const registered = [panel(TREE, "Model"), panel(TOOLBAR), panel(CHIPS, "Chips")];
    const offered = customizablePanels(WORKSPACE, registered);
    // TOOLBAR is placed but unnamed; CHIPS is named but not placed.
    expect(offered.map((p) => p.id)).toEqual([TREE]);
  });

  it("does not offer the same panel twice when a workspace places it twice", () => {
    const twice: WorkspaceDefinition = {
      ...WORKSPACE,
      panels: [
        { panelId: TREE, slot: Slots.ShellLeft },
        { panelId: TREE, slot: Slots.ShellRight },
      ],
    };
    expect(customizablePanels(twice, [panel(TREE, "Model")])).toHaveLength(1);
  });
});

describe("active workspace resolution", () => {
  const other: WorkspaceDefinition = { id: wid("other"), title: "Other", panels: [] };

  it("resolves the active id", () => {
    expect(activeWorkspace([WORKSPACE, other], other.id, WORKSPACE.id)).toBe(other);
  });

  it("falls back when the active id resolves to nothing", () => {
    // An add-on's workspace can be unloaded while it is active; blanking the
    // shell is not an acceptable answer to that.
    expect(activeWorkspace([WORKSPACE], wid("gone"), WORKSPACE.id)).toBe(WORKSPACE);
  });

  it("returns undefined when even the fallback is gone", () => {
    expect(activeWorkspace([], wid("gone"), WORKSPACE.id)).toBeUndefined();
  });
});
