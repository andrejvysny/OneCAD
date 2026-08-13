/*
 * Render contribution id namespace, split from `module.ts` for the same reason
 * `panelIds.ts`/`ids.ts` are split in the other modules: something that only
 * needs an id must not drag in whatever the module eventually imports for its
 * real registrations.
 *
 * No contribution ids are minted yet — this file exists so future panels,
 * tools, commands and workspaces have a canonical constructor to go through
 * (`onecad.render.<kind>.<name>`) instead of ad hoc strings.
 */
import { contributionId, type PanelId } from "@/platform";
import { RENDER_MODULE_ID } from "./manifest";

export const renderPanelId = (name: string): PanelId =>
  contributionId<PanelId>(RENDER_MODULE_ID, `onecad.render.panel.${name}`);
