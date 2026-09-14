# OCAK1 — the assistant bridge wire contract

**Status:** normative for both implementations.
**Implementations:** `src-tauri/crates/onecad-assistant-protocol` (Rust, host side) and
`assistant-host/src/bridge/` (TypeScript, sidecar side).

OCAK1 carries traffic between the OneCAD Rust host and the supervised Bun assistant host
over the child process's stdin/stdout. It is **not** OCW1 and shares no code with it: the
geometry worker's protocol is the modeling module's contract (`docs/ARCHITECTURE.md` §3),
and nothing outside `onecad.modeling` speaks it. OCAK1 borrows OCW1's *shape* because that
shape has been proven in this codebase, not its bytes.

## 1. Framing

```
 offset  size  field
 ------  ----  ---------------------------------------------
      0     4  magic, the ASCII bytes "OCAK"
      4     4  jsonLen, u32 little-endian
      8     4  binLen,  u32 little-endian
     12     n  json, exactly jsonLen bytes of UTF-8 JSON
   12+n     m  bin,  exactly binLen bytes, opaque
```

Header is always 12 bytes. `json` MUST be a single JSON object. `bin` is uninterpreted
bytes and is empty on every envelope except `chunk`.

Limits, enforced by both sides on decode:

| Constant | Value | Rationale |
|---|---|---|
| `MAX_JSON_LEN` | 1 MiB | The implementation guide's stated starting limit for a control frame. |
| `MAX_BIN_LEN` | 8 MiB | One stream chunk. Larger payloads are split across `chunk` frames. |

**A malformed frame is fatal.** Bad magic, an over-cap length, or JSON that is not a valid
envelope tears the connection down; the supervisor then restarts the child. There is no
resync and no skip-and-continue — the same rule OCW1 follows, for the same reason: a
desynchronised length prefix cannot be recovered from without guessing.

## 2. Envelopes

Every `json` object has a string tag `t`. Unknown `t` is a protocol error, not an ignorable
frame — silently dropping an unrecognised envelope is how a stream loses an event nobody
notices.

### `hello` — sidecar → host, unsolicited, exactly once, first

```json
{ "t": "hello", "protocolVersion": 1, "hostVersion": "0.1.0",
  "agentkitContractVersion": "…", "pid": 1234, "sessionNonce": "…" }
```

The host MUST receive `hello` before anything else. Anything else first is fatal.

### `accept` / `reject` — host → sidecar, in reply to `hello`

```json
{ "t": "accept", "protocolVersion": 1, "appVersion": "…", "bridgeVersion": "0.1.0" }
{ "t": "reject", "reason": "unsupported protocol version 2" }
```

A version the peer cannot serve is refused **before any other frame is exchanged**, so an
incompatible pair can never perform a partial operation.

### `req` — either direction

```json
{ "t": "req", "id": 12, "principal": "ui", "verb": "agentkit.fetch", "payload": { } }
```

### `res` — either direction, answering one `req`

```json
{ "t": "res", "id": 12, "ok": true,  "payload": { } }
{ "t": "res", "id": 12, "ok": false, "error": { "code": "…", "message": "…" } }
```

### `chunk` / `end` — streaming response body for one `req`

```json
{ "t": "chunk", "id": 12, "seq": 0 }      // bytes ride in the bin tail
{ "t": "end",   "id": 12, "ok": true }
{ "t": "end",   "id": 12, "ok": false, "error": { "code": "stream_overflow", "message": "…" } }
```

`seq` starts at 0 and increases by one per chunk of that request. A gap is fatal. A
streaming request is answered by a `res` carrying the response head (status, headers),
then zero or more `chunk`s, then exactly one `end`.

### `cancel` — either direction

```json
{ "t": "cancel", "id": 12 }
```

Best-effort. A `res`/`end` for a cancelled id MAY still arrive and MUST be discarded by the
originator. Cancelling an unknown id is a no-op, not an error: the race is normal.

### `ping` / `pong` — liveness, either direction

```json
{ "t": "ping", "id": 7 }
{ "t": "pong", "id": 7 }
```

## 3. Request ids

Ids are `u64` and are allocated **per originator**: the host allocates even ids, the
sidecar allocates odd ids. One counter each, monotonically increasing, never reused within
a connection. This removes any need for a direction field to disambiguate, and a reused id
is a protocol error rather than a silently crossed response.

## 4. Principals — the authority rule

Every `req` carries `principal`. There are two:

| Principal | Means |
|---|---|
| `ui` | The request originated in the OneCAD webview, through a trusted `#[tauri::command]`. |
| `host` | The request originated inside the assistant host — the AgentKit loop, a tool, or a provider callback. |

**The `principal` field on an inbound frame is never trusted.** The Rust reader *stamps*
every sidecar-originated request as `host`, overwriting whatever the frame claimed. A
principal is asserted by the transport a request arrived on, never by its own payload.
This is the same reason an `actor: "user"` field cannot create authority: data does not
grant permission.

Each side keeps a verb → allowed-principals table and refuses a verb the principal may not
call. There is no wildcard entry.

## 5. Verbs

### Host → sidecar

| Verb | Principals | Meaning |
|---|---|---|
| `agentkit.fetch` | `ui` | One AgentKit REST operation. Payload is the serialised request; the reply is the response head plus, for `streamRun`, a chunk stream. |
| `shutdown` | `ui` | Finish in-flight work and exit. |

### Sidecar → host

| Verb | Principals | Meaning |
|---|---|---|
| `provider.fetch` | `host` | A model call, routed through the Rust local-provider gateway. The sidecar names a **registered provider id**, never a URL. |

There is deliberately no verb by which the sidecar can mutate a OneCAD document, read the
filesystem, spawn a process, or open a socket. Adding one is a work-package-sized decision
with its own authorization design, not a new table row.

## 6. Concurrency

Both sides MUST keep draining inbound frames while an outbound request is awaiting its
reply. A reader that blocks on its own pending response deadlocks the bridge the first time
a provider callback lands mid-request — this is the specific failure the implementation
guide names, and it is the reason the codec is duplex rather than request/response
lockstep.

## 7. Streams and backpressure

The transport gives no end-to-end backpressure: Tauri channels (host → webview) do not
provide it, and neither does a pipe write that the peer is slow to drain. Each side
therefore holds a **bounded** per-stream buffer, and on overflow ends the stream with
`end { ok: false, error.code: "stream_overflow" }`.

Overflow is a loud, resumable failure, never a silent drop. AgentKit's client resumes a run
stream from `Last-Event-ID`, so a client that sees `stream_overflow` re-subscribes and the
durable event log replays what it missed. Dropping chunks to keep a stream alive would make
the log and the client disagree with nothing to detect it.

## 8. Logging

The child's **stdout carries frames only.** Every log line goes to stderr, where the
supervisor forwards it into `tracing` under the target `assistant` — the same split the
geometry worker uses, and for the same reason: one stray `console.log` on stdout corrupts
the frame stream.
