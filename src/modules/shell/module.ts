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
import { SHELL_WORKSPACES } from "./workspaces";
import { viewportNavigation } from "./viewportNavigation";
import { SHELL_GLOBAL_BINDINGS } from "./bindings";
import { paletteStore } from "@/stores/paletteStore";
import { extensionsStore } from "@/stores/extensionsStore";
import { workspaceStore } from "@/stores/workspaceStore";
import { layersStore } from "@/stores/layersStore";

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
      for (const workspace of SHELL_WORKSPACES) scope.registerWorkspace(workspace);
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

      /*
       * The modular-platform surfaces, as commands.
       *
       * Registered at BOOTSTRAP with the rest of this module rather than with
       * the editor chrome, so they are one currency everywhere: the palette
       * finds them, a keystroke can reach them and an addon could invoke them,
       * instead of each living only inside the one popover that opens it. The
       * stores they touch are session state and cost nothing at startup.
       */
      scope.registerCommand({
        id: ShellCommands.commandPalette,
        title: "Show all commands",
        group: "shell.app",
        priority: 100,
        keywords: ["palette", "search", "run"],
        // DECLARED here, DISPATCHED in `useShortcuts`. The registry lane runs
        // only for chords the built-in tables did not claim and deliberately
        // ignores modified keys, so ⌘K is handled with ⌘S/⌘O/⌘W alongside the
        // other app chords; this field is what makes the palette print "⌘K"
        // next to its own entry instead of nothing.
        defaultShortcut: { key: "k", meta: true },
        execute: () => {
          paletteStore.getState().show();
          return { status: "done" as const };
        },
      });
      scope.registerCommand({
        id: ShellCommands.openExtensions,
        title: "Manage extensions…",
        group: "shell.app",
        priority: 110,
        keywords: ["addon", "add-on", "plugin", "extension"],
        execute: () => {
          extensionsStore.getState().openManager();
          return { status: "done" as const };
        },
      });
      scope.registerCommand({
        id: ShellCommands.customizeWorkspace,
        title: "Customize workspace…",
        group: "shell.app",
        priority: 120,
        execute: () => {
          workspaceStore.getState().setCustomizeOpen(true);
          return { status: "done" as const };
        },
      });
      scope.registerCommand({
        id: ShellCommands.toggleLayers,
        title: "Viewport layers",
        group: "shell.view",
        priority: 120,
        keywords: ["layers", "show", "hide"],
        execute: () => {
          layersStore.getState().toggleOpen();
          return { status: "done" as const };
        },
      });
    },
  });
}
