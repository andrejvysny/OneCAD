/*
 * The assistant's UI contributions.
 *
 * Split out of `register.ts` for the same reason modeling's live in `ui.ts`:
 * `bootstrap.ts` imports `registerAssistantModule` at STARTUP, so anything
 * `register.ts` imports is in the startup chunk. `AssistantPanel` pulls
 * `agentkit/client` and `agentkit/contracts` (TypeBox schemas) behind it, and the
 * start screen has no business paying for the AgentKit client to render a list of
 * recent projects.
 *
 * Registered when the EDITOR mounts (`EditorShell.tsx`), which is already a
 * deliberate code-split chunk.
 */
import { Slots, type ModuleScope } from "@/platform";
import { AssistantPanels } from "./panelIds";
import { AssistantPanel } from "./ui/AssistantPanel";

/**
 * Editor-mount-time contributions.
 *
 * Priority is mount ORDER, not visibility: the panel renders `null` unless it is
 * the active sidebar tab AND `settingsStore.assistantEnabled` is on, so a build
 * with the gate off still mounts a component — it just draws nothing. That is the
 * same shape `ModelTreePanel` and `VariablesPanel` use, and it is why the mount
 * order contract lists the panel whether or not the feature is enabled.
 */
export function contributeAssistantUi(scope: ModuleScope): void {
  // THIRD tab in the left sidebar, after ModelTree (100) and Variables (110).
  scope.registerPanel({
    id: AssistantPanels.Assistant,
    slot: Slots.ShellLeft,
    title: "Assistant",
    priority: 120,
    component: AssistantPanel,
  });
}
