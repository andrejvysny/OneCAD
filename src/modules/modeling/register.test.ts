/*
 * W2 proof: the shipped toolbar and keyboard arrangement survive being expressed
 * as platform contributions.
 *
 * The toolbar assertion goes through `toolbarFromRegistry`, i.e. it rebuilds the
 * arrangement from the platform registry rather than from the descriptor table
 * the registry was built from — otherwise it would only prove the table equals
 * itself.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { createPlatform, type Platform } from "@/platform";
import { MODEL_TOOLS_CONTRACT, SKETCH_TOOLS_CONTRACT } from "@/test/contracts/toolbarContract";
import { MODEL_KEYS_CONTRACT, SKETCH_KEYS_CONTRACT } from "@/test/contracts/keymapContract";
import { registerModelingModule } from "./register";
import {
  MODELING_MODULE_ID,
  ModelingServices,
  type CommandApiService,
  type GeometryQueryService,
} from "./manifest";
import { toolbarFromRegistry, registeredTools, toolFromId } from "./registryToolbar";
import { ModelingCommands, ModelingModelTools, ModelingSketchTools } from "./ids";
import { MODELING_TOOL_DESCRIPTORS } from "./tools";
import { documentStore } from "@/stores/documentStore";
import { buildPaletteItems, filterPaletteItems } from "@/features/palette/paletteItems";

vi.mock("@/tools/activateTool", () => ({ activateTool: vi.fn(async () => {}) }));
vi.mock("@/shortcuts/useShortcuts", () => ({ runAction: vi.fn() }));
// Undo/Redo must DELEGATE to the one router every surface shares; spying it is
// how a second, palette-only undo implementation would be caught.
vi.mock("@/features/shell/undoActions", () => ({
  runUndo: vi.fn(async () => {}),
  runRedo: vi.fn(async () => {}),
}));

const EMPTY_CTX = { selection: [], scopes: [] } as const;

async function bootPlatform(): Promise<Platform> {
  const platform = createPlatform();
  registerModelingModule(platform);
  await platform.initialize();
  return platform;
}

describe("modeling module registration", () => {
  let platform: Platform;

  beforeEach(async () => {
    platform = await bootPlatform();
  });

  it("reproduces the frozen model toolbar from the registry", () => {
    expect(toolbarFromRegistry(platform, "model")).toEqual([...MODEL_TOOLS_CONTRACT]);
  });

  it("reproduces the frozen sketch toolbar from the registry", () => {
    expect(toolbarFromRegistry(platform, "sketch")).toEqual([...SKETCH_TOOLS_CONTRACT]);
  });

  it("registers every descriptor exactly once, under a unique id", () => {
    expect(platform.tools.size).toBe(MODELING_TOOL_DESCRIPTORS.length);
    const ids = platform.tools.entries().map((t) => t.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("scope-qualifies the ids that exist in both modes", () => {
    // `select` and `mirror` mean different things per mode; collapsing them would
    // have let one shadow the other in the registry.
    expect(ModelingModelTools.mirror).not.toBe(ModelingSketchTools.mirror);
    expect(toolFromId(ModelingModelTools.mirror)).toEqual({ scope: "model", tool: "mirror" });
    expect(toolFromId(ModelingSketchTools.mirror)).toEqual({ scope: "sketch", tool: "mirror" });
  });

  it("carries each tool's key binding as its default shortcut", () => {
    const chordOf = (id: string) => {
      const def = platform.tools.get(id);
      return def?.defaultShortcut;
    };
    expect(chordOf(ModelingModelTools.extrude)).toEqual({ key: "e" });
    expect(chordOf(ModelingModelTools.offsetFace)).toEqual({ key: "o", shift: true });
    // Bound as ⇧/ but written "?" — the two are carried separately on purpose.
    expect(chordOf(ModelingModelTools.measure)).toEqual({ key: "?", shift: true });
    expect(platform.tools.get(ModelingModelTools.measure)?.shortcutLabel).toBe("?");
  });

  it("every tool binding in the frozen keymap reaches its registered tool", () => {
    for (const [contract, ids] of [
      [MODEL_KEYS_CONTRACT, ModelingModelTools],
      [SKETCH_KEYS_CONTRACT, ModelingSketchTools],
    ] as const) {
      for (const binding of contract) {
        if (binding.action.type !== "tool") continue;
        const id = (ids as Record<string, string>)[binding.action.tool];
        expect(id, `no id for ${binding.action.tool}`).toBeDefined();
        expect(platform.tools.get(id)?.defaultShortcut).toEqual(
          binding.shift === undefined
            ? { key: binding.key }
            : { key: binding.key, shift: binding.shift },
        );
      }
    }
  });

  it("registers a real GeometryQuery service (Component Library WP-0.1)", () => {
    // Proves the service is actually REGISTERED, not just declared in
    // manifest.ts — `platform.services.require` throws on a merely-declared,
    // never-registered id, which is exactly the gap this WP closes.
    const svc = platform.services.require<GeometryQueryService>(ModelingServices.GeometryQuery);
    expect(typeof svc.classifyElement).toBe("function");
  });

  it("registers a real CommandApi service (Component Library WP-1.3)", () => {
    const svc = platform.services.require<CommandApiService>(ModelingServices.CommandApi);
    expect(typeof svc.placeComponent).toBe("function");
    expect(typeof svc.detachComponent).toBe("function");
  });

  it("registers the non-tool actions as commands", () => {
    for (const id of Object.values(ModelingCommands)) {
      expect(platform.commands.get(id), id).toBeDefined();
    }
    expect(platform.commands.size).toBe(Object.keys(ModelingCommands).length);
  });

  it("a tool activation routes through the shared dispatcher", async () => {
    const { activateTool } = await import("@/tools/activateTool");
    await platform.tools
      .get(ModelingModelTools.extrude)
      ?.activate({ selection: [], scopes: [] });
    expect(activateTool).toHaveBeenCalledWith("extrude");
  });

  it("offers Undo/Redo as commands, enabled by history DEPTH and named by label", () => {
    const undo = platform.commands.get(ModelingCommands.undo);
    const redo = platform.commands.get(ModelingCommands.redo);
    expect(undo, "undo must be registered — the palette projects off this registry").toBeDefined();
    expect(redo).toBeDefined();

    documentStore.getState().applyChange({
      undoDepth: 0,
      redoDepth: 0,
      undoLabel: null,
      redoLabel: null,
    });
    expect(undo?.title).toBe("Undo");
    expect(undo?.canExecute?.(EMPTY_CTX)).toEqual({
      enabled: false,
      reason: "Nothing to undo",
    });
    expect(redo?.canExecute?.(EMPTY_CTX)).toEqual({
      enabled: false,
      reason: "Nothing to redo",
    });

    documentStore.getState().applyChange({
      undoDepth: 2,
      redoDepth: 1,
      undoLabel: "Extrude",
      redoLabel: "Fillet",
    });
    // The title is read LIVE: the row named "Undo" a moment ago must now name
    // the step actually on top of the stack.
    expect(undo?.title).toBe("Undo Extrude");
    expect(redo?.title).toBe("Redo Fillet");
    expect(undo?.canExecute?.(EMPTY_CTX)).toEqual({ enabled: true });
    expect(redo?.canExecute?.(EMPTY_CTX)).toEqual({ enabled: true });
  });

  it("...and those commands reach the palette, searchable by 'undo'", () => {
    documentStore.getState().applyChange({
      undoDepth: 1,
      redoDepth: 0,
      undoLabel: "Extrude",
      redoLabel: null,
    });
    const items = buildPaletteItems({
      platform,
      commandContext: EMPTY_CTX,
      toolContext: EMPTY_CTX,
      activateWorkspace: () => {},
    });
    const hits = filterPaletteItems(items, "undo");
    expect(hits.map((i) => i.title)).toContain("Undo Extrude");
    // "revert"/"history" are keywords on both rows, so the disabled Redo is
    // findable by them too rather than silently vanishing.
    expect(filterPaletteItems(items, "revert").map((i) => i.id)).toEqual(
      expect.arrayContaining([ModelingCommands.undo, ModelingCommands.redo]),
    );
    expect(hits.find((i) => i.id === ModelingCommands.undo)?.enabled).toBe(true);
  });

  it("Undo executes through the shared router (not a second undo implementation)", async () => {
    const { runUndo } = await import("@/features/shell/undoActions");
    await platform.commands.get(ModelingCommands.undo)?.execute(EMPTY_CTX);
    expect(runUndo).toHaveBeenCalledTimes(1);
  });

  it("a command execution routes through the shared action runner", async () => {
    const { runAction } = await import("@/shortcuts/useShortcuts");
    await platform.commands.get(ModelingCommands.finishSketch)?.execute({
      selection: [],
      scopes: [],
    });
    expect(runAction).toHaveBeenCalledWith({ type: "finishSketch" });
  });

  it("disposing the module removes every modeling contribution", () => {
    platform.scopeFor(MODELING_MODULE_ID).dispose();
    expect(platform.tools.size).toBe(0);
    expect(platform.commands.size).toBe(0);
    expect(registeredTools(platform, "model")).toEqual([]);
  });
});
