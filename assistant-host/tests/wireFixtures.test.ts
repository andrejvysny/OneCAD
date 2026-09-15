/**
 * The sidecar half of the shared adverse-wire fixture corpus
 * (`docs/assistant/wire-fixtures/`, contract §2a and §10).
 *
 * The Rust host runs the SAME files, in
 * `src-tauri/src/assistant/bridge.rs::every_shared_wire_fixture_produces_the_contracted_outcome`.
 * That is the whole point of the corpus: F09 was not one bug but two peers that
 * had each decided something reasonable and different about the same bytes, and
 * no test either of them owned could have noticed. Only a case both dispatchers
 * execute can.
 *
 * Ids in a fixture are symbolic. `self:N` is the Nth id in the RECEIVING peer's
 * own space — odd here, even in the host — and `peer:N` is the Nth in the
 * sender's. One corpus therefore describes both directions with no hard-coded
 * integer to keep in sync.
 */
import { afterEach, expect, test } from "bun:test";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { Buffer } from "node:buffer";
import { BridgePeer, type HandlerResult, type VerbRoute } from "../src/bridge/peer.js";
import { FRAME_HEADER_LEN, type Envelope } from "../src/bridge/frame.js";
import { HostMock, Pipe } from "./harness.js";

const FIXTURE_DIR = path.join(import.meta.dir, "../../docs/assistant/wire-fixtures");

/** The verb the `@inflight` placeholder resolves to on this side. */
const INFLIGHT_VERB = "test.inflight";

interface Fixture {
  name: string;
  rule: string;
  setup: "none" | "outboundStream" | "inboundInFlight";
  deliver: Record<string, unknown>[];
  expect: "close" | "refuse" | "discard";
  refusal?: { id: string; code: string };
}

/** §3, seen from the sidecar: this peer allocates ODD ids, the host EVEN ones. */
function resolveToken(token: string): number {
  const [space, index] = token.split(":");
  const n = Number(index);
  if (space === "self") return n * 2 + 1;
  if (space === "peer") return n * 2;
  throw new Error(`unknown id space ${token}`);
}

/**
 * A frame built WITHOUT `encodeFrame`.
 *
 * The encoder refuses the very frames these cases are made of — a tail on a
 * control envelope, a chunk with none, a duplicate key. Going around it is what
 * lets the corpus test the decoder rather than the encoder.
 */
function rawFrame(json: string, bin: Uint8Array): Uint8Array {
  const jsonBytes = new TextEncoder().encode(json);
  const frame = new Uint8Array(FRAME_HEADER_LEN + jsonBytes.byteLength + bin.byteLength);
  frame.set(new TextEncoder().encode("OCAK"), 0);
  const header = new DataView(frame.buffer, 0, FRAME_HEADER_LEN);
  header.setUint32(4, jsonBytes.byteLength, true);
  header.setUint32(8, bin.byteLength, true);
  frame.set(jsonBytes, FRAME_HEADER_LEN);
  frame.set(bin, FRAME_HEADER_LEN + jsonBytes.byteLength);
  return frame;
}

function fixtureFrame(spec: Record<string, unknown>): Uint8Array {
  if (typeof spec.json === "string") {
    // A literal: a duplicate key and an out-of-range id cannot be expressed as
    // structured JSON, so those cases name their bytes exactly.
    return rawFrame(spec.json, new Uint8Array(0));
  }
  const envelope: Record<string, unknown> = { ...spec };
  const bin =
    typeof envelope.bin === "string"
      ? new Uint8Array(Buffer.from(envelope.bin, "base64"))
      : new Uint8Array(0);
  delete envelope.bin;
  if (typeof envelope.id === "string") envelope.id = resolveToken(envelope.id);
  if (envelope.verb === "@inflight") {
    envelope.verb = INFLIGHT_VERB;
    envelope.payload = {};
  }
  return rawFrame(JSON.stringify(envelope), bin);
}

const open: { close(): void }[] = [];
afterEach(() => {
  for (const wired of open.splice(0)) wired.close();
});

function withTimeout<T>(promise: Promise<T>, ms: number, what: string): Promise<T> {
  return Promise.race([
    promise,
    new Promise<T>((_, reject) => setTimeout(() => reject(new Error(`timed out: ${what}`)), ms)),
  ]);
}

/** A route that answers nothing, so its id stays genuinely in flight. */
const inflightRoute: VerbRoute = {
  principals: ["ui", "host"],
  handle(): Promise<HandlerResult> {
    return new Promise<HandlerResult>(() => {});
  },
};

async function runFixture(fixture: Fixture): Promise<void> {
  const toSidecar = new Pipe();
  const fromSidecar = new Pipe();
  const host = new HostMock(toSidecar, fromSidecar);
  const peer = new BridgePeer({
    input: toSidecar,
    write: fromSidecar.write,
    identity: { hostVersion: "0.1.0", sessionNonce: "fixture" },
    routes: { [INFLIGHT_VERB]: inflightRoute },
  });
  open.push({
    close: () => {
      peer.close(new Error("fixture over"));
      toSidecar.close();
      fromSidecar.close();
    },
  });
  await peer.start();

  let held: Promise<unknown> | undefined;
  if (fixture.setup === "outboundStream") {
    // Issued and then HELD: the per-request state machine needs a request to be
    // in a state about. The rejection is expected on the fatal cases.
    held = peer.requestStream("provider.fetch", {}).catch(() => undefined);
    await withTimeout(
      host.waitFor(resolveToken("self:0"), (f) => f.envelope.t === "req"),
      2000,
      `${fixture.name}: the sidecar's req`,
    );
  } else if (fixture.setup === "inboundInFlight") {
    await host.send({
      t: "req",
      id: resolveToken("peer:0"),
      principal: "ui",
      verb: INFLIGHT_VERB,
      payload: {},
    });
    // One turn of the loop, so the handler is registered before the duplicate.
    await new Promise((resolve) => setTimeout(resolve, 5));
  }

  for (const spec of fixture.deliver) await toSidecar.write(fixtureFrame(spec));

  // The liveness probe. A connection that is still open answers it; one that
  // tore down never will, and that difference is the whole assertion.
  const probe = resolveToken("peer:9");
  await toSidecar.write(fixtureFrame({ t: "ping", id: "peer:9" }));

  const sawPong = (): boolean =>
    host.received.some((f) => f.envelope.t === "pong" && f.envelope.id === probe);

  if (fixture.expect === "close") {
    await withTimeout(peer.waitForClose(), 2000, `${fixture.name}: must tear down`);
    expect(peer.isOpen).toBe(false);
    expect(sawPong()).toBe(false);
  } else {
    if (fixture.expect === "refuse") {
      const refusal = fixture.refusal!;
      const frame = await withTimeout(
        host.waitFor(
          resolveToken(refusal.id),
          (f) => f.envelope.t === "res" && f.envelope.ok === false,
        ),
        2000,
        `${fixture.name}: refusal`,
      );
      const envelope = frame.envelope as Extract<Envelope, { t: "res"; ok: false }>;
      expect(envelope.error.code).toBe(refusal.code);
    }
    await withTimeout(
      host.waitFor(probe, (f) => f.envelope.t === "pong"),
      2000,
      `${fixture.name}: the probe must be answered`,
    );
    expect(peer.isOpen).toBe(true);
  }
  void held;
}

const fixtures: Fixture[] = readdirSync(FIXTURE_DIR)
  .filter((name) => name.endsWith(".json"))
  .sort()
  .map((name) => JSON.parse(readFileSync(path.join(FIXTURE_DIR, name), "utf8")) as Fixture);

test("the shared corpus is present on this side too", () => {
  expect(fixtures.length).toBeGreaterThanOrEqual(14);
});

for (const fixture of fixtures) {
  test(`wire fixture: ${fixture.name}`, async () => {
    await runFixture(fixture);
  });
}
