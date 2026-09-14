# 0016 — The assistant bridge is private framed stdio, not HTTP

**Status:** Accepted (2026-09-14)
**Supersedes nothing. Depends on:** ADR-0015.

## Context

The webview needs to talk to the AgentKit host. AgentKit ships a REST contract
(`@agentkit/contracts`'s `REST_ROUTES`), an in-memory handler (`createRestHandler`), a typed
client, and React hooks. The obvious path is to bind a local HTTP port and point the client
at it.

## Decision

**No listening socket.** The AgentKit REST semantics are reused *virtually*:

- The webview builds the normal client with an injected transport —
  `createAgentKitClient({ baseUrl: "https://agentkit.invalid", fetch: desktopFetch })`. The
  `fetch` option exists in AgentKit for exactly this (its own doc comment anticipates an
  Electron session-scoped fetch).
- `desktopFetch` refuses any origin but that synthetic one and never falls back to the
  browser's `fetch`. A malformed origin is an error, not a network request.
- Requests cross to Rust through a `#[tauri::command]`, then to the child over a private
  framed pipe, where the host calls `createRestHandler(deps)` in memory.

The frame format is **OCAK1**, specified normatively in `docs/assistant/wire-protocol.md`
and implemented twice: `src-tauri/crates/onecad-assistant-protocol` and
`assistant-host/src/bridge/`.

## Why not a local HTTP port

A bound port is reachable by anything on the machine, appears in firewall prompts, needs an
auth story of its own, and can collide. None of that buys anything when both ends are ours
and already connected by a pipe.

## Why not OCW1

`docs/ARCHITECTURE.md` §3 states that the C++ worker is the modeling module's geometry
service and nothing else speaks its protocol. Beyond the law, `onecad-protocol` is not
reusable in practice: the magic and header are baked into `parse_header`, `OcwCodec` takes
no configuration, and `ProtocolClient::connect` requires an OCW1 `hello` frame and the
`messages::Frame` envelope before it will hand back a client.

## Why not newline-delimited JSON

It is the cheap option and it means parsing unbounded lines from a child process. A length
prefix bounds every read before it happens. OCAK1 therefore mirrors OCW1's *shape* — magic,
two `u32` lengths, JSON, binary tail — and inherits its most important rule verbatim: **a
malformed frame is fatal.** There is no resync; the connection tears down and the
supervisor restarts the child.

## Consequences

Streaming crosses to the webview over `tauri::ipc::Channel`, which provides no
backpressure. Each side therefore holds a bounded per-stream buffer and, on overflow, ends
the stream with a typed `stream_overflow` error. AgentKit's client resumes from
`Last-Event-ID` against the durable event log, so an overflow costs a re-subscribe.
Dropping chunks to keep the stream alive was rejected: it would leave the client and the
log disagreeing with nothing able to detect it.

Principals (`ui`, `host`) are carried on every request and **stamped by the receiver**, never
read from the frame. A sidecar-originated request is `host` no matter what it claims. The
distinction has nothing to guard yet — there is no CAD mutation in this work package — and
exists now so it is not retrofitted around a feature later.
