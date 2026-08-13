/*
 * Proof that the stub reaches "ready" with zero contributions, and that
 * `bootstrapOneCAD()` still initializes cleanly with it present. Nothing else to
 * assert until the module registers something real.
 */
import { describe, it, expect } from "vitest";
import { createPlatform, type Platform } from "@/platform";
import { registerRenderModule } from "./module";
import { RENDER_MODULE_ID } from "./manifest";

function bootPlatform(): Platform {
  const platform = createPlatform();
  registerRenderModule(platform);
  platform.initializeSync();
  return platform;
}

describe("render module registration", () => {
  it("reaches ready with no contributions registered", () => {
    const platform = bootPlatform();
    expect(platform.moduleState(RENDER_MODULE_ID)).toBe("ready");
    expect(platform.commands.size).toBe(0);
    expect(platform.tools.size).toBe(0);
    expect(platform.panels.size).toBe(0);
    expect(platform.workspaces.size).toBe(0);
  });

  it("disposing the module leaves nothing behind", () => {
    const platform = bootPlatform();
    platform.scopeFor(RENDER_MODULE_ID).dispose();
    expect(platform.commands.size).toBe(0);
  });
});

describe("bootstrap with the render stub present", () => {
  it("initializes shell, modeling and render together", async () => {
    const { bootstrapOneCAD } = await import("@/app/bootstrap");
    const platform = bootstrapOneCAD();
    expect(platform.moduleState(RENDER_MODULE_ID)).toBe("ready");
    expect(platform.moduleIds()).toContain(RENDER_MODULE_ID);
  });
});
