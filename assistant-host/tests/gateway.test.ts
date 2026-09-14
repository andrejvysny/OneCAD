import { afterEach, describe, expect, test } from "bun:test";
import type { AiProviderConfig } from "agentkit/contracts";
import { BridgePeer } from "../src/bridge/peer.js";
import { createGatewayFetch, type ProviderFetchPayload } from "../src/bridge/verbs.js";
import { HostMock, Pipe, bytes } from "./harness.js";

const CONFIG: AiProviderConfig = {
  id: "default",
  label: "Local",
  kind: "openai-compatible",
  baseUrl: "http://provider.invalid/v1",
  defaultModel: "model",
  enabled: true,
};

const teardown: Array<() => void> = [];
afterEach(() => {
  for (const stop of teardown.splice(0)) stop();
});

async function gateway(): Promise<{ host: HostMock; fetchImpl: typeof fetch; peer: BridgePeer }> {
  const toSidecar = new Pipe();
  const fromSidecar = new Pipe();
  const host = new HostMock(toSidecar, fromSidecar);
  const peer = new BridgePeer({
    input: toSidecar,
    write: fromSidecar.write,
    identity: { hostVersion: "0.1.0", sessionNonce: "n" },
    routes: {},
  });
  teardown.push(() => {
    peer.close(new Error("test over"));
    toSidecar.close();
    fromSidecar.close();
  });
  await peer.start();
  return { host, peer, fetchImpl: createGatewayFetch(peer, CONFIG) };
}

describe("provider.fetch gateway", () => {
  test("names a registered provider id and an operation path, never a URL (§5)", async () => {
    const { host, fetchImpl } = await gateway();
    let seen: ProviderFetchPayload | undefined;
    host.routes.set("provider.fetch", async (payload, id) => {
      seen = payload as ProviderFetchPayload;
      await host.send({
        t: "res",
        id,
        ok: true,
        payload: { status: 200, statusText: "OK", headers: { "content-type": "application/json" } },
      });
      await host.send({ t: "chunk", id, seq: 0 }, bytes('{"ok":true}'));
      await host.send({ t: "end", id, ok: true });
    });

    const response = await fetchImpl("http://provider.invalid/v1/chat/completions", {
      method: "POST",
      headers: { "content-type": "application/json", authorization: "Bearer leaked" },
      body: '{"model":"m"}',
    });

    expect(seen).toMatchObject({
      providerId: "default",
      method: "POST",
      path: "/chat/completions",
    });
    expect(Object.keys(seen as object)).not.toContain("url");
    // The sidecar holds no credential; a header it does not own must not shadow
    // the one the host injects.
    expect(seen?.headers.authorization).toBeUndefined();
    expect(Buffer.from(seen?.bodyBase64 ?? "", "base64").toString()).toBe('{"model":"m"}');

    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toBe("application/json");
    expect(await response.text()).toBe('{"ok":true}');
  });

  test("a streamed body arrives incrementally, not buffered into one blob", async () => {
    const { host, fetchImpl } = await gateway();
    host.routes.set("provider.fetch", async (_payload, id) => {
      await host.send({ t: "res", id, ok: true, payload: { status: 200, statusText: "OK", headers: {} } });
      await host.send({ t: "chunk", id, seq: 0 }, bytes("data: a\n\n"));
      await host.send({ t: "chunk", id, seq: 1 }, bytes("data: b\n\n"));
      await host.send({ t: "end", id, ok: true });
    });
    const response = await fetchImpl("http://provider.invalid/v1/chat/completions", { method: "POST" });
    const reader = response.body!.getReader();
    const first = await reader.read();
    expect(new TextDecoder().decode(first.value)).toBe("data: a\n\n");
    const second = await reader.read();
    expect(new TextDecoder().decode(second.value)).toBe("data: b\n\n");
    expect((await reader.read()).done).toBe(true);
  });

  test("res{ok:false} surfaces as a normal fetch rejection", async () => {
    const { host, fetchImpl } = await gateway();
    host.routes.set("provider.fetch", async (_payload, id) => {
      await host.send({
        t: "res",
        id,
        ok: false,
        error: { code: "provider_unreachable", message: "connection refused" },
      });
    });
    await expect(
      fetchImpl("http://provider.invalid/v1/models", { method: "GET" }),
    ).rejects.toThrow(/connection refused/);
  });

  test("a URL outside the provider's own base is refused rather than bridged", async () => {
    const { fetchImpl } = await gateway();
    await expect(fetchImpl("http://evil.invalid/v1/chat/completions")).rejects.toThrow(
      /outside provider 'default' base URL/,
    );
  });

  test("a null-body status is reconstructed without a body", async () => {
    const { host, fetchImpl } = await gateway();
    host.routes.set("provider.fetch", async (_payload, id) => {
      await host.send({
        t: "res",
        id,
        ok: true,
        payload: { status: 204, statusText: "No Content", headers: {} },
      });
      await host.send({ t: "end", id, ok: true });
    });
    const response = await fetchImpl("http://provider.invalid/v1/models");
    expect(response.status).toBe(204);
    expect(response.body).toBeNull();
  });
});
