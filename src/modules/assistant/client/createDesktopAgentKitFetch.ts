/*
 * The virtual transport: AgentKit's `FetchLike`, served by the Rust bridge.
 *
 * There is no HTTP server. AgentKit's client builds absolute URLs from
 * `REST_ROUTES`, so it needs an origin to build them against — that origin is
 * the synthetic `https://agentkit.invalid`, which resolves nowhere by
 * construction (RFC 6761 reserves `.invalid`). Every request is marshalled into
 * one `#[tauri::command]` call and answered by the supervised assistant host
 * over OCAK1 (`docs/assistant/wire-protocol.md` §5, verb `agentkit.fetch`).
 *
 * TWO RULES THIS FILE EXISTS TO ENFORCE:
 *
 *   1. A URL that is not on the synthetic origin is REFUSED, by throwing. The
 *      webview must never be able to turn an assistant call into a real network
 *      request, and a malformed URL is a programming error, not a request.
 *   2. There is NO fallback to `globalThis.fetch`. Not on an unknown route, not
 *      on a bridge error, not when `invoke` is missing. A fallback is exactly
 *      the code path rule 1 is written to prevent.
 *
 * The request is keyed on AgentKit's own `RestOperation` rather than on a
 * transcribed path: `REST_ROUTES` is the one table both ends read, so a renamed
 * segment breaks the match here instead of silently reaching the bridge as an
 * unauthorized path.
 */
import { Channel, invoke } from "@tauri-apps/api/core";
import { REST_ROUTES, type RestOperation } from "agentkit/contracts";
import type { FetchLike } from "agentkit/client";

/** The only origin this transport will serve. Resolves nowhere by design. */
export const ASSISTANT_BRIDGE_ORIGIN = "https://agentkit.invalid";

/** The `#[tauri::command]` the bridge answers on. */
export const ASSISTANT_BRIDGE_COMMAND = "assistant_bridge_fetch";

/** The operation whose response body is a stream rather than a single blob. */
const STREAMING_OPERATION: RestOperation = "streamRun";

/**
 * Every refusal and every bridge-reported stream failure. `code` is stable and
 * machine-readable: the backend's own codes (`stream_overflow`, §7) arrive
 * verbatim, so a caller can tell a resumable overflow from a local refusal.
 */
export class AssistantBridgeError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "AssistantBridgeError";
  }
}

/** One message on a streaming response's channel. */
export type AssistantBridgeStreamMessage =
  | { readonly event: "chunk"; readonly data: string }
  | { readonly event: "end" }
  | { readonly event: "error"; readonly code: string; readonly message: string };

/** What `assistant_bridge_fetch` is invoked with. */
export interface AssistantBridgeRequest {
  /** Resolved against `REST_ROUTES`; the bridge authorizes on this, not on `path`. */
  readonly operation: RestOperation;
  readonly method: string;
  /** Path plus query, rooted at the origin — never an absolute URL. */
  readonly path: string;
  /** Lowercase header names; the transport's own `accept`/`content-type` included. */
  readonly headers: Record<string, string>;
  /** JSON text, or null for a bodiless request. */
  readonly body: string | null;
  /** Present only for a streaming operation; the body rides on it. */
  readonly onChunk?: Channel<AssistantBridgeStreamMessage>;
}

/** The response head the bridge answers with. */
export interface AssistantBridgeResponse {
  readonly status: number;
  readonly statusText?: string;
  readonly headers: Record<string, string>;
  /**
   * The whole body, for a non-streaming answer. `null` when `streaming` is
   * true — and also on a 204, which has no body to carry.
   */
  readonly body: string | null;
  /**
   * Whether the body follows on the channel. The bridge decides: a streaming
   * operation that failed its status check answers with a plain body instead,
   * which is what lets the client read the error without a stream at all.
   */
  readonly streaming?: boolean;
}

/**
 * `POST /v1/chats/:chatId/messages` → `submitMessage`.
 *
 * Built once from `REST_ROUTES` so the table stays the single source of truth
 * for what a legal assistant request is. `:param` matches one segment and never
 * a `/`, which is what keeps `/v1/runs/:runId` from swallowing
 * `/v1/runs/:runId/stream`.
 */
const ROUTE_MATCHERS: readonly {
  readonly operation: RestOperation;
  readonly method: string;
  readonly pattern: RegExp;
}[] = Object.entries(REST_ROUTES).map(([operation, route]) => ({
  operation: operation as RestOperation,
  method: route.method,
  pattern: new RegExp(
    `^${route.path
      .replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
      .replace(/:[A-Za-z0-9_]+/g, "[^/]+")}$`,
  ),
}));

function resolveOperation(method: string, pathname: string): RestOperation {
  const match = ROUTE_MATCHERS.find((r) => r.method === method && r.pattern.test(pathname));
  if (match === undefined) {
    throw new AssistantBridgeError(
      "unknown-route",
      `${method} ${pathname} is not an AgentKit REST operation`,
    );
  }
  return match.operation;
}

function requireBridgeUrl(input: string): URL {
  let url: URL;
  try {
    url = new URL(input);
  } catch {
    throw new AssistantBridgeError("invalid-origin", `"${input}" is not an absolute URL`);
  }
  if (url.origin !== ASSISTANT_BRIDGE_ORIGIN) {
    throw new AssistantBridgeError(
      "invalid-origin",
      `refusing ${url.origin} — the assistant transport serves ${ASSISTANT_BRIDGE_ORIGIN} only`,
    );
  }
  return url;
}

/** `RequestInit.headers` in any of its three shapes → lowercase pairs. */
function normalizeHeaders(init: RequestInit | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  new Headers(init?.headers).forEach((value, key) => {
    out[key.toLowerCase()] = value;
  });
  return out;
}

/**
 * The wire carries JSON text. AgentKit's transport only ever sends
 * `JSON.stringify(...)`, so anything else is a caller that has mistaken this
 * for a general-purpose fetch.
 */
function normalizeBody(init: RequestInit | undefined): string | null {
  const body = init?.body;
  if (body === undefined || body === null) return null;
  if (typeof body === "string") return body;
  throw new AssistantBridgeError(
    "unsupported-body",
    "the assistant transport sends JSON text only",
  );
}

function decodeBase64(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/**
 * A channel and the stream it feeds.
 *
 * Chunks can land before anything reads the response — `invoke` has not even
 * resolved yet when the first one arrives — so messages queue and the stream
 * drains the queue once it has a controller.
 *
 * Backpressure is deliberately absent, matching the protocol: OCAK1 §7 states
 * the transport gives none end-to-end and makes the SENDER hold a bounded
 * buffer, ending the stream with `stream_overflow` on overrun. That error
 * surfaces here as a stream ERROR, never as a quiet truncation, because a
 * truncated SSE log and the client's resume cursor would then disagree with
 * nothing able to detect it.
 */
function createChannelStream(): {
  channel: Channel<AssistantBridgeStreamMessage>;
  stream: ReadableStream<Uint8Array>;
  fail(error: Error): void;
} {
  const queue: Uint8Array[] = [];
  let controller: ReadableStreamDefaultController<Uint8Array> | null = null;
  let failure: Error | null = null;
  let ended = false;

  const flush = (): void => {
    if (controller === null) return;
    while (queue.length > 0) controller.enqueue(queue.shift()!);
    if (failure !== null) {
      controller.error(failure);
      controller = null;
      return;
    }
    if (ended) {
      controller.close();
      controller = null;
    }
  };

  const settle = (error: Error | null): void => {
    if (ended || failure !== null) return; // first terminal message wins
    if (error === null) ended = true;
    else failure = error;
    flush();
  };

  const channel = new Channel<AssistantBridgeStreamMessage>();
  channel.onmessage = (message) => {
    if (message.event === "chunk") {
      if (ended || failure !== null) return;
      queue.push(decodeBase64(message.data));
      flush();
      return;
    }
    settle(
      message.event === "end"
        ? null
        : new AssistantBridgeError(message.code, message.message),
    );
  };

  const stream = new ReadableStream<Uint8Array>({
    start(c) {
      controller = c;
      flush();
    },
  });

  return { channel, stream, fail: (error) => settle(error) };
}

/**
 * The `fetch` handed to `createAgentKitClient`.
 *
 * Every path out of here goes through `invoke`. Outside a Tauri webview that
 * throws, which is the intended outcome: this transport fails loudly rather
 * than reaching the network.
 */
export function createDesktopAgentKitFetch(): FetchLike {
  return async function desktopAgentKitFetch(input, init): Promise<Response> {
    const url = requireBridgeUrl(input);
    const method = (init?.method ?? "GET").toUpperCase();
    const operation = resolveOperation(method, url.pathname);

    const signal = init?.signal;
    if (signal?.aborted === true) throw signal.reason;

    const streaming = operation === STREAMING_OPERATION;
    const sink = streaming ? createChannelStream() : null;

    const head = await invoke<AssistantBridgeResponse>(ASSISTANT_BRIDGE_COMMAND, {
      request: {
        operation,
        method,
        path: `${url.pathname}${url.search}`,
        headers: normalizeHeaders(init),
        body: normalizeBody(init),
        ...(sink === null ? {} : { onChunk: sink.channel }),
      } satisfies AssistantBridgeRequest,
    });

    if (sink !== null && head.streaming === true) {
      // Abort ends the reader's wait. The bridge's own cancellation (OCAK1's
      // `cancel` envelope) belongs to the backend command, which is where the
      // request id lives.
      signal?.addEventListener("abort", () => sink.fail(signal.reason as Error), { once: true });
      return new Response(sink.stream, {
        status: head.status,
        statusText: head.statusText,
        headers: head.headers,
      });
    }

    return new Response(head.body, {
      status: head.status,
      statusText: head.statusText,
      headers: head.headers,
    });
  };
}
