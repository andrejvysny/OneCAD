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

  // ---- Modular platform surfaces (prototype turn 2) ----
  /** Non-blocking notice: this document uses an extension that is not installed. */
  MissingExtensionBanner: panelId("missingExtensionBanner"),
  /** Non-blocking notice: this workspace's module is not installed. */
  WorkspacePlaceholder: panelId("workspacePlaceholder"),
  /** ⌘K. */
  CommandPalette: panelId("commandPalette"),
  ExtensionsManager: panelId("extensionsManager"),
  MissingExtensionDialog: panelId("missingExtensionDialog"),
  CustomizeWorkspaceSheet: panelId("customizeWorkspaceSheet"),
} as const;
