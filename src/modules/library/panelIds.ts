/*
 * Library panel ids, split out from `ui.ts` on purpose — mirrors
 * `modules/modeling/panelIds.ts`'s reasoning exactly: `ui.ts` imports every
 * editor component, so anything that only needs an ID (a workspace
 * definition, a test) must import from HERE and not drag the editor chunk
 * into the startup bundle.
 */
import {
  contributionId,
  type CommandId,
  type MenuContributionId,
  type PanelId,
  type ToolId,
} from "@/platform";
import { LIBRARY_MODULE_ID } from "./manifest";

const panelId = (name: string) =>
  contributionId<PanelId>(LIBRARY_MODULE_ID, `onecad.library.panel.${name}`);

export const LibraryPanels = {
  StatusSection: panelId("statusSection"),
  /** The authoring dialog's `Slots.ShellOverlay` occupant (WP-B2). */
  SaveAsComponentHost: panelId("saveAsComponentHost"),
  /** The `LibraryModal`'s `Slots.ShellOverlay` occupant. */
  LibraryModalHost: panelId("libraryModalHost"),
} as const;

const menuItemId = (name: string) =>
  contributionId<MenuContributionId>(LIBRARY_MODULE_ID, `onecad.library.menu.${name}`);

const commandId = (name: string) =>
  contributionId<CommandId>(LIBRARY_MODULE_ID, `onecad.library.command.${name}`);

const toolId = (name: string) =>
  contributionId<ToolId>(LIBRARY_MODULE_ID, `onecad.library.tool.${name}`);

export const LibraryCommands = {
  /** "Save as Template…" — palette-reachable (spec §8, WP-B3). */
  SaveAsTemplate: commandId("saveAsTemplate"),
} as const;

export const LibraryTools = {
  /** Opens the full-size `LibraryModal` from the model-mode toolbar. */
  OpenLibrary: toolId("openLibrary"),
} as const;

export const LibraryMenuItems = {
  /** "Save as Component…" on a body row (`Slots.TreeContext`, WP-B1/B2). */
  SaveAsComponent: menuItemId("saveAsComponent"),
} as const;
