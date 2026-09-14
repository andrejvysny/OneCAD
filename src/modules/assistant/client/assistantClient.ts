/*
 * The assistant's AgentKit client.
 *
 * One seam, one construction site: everything in `modules/assistant/ui` talks to
 * the assistant host through this client, and the client talks to the bridge
 * through `createDesktopAgentKitFetch` — never through the browser's `fetch`.
 *
 * NOTE ON THE OPTION NAME: `createAgentKitClient` takes `fetch`. The `fetchImpl`
 * option that looks like it belongs to AgentKit is the AI PROVIDER's, a
 * different seam entirely; passing one where the other is wanted silently leaves
 * the transport on the global fetch.
 */
import { createAgentKitClient, type AgentKitClient, type FetchLike } from "agentkit/client";
import {
  ASSISTANT_BRIDGE_ORIGIN,
  createDesktopAgentKitFetch,
} from "./createDesktopAgentKitFetch";

export function createAssistantClient(
  fetchImpl: FetchLike = createDesktopAgentKitFetch(),
): AgentKitClient {
  return createAgentKitClient({ baseUrl: ASSISTANT_BRIDGE_ORIGIN, fetch: fetchImpl });
}

let client: AgentKitClient | null = null;

/**
 * The process-wide client, built on first use.
 *
 * Lazy rather than module-scope so importing the panel does not construct a
 * Tauri-bound transport in a test or in the browser dev lane; nothing here
 * touches `invoke` until a request is actually made.
 */
export function getAssistantClient(): AgentKitClient {
  client ??= createAssistantClient();
  return client;
}
