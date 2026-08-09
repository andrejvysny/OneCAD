/*
 * The palette's contract: everything registered is findable, a disabled entry
 * still shows and says WHY, and nothing has to opt in.
 *
 * These drive a real platform rather than a fixture list — a projection test
 * against hand-built objects would pass even if `buildPaletteItems` stopped
 * reading a registry.
 */
import { describe, it, expect } from "vitest";
import {
  addonId,
  contributionId,
  createPlatform,
  moduleId,
  type CommandId,
  type Platform,
  type ToolId,
  type WorkspaceId,
} from "@/platform";
import { buildPaletteItems, filterPaletteItems, formatShortcut } from "./paletteItems";

const OWNER = moduleId("onecad.demo");
const EMPTY = { selection: [], scopes: [] };

function boot(): Platform {
  const platform = createPlatform();
  const scope = platform.createScope(OWNER);

  scope.registerCommand({
    id: contributionId<CommandId>(OWNER, "onecad.demo.command.export"),
    title: "Export STEP…",
    group: "file",
    defaultShortcut: { key: "e", meta: true },
    execute: () => ({ status: "done" as const }),
  });
  scope.registerCommand({
    id: contributionId<CommandId>(OWNER, "onecad.demo.command.fillet"),
    title: "Fillet",
    canExecute: () => ({ enabled: false, reason: "Requires one or more edges" }),
    execute: () => ({ status: "done" as const }),
  });
  scope.registerTool({
    id: contributionId<ToolId>(OWNER, "onecad.demo.tool.line"),
    title: "Line",
    shortcutLabel: "L",
    activate: () => {},
    deactivate: () => {},
  });
  scope.registerWorkspace({
    id: contributionId<WorkspaceId>(OWNER, "onecad.demo.workspace.main"),
    title: "Main",
    panels: [],
  });
  return platform;
}

const build = (platform: Platform) =>
  buildPaletteItems({
    platform,
    commandContext: EMPTY,
    toolContext: EMPTY,
    activateWorkspace: () => {},
  });

describe("palette items", () => {
  it("offers commands, tools and workspaces with no opt-in", () => {
    const items = build(boot());
    expect(items.map((i) => i.title)).toEqual([
      "Export STEP…",
      "Fillet",
      "Line",
      "Switch to Main",
    ]);
    expect(items.map((i) => i.kind)).toEqual(["command", "command", "tool", "workspace"]);
  });

  it("keeps a disabled item visible and carries its owner's reason", () => {
    const fillet = build(boot()).find((i) => i.title === "Fillet");
    expect(fillet?.enabled).toBe(false);
    expect(fillet?.reason).toBe("Requires one or more edges");
  });

  it("prefers the tool's written shortcut label over the derived chord", () => {
    const line = build(boot()).find((i) => i.title === "Line");
    expect(line?.shortcut).toBe("L");
  });

  it("derives a chord when the definition supplied no label", () => {
    const exp = build(boot()).find((i) => i.title === "Export STEP…");
    expect(exp?.shortcut).toBe("⌘E");
  });

  it("labels an add-on's contribution as an add-on", () => {
    const platform = boot();
    const owner = addonId("com.example.robot");
    platform.createScope(owner).registerCommand({
      id: contributionId<CommandId>(owner, "com.example.robot.command.reach"),
      title: "Robot reach check",
      execute: () => ({ status: "done" as const }),
    });
    const item = build(platform).find((i) => i.title === "Robot reach check");
    expect(item?.source).toBe("Add-on · com.example.robot");
  });

  it("filters on title, source and keywords", () => {
    const items = build(boot());
    expect(filterPaletteItems(items, "fill").map((i) => i.title)).toEqual(["Fillet"]);
    expect(filterPaletteItems(items, "file").map((i) => i.title)).toEqual(["Export STEP…"]);
    expect(filterPaletteItems(items, "zzz")).toEqual([]);
  });

  it("sorts usable answers above greyed-out ones", () => {
    const items = build(boot());
    const ranked = filterPaletteItems(items, "");
    expect(ranked[ranked.length - 1]?.title).toBe("Fillet");
  });

  it("formats modifiers in the conventional order", () => {
    expect(formatShortcut({ key: "f", shift: true })).toBe("⇧F");
    expect(formatShortcut({ key: "s", meta: true, shift: true })).toBe("⇧⌘S");
    expect(formatShortcut(undefined)).toBe("");
  });

  it("running a workspace item hands back the workspace, not its id", () => {
    let picked: string | null = null;
    const platform = boot();
    const items = buildPaletteItems({
      platform,
      commandContext: EMPTY,
      toolContext: EMPTY,
      activateWorkspace: (ws) => {
        picked = ws.title;
      },
    });
    items.find((i) => i.kind === "workspace")?.run();
    expect(picked).toBe("Main");
  });
});
