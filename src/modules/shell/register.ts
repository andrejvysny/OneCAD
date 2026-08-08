/*
 * `onecad.shell` — the application's own chrome, contributed the same way a
 * module contributes its UI.
 *
 * Title bar, status bar, view-orientation pill and corner cluster are not
 * modeling: they belong to the application regardless of which domain is in
 * front. Registering them rather than hard-coding them keeps ONE composition
 * mechanism (spec §143) — the shell renders slots, and everything in a slot got
 * there the same way.
 */
import { Slots, type ModuleScope } from "@/platform";
import { SHELL_MODULE_ID, ShellPanels } from "./panelIds";
import { TitleBar } from "@/features/shell/TitleBar";
import { StatusBar } from "@/features/shell/StatusBar";
import { NavPill } from "@/features/shell/NavPill";
import { CornerCluster } from "@/features/shell/CornerCluster";

export { SHELL_MODULE_ID, ShellPanels };

/** Registers the application chrome into `scope`. */
export function contributeShellChrome(scope: ModuleScope): void {
  scope.registerPanel({
    id: ShellPanels.TitleBar,
    slot: Slots.ShellTop,
    priority: 100,
    component: TitleBar,
  });

  // Viewport-frame chrome, in the shipped order: the corner cluster is rendered
  // before the nav pill.
  scope.registerPanel({
    id: ShellPanels.CornerCluster,
    slot: Slots.ViewportChrome,
    priority: 100,
    component: CornerCluster,
  });
  scope.registerPanel({
    id: ShellPanels.NavPill,
    slot: Slots.ViewportChrome,
    priority: 110,
    component: NavPill,
  });

  scope.registerPanel({
    id: ShellPanels.StatusBar,
    slot: Slots.ShellBottom,
    priority: 100,
    component: StatusBar,
  });
}
