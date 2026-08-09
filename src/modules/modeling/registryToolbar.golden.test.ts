/*
 * Toolbar GOLDEN probe.
 *
 * Was `features/toolbar/toolbarConfig.golden.test.ts`, which read the two
 * module-load-time arrays. The mechanism changed — the toolbar is projected from
 * the platform tool registry now — so the PROBE follows it while the contract it
 * asserts against does not move (src/test/contracts/README.md).
 *
 * The projection is rebuilt from the registry, never from the descriptor table
 * the registry was populated from, or the assertion would only prove the table
 * equals itself.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { addonId, contributionId, createPlatform, type Platform, type ToolId } from "@/platform";
import { MODEL_TOOLS_CONTRACT, SKETCH_TOOLS_CONTRACT } from "@/test/contracts/toolbarContract";
import { isSeparator } from "@/features/toolbar/toolbarConfig";
import { registerModelingModule } from "./register";
import { MODELING_MODULE_ID, ModelingScopes } from "./manifest";
import { toolbarFromRegistry } from "./registryToolbar";

vi.mock("@/tools/activateTool", () => ({ activateTool: vi.fn(async () => {}) }));
vi.mock("@/shortcuts/useShortcuts", () => ({ runAction: vi.fn() }));

describe("toolbar contract", () => {
  let platform: Platform;

  beforeEach(() => {
    platform = createPlatform();
    registerModelingModule(platform);
    platform.initializeSync();
  });

  it("the model toolbar matches the frozen arrangement", () => {
    expect(toolbarFromRegistry(platform, "model")).toEqual([...MODEL_TOOLS_CONTRACT]);
  });

  it("the sketch toolbar matches the frozen arrangement", () => {
    expect(toolbarFromRegistry(platform, "sketch")).toEqual([...SKETCH_TOOLS_CONTRACT]);
  });

  it("every non-separator entry carries a complete presentation", () => {
    const entries = [
      ...toolbarFromRegistry(platform, "model"),
      ...toolbarFromRegistry(platform, "sketch"),
    ];
    for (const entry of entries) {
      if (isSeparator(entry)) continue;
      expect(entry.id, "tool id").toBeTruthy();
      expect(entry.icon, `icon for ${entry.id}`).toBeTruthy();
      expect(entry.label, `label for ${entry.id}`).toBeTruthy();
      expect(entry.shortcut, `shortcut for ${entry.id}`).toBeTruthy();
    }
  });

  it("no tool id repeats inside a scope's set", () => {
    for (const scope of ["model", "sketch"] as const) {
      const ids = toolbarFromRegistry(platform, scope)
        .filter((e) => !isSeparator(e))
        .map((e) => (isSeparator(e) ? "" : e.id));
      expect(new Set(ids).size).toBe(ids.length);
    }
  });

  it("a tool disposed after boot leaves the projection", () => {
    // The point of reading the registry live: the arrangement is not frozen at
    // module load.
    expect(toolbarFromRegistry(platform, "model").length).toBeGreaterThan(0);
    platform.scopeFor(MODELING_MODULE_ID).dispose();
    expect(toolbarFromRegistry(platform, "model")).toEqual([]);
  });

  it("a FOREIGN tool does not disturb the frozen arrangement", () => {
    // The toolbar itself now renders every registration, foreign ones included
    // (`FloatingToolbar.test.tsx`). This projection is the CONTRACT's shape, and
    // the contract is modeling's arrangement — so a third-party registration must
    // leave it byte-identical rather than appearing in the middle of it.
    const foreign = addonId("com.example.foo");
    platform.createScope(foreign).registerTool({
      id: contributionId<ToolId>(foreign, "com.example.foo.tool.inspect"),
      title: "Inspect",
      icon: "select",
      scopes: [ModelingScopes.Model],
      priority: 105, // deliberately between two modeling tools
      activate: () => {},
      deactivate: () => {},
    });

    expect(toolbarFromRegistry(platform, "model")).toEqual([...MODEL_TOOLS_CONTRACT]);
  });
});
