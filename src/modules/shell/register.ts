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
import { MissingExtensionBanner } from "@/features/extensions/MissingExtensionBanner";
import { MissingExtensionDialog } from "@/features/extensions/MissingExtensionDialog";
import { ExtensionsManager } from "@/features/extensions/ExtensionsManager";
import { CommandPalette } from "@/features/palette/CommandPalette";
import { CustomizeWorkspaceSheet } from "@/features/workspace/CustomizeWorkspaceSheet";
import { WorkspacePlaceholder } from "@/features/workspace/WorkspacePlaceholder";
import { missingModulesTreeProvider } from "./missingModulesTree";

export { SHELL_MODULE_ID, ShellPanels };

/** Registers the application chrome into `scope`. */
export function contributeShellChrome(scope: ModuleScope): void {
  scope.registerPanel({
    id: ShellPanels.TitleBar,
    slot: Slots.ShellTop,
    priority: 100,
    component: TitleBar,
  });

  // Flow strips UNDER the title bar. `shell.top` rather than
  // `shell.notification` because they must take layout space: the
  // notification slot's occupants float over the viewport, and a full-width
  // strip there would cover the toolbar it is supposed to sit above.
  scope.registerPanel({
    id: ShellPanels.MissingExtensionBanner,
    slot: Slots.ShellTop,
    priority: 200,
    component: MissingExtensionBanner,
  });
  scope.registerPanel({
    id: ShellPanels.WorkspacePlaceholder,
    slot: Slots.ShellTop,
    priority: 210,
    component: WorkspacePlaceholder,
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

  // Modal surfaces. Each renders null until its own open-state says otherwise,
  // so mounting them all costs one null render apiece and keeps "who may open a
  // modal" out of the shell's layout code.
  scope.registerPanel({
    id: ShellPanels.CommandPalette,
    slot: Slots.ShellOverlay,
    priority: 100,
    component: CommandPalette,
  });
  scope.registerPanel({
    id: ShellPanels.ExtensionsManager,
    slot: Slots.ShellOverlay,
    priority: 110,
    component: ExtensionsManager,
  });
  scope.registerPanel({
    id: ShellPanels.CustomizeWorkspaceSheet,
    slot: Slots.ShellOverlay,
    priority: 120,
    component: CustomizeWorkspaceSheet,
  });
  scope.registerPanel({
    id: ShellPanels.MissingExtensionDialog,
    slot: Slots.ShellOverlay,
    priority: 130,
    component: MissingExtensionDialog,
  });

  // "Unavailable data" in the Document Explorer — a tree provider like any
  // other, so the panel never learns what a missing module is.
  scope.registerTreeProvider(missingModulesTreeProvider);
}
