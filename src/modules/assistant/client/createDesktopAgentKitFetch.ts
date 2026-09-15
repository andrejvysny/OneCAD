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
 * §9, webview hop: bytes this transport will hold for a body nobody is reading.
 *
 * The producer's 8 MiB bound does not protect this queue — draining Rust's
 * buffer quickly into a Tauri channel moves the bytes, it does not bound them,
 * and a `ReadableStream` that is enqueued into regardless of `desiredSize` grows
 * its own internal queue behind a stalled reader. This is the bound for THIS hop.
 */
export const MAX_STREAM_BUFFER_BYTES = 8 * 1024 * 1024;

/**
 * §9: chunks this transport will hold for a body nobody is reading.
 *
 * The second axis. Byte accounting alone accepts an unbounded number of tiny
 * SSE frames, each one an array slot and a wakeup, while reporting the stream
 * as comfortably inside its budget.
 */
export const MAX_STREAM_BUFFER_ITEMS = 4096;

/** Statuses the Fetch standard forbids a body on; constructing one throws. */
const NULL_BODY_STATUS: ReadonlySet<number> = new Set([101, 103, 204, 205, 304]);

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
 * A channel, the stream it feeds, and the handle that detaches both.
 *
 * Chunks can land before anything reads the response — `invoke` has not even
 * resolved yet when the first one arrives — so messages queue and the stream
 * drains the queue once a reader asks for them.
 *
 * Three rules this shape exists to enforce:
 *
 *  1. **The queue is bounded on this hop.** OCAK1 §7 makes the SENDER hold a
 *     bounded buffer, and §9 makes every hop hold its own: the sender's bound
 *     says nothing about how much a stalled webview accumulates after the bytes
 *     have already crossed the channel.
 *  2. **Delivery is pull-driven.** Enqueuing on arrival ignores `desiredSize`
 *     and grows the `ReadableStream`'s own internal queue behind a reader that
 *     is not reading, which is the one place a bound cannot see.
 *  3. **Terminal is idempotent and detaching.** Cancelling the reader detaches
 *     the channel, so a chunk that arrives afterwards is dropped instead of
 *     throwing into a closed controller (F04/R03).
 */
function createChannelStream(): {
  channel: Channel<AssistantBridgeStreamMessage>;
  stream: ReadableStream<Uint8Array>;
  /** Terminal, idempotent: fails the stream and detaches the channel. */
  fail(error: Error): void;
  /** Detaches the channel without failing a stream nobody took. */
  detach(): void;
  /**
   * Resolves once this request can no longer deliver anything — end, error,
   * abort or reader cancellation. The caller hangs its listener cleanup off it
   * instead of reading the body, which would lock a stream the `Response` needs.
   */
  whenSettled: Promise<void>;
  /** §9 instrumentation: what this hop is currently holding. */
  held(): { bytes: number; items: number };
} {
  const queue: Uint8Array[] = [];
  let queuedBytes = 0;
  let failure: Error | null = null;
  let ended = false;
  let detached = false;
  let wake: (() => void) | null = null;
  let markSettled!: () => void;
  const whenSettled = new Promise<void>((resolve) => {
    markSettled = resolve;
  });

  const signal = (): void => {
    const resume = wake;
    wake = null;
    resume?.();
  };

  const settle = (error: Error | null): void => {
    if (ended || failure !== null) return; // first terminal message wins
    if (error === null) ended = true;
    else failure = error;
    markSettled();
    signal();
  };

  const detach = (): void => {
    if (detached) return;
    detached = true;
    // The channel object stays alive inside Tauri until the native side stops
    // sending; replacing the handler is what makes a late message harmless.
    channel.onmessage = () => {};
    queue.length = 0;
    queuedBytes = 0;
    markSettled();
    signal();
  };

  const channel = new Channel<AssistantBridgeStreamMessage>();
  channel.onmessage = (message) => {
    if (message.event === "chunk") {
      if (ended || failure !== null || detached) return;
      let bytes: Uint8Array;
      try {
        bytes = decodeBase64(message.data);
      } catch {
        // §9: a malformed chunk is a terminal failure, not a thrown exception
        // on whatever stack the channel callback happens to run on.
        settle(new AssistantBridgeError("invalid-chunk", "a stream chunk was not valid base64"));
        return;
      }
      if (
        queuedBytes + bytes.byteLength > MAX_STREAM_BUFFER_BYTES ||
        queue.length + 1 > MAX_STREAM_BUFFER_ITEMS
      ) {
        settle(
          new AssistantBridgeError(
            "stream_overflow",
            `the assistant stream exceeded this transport's buffer (${MAX_STREAM_BUFFER_BYTES} bytes / ${MAX_STREAM_BUFFER_ITEMS} chunks)`,
          ),
        );
        return;
      }
      queue.push(bytes);
      queuedBytes += bytes.byteLength;
      signal();
      return;
    }
    settle(
      message.event === "end" ? null : new AssistantBridgeError(message.code, message.message),
    );
  };

  const stream = new ReadableStream<Uint8Array>({
    async pull(controller) {
      for (;;) {
        const next = queue.shift();
        if (next !== undefined) {
          queuedBytes -= next.byteLength;
          controller.enqueue(next);
          return;
        }
        if (failure !== null) {
          controller.error(failure);
          return;
        }
        if (ended || detached) {
          controller.close();
          return;
        }
        await new Promise<void>((resolve) => {
          wake = resolve;
        });
      }
    },
    /**
     * UNSUBSCRIBE, not "stop the run". Cancelling this reader detaches the
     * webview from the delivery channel; the run keeps executing and its durable
     * event log keeps filling, so a reopened panel replays from `Last-Event-ID`.
     * Cancelling the RUN is a separate, explicit AgentKit REST operation, and
     * conflating the two would make closing a panel destroy work.
     */
    cancel() {
      detach();
    },
  });

  return {
    channel,
    stream,
    fail: (error) => {
      settle(error);
    },
    detach,
    whenSettled,
    held: () => ({ bytes: queuedBytes, items: queue.length }),
  };
}

/**
 * The one place an IPC rejection becomes a typed error (F16).
 *
 * Tauri rejects with whatever the command's error serialised to — for OneCAD
 * that is `ApiError { kind, message, diagnostics? }`, a plain object. A `catch`
 * that interpolates it renders `[object Object]`, which loses the kind, the
 * message and any diagnosis with it. Callers outside this module import this
 * rather than writing a second, differently-wrong version.
 */
export function normalizeBridgeFailure(error: unknown): AssistantBridgeError {
  if (error instanceof AssistantBridgeError) return error;
  if (error instanceof Error) return new AssistantBridgeError("bridge-error", error.message);
  if (typeof error === "string") return new AssistantBridgeError("bridge-error", error);
  if (typeof error === "object" && error !== null) {
    const record = error as Record<string, unknown>;
    const code = typeof record.kind === "string" ? record.kind : "bridge-error";
    const message =
      typeof record.message === "string" ? record.message : JSON.stringify(error);
    const diagnostics = Array.isArray(record.diagnostics) ? record.diagnostics : undefined;
    return new AssistantBridgeError(
      code,
      diagnostics === undefined || diagnostics.length === 0
        ? message
        : `${message} (${diagnostics.map((d) => String(d)).join("; ")})`,
    );
  }
  return new AssistantBridgeError("bridge-error", String(error));
}

/**
 * The response head, checked before it is turned into a `Response` (F16).
 *
 * `new Response()` throws on a status outside 200–599 and on a non-null body for
 * a null-body status, and a throw from a constructor is indistinguishable at the
 * call site from the transport itself failing. Validating first means an
 * out-of-range status is reported as the malformed head it is, rather than
 * truncated into a plausible-looking one.
 */
function requireStatus(head: AssistantBridgeResponse): number {
  const { status } = head;
  if (!Number.isInteger(status) || status < 200 || status > 599) {
    throw new AssistantBridgeError(
      "invalid-status",
      `the assistant bridge answered with status ${String(status)}, which is not a final HTTP status`,
    );
  }
  return status;
}

/**
 * The body a `Response` may legally carry for this status and method.
 *
 * A collected body that is empty comes back as `""`, and `new Response("", {
 * status: 204 })` THROWS — AgentKit's revoke-allowance route answers 204, so
 * this is a supported contract edge and not a hypothetical one (F16/R08).
 */
function responseBody(method: string, status: number, body: string | null): string | null {
  if (method === "HEAD" || NULL_BODY_STATUS.has(status)) return null;
  return body === null || body === "" ? null : body;
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

    const signal = init?.signal ?? null;
    // Read through a function, never as a narrowed property: `aborted` changes
    // between the two checks below, and the compiler's control-flow narrowing
    // would otherwise decide the second one is unreachable.
    const isAborted = (): boolean => signal?.aborted === true;
    // §2a: an already-aborted request is not dispatched at all. Sending it costs
    // the host a registration and the sidecar a handler, and the only outcome
    // available is the rejection the caller has already asked for.
    if (isAborted()) throw signal?.reason;

    const streaming = operation === STREAMING_OPERATION;
    const sink = streaming ? createChannelStream() : null;

    // The handle exists BEFORE the dispatch, so an abort during the `invoke`
    // wait has something to act on. Attaching the listener after the head
    // arrived — the old shape — missed every abort raised while waiting, which
    // is the whole window a slow host actually spends (F04/R05).
    let terminal = false;
    const onAbort = (): void => {
      if (terminal) return;
      terminal = true;
      const reason = signal?.reason;
      sink?.fail(
        reason instanceof Error
          ? reason
          : new AssistantBridgeError("aborted", "the assistant request was aborted"),
      );
      sink?.detach();
    };
    signal?.addEventListener("abort", onAbort, { once: true });
    /** Idempotent terminal cleanup: removes the listener, detaches nothing else. */
    const release = (): void => {
      signal?.removeEventListener("abort", onAbort);
    };

    let head: AssistantBridgeResponse;
    try {
      head = await invoke<AssistantBridgeResponse>(ASSISTANT_BRIDGE_COMMAND, {
        request: {
          operation,
          method,
          path: `${url.pathname}${url.search}`,
          headers: normalizeHeaders(init),
          body: normalizeBody(init),
          ...(sink === null ? {} : { onChunk: sink.channel }),
        } satisfies AssistantBridgeRequest,
      });
    } catch (err) {
      // An allocated sink is ALWAYS settled and detached when the dispatch
      // fails: a stream left open here is a body no one will ever close (F16).
      const failure = normalizeBridgeFailure(err);
      sink?.fail(failure);
      sink?.detach();
      release();
      throw failure;
    }

    // Rechecked after the transition: the abort may have landed while `invoke`
    // was in flight, in which case `onAbort` has already failed the sink.
    if (isAborted()) {
      sink?.detach();
      release();
      throw signal?.reason;
    }

    let status: number;
    try {
      status = requireStatus(head);
    } catch (err) {
      sink?.fail(normalizeBridgeFailure(err));
      sink?.detach();
      release();
      throw err;
    }

    if (sink !== null && head.streaming === true) {
      // The reader owns the lifetime from here: `cancel()` detaches the channel,
      // and `release()` runs when the body terminates for any reason. Hung off
      // `whenSettled` rather than off a reader, because taking a reader would
      // lock the very stream this `Response` is about to be handed.
      void sink.whenSettled.finally(release);
      return new Response(sink.stream, {
        status,
        statusText: head.statusText,
        headers: head.headers,
      });
    }

    // Not streaming after all: the bridge answered with a whole body, so the
    // channel this request allocated is detached rather than left subscribed.
    sink?.detach();
    release();
    return new Response(responseBody(method, status, head.body), {
      status,
      statusText: head.statusText,
      headers: head.headers,
    });
  };
}
