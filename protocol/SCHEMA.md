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
| `BodyId` | JSON string | Opaque, globally unique (e.g. `"body_7"`). **Minting is split (D1):** a **NewBody** body is **worker-minted deterministic** `body_<opId>` (the `opId` is the Rust-minted op record id, so replay is stable); an op whose result is **N > 1 ordered bodies** (today: a boolean split; the rule is generic to any N-body op, e.g. a multi-solid import) mints `body_<opId>:<k>` with deterministic `k`-ordering, while an op producing exactly **one** new body normally mints the plain `body_<opId>` form. **Pattern V2 is a source-preserving exception:** non-fused count `N` creates `N−1` ordinal children, and fused V2 creates no new body because it modifies source in place. Rust **adopts** these ids from `planStep` `bodyEvents` at `AcceptPrepared` time, validating format (`body_` prefix + a known `opId` in the plan) and uniqueness, and **rejects** the prepared plan (`PROTOCOL_ERROR`, discard — never publish) on malformation/collision. All *other* body ids (bodies loaded from a saved document) stay Rust-minted; **imported** bodies (§7.3 `ImportStep`) are worker-minted ordinal children under the N-body rule above. See [§7.2](#72-regen--executeplan). |
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
    "occt": { "version": "8.0.1", "fingerprint": "9a1c33f0e7b24d10" },
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

- `occt.fingerprint`: 64-bit hash of `{occtVersion, sourceCommit,
  normalizedBuildOptions, buildId, kernelPolicyVersion}`. Governs
  BREP/checkpoint cache compatibility. Per-operation `occtOptions` remain part
  of operation/history hashing and are not duplicated into this global value.
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
  "editedFrom": 4,                            // optional — step of the upstream content edit
  "checkpointFallbackReplay": true,           // optional — this replay stands in for a failed checkpoint plan
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
- **`editedFrom` (OPTIONAL) — the edit-context declaration.** The timeline step
  index of the upstream **content edit** that triggered this regen. It carries no
  fencing, changes no geometry, and gates exactly one thing: the
  [§10](#10-resolution-ladder) descriptor-tie veto, which applies to refs owned by
  a step **strictly greater than** `editedFrom`. `> `, not `>= `: step
  `editedFrom` is the edited op itself and Rust re-stamped its refs as part of the
  edit, so they are fresh.
  **ABSENT means "no claim", and absence is the safe default.** Only the edit lane
  sets it — a `RegenToEnd(from)` whose `from > 0`, i.e. one the scheduler derived
  from an `UpdateOperationParams` / `EditOperationInput` / `AddOperation` /
  `RemoveOperation` / suppression command's dirty floor. Every **no-edit replay**
  lane omits it: open-time replay, STEP import, crash recovery, undo, redo — all of
  which request `RegenToEnd(0)` explicitly — and every `RegenToStep(k)` preview.
  **`from == 0` is deliberately treated as absent**: a from-0 replay is
  indistinguishable from a first-record edit here, and it rebuilds exactly the
  geometry every stored anchor was authored against, so claiming an edit there
  would veto every congruent-twin resolution in the document and make a clean
  reopen un-resolvable. Under-claiming costs one veto on a step-0 edit;
  over-claiming breaks reopen — the conservative direction is ABSENT.
  Rust owns this field entirely: it is the only side that knows *why* a regen was
  requested. The worker MUST tolerate its absence (pre-`resolverVersion`-2
  behaviour) and SHOULD treat a non-integer value as absent rather than as an
  error, per [§4](#4-json-encoding-rules).

- **`checkpointFallbackReplay` (OPTIONAL) — the replay-provenance declaration.**
  `true` iff this plan is a replay **substituted for a checkpoint plan that failed
  before its first `planStep`** — a restore that reported `restored:false`, a
  restore that reported drift, or a pre-step engine failure or crash. Rust's regen
  executor sets it on that fallback re-plan and **nowhere else**; absent (the
  default) means an ordinary regen, including an ordinary replay-from-0.
  It carries no fencing and changes no geometry. It gates exactly one thing: the
  **anchor-exact carve-out** inside the [§10](#10-resolution-ladder) descriptor-tie
  veto. Stored anchors are world-frozen at ref-authoring time; on this lane the
  basis they were frozen against is one the restore failed to reproduce, so an
  element sitting exactly on a stored anchor has NOT demonstrably "stayed put" and
  must not be allowed to settle a tie. On every other lane the carve-out stays ON —
  the teleport case there (an edit that parks a congruent twin precisely at the
  stale anchor) remains the accepted, documented residual.
  **Why this cannot be derived worker-side.** The fallback re-plan and an ordinary
  replay-from-0 are byte-identical as plans: same empty `expectedBaseHash` anchor,
  same absent `baseCheckpoint`, same op list. Partition emptiness does not stand in
  for it either — under [D5](#72-regen) *every* from-0 plan clones an empty base, so
  that discriminator is true of every ordinary regen and degenerates to
  `editedFrom.is_some()`, which vetoes the ordinary edit lane. Only Rust knows the
  provenance, so only Rust may declare it.
  The worker MUST tolerate its absence and SHOULD treat a non-boolean value as
  absent rather than as an error, per [§4](#4-json-encoding-rules). It is meaningful
  only together with `editedFrom` — the veto it gates is itself edit-scoped — but
  the two are independent fields and either may appear without the other.

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
  "diagnostics": [ { "severity": "warning", "code": "…", "message": "…" } ],
  "matePlacement": { "translate": [0, 0, 5], "rotate": { "center": [0,0,0], "axis": [0,0,1], "angleDeg": 0 } }
}
```

- **`matePlacement` (OPTIONAL, Component Library P3 WP-3.1, spec §5.5).**
  Present ONLY on a `PlaceComponent` step whose `params.mate` resolved
  `AutoBind` through the ladder AND the recomputed seat moved beyond the
  worker's pinned reseat epsilon (`ComponentOp.cpp`'s
  `kMateReseatTranslationEpsilonMm`/`kMateReseatRotationEpsilonDeg`) —
  absent on every other step, keeping the pre-WP-3.1 wire byte-identical.
  Same `FrozenPlacement` shape as `PlaceComponentParams.placement`
  (`{translate, rotate:{center, axis, angleDeg}}`). Rust persists this as a
  DERIVED, no-undo writeback of that record's `placement` field (mirrors
  `sync_record_outputs`) — it is never itself a fencing input, and a mate
  that resolves `NeedsRepair` (target vanished/ambiguous) never populates
  this field; the component publishes at its last frozen `placement`
  instead, per the "never drop it, never silently move it" rule.

`diagnostics[]` is additive structured evidence. Required fields are
`severity` (`"info" | "warning" | "error"`), `code` (≤128 bytes), and
`message` (≤4096 bytes). Optional `stage` is ≤64 bytes. Optional `evidence` is a
JSON object whose encoded size is ≤64 KiB; producers also bound every contained
array. Readers ignore and log malformed diagnostic entries or optional fields;
malformed diagnostics never invalidate an otherwise valid frame. At most 64
diagnostics are consumed per carrier.

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
- **`bodyEvents[].rankKey` — OPTIONAL identity-tripwire evidence (VF-B6).** An op
  whose child ordinals are a **geometric rank** MAY attach, to each `created` (and
  the single-solid `modified` that op would otherwise have produced), the exact key
  it ranked that solid by:

  ```json
  { "kind": "created", "bodyId": "body_op4:0",
    "rankKey": [7500000000, -10000000, 7500000, 12500000, 6] }
  ```

  Normative form: a **5-element array of signed integers**,
  `[volume, centroidX, centroidY, centroidZ, faceCount]`, where the first four are
  quantized as `llround(value * 1e6)` (the §10 `quantizationVersion = 1` step) over
  `BRepGProp::VolumeProperties` mass + centre of mass, and `faceCount` is the exact
  face count. Ordinals are assigned by a **stable sort on the lexicographic order of
  this tuple**, ascending — so `rankKey` is not a description of the child, it *is*
  the reason the child got ordinal `k`.

  The field is **diagnostic/identity evidence, consumed by Rust**, and carries no
  execution semantics: the worker never reads it back, and it is deliberately NOT
  folded into the §12 `bodyLifecycle` signature (that signature pins the ordered
  create/modify/delete lineage; folding a geometric measurement into it would make
  every dimensional edit look like a lifecycle change).

  **Absence means "no claim", never "no key".** A producer whose ordinals come from
  *lineage* rather than geometry MUST omit it — `TransformBody`'s copy path (ordinal
  = the caller's target-list index) and `ImportStep` (ordinal = the position in the
  content-addressed source blob, which no parametric edit can permute) both omit it
  today. Rust MUST tolerate the omission by skipping its tripwire for that body, and
  MUST NOT infer anything from a missing key.

  *Why Rust needs it:* `body_<opId>:<k>` is stable in NAME but its `k` is a
  **geometric rank, not lineage**. A parametric edit that makes two split pieces
  cross in that rank order silently re-points `:0` at a different solid, and every
  downstream reference then re-resolves *cleanly* to the wrong body. Publishing the
  rank key lets Rust detect the permutation and raise a deterministic `NeedsRepair`
  (§9 `ordinal-permutation`) instead of binding silently.

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
    { "stepIndex": 6, "status": "needsRepair", "refCount": 1 },
    { "stepIndex": 7, "status": "opFailed", "message": "Fillet failed",
      "diagnostics": [
        { "severity": "error", "code": "FILLET_WALKING_FAILED",
          "message": "Fillet failed", "stage": "build", "evidence": {} }
      ] }
  ],
  "historyPrefixHash": "9c4d…",

  // OPTIONAL — present iff req.args carried `artifacts.tessellate` (see below).
  "artifacts": {
    "tessellate": {
      "meshes": [
        { "bodyId": "body_3", "format": "MESH1", "bin": "mesh:body_3",
          "lod": "coarse", "totalBytes": 4096, "triangleCount": 12,
          "sha256": "…", "snapshotId": 5013 }
      ]
    }
  }
}
// bin: [ { "name": "mesh:body_3", "off": 0, "len": 4096 } ]
```

All terminal fields above are mandatory and strictly typed. `perStepResults` is
the unique, ordered prefix of the requested `ops[].stepIndex` sequence; indices
may not be missing, duplicated, reordered, or invented. `completed` reaches the
last requested step and contains only `ok` rows. `opFailed`/`needsRepair` ends in
exactly one matching terminal row, and `lastValidStep` names the immediately
preceding `ok` row (or is `null`). Rust MUST discard scratch and return
`PROTOCOL_ERROR` before `AcceptPrepared` when any stream/terminal consistency
rule fails. Unknown status or stopped-reason values are never compatibility
defaults.

**The mate carve-out.** A per-step `ok` row means *this step published geometry*,
not *this step has nothing left to repair*. A published step MAY carry a non-empty
`planStep.needsRepair[]` when — and only when — every entry names a **mate**
(`PlaceComponent.mate`), never a topological input the step consumed to build its
shape. This is the one place the two are separable: a component's geometry does not
depend on its mate, so a mate that cannot re-seat leaves a perfectly valid body
sitting at its frozen placement. Failing the step there would destroy geometry to
report a *placement* problem, and truncating the stream would take every later step
with it; publishing with `needsRepair[]` hands the user the same repair affordance a
stopped stream would, and keeps the model. `stoppedReason: "completed"` is therefore
compatible with repair state on an `ok` row, and Rust's stream validation accepts it
by design — an `ok` row whose `needsRepair[]` named a topological input would be a
worker defect, not a variant of this rule.

The prepared snapshot is held in scratch, NOT published. `preparedSnapshotId`
becomes live only after `AcceptPrepared`.

- **`artifacts.tessellate` (OPTIONAL) — the prepare's inline meshes.** When the
  request carried an `artifacts.tessellate` rider, the worker tessellates **every
  body in the prepared scratch** and returns one §7.6-shaped mesh handle per body,
  with the MESH1 bytes appended to *this* `resp`'s binary tail and addressed by the
  handle's `bin` section name. `snapshotId` is the prepare's `preparedSnapshotId`.
  Emitting it is what makes a regen tessellate each body **once** instead of twice
  (prepare, then a separate `Tessellate` pull per body for the viewport).
- **ABSENT on the idempotent cached re-return.** Re-sending the same `jobId` while
  the prepare is still held returns the cached `PlanPrepared` JSON, which carries
  **no** `artifacts` key — the bytes rode the original `resp`'s tail only, and a
  cached result referencing `mesh:*` sections that are not in *this* frame would
  dangle. Readers MUST tolerate the absence.
- **Readers MUST treat this as a cache, never as geometry authority (Invariant 7).**
  A missing, truncated, or failing-verification handle degrades to a `Tessellate`
  pull for that body and MUST NOT fail the plan. In particular a reader MUST verify
  `totalBytes` + `sha256` and the MESH1 header before using a blob, MUST check the
  handle's `snapshotId` against `preparedSnapshotId`, and MUST cap total ingest.
  Rust does exactly this (`worker::wire::parse_plan_artifact_meshes`): a malformed
  handle is warned and skipped individually; an invalid section table drops the
  whole artifact set; both leave the plan a normal success.
- **Transport limit.** A prepared mesh is inlined only when that blob fits the
  advertised `chunkSize` and the response aggregate fits `initialBulkCredit`.
  Bodies that do not fit are omitted from this cache; Rust uses the existing
  chunked `Tessellate` pull.

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
| `TransformBody` | `Hole` | `OffsetFace` | `PlaceComponent` | `DetachComponent`
| `Gear`
(the M6a breadth ops, the 2026-08-02 `ImportStep`/`TransformBody`, the
Component Library ops, and the 2026-08-14 `Gear` generator extend the original
vertical slice — see the [Changelog](#14-changelog)).
`Loft` and `Sweep` remain **`UNSUPPORTED`** ([§8](#8-error-taxonomy)). Values
keep OneCAD-CPP `operationTypeName` spelling (PascalCase). Spec §3.2/§3.3's
`SetComponentParams`/`ReplaceComponent` are NOT `opType` values at all — both
are in-place edits of `PlaceComponent`'s own params at the Rust layer (see
`DetachComponent`'s entry below for why only it earned a real `opType`).

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
  "regionId": "r_ac127d8846949…",
  "regionIdentityVersion": 3,    // new authoring; persisted V1/V2 remain frozen
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

- **Draft is applied or refused, never silently dropped.** A non-zero
  `draftAngleDeg` must add at least one eligible side face, the builder must
  complete, and the result must differ from the undrafted prism; otherwise the step
  is a recoverable `OP_FAILED` carrying one of the draft diagnostic codes
  (`stage:"build"`, evidence `{draft:{angleDeg,eligibleFaces,addedFaces}}`):

  | code | meaning |
  |---|---|
  | `EXTRUDE_DRAFT_NO_PLANAR_FACE` | the profile has no planar side face to draft (e.g. a circular profile) |
  | `EXTRUDE_DRAFT_NO_FACE_ACCEPTED` | eligible walls existed; the kernel rejected every one |
  | `EXTRUDE_DRAFT_NO_CHANGE` | the builder completed and changed nothing (adds `volumeBefore`/`volumeAfter`) |
  | `EXTRUDE_DRAFT_BUILD_FAILED` | the draft build itself failed or threw |

  These are diagnostic codes only; the §8 top-level code stays `OP_FAILED`. The
  preview lane reports the same code as the commit for the same candidate.

**Profile binding (NORMATIVE, Extrude / Revolve / Sweep).** The sketch profile is
carried as **flat `params.sketchId` + `params.regionId`**, NOT as a semantic ref
in `inputs[]`. A region is identified by the derived `regionId` (§7.4), which is
already a stable, content-addressed identity — it needs no anchor/intent evidence
and no ladder, so the semantic-ref machinery does not apply to it. Rust's core
`ExtrudeParams`/`RevolveParams` hold a typed `profile { sketchId, regionId,
regionIdentityVersion? }`
object; the wire layer FLATTENS it (`src-tauri/src/worker/wire.rs`
`lift_profile_to_params`) and the worker reads the flat keys
(`worker/src/ops/OpCommon.cpp` `build_profile_face`). Producers MUST send the
flat form; a nested `params.profile` is not consumed by the worker.

`inputs[]` still carries genuine semantic refs for elements that DO need the
ladder — the Extrude `ToFace` target face, typed Revolve body-edge axis,
fillet/chamfer edges, and shell open faces. For a plain `Blind` extrude `inputs[]`
is empty. A legacy Revolve axis keeps `inputs: []`; a typed body-edge axis owns
exactly `inputs[0]`, which is the same `params.axis.edgeRef` evidence.

Corrected 2026-07-26 — see [Changelog](#14-changelog). The earlier prose here
described a `SketchRegion` semantic ref in `inputs[]` that no layer has ever
produced or consumed.

**Revolve** (`op.revolve`) — field names from OneCAD-CPP `RevolveParams`.

```json
// inputs: [] for sketch-line/legacy-edge axes; typed body-edge axis echoes edgeRef at inputs[0]
// params
{
  "sketchId": "sk_1",
  "regionId": "r_ac127d8846949…",
  "regionIdentityVersion": 3,
  "angleDeg": 360.0,
  "axis": { "kind": "sketchLine", "sketchId": "sk_1", "lineId": "e1" },
              // axis.kind ∈ "sketchLine" {sketchId,lineId} | "edge" {bodyId,edgeId,edgeRef?} | "none"
  "booleanMode": "NewBody",       // NewBody | Add | Cut | Intersect
  "targetBodyId": ""
}
```

A legacy edge axis is exactly `{kind:"edge",bodyId,edgeId}` and remains
byte-compatible. New edge-axis records add `edgeRef`, an ordinary versioned
semantic reference (`{primary:{bodyId,elementId,kind:"edge"},intent:{version,…},anchor}`),
and duplicate that object at `inputs[0]`. When `edgeRef` is present it is
authoritative: the worker resolves it through the partition/descriptor ladder and
never falls back to the snapshot-ordinal `edgeId`. Its `primary.bodyId` MUST equal
the legacy `bodyId`; a malformed, foreign, or curved axis is named refused, while
missing/deleted/ambiguous typed edges surface `NeedsRepair` with refId
`<opId>.input0`. Repair writes `edgeRef.primary` and the legacy `bodyId`/`edgeId`
together. This is additive; old readers may continue to use the legacy pair.

**Fillet** (`op.fillet`) and **Chamfer** (`op.chamfer`) — split ops sharing the
OneCAD-CPP `FilletChamferParams` shape (`mode` distinguishes; radius doubles as
chamfer distance).

```json
// Fillet params
{ "mode": "Fillet", "radius": 2.0, "edgeIds": ["el_…14", "el_…15"],
  "chainTangentEdges": true, "tangentClosureVersion": 1 }
// Chamfer params (equal-leg)
{ "mode": "Chamfer", "radius": 1.0, "edgeIds": ["el_…14"],
  "chainTangentEdges": true, "tangentClosureVersion": 1 }
// Chamfer params (two-distance, 2026-08-03 — Chamfer only, optional + skip-none)
{ "mode": "Chamfer", "radius": 1.0, "distance2": 2.5, "edgeIds": ["el_…14"],
  "chainTangentEdges": true, "tangentClosureVersion": 1 }
```

`edgeIds` entries are TopoKeys (snapshot-scoped) or `ElementId`s; the worker
resolves each through the ladder ([§10](#10-resolution-ladder)). The `inputs[]`
array carries the corresponding semantic refs (one per edge) supplying descriptor
+ anchor evidence.

Fresh records store the full OCCT-authoritative contour closure returned by
`PrepareEdgeOp`, use Rust-minted `ElementId`s in `edgeIds`, and carry
`tangentClosureVersion:1`. Its absence identifies a legacy record: regeneration
keeps the historical seed-only behavior, and re-edit MUST NOT add the field.
Version 1 regeneration requires the current contour union to equal the stored
closure exactly, deduplicates the build to one seed per contour, and refuses
drift with diagnostic `EDGE_OP_TANGENT_CLOSURE_CHANGED` (top-level `OP_FAILED`,
never `NeedsRepair`). Fillet⇄Chamfer swaps preserve the stored closure and
version. `chainTangentEdges:false` means exact picks; authoring refuses
`chainMismatch` when OCCT would expand beyond them.

- `distance2` (Chamfer only, optional, skip-none): asymmetric two-distance
  chamfer — `radius` is the distance on the FIRST adjacent face of each edge
  (the reference face the worker derives deterministically: the adjacent face
  with the smaller resolved face ordinal), `distance2` on the other
  (`BRepFilletAPI_MakeChamfer` two-distance form). Absent ⇒ equal-leg,
  byte-identical to every existing document. A Fillet MUST NOT carry it.
  **Type-flip interaction (FILLET-CHAMFER-UNIFY W3)**: the sanctioned
  Fillet⇄Chamfer `updateOperationParams` swap requires field-identical params —
  a Chamfer carrying `distance2` is NOT flippable to Fillet (the edit is
  rejected with the standard allow-list reason) until `distance2` is cleared.

Fillet execution is constant-radius only. `radius` MUST be finite and at least
`1e-3` mm. The worker MUST NOT clamp it or retry with a different radius. OCCT
contours are authoritative: duplicate requested edges on one contour are built
once, and every requested edge MUST belong to a known contour. A successful
contour MUST retain a constant law equal to the requested radius and publish
generated blend-face evidence.

Before publication, Fillet rejects null/invalid/non-single-solid/non-positive
results, self-interference, OCCT partial results, and `BadShape`. Shape audit
evidence records solid count, volume, self-interference count, and per-kind
input/output maximum tolerances. Tolerances are measured only in this milestone;
they are not mutated and no uncalibrated tolerance-growth rejection applies.
`Simulate()`/`Sect()` are characterization tools only and never production
acceptance gates. Kernel refusals retain top-level `OP_FAILED` or
`GEOMETRY_INVALID` and attach bounded `FILLET_*` diagnostics through §8.

**Hole** (`op.hole`) — machined hole on a planar face: simple / counterbore /
countersink, parametric as ONE feature. Added 2026-08-03 (WP-C T3).

```json
// inputs: [ semanticRef(host body), semanticRef(host face) ]
// params
{ "targetBodyId": "body_1",
  "face": { /* ElementRef: elementId/topoKey + descriptor/anchor evidence */ },
  "point": [25.0, 10.0, 30.0],
  "holeType": "counterbore",
  "diameter": 5.5, "depth": 20.0,
  "cbDiameter": 9.5, "cbDepth": 5.4,
  "csDiameter": null, "csAngleDeg": null }
```

- `point` — world-space hole center, frozen at authoring; MUST lie on the
  resolved face (worker re-projects onto the face plane and fails loudly past
  1e-3 mm). Axis = the face's inward normal (−outward) at `point`.
- `holeType` ∈ `"simple"` | `"counterbore"` | `"countersink"`; `depth` is a
  scalar or `null` = through-all (ray-extent + margin, ToNext-style bounded).
  `cb*` REQUIRED iff counterbore (cbDiameter > diameter); `cs*` REQUIRED iff
  countersink (csDiameter > diameter; csAngleDeg ∈ {82, 90, 100, 120}).
- Tool solid = drill cylinder (+ cb cylinder / cs cone seated at the face) cut
  from the host: lineage = `modified` on `targetBodyId`, nothing minted.
- Failures recoverable `OP_FAILED`: non-planar resolved face, point off face,
  drill deeper than through-all extent is fine (clamped = through), cb/cs
  invariant violations, OCCT boolean failure — all name the reason.
- Standard-size TABLES (M-series clearance, SHCS counterbores, DIN 74
  countersinks) are a FRONTEND concern — params always carry raw mm.

**Gear** (`op.gear`) — a fully parametric generated gear body. No sketch, no
host body: the op MINTS a new body from a typed parameter block and a
placement. Added 2026-08-14 (Gear Generator G1).

```json
// inputs: [ semanticRef(placement face) ]   — EMPTY for a frame placement
// params
{ "recipe": "involuteExternal",
  "placement": {
    "face":  { /* ElementRef: elementId/topoKey + descriptor/anchor evidence */ },
    "frame": null,
    "point": [25.0, 10.0, 30.0]
  },
  "involuteExternal": {
    "teeth": 20, "module": 2.0, "height": 5.0,
    "pressureAngleDeg": 20.0, "shift": 0.0,
    "helixAngleDeg": 0.0, "doubleHelix": false,
    "propertiesFromTool": false, "undercut": false,
    "backlash": 0.0, "clearance": 0.25, "head": 0.0,
    "sampleCount": 20,
    "axleHole": false, "axleHoleDiameter": 10.0,
    "offsetHole": false, "offsetHoleDiameter": 10.0, "offsetHoleOffset": 10.0
  }
}
```

- `recipe` selects which recipe block is non-null. **Every inactive recipe key
  MUST be spelled `null`**, never omitted — the same conditional-block contract
  `Hole`'s `cb*`/`cs*` fields carry, validated at all four trust boundaries
  (FSM → wire mapper → `edit/session.rs` → worker). `recipe` ∈
  `"involuteExternal"` in this version; the key set is versioned by this
  section, not by an addon (ADR-0002 — recipes are typed variants, never
  free-form strings, and no addon can register one).
- **Placement is EITHER a face or a frame, never both and never neither.**
  - `face` non-null: `point` is a world-space centre frozen at authoring and
    MUST lie on the resolved face — the worker re-projects onto the face plane
    and fails past **1e-3 mm**, the same fence `Hole` uses. The gear's axis is
    the face's INWARD normal (−outward) at `point`, so the body grows into the
    material side, and the op contributes exactly one `derive_inputs` entry.
  - `frame` non-null (`{origin, axis, xDir}`, all world-space): datum/world
    placement, frozen at authoring, no input refs, no reprojection. `xDir`
    fixes the gear's angular phase — which matters for a meshing pair, so it
    is carried explicitly rather than derived from the axis.
  - `point` is REQUIRED and is the body's centre in both modes; with a frame it
    MUST equal `frame.origin`.
- Angles are DEGREES with a `Deg` suffix, matching `Hole`'s `csAngleDeg`.
  Lengths are mm. `clearance`, `head` and `shift` are dimensionless
  coefficients (multiples of module) per gear-design convention, NOT lengths.
- **Lineage: `NewBody` mint.** `body_<opId>` (decision D1), one `created` body
  event, and an **empty element-map delta** — the op tracks no pre-existing
  element, so `apply_history` is not involved. Never `modified`; a `Gear` op
  has no target body to modify.
- **Referenceability is deliberately narrow.** Only the minted body's root id,
  the placement input, and (when the corresponding parameter is on) the BORE
  cylindrical faces carry referenceable identity. **Tooth flanks, tips, roots
  and fillets are NOT referenceable and MUST NOT be promoted**: their count and
  identity change with `teeth`, so a descriptor bound to "tooth 7's flank"
  is the silent-wrong-bind scenario §10's ladder exists to prevent. The worker
  refuses such a promotion by name rather than minting an id that will
  mis-resolve after the next parameter edit. A parameter edit that changes
  tooth count is an ordinary param edit (record identity preserved); downstream
  refs bound to the placement or a bore survive, and refs to anything else on
  the gear were never allowed to exist.
- **Publication: `single_solid_policy("Gear", TierB)`** — the FULL audit,
  including self-interference via `BRepAlgoAPI_Check`. A generated profile is
  exactly where a bad parameter combination produces a plausible-looking but
  self-intersecting solid, so this op does not take the lighter tier.
- Failures are recoverable `OP_FAILED` / `GEOMETRY_INVALID` naming the
  parameter at fault: `teeth` below 3, non-positive `module`/`height`,
  `pressureAngleDeg` outside (0, 90), a shift/clearance combination with no
  valid tooth depth, a bore that reaches the tooth roots, a flank that will not
  interpolate at the requested `sampleCount`, or a profile that will not close.
  A stale/unresolvable placement ref is `NeedsRepair`, never `Err`.
- **`helixAngleDeg` ≠ 0 and `doubleHelix` = true are `UNSUPPORTED`** in this
  version (§8) — helical and herringbone gears need the Frenet sweep
  infrastructure scheduled for a later phase. The fields are present and
  round-trip so the payload does not change shape when that lands, the same
  forward-compatibility posture `Loft`/`Sweep` carry as `KnownOperation`
  variants.
- `sampleCount` is an ACCURACY knob (spline samples per curve segment), not a
  geometric parameter: it changes the fidelity of the flank approximation and
  therefore the body's topology, so it participates in the plan hash like any
  other parameter.

**Boolean** (`op.boolean`) — standalone body-body boolean. Field names from
OneCAD-CPP `BooleanParams` (`operation` ∈ Union/Cut/Intersect; distinct from the
`booleanMode` fused into feature ops).

```json
// inputs: [ semanticRef(target body), semanticRef(tool body) ]
// params
{ "operation": "Union", "targetBodyId": "body_1", "toolBodyId": "body_2" }
```

`operation` ∈ `Union` | `Cut` | `Intersect`.

- A zero-solid `Cut` or `Intersect` is a recoverable `OP_FAILED`: the target and
  tool remain intact, no body lifecycle event or mesh is emitted, and the caller
  may revise the operation. Complete-consumption deletion is not implicit in this
  standalone operation. It carries diagnostic `BOOLEAN_EMPTY_RESULT`
  (`stage:"publish"`) with evidence
  `{boolean:{operation,targetBodyId,toolBodyId,solidCount:0}}`, so a caller routes
  on the CODE rather than on message text — the §8 top-level value is the generic
  `OP_FAILED` every Boolean failure shares. Cross-track fixture:
  `protocol/fixtures/boolean_empty_refusal.ndjson`.

**Shell** (`op.shell`) — hollow a body, removing (opening) selected faces. Field
names from OneCAD-CPP `ShellParams`. Added M6a (see the [Changelog](#14-changelog)).

```json
// inputs: [ semanticRef(face) per open face — kind "face" ]
// params
{ "thickness": 2.0, "targetBodyId": "body_1",
  "openFaces": ["el_…7c", "el_…8d"],
  "faces": [
    { /* typed ElementRef for el_…7c: descriptor + anchor evidence */ },
    { /* typed ElementRef for el_…8d: descriptor + anchor evidence */ }
  ] }
```

- `thickness` is the (positive) wall thickness; the worker offsets **inward**
  (`BRepOffsetAPI_MakeThickSolid::MakeThickSolidByJoin(target, removed,
  −thickness, …)`, OneCAD-CPP parity). `thickness < 1e-3` ⇒ recoverable
  `OP_FAILED` ("Shell thickness too small").
- `openFaces` remains the ordered bare-`ElementId` compatibility field. `faces`
  carries the corresponding typed face refs (descriptor + anchor evidence).
  **Lockstep is normative:** a non-empty `faces` array has the same length and
  order as `openFaces`, and each `faces[i].primary.elementId` equals
  `openFaces[i]`; every typed primary is kind `face` and its `bodyId` equals
  `targetBodyId`. A mismatch fails closed before geometry. New authoring MUST
  persist non-empty `faces` and derive `inputs[]` from that same array.
- Absent or empty `faces` is accepted only as legacy compatibility and retains the
  historical bare-id resolution path. A reader MUST NOT invent descriptor/anchor
  evidence for such a record. Either path resolves on the predecessor snapshot
  through the partition binding and shared ladder ([§10](#10-resolution-ladder));
  unresolved or ambiguous ⇒ **NeedsRepair** ([§9](#9-needsrepair-payload)), never
  a guessed face. The result **replaces** the shelled body (id preserved; OCCT
  history folds into its partition).

**OffsetFace** (`op.offsetFace`) — Shapr3D-style direct-modeling face offset:
selected face(s) move along their surface normals, adjacent faces extend/trim
(`BRepOffset_MakeOffset` per-face `SetOffsetOnFace`, mode Skin, boolean
Intersection=false, join `GeomAbs_Intersection`). Always MODIFIES `targetBodyId`
in place — never NewBody, never a body fan-out (>1 output solid ⇒ recoverable
`OP_FAILED`). V1 scope: planar + cylindrical operative faces. Added 2026-08-06.

```json
// inputs: [ semanticRef(face) per faceIds entry, in order; + semanticRef(opposite face) LAST iff distanceType == "Total" ]
// params
{ "faceIds": ["el_…a1", "el_…b2"],
  "distance": 2.5,
  "distanceType": "Offset",       // Offset | Total | Radius | Diameter (default Offset)
  "chainTangentFaces": true,      // default true — authoring metadata (see below)
  "oppositeFaceId": "el_…c3",     // Total only, optional + skip-none
  "targetBodyId": "body_1" }
```

- `faceIds` mirrors Fillet's `edgeIds` discipline: entries are `ElementId`s (or
  snapshot-scoped TopoKeys pre-promotion), and `inputs[]` carries the
  corresponding TYPED semantic refs (descriptor + anchor evidence, one per face,
  same order) — the ladder ([§10](#10-resolution-ladder)) resolves each;
  unresolved/ambiguous ⇒ **NeedsRepair** ([§9](#9-needsrepair-payload)), never a
  guess. **Slot order is NORMATIVE**: operative faces in stored order, then the
  Total opposite face (when present) LAST — the Rust repair paths
  (`InputPath::OffsetFaceFace{index}` / `OffsetFaceOpposite`) and the frontend
  `inputPathFor` table mirror it.
- `faceIds` is the FULL FROZEN operative set — picked faces PLUS the tangent
  chain, expanded once at authoring by `PrepareOffsetFace`
  ([§7.6](#prepareoffsetface)) and persisted. The worker NEVER re-expands at
  regen (an upstream edit must not silently widen/narrow the operative set);
  `chainTangentFaces` is retained as authoring metadata for re-edit UX.
  **Kernel constraint (spike-characterized)**: `BRepOffset_MakeOffset`
  auto-propagates the offset across G1-tangent junctions, so a resolved
  operative set whose tangent closure (G1 at ≈1e-4 rad via
  `BRepLib::ContinuityOfFaces`) contains non-member faces ⇒ recoverable
  `OP_FAILED` naming the missing faces — the kernel cannot hold them fixed.
- `distance` is the USER's value, interpreted per `distanceType`; the per-face
  signed kernel offset `d` is derived EVERY regen from current upstream geometry:
  - `Offset`: `d = distance` (signed; positive = along the topological outward
    normal = grows material). Multi-face allowed; the ONLY type valid for a
    multi-face set that is not a coaxial cylindrical closure.
  - `Radius` / `Diameter` (cylindrical only): `σ = sign(n_out·r̂)` derived
    geometrically (boss +1, hole −1; requires `|n_out·r̂| ≈ 1`);
    `d = σ(distance − R)` for Radius, `d = σ(distance/2 − R)` for Diameter.
    Valid for ONE face or a coaxial equal-radius same-σ cylindrical set.
    Preflight `R + σd > tol` — the OCCT negative-radius inside-out result is
    never allowed.
  - `Total` (single planar face, chain OFF, `oppositeFaceId` present):
    thickness `t = n·(p_sel − p_opp)` against the PERSISTED opposite face
    (re-resolved verbatim each regen, never re-discovered); `d = distance − t`.
- `|d| ≤ tol` ⇒ identity no-op SUCCESS (body unchanged, `modified` event).
- Validity gate beyond `IsDone` + `BRepCheck`: exactly one solid, positive
  finite volume, no self-interference, AND semantic postconditions (each
  operated plane moved by exactly `d`; each operated cylinder coaxial at the
  predicted radius). Any miss ⇒ recoverable `OP_FAILED`; the result is never
  published. Values are NEVER clamped.
- Lineage: `modified` on `targetBodyId` (+ `rankKey`); element history folds via
  the offset image (`OffsetFacesFromShapes`/`OffsetEdgesFromShapes` — the public
  `Generated`/`Modified` lists are empty for faces, spike-characterized) through
  the partition; image gaps fall to the descriptor ladder.

**LinearPattern** (`op.linearPattern`) — `count` copies of a source body translated
`spacing` along `direction`. Field names from OneCAD-CPP `LinearPatternParams`
(the C++ flat `dirX/Y/Z` is a single `direction: [x,y,z]`). Added M6a.

```json
// inputs: [ semanticRef(source body) ]
// params
{ "sourceBodyId": "body_1", "direction": [1,0,0], "spacing": 40.0, "count": 3,
  "fuseResult": false, "resultPolicyVersion": 2 }
```

- `count` is an integer `[2,128]`; `|spacing| ≥ 1e-9`; `direction` non-zero
  (normalized). Instance `i ∈ [1, count)` is translated `direction·spacing·i`.
- **Absent `resultPolicyVersion` is frozen V1:** `fuseResult:true` fuses source +
  instances into legacy aggregate body `body_<opId>`; `false` gathers the same into
  one compound body. Source remains unchanged. This compatibility path permits its
  historic aggregate/compound result and does not apply V2 connected-solid policy.
- **`resultPolicyVersion:2`:** source is instance zero. With `fuseResult:false`, it
  emits exactly `count−1` created children `body_<opId>:<k>`, where child `k` is
  transformed instance `k+1`; source emits no lifecycle event and stays unchanged.
  With `fuseResult:true`, the connected fused result modifies `sourceBodyId` in place;
  a disconnected result refuses `PATTERN_DISJOINT_RESULT`. New child bodies inherit
  source body visibility/color, but not source face identities or face colors. New
  authoring emits only integer `2`. Other numeric versions load and re-save verbatim,
  but execution refuses recoverably with `UNSUPPORTED_PATTERN_RESULT_POLICY_VERSION`.
  Re-edit preserves both legacy absence and stored `fuseResult`. Count reduction removes only tail children;
  retained child IDs remain stable, and suppression removes children without
  modifying source.

**CircularPattern** (`op.circularPattern`) — `count` copies rotated about an axis.
Field names from OneCAD-CPP `CircularPatternParams` (flat `axisX/Y/Z` +
`axisDirX/Y/Z` → `axisOrigin` + `axisDirection`). Added M6a.

```json
// inputs: [ semanticRef(source body) ]
// params
{ "sourceBodyId": "body_1", "axisOrigin": [0,0,0], "axisDirection": [0,0,1],
  "angleDeg": 360.0, "count": 3, "fuseResult": false, "resultPolicyVersion": 2 }
```

- `count` is an integer `[2,128]`; `axisDirection` non-zero. The per-instance step angle is
  `angleDeg / count` (OneCAD-CPP parity — divides by `count`, **not** `count−1`);
  instance `i ∈ [1, count)` is rotated `step·i` about `(axisOrigin, axisDirection)`.
- `fuseResult`, `resultPolicyVersion`, child ordinal mapping, and lineage are identical
  to LinearPattern.

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

**TransformBody** (`op.transformBody`) — rigid placement of one or more bodies:
translate + rotate about a frozen pivot, optionally as copies. The light
multi-part "position parts for fit-check" primitive — parametric, ONE cumulative
record per placement intent, re-edited in place (never a stack of nudges).
Added 2026-08-02 (WP-B W0).

```json
// inputs: [ semanticRef(target body) × N ] — mirrors params.targets
// params
{ "targets": ["body_1", "body_2"],
  "translate": [10.0, 0.0, 5.0],
  "rotate": { "center": [0,0,0], "axis": [0,0,1], "angleDeg": 90.0 },
  "copy": false }
```

- **Evaluation order is normative**: `X' = T ∘ R(center, axis, angleDeg) · X`
  (rotate about the pivot first, then translate). Each `translate` component and
  `angleDeg` is a scalar (expression-capable); `center`/`axis` are plain Vec3.
- `center` is **frozen at first authoring** (the targets' combined bbox centre
  at that moment) and never re-derived — re-edits recompose against the stored
  pivot, so repeated edits are exact (no drift) and the stored form stays
  canonical.
- Validation: `axis` non-zero when `angleDeg ≠ 0`; all components finite;
  `targets` non-empty, no duplicates; a zero motion (identity) is legal and a
  geometric no-op.
- **Lineage**: `copy: false` ⇒ every target emits `modified` — the FIRST
  body-level op with modify lineage; the worker keeps its
  `BRepBuilderAPI_Transform` history alive so the element-map partition rebinds
  every tracked element at level 1 (no descriptor scoring), and additionally
  applies the same rigid motion to each entry's stored `anchor` world point
  (`apply_placement` — anchors are physical points that move WITH the body).
  `copy: true` ⇒ sources preserved (no event) and the copies mint under the §2
  N-body rule: one target ⇒ plain `body_<opId>`, N > 1 ⇒ `body_<opId>:<k>` with
  `k` = the target's index in `targets`.
- **§9/§10 edit-safety gate (V1, normative until anchor pick-frame
  compensation ships)**: level-1 partition rebinds stay EXACT under a rigid
  transform, so ops authored after a transform resolve honestly. Descriptor
  SCORING, however, compares frozen record evidence against moved geometry — a
  congruent-feature decoy at the anchor's new location can outscore the true
  element (a silent wrong bind, the H5-B class). Therefore **editing the params
  of, suppressing, or un-suppressing a `TransformBody` seeds `NeedsRepair` on
  every downstream ref whose producing body is in the transform's target
  lineage** — those refs re-resolve only through the repair flow, never by
  silent scoring. The seeding rides the same undo entry as the edit, so undo
  restores the previous binding state exactly. Deterministic-loud beats
  silent-wrong.
  `body_<opId>` (NewBody lineage; source preserved). Empty `elementMapDelta`.

**PlaceComponent** (`op.placeComponent`) — instantiate a Component Library
entity as a first-class placed instance (spec §3.1). New v2 op, no
OneCAD-CPP analogue. Added 2026-08-12 (Component Library WP-0.2/WP-1.2).

```json
// inputs: []  — ALWAYS empty, even when `mate` is set (P3 WP-3.1; see the
//          `mate` bullet below and the 2026-08-13 §14 entry for why)
// params
{ "componentId": "onecad.std.iso4762",
  "componentVersion": "1.0.0",
  "componentRevision": "sha256:9f2c…",
  "params": { "thread": "M6" },
  "source": { "kind": "generator", "generatorId": "iso4762", "generatorVersion": 1 },
  "mate": { "selfAttachment": "shank_axis",
            "target": { "primary": {"bodyId":"body_1","elementId":"el_…","kind":"face"},
                        "anchor": {"worldPoint":[10,5,0]} },
            "kind": "concentric", "flipped": false,
            "selfFrame": { "origin":[0,0,10], "z":[0,0,1], "x":[1,0,0] } },   // optional
  "placement": { "translate": [10.0, 5.0, 0.0],
                 "rotate": { "center": [0,0,0], "axis": [0,0,1], "angleDeg": 0.0 } } }
```

- **Lineage: mints a NewBody** (`body_<opId>`), `modified` on nothing — a
  placed component is a first-class instance, never a copied-in body (spec
  §3). A component resolves to exactly ONE solid in v1 (spec §9,
  `single_solid_policy`).
- `source.kind` ∈ `generator` | `embedded` | `document` — **all three
  implemented** (WP-3.2). An unknown kind refuses recoverably with
  `UNSUPPORTED`.
  - `generator` — `{generatorId, generatorVersion, params}`. Table-driven per
    thread size as of WP-2.1 (spec §6), and DISPATCHED PER FAMILY on
    `generatorId` as of WP-A1, extended with the non-fastener families in
    WP-F2: `iso15` · `iso4014` · `iso4017` · `iso4032` · `iso4762` ·
    `iso7089` · `iso7093` · `iso7380` · `nema17` · `nema23`. An unregistered
    `generatorId` fails recoverably with `OP_FAILED`, naming the registered
    ids — it does **not** fall back to any family (before WP-A1 every id built
    an ISO 4762 socket cap, which is the silent substitution spec §0
    invariant 4 forbids). `params.thread` / `.length` / `.thread_detail` are
    the fastener free params; families with no external thread (nuts, washers,
    bearings, motors) ignore `thread_detail`, and families with no length
    (nuts, washers, bearings) ignore `length`. A family keyed by something
    other than a thread reads its OWN string param — `iso15` reads
    `params.code` (the bearing code) — and every string under `params` reaches
    the generator verbatim, so a new key is not a wire change. An unknown
    `thread` / `code` for the addressed family fails loudly with the known
    values; an ABSENT one takes the family's documented default (`M6`, `608`,
    and the frame's own body length for a motor).
  - `embedded` / `document` — a BAKED solid, carried as a content-addressed
    blob in the placing document's own `imports/` section:
    `{sha256, codec, brepFormat}`, plus `documentSha256` (the frozen authoring
    document, provenance only) on `document`. The bytes reach the worker as a
    **wire-only, NON-hashed `source.path`** Rust injects from its materialized
    blob — the same mechanism, and the same "an unmaterialized blob lowers an
    EMPTY path so only THAT step fails" rule, as `ImportStep` (§7.3). `codec` /
    `brepFormat` carry the identical meaning and the identical
    version-pin refusal.
    A `document` source may also carry `params` — the free-parameter values its
    blob was RE-BAKED at (WP-F1.3, spec §3.2). They are provenance and UI state,
    never a regen input: the geometry they produced is already in `sha256`, the
    re-bake happens Rust-side on its own worker, and this worker ignores them
    exactly as it always has.
  - The two blob kinds are read by the SAME reader and differ only in the
    record's provenance fields; both must resolve to **exactly one solid**
    (spec §9), and a blob carrying more is refused rather than reduced.
- `mate` is optional; absent ⇒ dropped in free space, positioned by
  `placement` alone. When present, `target` is a full semantic ref so the
  resolution ladder can re-resolve it after upstream edits — this is what
  makes the mate PERSISTENT (spec §5.5). **Re-seated by the worker on every
  regen since P3 WP-3.1**: the target is re-resolved mid-`ExecutePlan` (so it
  sees same-tick geometry), the seat recomputed from the mate kind + resolved
  frame + flip, and the new transform echoed back on `planStep.matePlacement`
  (§7.2) for Rust to persist. Unresolvable ⇒ a `NeedsRepair` item and the
  frozen `placement` stands — the component ALWAYS publishes, never drops,
  never silently moves. `mate.target` deliberately does NOT ride in `inputs[]`:
  the generic `resolve_input_refs` pre-flight treats an unresolved input as
  blocking, which would publish ZERO bodies for a component whose target was
  deleted — the opposite of the rule above.
- `mate.selfFrame` is **optional** (Component Library WP-F1.1, spec §2.1/§5) —
  the component-LOCAL basis `selfAttachment` seats from, frozen into the record
  at authoring out of the package's `[attachments].<key>.frame`. Three plain
  Vec3s: `origin` is the local point that lands on the target's seat point, `z`
  the local direction aligned to the target axis / outward normal, `x` the roll
  reference about `z`. Right-handed — `y` is DERIVED (`z × x`) and never sent,
  so a left-handed basis cannot be expressed. Both axes are already orthonormal
  when they reach the worker (Rust normalizes and re-orthogonalizes at manifest
  parse); a record carrying otherwise is re-orthogonalized rather than trusted.
  - The seat solve composes it as **`M = S ∘ F⁻¹`**: `S` is the seat transform
    from (`kind`, resolved frame, `flipped`), `F` maps component-local identity
    onto the attachment frame. So the ATTACHMENT POINT lands on the target,
    not the component's model origin.
  - The regen re-seat's stand-in for the cursor becomes **the attachment
    point's current world position** (`placement` applied to `selfFrame.origin`)
    rather than the raw `placement.translate`. Anchoring on the body origin
    would re-subtract the frame offset on every regen and walk the component
    along the axis a frame-length per tick; anchoring on the attachment makes
    an unchanged re-seat an exact fixed point. Identical for an absent frame,
    where the two are the same point.
  - **ABSENT ⇒ the identity frame**, and the worker takes an early return
    through the pre-WP-F1.1 arithmetic — every document written before this
    lands re-seats byte-identically. It is frozen, never re-read from the
    library on regen, for the same reason `source` is: spec §4 requires a
    placement to re-seat with the library deleted, and a package revision that
    moves its attachment must not silently move already-placed instances (that
    is an explicit `replaceComponent`, which re-freezes).
- `placement` — SAME normative order as TransformBody: `X' = T ∘ R(center,
  axis, angleDeg) · X`. `rotate` defaults to the identity rotation when
  absent.
- `componentRevision` is the package content hash at PLACE TIME (spec §4);
  the library re-verifies it on regen and surfaces `NeedsRepair` on mismatch
  at the app-crate layer (WP-1.3) — the worker itself never touches the
  library, only the frozen `source`/`placement` this op already carries.

**DetachComponent** (`op.detachComponent`) — drop a placed component's
library identity, keeping its cached geometry as an ordinary body: "the
honest break link" (spec §3.4). New v2 op, no OneCAD-CPP analogue. Added
2026-08-12 (Component Library WP-1.2).

```json
// inputs: []  — no mate, no identity, no topological dependency at all
// params
{ "source": { "kind": "generator", "generatorId": "iso4762", "generatorVersion": 1 },
  "placement": { "translate": [10.0, 5.0, 0.0] } }
```

- **Same `source`/`placement` shape as `PlaceComponent`, minus
  `componentId`/`componentVersion`/`componentRevision`/`mate`** — spec §3.4:
  "after detach, no `component_*` fields remain; the op becomes inert
  provenance." A `generator` source re-runs deterministically, so the result
  is indistinguishable from a static copy.
- **This is an in-place edit at the SAME `RecordId`** — the sanctioned
  op-type swap `PlaceComponent → DetachComponent` (mirrors the existing
  Fillet⇄Chamfer swap precedent), applied via the ordinary
  `UpdateOperationParams` edit command, not a new record. The reverse
  (re-attaching a library identity to a detached body) is NOT sanctioned —
  one-directional, matching the "honest break link" framing.
- **Lineage: mints a NewBody** (`body_<opId>`) — identical publish shape to
  `PlaceComponent` (the same `body_<opId>` the swap's `RecordId` already
  produced, so the `BodyId` is unchanged across the swap).

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
Opens a drag gesture against a specific sketch revision, and declares WHAT the
pointer grabbed.

```json
// req.args
{ "sketchId": "sk_1", "sketchRevision": 4, "gestureId": 51, "solverPolicyHash": "3e9a…",
  "drag": { "kind": "radius", "entity": "e7", "grab": [31.2, 4.0] } }
// result
{ "gestureId": 51, "ready": true }
```

The optional `drag` object is `{ "kind"?, "entity"?, "role"?, "grab"?,
"pointId"? }` and names the gesture's **target kind**:

| `kind` | `entity` | `role` | `grab` | resolves to |
|---|---|---|---|---|
| `point` | any (or `pointId`) | `start`\|`end`\|`center`\|`p0`\|`p1` | ignored | one point handle |
| `arcEnd` | `Arc` | `start`\|`end`, REQUIRED | ignored | that endpoint (a real solver point, §7.3) |
| `radius` | `Circle`\|`Arc` | ignored | optional | the curve's radius parameter |
| `entityBody` | any owning ≥1 point | ignored | RECOMMENDED | every point the entity owns |

**An absent `kind` means `point`.** The pre-existing forms — `{"drag":
{"pointId": …}}` and a bare top-level `pointId` — keep their exact meaning and
are resolved FIRST; when both `pointId` and `entity` are present **on the
`point` path** (kind absent or `"point"`), `pointId` wins. A non-`point` kind
ignores `pointId` entirely — honouring it would silently degrade the gesture to
a point drag, the exact failure mode the unknown-kind rule below forbids. An
**unknown** `kind` MUST fail the request (`OP_FAILED`) and MUST NOT
degrade to `point`: degrading would move a handle the user never grabbed.

A `drag` that names no resolvable target is `REF_UNRESOLVED` — an
`entity`/`role` pair naming no point, `arcEnd` on a non-arc or on an arc with
derived (non-entity) endpoints (§7.3), `radius` on a non-curve, `entityBody` on
an entity that owns no point.

`grab` is the pointer-down position in sketch-plane coordinates. The worker
derives the per-kind offset from it **ONCE, at `BeginGesture`**, so a gesture
cannot drift as it is dragged:
* `entityBody` — the drag delta is `target − grab`, applied to each owned
  point's `BeginGesture` pose. An absent `grab` anchors on the first owned point
  in handle order (`center` < `start` < `end`, `p0` < `p1`), which teleports that
  point to the cursor.
* `radius` — the radial offset is `|grab − center| − radius`, both read at
  `BeginGesture`; absent ⇒ `0`.
* `point`/`arcEnd` — `grab` is IGNORED: the handle teleports to the cursor,
  byte-identical to the behaviour before the kinds existed.

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
  "curves": { "e7": { "radius": 12.5 } },                            // CHANGED curve members only
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

`positions` reports moved POINTS only, which is not the whole result of a drag:
a `radius` gesture moves no point at all, an `arcEnd` gesture reshapes the arc's
radius and angles, and a `Tangent` propagates even a plain `point` drag into a
neighbouring curve's radius. The additive `curves` channel carries exactly that:

```json
"curves": { "e7": { "radius": 12.5 }, "e9": { "startAngle": 0.3926, "endAngle": 1.9634 } }
```

It maps a wire entity id to the members of that curve that **CHANGED**, under
the same incremental-vs-baseline discipline as `positions`: `SolveDrag` reports
the delta since the previous report for that gesture, `EndGesture` the delta
since `BeginGesture`. An entity with nothing changed is omitted; an absent
`curves` parses as `{}` (optional/additive — all parsers tolerate the missing
key). It is emitted for **every** `drag.kind`, not only `radius`. An `Ellipse` is
never reported: it is not registered with the solver (see the deviation above),
so no drag can move its parameters. Angles are radians CCW from +X and pair with
`ccw` exactly as §7.6 defines.

Three degenerate guards are NORMATIVE for both verbs:
* **Radius floor.** A `radius` target is clamped at `MIN_GEOMETRY_SIZE`
  (0.01 mm) before it reaches the solver, so a drag across the center can never
  produce a zero or negative radius. It saturates at the floor and recovers on
  the way back out — the gesture stays live rather than failing.
* **Arc sweep floor.** An `arcEnd` step whose solved CCW arc EXTENT falls below
  `MIN_ARC_SWEEP` (1e-3 rad) — or above `2π − MIN_ARC_SWEEP`, the same collapse
  approached from the other side once angles normalize — MUST be REFUSED: the worker restores the
  pre-step pose (positions AND curves), answers `status: "partial"`, and leaves
  the gesture OPEN. A collapsed arc cannot be recovered by dragging further, so
  the state must never be entered.
* **`entityBody` targets are ADVISORY.** They are temporary, gesture-scoped
  drives and every committed constraint outranks them. An entity that converges
  away from its target because a constraint holds it is `status: "success"`
  reporting the SOLVED positions — lagging the cursor is the CORRECT answer
  there, and only non-convergence is `partial`.

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
  "curves": { /* final exact curve members, changed since BeginGesture */ },
  "sketchRevision": 5 }
```

`conflicting` follows the same precedence as `SolveDrag`: the final exact solve's
conflicts, else the gesture-fixed set diagnosed at `BeginGesture`. Optional/additive
(absent ⇒ `[]`). `curves` is the same additive channel `SolveDrag` defines above,
baselined at `BeginGesture` (absent ⇒ `{}`).

#### SketchRegions
Computes closed profile regions for a sketch (for extrude/revolve selection and
preview fill).

```json
// req.args
{ "sketchId": "sk_1" }
// result
{
  "sketchId": "sk_1", "sketchRevision": 5, "regionIdentityVersion": 3,
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

  Version 2 (frozen) hashes a cell with holes or an intersected source entity as one canonical UTF-8
  member string, then winding byte `0`, through the same FNV/rendering rule. The
  string is `cell-v2|outer{L}|holes{H...}`:

  - `L` is the lexicographically smallest cyclic rotation of the oriented loop's
    length-prefixed tokens (outer normalized CCW; holes CW).
  - A token is the mapped base wire UUID, followed for a fragment by its curve
    kind and start/end parameters normalized to the authored base range at 1e-9,
    then `:f` or `:r` for traversal. Tessellation indices are never identity.
  - Each hole loop is canonicalized independently; the resulting hole strings
    are sorted and length-prefixed before concatenation.

  Version 3 hashes `cell-v3` from every ordered fragment's analytic provenance
  and curve-domain-normalized source interval. Split proximity is measured as
  physical curve distance, including shortest distance across a periodic seam;
  raw curve parameters are not compared to a model-length tolerance. Supported
  Line/Circle/Arc/Ellipse fragments remain analytic. Missing, mismatched,
  non-finite, non-positive, or discontinuous provenance refuses the profile.

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

- **Region identity version.** `SketchRegions` emits
  `regionIdentityVersion: 3` for new authoring. A persisted V1 or V2 record keeps
  that version through load/save/re-edit/undo/reopen; neither bytes nor lookup
  behavior migrate implicitly. V2 keeps its frozen raw-parameter behavior. V3
  requires one exact `cell-v3` id and fails closed. An absent persisted version
  is V1 and keeps its legacy first-region fallback. A future explicit migration
  may proceed only when every old region maps uniquely; ambiguity aborts it.

- **Exact fragment BReps.** Supported Line/Circle/Arc/Ellipse fragments are
  intersected with `Geom2dAPI_InterCurveCurve`, then built as trimmed analytic
  BRep edges with shared endpoints. Tessellation remains only for planar walk,
  containment, and preview fill; it is never a committed fragment edge. Positive
  overlap/coincidence refuses; point tangencies collapse to one split. `previewTriangles` is a
  tessellation in **both** cases (region area/fill for an ellipse is a sampled
  polygon and therefore slightly under-reports the analytic π·a·b); it is
  display/selection evidence, never committed topology.

### 7.5 Element identity

#### QueryMassProperties

Read-only exact kernel mass properties for one body (GProp over the head
snapshot — no fence, no scratch, no session mutation; addressed like the other
§7.5 read verbs). Added 2026-08-02 (WP-C measure upgrades).

```json
// req.args
{ "bodyId": "body_3" }
// result
{ "volume": 15000.0, "surfaceArea": 4300.0,
  "centroid": [10.0, 5.0, 7.5],
  "principalMoments": [1.2e6, 3.4e6, 3.9e6],
  "principalAxes": [[1,0,0],[0,1,0],[0,0,1]] }
```

- Units: mm³ / mm² / mm; moments are volume-integrals (mm⁵) about the
  centroid — density-free (the app has no material system; a consumer
  multiplies by density). `principalAxes` rows are unit vectors, right-handed,
  paired with `principalMoments` in order.
- Unknown `bodyId` ⇒ `REF_UNRESOLVED` error resp (recoverable).

#### QueryBodyTopology

Read-only exact BRep topology counts for one body. This is deliberately separate
from `Tessellate`: faceting LOD must never change a corpus oracle.

```json
// req.args
{ "bodyId": "body_3" }
// result
{ "solidCount": 1, "faceCount": 6 }
```

- Counts are `TopExp` counts over the current head body BRep; no fence, scratch,
  session mutation, or identity minting.
- Unknown or absent `bodyId` ⇒ recoverable `REF_UNRESOLVED` error response.

#### AcquireElementIds
Promotes snapshot-scoped TopoKeys to persistent, globally-unique `ElementId`s
(**ID-on-demand**). ElementIds do **not** embed `BodyId`.

```json
// req.args
{ "snapshotId": 5012, "bodyId": "body_3",
  "picks": [ { "topoKey": "f:22", "anchor": { "worldPoint": [1,2,3], "surfaceUv": [0.5,0.5] } } ] }
// worker result: authoritative evidence; elementId is present only for an existing binding
{ "ids": [ { "topoKey": "f:22", "elementId": "", "bodyId": "body_3", "kind": "face",
             "descriptor": { … }, "anchor": { … } } ] }
```

A `snapshotId` that is present and does not equal the worker's current head
snapshot MUST be refused with `REF_UNRESOLVED` (a `TopoKey` is snapshot-scoped
evidence ([§9](#9-needsrepair-payload)), so promoting a pick taken against a
superseded snapshot would mint a persistent id for an arbitrary element); an
absent `snapshotId` is "no claim" and is resolved against the head.

`AcquireElementIds` is read-only. `elementId` is **minted by Rust**, not the
worker: the worker returns authoritative `topoKey → (bodyId, kind, descriptor,
anchor)` evidence and any already-installed id. Rust reuses that id or mints its
own, then MUST complete [`BindElementIds`](#bindelementids) against the echoed
head before a promotion command returns the id to the frontend. When Rust already
holds an id for the same stable element, it supplies that id to the bind step
(Invariant 1: an ElementId never changes because geometry changed).

#### BindElementIds

Internal Rust→worker completion of ID-on-demand promotion. It installs
Rust-minted ids into the authoritative current-head `ElementMapPartition`; it is
not a frontend authoring verb.

```json
// req.args
{ "snapshotId": 5012,
  "bindings": [
    { "elementId": "el_00000000000004a1", "bodyId": "body_3",
      "topoKey": "f:22", "kind": "face",
      "anchor": { … } }
  ] }
// result
{ "bound": [
    { "elementId": "el_00000000000004a1", "bodyId": "body_3",
      "topoKey": "f:22", "kind": "face" }
  ] }
```

- `snapshotId` is REQUIRED and MUST equal the worker head. A mismatch returns
  `REF_UNRESOLVED` with requested/head detail and changes nothing.
- The worker validates the whole batch against one locked published-head view:
  body and TopoKey exist there; TopoKey kind agrees with the OCCT shape kind;
  every `ElementId` is well formed; and neither the batch nor the head maps an id
  or topology element inconsistently. The worker recomputes the descriptor from
  that exact current-head shape; descriptor evidence is never trusted back from
  Rust. Validation failure changes nothing.
- Installation is atomic. Only after every entry validates does the worker
  publish every binding into the current head partition. Geometry,
  `snapshotId`, signatures, and meshes do not change. A subsequent
  `QueryElement(elementId)` or `ResolveRefs` on that head MUST observe the whole
  batch, never a prefix.
- The request is idempotent only for an exact existing identity binding: same
  snapshot, id, body, TopoKey, kind, and shape succeeds as a no-op and does not
  overwrite its stored descriptor/anchor. Reusing an id for different identity,
  binding one topology element to another id, or conflicting duplicate entries
  fails closed; it MUST NOT overwrite an existing mapping. The successful
  `bound[]` response preserves request order and exactly echoes each accepted
  `elementId`, `bodyId`, `topoKey`, and `kind`; Rust MUST validate that exact echo
  before treating promotion as committed.

Thus a promotion is `AcquireElementIds → Rust mint/reuse → BindElementIds` as one
Rust-side transaction. Rust inserts only the minted/reused id and forwards the
worker's body, TopoKey, kind, and anchor unchanged. It MUST NOT update its
promotion cache or return the promoted refs until the bind succeeds; a bind
refusal leaves the Rust cache and frontend unchanged. This establishes
REF-FRESH-1: a freshly promoted ref resolves directly and uniquely on the same
unchanged head.

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
    { "snapshotId": 5012, "revision": 44, "refId": "op_5.input0", "bodyId": "body_3",
      "outcome": "autoBind", "elementId": "el_…", "score": 0.94, "margin": 0.31 },
    { "snapshotId": 5012, "revision": 44, "refId": "op_5.input1", "bodyId": "body_3",
      "outcome": "needsRepair", "needsRepair": { /* §9 */ } }
] }
```

`outcome` ∈ `autoBind` | `needsRepair` | `unchanged`.
Every resolution carries the exact `snapshotId`, document `revision`, `refId`, and
the `bodyId` used to enumerate candidates when one exists. An echoed `bodyId` MUST
equal that ref's primary body. Body omission is allowed only for a non-promotable
`needsRepair` missing-body result with no candidates. A client MUST cache a
candidate set by `{revision, snapshotId, refId}` and MUST promote its TopoKeys only
against that echoed snapshot; a mismatch requires a fresh resolve, never ordinal reuse.

The echo is per-RESOLUTION and mandatory on every branch, `needsRepair` included — a
failed resolution still has to say which head it failed against. `snapshotId` is the
snapshot the ladder actually ran on (the request's when it names one, else the head)
and `revision` is the document revision that head last accepted; `bodyId` is present
only when a body was there to enumerate. **Rust MUST validate the echo before a
resolution is used**: request order preserved, one resolution per requested ref, and
`snapshotId` equal to the requested snapshot, valid revision evidence, and actual
worker provenance preserved — a resolution computed against another
snapshot, filed under the requested one, is precisely the stale-candidate mis-bind
this rule exists to prevent. `documentRevision` remains a Rust-owned advisory stamp
(D4), so the echoed `revision` is evidence about the engine's head, not a value the
client keys its own candidate cache by.

#### ClassifyElement

Read-only surface/curve classification of a picked face or edge, plus a
seatable geometric frame — the Component Library placement solver's
interactive hover query (WP-0.1; spec §5.2). Addressed like `QueryElement`
(`elementId`, or `{bodyId, topoKey}`), but **no `snapshotId`**: unlike
`QueryElement`'s pick-time addressing (Invariant 4), this verb is
continuously re-issued against a live drag gesture, so it always reads the
current head — the same choice `QueryMassProperties` makes. It does not
fence, prepare, accept, discard, or mint anything.

```json
// req.args
{ "bodyId": "body_3", "elementId": "el_…4a1" }   // or { "bodyId", "topoKey" }
// result — a planar face
{ "present": true, "kind": "face", "surfaceType": "plane",
  "frame": { "origin": [10.0, 5.0, 0.0], "normal": [0.0, 0.0, 1.0] } }
// result — a cylindrical face
{ "present": true, "kind": "face", "surfaceType": "cylinder",
  "frame": { "origin": [10.0, 5.0, 0.0], "axis": [0.0, 0.0, 1.0], "radius": 3.0 } }
// result — a circular edge (a hole rim)
{ "present": true, "kind": "edge", "curveType": "circle",
  "frame": { "origin": [10.0, 5.0, 0.0], "axis": [0.0, 0.0, 1.0], "radius": 3.0 } }
// result — absent
{ "present": false }
```

- `kind` ∈ `face` | `edge` | `other`. `surfaceType` ∈ `plane` | `cylinder` |
  `cone` | `sphere` | `torus` | `other` (present only when `kind === "face"`).
  `curveType` ∈ `line` | `circle` | `ellipse` | `other` (present only when
  `kind === "edge"`).
- `frame` is present only for the kinds a mate solver can seat against —
  plane, cylinder, line, circle — and absent for everything else (a torus
  face, an ellipse edge, `kind: "other"`). A plane frame carries `normal`; a
  cylinder/circle/line frame carries `axis` (never both on the same frame).
  `radius` is present only for cylinder and circle frames.
- A stale or absent reference resolves `{ "present": false }` — an ANSWER,
  not an error, matching `ProjectFaceBoundary`'s convention.
- Distinct from `QueryElement`'s descriptor: that one's `normal` is a face's
  surface normal at its UV midpoint, which is **not** an axis for a
  cylinder, and it carries no radius at all. `ClassifyElement` exists
  specifically for the frames a concentric/flush mate needs.

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
- A failed preview MAY carry the same bounded `diagnostics[]` as its candidate
  `ExecutePlan` step under `error.detail.diagnostics`. The arrays MUST be
  byte-equivalent for the same candidate. `error.code` remains the §8 taxonomy
  code; operation-specific `FILLET_*` values are diagnostic codes only.
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

#### PrepareEdgeOp

Read-only, snapshot-fenced Fillet/Chamfer authoring handshake. OCCT contour
membership is execution authority; the worker first verifies the shared
`EdgeChainer` gives the same closure, then returns deterministic edge evidence.
It mints no ids. Rust batch-promotes the entire accepted response atomically;
partial promotion fails the arm before any id/cache mutation. A fresh closure
must receive a successful final exact `PreviewOp` response before commit; timeout
is refusal, not approval.

The JSON below is the worker result. The Tauri command promotes that exact batch
against the echoed `snapshotId` before returning it to the frontend and adds
`elementId`, `bodyId`, and `kind:"edge"` to every accepted `edges[]` entry. A
head change between preparation and promotion rejects the whole command.

```json
// req.args
{ "snapshotId": 5012,
  "mode": "Fillet",
  "pickedEdges": [ { "bodyId": "body_3", "topoKey": "e:4" } ],
  "chainTangentEdges": true }
// accepted result
{ "snapshotId": 5012,
  "targetBodyId": "body_3",
  "edges": [
    { "topoKey": "e:4", "picked": true,
      "anchor": { "worldPoint": [1.0, 2.0, 3.0] }, "descriptor": { } },
    { "topoKey": "e:5", "picked": false,
      "anchor": { "worldPoint": [2.0, 2.0, 3.0] }, "descriptor": { } }
  ],
  "refusal": null }
```

- Each pick uses exactly one address: `{bodyId,topoKey}` or `{elementId}`.
- Refusals are successful answers with empty closure: `crossBody`,
  `unsupportedEdge`, or `chainMismatch`; they carry `{code,message,edges}`.
- Missing/stale `snapshotId` refuses; identical head + request gives a
  byte-identical ordinal-ordered response.

#### PrepareOffsetFace

Read-only OffsetFace authoring handshake — the first half of the
[`op.offsetFace`](#73-op-payload-schemas-vertical-slice) freeze. Given the
user's picked faces, it computes on a copy of the head: the G1 tangent-chain
closure (`BRepLib::ContinuityOfFaces ≥ G1` at `angleTol` ≈ 1e-4 rad, BFS over
2-manifold edges, output sorted by face ordinal), the Total opposite-face
candidate (fail-closed: full-footprint anti-parallel coverage + material-column
validation, exactly one passing candidate), and the current dimensions that seed
absolute distance types. Added 2026-08-06.

It does **not** fence, prepare, accept, discard, or — critically — **mint**:
the response carries snapshot-scoped TopoKeys + descriptor/anchor EVIDENCE only,
never `ElementId`s. Rust promotes the evidence through the snapshot-fenced
[`AcquireElementIds`](#acquireelementids) →
[`BindElementIds`](#bindelementids) transaction; the frontend then builds the
final persisted params and runs a final exact [`PreviewOp`](#previewop) with them
before commit. OffsetFace commits FAIL CLOSED on a missing/failed handshake
(op-specific strictness — the generic preview barrier's timeout-success is not
sufficient here).

```json
// req.args
{ "snapshotId": 5012,
  "pickedFaces": [ { "bodyId": "body_3", "topoKey": "f:22" },   // or { "elementId": "el_…4a1" }
                   { "elementId": "el_…9c" } ],
  "chainTangentFaces": true,
  "distanceType": "Offset" }        // Offset | Total | Radius | Diameter
// result
{ "snapshotId": 5012,               // ECHO of the head it answered against
  "targetBodyId": "body_3",
  "faces": [                        // FULL operative closure, stable face-ordinal order,
    {                               // picked ∪ tangent chain (picked flagged)
      "topoKey": "f:22", "picked": true,
      "anchor": { "worldPoint": [1.0, 2.0, 30.0], "surfaceUv": [0.5, 0.5] },
      "descriptor": { /* §10 evidence descriptor, verbatim */ } },
    { "topoKey": "f:23", "picked": false, "anchor": { … }, "descriptor": { … } }
  ],
  "oppositeFace": { "topoKey": "f:04", "anchor": { … }, "descriptor": { … } },  // Total only
  "currentDims": { "radius": 5.0, "thickness": 10.0 },  // keys present when measurable
  "refusal": null }                 // or { "code", "message", "faces": ["f:31"] }
```

- **Refusals are answers, not errors** (`ok:true`): `crossBody` (picks span
  bodies), `unsupportedSurface` (non-planar/non-cylindrical in the closure,
  naming the faces), `chainMismatch` (`chainTangentFaces:false` but the closure
  exceeds the picks — the kernel auto-propagates across tangent junctions and
  cannot hold them fixed, spike-characterized), `noUniqueOpposite` /
  `notPlanar` / `chainOnTotal` (Total constraints), `notCylindrical` /
  `mixedAxis` (Radius/Diameter coaxial-set constraints), `nonManifold`.
- A stale `snapshotId` MUST be refused with `STALE_PREVIEW` semantics (compare
  [AcquireElementIds](#acquireelementids) `REF_UNRESOLVED`): the closure is
  about to be FROZEN into a document record, so unlike the advisory §7.5 reads
  this verb IS snapshot-fenced.
- Determinism: identical head + identical request ⇒ byte-identical response
  (ordinal ordering everywhere, no pointer/hash iteration).

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

#### Checkpoint lifetime — in-session only (V2 policy)

A checkpoint lives **only for the worker session that minted it**. `RestoreCheckpoint`
resolves `checkpointId` against the worker's in-session map, so a checkpoint that
outlives the worker process can only ever answer `restored:false`.

Normative consequences:

* The app **MUST NOT write** the container's `checkpoints/` cache section. The section
  and its `checkpoints/<step>.json` / `.bin` layout stay part of the container format —
  readers **MUST tolerate it present or absent** — but nothing produces it.
* A reader that finds `checkpoints/` entries (a legacy container) **MUST ignore them**
  and log once; the next save drops them and the container shrinks.
* Rust keeps a **bounded** in-session ladder and evicts every checkpoint at or above a
  mutated timeline step (those prefixes are hash-stale by definition).
* Nothing above is a correctness dependency: with zero checkpoints every plan replays
  from 0 and produces the identical head (Invariant 7).

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

#### ExportGeometry

Bakes live bodies into one of the **§7.3 replay codecs** — the byte forms
`ImportStep.sourceCodec` reads back — at a Rust-provided temp path.

```json
// req.args
{ "path": "/tmp/onecad/bake_ef56.brep", "bodyIds": ["body_3"], "codec": "brep",
  "union": false }
// result
{ "written": true, "bytes": 91234, "codec": "brep", "format": 4, "solidCount": 1 }
```

- The inverse of `InspectStep`'s conversion lane: that one converts a FOREIGN
  STEP file into the replay form, this one converts geometry ALREADY IN THE
  SESSION. It exists because a Component Library `embedded` / `document` source
  (spec §2.1) is a baked solid cached in the placing document, and "Save as
  Component" has to produce that solid from what the user modelled.
- `codec` ∈ `"brep"` | `"xbf"`, **required — there is no default**: the caller
  pins the value it will record as `sourceCodec`, so the worker must never
  silently pick a different byte form than the record claims. `xbf` additionally
  carries per-face colors (`brep` does not — the same asymmetry that moved the
  STEP conversion lane to `xbf`, see 2026-08-02 below).
- `format` echoes the binary format version actually written (BinTools `4` /
  OCAF `12`), so a record's `brepFormat` pin is never hardcoded Rust-side.
- A published body is SOLID-LIKE, not necessarily a bare `TopAbs_SOLID`
  (`single_solid_policy` admits a compound wrapper), so each body is flattened
  to the solids it contains and `solidCount` reports the total written. A body
  containing NO solid is a recoverable `OP_FAILED` naming the body — both replay
  readers reject a non-solid, and refusing here reports it where it can still be
  acted on rather than at the far end of a save/reopen.
- `xbf` carries face colors only for a body that flattens to exactly one solid
  (what a component is, spec §9). A multi-solid body's colors are indexed by the
  body's own face map and would need a `ModifiedShape` remap to re-index per
  solid; they are dropped rather than misapplied.
- `bodyIds` is **required and non-empty** (unlike `ExportStep`'s "all" default) —
  a bake is always about a chosen body, and "everything in the session" is never
  a meaningful component.
- `union` (optional, default `false`): when the addressed bodies flatten to more
  than one solid, fuse them (`BRepAlgoAPI_Fuse` chain) into ONE before writing.
  Opt-in because spec §9's one-solid rule is enforced Rust-side at save time, and
  the author is offered the fuse only after being told their body is not one
  solid — a bake that fused silently would change what they modelled. A fuse of
  DISJOINT solids "succeeds" in OCCT and returns a compound that still holds
  both, so the RESULT's solid count is checked as well as `IsDone()`; anything
  other than exactly one solid is a recoverable `OP_FAILED` (`"union refused —
  …"`) and nothing is written. `union` never affects a body that already
  flattens to one solid. Face colors are dropped for a fused result — a fuse
  rewrites the face set, and the source body's face indices no longer address
  anything (the same rule the multi-solid case above follows).

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

For recoverable kernel failures, `detail.diagnostics` MAY carry the bounded
structured array defined in §7.2. It is optional for compatibility. Readers
ignore malformed optional detail while retaining the valid top-level error.

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
                                         //   | "ordinal-permutation" (Rust-seeded only)
  "scoringVersion": 3,                   // = resolverVersion the scores were computed under
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
  This is the version the **worker actually scored under**, which is not required to
  equal the `policyVersions.resolverVersion` Rust pinned in §7.2 — that axis gates
  checkpoint-cache compatibility. When the two differ, this field wins for
  interpreting the numbers in THIS payload.
- `reason`: the four tokens above. `ambiguous` / `no-candidates` / `low-confidence`
  are ladder outcomes a **worker** may emit. **`ordinal-permutation` is
  Rust-seeded only** (VF-B6): it marks a reference standing on an N-body op's
  ordinal child `body_<opId>:<k>` whose [§7.2 `rankKey`](#72-regen--executeplan)
  evidence shows the ordinals permuted under a parametric edit — the ref would
  otherwise re-resolve cleanly to the WRONG solid. A worker MUST NOT emit it.
  **Readers MUST tolerate an unknown `reason` token** rather than failing the
  payload; a Rust reader degrades an unrecognized token to an opaque `unknown` so an
  older release can still open a document written by a newer one.
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
   step. Images are deduplicated by TopoDS identity. A unique image auto-binds;
   multiple images pass the same confidence/margin gate. `Modified()` contributes
   `0.09` provenance confidence, deliberately below the `0.10` margin: it cannot
   resolve an otherwise tied Modified/Generated pair by itself.
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
it with a **normalized `[0,1]` versioned confidence** (`resolverVersion = 3`;
version 2 added the edit-scoped veto and proportional anchor scale; version 1 had
neither). Version 3 adds Modified-channel provenance only at the history rung.
Higher = better match. Policy:

- **Auto-bind iff** `score1 ≥ 0.85` **AND** `(score1 − score2) ≥ 0.10`
  (score1/score2 = best/second-best candidate) **AND the edit-scoped
  descriptor-tie veto below does not fire**.
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
- The **anchor proximity feature scales by `max(0.5 × bodyDiagonal, 1e-7)`** — the
  floor is a divide-by-zero guard, NOT a modelling-scale constant. (In
  `resolverVersion = 1` it was `1.0`, which silently stopped being proportional for
  every body under 2 mm across: on a sub-millimetre part every candidate sat a
  small fraction of a millimetre from the anchor, every anchor similarity collapsed
  towards 1, the margin vanished and the whole part became un-resolvable. The
  degenerate-bbox case is already floored at `1.0` when the diagonal is computed.)

**Edit-scoped descriptor-tie veto (`resolverVersion = 2`).** Congruent twins — two
sub-shapes with identical type, magnitude, direction and adjacency hash — tie
EXACTLY in descriptor space, so the `anchor` is the only feature that can separate
them. Whether letting it do so is correct depends on something the worker cannot
see in the geometry:

- On a **no-edit replay** (no [§7.2](#72-regen--executeplan) `editedFrom`) the
  geometry is rebuilt exactly as the ref was authored against, so the stored anchor
  sits on its element and **the anchor MAY decide the tie** — this is what makes a
  reopen, a rollback and an undo/redo resolve cleanly.
- After an **upstream content edit** (`editedFrom = k`, for refs owned by a step
  `> k`) the geometry moved out from under the stored anchor, which can now sit
  closer to a twin than to the real element. There the anchor is precisely the
  evidence the edit invalidated, so **it MUST NOT decide**: the worker emits
  NeedsRepair with `reason: "ambiguous"` and both candidates in the evidence.

Normatively, for a ref owned by a step `> editedFrom`, auto-bind is REFUSED when
**both** hold:

1. **Descriptor tie** — the assigned candidate and its best rival are separated by
   **less than `0.02` in DESCRIPTOR-ONLY score**: the same weighted confidence
   recomputed with the `anchor` feature removed and renormalized over the remaining
   weights, with the `anchor` feature having contributed at all. The comparison is
   signed, so it also refuses when the anchor overrode a descriptor-BETTER rival.
   `0.02` is an order of magnitude below the `0.10` margin gate on purpose — the
   veto must fire only where the descriptor genuinely cannot tell candidates apart
   (congruent twins separate by exactly `0`), never merely because descriptor
   evidence is weak.
2. **NOT anchor-exact** — the assigned candidate does not still lie on its stored
   anchor. "Lies on" is measured as the distance from the anchor world point to the
   candidate **sub-shape** (NOT to its centroid), against `0.05 × ` the anchor
   scale. **Shape distance is load-bearing**: a parametric edit routinely slides a
   feature along its own axis — growing an extrude's depth moves every vertical
   edge's midpoint by half the delta while the edge still passes exactly through its
   anchor — and centroid distance would misreport that as a move.

Clause 2 is the **anchor-exact carve-out**, and it is what keeps the veto scoped to
the class it was built for. An element still sitting on its anchor demonstrably did
NOT move, so the edit never made *its* anchor stale and there is nothing to
distrust; that covers ~all real edits. The veto therefore fires on the **DRIFT**
class — a twin merely NEARER to a stale anchor than the moved original, where
nothing is sitting where it was authored and proximity alone would be a guess.

A ref with **no frozen descriptor** ties at descriptor score `0` against every
candidate, so clause 1 always holds for it; clause 2 still resolves the common case
(a vertex pick whose element did not move), so such a ref is not blanket-refused
after an edit.

**Accepted residual — the TELEPORT case.** An edit that parks an EXACT congruent
twin *precisely* at the stale anchor, while moving the original away, still
auto-binds — to the twin, silently. This is locally undecidable at this rung: the
worker sees two byte-identical descriptors, one exactly at the anchor, and no
evidence separating "it never moved" from "something else moved onto it". Refusing
it means dropping the carve-out, which regresses the flagship gesture (a fillet on a
plain box — whose four vertical edges are exact twins — would then NeedsRepair after
every upstream edit). The residual is accepted and documented here; closing it is
reserved for the from-0 history rung, which has the lineage this stage lacks.

Note the two version fields answer different questions and MAY differ: §7.2
`policyVersions.resolverVersion` is the axis **Rust pins** (it gates checkpoint-cache
compatibility), while [§9](#9-needsrepair-payload) `scoringVersion` is the version
the **worker actually scored under** and is the authority for interpreting the
numbers in a payload.

The worker returns, per ref: candidates, `featureContributions`, `score`,
`margin`, and the ladder level reached — full evidence for repair UI and for
moving policy to Rust later.

**History-channel provenance (`resolverVersion = 3`).** Generated images are kept
in the candidate union because some builders expose the only successor there, but
generated side topology must not erase stronger direct lineage. A Modified image
therefore adds `0.09` to its normalized history score (clamped to `1.0`). The value
is strictly below the locked `0.10` margin, so a descriptor/anchor tie still emits
NeedsRepair; the underlying geometry evidence must supply the remaining separation.

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
- **App and worker deploy LOCKSTEP, by construction.** The worker ships as a Tauri
  `bundle.externalBin` sidecar staged into the app bundle, so a build cannot pair a
  new Rust side with an old worker binary. An OPTIONAL additive field therefore does
  not need capability negotiation — but it does need this to stay true: nothing may
  resolve the worker from outside the bundle in a shipping configuration
  (`ONECAD_WORKER_PATH` is a development and test seam only). A field whose absence
  silently degrades CORRECTNESS rather than performance — such as
  [`checkpointFallbackReplay`](#72-regen) — relies on this property.
  Every release embeds a generated manifest containing the bundled binary SHA-256,
  protocol/worker/quantization/solver axes, and OCCT version/fingerprint. The app
  verifies the digest before spawn and every hello axis before the session becomes
  ready. Release resolution accepts only the executable-adjacent bundled sidecar.

---

## 14. Changelog

`protocolVersion` stays **1** for all entries below — these are pre-implementation
contract refinements (no worker has shipped against the prior text), so they are
edits to version 1 rather than a version bump. They still fall under the
[§13](#13-versioningchange-policy) change policy (fixture bump + cross-track
sign-off) once fixtures exist.

- **2026-08-14 — §7.3 NEW op `Gear`** (Gear Generator G1). A fully parametric
  generated gear body: no sketch, no host, MINTS `body_<opId>` (D1) from a
  typed recipe block plus a placement that is EITHER a planar face (Hole's
  frozen-point semantics and its 1e-3 mm reprojection fence, axis = inward
  normal) or an explicit frozen frame. `recipe` selects which recipe block is
  non-null and every inactive key is spelled `null`, reusing Hole's
  conditional-block contract at all four trust boundaries rather than inventing
  a second one; `recipe` ∈ `involuteExternal` for now, and the key set is
  versioned by §7.3 rather than by any addon (ADR-0002 — typed variants, never
  free-form strings). Purely ADDITIVE: one new `opType` value, no existing
  payload changes shape, no existing caller changes.
  - Publication takes `single_solid_policy("Gear", TierB)` — the full audit
    including self-interference — because a generated profile is precisely
    where a bad parameter combination yields a plausible but self-intersecting
    solid. This is the second op family after Fillet/Chamfer to take the deep
    tier deliberately rather than by inheritance.
  - **Referenceability is narrowed on purpose**: the minted body root, the
    placement input and the optional BORE faces are referenceable; tooth
    flanks/tips/roots are NOT, and a promotion attempt is refused BY NAME. A
    tooth's identity changes with the tooth count, so binding a descriptor to
    one is the silent-wrong-bind case §10 exists to prevent — declaring the
    sub-geometry unreferenceable is the honest form of that rule, not an
    exception to it.
  - `helixAngleDeg` ≠ 0 and `doubleHelix` = true are **`UNSUPPORTED`** in this
    version (the Frenet sweep infrastructure is a later phase). The fields are
    present and round-trip so the payload will not change shape when helical
    lands — the same forward-compatibility posture `Loft`/`Sweep` already carry.
  No fixture bump is owed by this entry alone: the op adds files rather than
  moving any existing shape, and its own fixtures land with the worker path.
- **2026-08-14 — §7.2 the mate carve-out, written down.** The hardening entry below
  states that a `completed` stream "contains only `ok` rows"; `PlaceComponent` has
  always published an `ok` row carrying `planStep.needsRepair[]` when its mate could
  not re-seat. Both are deliberate and they were never reconciled in prose — the two
  programs landed a day apart. §7.2 now says explicitly that a published step MAY
  carry `needsRepair[]` **only** for a mate, never for a topological input it built
  from. No code changes on either track: Rust's `validate_prepared_stream` already
  accepted this shape, and the worker already emitted it. Documentation catching up
  to a deliberate design, not a contract change.

- **2026-08-13 — atomic publication and identity V3 hardening.** Plan streams are
  now strict ordered prefixes and malformed terminals are discarded before
  publication. Release builds verify a generated worker manifest and accept only
  the bundled sidecar. New sketches author `regionIdentityVersion:3`/`cell-v3`;
  V1/V2 remain frozen. `ResolveRefs` validates revision/body provenance. STEP
  conversion refuses exact canonical-order ties with `AMBIGUOUS_IMPORT_ORDER`.
  Prepared meshes obey advertised per-blob and aggregate limits.

- **2026-08-13 — resolver scoring version 3 history provenance.** History now
  unions and deduplicates `Modified()` + `Generated()` images. Multiple images
  remain confidence-gated; direct Modified lineage contributes `0.09`, below the
  locked `0.10` margin, so it cannot settle an otherwise tied pair. This prevents
  generated side topology from masking a descriptor-separated Modified successor.
  `scoringVersion` fixtures move 2 → 3; protocol and checkpoint policy axes do not.

- **2026-08-13 — §7.8 `ExportGeometry`: new OPTIONAL `union` arg** (Component
  Library WP-F1.2, spec §9, single-repo, both tracks land together). Purely
  ADDITIVE: absent/`false` is the pre-WP-F1.2 behavior byte-for-byte, and no
  existing caller changes. "Save as Component" refuses a body that bakes to more
  than one solid; the author's only recourse was to leave the dialog and rebuild
  the body. With `union` the dialog can offer the fuse in place, and it is the
  BAKE that fuses — not the document — so the author's own model is untouched
  and the fused solid exists only inside the component package.
  - The fuse is checked by RESULT, not by `IsDone()`: OCCT happily "fuses"
    disjoint solids into a compound of both, and writing that would move the
    one-solid failure to whoever places the component. Refused at the bake.
  - Pick-primary and split-into-several-components are NOT offered — each needs
    a solid-picking UI the authoring dialog does not have, and guessing which
    solid the author meant is exactly the substitution failure spec §0 forbids.
  - No fixture bump: `protocol/fixtures/` carries no `ExportGeometry` NDJSON.

- **2026-08-13 — §7.3 `PlaceComponent`: new OPTIONAL `mate.selfFrame`**
  (Component Library WP-F1.1, spec §2.1/§5, single-repo, both tracks land
  together). Closes spec §11's open item 4 ("components seat at their model
  origin"): an attachment's local basis now travels end-to-end — authored in
  `component.toml [attachments].<key>.frame`, frozen into the record at
  placement, and honored by BOTH placement solvers (the FE ghost in
  `placementSolver.ts` and the worker's regen re-seat in
  `ComponentMateSolver.cpp`, which stay a 1:1 port of each other and share
  their numeric test cases).
  - **Purely ADDITIVE.** One new optional field on an existing optional
    object; nothing is removed or retyped. Absent ⇒ the identity frame, and
    the worker short-circuits to the pre-WP-F1.1 code path rather than
    multiplying by identity, so every existing document lowers AND re-seats
    byte-identically (pinned by the unchanged `test_component_mate_reseat`
    cases plus an explicit byte-identity check in `test_component_mate_solver`).
  - **No fixture bump**: `protocol/fixtures/` carries no component-op NDJSON at
    all, so there is no recorded frame to move. When one is added it should
    cover both the framed and unframed mate.
  - Composition is `M = S ∘ F⁻¹` (seat transform ∘ inverse attachment frame) —
    see the `mate.selfFrame` bullet in §7.3 for the full definition.

- **2026-08-13 — §7.3 `PlaceComponent` / `DetachComponent`: three NEW
  registered `source.generatorId`s** (Component Library WP-F2, spec §6.2,
  single-repo, both tracks land together). **Purely ADDITIVE — no wire shape
  changes**: no field is added, removed or retyped, `source.params` was
  already an open map, every existing document lowers byte-identically, and no
  fixture moves.
  - Registered now: `iso15` (ISO 15 deep-groove ball bearing) · `nema17` /
    `nema23` (NEMA stepper motors), alongside WP-A1's seven fastener ids. The
    unregistered-id refusal is unchanged in kind; only the list it names grew.
  - `iso15` is keyed by `params.code` (`"608"`), not `params.thread` — the
    op layer now forwards EVERY string under `source.params` to the generator
    verbatim instead of naming three keys, so a future family keyed by
    something else needs no §7.3 change either. `params.length` gains an
    "absent vs. present" distinction so a motor can default to its own frame's
    body length rather than a screw's 20 mm; a length below the frame's
    minimum is refused, never clamped.
  - Seeded packages `onecad.std.iso15` / `.nema17` / `.nema23` ship with them
    (`SEED_VERSION` 2).

- **2026-08-13 — §7.3 `PlaceComponent` / `DetachComponent` `source.generatorId`
  is now DISPATCHED, and two stale §7.3 paragraphs corrected** (Component
  Library WP-A1, spec §6.2, single-repo, both tracks land together).
  - **Behavior change, deliberate and loud:** an unregistered `generatorId`
    now fails recoverably with `OP_FAILED` naming the registered ids. It used
    to build an ISO 4762 socket cap screw for ANY id — harmless while one
    family existed, a silent substitution (spec §0 invariant 4) the moment a
    second one did. Registered: `iso4014` `iso4017` `iso4032` `iso4762`
    `iso7089` `iso7093` `iso7380`. No document changes shape; no fixture
    moves; `iso4762` output is byte-identical (pinned by its exact-volume
    ctests, which is what made the extraction safe).
  - **Corrections to text that had gone stale**, both from P3 WP-3.1 landing
    without its §7.3 prose being updated (the §14 entry for that change was
    written; these two lines were not): `inputs[]` is documented as ALWAYS
    empty (it has carried no mate ref since WP-3.1), and the "not yet
    re-seated by the worker" paragraph is replaced by what actually ships.
    Documentation-only — no wire or code change in this entry.

- **2026-08-13 — §7.8 NEW verb `ExportGeometry`; §7.3 `PlaceComponent` /
  `DetachComponent` source kinds `embedded` + `document` IMPLEMENTED**
  (Component Library WP-3.2, spec §2.1/§4/§9, single-repo, both tracks land
  together). Additive on both counts — no existing document changes shape, and
  a `generator` source lowers byte-identically to before.
  - `ExportGeometry` is the missing inverse of `InspectStep`'s conversion lane:
    it bakes a body ALREADY IN THE SESSION into a §7.3 replay codec at a
    Rust-provided temp path. `GetBodies` (§7.6) is specified for BREP-out but
    has never been implemented, and its bulk-stream shape is heavier than the
    file hop every other §7.8 export already does; a component bake is a
    once-per-authoring operation, not a streaming one. The response echoes
    `codec`/`format` so a record's `brepFormat` pin is the version the worker
    actually wrote, never a Rust-side constant.
  - `embedded`/`document` were declared in this section from WP-0.2 but refused
    at the worker. They now resolve through the same readers `ImportStep` uses,
    from a wire-only `source.path`. The alternative — resolving a `document`
    component by REPLAYING its frozen `.onecad` at regen — was rejected: the
    worker is one-session-per-process, so a nested replay needs a second worker
    process AND the library folder still present at every regen, and spec §4
    requires a placed component to render with the library deleted. Baking at
    authoring keeps the geometry in the document, where the invariant needs it.
- **2026-08-13 — §7.2 ADDITIVE `planStep.matePlacement`; §7.3 `PlaceComponent`
  CORRECTIVE `inputs[]` — `mate.target` REMOVED** (Component Library P3
  WP-3.1, spec §5.5, single-repo, both tracks land together). The prior
  entry below (2026-08-12) put `mate.target` in `inputs[]`; this was
  correct only while the worker never resolved `mate` at all. The worker's
  generic `resolve_input_refs` pre-flight treats ANY unresolved `inputs[]`
  entry as blocking — genuinely correct for a face/edge an op structurally
  needs, but WRONG for a mate: an unresolvable target must still let the
  component publish at its frozen `placement` (spec: "never drop it, never
  silently move it"), not skip the op. Found tracing `wire_op_inputs` while
  building the Rust-side end-to-end regen test for the NeedsRepair path —
  `mate.target` now travels ONLY in `params` (unchanged) and is resolved
  entirely
  in-process by the worker's own `resolve_mate_reseat`, never through the
  wire `inputs[]` pre-flight. Persistent
  mate re-seating on regen: a `PlaceComponent` step whose `mate` resolves
  `AutoBind` through the ladder (worker-side, mid-`ExecutePlan`, so it sees
  SAME-TICK geometry — no new hashing hazard, since `params.mate` itself
  never changes and `history_prefix_hash` never needs to see the
  recomputed `placement` before executing) and moves past the worker's
  pinned reseat epsilon publishes with the new seat AND echoes it via this
  optional field; absent on every other step (byte-identical wire
  otherwise). Rust persists it as a derived, no-undo `placement` writeback
  (`document_runtime.rs::sync_mate_placements`, mirrors
  `sync_record_outputs`) — never a fencing input, never touched on
  `NeedsRepair` (the component publishes at its last frozen `placement`
  instead, per spec's "never drop it, never silently move it"). Worker:
  `worker/src/ops/ComponentMateSolver.h/.cpp` (a verbatim port of
  `src/modules/library/placementSolver.ts`'s WP-1.5 interactive-gesture
  math), `worker/src/session/ClassifyElement.{h,cpp}` (new in-process
  `classify_shape`, no wire round trip), `worker/src/ops/ComponentOp.cpp`
  (`resolve_mate_reseat` — cross-body-safe ladder resolution, VF-M7
  discipline: the target body is ALWAYS read from `mate.target.primary
  .bodyId`, never assumed to be the placed component's own body), threaded
  through `CandidateResult`/`emit_plan_step` in
  `worker/src/session/PlanExecutor.{h,cpp}`. Purely additive; no existing
  wire form changes.

- **2026-08-13 — §7.3 Pattern V2 lineage gains a cross-track fixture** (roadmap A6).
  No contract change: `circular_pattern_lineage.ndjson` pins the already-normative V2
  rules on the wire — `count-1` children named `body_<opId>:<k>`, the source preserved
  as instance zero with no lifecycle event of its own, and the `perStepResults`
  body-id set. The per-instance step angle (`angleDeg / count`) is geometry and is
  NOT asserted here; it stays pinned in `test_m6a_ops.cpp` and the frontend
  `patternPreview` unit test, because an NDJSON exchange carries nothing to measure
  it with. **No wire change; fixture ADDITION only.**

- **2026-08-13 — §7.5 `ResolveRefs` resolutions carry the snapshot echo they always
  specified** (roadmap A6, Rust + worker sign-off). The §7.5 text has required
  `{snapshotId, revision, refId, bodyId}` on every resolution since it was written,
  and neither the C++ worker nor the Rust stub emitted any of the three: Rust
  manufactured all of them app-side from its own state and validated nothing, so a
  resolution computed on an older snapshot was cached under a freshly minted key —
  the silent wrong bind the rule forbids. The worker now echoes them on every branch
  (`unchanged`, `autoBind`, `needsRepair`; `bodyId` only when a body was enumerated),
  the stub matches, and `wire::validate_resolve_refs_result` refuses a mismatched
  snapshot, a re-ordered `refId`, a wrong arity, or a missing echo. *Reason:* the
  contract was normative and unenforced; the fields are additive, so the frame shape
  and every existing top-level code are unchanged. **Fixture bump — 2 files:**
  `resolve_refs_snapshot_echo.ndjson` (new — both branches plus a stale-snapshot
  refusal) and `bind_element_ids.ndjson` (its `ResolveRefs` expectation now asserts
  the echo). Verified non-vacuous: dropping the `revision` echo in the worker reds
  `canonical_resolve_refs_snapshot_echo`.

- **2026-08-13 — §7.3 stable diagnostic codes for the Draft and zero-solid Boolean
  refusals.** Both previously returned a bare `OP_FAILED` with the reason only in
  the message, so a caller wanting to tell "no planar wall to draft" from "the
  kernel rejected the walls I offered" — or "your Cut consumed the target
  completely" from any other Boolean failure — had to match on message TEXT, which
  the diagnostics contract forbids. Adds `EXTRUDE_DRAFT_NO_PLANAR_FACE`,
  `EXTRUDE_DRAFT_NO_FACE_ACCEPTED`, `EXTRUDE_DRAFT_NO_CHANGE`,
  `EXTRUDE_DRAFT_BUILD_FAILED` (`stage:"build"`, evidence
  `{draft:{angleDeg,eligibleFaces,addedFaces}}`) and `BOOLEAN_EMPTY_RESULT`
  (`stage:"publish"`, evidence
  `{boolean:{operation,targetBodyId,toolBodyId,solidCount}}`). Diagnostic codes
  only — the §8 top-level code is unchanged, so this is additive on the wire and
  every existing fixture stays byte-valid. `boolean_empty_refusal.ndjson` is
  extended to assert the new diagnostic (verified to fail on a wrong code).

- **2026-08-12 — §7.3 NEW ops `PlaceComponent`/`DetachComponent`** (Component
  Library WP-0.2/WP-1.2, single-repo, both tracks land together). Instantiate
  a library component as a first-class placed instance, and drop a placed
  instance's library identity while keeping its cached geometry ("the honest
  break link", spec §3.1/§3.4). Both mint a NewBody (`body_<opId>`); a
  component resolves to exactly one solid in v1 (`single_solid_policy`).
  `PlaceComponent`'s `mate`, when present, is carried as a full semantic ref
  in `inputs[]` (absent ⇒ no inputs at all) but is READ, not yet re-seated —
  `placement` is the only transform this build applies; persistent
  mate-driven re-seating on regen is P3. `source.kind` ∈ `generator` |
  `embedded` | `document`; this build implements `generator` only (a
  HARDCODED ISO 4762 M6×20 SHCS solid, regardless of `generatorId` —
  table-driven per-generator dispatch is P2), the other two kinds refuse
  recoverably with `UNSUPPORTED`. `DetachComponent` is applied as an in-place
  `PlaceComponent → DetachComponent` op-type swap at the SAME `RecordId` (the
  Fillet⇄Chamfer swap precedent, newly sanctioned in
  `edit::session::op_type_edit_allowed`) — one-directional, no reverse swap.
  Spec §3.2/§3.3's `SetComponentParams`/`ReplaceComponent` are deliberately
  NOT wire ops (in-place edits of `PlaceComponent`'s own params instead) — see
  the `DetachComponent` entry above for the reasoning. Purely additive; no
  existing wire form changes. Worker:
  `worker/src/ops/ComponentOp.h/.cpp` (shared `resolve_source_and_publish`
  pipeline), dispatch in `worker/src/session/PlanExecutor.cpp`. Rust: 6
  mirror sites in `onecad-core::document::record` (`KnownOperation` enum +
  `KNOWN_OP_TYPES`, `element_refs_mut`, `op_type`, `derive_inputs`) and
  `edit::session` (`validate_place_component`/`validate_detach_component`,
  wired into both `add_operation` and `update_operation_params`), plus
  `worker::wire::wire_op_inputs` and `document_runtime::element_ref_input`
  (`src-tauri/src/`). No fixture yet — gated by `worker/tests/test_component_ops.cpp`
  (7 cases) and `src-tauri/tests/component_ops.rs` (3 cases incl. the swap
  through the real worker) rather than a `protocol/fixtures/*.ndjson` pin; a
  fixture lands once WP-1.3 gives the op family a non-spike consumer.

- **2026-08-12 — §7.5 NEW read-only verb `ClassifyElement`** (Component
  Library WP-0.1, single-repo, both tracks land together). Interactive
  surface/curve classification + a seatable frame (plane origin+normal,
  cylinder/circle axis+radius) for the placement/mate-snap solver's hover
  gesture. Addressed like `QueryElement` (`elementId` or `{bodyId, topoKey}`)
  but with no `snapshotId` — it always reads the current head, matching
  `QueryMassProperties`'s reasoning, since it is re-issued every hover frame
  rather than tied to one pick. Purely additive new verb; no existing wire
  form changes. Worker: `worker/src/session/ClassifyElement.h/.cpp`, dispatch
  in `worker/src/main.cpp`. Rust: `worker::wire::classify_element_args` /
  `parse_classify_element` (`src-tauri/src/worker/wire.rs`), the
  `ElementQuery::classify_element(_by_topo_key)` trait methods
  (`src-tauri/src/worker/mod.rs`), `WorkerManager` impl (`manager.rs`), the
  `#[tauri::command] classify_element` (`src-tauri/src/api/mod.rs`). No
  fixture yet — P0 gate is `src-tauri/tests/classify_latency.rs`'s p95
  measurement, not a wire-contract fixture; a fixture lands with WP-1.2 once
  the verb has a non-spike consumer.

- **2026-08-11 — §2, §7.2, §7.3 Pattern V2 publication/lineage policy.**
  `resultPolicyVersion` absent remains frozen V1. New records author literal `2`;
  other numeric values load/resave losslessly and execution refuses recoverably with
  `UNSUPPORTED_PATTERN_RESULT_POLICY_VERSION`.
  V2 non-fused Pattern preserves source as instance zero and creates only ordinal
  children for transformed instances. V2 fused Pattern modifies source in place
  and requires one connected result. This supersedes the M6a statement below that
  Patterns always mint one `body_<opId>`; that statement is V1-only. Additive
  parameter semantics and lifecycle behavior; legacy fixture bytes remain valid.

- **2026-08-09 — §7.5 internal `BindElementIds`; §7.3 typed Shell face refs**
  (REF-H0, cross-track sign-off in one repository). `AcquireElementIds`
  remains read-only and Rust remains the sole `ElementId` minting authority;
  promotion now completes only after snapshot-fenced, exact, idempotent, atomic
  installation into the authoritative worker-head partition. This makes a fresh
  id directly resolvable on the unchanged head instead of forcing descriptor
  fallback. `ShellParams.faces` add typed face evidence in lockstep with the
  retained `openFaces` ids; absent/empty `faces` preserves legacy bare-id
  behavior without synthesizing evidence. Additive version-1 contract refinement.
  `protocol/fixtures/bind_element_ids.ndjson` signs off the Rust parser and C++
  harness on face/edge/vertex direct hits plus invalid-batch rollback.

- **2026-08-09 — §7.2 ADDITIVE `checkpointFallbackReplay`; §13 lockstep note**
  (VF-M5, cross-track sign-off in one repository). Closes the residual left when the
  from-zero-replay gate was disabled: the hazardous lane is the regen executor's
  post-restore-failure fallback, and it is indistinguishable from an ordinary
  replay-from-0 in the plan, so the worker cannot infer it. Rust now declares it and
  the worker gates the §10 anchor-exact carve-out on that declaration alone. The
  ordinary edit lane is UNCHANGED — its teleport case remains the accepted residual.
  Omitted when false, so an ordinary plan's `args` are byte-identical to the prior
  text and **no bump is required for existing fixtures**. Evidence, split by what
  each artifact actually proves: the BEHAVIOUR is proven end to end by
  `src-tauri/tests/topology_rebind.rs` lane D (RED before this entry, green after)
  with the unchanged ordinary lane pinned beside it; the new canonical fixtures
  `protocol/fixtures/execute_plan_{ordinary,checkpoint_fallback}.ndjson` prove only
  CROSS-LANGUAGE PARSE AGREEMENT — that both tracks read the field and that a
  worker accepts a plan carrying it. They are not a second behavioural proof.

- **2026-08-07 — §7.3/§7.6 frozen Fillet/Chamfer tangent intent** (F2,
  cross-track sign-off in one repository). Added read-only `PrepareEdgeOp`,
  optional `tangentClosureVersion:1`, atomic batch promotion, exact-preview
  commit, contour deduplication, and structured closure-drift refusal. Legacy
  records remain byte-shape stable and execute seed-only behavior.

- **2026-08-07 — §7.3 constant Fillet execution hardened** (F1,
  cross-track sign-off in one repository). Constant-law assignment, contour
  deduplication, partial-result rejection, structural/self-interference audit,
  history-safe builder lifetime, and bounded `FILLET_*` evidence are now
  normative. No wire shape changed; no fixture bump required.

- **2026-08-07 — §6 OCCT fingerprint input made reproducible** (K0,
  cross-track sign-off in one repository). Primary worker builds require exact
  OCCT 8.0.1 from a caller-selected artifact. Fingerprints now include OCCT
  version, pinned source commit, normalized build options, build id, and OneCAD
  kernel-policy version. Per-operation `occtOptions` remain history-hashed.
  Existing wire shape is unchanged; no fixture bump required.

- **2026-08-07 — §7.2/§7.6/§8 structured failure diagnostics** (D1,
  cross-track sign-off in one repository). Failed `perStepResults[]` and preview
  `error.detail` may carry the same bounded diagnostic array; top-level error
  codes remain unchanged. Additive optional fields; legacy readers and missing
  arrays remain compatible. No canonical fixture bump required.

- **2026-08-06 — §7.3 NEW op `OffsetFace` (`op.offsetFace`) + §7.6 NEW read-only
  verb `PrepareOffsetFace`.** Cross-track sign-off recorded as
  **orchestrator-approved 2026-08-06 (single-repo, both tracks land together)**.
  Direct-modeling per-face offset (planar + cylindrical V1): params carry the
  Fillet-discipline `faceIds` + typed `inputs[]` refs (operative faces in stored
  order, Total `oppositeFaceId` ref LAST — slot order normative for the repair
  paths), `distance` interpreted per `distanceType`
  (Offset|Total|Radius|Diameter), mandatory `targetBodyId`, always `modified`
  lineage (never NewBody / fan-out). The operative set is FROZEN at authoring:
  `PrepareOffsetFace` computes the G1 tangent closure + Total opposite + current
  dims on a head copy and returns TopoKey EVIDENCE only (Rust mints, per
  Invariant 2); the worker never re-expands at regen. Kernel notes pinned by the
  Phase-0 spike (`worker/tests/test_offsetface_spike.cpp`): tangent
  auto-propagation ⇒ `chainMismatch` refusal, face history via the offset image
  (public `Generated`/`Modified` are face-empty), never-clamp + semantic
  postconditions (exact plane shift / coaxial radius), single-solid gate.
  Fixtures: `worker/tests/fixtures/executeplan_offsetface.ndjson` (new).

- **2026-08-05 — §7.2 DESCRIPTIVE: document the OPTIONAL `PlanPrepared.artifacts`
  terminal key.** Cross-track sign-off recorded as **orchestrator-approved
  2026-08-05 (single-repo, worker unchanged)**. **No wire change of any kind.** The
  C++ worker has emitted `result.artifacts.tessellate.meshes[]` (with the MESH1
  blobs in the terminal `resp`'s binary tail) since the `artifacts.tessellate`
  request rider shipped — `worker/src/session/PlanExecutor.cpp::attach_tessellate`.
  The §7.2 terminal block simply never described it, so what the worker sent was an
  undocumented extension that Rust discarded, then re-fetched body-by-body via
  `Tessellate` — every body tessellated twice per regen. This entry documents the
  existing shape, pins the two properties readers were already relying on
  implicitly (present iff the request carried the rider; **absent** on the
  idempotent cached re-return), and states the Invariant-7 reader rule: it is a
  cache fill, so any defect degrades to a `Tessellate` pull and never fails a plan.
  It also records the standing §5.2 tension — the worker inlines these blobs
  regardless of `chunkSize`; chunking that lane is the forward direction, and the
  reader-side ingest cap is the interim guard.
  **No worker change, no `protocolVersion` change, no fixture bump — 0 bytes move.**
  Rust-side consumer: `worker::wire::parse_plan_artifact_meshes` →
  `PlanPrepared.artifact_meshes` → `RegenExecutor` mesh sink → the runtime
  `MeshCache`, seeded strictly inside the same fencing guard that commits the
  snapshot (a superseded prepare caches nothing).

- **2026-08-04 — §7.4 ADDITIVE `BeginGesture.drag.{kind,entity,role,grab}` +
  `SolveDrag`/`EndGesture` `curves`** (SP-2 direct manipulation; cross-track
  sign-off recorded in `TODO.md` by the SP-2 gate). **Purely additive in BOTH
  directions; `protocolVersion` stays 1.** Requests gain four optional members on
  an object that was itself optional, and **an absent `kind` is defined to mean
  `point`**, so every request shape a caller can send today keeps its exact
  meaning. Responses gain one optional map that parses as `{}` when absent.
  Rationale: `positions` is a point-only channel, and a drag is not point-only.
  A `radius` gesture moves no point whatsoever; an `arcEnd` gesture reshapes an
  arc's radius and angles; and even a plain `point` drag propagates through a
  `Tangent` into a neighbouring curve's radius. The Rust `Arc` is
  `{center, radius, startAngle, endAngle}` with NO endpoint entities, so
  `apply_solved_positions` — which moves `Point` entities only — is structurally
  incapable of receiving any of that. The frontend seam is already documented as
  open (`src/ipc/sketchWireMap.ts`: arc `<id>.start`/`<id>.end` keys are unmapped
  and skipped, "coords go stale"); `curves` is what closes it.
  This also fixes a LATENT silent-revert defect that predates the kinds: a point
  drag propagating through a `Tangent` changes a radius in the worker's stored
  wire, Rust never learns of it, and the next `SketchUpsert` quietly reverts the
  geometry to the stale radius. `curves` is therefore emitted for EVERY kind, not
  only `radius`.
  The three degenerate guards (`MIN_GEOMETRY_SIZE` radius floor, `MIN_ARC_SWEEP`
  refuse-step-with-`partial`, `entityBody` targets advisory / solver-wins) are new
  NORMATIVE text, not new wire shape.
  **No fixture bump — 0 files.** No existing shape, id or byte moves: an absent
  `kind` reproduces the previous request byte-for-byte on the `point` path, and
  the NDJSON fixtures under `worker/tests/fixtures/` are **subset** matchers, so
  the extra `curves` key on a response is tolerated by construction — the same
  argument as the 2026-08-03 `bodyEvents[].rankKey` entry below.
  `protocol/fixtures/` (`hello` + `echo_error`) is untouched. A NEW fixture,
  `worker/tests/fixtures/sketch_gesture_kinds.ndjson`, covers the added kinds;
  the existing `worker/tests/fixtures/sketch_gesture.ndjson` stays
  **byte-identical** and is the back-compat regression for the absent-`kind`
  path.

- **2026-08-04 — §7.2 ADDITIVE `editedFrom` + §10 edit-scoped descriptor-tie veto,
  proportional anchor floor, `resolverVersion` 2** (HISTORY-HARDEN H6a, review
  finding B3; cross-track sign-off recorded in `TODO.md` by the HISTORY-HARDEN H6a
  gate). §7.2 gains ONE optional request field, `editedFrom` — the timeline step of
  the upstream content edit that triggered the regen. **Purely additive; absence is
  "no claim" and reproduces the previous behaviour exactly.** Only the edit lane
  sets it (`RegenToEnd(from)` with `from > 0`); open/import/recovery/undo/redo all
  request `RegenToEnd(0)` and every `RegenToStep(k)` preview omits it, and
  **`from == 0` is deliberately treated as ABSENT** — a from-0 replay is
  indistinguishable from a first-record edit, and claiming an edit there would veto
  every congruent-twin resolution and make a clean reopen un-resolvable.
  §10 changes the scoring POLICY in two ways, hence `resolverVersion` 1 → 2: (a) for
  a ref owned by a step **strictly after** `editedFrom`, auto-bind is refused when
  the assigned candidate and its best rival are within `0.02` in **descriptor-only**
  score (the anchor feature removed, the anchor having contributed) **AND** the
  winner is not **anchor-exact** — i.e. it no longer lies within `0.05 × ` the anchor
  scale of its stored anchor, measured to the SUB-SHAPE rather than to its centroid
  so that a feature sliding along its own axis is correctly read as "did not move";
  (b) the anchor-proximity scale floor `max(0.5 × bodyDiagonal, 1.0)` becomes
  `max(0.5 × bodyDiagonal, 1e-7)`, a divide-by-zero guard rather than a
  modelling-scale constant.
  The anchor-exact carve-out scopes the veto to the **DRIFT** class (a twin merely
  nearer to a stale anchor than the moved original). Without it the veto is a blanket
  post-edit refusal that regresses the flagship gesture — a fillet on a plain box,
  whose four vertical edges are exact descriptor twins, would NeedsRepair after every
  upstream edit. The **TELEPORT** case (an edit that parks an exact twin precisely at
  the stale anchor) remains an ACCEPTED, documented residual: it is locally
  undecidable at this rung and is reserved for the from-0 history rung, which has the
  lineage the descriptor stage lacks.
  Rationale: H5 measured that congruent twins tie EXACTLY in descriptor space, so
  the anchor decides. That is correct on a clean reopen — the anchor sits on its
  element — and wrong after an upstream edit, where the geometry moved and a twin
  can sit closer to the STALE anchor than the real element; the ladder then bound it
  and reported zero repairs (finding B3, the H5-B silent-wrong-bind class). The two
  cases are locally indistinguishable in the worker, so Rust — the only side that
  knows why a regen was requested — declares the context and the worker gates on it.
  The old `1.0 mm` floor separately made every sub-2 mm part un-resolvable by
  collapsing all anchor similarities towards 1.
  **Fixture bump — 2 files.** `worker/tests/fixtures/executeplan_needsrepair.ndjson`
  and `worker/tests/fixtures/resolve_refs.ndjson` pin `scoringVersion` and move
  `1` → `2`. No shape, signature, id or geometry moves: the NeedsRepair payloads are
  otherwise byte-identical and both files are subset matchers. `protocol/fixtures/`
  (hello/echo only) is untouched, and no `policyVersions.resolverVersion` on the
  wire changes — §10 documents why the pinned axis and the reported `scoringVersion`
  are allowed to differ.

- **2026-08-04 — §7.5 `AcquireElementIds` MUST refuse a stale `snapshotId`**
  (HISTORY-HARDEN H4, VF-M3; cross-track sign-off recorded in `TODO.md` by the
  HISTORY-HARDEN H4 gate). **Additive semantics on an existing field — no wire
  shape change.** `snapshotId` has always ridden in the request and was ignored by
  the worker; it now gates the verb: present-and-not-head ⇒ `REF_UNRESOLVED`
  (`detail: {requested, head}`), absent ⇒ unchanged behaviour. Rationale: a
  `TopoKey` is a 1-based ordinal into `TopExp::MapShapes`, so a pick captured before
  a regen names a *different* face after it; resolving it anyway minted a
  persistent, op-referencable id for geometry the user never picked — the H5-B
  silent-wrong-bind class. Rust holds the primary gate (`DocumentRuntime::
  promote_selection` refuses against its own published head, skipping the check
  when the head is `0` — a cleared document publishes no worker snapshot id); the
  worker's is defense-in-depth for a drifted head.
  **No fixture bump.** No shape, signature or id moves; every existing caller
  already sends the head id (`worker/tests/fixtures/tessellate_acquire.ndjson`
  sends `snapshotId:1` against head `1`), and the NDJSON fixtures are subset
  matchers, so no golden file changed.

- **2026-08-03 — §7.2 ADDITIVE `bodyEvents[].rankKey` + §9 `reason:
  "ordinal-permutation"`** (WP-FIX W5, VF-B6; cross-track sign-off recorded in
  `TODO.md` by the WP-FIX gate). Purely **additive on both**. §7.2: an N-body op
  whose child ordinals are a geometric rank now publishes the quantized 5-tuple key
  `[volume, cx, cy, cz, faceCount]` it ranked each solid by; absence stays legal and
  means "no claim". §9: a fourth `reason` token, **Rust-seeded only**, plus a
  normative "tolerate unknown `reason`" rule for readers. Rationale: `body_<opId>:<k>`
  is stable in name but `k` is a geometric rank, so an edit that crosses two split
  pieces' rank order silently re-points `:0` at a different solid and every
  downstream ref re-resolves *cleanly* to the wrong body (H5-B class). With the key
  published, Rust detects the permutation at adoption and raises a deterministic
  `NeedsRepair` instead.
  **No fixture bump.** The ordinal assignment itself is byte-identical (the same
  `stable_sort` over the same `llround(v * 1e6)` lexicographic tuple — the key was
  always computed, it was just discarded inside the comparator), so no shape moves,
  no signature moves, and no `:<k>` id changes. The NDJSON fixtures under
  `worker/tests/fixtures/` and `protocol/fixtures/` are **subset** matchers, so the
  extra key on split-producing steps is tolerated by construction; no golden file
  changed.

- **2026-08-03 — §7.7 checkpoints are IN-SESSION ONLY** (WP-FIX W2, VF-B3;
  cross-track sign-off recorded in `TODO.md` by the WP-FIX gate). Doc-only
  clarification of an existing constraint: `RestoreCheckpoint` has always resolved
  `checkpointId` against the worker's in-session map, so a container-persisted
  checkpoint could only ever answer `restored:false` — costing container growth and a
  guaranteed replay detour, and (with a stale D1 known-op set) wedging the
  reopen→append regen entirely. The app therefore no longer WRITES the container's
  `checkpoints/` cache and ignores it on read; the section stays in the container
  format and readers must tolerate present-or-absent. **No wire shape changes, no
  worker changes — no fixture bump.**

- **2026-08-03 — §7.3 NEW op `Hole`** (WP-C T3; cross-track sign-off recorded
  2026-08-03). Simple/counterbore/countersink as one parametric feature on a
  planar face; frozen world `point` re-projected + fenced 1e-3; inward-normal
  axis; modified-host lineage; standards tables frontend-only (raw mm on the
  wire). Additive — **no fixture bump**.

- **2026-08-03 — §7.3 Chamfer `distance2` (two-distance chamfer)** (WP-C
  tranche 2; cross-track sign-off recorded 2026-08-03). Optional skip-none
  Chamfer-only field; deterministic reference-face rule (smaller resolved face
  ordinal); absent = equal-leg byte-identical; Fillet⇄Chamfer type-flip
  rejected while set (field-identity precondition of the sanctioned pair).
  Additive — **no fixture bump**.

- **2026-08-02 — §7.5 NEW read-only verb `QueryMassProperties`** (WP-C measure
  upgrades; cross-track sign-off recorded 2026-08-02). Exact GProp
  volume/area/centroid/principal-inertia for one body, density-free, no session
  mutation. Additive verb, no existing shape moves, fixtures untouched — **no
  fixture bump**.

- **2026-08-02 — §7.3 NEW op `TransformBody`** (WP-B W0 BODY-TRANSFORM;
  cross-track sign-off recorded 2026-08-02). Rigid multi-target placement with a
  frozen pivot, normative `T ∘ R` evaluation, modify lineage for `copy:false`
  (first body-level modify-lineage op — partition level-1 rebind + rigid
  `anchor` migration keep tracked elements exact with zero scoring), §2 N-body
  minting for `copy:true`, and the V1 edit-safety gate: transform param
  edit/suppress seeds `NeedsRepair` on downstream lineage refs (descriptor
  scoring against moved geometry admits congruent-decoy wrong binds — H5-B
  class; gate holds until anchor pick-frame compensation ships). Additive op,
  no wire byte moves, `protocol/fixtures/` untouched — **no fixture bump**.

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
  step angle `angleDeg/count`; mirror: `gp_Trsf::SetMirror`). At M6a, **Shell**
  replaced its body (Modify lineage; OCCT history → partition) and resolved
  open-face refs, originally frozen as bare ids, via the partition-tracked binding
  or the [§10](#10-resolution-ladder) ladder — a face that resolves via neither ⇒
  NeedsRepair ([§9](#9-needsrepair-payload)). This historical bare-only shape
  remains the legacy empty-`faces` case; the 2026-08-09 REF-H0 entry supersedes it
  for new authoring with typed lockstep evidence.
  **V1 Patterns/MirrorBody** mint ONE new `body_<opId>` (NewBody lineage; source
  preserved; empty `elementMapDelta`). Pattern V2 is superseded by the 2026-08-11
  entry above. No `protocolVersion` bump (still 1 —
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
  regionId, regionIdentityVersion?}`) to top-level `params.sketchId` +
  `params.regionId` + optional `params.regionIdentityVersion` (dropping the
  `profile` wrapper — §7.3 has no `profile`; Extrude AND Revolve), and the worker's
  `build_profile_face` selects the closed region whose normative FNV `regionId`
  ([§7.4](#74-sketch-solver-lane) `derive_region_id`, `r_<hash>`) matches. **Strict
  semantics:** a **non-empty** `regionId` MUST match a detected region — **no match is
  a deterministic `OP_FAILED`** (the `perStepResults` message names the requested id +
  the available ids; downstream is blocked, publish ≤ m−1), **never** a silent fallback
  to a different region (a stale id after a sketch edit must fail loudly, not extrude a
  wrong profile — the "never a silent wrong bind" principle). An **empty/absent**
  `regionId` keeps the V1 **first-region** fallback only when
  `regionIdentityVersion` is absent (single-region sketches; the region-selection
  micro-slice does not yet author a real id everywhere). Additive:
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
