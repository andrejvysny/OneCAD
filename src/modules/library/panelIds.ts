/*
 * Library panel ids, split out from `ui.ts` on purpose — mirrors
 * `modules/modeling/panelIds.ts`'s reasoning exactly: `ui.ts` imports every
 * editor component, so anything that only needs an ID (a workspace
 * definition, a test) must import from HERE and not drag the editor chunk
 * into the startup bundle.
 */
import { contributionId, type PanelId } from "@/platform";
import { LIBRARY_MODULE_ID } from "./manifest";

const panelId = (name: string) =>
  contributionId<PanelId>(LIBRARY_MODULE_ID, `onecad.library.panel.${name}`);

export const LibraryPanels = {
  LibraryPanel: panelId("libraryPanel"),
  StatusSection: panelId("statusSection"),
} as const;
