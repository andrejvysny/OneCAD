/*
 * `onecad.render` module registration — INERT STUB.
 *
 * `activate` registers nothing. This module exists only so `onecad.render` is a
 * real, reserved module id that reaches `"ready"` at boot, ahead of any actual
 * capability being built behind it.
 *
 * Its future UI home is the existing `onecad.shell` `Visualization` workspace
 * placeholder (`src/modules/shell/workspaces.ts`, `ShellWorkspaces.Visualization`)
 * — no new workspace is registered here. When this module gains real capability,
 * a follow-up ADR decides whether it registers its own `onecad.render.workspace.*`
 * and the shell placeholder retires (the transition `workspaceIds.ts` already
 * describes for Simulation/Drawing), or whether the shell hands that workspace
 * off some other way. Not decided yet — see `docs/RENDER_MODULE.md`.
 */
import type { Platform } from "@/platform";
import { RENDER_MODULE_ID } from "./manifest";

export function registerRenderModule(platform: Platform): void {
  platform.registerModule({
    id: RENDER_MODULE_ID,
    version: "0.1.0",
    activate: () => {
      // Intentionally empty. See file header.
    },
  });
}
