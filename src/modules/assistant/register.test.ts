/*
 * The module's registration surface: it activates, its panel lands where the
 * shell renders it, its ids sit under its own namespace, and a remount leaves
 * nothing behind (React 19 StrictMode double-invokes mount effects, so the
 * editor's `useLayoutEffect` really does register twice on the same platform).
 */
import { describe, it, expect } from "vitest";
import { createPlatform, Slots, type Platform } from "@/platform";
import { ASSISTANT_MODULE_ID, ASSISTANT_SCHEMA_VERSION } from "./manifest";
import { AssistantPanels } from "./panelIds";
import { registerAssistantModule } from "./register";
import { contributeAssistantUi } from "./ui";

function bootPlatform(): Platform {
  const platform = createPlatform();
  registerAssistantModule(platform);
  platform.initializeSync();
  return platform;
}

describe("assistant module registration", () => {
  it("reaches ready with no bootstrap-time contributions", () => {
    const platform = bootPlatform();
    expect(platform.moduleState(ASSISTANT_MODULE_ID)).toBe("ready");
    expect(platform.moduleIds()).toContain(ASSISTANT_MODULE_ID);
    expect(platform.panels.size).toBe(0);
    expect(platform.commands.size).toBe(0);
    expect(platform.tools.size).toBe(0);
    expect(ASSISTANT_SCHEMA_VERSION).toBe(1);
  });

  it("puts exactly one panel in ShellLeft, after Variables' 110", () => {
    const platform = bootPlatform();
    contributeAssistantUi(platform.createScope(ASSISTANT_MODULE_ID));

    const panels = platform.panels.entries();
    expect(panels).toHaveLength(1);
    expect(panels[0]!.id).toBe(AssistantPanels.Assistant);
    expect(panels[0]!.slot).toBe(Slots.ShellLeft);
    expect(panels[0]!.priority).toBe(120);
    // A `title` is what makes it offerable in the customize sheet.
    expect(panels[0]!.title).toBe("Assistant");
  });

  it("namespaces every id under its owner", () => {
    for (const id of Object.values(AssistantPanels)) {
      expect(id.startsWith(`${ASSISTANT_MODULE_ID}.`)).toBe(true);
    }
    expect(ASSISTANT_MODULE_ID).toBe("onecad.assistant");
  });

  it("survives a remount without a duplicate-id collision", () => {
    const platform = bootPlatform();
    const first = platform.createScope(ASSISTANT_MODULE_ID);
    contributeAssistantUi(first);
    first.dispose();
    expect(platform.panels.size).toBe(0);

    const second = platform.createScope(ASSISTANT_MODULE_ID);
    expect(() => contributeAssistantUi(second)).not.toThrow();
    expect(platform.panels.size).toBe(1);
  });

  it("bootstrap registers the module", async () => {
    const { bootstrapOneCAD } = await import("@/app/bootstrap");
    const platform = bootstrapOneCAD();
    expect(platform.moduleState(ASSISTANT_MODULE_ID)).toBe("ready");
  });
});
