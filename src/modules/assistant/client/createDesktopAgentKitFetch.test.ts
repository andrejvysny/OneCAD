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
  MAX_STREAM_BUFFER_ITEMS,
  normalizeBridgeFailure,
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

// ─────────────────────────────────────────────────────────────────────────────
// Cancellation (CAN-01..03), boundedness (MEM-01/02) and Fetch faithfulness
// (REST-01). Every case here is a trace the old shape got wrong.
// ─────────────────────────────────────────────────────────────────────────────

describe("desktop AgentKit fetch — cancellation", () => {
  it("does not dispatch a request whose signal is already aborted", async () => {
    bridge({ status: 200, headers: {}, body: "{}" });
    const aborter = new AbortController();
    aborter.abort(new Error("already gone"));

    await expect(
      createDesktopAgentKitFetch()(`${ORIGIN}/v1/version`, { signal: aborter.signal }),
    ).rejects.toThrow("already gone");
    expect(calls).toHaveLength(0);
  });

  it("observes an abort raised WHILE the head is still being awaited", async () => {
    // The old shape attached its listener only after `invoke` resolved, so an
    // abort during the wait — the entire window a slow host occupies — was lost.
    let channel: Channel<AssistantBridgeStreamMessage> | undefined;
    let release: (() => void) | undefined;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    mockIPC(async (_cmd, args) => {
      channel = (args as { request: AssistantBridgeRequest }).request.onChunk;
      await held;
      return { status: 200, headers: {}, body: null, streaming: true };
    });

    const aborter = new AbortController();
    const pending = createDesktopAgentKitFetch()(`${ORIGIN}/v1/runs/r1/stream`, {
      signal: aborter.signal,
    });
    await Promise.resolve();
    aborter.abort(new Error("user cancelled"));
    release!();

    await expect(pending).rejects.toThrow("user cancelled");
    // And the channel this request allocated is detached: a late chunk is
    // dropped rather than thrown into a stream nobody owns.
    expect(() => deliver(channel!, chunk("late"))).not.toThrow();
  });

  it("cancelling the reader detaches the channel and a later chunk is harmless", async () => {
    let channel: Channel<AssistantBridgeStreamMessage> | undefined;
    bridge({ status: 200, headers: {}, body: null, streaming: true }, (request) => {
      channel = request.onChunk;
    });

    const response = await createDesktopAgentKitFetch()(`${ORIGIN}/v1/runs/r1/stream`);
    await response.body!.cancel();

    // R03: this threw a TypeError into whatever stack the channel callback ran
    // on, because the controller was already closed.
    expect(() => deliver(channel!, chunk("after cancel"))).not.toThrow();
    expect(() => deliver(channel!, { event: "end" })).not.toThrow();
  });

  it("settles and detaches an allocated sink when the dispatch itself fails", async () => {
    // F16: an `invoke` rejection used to leave the sink open and the channel
    // subscribed, so the body was one no one would ever close.
    mockIPC(() => {
      throw { kind: "worker", message: "the assistant host is not running" };
    });

    await expect(
      createDesktopAgentKitFetch()(`${ORIGIN}/v1/runs/r1/stream`),
    ).rejects.toMatchObject({
      name: "AssistantBridgeError",
      code: "worker",
      message: "the assistant host is not running",
    });
  });
});

describe("desktop AgentKit fetch — boundedness", () => {
  it("refuses an unread stream past its own buffer instead of accepting 9 MiB", async () => {
    // R04: the queue was unbounded and `desiredSize` was ignored, so a stalled
    // reader accepted every byte the producer could hand over.
    let channel: Channel<AssistantBridgeStreamMessage> | undefined;
    bridge({ status: 200, headers: {}, body: null, streaming: true }, (request) => {
      channel = request.onChunk;
    });
    const response = await createDesktopAgentKitFetch()(`${ORIGIN}/v1/runs/r1/stream`);

    const mib = btoa("x".repeat(1024 * 1024));
    for (let i = 0; i < 9; i += 1) deliver(channel!, { event: "chunk", data: mib });

    await expect(readAll(response)).rejects.toMatchObject({ code: "stream_overflow" });
  });

  it("refuses a flood of tiny chunks on the item bound", async () => {
    let channel: Channel<AssistantBridgeStreamMessage> | undefined;
    bridge({ status: 200, headers: {}, body: null, streaming: true }, (request) => {
      channel = request.onChunk;
    });
    const response = await createDesktopAgentKitFetch()(`${ORIGIN}/v1/runs/r1/stream`);

    const one = btoa("x");
    for (let i = 0; i < MAX_STREAM_BUFFER_ITEMS + 1; i += 1) {
      deliver(channel!, { event: "chunk", data: one });
    }
    await expect(readAll(response)).rejects.toMatchObject({ code: "stream_overflow" });
  });

  it("turns a malformed chunk into a terminal stream error, not a thrown callback", async () => {
    let channel: Channel<AssistantBridgeStreamMessage> | undefined;
    bridge({ status: 200, headers: {}, body: null, streaming: true }, (request) => {
      channel = request.onChunk;
    });
    const response = await createDesktopAgentKitFetch()(`${ORIGIN}/v1/runs/r1/stream`);

    expect(() => deliver(channel!, { event: "chunk", data: "not base64!!" })).not.toThrow();
    await expect(readAll(response)).rejects.toMatchObject({ code: "invalid-chunk" });
  });
});

describe("desktop AgentKit fetch — Fetch faithfulness", () => {
  it("builds a 204 with a genuinely null body", async () => {
    // R08: `new Response("", { status: 204 })` THROWS, and AgentKit's
    // revoke-allowance route answers 204 with an empty collected body.
    bridge({ status: 204, headers: {}, body: "" });

    const response = await createDesktopAgentKitFetch()(
      `${ORIGIN}/v1/chats/c1/write-policy/allowances/a1`,
      { method: "DELETE" },
    );
    expect(response.status).toBe(204);
    expect(response.body).toBeNull();
  });

  it("refuses a status outside the final-response range instead of truncating it", async () => {
    bridge({ status: 70_000, headers: {}, body: "{}" });

    await expect(createDesktopAgentKitFetch()(`${ORIGIN}/v1/version`)).rejects.toMatchObject({
      code: "invalid-status",
    });
  });

  it("normalizes a non-Error IPC rejection into a readable typed error", () => {
    const normalized = normalizeBridgeFailure({
      kind: "internal",
      message: "assistant bridge closed",
      diagnostics: ["writer failed"],
    });
    expect(normalized).toBeInstanceOf(AssistantBridgeError);
    expect(normalized.code).toBe("internal");
    // The panel's generic catch used to render this object as `[object Object]`.
    expect(String(normalized)).toContain("assistant bridge closed");
    expect(String(normalized)).toContain("writer failed");
  });
});
