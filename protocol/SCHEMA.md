# OneCAD Worker Protocol — Wire Contract (SCHEMA)

Status: canonical. Protocol version `1`. Both the C++ sidecar (`worker/`) and the
Rust core (`src-tauri/crates/onecad-protocol`) implement against this document
verbatim. Any change requires a fixture bump + cross-track sign-off (see
[§13 Versioning](#13-versioningchange-policy)).

The transport is **stdio between the Rust core (parent) and one C++ worker
process (child)**. `stdout` carries frames only; all logs go to `stderr`
(grep-gated). There is **no JavaScript on this path** — only `serde_json` (Rust)
and `nlohmann::json` (C++) parse envelopes — so `u64` integers are safe as JSON
numbers (both round-trip `u64` losslessly). The frontend never sees a raw
envelope; it receives projection DTOs from Rust.

All multi-byte integers and floats are **little-endian**.

Reference: this contract realizes the decisions in the migration plan
`~/.claude/plans/act-as-senior-software-transient-popcorn.md` ("Key protocol
decisions", "Architecture (final)"). The 7 invariants in [§11](#11-invariants)
are the correctness spine; every verb below is defined so as not to violate them.

---

## Table of contents

1. [Frame layout (OCW1)](#1-frame-layout-ocw1)
2. [Identifier & scalar types](#2-identifier--scalar-types)
3. [Envelope shapes](#3-envelope-shapes)
4. [JSON encoding rules](#4-json-encoding-rules)
5. [Logical lanes, chunking & flow control](#5-logical-lanes-chunking--flow-control)
6. [Handshake](#6-handshake)
7. [Verb catalogue](#7-verb-catalogue)
8. [Error taxonomy](#8-error-taxonomy)
9. [NeedsRepair payload](#9-needsrepair-payload)
10. [Resolution ladder](#10-resolution-ladder)
11. [Invariants](#11-invariants)
12. [Signatures](#12-signatures)
13. [Versioning/change policy](#13-versioningchange-policy)
14. [Changelog](#14-changelog)

---

## 1. Frame layout (OCW1)

Every message — control or bulk — is one **frame**:

```
offset  size            field
0        4 bytes        magic   = "OCW1" = 0x4F 0x43 0x57 0x31  (u32 LE 0x3157434F*)
4        4 bytes        jsonLen (u32 LE)   length of the JSON envelope in bytes
8        4 bytes        binLen  (u32 LE)   length of the binary tail in bytes
12       jsonLen bytes  json    UTF-8 JSON envelope (no BOM, no trailing NUL)
12+jsonLen  binLen bytes binary tail (raw bytes; addressed by the envelope "bin" table)
```

\* The magic is the ASCII bytes `O C W 1` in stream order. Written/read as the
4-byte sequence `4F 43 57 31`. Implementations MUST compare the 4 bytes, not an
endian-decoded integer, to avoid endianness confusion. (`"OCW1"` as a `u32` read
little-endian is `0x3157434F`; read big-endian it is `0x4F435731`. The byte
sequence is authoritative.)

### Caps

- `jsonLen` ≤ **16 MiB** (`16 * 1024 * 1024 = 16777216`).
- `binLen` ≤ **1 GiB** (`1024 * 1024 * 1024 = 1073741824`).

A frame that declares a length over cap is a fatal `PROTOCOL_ERROR`. There is **no
resync** after a malformed frame: the reader stops, the connection is torn down,
and the worker is **restarted** (see [§8](#8-error-taxonomy)). Readers MUST NOT
attempt to scan forward for the next magic.

### Binary tail addressing

The binary tail is a flat byte region. Named sections inside it are described by a
`bin` array in the JSON envelope:

```json
"bin": [
  { "name": "mesh:body_3", "off": 0,      "len": 524288 },
  { "name": "brep:body_3", "off": 524288, "len": 91234 }
]
```

- `off` and `len` are byte offsets/lengths **relative to the start of the binary
  tail** (i.e. relative to byte `12 + jsonLen`). Both `u32`.
- Sections MUST NOT overlap and MUST lie within `[0, binLen)`.
- Section `name` is a UTF-8 string, unique within the frame.
- The order of the `bin` array is not significant; addressing is by `off`/`len`.
- The concatenation of all sections need not cover the whole tail (gaps for
  4-byte alignment are permitted); readers address only named sections.

A frame with no binary payload sets `binLen = 0` and omits `bin` (or sets `bin:
[]`).

---

## 2. Identifier & scalar types

| Type | Wire form | Notes |
|------|-----------|-------|
| `id` | JSON integer (`u64`) | Correlation id. **Rust-assigned, strictly monotonic** per connection. One request → one terminal `resp` with the same `id`. |
| `seq` | JSON integer (`u64`) | Worker's global output sequence number. Monotonic across **every** frame the worker emits on the connection. Lets Rust detect drops/reordering. |
| `documentRevision` | JSON integer (`u64`) | **Rust-owned document revision (an edit counter); ADVISORY stamp, NOT a fencing token (D4).** The worker MUST NOT reject a request on `documentRevision` (a post-edit regen legitimately runs ahead of the worker's last-accepted head). It is carried in `ExecutePlan.args`, stored in the prepared scratch, and **adopted** as the worker head's `documentRevision` at `AcceptPrepared` (worker frame stamps thereafter echo it). See [§7.2](#72-regen--executeplan). |
| `workerEpoch` | JSON integer (`u64`) | Incremented by Rust on every worker (re)start / `ResetSession`. **Fencing token.** |
| `snapshotId` | JSON integer (`u64`) | Identifies one published geometry snapshot. Bodies/maps/signatures/meshes of one publish share it (Invariant 4). |
| `jobId` | JSON integer (`u64`) | Rust-assigned id for one `ExecutePlan` job. Idempotent: re-sending the same `jobId` is a no-op if already prepared. |
| `sketchRevision` | JSON integer (`u64`) | Rust-owned sketch revision. |
| `gestureId` | JSON integer (`u64`) | Rust-assigned drag-gesture id. |
| `streamId` | JSON integer (`u64`) | Worker-assigned bulk-stream id, unique per connection. |
| `BodyId` | JSON string | Opaque, globally unique (e.g. `"body_7"`). **Minting is split (D1):** a **NewBody** body is **worker-minted deterministic** `body_<opId>` (the `opId` is the Rust-minted op record id, so replay is stable); an op whose result is **N > 1 ordered bodies** (today: a boolean split; the rule is generic to any N-body op, e.g. a multi-solid import) mints `body_<opId>:<k>` with deterministic `k`-ordering, while an op producing exactly **one** new body always mints the plain `body_<opId>` form (mirror/pattern precedent). Rust **adopts** these ids from `planStep` `bodyEvents` at `AcceptPrepared` time, validating format (`body_` prefix + a known `opId` in the plan) and uniqueness, and **rejects** the prepared plan (`PROTOCOL_ERROR`, discard — never publish) on malformation/collision. All *other* body ids (bodies loaded from a saved document) stay Rust-minted; **imported** bodies (§7.3 `ImportStep`) are worker-minted ordinal children under the N-body rule above. See [§7.2](#72-regen--executeplan). |
| `ElementId` | JSON string | Opaque, Rust-minted, **globally unique and DOES NOT embed BodyId** (e.g. `"el_00000000000004a1"`). Partition membership (which body an element belongs to) is a *mapping*, never encoded in the id. |
| `TopoKey` | JSON string | **Snapshot-scoped** topology address: `"<kind>:<index>"`, kind ∈ `f` (face) / `e` (edge) / `v` (vertex) / `b` (body). Example `"f:22"`. Valid only within the `snapshotId` that produced it. NEW scheme (OneCAD-CPP used path-style ids; this protocol uses compact snapshot-scoped TopoKeys promoted on demand to `ElementId`). |
| hash | JSON string, lowercase hex, no `0x` | 64-bit hash → 16 hex chars (e.g. `"cbf29ce484222325"`). SHA-256 → 64 hex chars. Applies to `expectedBaseHash`, `historyPrefixHash`, all signatures, `brepContentHash`, `contentHash`, `tolerancePolicyHash`, `solverPolicyHash`, `occtFingerprint`, chunk `sha256`. |
| coordinate / scalar geometry | JSON number (`f64`) | Subject to [§4](#4-json-encoding-rules) float rules. |

**Fencing (D4).** A session-mutating request is fenced on **`workerEpoch` +
`expectedBaseHash` ONLY**: the worker rejects with `PROTOCOL_ERROR` (Rust reconciles
via `GetWorkerHead`) when the request's `workerEpoch` does not match the head epoch,
or its `expectedBaseHash` does not equal the head `historyPrefixHash`.
`documentRevision` is **NOT** a fencing token — it is a Rust-owned advisory stamp
(an edit counter) that the worker stores and **adopts** as its head at
`AcceptPrepared` (see [§7.2](#72-regen--executeplan)). Fencing on it would reject
every legitimate post-edit regen, whose `documentRevision` runs ahead of the
worker's last-accepted head.

---

## 3. Envelope shapes

Every envelope is a JSON object with `v` (protocol version, `1`) and `t` (frame
type). Types: `hello`, `req`, `resp`, `progress`, `event`, `cancel`, `credit`,
`chunk`.

Frames **originating from the worker** (`resp`, `progress`, `event`, `chunk`)
carry a **stamp**: `documentRevision`, `workerEpoch`, `snapshotId`, `seq`, and
`jobId` where a job is in flight. Frames from Rust (`req`, `cancel`, `credit`)
never carry the stamp.

### 3.1 `req` (Rust → worker)

```json
{
  "v": 1,
  "t": "req",
  "id": 42,
  "verb": "Tessellate",
  "lane": "control",
  "args": { "...": "verb-specific" },
  "bin": []
}
```

- `lane`: `"control"` (default) or `"bulk"`. Omitted ⇒ `"control"`.
- `bin`: optional; present when the request carries a binary payload (e.g.
  `LoadBodies`).

### 3.2 `resp` (worker → Rust, terminal — exactly one per request `id`)

```json
{
  "v": 1,
  "t": "resp",
  "id": 42,
  "ok": true,
  "result": { "...": "verb-specific" },
  "documentRevision": 17,
  "workerEpoch": 3,
  "snapshotId": 5012,
  "jobId": 88,
  "seq": 921,
  "bin": []
}
```

On failure:

```json
{
  "v": 1, "t": "resp", "id": 42, "ok": false,
  "error": { "code": "OP_FAILED", "message": "…", "detail": { }, "retriable": false },
  "documentRevision": 17, "workerEpoch": 3, "snapshotId": 5012, "seq": 922
}
```

Exactly one terminal `resp` is emitted per request `id`. `ok` MUST be present.
`result` present iff `ok:true`; `error` present iff `ok:false`. `jobId` present
only where a job is associated.

### 3.3 `progress` (worker → Rust, non-terminal)

```json
{
  "v": 1, "t": "progress", "id": 42,
  "phase": "tessellating", "fraction": 0.4, "message": "body 2/5",
  "documentRevision": 17, "workerEpoch": 3, "snapshotId": 5012, "seq": 900
}
```

`fraction` ∈ `[0,1]` optional. Progress frames are informational and MUST NOT be
required for correctness.

### 3.4 `event` (worker → Rust, non-terminal)

Structured, correlation-scoped domain events. Used by `ExecutePlan` for per-step
results (see [§7.2](#72-regen--executeplan)).

```json
{
  "v": 1, "t": "event", "id": 42, "event": "planStep",
  "stepIndex": 3, "payload": { "...": "event-specific" },
  "documentRevision": 17, "workerEpoch": 3, "snapshotId": 5012, "jobId": 88, "seq": 905
}
```

### 3.5 `cancel` (Rust → worker)

```json
{ "v": 1, "t": "cancel", "id": 42 }
```

Cancels the in-flight request `id`. The worker MUST still emit a terminal `resp`
for `id` with `error.code = "CANCELLED"` (cancellation is cooperative; the
terminal frame is **never dropped**). If `id` is already complete, `cancel` is a
no-op.

### 3.6 `credit` (Rust → worker) — bulk flow control

```json
{ "v": 1, "t": "credit", "lane": "bulk", "bytes": 4194304 }
```

Grants `bytes` of additional bulk-lane byte budget. See
[§5.3](#53-byte-budget-flow-control).

### 3.7 `chunk` (worker → Rust) — bulk stream frame

Two kinds, discriminated by `kind`:

Manifest (first frame of a stream):

```json
{
  "v": 1, "t": "chunk", "id": 42, "streamId": 700, "kind": "manifest",
  "purpose": "mesh", "count": 8, "totalBytes": 4194304,
  "sha256": "…64 hex…", "meta": { "bodyId": "body_3", "lod": "coarse", "format": "MESH1" },
  "documentRevision": 17, "workerEpoch": 3, "snapshotId": 5012, "jobId": 88, "seq": 906
}
```

Data (frames `index` 0…`count`-1):

```json
{
  "v": 1, "t": "chunk", "id": 42, "streamId": 700, "kind": "data",
  "index": 0, "byteOffset": 0,
  "bin": [ { "name": "chunk", "off": 0, "len": 524288 } ],
  "documentRevision": 17, "workerEpoch": 3, "snapshotId": 5012, "jobId": 88, "seq": 907
}
```

The receiver assembles the payload by `byteOffset`, verifies assembled length ==
`totalBytes` and SHA-256 == `sha256`, then hands the buffer off. `purpose` ∈
`"mesh"` | `"brep"`. See [§5.2](#52-chunked-bulk-streams).

---

## 4. JSON encoding rules

- **camelCase** for all object keys and enum-tag string values (e.g. `opType`,
  `documentRevision`, `"throughAll"` — but see op `opType`/`kind` tags which keep
  their PascalCase spelling as domain type names, e.g. `"Extrude"`,
  `"ThroughAll"`, `"Union"`, matching OneCAD-CPP `operationTypeName`).
- **64-bit hashes are hex strings**, never numbers ([§2](#2-identifier--scalar-types)).
- **`NaN`, `+Infinity`, `-Infinity` are rejected** on read → `PROTOCOL_ERROR`.
  Producers MUST NOT emit them.
- **`-0` is normalized to `0`** by producers; readers treat `-0.0` and `0.0` as
  equal.
- No trailing whitespace requirement; parsers MUST accept minified JSON.
- Unknown object keys are **ignored** by readers (forward-compat), except inside a
  frame header (`v`,`t`,`id`,`ok`) where they are errors. Op params carry unknown
  keys forward (Rust `flatten extra`); the worker ignores keys it does not know.
- Duplicate keys in one object → `PROTOCOL_ERROR`.
- Integers that exceed their declared width (`u64`) → `PROTOCOL_ERROR`.

---

## 5. Logical lanes, chunking & flow control

### 5.1 Lanes

Two **logical** lanes multiplex over the single stdio frame stream:

- **control** — requests, responses, progress, events, cancel, credit,
  handshake, all diagnostics and NeedsRepair state. Never blocked by flow
  control. Small, latency-sensitive.
- **bulk** — chunk streams carrying MESH1 meshes and BREP blobs. Subject to
  byte-budget credit.

Because meshes/BREP are **chunked**, control frames (cancel, progress, credit)
**interleave** with bulk frames: a cancel or a solver response is never stuck
behind a 50 MB mesh. A worker that has bulk data to send MUST yield the writer
between bulk frames so pending control frames go out first.

### 5.2 Chunked bulk streams

A bulk payload is transmitted as one stream: a **manifest** frame followed by
`count` **data** frames ([§3.7](#37-chunk-worker--rust--bulk-stream-frame)). The
terminal `resp` of the producing verb references the stream(s) it emitted, e.g.:

```json
"result": { "meshes": [ { "bodyId": "body_3", "streamId": 700,
  "format": "MESH1", "totalBytes": 4194304, "sha256": "…" } ] }
```

The worker MAY inline a small bulk payload (≤ negotiated `chunkSize`, default
**1 MiB**) directly in the terminal `resp`'s binary tail instead of opening a
stream; in that case the `resp` result references a `bin` section name rather than
a `streamId`. Payloads larger than `chunkSize` MUST be chunked so control frames
interleave.

Stream integrity: the manifest's `sha256` is the SHA-256 of the concatenated
payload bytes (all data frames in `byteOffset` order). A mismatch is a
`PROTOCOL_ERROR` → restart.

### 5.3 Byte-budget flow control

Bulk data flows worker → Rust. Rust grants credit; the worker spends it:

- Rust sends `credit{lane:"bulk", bytes:N}` control frames.
- The worker MUST NOT have more than the outstanding-credit total of **bulk
  payload bytes** (sum of data-frame `bin` lengths) in flight beyond what has been
  granted. When credit is exhausted it pauses the bulk stream (control frames keep
  flowing).
- Rust replenishes credit as it consumes/assembles. Initial credit is granted at
  handshake (`initialBulkCredit`, default **8 MiB**).
- Manifest and control frames do **not** consume bulk credit.

### 5.4 Never-dropped classes

Cancellation acknowledgements, terminal `resp` frames (including error terminals),
and NeedsRepair state are control-lane and MUST NEVER be dropped or coalesced away
by flow control or backpressure.

---

## 6. Handshake

Immediately after spawn, the worker emits an unsolicited `hello` frame (it is the
only worker frame with `t:"hello"` and no request `id`). Rust reads it before
sending any `req`.

Worker → Rust:

```json
{
  "v": 1,
  "t": "hello",
  "seq": 0,
  "result": {
    "protocolVersion": 1,
    "workerVersion": "0.1.0",
    "occt": { "version": "7.9.3", "fingerprint": "9a1c33f0e7b24d10" },
    "quantizationVersion": 1,
    "solverPolicyVersion": 1,
    "capabilities": [
      "op.sketch", "op.extrude", "op.revolve", "op.fillet", "op.chamfer",
      "op.boolean", "op.importStep", "solver.planegcs", "tessellate.mesh1",
      "io.step", "io.step.import", "io.stl", "io.obj", "checkpoint.v1"
    ],
    "limits": { "chunkSize": 1048576, "initialBulkCredit": 8388608 }
  }
}
```

Rust verifies `protocolVersion == 1` and applies the fingerprint policy
(migration plan; V1/V2 §8): matching fingerprint ⇒ proceed; mismatch ⇒ warn →
deterministic rebuild → read-only on failure. Rust then drives
[`OpenSession`](#71-lifecycle).

- `occt.fingerprint`: 64-bit hash of `{occtVersion, build flags, relevant
  algorithm knobs}`. Governs BREP/checkpoint cache compatibility.
- `quantizationVersion`: descriptor quantization scheme (currently `1` = `1e-6`
  quantization, FNV-1a 64-bit; see [§10](#10-resolution-ladder)).
- `solverPolicyVersion`: PlaneGCS policy/tuning revision.

---

## 7. Verb catalogue

Conventions: each verb shows `args` (request) and `result` (success response).
Only the verb-specific bodies are shown; the outer frame wrapping is per
[§3](#3-envelope-shapes). Fencing tokens `documentRevision`/`workerEpoch` appear
in `args` for every session-mutating verb.

### 7.1 Lifecycle

#### Hello
Emitted unsolicited by the worker; see [§6](#6-handshake). Not a `req`.

#### Shutdown
Graceful stop. Worker flushes, replies, then exits 0.

```json
// req.args
{}
// result
{ "goodbye": true }
```

#### OpenSession

```json
// req.args
{
  "documentId": "doc_1",
  "documentRevision": 0,
  "workerEpoch": 3,
  "tolerancePolicy": { "linear": 1e-7, "angular": 1e-9, "tolerancePolicyHash": "b2c9…" },
  "mode": "determinism"
}
// result
{ "sessionOpen": true, "workerHead": { "documentRevision": 0, "snapshotId": 0 } }
```

`mode` ∈ `"determinism"` (single-threaded OCCT, `parallel:false`, reproducible)
| `"fast"` (parallelism permitted; must still satisfy Invariant 5 — never change
IDs/mappings, only performance). One session per document (V1).

#### CloseSession

```json
// req.args
{ "documentId": "doc_1", "workerEpoch": 3 }
// result
{ "sessionClosed": true }
```

#### ResetSession
Drops all session + scratch state, **increments `workerEpoch`** (Rust echoes the
new epoch in subsequent requests), keeps the process alive.

```json
// req.args
{ "documentId": "doc_1", "workerEpoch": 3 }
// result
{ "reset": true, "workerEpoch": 4 }
```

#### GetWorkerHead
Reconciliation probe after a suspected desync (no side effects).

```json
// req.args
{}
// result
{ "documentRevision": 17, "workerEpoch": 3, "snapshotId": 5012,
  "historyPrefixHash": "7f1a…", "hasScratch": false }
```

### 7.2 Regen — ExecutePlan

Regen is an **ExecutePlan** model (NOT per-op). Rust compiles an immutable plan;
the worker executes step-by-step into **scratch job state** (never mutating the
active session mid-plan), streams per-step `event`s, stops at the first
failure/NeedsRepair preparing snapshot `m−1`, and ends with a terminal
`PlanPrepared` resp. Rust then publishes (`AcceptPrepared`) or drops
(`DiscardPrepared`). An interactive single-op commit is a plan of length 1.

#### ExecutePlan

```json
// req.args
{
  "jobId": 88,
  "documentRevision": 17,
  "workerEpoch": 3,
  "expectedBaseHash": "7f1a2b3c4d5e6f70",     // opaque base token (Rust-minted)
  "prefixHashes": [ "a1b2…", "c3d4…", "e5f6…" ],  // opaque per-executed-op tokens
  "baseCheckpoint": { "stepIndex": 2, "checkpointId": "ckpt_9" },  // optional
  "policyVersions": { "quantizationVersion": 1, "solverPolicyVersion": 1,
                      "descriptorVersion": 1, "resolverVersion": 1, "signatureVersion": 1 },
  "targetStep": 6,
  "artifacts": { "tessellate": { "lod": "coarse", "includeEdges": true } },
  "ops": [ /* ordered op payloads — see §7.3 */ ]
}
```

- **Fencing (D4) — `workerEpoch` + `expectedBaseHash` ONLY.** The worker fences an
  `ExecutePlan` on its `workerEpoch` (must match the head epoch) and its
  `expectedBaseHash` (must equal the head `historyPrefixHash`); either mismatch ⇒
  `error.code = "PROTOCOL_ERROR"` (Rust reconciles via `GetWorkerHead`).
  `documentRevision` is a **Rust-owned advisory stamp (an edit counter) and is NEVER
  fenced** — the worker stores the plan's `documentRevision` in the prepared scratch
  and **adopts** it as its head `documentRevision` at `AcceptPrepared` (rather than
  incrementing a worker-owned accept counter). A post-edit regen legitimately carries
  a `documentRevision` ahead of the worker's last-accepted head, so rejecting on it
  would break every such regen.
- **From-0 plans are always base-valid (D5).** A **from-0 plan** — one with **no
  `baseCheckpoint`** AND `expectedBaseHash` == the empty-prefix anchor (`e3b0c442…`)
  — is ALWAYS base-valid: the worker **SKIPS the `expectedBaseHash` head-hash
  comparison**, builds the scratch from a **genuinely empty base** (no bodies, no
  partitions — discarding any prior head state), and on `AcceptPrepared` **REPLACES
  the head wholesale** (bodies, partitions, `historyPrefixHash` = the echoed last
  prefix token, adopted `documentRevision`, bumped `snapshotId`). Rationale:
  `expectedBaseHash` pins the base state a plan builds on; a from-0 plan's base IS
  empty by definition, so the precondition is satisfiable regardless of the head. V1
  is full-replay + wholesale-publish (the `RegenPlanner` always emits from-0 plans,
  and checkpoints are UNSUPPORTED — [§7.7](#77-checkpoints)); after the first
  `AcceptPrepared` the head token is nonzero, so without this rule every subsequent
  regen would fail the `expectedBaseHash` precondition. `workerEpoch` fencing and all
  `AcceptPrepared`/`DiscardPrepared` fencing are **unchanged**. **Incremental plans**
  (`expectedBaseHash` != the empty anchor, e.g. a checkpoint-accelerated regen) keep
  the **strict head-hash fence** exactly as above.
- **Hash provenance — Rust is the sole hash authority.** `expectedBaseHash` and
  every entry of `prefixHashes` are **opaque tokens minted by Rust**. Rust computes
  them from the **geometry-relevant canonical wire-op form** of each op — the
  sorted-key JSON of `{opId, opType, stepIndex, inputs, params, determinism}`,
  SHA-256 over the newline-joined lines, lowercase hex (the empty base is the
  SHA-256 anchor `e3b0c442…`). The form deliberately **excludes** record-level
  cosmetics (`name`, record `extra`, the `suppressed` flag) so a rename never
  invalidates a checkpoint while any geometry-affecting edit does. **The worker
  MUST store/compare/echo these tokens verbatim and MUST NOT recompute them** — it
  has no visibility into the Rust record shape and any independent computation
  would diverge.
- `expectedBaseHash`: the worker compares its restored/replayed base against this
  opaque token before executing; mismatch ⇒ `error.code = "PROTOCOL_ERROR"` (Rust
  reconciles). Precondition enforcement (migration plan defenses).
- `prefixHashes`: one opaque token **per executed op**, in `ops` order —
  `prefixHashes[i]` is the history-prefix token **after executing `ops[i]`**.
  Suppressed steps are not in `ops`, so this array is indexed by execution order,
  not timeline step index. On `PlanPrepared` the worker echoes the token for its
  **last executed op** (or `expectedBaseHash` when only the base is valid) as
  `historyPrefixHash`; Rust verifies that echo (mismatch ⇒ `PROTOCOL_ERROR`).
- `baseCheckpoint`: optional; if present the worker restores it as the base
  instead of replaying from empty.
- `ops`: the ordered op slice; each op is executed on the **exact snapshot
  produced by its predecessor** (Invariant 3).

Per-step `event`s (`event:"planStep"`), one per executed step:

```json
{
  "stepIndex": 3,
  "bodyEvents": [ { "kind": "created", "bodyId": "body_3" },
                  { "kind": "modified", "bodyId": "body_1" } ],
  "elementMapDelta": { "added": [ /* {elementId, topoKey, kind, bodyId} */ ],
                       "removed": [ /* elementId */ ],
                       "relabeled": [ /* {elementId, topoKey, kind, bodyId} */ ] },
  "needsRepair": [ /* NeedsRepair payloads — §9 — STATE, not error */ ],
  "signatures": { "geometry": "aa11…", "bodyLifecycle": "bb22…", "referencedBinding": "cc33…" },
  "diagnostics": [ { "severity": "warning", "code": "…", "message": "…" } ]
}
```

- `elementMapDelta.added` / `.relabeled` entries carry a **REQUIRED `bodyId`**:
  `{ elementId, topoKey, kind, bodyId }`. A single step can create/modify several
  bodies, so each element names its owning body **explicitly** — Rust folds the
  partition from this field. (Without it Rust would have to guess the body, which
  mis-partitioned elements when one step produced two bodies.) `bodyId` is the
  partition the element currently maps to; an element's *identity* (`elementId`)
  never changes because geometry changed (Invariant 1) — only its `bodyId`/`topoKey`
  moves across split/merge.
- **`bodyEvents` NewBody id minting + adoption (D1).** A `{ "kind": "created" }`
  event's `bodyId` is **worker-minted deterministic** `body_<opId>` — `<opId>` is
  the Rust-minted op record id of the step that produced the body, so the id is a
  pure function of the (Rust-owned) plan and replay is stable across worker
  processes. Rust **adopts** each `created` id: at `AcceptPrepared` it validates the
  `body_` prefix, that `<opId>` is a **known op in the plan**, and **uniqueness**
  (no collision with a session body or a duplicate earlier in the same plan); a
  malformed or colliding id **rejects** the prepared plan (the worker's terminal is
  treated as `PROTOCOL_ERROR`, the scratch is **discarded, never published**).
  `modified`/`deleted` events reference bodies that already exist. A **boolean split**
  (a boolean-mode op — Boolean, or boolean-mode Extrude/Revolve — whose result is
  **multi-solid**, in practice a Cut/Intersect that bisects a body) **deletes the
  parent** (a `deleted` event) and mints ordered split children `body_<opId>:<k>`, each
  emitted as its **own `created` event** and adopted + fenced under the **same** D1
  rules above (the `body_` prefix + `<opId>` a known plan op, the `:<k>` ordinals
  contiguous from 0, ids unique — else the plan is rejected). A **single-solid**
  boolean result instead emits a `modified` on the surviving target (its `BodyId`
  preserved). The `:<k>` rule is **generic**: ANY op whose result is N > 1
  deterministically-ordered bodies mints `body_<opId>:<k>` children under these
  same adoption rules — a boolean split is today's only producer, but the naming
  contract does not assume a `deleted` parent accompanies the `created` children
  (an op may create N children ex nihilo). The ordinal domain is **unbounded**
  (adoption fences contiguity-from-0 and uniqueness, never a maximum). See the
  2026-07-19 (M5a) and 2026-07-22 (Revolve parity) changelog
  entries for the shipped derived-uuid representation + interner. This is a normative
  refinement of the §2 `BodyId` "Rust-minted" note: for NewBody and split children the
  worker mints and Rust adopts+fences, rather than Rust pre-minting (split/merge body
  counts are unknowable before OCCT executes, so pre-minting could never cover them
  anyway).

Terminal resp — `PlanPrepared`:

```json
// result
{
  "planPrepared": true,
  "preparedSnapshotId": 5013,
  "lastValidStep": 6,          // = targetStep on full success; < targetStep if stopped early
  "stoppedReason": "completed", // "completed" | "opFailed" | "needsRepair"
  "perStepResults": [
    { "stepIndex": 0, "status": "ok",          "bodyIds": ["body_3"] },
    { "stepIndex": 1, "status": "ok",          "bodyIds": ["body_3"] },
    { "stepIndex": 6, "status": "needsRepair", "refCount": 1 }
  ],
  "historyPrefixHash": "9c4d…"
}
```

The prepared snapshot is held in scratch, NOT published. `preparedSnapshotId`
becomes live only after `AcceptPrepared`.

#### AcceptPrepared
Publishes the prepared scratch snapshot into the active session atomically. The
worker re-fences **`workerEpoch` ONLY** (a restart between prepare and accept bumps
the epoch — Rust catches that here); `documentRevision` is not fenced. On publish the
worker **adopts the accepted plan's `documentRevision`** (the value carried in the
originating `ExecutePlan`, held in the scratch) as its head `documentRevision` —
**not** a worker-owned `+1` accept counter (D4). The result echoes the adopted head
revision.

```json
// req.args  (documentRevision = the plan's Rust-owned edit counter)
{ "jobId": 88, "documentRevision": 17, "workerEpoch": 3 }
// result  (head ADOPTS the plan's documentRevision — echoes 17, not 18)
{ "accepted": true, "snapshotId": 5013, "documentRevision": 17 }
```

#### DiscardPrepared
Drops the scratch job state; session unchanged.

```json
// req.args
{ "jobId": 88 }
// result
{ "discarded": true }
```

#### Double-`ExecutePlan` while prepared (idempotency rule)

A worker holds **at most one** prepared scratch job at a time. When an
`ExecutePlan` arrives while a job is already prepared (awaiting
`AcceptPrepared`/`DiscardPrepared`):

- **Same `jobId`** — the request is a **retransmit** (Rust job ids are idempotent).
  The worker MUST NOT re-execute; it replies with the **cached `PlanPrepared`** for
  that job (byte-identical `preparedSnapshotId`/`historyPrefixHash`/`perStepResults`).
- **Different `jobId`** — a second plan cannot prepare over an outstanding one. The
  worker replies `error.code = "PROTOCOL_ERROR"` with
  `detail = { "preparedJobId": <held>, "requestedJobId": <new> }` and leaves the
  held prepared job untouched. Rust must `AcceptPrepared`/`DiscardPrepared` the
  outstanding job before sending a new plan.

### 7.3 Op payload schemas (vertical slice)

Each op in `ExecutePlan.ops` is:

```json
{
  "opType": "Extrude",
  "opId": "op_5",
  "inputs": [ /* semantic refs — see below */ ],
  "params": { /* opType-specific */ },
  "determinism": {
    "parallel": false,
    "occtOptions": { "fuzzyValue": 0.0, "useOBB": false },
    "tolerancePolicyHash": "b2c9…"
  }
}
```

`opType` ∈ `Sketch` | `Extrude` | `Revolve` | `Fillet` | `Chamfer` | `Boolean`
| `Shell` | `LinearPattern` | `CircularPattern` | `MirrorBody` | `ImportStep`
(the M6a breadth ops and the 2026-08-02 `ImportStep` extend the original vertical
slice — see the [Changelog](#14-changelog)).
`Loft` and `Sweep` remain **`UNSUPPORTED`** ([§8](#8-error-taxonomy)). Values keep
OneCAD-CPP `operationTypeName` spelling (PascalCase).

**Scalar / dimension fields.** Every dimensional param (`distance`, `radius`,
`angleDeg`, `thickness`, `spacing`, …) is a **scalar**: it MAY be either a bare
JSON number (`"distance": 25.0`, as the examples below spell it for brevity) **or
a `{ "value": <number>, "expr"?: <string> }` object** (`expr` = a bare V1 variable
name). Readers — this worker AND the Rust core — MUST accept both forms. The Rust
core **normalizes to the object form on write**, so an `ExecutePlan` op authored
by the core arrives here as `{ "value": … }`; hand-authored/legacy payloads may
carry a bare number. `NaN`/`±Infinity` are rejected either way ([§4](#4-json-encoding-rules)).

**Semantic reference** (`inputs[]` element) — the topological input to an op,
carried as evidence + identity so the resolution ladder can rebind after edits
(Invariant 2/3):

```json
{
  "primary": { "bodyId": "body_1", "elementId": "el_…4a1", "kind": "face" },
  "intent":  { "version": 1, "kind": "face",
               "descriptor": { /* see §10 descriptor fields */ } },
  "anchor":  { "worldPoint": [12.0, 3.5, 0.0],
               "surfaceUv":  [0.25, 0.75],
               "localFrame": { "origin": [12.0,3.5,0.0], "x": [1,0,0], "y": [0,1,0], "z": [0,0,1] },
               "adjacencyHint": "d41d8cd98f00b204" }
}
```

- `primary.kind` ∈ `body` | `face` | `edge` | `vertex`.
- `intent.descriptor` is the frozen descriptor captured when the ref was authored;
  it is **evidence, never identity** (Invariant 2). The worker MUST NOT overwrite
  the stored anchor with an op's own output (Invariant 3).

**Sketch** (`op.sketch`) — materializes a sketch feature; sketch geometry is
authored/solved in the [solver lane](#74-sketch-solver-lane) but a plan carries
the full authoritative sketch so replay is deterministic.

```json
// params
{
  "sketchId": "sk_1",
  "plane": {
    "kind": "XY",
    "origin": [0,0,0], "xAxis": [0,1,0], "yAxis": [-1,0,0], "normal": [0,0,1]
  },
  "entities": [
    { "id": "e1", "type": "Line",   "p0": [0,0], "p1": [40,0] },
    { "id": "e2", "type": "Line",   "p0": [40,0], "p1": [40,20] },
    { "id": "e3", "type": "Arc",    "center": [0,20], "radius": 40, "start": [40,20], "end": [0,60] },
    { "id": "e4", "type": "Circle", "center": [10,10], "radius": 3 },
    { "id": "e5", "type": "Ellipse", "center": [20,10], "majorR": 6, "minorR": 3, "rotation": 0.25 }
  ],
  "constraints": [
    { "id": "c1", "type": "Horizontal", "entities": ["e1"] },
    { "id": "c2", "type": "Coincident", "entities": ["e1", "e2"], "positions": ["End","Start"] },
    { "id": "c3", "type": "Distance",   "entities": ["e1"], "value": 40.0 }
  ]
}
```

- `plane.kind` ∈ `XY` | `XZ` | `YZ` | `custom`. A `custom` plane carries an arbitrary
  `origin`/`xAxis`/`yAxis`/`normal` and is what a sketch attached to a DATUM or a
  model FACE sends (the attachment itself is core-owned and never crosses the
  wire — see the 2026-07-26 changelog entry). **Hard invariant — non-standard
  XY basis** (ported verbatim from OneCAD-CPP `Sketch.h` `SketchPlane::XY()`):
  `xAxis = (0,1,0)`, `yAxis = (−1,0,0)`, `normal = (0,0,1)` (User X → World Y+,
  User Y → World X−). `XZ` = `{x:(0,1,0), y:(0,0,1), n:(1,0,0)}`; `YZ` =
  `{x:(−1,0,0), y:(0,0,1), n:(0,1,0)}`. Producers MUST send these exact bases for
  the named planes; readers MUST lock-test them.
- `entities[].type` ∈ `Point` | `Line` | `Arc` | `Circle` | `Ellipse` | `Spline`.
  - **Solver-lane entity support (§7.4).** The sketch solver lane materializes
    `Point` | `Line` | `Arc` | `Circle` | `Ellipse`. `Spline` is **UNSUPPORTED**
    on the solver lane: a `SketchUpsert`/gesture carrying one fails wire
    translation (recoverable `OP_FAILED`, "unsupported entity type"). It remains
    valid in the frozen Sketch op geometry for future solver support.
  - An **`Ellipse`** carries its center INLINE exactly like `Circle`, plus the
    three shape scalars:

    ```json
    { "id": "e5", "type": "Ellipse", "center": [0,0], "centerRef": "<uuid>",
      "majorR": 6.0, "minorR": 3.0, "rotation": 0.25, "construction": false }
    ```

    `majorR`/`minorR` are the semi-axes in mm; `rotation` is the major axis's
    angle from the sketch +X axis in **radians** (optional, default `0`), the
    same wire-domain rule the `Angle` constraint follows. The reader mints the
    center `Point` itself and registers the `<id>.center` handle (the `Circle`
    contract), so the center is addressable as a `SolveDrag` `pointId`.
    **Parameter normalization**: a reader MAY enforce `majorR >= minorR` by
    swapping the radii and adding π/2 to `rotation` (the reference implementation
    does — `Sketch::addEllipse`). A reader that normalizes MUST echo the
    normalized `majorR`/`minorR`/`rotation` on its return wire, so a producer
    never re-sends parameters the reader does not hold.
  - An **`Arc`**'s START and END points are addressable. The reader mints a
    `Point` for each at its derived coordinate (center + radius + angle) and
    registers the handles `<id>.start` and `<id>.end`, exactly as it mints and
    registers `<id>.center` for a `Circle`/`Arc`/`Ellipse`. The three are kept
    consistent by an **internal, DOF-neutral arc-rules constraint** (the
    reference implementation uses PlaneGCS `addConstraintArcRules` under the
    reserved tag `0`): it adds four parameters and four independent equations, so
    the diagnosed `dof` is unchanged, and — being internal — it MUST NOT appear
    in `conflicting[]` or be reported as a redundant constraint. Consequences,
    both normative: a `Coincident` MAY name an arc with `positions` `Start`/`End`
    (the example above), and a `SolveDrag` MAY address `<id>.start` as its
    `pointId` (§7.4). A reader that does not mint them MUST fail such a
    constraint loudly rather than silently binding the arc's center.
    The arc's stored parameterization is unchanged — it is still
    center + radius + `startAngle`/`endAngle`, and a reader echoing solved
    geometry back MUST echo whichever endpoint form the request carried
    (`start`/`end` coordinates, or `startAngle`/`endAngle`).
  - A `Circle`/`Arc`/`Ellipse` returned by the solver lane
    (`enter_sketch`/`get_sketch` return wire) carries an **optional `centerRef`**
    — the backend point-entity uuid of its center — alongside the inlined
    `center` coordinate. Producers of the return wire SHOULD emit it so re-entry
    hydration can re-own the center point (avoiding orphaned child Points);
    readers MUST treat it as optional. It is informational on the *inbound*
    solver-sync wire (the worker resolves the center from `center` coords and
    ignores `centerRef`).
- `entities[].construction` (bool, **optional**, default `false`) — reference-only
  geometry (construction/centerline). Readers MUST default an absent field to
  `false`; producers SHOULD emit it explicitly. Construction entities are
  **EXCLUDED from loop/region detection** — both `SketchRegions` (§7.4) and
  plan-time profile derivation — so flipping the flag changes the region set
  exactly as REMOVING the entity would, while the ids of the remaining regions
  are **unchanged** (a region signature derives only from its loop's edge ids,
  §7.4). Construction entities are nevertheless **still materialized in the
  solver**: they carry DOF and constraints referencing them participate in every
  solve, exactly like real geometry. A child point synthesized by a reader for an
  inline-coordinate parent (line endpoints, arc/circle centers) inherits that
  parent's flag.
- `entities[].referenceLocked` (bool, **optional**, default `false`) — geometry
  PROJECTED from the model face a sketch is attached to (`attachment.kind`
  `hostFace`). Readers MUST default an absent field to `false`; producers MUST
  emit it **only when `true`** (so a sketch with no locked geometry is
  byte-identical to one authored before the flag existed). A child point
  synthesized by a reader for an inline-coordinate parent inherits the parent's
  flag, exactly as `construction` does. A point referenced by id (`p0Ref`/
  `p1Ref`) carries its OWN flag and MUST NOT be relabelled by its parent.

  It is the deliberate OPPOSITE of `construction` on the axis that matters most:

  | flag              | bounds regions | editable | in the solver |
  |-------------------|----------------|----------|---------------|
  | `construction`    | **no**         | yes      | yes           |
  | `referenceLocked` | **yes**        | **no**   | pinned        |

  - **NOT excluded from loop/region detection.** A locked loop bounds a region
    like any other geometry — extruding a profile off the projected face boundary
    is the whole point of projecting it. Region ids are unaffected by the flag
    (a signature derives only from its loop's edge ids, §7.4), so setting or
    clearing it never remaps a region.
  - **Readers MUST refuse geometry-mutating edits** naming locked geometry:
    removal (including anything a removal would cascade onto), repositioning,
    splitting, dragging, a `construction` flip, and any partial translation of a
    set containing locked geometry (refuse the whole translation rather than
    shear the sketch). Refusal is loud and mutates nothing — a projected boundary
    that has silently drifted from its face is worse than a rejected edit.
  - **Constraints MAY reference locked geometry**, of any kind. That is how a
    profile snaps to the face boundary (`Coincident` onto a locked arc's
    `<id>.start`, a `Distance` from a free point, …). Immobility does not come
    from vetoing the constraint; it comes from the solver.
  - **Locked entities are fully pinned in the solver.** A reader MUST hold every
    parameter a locked entity owns at its current value, so a constraint spanning
    locked and free geometry can only ever move the FREE side. The pins are
    INTERNAL — same reserved tag `0` contract as the arc rules above: never
    surfaced in `conflicting[]`, never reported as redundant, and counted in the
    §7.4 naive-dof fallback (which must subtract one equation per pinned scalar,
    two per pinned point). The reference implementation pins the entity's own
    parameterization — a `Line`'s two endpoint points; a `Circle`'s center point
    and radius; an `Arc`'s center point, radius and start/end ANGLE (its
    `<id>.start`/`<id>.end` then follow from the arc rules; pinning those
    positions directly would add two redundant equations and become singular on a
    180° arc) — and skips any point an explicit `Fixed` constraint already holds.
    An `Ellipse` is not registered with PlaneGCS at all, so it carries no pins.
- `constraints[].type` ∈ the 18 kinds (verbatim from OneCAD-CPP
  `SketchTypes.h ConstraintType`): `Coincident`, `Horizontal`, `Vertical`,
  `Fixed`, `Midpoint`, `OnCurve`, `Parallel`, `Perpendicular`, `Tangent`,
  `Concentric`, `Equal`, `Distance`, `HorizontalDistance`, `VerticalDistance`,
  `Angle`, `Radius`, `Diameter`, `Symmetric`.
  - **Units.** The `Angle` constraint's `value` is in **radians** on the wire
    (parity with OneCAD-CPP `AngleConstraint`/PlaneGCS and Rust
    `Constraint::Angle`), distinct from op-param angles (`angleDeg`,
    `draftAngleDeg`) which are degrees. UI layers author/display degrees and
    MUST convert at the wire boundary.

**Extrude** (`op.extrude`) — end conditions `Blind` / `ThroughAll` / `Symmetric`
/ `ToNext` / `ToFace`, optional two directions. Field names ported from
OneCAD-CPP `ExtrudeParams`.

```json
// inputs: [] — the profile does NOT ride in inputs[]; see "Profile binding" below
// params
{
  "sketchId": "sk_1",             // profile sketch (see "Profile binding")
  "regionId": "r_ac127d8846949…", // omitted ⇒ first-region fallback
  "distance": 25.0,
  "draftAngleDeg": 0.0,
  "extrudeMode": "Blind",         // Blind | ThroughAll | Symmetric | ToNext | ToFace
  "booleanMode": "NewBody",       // NewBody | Add | Cut | Intersect
  "targetBodyId": "",             // for Add/Cut/Intersect
  "twoDirections": false,
  "extrudeMode2": "Blind",        // direction-2 end condition (when twoDirections)
  "distance2": 0.0
  // For ToFace, add "targetFace" (direction 1) and/or "targetFace2"
  // (direction 2) — a **semantic reference** object (same {primary, intent,
  // anchor} shape as the "Semantic reference" above and the fillet edge refs):
  // "targetFace": {
  //   "primary": { "bodyId": "body_1", "elementId": "el_…", "kind": "face" },
  //   "intent":  { "version": 1, "kind": "face", "descriptor": { /* §10 */ } },
  //   "anchor":  { "worldPoint": [12.0,3.5,0.0], "surfaceUv": [0.25,0.75] }
  // }
}
```

- `targetFace`/`targetFace2` are **typed semantic refs**, not bare ids
  (amended 2026-07-16 — see [Changelog](#14-changelog)). A bare `targetFaceId`
  string could carry no anchor/intent, so a ToFace target would be
  **un-repairable** across parametric edits, violating Invariants 2/3; the typed
  ref lets the resolution ladder rebind it. Absent for non-`ToFace` extrudes.

**Profile binding (NORMATIVE, Extrude / Revolve / Sweep).** The sketch profile is
carried as **flat `params.sketchId` + `params.regionId`**, NOT as a semantic ref
in `inputs[]`. A region is identified by the derived `regionId` (§7.4), which is
already a stable, content-addressed identity — it needs no anchor/intent evidence
and no ladder, so the semantic-ref machinery does not apply to it. Rust's core
`ExtrudeParams`/`RevolveParams` hold a typed `profile { sketchId, regionId }`
object; the wire layer FLATTENS it (`src-tauri/src/worker/wire.rs`
`lift_profile_to_params`) and the worker reads the flat keys
(`worker/src/ops/OpCommon.cpp` `build_profile_face`). Producers MUST send the
flat form; a nested `params.profile` is not consumed by the worker.

`inputs[]` still carries genuine semantic refs for elements that DO need the
ladder — the Extrude `ToFace` target face, fillet/chamfer edges, shell open
faces. For a plain `Blind` extrude `inputs[]` is empty; Revolve's `inputs[]` is
always empty (its axis rides in `params.axis`).

Corrected 2026-07-26 — see [Changelog](#14-changelog). The earlier prose here
described a `SketchRegion` semantic ref in `inputs[]` that no layer has ever
produced or consumed.

**Revolve** (`op.revolve`) — field names from OneCAD-CPP `RevolveParams`.

```json
// inputs: [] — profile is flat params.sketchId/regionId (see "Profile binding")
// params
{
  "sketchId": "sk_1",
  "regionId": "r_ac127d8846949…",
  "angleDeg": 360.0,
  "axis": { "kind": "sketchLine", "sketchId": "sk_1", "lineId": "e1" },
              // axis.kind ∈ "sketchLine" {sketchId,lineId} | "edge" {bodyId,edgeId} | "none"
  "booleanMode": "NewBody",       // NewBody | Add | Cut | Intersect
  "targetBodyId": ""
}
```

**Fillet** (`op.fillet`) and **Chamfer** (`op.chamfer`) — split ops sharing the
OneCAD-CPP `FilletChamferParams` shape (`mode` distinguishes; radius doubles as
chamfer distance).

```json
// Fillet params
{ "mode": "Fillet", "radius": 2.0, "edgeIds": ["e:14", "e:15"], "chainTangentEdges": true }
// Chamfer params
{ "mode": "Chamfer", "radius": 1.0, "edgeIds": ["e:14"], "chainTangentEdges": true }
```

`edgeIds` entries are TopoKeys (snapshot-scoped) or `ElementId`s; the worker
resolves each through the ladder ([§10](#10-resolution-ladder)). The `inputs[]`
array carries the corresponding semantic refs (one per edge) supplying descriptor
+ anchor evidence.

**Boolean** (`op.boolean`) — standalone body-body boolean. Field names from
OneCAD-CPP `BooleanParams` (`operation` ∈ Union/Cut/Intersect; distinct from the
`booleanMode` fused into feature ops).

```json
// inputs: [ semanticRef(target body), semanticRef(tool body) ]
// params
{ "operation": "Union", "targetBodyId": "body_1", "toolBodyId": "body_2" }
```

`operation` ∈ `Union` | `Cut` | `Intersect`.

**Shell** (`op.shell`) — hollow a body, removing (opening) selected faces. Field
names from OneCAD-CPP `ShellParams`. Added M6a (see the [Changelog](#14-changelog)).

```json
// inputs: [ semanticRef(face) per open face — kind "face" ]
// params
{ "thickness": 2.0, "targetBodyId": "body_1", "openFaces": ["el_…7c", "el_…8d"] }
```

- `thickness` is the (positive) wall thickness; the worker offsets **inward**
  (`BRepOffsetAPI_MakeThickSolid::MakeThickSolidByJoin(target, removed,
  −thickness, …)`, OneCAD-CPP parity). `thickness < 1e-3` ⇒ recoverable
  `OP_FAILED` ("Shell thickness too small").
- `openFaces` entries are `ElementId`s (bare). **Unlike Fillet/Chamfer edges, the
  frozen `ShellParams` carries no per-face typed ref**, so the `inputs[]` face refs
  are **element-only** (no `intent`/`anchor`). The worker resolves each on the
  predecessor snapshot via the partition-tracked binding (an id already minted by
  an earlier op / this plan's `resolve_input_refs`) OR the descriptor+anchor
  ladder ([§10](#10-resolution-ladder)); a face that resolves via neither ⇒
  **NeedsRepair** ([§9](#9-needsrepair-payload)), never a wrong bind. The result
  **replaces** the shelled body (id preserved; OCCT history folds into its
  partition).

**LinearPattern** (`op.linearPattern`) — `count` copies of a source body translated
`spacing` along `direction`. Field names from OneCAD-CPP `LinearPatternParams`
(the C++ flat `dirX/Y/Z` is a single `direction: [x,y,z]`). Added M6a.

```json
// inputs: [ semanticRef(source body) ]
// params
{ "sourceBodyId": "body_1", "direction": [1,0,0], "spacing": 40.0, "count": 3, "fuseResult": true }
```

- `count ≥ 2` (else recoverable `OP_FAILED`); `|spacing| ≥ 1e-9`; `direction`
  non-zero (normalized). Instance `i ∈ [1, count)` is translated `direction·spacing·i`.
- `fuseResult` (default `true`): `true` ⇒ source + instances FUSED into one solid;
  `false` ⇒ gathered into one compound. **Either way the op produces ONE new body
  `body_<opId>`** (NewBody lineage — the source body is preserved). The result
  INCLUDES the source geometry (OneCAD-CPP parity). Empty `elementMapDelta`
  (ID-on-demand; a pattern face is minted when first referenced).

**CircularPattern** (`op.circularPattern`) — `count` copies rotated about an axis.
Field names from OneCAD-CPP `CircularPatternParams` (flat `axisX/Y/Z` +
`axisDirX/Y/Z` → `axisOrigin` + `axisDirection`). Added M6a.

```json
// inputs: [ semanticRef(source body) ]
// params
{ "sourceBodyId": "body_1", "axisOrigin": [0,0,0], "axisDirection": [0,0,1],
  "angleDeg": 360.0, "count": 3, "fuseResult": true }
```

- `count ≥ 2`; `axisDirection` non-zero. The per-instance step angle is
  `angleDeg / count` (OneCAD-CPP parity — divides by `count`, **not** `count−1`);
  instance `i ∈ [1, count)` is rotated `step·i` about `(axisOrigin, axisDirection)`.
- `fuseResult` + lineage identical to LinearPattern.

**MirrorBody** (`op.mirrorBody`) — reflect a source body across a plane. Field names
from OneCAD-CPP `MirrorBodyParams` (flat `planePointX/Y/Z` + `planeNormalX/Y/Z` →
`planePoint` + `planeNormal`). Added M6a.

```json
// inputs: [ semanticRef(source body) ]
// params
{ "sourceBodyId": "body_1", "planePoint": [0,0,0], "planeNormal": [1,0,0], "fuseWithOriginal": false }
```

- The mirror plane passes through `planePoint` perpendicular to `planeNormal`
  (`gp_Trsf::SetMirror(gp_Ax2(planePoint, planeNormal))`).
- `fuseWithOriginal` (default `false`): `true` ⇒ source + mirror image FUSED into
  one solid; `false` ⇒ the mirror image alone. Either way ONE new body

**ImportStep** (`op.importStep`) — materialize the solids of a STEP file as
document bodies, as a **plan step** (NOT a session verb: an import must live on
the timeline so full-replay regen reproduces it, be fenced like every op, and
advance `historyPrefixHash`). Added 2026-08-02 (WP-A).

```json
// inputs: [] — an import depends on nothing
// params (hashed — no bytes, no paths)
{ "sourceSha256": "ab12…64 hex chars…", "sourceCodec": "step",
  "sourceName": "bracket.step", "healPolicy": "v1", "unitScale": 1.0,
  "brepFormat": null, "provenanceSha256": null }
// wire-only, NON-hashed, injected by Rust at lowering time (§7.8 temp-path rule):
{ "path": "/tmp/onecad/import_ab12.step" }
```

- `sourceSha256` — content address of the authoritative source bytes, stored in
  the document container's `imports/` section; Rust materializes them to a temp
  `path` for the worker. The hash covers the params, so a re-import (new file
  version) is an ordinary `updateOperationParams` that dirties the step.
- `sourceCodec` ∈ `"step"` | `"brep"` — which byte form the worker replays.
  `step` runs the full pinned reader pipeline below; `brep` deserializes
  BinTools bytes previously produced by that pipeline (`brepFormat` pins the
  BinTools format version; REQUIRED iff codec is `brep`).
- `provenanceSha256` (optional, skip-none) — when `sourceCodec` is a CONVERTED
  replay form (brep-primary policy), the sha of the user's ORIGINAL bytes,
  co-stored in `imports/` for re-export / future re-heal. Referenced here so the
  save-time refcount pins the provenance blob; absent when the replayed blob IS
  the original. MUST differ from `sourceSha256`.
- `healPolicy` (`"v1"`) versions the fixed, unconditional pipeline: pinned
  `Interface_Static` knobs (`xstep.cascade.unit=MM`, `read.precision.mode=0`,
  `read.step.product.mode=1`; all knobs saved + restored around the read) →
  `STEPControl_Reader` → `TransferRoots` → sew → `ShapeFix_Shape` → solid
  promotion → deterministic solid ordering. The transform is a pure function of
  the source bytes, so replay is stable; a future `v2` never silently re-heals
  existing documents.
- `unitScale` (default `1.0`) — explicit post-transfer uniform scale escape
  hatch for files with missing/ambiguous `length_unit` (the reader itself always
  converts to mm; a conversion is reported as a `STEP_UNIT_CONVERTED`
  diagnostic).
- **Body minting**: N resulting solids mint ordered `created` children under the
  §2/§7.2 rules — `body_<opId>:<k>` for N > 1, plain `body_<opId>` for exactly
  one — with **no** `deleted` parent (creation ex nihilo). Bodies SHOULD be
  named from STEP product names where recoverable (delivered via the step's
  diagnostics/metadata, not the id).
- **Failure is recoverable**: a malformed/unreadable file is `OP_FAILED` on the
  step (publish ≤ m−1, Invariant 6), NEVER `PROTOCOL_ERROR` — a user data
  problem must not tear down the worker. Diagnostics vocabulary: `STEP_SEWN`,
  `STEP_HEALED`, `STEP_UNIT_CONVERTED`, `STEP_NO_SOLIDS`, `STEP_ROOT_SKIPPED`,
  `STEP_INVALID_SHAPE`, `STEP_DUPLICATE_SOLIDS`.
  `body_<opId>` (NewBody lineage; source preserved). Empty `elementMapDelta`.

### 7.4 Sketch solver lane

A **separate worker thread/actor** runs PlaneGCS. It follows a **latest-wins**
mailbox: drags never queue behind OCCT ops (migration plan — solver lane in V1).
Requests here are ordinary `req` frames; the worker routes them to the solver
thread by verb.

#### SketchUpsert
Upserts the authoritative sketch (plane + entities + constraints). Increments
`sketchRevision`.

```json
// req.args  (entities/constraints as in the Sketch op params, §7.3)
{ "sketchId": "sk_1", "plane": { "kind": "XY", "...": "..." },
  "entities": [ … ], "constraints": [ … ] }
// result
{ "sketchId": "sk_1", "sketchRevision": 4, "dof": 2,
  "state": "UnderConstrained",    // state ∈ UnderConstrained|FullyConstrained|OverConstrained|Conflicting
  "conflicting": [] }             // constraint ids in conflict (non-empty iff state=Conflicting); absent ⇒ []
```

`conflicting` lists the constraint ids PlaneGCS reports as mutually unsatisfiable
(the same id set `SolveDrag`/`EndGesture` emit); it is non-empty exactly when
`state == "Conflicting"` and empty otherwise. It is **optional/additive** — an
absent field parses as `[]` (all parsers tolerate the missing/unknown key).

The `state` is computed after a full solve, by descending priority:
**`Conflicting`** (PlaneGCS reports genuinely conflicting constraints — no
solution) → **`OverConstrained`** (solvable but PlaneGCS reports one or more
*benign, DOF-preserving redundant* constraints, e.g. a duplicate dimension;
`!conflicting`) → **`FullyConstrained`** (`dof == 0`, no redundancy) →
**`UnderConstrained`**. `OverConstrained` deliberately outranks
`FullyConstrained`: a redundant constraint removes no DOF, so a fully-determined
sketch (dof 0) carrying one still reports `OverConstrained` — a *warning* the
frontend/dto and the dimension tool treat as a reject signal, never a hard error
(a solution exists).

**Documented deviation — an `Ellipse` forces NAIVE dof counting.** An ellipse is
materialized like any other entity but is **not registered with the constraint
solver** (parity with OneCAD-CPP, whose solver has no ellipse binding). A sketch
containing at least one ellipse therefore reports `dof` from a static count —
Σ entity DOF (`Point` 2, `Line` 0, `Arc` 3, `Circle` 1, `Ellipse` 3, each
inline-minted center Point a further 2) − Σ constraint arity — instead of a
PlaneGCS diagnosis. An `Arc`'s two inline-minted ENDPOINT points (§7.3) each
contribute 2 the same way, but the internal arc rules that couple them live
outside `constraints[]` and so subtract nothing: a reader counting naively MUST
therefore subtract a further **4 per endpoint-bearing arc**, or the fallback
reports 4 phantom degrees of freedom per arc. (The PlaneGCS path needs no such
fix-up — there the 4 parameters and 4 equations cancel in the diagnosis.)
Consequences, all normative:
* **Redundancy is unreported.** `OverConstrained` is a PlaneGCS notion; on the
  naive path the state can only be `Conflicting` (never, since no solve
  diagnoses), `FullyConstrained` (naive count == 0) or `UnderConstrained`. The
  SAME redundant constraint pair that reports `OverConstrained` without an
  ellipse reports nothing with one.
* **A redundant constraint still subtracts**, so the reported `dof` can read
  lower than the true remaining freedom (it is `max(count, 0)`).
* **`OverConstrained` from a naive count** means `count < 0` only — i.e. more
  constraint arity than entity DOF.
No constraint may reference an ellipse *entity*: every curve-taking kind
(`Radius`, `Diameter`, `Concentric`, `Tangent`, `Equal`, `OnCurve`) accepts only
lines/arcs/circles, so an ellipse operand is an unsupported-constraint failure
and the naive count never has to model one. A constraint on the ellipse's center
**Point** (e.g. `Fixed`, `Coincident`) is ordinary and does subtract. Lifting the
ellipse into PlaneGCS is deferred past V1.

#### BeginGesture
Opens a drag gesture against a specific sketch revision.

```json
// req.args
{ "sketchId": "sk_1", "sketchRevision": 4, "gestureId": 51, "solverPolicyHash": "3e9a…" }
// result
{ "gestureId": 51, "ready": true }
```

#### SolveDrag
Latest-wins incremental solve. Superseded in-flight drags may be dropped; only the
newest `seq` per gesture must resolve.

```json
// req.args
{ "gestureId": 51, "seq": 129, "pointId": "e3.start", "target": [42.0, 19.5] }
// result
{
  "gestureId": 51, "seq": 129,
  "status": "success",       // success | partial | conflicting | redundant
  "dof": 1,
  "conflicting": [],         // constraint ids in conflict (when status=conflicting)
  "positions": { "e3.start": [42.0, 19.5], "e2.p1": [40.0, 19.5] },  // CHANGED points only
  "solveMicros": 1840
}
```

`status` by descending priority: **`conflicting`** (the drag solve or the
gesture's committed sketch reports conflicting constraints) → **`redundant`**
(the solve succeeded and the committed sketch carries benign redundant
constraints, as diagnosed at `BeginGesture`) → **`success`** (converged, no
redundancy) → **`partial`** (did not converge). Redundancy is a fixed property
of the committed sketch for the whole gesture; it is NOT re-derived per drag
step (a drag pins the non-dragged points, which would spuriously flag the
committed constraints as redundant). `EndGesture` uses the same precedence.

#### EndGesture
Pointer-up: performs the final **exact** solve (Rust commits one undo command from
its result).

```json
// req.args
{ "gestureId": 51 }
// result
{ "gestureId": 51, "status": "success", "dof": 0,
  "conflicting": [],   // constraint ids in conflict (non-empty iff status=conflicting); absent ⇒ []
  "positions": { /* final exact positions, changed since BeginGesture */ },
  "sketchRevision": 5 }
```

`conflicting` follows the same precedence as `SolveDrag`: the final exact solve's
conflicts, else the gesture-fixed set diagnosed at `BeginGesture`. Optional/additive
(absent ⇒ `[]`).

#### SketchRegions
Computes closed profile regions for a sketch (for extrude/revolve selection and
preview fill).

```json
// req.args
{ "sketchId": "sk_1" }
// result
{
  "sketchId": "sk_1", "sketchRevision": 5,
  "regions": [
    {
      "regionId": "r0",
      "outerLoop": ["e1", "e2", "e3"],
      "holes": [ ["e4"] ],
      "previewTriangles": { "format": "f32xyz+u32idx", "vertexCount": 8,
        "triangleCount": 8, "holesSubtracted": 1, "bin": "region:r0" }
    }
  ]
}
// bin: [ { "name": "region:r0", "off": 0, "len": … } ]  // f32 positions then u32 indices
```

- **`regionId` derivation is NORMATIVE and cell-complete.** A region is one
  bounded planar **cell**, not merely one detector face: every closed loop is an
  independently selectable outer boundary and its immediate contained children
  are that cell's holes. A rectangle containing a circle therefore publishes
  both the rectangle-minus-circle cell and the circle-disc cell.

  A hole-free, unsplit cell retains the original byte-compatible algorithm:
  **FNV-1a-64** (offset `0xcbf29ce484222325`, prime `0x100000001b3`) over each
  outer-loop entity UUID as its 16 raw bytes in ascending order, then winding
  byte `0`, rendered `"r_%016x"`.

  A cell with holes or an intersected source entity hashes one canonical UTF-8
  member string, then winding byte `0`, through the same FNV/rendering rule. The
  string is `cell-v2|outer{L}|holes{H...}`:

  - `L` is the lexicographically smallest cyclic rotation of the oriented loop's
    length-prefixed tokens (outer normalized CCW; holes CW).
  - A token is the mapped base wire UUID, followed by `#segN_pM` when that base
    entity was subdivided by an intersection, then `:f` or `:r` for traversal.
  - Each hole loop is canonicalized independently; the resulting hole strings
    are sorted and length-prefixed before concatenation.

  The worker MUST reject duplicate canonical ids instead of publishing ambiguous
  regions or duplicate binary section names. An older outer-only id may resolve
  only when it matches exactly one current cell; zero matches is stale and
  multiple matches is ambiguous/`NeedsRepair`. Neither case may choose the first
  cell. The Rust `derive_region_id` reference remains byte-authoritative for the
  unchanged hole-free/unsplit form; the worker `RegionTable` owns the extended
  cell form.

- **Construction geometry never bounds a region.** Entities carrying
  `construction: true` (§7.3) are dropped before any loop is formed, so they
  appear in no `outerLoop`/`holes`, publish no cell of their own, and perturb no
  other cell's `regionId`. The same exclusion applies to plan-time profile
  derivation, so a region selectable here is exactly a region an op can extrude.
  **`construction` is the ONLY entity flag with this effect** — in particular
  `referenceLocked: true` (§7.3) geometry participates in loop detection exactly
  like free geometry.

- **`previewTriangles` SUBTRACTS the region's holes** (changed 2026-07-26 — see
  [Changelog](#14-changelog)). The fill MUST cover exactly the material the
  kernel builds a face from, because it is the geometry consumers use both to
  draw the extrude/revolve preview and to hit-test a region: filling a hole made
  the preview disagree with the committed solid AND made a click inside a hole
  select the enclosing region. `vertexCount` therefore covers the outer loop's
  vertices followed by each subtracted hole's vertices.

  **Triangulation topology is normative for consumers that recover rings.** Any
  internal bridging a producer uses to merge holes into one loop MUST leave every
  bridge segment shared by exactly two triangles, so the only edges used by
  exactly ONE triangle are the real outer and hole boundaries. Consumers derive
  the extrusion rings from that property (`prismPreview.profileFromRegion`); a
  bridge that read as a boundary edge would fabricate a wall. The reference
  implementation is `worker/src/loop/PolygonFill.cpp`.

- `holesSubtracted` (additive, optional for compatibility) reports how many
  holes the fill removed. Producers MUST NOT publish a region when the value
  would be below `holes.length`; required-hole triangulation/build failure is an
  operation failure. Consumers that receive the field MUST reject an incomplete
  fill rather than preview or hit-test different material.

  **A producer MUST fail closed on a partial triangulation.** An ear-clip (or
  equivalent) pass that stalls on a degenerate loop and cannot consume the whole
  merged boundary MUST fail the `SketchRegions` request rather than publish the
  partial triangle list — a partial fill reads as a wrong boundary to the
  ring-recovery consumers above.

- **Known V1 limitation — intersection-fragment boundaries are chords.** Cells
  produced by curve-curve intersection splitting (fragment edges, e.g. the lens
  and crescents of two overlapping circles) currently build their committed
  faces from the planarized polygon (chord approximation of arcs), not trimmed
  analytic curves; the polygon fill visually agrees with the committed solid.
  Plain and nested (hole-bearing) cells are exact. **This applies to `Ellipse`
  fragment cells identically**: a PURE (unsplit) ellipse loop builds an exact
  `Geom_Ellipse` edge — an extrude of one is analytic, with a single lateral
  face and volume π·`majorR`·`minorR`·distance — but an ellipse cut by another
  curve contributes chord fragments like any arc. `previewTriangles` is a
  tessellation in **both** cases (region area/fill for an ellipse is a sampled
  polygon and therefore slightly under-reports the analytic π·a·b). Lifting
  fragments to analytic trimmed wires is tracked in `TODO.md` backlog.

### 7.5 Element identity

#### AcquireElementIds
Promotes snapshot-scoped TopoKeys to persistent, globally-unique `ElementId`s
(**ID-on-demand**). ElementIds do **not** embed `BodyId`.

```json
// req.args
{ "snapshotId": 5012, "bodyId": "body_3",
  "picks": [ { "topoKey": "f:22", "anchor": { "worldPoint": [1,2,3], "surfaceUv": [0.5,0.5] } } ] }
// result
{ "ids": [ { "topoKey": "f:22", "elementId": "el_00000000000004a1", "kind": "face" } ] }
```

Note: `elementId` is **minted by Rust**, not the worker — the worker returns the
resolved `topoKey → (kind, descriptor, anchor)` binding and Rust assigns/echoes
the persistent id it owns. When Rust already holds an id for that stable element,
the worker's response includes the existing binding so Rust returns the same id
(Invariant 1: an ElementId never changes because geometry changed).

#### QueryElement
Looks up an element's current binding within a snapshot (no mutation).

```json
// req.args
{ "snapshotId": 5012, "elementId": "el_…4a1" }   // or { "snapshotId", "topoKey", "bodyId" }
// result
{ "elementId": "el_…4a1", "topoKey": "f:22", "bodyId": "body_3", "kind": "face",
  "descriptor": { … }, "anchor": { … }, "present": true }
```

#### ResolveRefs
**Dry-run** ladder execution for repair dialogs — returns full evidence per ref
without binding anything.

```json
// req.args
{ "snapshotId": 5012,
  "refs": [ { "refId": "op_5.input0", "primary": {…}, "intent": {…}, "anchor": {…} } ] }
// result
{ "resolutions": [
    { "refId": "op_5.input0", "outcome": "autoBind",   "elementId": "el_…", "score": 0.94, "margin": 0.31 },
    { "refId": "op_5.input1", "outcome": "needsRepair", "needsRepair": { /* §9 */ } }
] }
```

`outcome` ∈ `autoBind` | `needsRepair` | `unchanged`.

### 7.6 Geometry

#### PreviewOp
**Drag-time preview.** Runs ONE candidate op against a **throwaway copy** of the
session head and returns the resulting bodies' MESH1. Nothing is committed.

```json
// req.args
{ "op": { "opType": "Extrude", "opId": "preview_cut", "inputs": [ … ],
          "params": { "sketchId": "sk_1", "regionId": "r_…", … } },
  "sketchId": "sk_1",          // optional: seed this profile sketch (see below)
  "expectedSnapshotId": 5012,  // optional stale-head guard
  "lod": "coarse" }
// result
{ "snapshotId": 5012,          // the HEAD's id — a preview creates no snapshot
  "bodyEvents": [ { "kind": "modified", "bodyId": "body_3" } ],
  "changedBodies": ["body_3"],
  "deletedBodies": [],
  "needsRepair": [],
  "meshes": [ { "bodyId": "body_3", "bin": "mesh:body_3", … } ] }   // §7.6 handles
```

- `op` is the same **canonical worker operation** used inside `ExecutePlan`:
  profile binding is flat `params.sketchId`/`params.regionId`, body-bearing
  fields use worker `body_<uuid>` form, and inputs have already passed the shared
  Rust lowering. Preview callers MUST NOT maintain a second ad-hoc mapper.
- When supplied, `expectedSnapshotId` MUST equal the current head snapshot or the
  preview fails recoverably with `error.code:"STALE_PREVIEW"` ([§8](#8-error-taxonomy)).
  The code is normative — consumers MUST route on it, never on the message text.
  This prevents an old drag response from rendering against a newer document head.
- **A preview is INVISIBLE to fencing.** It MUST NOT fence, prepare, accept or
  discard: the head bodies, element-map partition, `historyPrefixHash`,
  `snapshotId`, `documentRevision` and `workerEpoch` are all unchanged
  afterwards, and no scratch is left behind. Implementations take the
  fencing-free head copy (the same one the §7.5 identity verbs use), **not** the
  `ExecutePlan` fence-and-clone path — that reserves a prepared snapshot id.
- `snapshotId` echoes the CURRENT head. A preview has no snapshot of its own, and
  reporting a fresh id would name something no other verb knows.
- **Only the bodies the op created or modified** are tessellated and returned.
  `bodyEvents` carries the full candidate lifecycle; `deletedBodies` names
  deletions that cannot have a mesh. A drag re-issues this per frame, so
  re-shipping untouched bodies is not allowed.
- `sketchId` seeds the profile sketch from the committed sketch store. A real
  plan materializes its profile from its own preceding `Sketch` op; a preview has
  no plan, so the caller names it. Absent/unknown ⇒ `REF_UNRESOLVED`.
- Candidate execution uses the same input-resolution, operation,
  `NeedsRepair`, cancellation and rollback routine as an `ExecutePlan` step.
  `NeedsRepair` returns an otherwise-empty successful result with evidence;
  other failures are normal errors (`OP_FAILED` / `UNSUPPORTED` /
  `GEOMETRY_INVALID`), never partial mutation. Callers may retain the last good
  mesh for a transient geometric miss, but structural binding failures must be
  surfaced and must disable commit.
- **Lane.** This is kernel-lane work; it shares the OCCT single-writer thread
  with `ExecutePlan`. It deliberately does NOT ride the solver lane, whose
  latest-wins coalescing is specific to `SolveDrag`. Callers are expected to bound
  their own in-flight previews (the reference client keeps ≤1 per preview session
  and discards stale epochs).

#### Tessellate
Produces MESH1 meshes; large meshes stream on the bulk lane
([§5.2](#52-chunked-bulk-streams)). `mesh_format.md` defines MESH1.

```json
// req.args
{ "bodyIds": "all", "lod": "coarse", "includeEdges": true }
       // bodyIds: "all" | ["body_1","body_3"];  lod: "coarse"|"medium"|"fine"
// result
{ "meshes": [
    { "bodyId": "body_1", "streamId": 700, "format": "MESH1",
      "totalBytes": 4194304, "sha256": "…", "snapshotId": 5012 }
] }
```

Meshes label faces/edges with snapshot-scoped TopoKeys (`"f:22"`) and persistent
`ElementId`s where already minted. Meshing parallelism never affects IDs
(Invariant 5).

#### GetBodies
Returns BREP blobs (OCCT `BinTools`) for the given bodies; streams on bulk lane.

```json
// req.args
{ "bodyIds": ["body_1"], "snapshotId": 5012 }
// result
{ "bodies": [ { "bodyId": "body_1", "streamId": 701, "format": "BREP",
  "brepContentHash": "…", "totalBytes": 91234, "sha256": "…" } ] }
```

#### LoadBodies
Loads BREP blobs into the session (input via request `bin`/stream).

```json
// req.args
{ "bodies": [ { "bodyId": "body_1", "bin": "brep:body_1", "brepContentHash": "…" } ] }
// bin: [ { "name": "brep:body_1", "off": 0, "len": 91234 } ]
// result
{ "loaded": ["body_1"], "snapshotId": 5014 }
```

#### ProjectFaceBoundary
**Read-only kernel query.** Resolves a picked planar face within the current head
and returns its exact plane frame plus its boundary — and, in `coplanarBody`
scope, the boundary of every OTHER face of the same body coplanar with the
supplied plane — as 2D points and Line/Circle/Arc entities in that plane's UV.

It does **not** fence, prepare, accept, discard, or mint. Like the
[§7.5](#75-element-identity) identity verbs it reads a copy of the head, so a
stale or absent reference is `present:false` — an **answer, not an error**.

```json
// req.args
{ "snapshotId": 5012,
  "bodyId": "body_3", "topoKey": "f:22",   // or { "elementId": "el_…4a1" }
  "frameOnly": false,
  "plane": { "origin": [0,0,30], "xAxis": [1,0,0], "yAxis": [0,1,0], "normal": [0,0,1] },
  "scope": "coplanarBody",                 // "faceOnly" | "coplanarBody" (default)
  "options": { "pointMergeTolerance": 1e-5, "normalDotTolerance": 0.9999,
               "planeDistanceTolerance": 1e-3, "fallbackSegmentsPerCurve": 24 } }
// result
{ "present": true,
  "exact": { "origin": [0,0,30], "normal": [0,0,1] },
  "hasClosedBoundary": true,
  "faceCount": 2,
  "points": [ { "ref": "p0", "at": [0,0] }, { "ref": "p1", "at": [80,0] } ],
  "entities": [
    { "type": "Line",   "p0Ref": "p0", "p1Ref": "p1" },
    { "type": "Circle", "centerRef": "p4", "radius": 10 },
    { "type": "Arc",    "centerRef": "p5", "radius": 10,
      "startAngle": 0, "endAngle": 1.5707963267948966, "ccw": true }
  ] }
```

**Addressing.** `elementId`, else `{bodyId, topoKey}`. The `elementId` form carries
one rung more than [`QueryElement`](#queryelement): partition entry →
`(bodyId, topoKey)` → the body's sub-shape. `QueryElement`'s `elementId` branch
answers from the partition descriptor and never reaches a shape; this verb MUST
resolve all the way to a real face. A miss anywhere on that chain — unknown
`elementId`, missing body, stale `topoKey` — is `{ "present": false }` with
`ok:true`. A reference that resolves to a non-face element is `OP_FAILED`.
`snapshotId` is **advisory** here, exactly as in [§7.5](#75-element-identity):
the query always answers against the current head, and a `topoKey` minted under an
older snapshot simply fails to resolve (`present:false`). It is NOT a fence —
callers needing a stale-head guard must compare the head themselves.

**`frameOnly`.** When `true`, `plane` and `scope` are IGNORED and the result is
`present` + `exact` only. This is the first half of the plane handshake: the
caller has no basis yet, takes the kernel-exact frame, builds one, and sends it
back on a second call.

**`exact` is ALWAYS returned when `present`** (both modes): the kernel `gp_Pln`
origin and the **orientation-corrected unit normal** of the **seed** face
(reversed for a `TopAbs_REVERSED` face, so it points out of the solid). It lies ON
the face plane — a descriptor `center` does not, being an axis-aligned bbox
centre, which for a tilted face sits off-plane and would extrude a sliver.
Outside `frameOnly` it is an echo the caller SHOULD compare against the plane it
supplied (a tripwire, not a fence).

**The `plane` argument is authoritative.** Every `at` and every arc/circle centre
is expressed in ITS UV; the worker MUST NOT substitute a basis of its own, even
when its own frame looks better. `plane` is REQUIRED unless `frameOnly:true`;
absent, it is `PROTOCOL_ERROR`. Units are **mm**; angles are **radians, CCW from
the plane's +X (U) axis**.

**Point refs are response-local.** `p<N>` is 0-based and indexes `points[]` of
THIS response only. They are not `ElementId`s, carry no persistence, and are not
comparable across responses. Every `p0Ref`/`p1Ref`/`centerRef` MUST resolve within
the same response's `points[]`. Points within `pointMergeTolerance` of each other
merge onto one entry, so **a ref reused across entities IS the same point** — that
is how adjacent boundary curves share an endpoint.

**Exactness.** A `GeomAbs_Line` edge stays a Line; a full circular edge stays a
Circle; a circular arc stays an Arc. An Arc's `startAngle`/`endAngle` are always
the CCW-ordered pair (sweeping from `startAngle` counter-clockwise reaches
`endAngle`); `ccw` reports the direction of the UNDERLYING kernel curve. Every
other curve type (B-spline, ellipse, …) falls back to a **Line polyline** of
`fallbackSegmentsPerCurve` segments. That fallback is **lossy** and permanent —
the emitted geometry no longer follows the true curve, and nothing downstream can
recover it.

**A non-planar SEED face is refused** with a recoverable `OP_FAILED`, in both
modes. It is never approximated.

**`scope: "coplanarBody"`** adds the edges of every OTHER face of the SAME body
coplanar with the **supplied** plane (tested against that plane, not the seed
face): `|n·n'| ≥ normalDotTolerance` and `|n·(p'−p)| ≤ planeDistanceTolerance` mm,
over ALL faces of the body — **not** an edge-adjacency walk, so DISCONNECTED
coplanar faces are included. Edges are deduped by `IsSame`, so a `TopoDS_Edge`
shared by two coplanar faces is emitted **ONCE**; an interior edge that survives
that way is intentional — it splits the projected profile into two regions, which
is faithful to the body's topology. `faceCount` reports how many faces were
walked (always `1` for `faceOnly`).

**`hasClosedBoundary`** is true when some wire closed: either it contributed a
full Circle, or it emitted ≥3 curves AND (the wire carries OCCT's closed flag OR
the walk's first and last points merged onto the same entry).

**Determinism.** For the same inputs against the same snapshot the response is
byte-identical across fresh worker processes. The emission order is normative:

1. the **seed** face first, then the remaining coplanar faces in
   `TopExp_Explorer(shape, TopAbs_FACE)` order;
2. within a face, its **outer** wire first, then its holes in
   `TopExp_Explorer(face, TopAbs_WIRE)` order;
3. within a wire, edges in `BRepTools_WireExplorer` order;
4. `entities[]` in that walk order, and `points[]` numbered by **first use**.

**Lane.** Kernel lane, alongside `ExecutePlan`/`PreviewOp`. It takes no locks
beyond the brief head copy, so it does not block an in-flight regen.

### 7.7 Checkpoints

#### SaveCheckpoint
Emits an **atomic artifact set** for a step: BREP blobs (BinTools) + ElementMap
partition JSON + the 3 signatures + `historyPrefixHash`, each wrapped in a
Rust-readable envelope. Blobs stream on the bulk lane.

```json
// req.args
{ "stepIndex": 4 }
// result
{
  "checkpointId": "ckpt_9",
  "stepIndex": 4,
  "artifacts": [
    {
      "envelope": {
        "artifactSchemaVersion": 1,
        "bodyId": "body_3",
        "step": 4,
        "historyPrefixHash": "9c4d…",
        "brepContentHash": "aa11…",
        "occtFingerprint": "9a1c33f0e7b24d10",
        "descriptorVersion": 1,
        "resolverVersion": 1,
        "quantizationVersion": 1,
        "signatureVersion": 1,
        "codec": "brep-bintools",
        "size": 91234,
        "contentHash": "bb22…"
      },
      "streamId": 702
    }
  ],
  "elementMapPartition": { "streamId": 703, "format": "elementmap-json", "sha256": "…" },
  "signatures": { "geometry": "…", "bodyLifecycle": "…", "referencedBinding": "…" }
}
```

Checkpoints are **disposable caches**: an envelope whose versions/fingerprint are
incompatible is discarded + replayed; a checkpoint never blocks opening the
authoritative JSON (Invariant 7).

#### RestoreCheckpoint
Restores a checkpoint as base state; verifies the envelope signature and reports
drift.

```json
// req.args
{ "checkpointId": "ckpt_9", "expectedHistoryPrefixHash": "9c4d…" }
// bin/streams: the artifact blobs Rust supplies back
// result
{ "restored": true, "snapshotId": 5015, "driftDetected": false,
  "driftDetail": null }   // when driftDetected: { signature: "geometry"|"bodyLifecycle"|"referencedBinding", expected, actual }
```

### 7.8 IO

Paths are **Rust-provided temp paths** (the webview has zero fs capability; Rust
does all IO and handles hostile files in the isolated worker).

#### InspectStep

Read-only preflight probe — parses a STEP file WITHOUT mutating any session
state (addressed like the §7.5 identity verbs: no fence, no scratch, no
publish). The actual import is the §7.3 `ImportStep` **op**, executed inside
`ExecutePlan` like every other op. (The former `ImportStep` *verb* specified
here was removed 2026-08-02 before any implementation existed — a
session-mutating import verb is invisible to full-replay regen, which would
delete its bodies on the next regen; see the changelog.)

```json
// req.args
{ "path": "/tmp/onecad/import_ab12.step", "includeGeometry": true }
// result
{ "solidCount": 2, "sourceUnit": "INCH", "bbox": { "min": [0,0,0], "max": [50.8,25.4,12.7] },
  "productNames": ["Bracket", "Pin"], "geometryCodec": "xbf", "geometryFormat": 12,
  "diagnostics": [ { "severity": "warning", "code": "STEP_HEALED", "message": "…" } ] }
// bin (only when includeGeometry): { "name": "geometry", … } — the healed
// result serialized in `geometryCodec`, solids in the §7.3 ordinal order
```

- `includeGeometry` (default `false`): when `true`, the response's binary tail
  carries the **healed, mm-normalized, UNSCALED** result serialized in the
  worker's preferred replay codec, named by `geometryCodec` with its binary
  format version in `geometryFormat` (pinned into §7.3
  `ImportStep.brepFormat`). Current codec is `"xbf"` (BinXCAF — round-trips
  shapes + XCAF product names + face colors; plain BinTools `.brep` loses the
  attributes, which is why the lane moved off it; `"brep"` remains a valid
  §7.3 `sourceCodec` for documents that carry it). Rust probes once at
  import-command time, persists the geometry bytes alongside the STEP source,
  and authors the record against them — replay never re-parses STEP.
  (2026-08-02 later same day: `includeBrep`/`brepFormat`/bin `"brep"` renamed
  to the codec-neutral forms before any release shipped — no compat shim.)
- The probe honors cancel; a malformed file is an `OP_FAILED`-class error
  response (recoverable), never `PROTOCOL_ERROR`.

#### ExportStep

```json
// req.args
{ "path": "/tmp/onecad/export_cd34.step", "bodyIds": ["body_3"], "schema": "AP214IS" }
// result
{ "written": true, "bytes": 40211 }
```

`schema` currently `"AP214IS"`.

#### ExportStl

```json
// req.args
{ "path": "/tmp/onecad/out.stl", "bodyIds": ["body_3"], "binary": true, "lod": "fine" }
// result
{ "written": true, "bytes": 120344, "triangleCount": 4012 }
```

#### ExportObj

```json
// req.args
{ "path": "/tmp/onecad/out.obj", "bodyIds": ["body_3"], "lod": "fine" }
// result
{ "written": true, "bytes": 98211 }
```

---

## 8. Error taxonomy

Errors are returned in a terminal `resp` with `ok:false` and an `error` object:

```json
{ "code": "OP_FAILED", "message": "human-readable", "detail": { … }, "retriable": false }
```

| Class | `code` | Session effect | Recovery |
|-------|--------|----------------|----------|
| Recoverable op failure | `OP_FAILED` | scratch only — **session intact** | Rust discards scratch; user edits and retries |
| Reference unresolved | `REF_UNRESOLVED` | scratch only | as above (distinct from NeedsRepair — this is a hard resolve failure, e.g. input body missing) |
| Invalid geometry produced | `GEOMETRY_INVALID` | scratch only | as above |
| Unsupported op/param (known verb) | `UNSUPPORTED` | none | Rust falls back / freezes node (the remaining un-shipped ops `opType:"Loft"` / `"Sweep"`; the M6a breadth ops Shell/LinearPattern/CircularPattern/MirrorBody are now supported, [§7.3](#73-op-payload-schemas-vertical-slice)) |
| Stale preview base (`PreviewOp` only) | `STALE_PREVIEW` | none — head untouched | caller re-previews against the fresh head snapshot ([§7.6](#76-meshes--previews)) |
| Cooperative cancellation | `CANCELLED` | in-flight job dropped; session intact | terminal frame always sent ([§3.5](#35-cancel-rust--worker)) |
| Protocol violation | `PROTOCOL_ERROR` | fatal | **restart worker** (no resync) |
| Worker crash / abnormal exit | *(no frame)* | fatal | **restart + replay** from last checkpoint/head; crash **circuit breaker** on repeated `(historyPrefixHash, opId, occtFingerprint)` |
| Timeout | *(Rust-side)* | Rust-enforced | see below |

`PROTOCOL_ERROR` covers two sub-cases:
- **Framing / envelope violation** (bad magic, over-cap length, malformed JSON,
  `NaN`/`Inf`, duplicate keys, chunk SHA-256 mismatch): the frame stream is
  unparseable — the reader tears down without resync; a terminal frame may not be
  produced.
- **Well-framed but protocol-illegal request** (**unknown verb**, mismatched
  `workerEpoch` or `expectedBaseHash` — the D4 fencing tokens; a stale
  `documentRevision` is **not** an error, [§2](#2-identifier--scalar-types)/[§7.2](#72-regen--executeplan) — malformed `args`): the frame parsed, so the worker
  replies with a terminal `resp` `ok:false` `error.code:"PROTOCOL_ERROR"`. Rust
  reconciles (`GetWorkerHead`) or restarts per severity.

**Timeouts** are enforced by **Rust**, not the worker:
- `SolveDrag`: **250 ms**. On timeout Rust drops the stale drag (latest-wins) and
  keeps the gesture; the frontend keeps its 120 Hz preview.
- `Tessellate`: **30 s**. On timeout Rust cancels the request and may retry at a
  coarser LOD.
- Hung worker: ping every **5 s**, ×2 misses → `SIGKILL` → restart.

**`OP_FAILED`, `REF_UNRESOLVED`, `GEOMETRY_INVALID`, `UNSUPPORTED`, and
`STALE_PREVIEW` are *recoverable*: the worker's active session is untouched (all
work was in scratch, or — for a preview — never touched the head at all).
Rust reports the failure and the document stays editable.**

**NeedsRepair is NOT an error.** It is per-step **state** inside `PlanPrepared`
(`perStepResults[].status = "needsRepair"`, payload in the step `event`'s
`needsRepair[]`). It is never returned in an `error` object, in any of the three
languages. A plan that hits NeedsRepair at step `m` still prepares snapshot `m−1`
and returns a successful `PlanPrepared` (`stoppedReason:"needsRepair"`).

---

## 9. NeedsRepair payload

Emitted in a `planStep` event's `needsRepair[]` and echoed by `ResolveRefs`. It is
STATE (see [§8](#8-error-taxonomy)).

```json
{
  "refId": "op_5.input0",
  "elementId": "el_…4a1",
  "ladderFailed": "descriptor",          // "history" | "descriptor"
  "reason": "ambiguous",                 // "ambiguous" | "no-candidates" | "low-confidence"
  "scoringVersion": 1,                   // = resolverVersion the scores were computed under
  "candidates": [
    {
      "topoKey": "f:31",
      "score": 0.91,                     // normalized [0,1], versioned (§10)
      "margin": 0.00,                    // score1 − score2
      "worldPos": [12.0, 3.5, 0.0],
      "summary": "planar face, area≈120mm²",
      "featureContributions": { "surfaceType": 0.2, "area": 0.25, "normal": 0.2,
                                "adjacency": 0.15, "anchor": 0.11 }
    },
    { "topoKey": "f:44", "score": 0.91, "margin": 0.00, "worldPos": [12.0,-3.5,0.0],
      "summary": "planar face, area≈120mm²", "featureContributions": { } }
  ],
  "anchor": { "worldPoint": [12.0,3.5,0.0], "surfaceUv": [0.25,0.75],
              "localFrame": { … }, "adjacencyHint": "d41d8cd9…" },
  "uiLabel": "Fillet edge on right pocket"
}
```

- `ladderFailed`: the ladder level that could not decide (`history` = OCCT history
  gave no/ambiguous mapping; `descriptor` = descriptor+anchor matching was
  ambiguous/low-confidence).
- `scoringVersion`: the `resolverVersion` (§10) the candidate scores were computed
  under. Present on every NeedsRepair evidence payload so a repair UI / a
  Rust-side policy knows which normalized-scoring scheme produced the numbers.
- `candidates[]` is sorted by `score` descending; a symmetric tie (equal scores,
  `margin` below the policy margin) MUST produce NeedsRepair, never a guess (false
  positive is worse than false negative).
- Repair is performed by **Rust** (rewrite the OperationRecord ref + re-regen);
  there is **no worker `BindRepair` verb**.

---

## 10. Resolution ladder

Worker-side, executed inside each plan step's input binding. Returns full typed
evidence so the policy can later move to Rust.

**Ladder:**

1. **OCCT history** — consult the modified/generated maps of **all** ops in the
   step's builders (not just booleans); builder objects are kept alive for the
   step. A unique history image auto-binds.
2. **Descriptor matching with anchor narrowing** — for unresolved refs, match the
   frozen `intent.descriptor` against candidate elements; narrow ambiguity using
   the `anchor` (world point, surface UV, local frame, adjacency hint).
3. **Confidence gate → NeedsRepair** — if no confident unique match, emit
   NeedsRepair ([§9](#9-needsrepair-payload)).

**Descriptor** (evidence, never identity — Invariant 2). Ported from OneCAD-CPP
`ElementMap.h`: an `ElementDescriptor` of `{shapeType, center, size (bbox
diagonal), magnitude (area/length/volume), surfaceType, curveType, normal,
tangent, hasNormal, hasTangent, adjacencyHash}`, quantized into a match key
(shape/surface/curve type + quantized center xyz + normal xyz + tangent xyz +
size + magnitude + adjacencyHash). Quantization step **`1e-6`**
(`llround(value / 1e-6)`). Hashing **FNV-1a 64-bit** (offset basis
`14695981039346656037`, prime `1099511628211`). `adjacencyHash` is FNV-1a over
sorted quantized incident-edge lengths (faces) or magnitude + vertex offsets +
count (edges). This is `quantizationVersion = 1` / `descriptorVersion = 1`.

**Scoring (REDESIGNED — normalized).** OneCAD-CPP's `score()` is an unbounded,
scale-dependent cost that cannot express the locked policy; this protocol replaces
it with a **normalized `[0,1]` versioned confidence** (`resolverVersion = 1`).
Higher = better match. Policy:

- **Auto-bind iff** `score1 ≥ 0.85` **AND** `(score1 − score2) ≥ 0.10`
  (score1/score2 = best/second-best candidate).
- Otherwise, attempt anchor narrowing; if still not confident ⇒ NeedsRepair.
- For a set of referenced elements, use **min-cost assignment** over the
  **referenced-only** candidate sets (greedy is a documented counterexample —
  never greedy).
- **Lineage semantics for split/merge are explicit: no forced 1:1.** A split may
  map one prior element to several successors; a merge, several to one. The
  assignment respects declared lineage rather than forcing bijection.
- A **symmetric tie** (e.g. `0.91` vs `0.91`, margin `< 0.10`) ⇒ NeedsRepair. A
  false positive (wrong silent bind) is strictly worse than a false negative
  (asking the user).

The worker returns, per ref: candidates, `featureContributions`, `score`,
`margin`, and the ladder level reached — full evidence for repair UI and for
moving policy to Rust later.

---

## 11. Invariants

Copied verbatim from the migration plan ("Invariants (test-enforced)"). Every verb
in this contract is defined so as not to violate them; the golden fixtures enforce
them.

1. ElementId never changes because geometry changed.
2. Descriptors are evidence never identity.
3. Every op resolves inputs on its exact predecessor snapshot (never overwrite
   stored input anchor with op's own output).
4. Published bodies/maps/signatures/meshes share one snapshot id.
5. Same plan+base+policies+fingerprint ⇒ identical lifecycle/mappings/quantized
   signatures.
6. Failure at m publishes ≤ m−1.
7. Incompatible cache degrades performance never correctness.

---

## 12. Signatures

**Three** signatures per step (counts alone cannot detect symmetric ElementId
swaps). All are 64-bit FNV-1a hex strings (`signatureVersion = 1`):

- `geometry` — over per-body counts (faces/edges/vertices), quantized bbox, and
  adjacency structure.
- `bodyLifecycle` — over the ordered body create/modify/delete/split/merge events
  of the step.
- `referencedBinding` — over the `(refId → ElementId)` bindings the step produced
  for **referenced** elements (catches symmetric swaps that leave counts intact).

They appear in `planStep` events, `SaveCheckpoint`, and `PlanPrepared` summaries,
and back Invariant 5 and checkpoint drift detection.

---

## 13. Versioning/change policy

- `protocolVersion` is `1`. A wire-incompatible change bumps it; the handshake
  negotiates and Rust refuses an unknown major.
- The independent version axes carried in the handshake and checkpoint envelopes —
  `quantizationVersion`, `descriptorVersion`, `resolverVersion`,
  `signatureVersion`, `solverPolicyVersion`, `occtFingerprint`,
  `artifactSchemaVersion` — evolve separately; a mismatch degrades caches to
  replay, never correctness (Invariant 7).
- **Any change to this file, to `mesh_format.md`, to the `Descriptor.*`
  computation, or to a serde/nlohmann schema requires a fixture bump
  (`protocol/fixtures/`) + cross-track sign-off (worker + Rust + orchestrator).**
- Golden fixtures in `protocol/fixtures/` are the executable form of this
  contract; both sides run them in CI.

---

## 14. Changelog

`protocolVersion` stays **1** for all entries below — these are pre-implementation
contract refinements (no worker has shipped against the prior text), so they are
edits to version 1 rather than a version bump. They still fall under the
[§13](#13-versioningchange-policy) change policy (fixture bump + cross-track
sign-off) once fixtures exist.

- **2026-08-02 — MESH1 `FACE_COLORS` section (type 12, flags bit 4) + `xbf`
  import replay codec** (WP-A W4.5 XCAF fidelity; cross-track sign-off recorded
  2026-08-02). Plain BinTools `.brep` bytes carry NO XCAF attributes, so the
  brep replay lane would silently drop imported face colors and product names on
  every reopen — the conversion lane therefore moves to BinXCAF (`.xbf`,
  `sourceCodec: "xbf"`, additive enum value; `brep` stays valid), which
  round-trips shapes + names + colors through replay. Face colors reach the
  frontend per-tessellation via the additive MESH1 `FACE_COLORS` section
  (`protocol/mesh_format.md` §2/§4) — never as document state (TopoKey is
  snapshot-scoped; minting thousands of ElementIds for appearance would bloat
  the ladder surface). Additive on both surfaces: unknown section types MUST be
  skipped (mesh_format §3) and existing meshes carry no new bytes — **no
  fixture bump**.

- **2026-08-02 — §7.3 NEW op `ImportStep`; §7.8 `ImportStep` verb REMOVED,
  replaced by read-only `InspectStep`** (WP-A STEP-IMPORT; cross-track sign-off
  recorded 2026-08-02). The import is a **timeline op** executed inside
  `ExecutePlan` — the never-implemented §7.8 verb shape mutated the session
  outside a plan (no fence, no `historyPrefixHash` advance, invisible to
  full-replay regen, which would delete the imported bodies on the next regen)
  and could not stand. Op params are content-addressed (`sourceSha256` into the
  container's `imports/` section) + a versioned heal policy, with the byte
  `path` injected wire-only and non-hashed under the §7.8 temp-path rule.
  Body minting follows the widened §2 N-body ordinal rule with no `deleted`
  parent. Capabilities gain `op.importStep` + `io.step.import`. The §7.8 →
  `InspectStep` swap is the one **subtractive** delta and removes a verb no
  layer ever implemented or fixtured, so no shipped behavior moves and
  `protocol/fixtures/` is untouched — **no fixture bump**.

- **2026-08-02 — §2 + §7.2 `body_<opId>:<k>` widened from "boolean split" to
  "ordered children of any N-body op"** (WP-0 identity prerequisite; cross-track
  sign-off recorded 2026-08-02. Doc-only normative widening: no wire byte, verb,
  arg or result field moves and today's sole producer — the boolean split — is
  byte-identical, so **no fixture bump**.) Motivations: (1) the forthcoming
  multi-solid STEP import mints `created` children with NO `deleted` parent, which
  the old prose implicitly coupled; (2) §2 now states the single-vs-multi minting
  rule explicitly (one new body ⇒ plain `body_<opId>`; N > 1 ⇒ `:<k>`); (3) the
  ordinal domain is declared unbounded — the Rust core's former 256-ordinal
  `split_origin` recovery probe (a silent cross-process identity loss for k ≥ 256)
  was replaced by an exact derivation-time memo (`onecad-core` `document/body.rs`,
  non-wire change, regression-locked at k = 300).

- **2026-08-01 — §7.6 NEW read-only verb `ProjectFaceBoundary`**
  (SKETCH-ON-FACE W1a; cross-track sign-off recorded 2026-08-01.
  Additive verb: no existing verb, arg, result field or wire
  byte moves, and `protocol/fixtures/` is untouched, so **no fixture bump**.)
  This is the missing PRODUCER for the `referenceLocked` chain the entry below
  gave a contract but no author. It returns a picked planar face's boundary as
  2D primitives so Rust can mint locked sketch geometry from it.
  * **Read-only** (§7.6): no fence, no prepare/accept/discard, no element-map
    minting. It reads a copy of the head, exactly like the §7.5 identity verbs, so
    a stale or absent reference answers `present:false` rather than erroring.
  * **Addressing** (§7.6): `elementId` or `{bodyId, topoKey}` — the same pair
    `QueryElement` takes, but with one extra rung on the `elementId` branch
    (partition entry → `(bodyId, topoKey)` → sub-shape). `QueryElement` stops at
    the partition descriptor and never reaches a `TopoDS_Shape`; this verb must,
    so the rung is spelled out normatively rather than left implied.
  * **The plane is INPUT and AUTHORITATIVE** (§7.6): Rust owns the basis; the
    worker expresses every coordinate in it and never substitutes its own. The
    two-round-trip handshake — `frameOnly` for the kernel-exact `gp_Pln` frame,
    then the real projection in the basis Rust built from it — exists because a
    descriptor `center` is an axis-aligned bbox centre and sits OFF the face plane
    for a tilted face (a sliver at extrude time).
  * **Response-local refs** (§7.6): `points[].ref` is `p<N>`, 0-based, scoped to
    the one response. It is deliberately NOT an `ElementId` — Invariant 1 keeps id
    minting in Rust, and a read-only query mints nothing.
  * **Lossy fallback documented** (§7.6): unsupported curve types (B-spline,
    ellipse, …) degrade to a Line polyline of `fallbackSegmentsPerCurve` segments.
    Called out explicitly because the loss is permanent and invisible downstream.
  * **Determinism** (§7.6): the emission order (seed face → outer wire → holes →
    `BRepTools_WireExplorer` edges; points numbered by first use) is normative, so
    the response is byte-identical across fresh worker processes.
  * **Not changed**: every existing verb, `QueryElement`'s own contract, the
    §7.3/§7.4 sketch wire shapes, and the §8 error taxonomy (this verb uses only
    the existing `OP_FAILED` / `PROTOCOL_ERROR` codes).
- **2026-08-01 — §7.3 `entities[].referenceLocked` documented AND honored; §7.4
  region-detection exclusion narrowed to `construction` alone**
  (W0b host-face reference geometry; cross-track sign-off recorded 2026-08-01;
  additive optional field, emitted only when `true`, so **no existing wire byte
  moves and no fixture bump** — `protocol/fixtures/` carries no sketch payloads
  at all, it remains `hello` + `echo_error`).
  The flag existed in OneCAD-CPP on the base `SketchEntity` and the ported worker
  still carried all eight of its edit guards, but nothing ever SET it: the wire
  never spoke it, so every guard was dead code. This entry gives it a contract
  and makes the chain live end to end (Rust core → wire → worker → solver →
  frontend). **There are still ZERO producers** — the face-boundary projection
  that authors locked geometry lands in a later wave, so in practice every flag
  on the wire today is absent.
  * **Wire shape** (§7.3): optional bool on all five entity kinds, default
    `false`, **emitted only when `true`**. Synthesized child points inherit it;
    a point referenced by `p0Ref`/`p1Ref` keeps its own.
  * **Regions** (§7.3/§7.4): locked geometry **DOES** bound regions. This is the
    explicit contrast with `construction`, and §7.4's exclusion paragraph is
    narrowed to say `construction` is the only flag that drops an entity from
    loop detection. Region ids are untouched either way.
  * **Edits** (§7.3): readers refuse every geometry-mutating edit against locked
    geometry — removal (and its cascade), reposition, split, drag, `construction`
    flip — and refuse a translation of any set containing it outright rather than
    translating the free half.
  * **Constraints** (§7.3): a constraint of ANY kind may reference locked
    geometry. The oracle vetoed everything but `Fixed` here; that veto is
    REMOVED, because snapping a profile to the projected boundary is the entire
    purpose of the flag.
  * **Solver pins** (§7.3/§7.4): immobility moves from the veto to the solver.
    Every parameter a locked entity owns is pinned under the reserved INTERNAL
    tag `0` (the arc-rules contract: never blamed, never redundant), and the
    naive-dof fallback subtracts those equations the same way it subtracts the
    arc rules.
  * **Not changed**: any existing wire shape, any region id, the DOF of any
    sketch without locked geometry, and the `construction` semantics.
- **2026-08-01 — §7.3 an `Arc`'s START/END are real, addressable points;
  §7.4 naive-dof gains a −4-per-endpoint-bearing-arc term** (W0b arc-endpoint
  handles; **aligns the implementation with text this document has always
  carried, additive, no fixture bump** — `protocol/fixtures/` remains
  `hello` + `echo_error`).
  This entry makes two long-standing examples TRUE rather than aspirational: the
  §7.3 `Coincident` with `"positions": ["End","Start"]` over an arc, and the §7.4
  `SolveDrag` with `"pointId": "e3.start"`. Both previously failed — a reader had
  no `<id>.start`/`<id>.end` handle to resolve, so an arc-endpoint `Coincident`
  was rejected ("unresolved point handle") and the frontend marshaller dropped it
  before it ever reached the wire.
  * **Minted endpoints** (§7.3): a reader mints a `Point` per arc endpoint at its
    derived coordinate and registers `<id>.start`/`<id>.end`, mirroring the
    `<id>.center` contract. They are coupled to center+radius+angle by an
    INTERNAL arc-rules constraint (reference implementation: PlaneGCS
    `addConstraintArcRules` under the reserved tag `0`) — 4 parameters, 4
    independent equations, so `dof` is unchanged, and it must never surface in
    `conflicting[]` or as a redundant constraint. The arc's stored
    parameterization is untouched.
  * **Echo** (§7.3): a reader MUST echo whichever endpoint form the request
    carried — `start`/`end` coordinates OR `startAngle`/`endAngle`. Welded caps
    genuinely rotate under a solve, so echoing only center+radius would leave the
    producer holding angles the reader no longer has.
  * **Naive dof** (§7.4): the fallback count must subtract 4 per
    endpoint-bearing arc, since the coupling equations are not in
    `constraints[]`.
  * **Not changed**: the constraint wire shape (`positions` already existed and
    already meant this), the entity wire shape, and the DOF of any sketch without
    an arc. A `Coincident` with both positions absent/`Arbitrary` serializes
    byte-identically to before.
  * **Interop note, not normative**: welding a line endpoint to an arc endpoint
    makes an ENTITY-level `Tangent` between the same pair first-order degenerate
    (`distance(center, line) == radius` is then at a maximum, so its gradient
    vanishes on the manifold of the other constraints). PlaneGCS diagnoses such
    tangents as redundant. Producers authoring both should expect that; endpoint
    tangency as a distinct constraint kind is not modeled in V1.
- **2026-08-01 — §7.3 `Ellipse` is SUPPORTED on the solver lane (wire entity
  shape pinned); §7.4 naive-dof deviation + ellipse fragment note** (W3 ellipse
  backend; **additive entity type, no existing shape moved, no fixture bump**).
  `Ellipse` was pinned UNSUPPORTED by the 2026-07-20 entry, but the only thing
  actually missing was one branch in the worker's `WireSketch::translate` (and
  the matching arm in Rust's `wire_entity`, which skipped the variant to match):
  `SketchEllipse`, `LoopDetector`'s ellipse tessellation, `FaceBuilder`'s true
  `Geom_Ellipse` edge, the `RegionTable` signature (type-agnostic — loop edge
  tokens only) and the solver-unsupported naive-dof fallback all already existed.
  Because plan-time profile derivation (`ops/OpCommon.cpp`) shares that same
  translate, the one branch unlocks BOTH lanes: an ellipse now solves, forms a
  region, and extrudes.
  * **Wire shape** (§7.3): `{ id, type:"Ellipse", center:[cx,cy], centerRef?,
    majorR, minorR, rotation?, construction? }` — center INLINE exactly like
    `Circle` (the reader mints the center `Point` and registers `<id>.center`,
    so it is draggable); `rotation` in **radians**, optional, default `0`;
    `centerRef` extended to `Ellipse` on the return wire (informational inbound).
  * **Normalization echo** (§7.3): a reader MAY enforce `majorR >= minorR` (the
    reference `Sketch::addEllipse` swaps and adds π/2 to `rotation`) and MUST
    then echo the normalized triple, so a producer never re-sends parameters the
    reader does not hold.
  * **Naive-dof deviation** (§7.4): an ellipse is not registered with PlaneGCS
    (legacy parity — the OneCAD-CPP oracle solver has no ellipse binding), so an
    ellipse-bearing sketch reports statically-counted dof; redundancy goes
    unreported and `OverConstrained` means `count < 0`.
  * **Fragments** (§7.4): a PURE ellipse loop is exact (3-face analytic prism,
    volume π·a·b·h); an ellipse split by another curve falls under the existing
    chord-fragment V1 limitation like any arc.
  `Spline` REMAINS UNSUPPORTED. Core serde untouched (the `Ellipse` variant was
  already complete and golden-pinned). Worker + Rust + frontend sign-off. No
  canonical `protocol/fixtures/` bump — those payloads (`hello`, `echo_error`)
  carry no sketch entities — and every existing wire shape is byte-stable
  (region ids unchanged: pinned by worker `test_sketch_ellipse`). Fixtures:
  `worker/tests/fixtures/sketch_ellipse_upsert.ndjson` (naive dof 5, then 3 with
  a Fixed centre), `sketch_ellipse_regions.ndjson` (corpus `regions_ellipse`, one
  cell), `sketch_ellipse_gesture.ndjson` (drag `<id>.center`),
  `executeplan_ellipse_extrude.ndjson` (real-OCCT plan profile). Tests: worker
  `sketch_ellipse` (+ the corpus area oracle and the normalization echo) and Rust
  real-worker `sketch_regions.rs`, `sketch_edit.rs`, `wire_contract.rs`,
  `sketch_reentry.rs`.

- **2026-07-31 (b) — §7.3 `entities[].construction` documented AND honored; §7.4
  construction exclusion stated** (W1-A construction geometry; **additive
  optional field, no shape moved, no fixture bump**). The Rust producer has
  emitted `construction` on every sketch entity since the wire existed, but the
  worker's `WireSketch` hardcoded `false` and dropped it — so `LoopDetector`'s
  construction filters were dead code and a closed construction rectangle still
  published a region and still extruded. The worker now reads the flag
  (absent ⇒ `false`) and honors it on BOTH consumers of loop detection
  (`SketchRegions` and plan-time profile derivation); synthesized child points
  (inline line endpoints, arc/circle centers) inherit their parent's flag.
  Construction geometry remains fully solved. Core gains
  `SketchEditOp::SetEntityConstruction` to flip one entity's flag. Worker + Rust
  + frontend sign-off. No fixture bump: no canonical `protocol/fixtures/` payload
  carries sketch entities, and every non-construction region id is byte-unchanged
  (signatures derive only from loop edge ids — pinned by worker
  `test_sketch_construction` and `src-tauri/tests/sketch_construction.rs`).

- **2026-07-31 — PREVIEW wave: §7.6 `PreviewOp` now exercised by ALL op types**
  (Revolve/Fillet/Chamfer/Shell/Boolean joined Extrude). **No wire change** — the
  verb, envelope, and op payloads were already generic; this entry records that
  the "preview callers MUST NOT maintain a second ad-hoc mapper" rule is now
  enforced client-side by a single shared builder table (`previewOps.ts`
  `OP_BUILDERS`, mirrored by the Rust lowering-equality test over all six
  opTypes) and proven per-op by real-worker preview==commit volume tests
  (`preview_revolve.rs`, `preview_edge_shell.rs`, `preview_boolean.rs`, worker
  `test_preview_op` cases 12–14 incl. a pre-cancelled-token `CANCELLED`
  terminal). No fixture bump (no shape moved).

- **2026-07-29 (b) — §8 new error code `STALE_PREVIEW`; §7.4 fail-closed partial
  triangulation + chord-limitation note; §7.3 `ToNext` binds bounded faces**
  (EXTRUDE-REGION-PARITY hardening; worker + Rust + frontend sign-off, no fixture
  bump — error path + op semantics only, no canonical fixture carries them).
  `PreviewOp` stale-base rejection now carries `error.code:"STALE_PREVIEW"` so
  Rust routes on the code instead of sniffing message text. A stalled ear clip
  fails the `SketchRegions` request instead of publishing partial material.
  `ToNext` resolves the extrude distance against bounded faces the profile
  actually reaches (rays from profile vertices + centroid); the legacy
  nearest-ray-PLANE rule could bind a face plane the profile never crosses.
  Intersection-fragment chord approximation documented as a V1 limitation.

- **2026-07-29 — §7.4 selectable planar cells + collision-safe region identity;
  §7.6 canonical preview lowering** (EXTRUDE-REGION-PARITY; worker + Rust +
  frontend sign-off). `SketchRegions` and modeling profile lookup now consume one
  solved region table. Every bounded cell is selectable, complete material
  boundaries (including holes/intersection fragments) participate in identity,
  duplicate canonical ids are fatal, and legacy outer-only ids resolve only
  when unique. Required holes fail closed. `PreviewOp` receives the same
  canonical worker operation form as `ExecutePlan`; missing explicit region/body
  bindings may no longer fall back. The worker sketch-region fixture and
  cross-layer non-first-region preview/commit tests cover the changed semantics;
  no existing canonical `protocol/fixtures/` payload carried these shapes.

- **2026-07-26 — §7.6 new verb `PreviewOp`** (MODEL-OPS W3; **additive — a new
  verb, no existing shape changed, no fixture bump**). Before it, the "exact"
  drag-time mesh was synthesized CLIENT-SIDE by the same function the mock client
  uses, so a `Cut` preview never subtracted and Revolve/Fillet/Shell had no
  preview at all — the commit could produce something the preview had never
  shown. The verb runs the candidate op through the same executor a real plan step
  uses, over a throwaway copy of the head, and returns MESH1 for only the bodies
  it touched. Its load-bearing guarantee is that it is invisible to fencing: no
  fence, no prepare, no accept, no scratch, head tokens unchanged. Kernel lane
  (OCCT stays single-writer); in-flight bounding is the caller's job.
  [§7.6](#76-geometry).

- **2026-07-26 — §7.3 `plane.kind: "custom"` provenance documented** (MODEL-OPS
  W2; **text-only, NO wire change, no fixture bump**). A sketch's wire `plane`
  carries only a resolved basis; the ATTACHMENT that justifies it
  (`world` / `datum` / `hostFace`) is core-owned state that never crosses the
  wire, and `plane_kind_str` collapses both non-world attachments to `custom`.
  That was already true and already implemented on both sides
  (`WireSketch::parse_plane` has always accepted an arbitrary custom
  origin/xAxis/yAxis/normal) — it simply had no producer until sketch-on-face
  landed, and no prose said where a `custom` basis legitimately comes from.
  Recorded now so a reader does not mistake `custom` for "unknown/legacy".
  [§7.3](#73-operation-payloads).

- **2026-07-26 — §7.4 `previewTriangles` now SUBTRACTS holes** (MODEL-OPS W0;
  **behavioural change to the fill, no wire-shape change, no fixture bump** —
  canonical fixtures carry no hole-bearing region). The solver lane previously
  ear-clipped a region's outer loop only, so a region with a hole was served as a
  solid fill while `FaceBuilder` built (and the kernel extruded) a face WITH the
  hole. The drag preview therefore disagreed with the committed solid, and a
  click inside a hole hit-tested as the enclosing region. Holes are now merged
  into the outer loop with bridges and the whole thing ear-clipped
  (`worker/src/loop/PolygonFill.cpp`). Two normative consequences: `vertexCount`
  now spans the outer loop plus every subtracted hole's vertices; and a
  producer's bridge segments MUST stay shared by two triangles, since consumers
  recover the extrusion rings from single-use-edge topology. Additive optional
  `previewTriangles.holesSubtracted` reports how many holes were actually
  removed (a lower value = degraded fill, never a corrupt one).
  [§7.4](#74-sketch--solver-lane).

- **2026-07-26 — §7.3 profile-binding prose corrected to match every shipped
  layer** (MODEL-OPS W0; **text-only, no wire change, no fixture bump**). The
  Extrude and Revolve payload blocks documented the sketch profile as a
  `SketchRegion` semantic ref in `inputs[]`. No layer has ever produced or
  consumed that: Rust's core holds a typed `profile {sketchId, regionId}`, the
  wire layer flattens it (`wire.rs lift_profile_to_params`), and the worker reads
  flat `params.sketchId`/`params.regionId` (`OpCommon.cpp build_profile_face`).
  A `regionId` is content-addressed identity (§7.4) and needs no anchor/intent
  evidence, so the semantic-ref/ladder machinery does not apply to it. The prose
  now states the flat form as normative and records what `inputs[]` genuinely
  carries (ToFace target face, fillet/chamfer edges, shell open faces).
  [§7.3](#73-operation-payloads).

- **2026-07-22 — §7.2 split-child wording aligned with shipped M5a/W3 behavior**
  (MODEL-HARDEN review fix; **text-only, no wire change, no fixture bump**). The §7.2
  `bodyEvents` normative prose still said a split's surviving child keeps the parent id
  and that split children `body_<opId>:<k>` were "deferred" — contradicting the shipped
  M5a (2026-07-19) + Revolve-parity (2026-07-22) behavior, where a boolean split
  **deletes the parent** and mints `created` children `body_<opId>:<k>` in the same
  `planStep`. The prose now describes the shipped grammar (deleted-parent event +
  created-child events; single-solid results emit `modified` on the preserved target).
  Adoption rules are unchanged (D1 — known `opId`, contiguous ordinals, uniqueness).
  No `protocolVersion` change; the wire form and fixtures are untouched (fixtures carry
  no split payloads). [§7.2](#72-regen--executeplan), [§2](#2-identifier--scalar-types).

- **2026-07-22 — Revolve boolean-mode multi-solid results split into deterministic
  `body_<opId>:<k>` children** (MODEL-HARDEN W3 worker parity fix; **orchestrator
  sign-off 2026-07-22** — fixture assessment verified: no revolve/ExecutePlan payloads
  in canonical fixtures, no bump). [§7.2](#72-regen--executeplan),
  [§2](#2-identifier--scalar-types). `RevolveOp` previously committed a boolean result
  with a direct in-place `bodies.create(target_id, …)`, so a Revolve `Cut` that
  bisected a body kept two disconnected solids under **one** body id — asymmetric with
  `Extrude`/`Boolean`, which already fan out via `publish_boolean_result`. The revolve
  boolean tail now calls the SAME `publish_boolean_result(ctx, op_id, target_id,
  br.shape, builder.get(), out)`: a single-solid result modifies the target in place
  (BodyId preserved), a multi-solid result deletes the parent and mints ordered
  children `body_<opId>:<k>`. The M5a minting / adoption / derived-uuid contract (the
  2026-07-19 entry below) applies **verbatim** — adoption keys on plan op ids and is
  op-agnostic, so there is **no Rust change**. The NewBody path is unchanged. *Tests:*
  worker `test_revolve_split` (in-process 360° revolve `Cut` — an annular disc slab cut
  clean through a box → 2 deterministically-ordered children, exact volumes 8000 / 8800,
  ids stable across a replay). **No canonical `protocol/fixtures/` change** — the
  fixtures carry no `ExecutePlan` / revolve / multi-solid-boolean payloads (verified:
  only `hello` + `echo_error`), so there is nothing to bump per
  [§13](#13-versioningchange-policy); this is a truthful widening of an existing split
  rule, wire-compatible (`protocolVersion` stays **1**).

- **2026-07-21 — Per-constraint conflict ids on every solve surface**
  (SKETCH-HARDEN W1; **orchestrator SIGNED OFF** after independent adversarial
  review APPROVE-WITH-FIXES — fixes applied: unconditional endGesture conflict
  write-back, clean-sketch emptiness pinned in `sketch_conflicts.rs`).
  [§7.4](#74-sketch-solver-lane).
  `SketchUpsert` and `EndGesture` results now carry `conflicting: string[]` — the
  constraint ids PlaneGCS reports as mutually unsatisfiable — matching the field
  `SolveDrag` already emitted, so the conflict id set is available on *every* solve
  surface (upsert, drag, end-gesture, and the Rust session-enter DTO that threads
  the `SketchUpsert` list through). **Additive + optional**: an absent field parses
  as `[]` — all parsers tolerate the missing/unknown key (Rust `str_array` defaults
  empty; the frontend types it `conflicting?: string[]`; the C++ subset-matcher
  fixtures tolerate presence-only). The tauri client maps the backend constraint
  uuids back to frontend constraint ids (unknown ids dropped). No `protocolVersion`
  bump (still 1 — pre-implementation contract refinement). Fixtures:
  `worker/tests/fixtures/sketch_conflicting.ndjson` (upsert + SolveDrag + EndGesture
  each assert a non-empty `conflicting`), `sketch_upsert.ndjson` (clean sketch ⇒
  empty `conflicting`). No canonical `protocol/fixtures/` change.
- **2026-07-20 — Solver lane emits the full §7.4 status vocabulary:
  `OverConstrained` state + `redundant` drag status** (AC-USABILITY W-WP1;
  orchestrator SIGNED OFF). [§7.3](#73-op-payload-schemas-vertical-slice),
  [§7.4](#74-sketch-solver-lane). The C++ worker now surfaces PlaneGCS
  **benign, DOF-preserving redundancy** (redundant constraints that remove no
  DOF — e.g. a duplicate dimension) that earlier revisions collapsed into
  `FullyConstrained` (the "documented deviation" citing corpus g is retired —
  the redundant system IS what §7.4 means by `OverConstrained`).
  `SketchUpsert`/gesture `state` gains the 4th value with priority
  `Conflicting > OverConstrained > FullyConstrained > UnderConstrained`;
  `SolveDrag`/`EndGesture` `status` gains `redundant` with priority
  `conflicting > redundant > success`, derived from the redundancy diagnosed
  once at `BeginGesture` (not the drag-polluted per-solve result). New
  `Sketch::hasRedundantConstraints()` accessor mirrors `isOverConstrained()`.
  Rust dto (`SketchSolveStatus::OverConstrained`, free-string drag status) +
  frontend `isConflictStatus` already parse both — worker-emission fix, no
  wire-shape/key change (byte-stable envelope). Also pins Ellipse/Spline as
  solver-lane `UNSUPPORTED` (§7.3). No canonical `protocol/fixtures/` change.
- **2026-07-20 — Solver-lane return wire: optional `centerRef` on Circle/Arc +
  Angle radians units note** (AC-USABILITY F-WP-S0; orchestrator SIGNED OFF).
  [§7.3](#73-op-payload-schemas-vertical-slice). `enter_sketch`/`get_sketch`
  return-wire Circle/Arc entities carry an optional `centerRef` (backend
  center-point uuid) alongside the inlined `center` coords, so re-entry
  hydration can re-own child points (fixes orphaned-Point hydration and makes
  a reopened circle/arc center draggable). Additive + optional: absent on
  legacy producers, ignored on the inbound wire, core serde untouched, no
  fixture change. Angle `value` documented as radians on the wire (UI converts
  deg↔rad at the boundary — fixes the silent degrees-as-radians defect).
- **2026-07-19 — M6a: breadth ops Shell / LinearPattern / CircularPattern /
  MirrorBody implemented (worker + Rust wire)** (§7.3; **orchestrator sign-off
  PENDING**). [§7.3](#73-op-payload-schemas-vertical-slice), [§8](#8-error-taxonomy).
  Additive extension of the §7.3 op catalogue: four op payload schemas derived
  1:1 from the Rust serde param shapes (`onecad-core` `ShellParams` /
  `LinearPatternParams` / `CircularPatternParams` / `MirrorBodyParams` — the wire
  truth). `opType` now also accepts these four; **`Loft`/`Sweep` remain
  `UNSUPPORTED`** ([§8](#8-error-taxonomy) table updated). The worker ports the
  OneCAD-CPP `RegenerationEngine` construction verbatim (Shell:
  `BRepOffsetAPI_MakeThickSolid::MakeThickSolidByJoin` with a **negative** offset;
  patterns: `BRepBuilderAPI_Transform` + chained `BRepAlgoAPI_Fuse` or a compound,
  step angle `angleDeg/count`; mirror: `gp_Trsf::SetMirror`). **Shell** replaces
  its body (Modify lineage; OCCT history → partition) and resolves its **bare**
  open-face refs (frozen `ShellParams` carries no per-face anchor) via the
  partition-tracked binding or the [§10](#10-resolution-ladder) ladder — a face
  that resolves via neither ⇒ NeedsRepair ([§9](#9-needsrepair-payload)).
  **Patterns/MirrorBody** mint ONE new `body_<opId>` (NewBody lineage; source
  preserved; empty `elementMapDelta`). No `protocolVersion` bump (still 1 —
  pre-implementation contract extension). Fixtures:
  `worker/tests/fixtures/executeplan_linearpattern.ndjson` (full apply),
  `worker/tests/fixtures/executeplan_shell.ndjson` (bare-ref → NeedsRepair). No
  canonical `protocol/fixtures/` change. Tests: worker `m6a_ops` (exact box
  arithmetic — shell 4112, patterns 30000, mirror 20000, guards, NeedsRepair) +
  Rust `src-tauri/tests/breadth_ops.rs` (real-worker exact volumes, determinism
  across two processes, upstream-edit re-run).
- **2026-07-19 — M5a: SaveCheckpoint/RestoreCheckpoint implemented (worker-retained,
  in-session); mesh export (ExportStl/ExportObj) shipped** (§7.7/§7.8;
  **orchestrator-approved 2026-07-19 for the §7.7 divergences**). [§7.7](#77-checkpoints),
  [§7.8](#78-io). **Checkpoints.** `SaveCheckpoint` serializes the session head
  (per-body BREP via **BinTools** + the 3 signatures + `historyPrefixHash`) into the
  resp binary tail AND **retains the head in-session** (keyed by step); Rust stores the
  bytes + parsed envelope metadata and persists them into the `.onecad` container's
  `checkpoints/` layout. `RestoreCheckpoint` **rolls the in-session head back** to the
  step (fenced on `workerEpoch`; a stale `expectedHistoryPrefixHash` ⇒
  `driftDetected`). The Rust `WorkerManager` reconstructs the base `BodyRegistry` +
  `ElementIndex` from the stored artifacts (executor seeds scratch from the immutable
  checkpoint, review F3); the planner selects a compatible checkpoint at/below the
  dirty floor so a post-checkpoint edit regens **incrementally**. Determinism proven:
  an incremental regen (RestoreCheckpoint + incremental plan) yields a head
  byte-identical to a from-0 replay (`src-tauri/tests/checkpoints.rs`).
  ***Divergences to sign off:*** **(a)** the artifact blobs ride **inline in the resp
  tail** (`bin` sections), NOT on the bulk lane / `streamId` the §7.7 example shows —
  sound for the small V1 artifacts; **(b)** restore is **in-session only** — the OCW1
  request path carries no binary, so RestoreCheckpoint cannot re-ship BREP; a worker
  that no longer retains the step (post-restart, post-reopen) reports `restored:false`
  and the executor **replays from 0** (Invariant 7 — the cache degrades to replay,
  never a wrong result). The checkpoint bytes ARE persisted to the container for a
  future cross-restart restore (transport request-binary is the follow-up);
  **(c)** the persisted `elementMapPartition` is a **placeholder** JSON (the in-session
  restore uses the retained partition) and the container form stores the whole
  `CheckpointArtifacts` as JSON (BREP bytes inline) rather than split json/bin — size
  inefficiency, sound. A minor **latent bug fixed:** the Rust `wire_op` omitted
  `stepIndex`, so the worker used the execution-order index; harmless for from-0 plans
  (exec index == step index) but wrong for the incremental plans checkpoints enable —
  now sent per [§7.3](#73-op-payload-schemas-vertical-slice).
  **Mesh export.** `ExportStl` (`{path, bodyIds, binary, lod}` → `{written, bytes,
  triangleCount}`) and `ExportObj` (`{path, bodyIds, lod}` → `{written, bytes}`) reuse
  the worker's tessellation (`tess::tessellate_raw`, same BRepMesh params/winding as the
  viewport mesh ⇒ the STL triangle count equals the tessellation), binary+ASCII STL and
  ASCII OBJ. stdout-hygiene asserted (`test_wp6_meshexport`). Fixtures:
  `worker/tests/fixtures/export_mesh.ndjson`, `checkpoint_roundtrip.ndjson`. No
  canonical `protocol/fixtures/` change.

- **2026-07-19 — M5a: boolean split children `body_<opId>:<k>` are minted, adopted +
  fenced; the Rust `BodyId` is a deterministic derived uuid** (D1 extension;
  **orchestrator-approved 2026-07-19**). [§2](#2-identifier--scalar-types), [§7.2](#72-regen--executeplan).
  When a Boolean op — or a boolean-mode **Extrude** or **Revolve** (any non-NewBody
  mode: `Add`/`Cut`/`Intersect`; the split fires whenever the boolean result is
  multi-solid, in practice a `Cut`/`Intersect` that bisects a body) — yields
  **multiple solids**, the worker deletes the parent (and, for a standalone Boolean
  op, the consumed tool body) and mints a deterministic child `body_<opId>:<k>`
  per solid, ordered by a **quantized geometric key** (volume, then centroid x/y/z,
  then face count, at 1e-6 — never unordered `TopExp` iteration), emitting a `Created`
  `bodyEvent` per child. (Extrude gained this at M5a via `publish_boolean_result`;
  Revolve at MODEL-HARDEN W3 — see the 2026-07-22 entry.) Rust **adopts** them at `AcceptPrepared` (`validate_created`):
  the wire id parses to `(opId, k)`, `opId` must be a **known op in the plan**, the
  per-op ordinals must be **contiguous from 0**, and the id must be **unique** (else
  `PROTOCOL_ERROR`, discard). **Rust-side representation (flagged for sign-off):**
  `BodyId` is a `Uuid` newtype, so a `:<k>` child cannot reuse the opId uuid; it maps
  to a **deterministic derived uuid** = first 16 bytes of `SHA-256("onecad.body.split.v1:"
  ‖ "<opId>:<k>")` (a uuid5-style stable hash — `uuid`'s `v5` feature is off; `sha2` is
  already a dep; the derivation lives in `onecad-core` so both the wire layer and the
  registry share it). The derivation is one-way, so the wire layer keeps a small
  **interner** `derived → "body_<opId>:<k>"`. **Cross-process persistence (orchestrator
  review fix):** the interner alone is only warm within the minting process, but a
  from-0 replay compiles the WHOLE plan *before* the worker re-mints anything — so a
  downstream op that references a persisted split child (e.g. a Cut targeting `:1`)
  would, on reopen in a fresh process, render a bare derived uuid the worker never
  minted (`REF_UNRESOLVED`). Fix: the core `BodyMeta` carries an **additive**
  `splitOf: {op, k}` (serde `skip_serializing_if=None` ⇒ non-split documents are
  byte-identical; `schema_freeze` stays green), populated at adoption/fold time and
  persisted in `document.json`; `DocumentRuntime::open` (and checkpoint restore, via the
  open path) walks the registry and **re-interns every split entry before any plan
  compiles**. (A split child can only be *referenced* by an op added AFTER a regen
  created it — you cannot select a body a not-yet-run op will mint — so within one
  session the interner is warm at compile time; the persisted `splitOf` covers the
  reopen.) The mapping stays deterministic + replay/persistence-stable: the document
  stores the derived uuid and a from-0 replay re-mints the SAME id.
  *Divergence to sign off:* the §2 note called split minting "deferred to W-WP6" and
  said BodyIds "DOES NOT embed BodyId"; this ships the split-child id form and the
  derived-uuid representation (an implementation choice §2 did not pin — §2 only fixed
  the *wire* string `body_<opId>:<k>`, which is honored exactly). On a split, the
  parent's referenced-element partition entries are **dropped** (a rebuildable
  ID-on-demand cache; a later ref re-mints against a child or NeedsRepairs) — no
  confident 1:1 child assignment exists. *Tests:* worker `test_wp6_split` (in-process
  bisecting Cut → 2 ordered children, exact volumes, ids stable), Rust
  `wire_contract::boolean_split_children_adopted` (real worker, 2 children adopted,
  volumes 7500 each, ids stable across replay), `validate_created` unit tests
  (contiguity / unknown-op / collision). No canonical `protocol/fixtures/` change (they
  carry no multi-solid boolean).

- **2026-07-19 — M4a: ResolveRefs `autoBind` returns `elementId` in its own slot;
  `topoKey` is evidence** (code-to-spec; **orchestrator-approved 2026-07-19**).
  [§7.5](#75-element-identity), [§9](#9-needsrepair-payload). The worker's
  `ResolveRefs` `autoBind` resolution now carries the **Rust-minted `elementId`** in
  the [§7.5](#75-element-identity) `elementId` slot (**empty** when the resolved
  element is not yet in the partition — a dry run binds nothing, so Rust would mint at
  real bind time) and the bound `topoKey` as **evidence** *alongside* it
  ([§9](#9-needsrepair-payload): a snapshot-scoped `topoKey` is evidence never
  identity, so it must not occupy the `elementId` slot). Previously the worker put the
  `topoKey` in the `elementId` slot (R-WP12 flag). The Rust parser now reads the
  `elementId` slot strictly (with a one-release tolerance: a legacy `topoKey`-only
  `autoBind` still parses, the `topoKey` landing as evidence). *Fixture bump:*
  `worker/tests/fixtures/resolve_refs.ndjson` `r_autobind` now asserts the `elementId`
  slot present beside `topoKey`. No wire-shape change beyond code-to-spec (§7.5 already
  specified `elementId`); no canonical `protocol/fixtures/` change (they carry no
  `ResolveRefs` `autoBind` flow).

- **2026-07-19 — M4a: Extrude/Revolve profile carries `params.sketchId` +
  `params.regionId`; the worker selects the region by normative FNV id**
  (code-to-spec; **orchestrator-approved 2026-07-19**).
  [§7.3](#73-op-payload-schemas-vertical-slice), [§7.4](#74-sketch-solver-lane). The
  Rust wire layer lifts the core-only `profile` (`SketchRegionRef {sketchId,
  regionId}`) to top-level `params.sketchId` + `params.regionId` (dropping the
  `profile` wrapper — §7.3 has no `profile`; Extrude AND Revolve), and the worker's
  `build_profile_face` selects the closed region whose normative FNV `regionId`
  ([§7.4](#74-sketch-solver-lane) `derive_region_id`, `r_<hash>`) matches. **Strict
  semantics:** a **non-empty** `regionId` MUST match a detected region — **no match is
  a deterministic `OP_FAILED`** (the `perStepResults` message names the requested id +
  the available ids; downstream is blocked, publish ≤ m−1), **never** a silent fallback
  to a different region (a stale id after a sketch edit must fail loudly, not extrude a
  wrong profile — the "never a silent wrong bind" principle). An **empty/absent**
  `regionId` keeps the V1 **first-region** fallback (single-region sketches; the
  region-selection micro-slice does not yet author a real id everywhere). Additive:
  `perStepResults[].message` on a failed step carries the §8 recoverable reason (a
  failed step emits no `planStep`, so this is its only channel to Rust; readers ignore
  unknown keys, §4). This closes the M2 `last_sketch_id` + first-region binding gap
  (multi-region / multi-sketch). *Fixture:* `worker/tests/fixtures/executeplan_region_nomatch.ndjson`
  (new) pins the no-match `OP_FAILED`. No canonical `protocol/fixtures/` change.

- **2026-07-19 — Rust wire layer conformance fix: params body-bearing fields now
  rendered in [§2](#2-identifier--scalar-types) `body_<uuid>` wire form**
  (code-to-spec; **no schema semantic change**). [§7.3](#73-op-payload-schemas-vertical-slice).
  The Rust wire translator now renders every body-bearing op-`params` field
  (`targetBodyId`, `toolBodyId`, `axis.bodyId`, and `targetFace(2).primary.bodyId`)
  in the worker's `body_<uuid>` id form on the way out, matching the `inputs[]`
  semantic-ref rendering that already did so. The `BodyId` core wire encoding was
  a bare uuid (the frozen document schema), which the worker's `body_<opId>`-keyed
  `BodyStore` could never resolve (standalone Boolean / Extrude-Cut/Add / ToFace all
  failed `REF_UNRESOLVED` / NeedsRepair). *This bullet aligns the code with the
  already-normative [§2](#2-identifier--scalar-types) `BodyId` wire form; the wire
  shape the worker parses is unchanged, so no `protocol/fixtures/` file changes (they
  already carry `body_<opId>`-form ids, never bare uuids). Orchestrator-approved
  2026-07-19.*

- **2026-07-18 — a from-0 plan is always base-valid; accept replaces the head
  wholesale** (D5, orchestrator-approved; R-WP11.2). [§7.2](#72-regen--executeplan).
  A **from-0 plan** — no `baseCheckpoint` AND `expectedBaseHash` == the empty-prefix
  anchor (`e3b0c442…`) — is now ALWAYS base-valid: the worker **skips the
  `expectedBaseHash` head-hash comparison**, builds the scratch from a **genuinely
  empty base**, and on `AcceptPrepared` **replaces the head wholesale** (bodies,
  partitions, `historyPrefixHash` = echoed last prefix token, adopted
  `documentRevision`, bumped `snapshotId`). `workerEpoch` fencing and all
  `AcceptPrepared`/`DiscardPrepared` fencing are unchanged; **incremental** plans
  (nonzero `expectedBaseHash`) keep the strict head-hash fence. *Reason:*
  `expectedBaseHash` pins the base a plan builds on, and a from-0 plan's base IS empty
  by definition, so its precondition is satisfiable regardless of the head. V1 is
  full-replay + wholesale-publish (the `RegenPlanner` always emits from-0 plans;
  checkpoints are UNSUPPORTED, [§7.7](#77-checkpoints)); once the head token advanced
  past the empty anchor at the first `AcceptPrepared`, the strict head-hash fence
  rejected every subsequent regen (the sequential-regen blocker before the M2 gate).
  No canonical `protocol/fixtures/` file embeds the old from-0 fencing rule (they
  carry no post-accept sequential-regen flow), so no canonical fixture bump is
  required; the worker's local `test_wp5_plan` + the Rust `worker_chaos` /
  `document_runtime` / `real_worker_smoke` tests cover the D5 sequence, and the
  `worker_chaos` F4 negative was retargeted from the empty anchor (now the from-0
  exemption) to a nonzero wrong hash.

- **2026-07-18 — `documentRevision` is a Rust-owned advisory stamp, not a fencing
  token; fencing = `expectedBaseHash` + `workerEpoch`** (D4, orchestrator-approved;
  R-WP11.1). [§2](#2-identifier--scalar-types), [§7.2](#72-regen--executeplan),
  [§8](#8-error-taxonomy). `ExecutePlan` and `AcceptPrepared` fencing drops the
  `documentRevision` equality check: a session-mutating request is fenced on
  `workerEpoch` (must match the head) **and** `expectedBaseHash` (must equal the head
  `historyPrefixHash`) only. `documentRevision` is a Rust-owned edit counter carried
  as an advisory stamp; the worker stores the plan's `documentRevision` in the
  prepared scratch and **adopts** it as its head `documentRevision` at
  `AcceptPrepared` (worker frame stamps thereafter echo it), instead of incrementing a
  worker-owned accept counter. *Reason:* the pre-D4 worker advanced `documentRevision`
  as its own accept counter, which diverged from Rust's edit counter; every post-edit
  regen (whose `documentRevision` runs ahead of the worker's last-accepted head) was
  then rejected with `PROTOCOL_ERROR`. Making `documentRevision` advisory + adopted
  fixes that while keeping the real precondition guards (`workerEpoch` +
  `expectedBaseHash`). No canonical `protocol/fixtures/` file embeds the old
  `documentRevision` fencing (they carry no `ExecutePlan` fencing flow), so no
  canonical fixture bump is required; the worker's local `executeplan_*` harness
  fixtures were updated to the D4 rule.

- **2026-07-17 — NeedsRepair evidence carries `scoringVersion`** (W-WP6,
  orchestrator sign-off pending). [§9](#9-needsrepair-payload) every NeedsRepair
  payload (in `planStep.needsRepair[]`, `ResolveRefs`, and the history-stage split
  ambiguity) now stamps `scoringVersion` = the `resolverVersion`
  ([§10](#10-resolution-ladder)) under which the candidate
  `score`/`margin`/`featureContributions` were computed. *Reason:* the normalized
  [0,1] scoring is versioned; a repair UI or a future Rust-side policy must know
  which scheme produced the numbers to compare or re-evaluate them. Additive +
  forward-compatible (readers ignore unknown keys per [§4](#4-json-encoding-rules));
  no shape change to existing fields.

- **2026-07-17 — NewBody `BodyId`s are worker-minted deterministic `body_<opId>`,
  adopted+fenced by Rust** (D1, orchestrator-approved; R-WP10). [§2](#2-identifier--scalar-types)
  and [§7.2](#72-regen--executeplan). A `bodyEvents` `created` id is now
  worker-minted `body_<opId>` (`<opId>` = the Rust-minted op record id, so the id is
  a pure function of the plan and replay is stable); Rust **adopts** it at
  `AcceptPrepared`, validating the `body_` prefix + a **known opId** + **uniqueness**,
  and **rejects** the prepared plan (`PROTOCOL_ERROR`, discard) on malformation or
  collision. A future split mints `body_<opId>:<k>` (deferred to W-WP6). *Reason:*
  split/merge body counts are unknowable before OCCT executes, so Rust could never
  pre-mint them; `opId` is Rust-owned, so determinism and replay stability hold with
  worker minting + Rust adoption. This refines the §2 `BodyId` "Rust-minted" note
  (loaded/imported bodies stay Rust-minted; only NewBody flips to worker-mint +
  adopt). No fixture embeds a contrary minting assumption (the current fixtures use
  `body_1` only as a loaded-body example), so no fixture bump is required.

- **2026-07-17 — Rust is the sole hash authority; `ExecutePlan` gains
  `prefixHashes`** (X-WP1, orchestrator-signed). [§7.2](#72-regen--executeplan)
  `ExecutePlan.args` adds `prefixHashes: [hex64, …]` (one opaque token per executed
  op, in `ops` order) alongside the existing `expectedBaseHash`. **Both are
  Rust-minted opaque tokens the worker stores/compares/echoes but NEVER computes.**
  Their provenance is now documented: the SHA-256 over the newline-joined
  *geometry-relevant canonical wire-op form* (`{opId, opType, stepIndex, inputs,
  params, determinism}`, sorted-key JSON), which **excludes** record-level cosmetics
  (`name`, record `extra`, `suppressed`) so a rename never invalidates a checkpoint
  while any geometry-affecting edit does. On `PlanPrepared` the worker echoes the
  token for its last executed op (or `expectedBaseHash` for a base-only prepare) as
  `historyPrefixHash`; Rust verifies the echo (mismatch ⇒ `PROTOCOL_ERROR`). *Reason:*
  the worker cannot see the Rust record shape, so an independently-computed hash
  would diverge; making the token opaque removes a class of false `PROTOCOL_ERROR`s
  and lets a rename/cosmetic edit reuse a checkpoint.

- **2026-07-17 — `elementMapDelta` entries require `bodyId`** (R-WP7.1 review F19,
  orchestrator-signed). [§7.2](#72-regen--executeplan) each `elementMapDelta.added`
  / `.relabeled` entry is now `{ elementId, topoKey, kind, bodyId }` — the owning
  body is **REQUIRED**, not inferred. *Reason:* a single step can create/modify
  several bodies; without an explicit `bodyId` the partition mapping had to guess
  (the "most-recently-created body" heuristic), which mis-partitioned elements when
  one step produced two bodies. The §7.2 example JSON was updated. `bodyId` is
  partition membership only — an element's identity (`elementId`) never changes
  because geometry changed (Invariant 1).

- **2026-07-17 — double-`ExecutePlan`-while-prepared rule** (W-WP4 → recorded here).
  [§7.2](#72-regen--executeplan) pins worker behaviour when an `ExecutePlan` arrives
  while a scratch job is already prepared: **same `jobId`** ⇒ idempotent retransmit,
  reply with the **cached `PlanPrepared`** (no re-execution); **different `jobId`** ⇒
  `PROTOCOL_ERROR` with `detail = { preparedJobId, requestedJobId }`, the held job
  left untouched. *Reason:* a worker holds at most one prepared job; the rule makes
  request retransmission safe and forbids clobbering an outstanding prepare.

- **2026-07-16 — Extrude ToFace targets are typed semantic refs** (R-WP2.1,
  orchestrator-signed). [§7.3](#73-op-payload-schemas-vertical-slice) Extrude
  replaces the bare-string `targetFaceId` / `targetFaceId2` with
  `targetFace` / `targetFace2` **semantic reference** objects (`{primary, intent,
  anchor}`, the shape already used by fillet edge refs). *Reason:* a bare id
  carries no anchor/intent, so a ToFace target could not be rebound by the
  resolution ladder after a parametric edit — it would be un-repairable,
  violating Invariants 2/3. The example JSON was updated accordingly. No other
  §7.3 op needed the same treatment: the Revolve `axis` is already a structured
  ref (`sketchLine`/`edge` with typed subfields), Boolean `targetBodyId`/
  `toolBodyId` reference whole **bodies** (referenced directly by id, not
  ladder-resolved sub-elements), and Fillet/Chamfer `edgeIds` stay bare strings
  because their per-edge repair evidence already rides in the op's `inputs[]`
  semantic refs (mirrored in the Rust core by `FilletParams.edges`).

- **2026-07-16 — Scalar/dimension fields accept number OR object** (R-WP2.1).
  [§7.3](#73-op-payload-schemas-vertical-slice) now states explicitly that a
  dimensional param may be a bare number or a `{value, expr?}` object and that
  both producers/readers must accept either; the Rust core normalizes to the
  object form on write. Documents the file↔wire form already in effect; no shape
  change to the examples (which keep the bare-number spelling).

- **2026-07-16 — `regionId` derivation made normative** (R-WP3 → recorded here).
  [§7.4](#74-sketch-solver-lane) SketchRegions now pins the exact FNV-1a-64
  algorithm (sorted 16-byte member UUIDs + winding byte, rendered `"r_%016x"`) so
  the C++ worker and Rust core produce identical region ids; reference impl is
  onecad-core `sketch/mod.rs::derive_region_id`.
