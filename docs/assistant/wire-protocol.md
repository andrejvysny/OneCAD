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

**Both versions gate the handshake.** `protocolVersion` gates the framing and
`agentkitContractVersion` gates the DTOs that ride inside it: a sidecar built against a
different AgentKit contract speaks the same envelopes and a different `agentkit.fetch`
payload. The host refuses an `agentkitContractVersion` it cannot serve with `reject`, before
any other frame — the last point at which the mismatch is one clear refusal rather than a
decode error in the middle of a user's first message. The match is exact against a list
(`SUPPORTED_AGENTKIT_CONTRACTS`, host side), not a range, because AgentKit is pre-1.0 and a
minor bump there may break anything.

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

**That order is on the WIRE, not merely in the sender's intent.** A writer that prioritises
small control frames over queued body frames — which both implementations do, so a `cancel`
cannot queue behind the flood it is meant to stop — will otherwise let a stream's `end`
overtake its own chunks and put `res → end → chunk` on the wire. The rule that prevents it:
**every frame a request sends after its head travels one lane, in production order.** The
head goes out before any body frame exists, so it cannot be overtaken by its own body, and
only unrelated requests interleave.

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

## 2a. Connection phases — the state machine both peers implement

A connection is in exactly one phase. Both implementations MUST agree on which frames are
legal in each, because a disagreement is how one side strands a reader the other has moved
on from.

| Phase | Entered by | Legal inbound | Everything else |
|---|---|---|---|
| `Idle` | the connection opening | `hello` (sidecar → host only) | fatal |
| `Handshake` | `hello` received | `accept` / `reject` (host → sidecar only) | fatal |
| `Open` | `accept` sent/received | `req`, `res`, `chunk`, `end`, `cancel`, `ping`, `pong` | fatal |
| `Closing` | local teardown begun | `res`, `chunk`, `end`, `pong` for already-issued ids | discarded, never dispatched |
| `Closed` | teardown complete | nothing | discarded |

**An application `req` before `Open` is refused, not queued.** A peer that lets work start
during `Handshake` has no configured provider and no acknowledged generation yet, so the
request can only fail in a way the caller cannot distinguish from a real error.

### Per-request states

`Issued` → (`res` head) → `Head`, and for a streaming request → `Streaming` → `Settled`.
A request is `Settled` exactly once. The rules that make that true:

- **A second `res` for a request that already has a head is FATAL to the connection.** It
  is not a replacement. Swapping the consumer silently leaves the original reader waiting
  forever while chunks flow to a sink nobody is reading.
- A `res` with `ok: false` arriving after a successful head is likewise fatal — the body is
  already owned by a consumer that must be failed, not abandoned.
- `chunk` or `end` for an unknown, already-settled, or not-yet-headed id is fatal.
- A `chunk` `seq` that is not exactly one greater than the previous is fatal (§2).
- An inbound `req` whose id has the wrong parity (§3) is FATAL on both sides. Answering it
  with an error, as one side previously did, assumes the peer's allocator is merely
  confused; a diverged allocator crosses responses, and there is no safe reply.
- An inbound `req` reusing an id that is still in flight is refused **without touching the
  in-flight registration**. Ownership is bound to the registered token, never to the bare
  integer, so a late handler cannot retire its replacement's entry.
- A non-`chunk` envelope carrying a non-empty binary tail is fatal. The framing layer
  yields the tail uninterpreted by design; rejecting it is the dispatcher's job.
- A `chunk` carrying an EMPTY binary tail is fatal. It costs a queue slot, a map lookup and
  a wakeup while charging nothing against any byte bound (§9), so it is the one frame shape
  a byte budget cannot see.

Cleanup is bound to the token issued at registration. Cancelling or dropping a request
settles it locally and detaches its consumer **without waiting for the peer** — a peer that
never answers must not pin a registration forever.

The refusal of a reused in-flight id is a `res { ok: false, error.code: "duplicate_id" }`,
and a peer over §9's inbound bound is refused with `too_many_requests`. Both leave the
connection open, because both are the other side misbehaving within a transport that still
works. A peer that receives such a refusal for an id it believes is in flight will then see
two `res` for one id and tear down under the rule above — which is correct: it reused an id
§3 says is never reused, and the refusal is the diagnostic that says so.

### The correlation-miss ladder

A `res`, `chunk` or `end` whose id is not in the pending table has THREE possible meanings,
not two, and collapsing them is how a diverged allocator goes unnoticed until it crosses two
responses:

| The id is… | Outcome | Why |
|---|---|---|
| in the receiver's own space, below its allocation cursor | discarded | A late answer to something the receiver cancelled. §2 says this race is normal. |
| in the receiver's own space, at or above its cursor | **fatal** | Nobody has allocated it. The peer is inventing correlation. |
| in the sender's own space (wrong parity) | **fatal** | The peer is answering its own id. Its allocator has diverged. |

The allocation cursor is what separates row one from row two, and every peer therefore keeps
it. "Unknown id, ignore" is not implementable without one.

### Numbers and JSON shapes

One documented policy, so neither parser's defaults decide the protocol:

| Case | Policy | Rationale |
|---|---|---|
| id above 2^53 − 1 | **fatal** | A JSON number larger than this cannot be compared for equality against the map key it is meant to match. TypeScript refuses it on decode; Rust caps at the same value rather than accepting bytes its peer will not. |
| negative or fractional id | **fatal** | Not a `u64`. |
| duplicate key in the ENVELOPE's own object | **fatal** | `JSON.parse` keeps the last; serde refuses the object. A frame whose meaning depends on which parser read it is a frame neither peer can reason about, so both refuse. |
| duplicate key inside `payload` or another opaque subtree | last wins | Verb-specific data this layer does not interpret. Both platforms resolve it identically, and pinning it is what makes that a decision rather than a coincidence. |
| unknown field on an envelope | ignored | Additive, so a newer peer may send one without partitioning the protocol. |
| `res` / `end` with `ok` and `error` disagreeing | **fatal** | `error` is present iff `ok` is false (§2). |

## 3. Request ids

Ids are `u64` and are allocated **per originator**: the host allocates even ids, the
sidecar allocates odd ids. One counter each, monotonically increasing, never reused within
a connection. This removes any need for a direction field to disambiguate, and a reused id
is a protocol error rather than a silently crossed response.

Ids are capped at **2^53 − 1**. The type on the wire is `u64`, but the TypeScript peer holds
one as a JSON number, and a value above `Number.MAX_SAFE_INTEGER` cannot be compared for
equality against the map key it is supposed to match. Both sides refuse a larger one, because
a cap only one side enforces is a divergence waiting for a peer to find it. At two ids per
request the cap is 2^52 requests on one connection.

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

### Host → sidecar, configuration

| Verb | Principals | Meaning |
|---|---|---|
| `config.install` | `ui` | Install the authoritative provider projection. Payload: `{ generation: u64, provider: { id, model, capabilities } | null }`. |

`config.install` is how Rust's configuration reaches AgentKit's provider store. It carries
**no endpoint and no credential** — the sidecar names a logical id on `provider.fetch` and
Rust resolves it, so the model-facing layer never holds a real endpoint to leak or rewrite.
A `null` provider clears the projection.

`generation` is a monotonic counter minted by Rust. The sidecar echoes the installed
generation in its `res`, and **that acknowledged value is what gates execution**: a host
may answer administrative requests while its execution gate is closed, but must not start
provider work until it has acknowledged a generation. Re-sent after every reconnect,
because a new child process starts with an empty store.

A stale generation is refused, so a reordered settings edit cannot win over a newer one.

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

## 9. Budgets — what each hop is allowed to hold

§7 says the transport gives no end-to-end backpressure. The consequence is that **every hop
holds its own bound**, and one hop's cap is never evidence about the next one: draining a
sender's 8 MiB buffer quickly into a Tauri channel MOVES the bytes, it does not bound them.

Every budget is charged when the bytes are taken and released when the consumer has them, so
the number measures what is actually held rather than what has passed through.

| Budget | Value | Charged at | Released by | Overflow |
|---|---|---|---|---|
| `MAX_JSON_LEN` | 1 MiB | frame encode and decode, both peers | — | fatal frame; a self-generated one is refused at its own call site |
| `MAX_BIN_LEN` | 8 MiB | frame encode and decode, both peers | — | fatal frame |
| per-stream buffered bytes | 8 MiB | receiver, as a `chunk` is queued | the consumer reading it | `end { stream_overflow }` outbound, `cancel` inbound |
| per-stream buffered items | 4096 chunks | receiver, as a `chunk` is queued | the consumer reading it | as above |
| minimum chunk payload | 1 byte | decoder | — | fatal frame (§2a) |
| active outbound requests | 64 per connection | registration | settlement | refused locally: `too_many_requests` |
| active inbound requests | 64 per connection | registration | handler retirement | `res { ok:false, too_many_requests }` |
| control queue depth | 64 frames | enqueue | writer flush | **connection failure** — see below |
| outbound body queue | 32 frames / 16 MiB | enqueue, in 64 KiB credits | writer flush | producer waits |
| webview stream buffer | 8 MiB / 4096 chunks | the desktop transport, per stream | the reader | terminal `stream_overflow` on the body |
| response-head deadline | 30 s, host → sidecar | request issue | head arrival | `cancel` sent, slot reclaimed, caller told |

Two hops in this table are specified here but enforced outside the bridge, and both are
currently **unbounded in the implementation**:

| Hop | Budget it needs | Where it lives |
|---|---|---|
| non-stream response bytes | 8 MiB, refused as a structured failure | `collect_body` in `src-tauri/src/assistant/commands.rs` |
| forwarded child stderr | a bounded read, with the rest of an over-long line drained and the buffer's capacity released | `forward_host_stderr` in `src-tauri/src/assistant/supervisor.rs` |

They are named here rather than left out because a budget table that omits the hops it does
not own is the same mistake as a cap that only one peer enforces: it reads as a guarantee and
is not one.

Two of these are deliberately asymmetric.

**The control queue is reserved and its overflow is fatal.** Control frames are small and
never carry a tail, so a full control queue is not a busy stream — it is a peer that has
stopped reading its stdin for 64 frames. Dropping one is not harmless: it is the `cancel`
that stops a runaway producer or the `res` that settles a caller who will otherwise wait
forever. Body frames cannot occupy this queue, which is what makes "full" mean something.

**Only the host → sidecar direction has a head deadline.** An `agentkit.fetch` is answered
out of the sidecar's memory, so a head that has not arrived in thirty seconds means the child
is wedged. A `provider.fetch` head waits on a local model that may still be loading, and a
finite RPC timeout there would cancel healthy runs. The BODY is never on a deadline in either
direction — an SSE subscription is long-lived by design.

Overflow is always a structured terminal result and never a silent drop: AgentKit's client
resumes from `Last-Event-ID`, so a client that sees `stream_overflow` re-subscribes and the
durable log replays what it missed. A dropped chunk would leave the client and the log
disagreeing with nothing able to detect it.

## 10. The shared adverse-wire fixture corpus

`docs/assistant/wire-fixtures/*.json` holds the cases that pin §2a, and **both peers execute
all of them**:

- Rust: `src-tauri/src/assistant/bridge.rs`, `every_shared_wire_fixture_produces_the_contracted_outcome`.
- TypeScript: `assistant-host/tests/wireFixtures.test.ts`.

The corpus exists because the defect it closes was not one bug. Each peer had decided
something reasonable and different about the same bytes — one tore down on a second response
head, the other swapped the consumer — and no test either of them owned could have noticed.
A case both dispatchers run is the only artefact that can.

A fixture names the peer under test only through symbolic ids, so one file describes both
directions:

| Field | Meaning |
|---|---|
| `setup` | `none`, `outboundStream` (the peer under test has one streaming request issued), or `inboundInFlight` (a handler is running and will not finish) |
| `deliver` | the frames to put on the wire, in order |
| `expect` | `close` (the connection tears down), `refuse` (a `res{ok:false}` comes back and the connection stays open), or `discard` (nothing happens and the connection stays open) |
| `refusal` | for `expect: "refuse"`, the id and `error.code` required |

Ids are written `self:N` — the Nth id in the RECEIVING peer's own space, even for the host
and odd for the sidecar — or `peer:N` in the sender's. A frame may carry `bin` (base64) or,
for bytes that structured JSON cannot express such as a duplicate key, a literal `json`
string. The verb `@inflight` is a placeholder each runner substitutes with a verb of its own
that never settles, because the two sides do not serve the same verbs (§5).

Every case ends with a `ping`. A connection that is still open answers it and a torn-down one
never will, which is the one observation that distinguishes all three outcomes without either
runner reaching inside its own implementation.

Adding a case to the corpus is how a new §2a rule is landed. A rule only one implementation
enforces is not a rule.
