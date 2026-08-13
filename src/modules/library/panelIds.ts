/*
 * Library panel ids, split out from `ui.ts` on purpose — mirrors
 * `modules/modeling/panelIds.ts`'s reasoning exactly: `ui.ts` imports every
 * editor component, so anything that only needs an ID (a workspace
 * definition, a test) must import from HERE and not drag the editor chunk
 * into the startup bundle.
 */
import { contributionId, type MenuContributionId, type PanelId } from "@/platform";
import { LIBRARY_MODULE_ID } from "./manifest";

const panelId = (name: string) =>
  contributionId<PanelId>(LIBRARY_MODULE_ID, `onecad.library.panel.${name}`);

export const LibraryPanels = {
  LibraryPanel: panelId("libraryPanel"),
  StatusSection: panelId("statusSection"),
  /** The authoring dialog's `Slots.ShellOverlay` occupant (WP-B2). */
  SaveAsComponentHost: panelId("saveAsComponentHost"),
} as const;

const menuItemId = (name: string) =>
  contributionId<MenuContributionId>(LIBRARY_MODULE_ID, `onecad.library.menu.${name}`);

export const LibraryMenuItems = {
  /** "Save as Component…" on a body row (`Slots.TreeContext`, WP-B1/B2). */
  SaveAsComponent: menuItemId("saveAsComponent"),
} as const;
