/*
 * Assistant panel ids, split out from `register.ts` on purpose — the same
 * reasoning `modules/library/panelIds.ts` and `modules/modeling/panelIds.ts`
 * carry: `register.ts` imports the panel components, so anything that only
 * needs an ID (a workspace definition, a test) must import from HERE and not
 * drag the assistant UI chunk in with it.
 */
import { contributionId, type PanelId } from "@/platform";
import { ASSISTANT_MODULE_ID } from "./manifest";

const panelId = (name: string) =>
  contributionId<PanelId>(ASSISTANT_MODULE_ID, `onecad.assistant.panel.${name}`);

export const AssistantPanels = {
  /** The left-sidebar chat panel — third tab beside Model and Variables. */
  Assistant: panelId("assistant"),
} as const;
