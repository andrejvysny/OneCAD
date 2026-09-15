/**
 * OCAK1 framing and envelope validation — the TypeScript half of
 * `docs/assistant/wire-protocol.md` §1–§2. The Rust half lives in
 * `src-tauri/crates/onecad-assistant-protocol`; the two are mirrors, and the
 * document, not either implementation, is normative.
 *
 * Two rules drive every decision in this file:
 *
 *  - **A malformed frame is fatal.** Bad magic, an over-cap length, an unknown
 *    `t`, a missing field: all of them throw {@link FrameError}, and the caller
 *    tears the connection down. There is no resync path because a
 *    desynchronised length prefix cannot be recovered from without guessing
 *    where the next header starts, and a wrong guess turns a protocol error
 *    into silent data corruption.
 *  - **Nothing is inferred.** An envelope is accepted only when every field the
 *    spec names is present and of the right type. An "unknown `t`" that is
 *    quietly ignored is how a stream loses an event nobody notices.
 */

export const OCAK_MAGIC = "OCAK";
export const FRAME_HEADER_LEN = 12;

/** §1: the control-frame cap. A JSON envelope larger than this is malformed. */
export const MAX_JSON_LEN = 1024 * 1024;
/** §1: one stream chunk. Larger payloads are split across `chunk` frames. */
export const MAX_BIN_LEN = 8 * 1024 * 1024;

/** §4. Asserted by the transport a request arrived on, never by its payload. */
export type Principal = "ui" | "host";

export interface BridgeErrorBody {
  code: string;
  message: string;
}

export interface HelloEnvelope {
  t: "hello";
  protocolVersion: number;
  hostVersion: string;
  agentkitContractVersion: string;
  pid: number;
  sessionNonce: string;
}

export interface AcceptEnvelope {
  t: "accept";
  protocolVersion: number;
  appVersion: string;
  bridgeVersion: string;
}

export interface RejectEnvelope {
  t: "reject";
  reason: string;
}

export interface RequestEnvelope {
  t: "req";
  id: number;
  principal: Principal;
  verb: string;
  payload: unknown;
}

export type ResponseEnvelope =
  | { t: "res"; id: number; ok: true; payload?: unknown }
  | { t: "res"; id: number; ok: false; error: BridgeErrorBody };

export interface ChunkEnvelope {
  t: "chunk";
  id: number;
  seq: number;
}

export type EndEnvelope =
  | { t: "end"; id: number; ok: true }
  | { t: "end"; id: number; ok: false; error: BridgeErrorBody };

export interface CancelEnvelope {
  t: "cancel";
  id: number;
}

export interface PingEnvelope {
  t: "ping";
  id: number;
}

export interface PongEnvelope {
  t: "pong";
  id: number;
}

export type Envelope =
  | HelloEnvelope
  | AcceptEnvelope
  | RejectEnvelope
  | RequestEnvelope
  | ResponseEnvelope
  | ChunkEnvelope
  | EndEnvelope
  | CancelEnvelope
  | PingEnvelope
  | PongEnvelope;

export interface Frame {
  envelope: Envelope;
  /** Empty on every envelope except `chunk` — enforced on both encode and decode. */
  bin: Uint8Array;
}

/** Fatal by construction: every throw site here ends the connection. */
export class FrameError extends Error {
  override readonly name = "FrameError";
}

const EMPTY = new Uint8Array(0);
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

export function encodeFrame(envelope: Envelope, bin: Uint8Array = EMPTY): Uint8Array {
  if (envelope.t !== "chunk" && bin.byteLength !== 0) {
    throw new FrameError(`bin tail is only valid on chunk, got ${envelope.t}`);
  }
  // §2a: a chunk with no tail is fatal at the receiver, so producing one here
  // would be this process inventing a frame the peer must tear down over.
  if (envelope.t === "chunk" && bin.byteLength === 0) {
    throw new FrameError(`chunk ${envelope.seq} on request ${envelope.id} has no bin tail`);
  }
  const json = encoder.encode(JSON.stringify(envelope));
  if (json.byteLength > MAX_JSON_LEN) {
    throw new FrameError(`json envelope of ${json.byteLength} exceeds MAX_JSON_LEN`);
  }
  if (bin.byteLength > MAX_BIN_LEN) {
    throw new FrameError(`bin tail of ${bin.byteLength} exceeds MAX_BIN_LEN`);
  }
  const frame = new Uint8Array(FRAME_HEADER_LEN + json.byteLength + bin.byteLength);
  frame[0] = 0x4f; // O
  frame[1] = 0x43; // C
  frame[2] = 0x41; // A
  frame[3] = 0x4b; // K
  const header = new DataView(frame.buffer, frame.byteOffset, FRAME_HEADER_LEN);
  header.setUint32(4, json.byteLength, true);
  header.setUint32(8, bin.byteLength, true);
  frame.set(json, FRAME_HEADER_LEN);
  if (bin.byteLength !== 0) frame.set(bin, FRAME_HEADER_LEN + json.byteLength);
  return frame;
}

/**
 * Accumulates bytes from a pipe and yields whole frames.
 *
 * A pipe read boundary has nothing to do with a frame boundary, so the decoder
 * keeps a carry buffer and emits only complete frames. It is deliberately a
 * class with mutable state rather than an async generator: the peer's reader
 * loop must stay synchronous between `push` and dispatch so that a frame is
 * never held while an unrelated promise settles (§6).
 */
export class FrameDecoder {
  private carry: Uint8Array = EMPTY;

  push(chunk: Uint8Array): Frame[] {
    this.carry = this.carry.byteLength === 0 ? chunk : concat(this.carry, chunk);
    const frames: Frame[] = [];
    let offset = 0;
    for (;;) {
      const available = this.carry.byteLength - offset;
      if (available < FRAME_HEADER_LEN) break;
      const view = new DataView(this.carry.buffer, this.carry.byteOffset + offset, available);
      if (
        this.carry[offset] !== 0x4f ||
        this.carry[offset + 1] !== 0x43 ||
        this.carry[offset + 2] !== 0x41 ||
        this.carry[offset + 3] !== 0x4b
      ) {
        throw new FrameError("bad frame magic: expected OCAK");
      }
      const jsonLen = view.getUint32(4, true);
      const binLen = view.getUint32(8, true);
      // Checked BEFORE waiting for the body: a corrupt length must not make the
      // decoder sit on a 4 GiB carry buffer waiting for bytes that never come.
      if (jsonLen > MAX_JSON_LEN) {
        throw new FrameError(`jsonLen ${jsonLen} exceeds MAX_JSON_LEN`);
      }
      if (binLen > MAX_BIN_LEN) {
        throw new FrameError(`binLen ${binLen} exceeds MAX_BIN_LEN`);
      }
      const total = FRAME_HEADER_LEN + jsonLen + binLen;
      if (available < total) break;

      const jsonStart = offset + FRAME_HEADER_LEN;
      const binStart = jsonStart + jsonLen;
      const envelope = parseEnvelope(
        decodeJson(this.carry.subarray(jsonStart, binStart)),
      );
      // Copied, not sub-viewed: the carry buffer is reallocated on the next
      // push, and a chunk handed downstream outlives this call.
      const bin = binLen === 0 ? EMPTY : this.carry.slice(binStart, binStart + binLen);
      // §2a, both directions of the same rule: the tail belongs to `chunk` and
      // to nothing else, and a `chunk` that carries none is not a chunk. An
      // empty one costs a queue slot and a wakeup while charging nothing against
      // any byte bound (§9).
      if (envelope.t !== "chunk" && binLen !== 0) {
        throw new FrameError(`bin tail on a ${envelope.t} envelope`);
      }
      if (envelope.t === "chunk" && binLen === 0) {
        throw new FrameError(`chunk ${envelope.seq} on request ${envelope.id} has no bin tail`);
      }
      frames.push({ envelope, bin });
      offset += total;
    }
    this.carry = offset === 0 ? this.carry : this.carry.slice(offset);
    return frames;
  }

  /** Bytes held back waiting for the rest of a frame. Read by tests only. */
  get pending(): number {
    return this.carry.byteLength;
  }
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.byteLength + b.byteLength);
  out.set(a, 0);
  out.set(b, a.byteLength);
  return out;
}

function decodeJson(bytes: Uint8Array): unknown {
  let text: string;
  try {
    text = decoder.decode(bytes);
  } catch {
    throw new FrameError("json payload is not valid UTF-8");
  }
  let value: unknown;
  try {
    value = JSON.parse(text) as unknown;
  } catch (err) {
    throw new FrameError(`json payload is not parseable: ${(err as Error).message}`);
  }
  const duplicate = findDuplicateEnvelopeKey(text);
  if (duplicate !== null) {
    throw new FrameError(`json envelope repeats the key '${duplicate}'`);
  }
  return value;
}

/**
 * §2a's duplicate-key policy, enforced on the ENVELOPE's own object only.
 *
 * `JSON.parse` silently keeps the last of two identical keys; Rust's serde
 * refuses the object outright. One of those had to win, and refusing does:
 * a frame whose meaning depends on which parser read it is a frame neither
 * peer can reason about. The scan stops at depth 1 because `payload` and any
 * verb-specific subtree are opaque to this layer — both platforms resolve a
 * duplicate there as last-wins, and that is the documented half of the policy.
 *
 * Cost is one pass over at most `MAX_JSON_LEN` bytes of already-validated JSON,
 * so it cannot loop, throw, or outrun the frame budget.
 */
function findDuplicateEnvelopeKey(text: string): string | null {
  const seen = new Set<string>();
  let depth = 0;
  let i = 0;
  let expectKey = false;
  while (i < text.length) {
    const ch = text[i];
    if (ch === '"') {
      const start = i;
      i += 1;
      while (i < text.length) {
        if (text[i] === "\\") {
          i += 2;
          continue;
        }
        if (text[i] === '"') break;
        i += 1;
      }
      const literal = text.slice(start, i + 1);
      i += 1;
      // A string is a KEY when it is followed by a colon at depth 1.
      let j = i;
      while (j < text.length && /\s/.test(text[j]!)) j += 1;
      if (depth === 1 && expectKey && text[j] === ":") {
        const key = JSON.parse(literal) as string;
        if (seen.has(key)) return key;
        seen.add(key);
      }
      continue;
    }
    if (ch === "{" || ch === "[") {
      depth += 1;
      expectKey = ch === "{";
    } else if (ch === "}" || ch === "]") {
      depth -= 1;
      expectKey = false;
    } else if (ch === ",") {
      expectKey = depth === 1;
    }
    i += 1;
  }
  return null;
}

function asRecord(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new FrameError("json payload is not an object");
  }
  return value as Record<string, unknown>;
}

function str(record: Record<string, unknown>, key: string): string {
  const value = record[key];
  if (typeof value !== "string") throw new FrameError(`missing string field '${key}'`);
  return value;
}

/**
 * §3: ids are `u64`. A JSON number above `Number.MAX_SAFE_INTEGER` cannot be
 * compared for equality against the map key it is supposed to match, so it is
 * refused here rather than silently aliasing a different request.
 */
function id(record: Record<string, unknown>, key: string): number {
  const value = record[key];
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new FrameError(`field '${key}' is not a u64 within the safe integer range`);
  }
  return value;
}

function ok(record: Record<string, unknown>): boolean {
  const value = record.ok;
  if (typeof value !== "boolean") throw new FrameError("missing boolean field 'ok'");
  return value;
}

function errorBody(record: Record<string, unknown>): BridgeErrorBody {
  const body = asRecord(record.error);
  return { code: str(body, "code"), message: str(body, "message") };
}

export function parseEnvelope(value: unknown): Envelope {
  const record = asRecord(value);
  switch (record.t) {
    case "hello":
      return {
        t: "hello",
        protocolVersion: id(record, "protocolVersion"),
        hostVersion: str(record, "hostVersion"),
        agentkitContractVersion: str(record, "agentkitContractVersion"),
        pid: id(record, "pid"),
        sessionNonce: str(record, "sessionNonce"),
      };
    case "accept":
      return {
        t: "accept",
        protocolVersion: id(record, "protocolVersion"),
        appVersion: str(record, "appVersion"),
        bridgeVersion: str(record, "bridgeVersion"),
      };
    case "reject":
      return { t: "reject", reason: str(record, "reason") };
    case "req": {
      const principal = str(record, "principal");
      if (principal !== "ui" && principal !== "host") {
        throw new FrameError(`unknown principal '${principal}'`);
      }
      return {
        t: "req",
        id: id(record, "id"),
        principal,
        verb: str(record, "verb"),
        payload: record.payload,
      };
    }
    case "res":
      return ok(record)
        ? { t: "res", id: id(record, "id"), ok: true, payload: record.payload }
        : { t: "res", id: id(record, "id"), ok: false, error: errorBody(record) };
    case "chunk":
      return { t: "chunk", id: id(record, "id"), seq: id(record, "seq") };
    case "end":
      return ok(record)
        ? { t: "end", id: id(record, "id"), ok: true }
        : { t: "end", id: id(record, "id"), ok: false, error: errorBody(record) };
    case "cancel":
      return { t: "cancel", id: id(record, "id") };
    case "ping":
      return { t: "ping", id: id(record, "id") };
    case "pong":
      return { t: "pong", id: id(record, "id") };
    default:
      throw new FrameError(`unknown envelope tag ${JSON.stringify(record.t)}`);
  }
}
