import { describe, expect, test } from "bun:test";
import {
  FrameDecoder,
  FrameError,
  MAX_BIN_LEN,
  MAX_JSON_LEN,
  encodeFrame,
  parseEnvelope,
  type Envelope,
} from "../src/bridge/frame.js";

function decodeOne(bytes: Uint8Array): { envelope: Envelope; bin: Uint8Array } {
  const frames = new FrameDecoder().push(bytes);
  expect(frames).toHaveLength(1);
  return frames[0]!;
}

describe("OCAK1 framing", () => {
  test("header layout matches the spec table", () => {
    const frame = encodeFrame({ t: "ping", id: 3 });
    expect(new TextDecoder().decode(frame.subarray(0, 4))).toBe("OCAK");
    const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
    expect(view.getUint32(4, true)).toBe(frame.byteLength - 12);
    expect(view.getUint32(8, true)).toBe(0);
  });

  test("round-trips every envelope tag", () => {
    const envelopes: Envelope[] = [
      {
        t: "hello",
        protocolVersion: 1,
        hostVersion: "0.1.0",
        agentkitContractVersion: "0.5.0",
        pid: 42,
        sessionNonce: "n",
      },
      { t: "accept", protocolVersion: 1, appVersion: "1.2.3", bridgeVersion: "0.1.0" },
      { t: "reject", reason: "unsupported protocol version 2" },
      { t: "req", id: 2, principal: "ui", verb: "agentkit.fetch", payload: { a: 1 } },
      { t: "res", id: 2, ok: true, payload: { b: 2 } },
      { t: "res", id: 2, ok: false, error: { code: "x", message: "y" } },
      { t: "end", id: 2, ok: true },
      { t: "end", id: 2, ok: false, error: { code: "stream_overflow", message: "m" } },
      { t: "cancel", id: 2 },
      { t: "ping", id: 7 },
      { t: "pong", id: 7 },
    ];
    for (const envelope of envelopes) {
      expect(decodeOne(encodeFrame(envelope)).envelope).toEqual(envelope);
    }
  });

  test("a chunk carries its bytes in the bin tail", () => {
    const payload = new Uint8Array([1, 2, 3, 4]);
    const frame = decodeOne(encodeFrame({ t: "chunk", id: 4, seq: 0 }, payload));
    expect(frame.envelope).toEqual({ t: "chunk", id: 4, seq: 0 });
    expect(Array.from(frame.bin)).toEqual([1, 2, 3, 4]);
  });

  test("a bin tail on any other envelope is refused on encode and on decode", () => {
    expect(() => encodeFrame({ t: "ping", id: 1 }, new Uint8Array([1]))).toThrow(FrameError);

    // Hand-built, because the encoder will not produce one.
    const json = new TextEncoder().encode(JSON.stringify({ t: "ping", id: 1 }));
    const frame = new Uint8Array(12 + json.byteLength + 1);
    frame.set(new TextEncoder().encode("OCAK"), 0);
    new DataView(frame.buffer).setUint32(4, json.byteLength, true);
    new DataView(frame.buffer).setUint32(8, 1, true);
    frame.set(json, 12);
    expect(() => new FrameDecoder().push(frame)).toThrow(/bin tail on a ping/);
  });

  test("bad magic is fatal", () => {
    const frame = encodeFrame({ t: "ping", id: 1 });
    frame[0] = 0x58;
    expect(() => new FrameDecoder().push(frame)).toThrow(/bad frame magic/);
  });

  test("an over-cap length is refused before the body is waited for", () => {
    const header = new Uint8Array(12);
    header.set(new TextEncoder().encode("OCAK"), 0);
    const view = new DataView(header.buffer);
    view.setUint32(4, MAX_JSON_LEN + 1, true);
    expect(() => new FrameDecoder().push(header)).toThrow(/exceeds MAX_JSON_LEN/);

    const binHeader = new Uint8Array(12);
    binHeader.set(new TextEncoder().encode("OCAK"), 0);
    const binView = new DataView(binHeader.buffer);
    binView.setUint32(4, 2, true);
    binView.setUint32(8, MAX_BIN_LEN + 1, true);
    expect(() => new FrameDecoder().push(binHeader)).toThrow(/exceeds MAX_BIN_LEN/);
  });

  test("unparseable JSON and a non-object payload are fatal", () => {
    const build = (body: string): Uint8Array => {
      const json = new TextEncoder().encode(body);
      const frame = new Uint8Array(12 + json.byteLength);
      frame.set(new TextEncoder().encode("OCAK"), 0);
      new DataView(frame.buffer).setUint32(4, json.byteLength, true);
      frame.set(json, 12);
      return frame;
    };
    expect(() => new FrameDecoder().push(build("{"))).toThrow(/not parseable/);
    expect(() => new FrameDecoder().push(build("[1,2]"))).toThrow(/not an object/);
  });

  test("an unknown tag is a protocol error, never an ignored frame", () => {
    expect(() => parseEnvelope({ t: "telemetry", id: 1 })).toThrow(/unknown envelope tag/);
    expect(() => parseEnvelope({ id: 1 })).toThrow(/unknown envelope tag/);
  });

  test("a missing or mistyped field is refused rather than defaulted", () => {
    expect(() => parseEnvelope({ t: "req", id: 2, verb: "v" })).toThrow(/principal/);
    expect(() => parseEnvelope({ t: "req", id: 2, principal: "root", verb: "v" })).toThrow(
      /unknown principal/,
    );
    expect(() => parseEnvelope({ t: "res", id: 2 })).toThrow(/'ok'/);
    expect(() => parseEnvelope({ t: "res", id: 2, ok: false })).toThrow(/not an object/);
  });

  test("an id outside the safe integer range is refused, not silently aliased", () => {
    expect(() => parseEnvelope({ t: "ping", id: Number.MAX_SAFE_INTEGER + 2 })).toThrow(/u64/);
    expect(() => parseEnvelope({ t: "ping", id: -1 })).toThrow(/u64/);
    expect(() => parseEnvelope({ t: "ping", id: 1.5 })).toThrow(/u64/);
  });

  test("frames are reassembled across arbitrary read boundaries", () => {
    const stream = new Uint8Array([
      ...encodeFrame({ t: "ping", id: 1 }),
      ...encodeFrame({ t: "chunk", id: 2, seq: 0 }, new Uint8Array([9, 9])),
      ...encodeFrame({ t: "end", id: 2, ok: true }),
    ]);
    const decoder = new FrameDecoder();
    const decoded: Envelope[] = [];
    for (let i = 0; i < stream.byteLength; i += 1) {
      for (const frame of decoder.push(stream.subarray(i, i + 1))) decoded.push(frame.envelope);
    }
    expect(decoded.map((e) => e.t)).toEqual(["ping", "chunk", "end"]);
    expect(decoder.pending).toBe(0);
  });

  test("a chunk handed out survives the next push reallocating the carry buffer", () => {
    const decoder = new Uint8Array([
      ...encodeFrame({ t: "chunk", id: 2, seq: 0 }, new Uint8Array([1, 2, 3])),
    ]);
    const d = new FrameDecoder();
    const first = d.push(decoder)[0]!;
    d.push(encodeFrame({ t: "ping", id: 5 }));
    expect(Array.from(first.bin)).toEqual([1, 2, 3]);
  });
});
