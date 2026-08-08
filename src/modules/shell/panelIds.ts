/*
 * Shell module identity and panel ids, split from `register.ts` for the same
 * reason modeling's are: `register.ts` imports the chrome components, and
 * anything that only needs an ID must not drag the editor chunk into the startup
 * bundle.
 */
import { contributionId, moduleId, type ModuleId, type PanelId } from "@/platform";

export const SHELL_MODULE_ID: ModuleId = moduleId("onecad.shell");

const panelId = (name: string) =>
  contributionId<PanelId>(SHELL_MODULE_ID, `onecad.shell.panel.${name}`);

export const ShellPanels = {
  TitleBar: panelId("titleBar"),
  CornerCluster: panelId("cornerCluster"),
  NavPill: panelId("navPill"),
  StatusBar: panelId("statusBar"),
} as const;
