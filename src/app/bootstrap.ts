/*
 * The composition root (spec §105).
 *
 * One place creates the Platform and registers the built-in modules. It lives in
 * `app/` and not in `platform/` on purpose: this is the only file allowed to know
 * both sides, and the Platform must never import a module
 * (docs/ARCHITECTURE.md §3).
 *
 * Only cheap, global registration happens here. The editor's UI contributions
 * register when the editor screen mounts, because that tree is a deliberate
 * code-split chunk (`App.tsx`) and the start screen must not pay for it.
 */
import { createPlatform, type Platform } from "@/platform";
import { registerShellModule } from "@/modules/shell/module";
import { registerModelingModule } from "@/modules/modeling/register";
import { registerRenderModule } from "@/modules/render/module";

export function bootstrapOneCAD(): Platform {
  const platform = createPlatform();

  // Order here decides nothing: activation order comes from declared
  // dependencies, and placement comes from each contribution's own priority.
  registerShellModule(platform);
  registerModelingModule(platform);
  // Stub — registers with zero contributions. See src/modules/render/module.ts.
  registerRenderModule(platform);

  platform.initializeSync();
  return platform;
}
