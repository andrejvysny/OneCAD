/**
 * The verb table of `docs/assistant/wire-protocol.md` §5, as payload shapes and
 * the two adapters that sit on either end of it.
 *
 * Both directions carry an HTTP-shaped exchange, and neither opens a socket.
 * `agentkit.fetch` turns a host frame into a `Request` for a handler already
 * held in memory; `provider.fetch` turns a provider client's `fetch` call into
 * a frame for the Rust gateway to perform. The symmetry is the point: AgentKit
 * is written against `fetch` at both seams, so bridging it needs no fork of
 * AgentKit and no listening port.
 *
 * What is NOT here is as load-bearing as what is. There is no verb for reading
 * a file, running a process, or touching a OneCAD document. Adding one is a
 * work package with its own authorization design, not a new entry in this file.
 */
import { Buffer } from "node:buffer";
import type { AiProviderConfig } from "agentkit/contracts";
import type { BridgePeer, HandlerResult, InboundRequest, VerbRoute } from "./peer.js";
import { BridgeRequestError } from "./peer.js";

export const VERB_AGENTKIT_FETCH = "agentkit.fetch";
export const VERB_SHUTDOWN = "shutdown";
export const VERB_PROVIDER_FETCH = "provider.fetch";

/**
 * The authority the synthesized `Request` claims.
 *
 * A `Request` needs an absolute URL and AgentKit's router only reads the path,
 * so this is a name that deliberately cannot resolve: if anything in this
 * process ever tries to actually fetch it, the failure is immediate and
 * obvious instead of quietly reaching a real server.
 */
export const SYNTHETIC_ORIGIN = "http://assistant.invalid";

/** `agentkit.fetch` / `provider.fetch` request payload — one HTTP call, serialised. */
export interface HttpRequestPayload {
  method: string;
  /** Path plus query, rooted at `/`. Never a host, never a scheme. */
  path: string;
  headers: Record<string, string>;
  /** Base64 because §1 reserves the frame's `bin` tail for `chunk` envelopes only. */
  bodyBase64?: string;
}

/** The `res` payload of a streaming HTTP verb: the head, before any body byte. */
export interface HttpResponseHead {
  status: number;
  statusText: string;
  headers: Record<string, string>;
}

/** `provider.fetch` names a registered provider id — never a URL (§5). */
export interface ProviderFetchPayload extends HttpRequestPayload {
  providerId: string;
}

/** Statuses the Fetch standard forbids a body on; constructing one throws. */
const NULL_BODY_STATUS = new Set([101, 103, 204, 205, 304]);

function headersToRecord(headers: Headers): Record<string, string> {
  const record: Record<string, string> = {};
  headers.forEach((value, key) => {
    record[key] = value;
  });
  return record;
}

function parsePayload(payload: unknown): HttpRequestPayload {
  if (typeof payload !== "object" || payload === null) {
    throw new BridgeRequestError("bad_request", "payload is not an object");
  }
  const record = payload as Record<string, unknown>;
  const method = record.method;
  const path = record.path;
  if (typeof method !== "string" || typeof path !== "string") {
    throw new BridgeRequestError("bad_request", "payload needs a string method and path");
  }
  if (!path.startsWith("/")) {
    throw new BridgeRequestError("bad_request", `path ${JSON.stringify(path)} is not rooted at /`);
  }
  const headers =
    typeof record.headers === "object" && record.headers !== null
      ? (record.headers as Record<string, string>)
      : {};
  const body = record.bodyBase64;
  if (body !== undefined && typeof body !== "string") {
    throw new BridgeRequestError("bad_request", "bodyBase64 must be a string when present");
  }
  return body === undefined
    ? { method, path, headers }
    : { method, path, headers, bodyBase64: body };
}

/**
 * §5 host → sidecar: one AgentKit REST operation, answered by the in-memory
 * handler from `@agentkit/transport-http`.
 *
 * Always a streaming answer — head first, then the body as `chunk`s — even for
 * a 200 with eight bytes of JSON. `streamRun` is Server-Sent Events and cannot
 * be anything else, and one response shape means the Rust side has one code
 * path instead of a branch that only the SSE route exercises.
 */
export function createAgentkitFetchRoute(
  restFetch: (request: Request) => Promise<Response>,
): VerbRoute {
  return {
    // §4: only the trusted `#[tauri::command]` surface may drive the assistant.
    // A request this process originated must never be able to re-enter its own
    // REST API and act as the user.
    principals: ["ui"],
    async handle(request: InboundRequest): Promise<HandlerResult> {
      const payload = parsePayload(request.payload);
      const init: RequestInit = {
        method: payload.method,
        headers: payload.headers,
        signal: request.signal,
      };
      if (payload.bodyBase64 !== undefined) {
        init.body = Buffer.from(payload.bodyBase64, "base64");
      }
      const response = await restFetch(
        new Request(new URL(payload.path, SYNTHETIC_ORIGIN).toString(), init),
      );
      const head: HttpResponseHead = {
        status: response.status,
        statusText: response.statusText,
        headers: headersToRecord(response.headers),
      };
      return { kind: "stream", payload: head, body: response.body };
    },
  };
}

/**
 * §5 host → sidecar: finish in-flight work and exit. The handler answers first
 * and stops afterwards, so the host gets its `res` rather than a closed pipe —
 * a supervisor that cannot tell a clean exit from a crash restarts the child it
 * just asked to leave.
 */
export function createShutdownRoute(onShutdown: () => void): VerbRoute {
  return {
    principals: ["ui"],
    async handle(): Promise<HandlerResult> {
      return { kind: "unary", payload: { ok: true }, onSent: onShutdown };
    },
  };
}

function stripTrailingSlash(value: string): string {
  return value.endsWith("/") ? value.slice(0, -1) : value;
}

/**
 * The `fetch` a provider client is handed instead of the network — §5's
 * `provider.fetch`, seen from the calling end.
 *
 * `OpenAiCompatibleClient` builds `${baseUrl}/chat/completions` and calls this;
 * what crosses the bridge is the provider ID and the OPERATION PATH, never the
 * URL. The host resolves the id against its own registry and injects the
 * credential, which is the whole reason this process can hold a chat loop
 * without ever holding an API key or being able to reach an endpoint of its
 * own choosing.
 *
 * `Authorization` is dropped rather than forwarded: this process has no
 * credential to send, and a blank or stale header arriving at the gateway would
 * shadow the real one.
 */
export function createGatewayFetch(peer: BridgePeer, config: AiProviderConfig): typeof fetch {
  const base = stripTrailingSlash(config.baseUrl);
  return async function gatewayFetch(
    input: string | URL | Request,
    init?: RequestInit,
  ): Promise<Response> {
    const request =
      input instanceof Request ? new Request(input, init) : new Request(input.toString(), init);
    if (!request.url.startsWith(base)) {
      // The client asked for an endpoint outside its own provider config. There
      // is no verb that can express that, and inventing one would be exactly
      // the "sidecar names a URL" case §5 forbids.
      throw new TypeError(
        `assistant gateway refused ${request.url}: outside provider '${config.id}' base URL`,
      );
    }
    const suffix = request.url.slice(base.length);
    const headers = headersToRecord(request.headers);
    delete headers.authorization;

    const payload: ProviderFetchPayload = {
      providerId: config.id,
      method: request.method,
      path: suffix.startsWith("/") ? suffix : `/${suffix}`,
      headers,
    };
    const body = await request.arrayBuffer();
    if (body.byteLength > 0) {
      payload.bodyBase64 = Buffer.from(body).toString("base64");
    }

    const answer = await peer.requestStream(
      VERB_PROVIDER_FETCH,
      payload,
      init?.signal ?? undefined,
    );
    const head = answer.head as HttpResponseHead | undefined;
    if (!head || typeof head.status !== "number") {
      throw new TypeError("assistant gateway received a provider reply with no status");
    }
    if (NULL_BODY_STATUS.has(head.status)) {
      void answer.body.cancel();
      return new Response(null, {
        status: head.status,
        statusText: head.statusText,
        headers: head.headers,
      });
    }
    return new Response(answer.body, {
      status: head.status,
      statusText: head.statusText,
      headers: head.headers,
    });
  } as typeof fetch;
}
