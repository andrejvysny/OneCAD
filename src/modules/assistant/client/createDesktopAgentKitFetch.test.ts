/*
 * The two properties that make the assistant transport safe, plus the stream
 * reassembly the SSE lane depends on.
 *
 * `globalThis.fetch` is replaced by a THROWING spy for every case in this file:
 * the shim's whole reason to exist is that no assistant request can become a
 * real network request, and "we never call the global fetch" is only a claim
 * until something fails loudly when it happens.
 *
 * Tauri is driven through `mockIPC`, not through an injected invoke: `Channel`
 * is constructed against `window.__TAURI_INTERNALS__`, so a hand-rolled fake
 * would skip the one part of the plumbing the streaming path actually needs.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import type { Channel } from "@tauri-apps/api/core";
import {
  ASSISTANT_BRIDGE_COMMAND,
  AssistantBridgeError,
  createDesktopAgentKitFetch,
  type AssistantBridgeRequest,
  type AssistantBridgeResponse,
  type AssistantBridgeStreamMessage,
} from "./createDesktopAgentKitFetch";

const ORIGIN = "https://agentkit.invalid";

/** The bridge's own base64 framing for one chunk's bytes. */
function chunk(text: string): AssistantBridgeStreamMessage {
  return { event: "chunk", data: btoa(text) };
}

/**
 * Deliver a message the way the backend would. The Channel's own ordering
 * wrapper belongs to Tauri and is not what these tests are about, so the
 * handler this module installed is invoked directly.
 */
function deliver(
  channel: Channel<AssistantBridgeStreamMessage>,
  message: AssistantBridgeStreamMessage,
): void {
  channel.onmessage(message);
}

async function readAll(response: Response): Promise<string> {
  const reader = response.body!.getReader();
  const decoder = new TextDecoder();
  let out = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return out;
    out += decoder.decode(value, { stream: true });
  }
}

let globalFetch: typeof globalThis.fetch;
let fetchSpy: ReturnType<typeof vi.fn>;
let calls: AssistantBridgeRequest[];

/** Installs a bridge that answers every invoke with `head`, recording requests. */
function bridge(
  head: AssistantBridgeResponse,
  onRequest?: (request: AssistantBridgeRequest) => void,
): void {
  mockIPC((cmd, args) => {
    expect(cmd).toBe(ASSISTANT_BRIDGE_COMMAND);
    const request = (args as { request: AssistantBridgeRequest }).request;
    calls.push(request);
    onRequest?.(request);
    return head;
  });
}

beforeEach(() => {
  calls = [];
  globalFetch = globalThis.fetch;
  fetchSpy = vi.fn(() => {
    throw new Error("the assistant transport reached globalThis.fetch");
  });
  globalThis.fetch = fetchSpy as unknown as typeof globalThis.fetch;
});

afterEach(() => {
  globalThis.fetch = globalFetch;
  clearMocks();
});

describe("desktop AgentKit fetch — origin refusal", () => {
  it("refuses a foreign origin without touching the network", async () => {
    bridge({ status: 200, headers: {}, body: "{}" });
    const desktopFetch = createDesktopAgentKitFetch();

    await expect(desktopFetch("https://example.com/v1/version")).rejects.toMatchObject({
      name: "AssistantBridgeError",
      code: "invalid-origin",
    });
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(calls).toHaveLength(0);
  });

  it("refuses a URL that is not absolute at all", async () => {
    bridge({ status: 200, headers: {}, body: "{}" });
    await expect(createDesktopAgentKitFetch()("/v1/version")).rejects.toBeInstanceOf(
      AssistantBridgeError,
    );
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("refuses a path that is not an AgentKit route, on the right origin", async () => {
    bridge({ status: 200, headers: {}, body: "{}" });
    await expect(createDesktopAgentKitFetch()(`${ORIGIN}/v1/not-a-route`)).rejects.toMatchObject({
      code: "unknown-route",
    });
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(calls).toHaveLength(0);
  });
});

describe("desktop AgentKit fetch — request marshalling", () => {
  it("keys a GET on its RestOperation and rebuilds the response", async () => {
    bridge({
      status: 200,
      statusText: "OK",
      headers: { "content-type": "application/json" },
      body: '{"restApiVersion":"1"}',
    });

    const response = await createDesktopAgentKitFetch()(`${ORIGIN}/v1/version`, {
      headers: { accept: "application/json" },
    });

    expect(calls[0]).toMatchObject({
      operation: "getVersion",
      method: "GET",
      path: "/v1/version",
      headers: { accept: "application/json" },
      body: null,
    });
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toBe("application/json");
    await expect(response.json()).resolves.toEqual({ restApiVersion: "1" });
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("resolves a parameterised POST and forwards headers and body verbatim", async () => {
    bridge({ status: 200, headers: {}, body: '{"runId":"r1"}' });

    await createDesktopAgentKitFetch()(`${ORIGIN}/v1/chats/c%2F1/messages`, {
      method: "POST",
      headers: { "Idempotency-Key": "k-1", "content-type": "application/json" },
      body: '{"content":"hi"}',
    });

    expect(calls[0]).toMatchObject({
      operation: "submitMessage",
      method: "POST",
      path: "/v1/chats/c%2F1/messages",
      body: '{"content":"hi"}',
    });
    // Header names are lowercased on the way out; the value is untouched.
    expect(calls[0]!.headers["idempotency-key"]).toBe("k-1");
    expect(calls[0]!.onChunk).toBeUndefined();
  });

  it("carries the query string and never falls back on a non-2xx", async () => {
    bridge({ status: 404, statusText: "Not Found", headers: {}, body: '{"code":"not_found"}' });

    const response = await createDesktopAgentKitFetch()(`${ORIGIN}/v1/search?q=fillet&limit=5`);

    expect(calls[0]).toMatchObject({ operation: "searchMessages", path: "/v1/search?q=fillet&limit=5" });
    expect(response.status).toBe(404);
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});

describe("desktop AgentKit fetch — streaming", () => {
  it("reassembles a body from several chunks, in order", async () => {
    let channel: Channel<AssistantBridgeStreamMessage> | undefined;
    bridge({ status: 200, headers: {}, body: null, streaming: true }, (request) => {
      channel = request.onChunk;
    });

    const response = await createDesktopAgentKitFetch()(`${ORIGIN}/v1/runs/r1/stream`, {
      headers: { accept: "text/event-stream" },
    });

    expect(calls[0]!.operation).toBe("streamRun");
    expect(channel).toBeDefined();

    deliver(channel!, chunk("event: run.started\n"));
    deliver(channel!, chunk("data: {}\n"));
    deliver(channel!, chunk("\n"));
    deliver(channel!, { event: "end" });

    await expect(readAll(response)).resolves.toBe("event: run.started\ndata: {}\n\n");
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("surfaces stream_overflow as a stream error, after the bytes it did deliver", async () => {
    let channel: Channel<AssistantBridgeStreamMessage> | undefined;
    bridge({ status: 200, headers: {}, body: null, streaming: true }, (request) => {
      channel = request.onChunk;
    });

    const response = await createDesktopAgentKitFetch()(`${ORIGIN}/v1/runs/r1/stream`);
    // Reading as the bytes arrive, which is what the SSE client does: an
    // errored body nobody ever reads is an unhandled rejection in any runtime.
    const reader = response.body!.getReader();
    deliver(channel!, chunk("data: one\n"));
    const first = await reader.read();
    expect(new TextDecoder().decode(first.value)).toBe("data: one\n");

    deliver(channel!, {
      event: "error",
      code: "stream_overflow",
      message: "per-stream buffer exceeded",
    });
    // Not a silent truncation: the next read REJECTS, carrying the code the
    // client needs to decide to resume from `Last-Event-ID`.
    await expect(reader.read()).rejects.toMatchObject({
      name: "AssistantBridgeError",
      code: "stream_overflow",
    });
  });

  it("queues chunks that arrive before anything reads the response", async () => {
    let channel: Channel<AssistantBridgeStreamMessage> | undefined;
    bridge({ status: 200, headers: {}, body: null, streaming: true }, (request) => {
      channel = request.onChunk;
      // Straight back down the channel, inside the invoke itself — the race the
      // queue exists for: no reader, and no Response object yet either.
      deliver(request.onChunk!, chunk("early"));
    });

    const response = await createDesktopAgentKitFetch()(`${ORIGIN}/v1/runs/r1/stream`);
    deliver(channel!, chunk("-late"));
    deliver(channel!, { event: "end" });

    await expect(readAll(response)).resolves.toBe("early-late");
  });

  it("uses a plain body when the bridge answers a stream route without streaming", async () => {
    bridge({ status: 409, headers: {}, body: '{"code":"run_finished"}', streaming: false });

    const response = await createDesktopAgentKitFetch()(`${ORIGIN}/v1/runs/r1/stream`);

    expect(response.status).toBe(409);
    await expect(response.text()).resolves.toBe('{"code":"run_finished"}');
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});
