/*
 * Shell command / service ids, split from `register.ts` for the same reason the
 * panel ids are: `register.ts` imports the chrome components.
 */
import { contributionId, type CommandId, type ServiceId } from "@/platform";
import { SHELL_MODULE_ID } from "./panelIds";

export const ShellCommands = {
  zoomFit: contributionId<CommandId>(SHELL_MODULE_ID, "onecad.shell.command.zoomFit"),
  home: contributionId<CommandId>(SHELL_MODULE_ID, "onecad.shell.command.home"),
} as const;

export const ShellServices = {
  /** `ViewportNavigation` — framing and the home view. */
  ViewportNavigation: contributionId<ServiceId>(
    SHELL_MODULE_ID,
    "onecad.shell.viewport-navigation",
  ),
} as const;
