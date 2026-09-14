/*
 * `registerAssistantModule(platform)` — the assistant's contribution surface.
 *
 * Mirrors `modules/library/register.ts`'s bootstrap/editor-mount split
 * (docs/ARCHITECTURE.md §6):
 *
 *   `contributeAssistant`   — bootstrap-time. Nothing yet: the module owns no
 *                             command, tool or service whose lifetime is the
 *                             whole app. Kept as the hook those land in, and so
 *                             `platform.moduleIds()` reports `onecad.assistant`
 *                             as present before the editor ever mounts (the
 *                             missing-extension banner reads that list).
 *
 * The editor-mount half lives in `./ui.ts`, not here: `bootstrap.ts` imports this
 * file at STARTUP, so a component import here would drag `AssistantPanel` — and
 * the AgentKit client behind it — into the startup bundle. Same split, same
 * reason, as `modules/modeling/ui.ts`.
 */
import type { ModuleScope, Platform } from "@/platform";
import { ASSISTANT_MODULE_ID } from "./manifest";

/** Bootstrap-time contributions. Empty — see the module doc comment. */
export function contributeAssistant(_scope: ModuleScope): void {
  // No tools/commands/services of its own yet.
}

export function registerAssistantModule(platform: Platform): void {
  platform.registerModule({
    id: ASSISTANT_MODULE_ID,
    version: "0.1.0",
    activate: (scope) => contributeAssistant(scope),
  });
}
