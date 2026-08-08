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
import { ShellCommands, ShellServices } from "./ids";
import { DESIGN_WORKSPACE } from "./workspaces";
import { viewportNavigation } from "./viewportNavigation";
import { SHELL_GLOBAL_BINDINGS } from "./bindings";

/**
 * View navigation registers HERE (bootstrap) rather than in `register.ts`: that
 * file imports the chrome components, and pulling them into the startup bundle
 * would undo the code split this module's own comment describes. The navigation
 * implementation imports `viewportStore` and nothing else, which bootstrap
 * already pays for.
 */
export function registerShellModule(platform: Platform): void {
  platform.registerModule({
    id: SHELL_MODULE_ID,
    version: "1.0.0",
    provides: [ShellServices.ViewportNavigation],
    activate: (scope) => {
      scope.registerWorkspace(DESIGN_WORKSPACE);
      scope.registerService(ShellServices.ViewportNavigation, viewportNavigation);

      const shortcutFor = (type: "zoomFit" | "home") => {
        const b = SHELL_GLOBAL_BINDINGS.find((x) => x.action.type === type);
        if (!b) return undefined;
        return b.shift === undefined ? { key: b.key } : { key: b.key, shift: b.shift };
      };

      scope.registerCommand({
        id: ShellCommands.zoomFit,
        title: "Zoom to fit",
        group: "shell.view",
        priority: 100,
        defaultShortcut: shortcutFor("zoomFit"),
        execute: () => {
          viewportNavigation.zoomFit();
          return { status: "done" as const };
        },
      });
      scope.registerCommand({
        id: ShellCommands.home,
        title: "Home view",
        group: "shell.view",
        priority: 110,
        defaultShortcut: shortcutFor("home"),
        execute: () => {
          viewportNavigation.home();
          return { status: "done" as const };
        },
      });
    },
  });
}
