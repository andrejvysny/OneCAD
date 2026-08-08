/*
 * `onecad.shell` module registration.
 *
 * At bootstrap the shell module contributes only what is cheap and global: the
 * default workspace. Its CHROME (title bar, status bar, nav pill, corner
 * cluster) registers when the editor screen mounts, from `register.ts`, so the
 * start screen does not pull the editor chunk in.
 */
import type { Platform } from "@/platform";
import { SHELL_MODULE_ID } from "./panelIds";
import { DESIGN_WORKSPACE } from "./workspaces";

export function registerShellModule(platform: Platform): void {
  platform.registerModule({
    id: SHELL_MODULE_ID,
    version: "1.0.0",
    activate: (scope) => {
      scope.registerWorkspace(DESIGN_WORKSPACE);
    },
  });
}
