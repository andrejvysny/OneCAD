/*
 * Shell command / service ids, split from `register.ts` for the same reason the
 * panel ids are: `register.ts` imports the chrome components.
 */
import {
  contributionId,
  type CommandId,
  type ServiceId,
  type TreeProviderId,
} from "@/platform";
import { SHELL_MODULE_ID } from "./panelIds";

export const ShellCommands = {
  zoomFit: contributionId<CommandId>(SHELL_MODULE_ID, "onecad.shell.command.zoomFit"),
  home: contributionId<CommandId>(SHELL_MODULE_ID, "onecad.shell.command.home"),
  openExtensions: contributionId<CommandId>(
    SHELL_MODULE_ID,
    "onecad.shell.command.openExtensions",
  ),
  commandPalette: contributionId<CommandId>(
    SHELL_MODULE_ID,
    "onecad.shell.command.commandPalette",
  ),
  customizeWorkspace: contributionId<CommandId>(
    SHELL_MODULE_ID,
    "onecad.shell.command.customizeWorkspace",
  ),
  toggleLayers: contributionId<CommandId>(SHELL_MODULE_ID, "onecad.shell.command.toggleLayers"),
} as const;

/** "Unavailable data" rows — module state whose owner is not installed. */
export const ShellTreeProvider: TreeProviderId = contributionId<TreeProviderId>(
  SHELL_MODULE_ID,
  "onecad.shell.tree.missingModules",
);

export const ShellServices = {
  /** `ViewportNavigation` — framing and the home view. */
  ViewportNavigation: contributionId<ServiceId>(
    SHELL_MODULE_ID,
    "onecad.shell.viewport-navigation",
  ),
} as const;
