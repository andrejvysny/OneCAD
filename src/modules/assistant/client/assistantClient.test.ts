/*
 * The client is wired to the bridge transport, not to the global fetch.
 *
 * `createAgentKitClient`'s option is `fetch`; passing the transport under any
 * other name leaves the client on `globalThis.fetch` and every assistant call
 * becomes a DNS lookup for `agentkit.invalid`. This is that check.
 */
import { describe, it, expect, vi } from "vitest";
import type { FetchLike } from "agentkit/client";
import { createAssistantClient } from "./assistantClient";
import { ASSISTANT_BRIDGE_ORIGIN } from "./createDesktopAgentKitFetch";

describe("assistant client", () => {
  it("is mounted on the synthetic origin", () => {
    expect(createAssistantClient(vi.fn<FetchLike>()).baseUrl).toBe(ASSISTANT_BRIDGE_ORIGIN);
  });

  it("routes a call through the injected transport", async () => {
    const fetchImpl = vi.fn<FetchLike>(async () =>
      new Response('{"contractVersion":"1","restApiVersion":"1"}', {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    );

    await createAssistantClient(fetchImpl).getVersion();

    expect(fetchImpl).toHaveBeenCalledTimes(1);
    expect(fetchImpl.mock.calls[0]![0]).toBe(`${ASSISTANT_BRIDGE_ORIGIN}/v1/version`);
  });
});
