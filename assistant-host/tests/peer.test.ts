import { afterEach, describe, expect, test } from "bun:test";
import { BridgePeer, BridgeRequestError, type VerbRoute } from "../src/bridge/peer.js";
import { encodeFrame } from "../src/bridge/frame.js";
import { HostMock, Pipe, bytes, text } from "./harness.js";

interface Wired {
  peer: BridgePeer;
  host: HostMock;
  toSidecar: Pipe;
  fromSidecar: Pipe;
}

const open: Wired[] = [];

afterEach(() => {
  for (const wired of open.splice(0)) {
    wired.peer.close(new Error("test over"));
    wired.toSidecar.close();
    wired.fromSidecar.close();
  }
});

async function wire(
  routes: Record<string, VerbRoute> = {},
  options: { maxStreamBufferBytes?: number; autoAccept?: boolean } = {},
): Promise<Wired> {
  const toSidecar = new Pipe();
  const fromSidecar = new Pipe();
  const host = new HostMock(toSidecar, fromSidecar);
  if (options.autoAccept === false) host.autoAccept = false;
  const peer = new BridgePeer({
    input: toSidecar,
    write: fromSidecar.write,
    identity: { hostVersion: "0.1.0", sessionNonce: "test-nonce" },
    routes,
    ...(options.maxStreamBufferBytes === undefined
      ? {}
      : { maxStreamBufferBytes: options.maxStreamBufferBytes }),
  });
  const wired: Wired = { peer, host, toSidecar, fromSidecar };
  open.push(wired);
  if (options.autoAccept !== false) await peer.start();
  return wired;
}

function echoRoute(): VerbRoute {
  return {
    principals: ["ui"],
    async handle(request) {
      return { kind: "unary", payload: { echoed: request.payload } };
    },
  };
}

describe("handshake", () => {
  test("hello goes out first, carries the pid and nonce, and is answered by accept", async () => {
    const { peer, host } = await wire();
    expect(host.received[0]?.envelope.t).toBe("hello");
    expect(host.helloSeen).toMatchObject({
      t: "hello",
      protocolVersion: 1,
      hostVersion: "0.1.0",
      pid: process.pid,
      sessionNonce: "test-nonce",
    });
    expect(peer.isOpen).toBe(true);
  });

  test("a reject tears the connection down instead of proceeding", async () => {
    const { peer, host } = await wire({}, { autoAccept: false });
    const started = peer.start();
    await host.waitFor(-1, (f) => f.envelope.t === "hello");
    await host.send({ t: "reject", reason: "unsupported protocol version 2" });
    await expect(started).rejects.toThrow(/unsupported protocol version 2/);
  });

  test("a frame before accept is fatal", async () => {
    const { peer, host } = await wire({}, { autoAccept: false });
    const started = peer.start();
    await host.waitFor(-1, (f) => f.envelope.t === "hello");
    await host.send({ t: "req", id: 0, principal: "ui", verb: "agentkit.fetch", payload: {} });
    await expect(started).rejects.toThrow(/expected accept or reject, got req/);
  });
});

describe("request routing", () => {
  test("a unary verb answers res{ok:true}", async () => {
    const { host } = await wire({ "test.echo": echoRoute() });
    const answer = await host.call("test.echo", { hello: "world" });
    expect(answer.res.envelope).toMatchObject({
      t: "res",
      ok: true,
      payload: { echoed: { hello: "world" } },
    });
  });

  test("an unknown verb is refused, not ignored", async () => {
    const { host } = await wire({});
    const answer = await host.call("test.nope", {});
    expect(answer.res.envelope).toMatchObject({
      t: "res",
      ok: false,
      error: { code: "unknown_verb" },
    });
  });

  test("a verb the principal may not call is refused by the table, with no wildcard", async () => {
    const { host } = await wire({ "test.echo": echoRoute() });
    const answer = await host.call("test.echo", {}, { principal: "host" });
    expect(answer.res.envelope).toMatchObject({
      t: "res",
      ok: false,
      error: { code: "forbidden" },
    });
  });

  test("a handler throw becomes res{ok:false} with the handler's code when it names one", async () => {
    const { host } = await wire({
      "test.boom": {
        principals: ["ui"],
        async handle() {
          throw new BridgeRequestError("bad_request", "nope");
        },
      },
      "test.crash": {
        principals: ["ui"],
        async handle(): Promise<never> {
          throw new Error("unclassified");
        },
      },
    });
    expect((await host.call("test.boom", {})).res.envelope).toMatchObject({
      ok: false,
      error: { code: "bad_request", message: "nope" },
    });
    expect((await host.call("test.crash", {})).res.envelope).toMatchObject({
      ok: false,
      error: { code: "handler_error", message: "unclassified" },
    });
  });

  test("an inbound request id with sidecar parity is fatal", async () => {
    const { peer, host } = await wire({ "test.echo": echoRoute() });
    const closed = peer.waitForClose();
    await host.send({ t: "req", id: 3, principal: "ui", verb: "test.echo", payload: {} });
    await closed;
    expect(peer.isOpen).toBe(false);
  });

  test("outbound request ids are odd and monotonic (§3)", async () => {
    const { peer, host } = await wire();
    host.routes.set("provider.fetch", async (_payload, id) => {
      await host.send({ t: "res", id, ok: true, payload: { got: id } });
    });
    const first = (await peer.request("provider.fetch", {})) as { got: number };
    const second = (await peer.request("provider.fetch", {})) as { got: number };
    expect(first.got % 2).toBe(1);
    expect(second.got).toBe(first.got + 2);
  });
});

describe("streaming", () => {
  test("a streaming verb answers head, chunks in order, then exactly one end", async () => {
    const { host } = await wire({
      "test.stream": {
        principals: ["ui"],
        async handle() {
          const body = new ReadableStream<Uint8Array>({
            start(controller) {
              controller.enqueue(bytes("one "));
              controller.enqueue(bytes("two"));
              controller.close();
            },
          });
          return { kind: "stream", payload: { status: 200 }, body };
        },
      },
    });
    const answer = await host.call("test.stream", {}, { stream: true });
    expect(answer.res.envelope).toMatchObject({ ok: true, payload: { status: 200 } });
    expect(text(answer.chunks)).toBe("one two");
    expect(answer.end?.envelope).toMatchObject({ t: "end", ok: true });
    const seqs = host.received
      .filter((f) => f.envelope.t === "chunk")
      .map((f) => (f.envelope as { seq: number }).seq);
    expect(seqs).toEqual([0, 1]);
  });

  test("a null body still gets exactly one end", async () => {
    const { host } = await wire({
      "test.empty": {
        principals: ["ui"],
        async handle() {
          return { kind: "stream", payload: { status: 204 }, body: null };
        },
      },
    });
    const answer = await host.call("test.empty", {}, { stream: true });
    expect(answer.chunks).toHaveLength(0);
    expect(answer.end?.envelope).toMatchObject({ t: "end", ok: true });
  });

  test("an outbound stream that outruns its bound ends with stream_overflow (§7)", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const { host, fromSidecar } = await wire(
      {
        "test.flood": {
          principals: ["ui"],
          async handle() {
            const body = new ReadableStream<Uint8Array>({
              start(controller) {
                for (let i = 0; i < 8; i += 1) controller.enqueue(new Uint8Array(64));
                controller.close();
              },
            });
            return { kind: "stream", payload: { status: 200 }, body };
          },
        },
      },
      { maxStreamBufferBytes: 128 },
    );

    // Stall the writer so bytes pile up in the per-stream buffer rather than
    // leaving the process; this is the "host stopped draining stdout" case.
    const realWrite = fromSidecar.write;
    const stalled = fromSidecar as unknown as { write: typeof realWrite };
    stalled.write = async (frame: Uint8Array) => {
      await gate;
      await realWrite(frame);
    };

    const answer = host.call("test.flood", {}, { stream: true });
    await Bun.sleep(20);
    release();
    const result = await answer;
    expect(result.end?.envelope).toMatchObject({
      t: "end",
      ok: false,
      error: { code: "stream_overflow" },
    });
    // Loud, never silent: some chunks arrived, and the stream said it stopped.
    expect(result.chunks.length).toBeGreaterThan(0);
  });

  test("an inbound stream reconstructs the body and closes on end{ok:true}", async () => {
    const { peer, host } = await wire();
    host.routes.set("provider.fetch", async (_payload, id) => {
      await host.send({ t: "res", id, ok: true, payload: { status: 200 } });
      await host.send({ t: "chunk", id, seq: 0 }, bytes("data: hi\n"));
      await host.send({ t: "chunk", id, seq: 1 }, bytes("data: bye\n"));
      await host.send({ t: "end", id, ok: true });
    });
    const answer = await peer.requestStream("provider.fetch", {});
    expect(answer.head).toEqual({ status: 200 });
    expect(await new Response(answer.body).text()).toBe("data: hi\ndata: bye\n");
  });

  test("end{ok:false} errors the inbound stream instead of closing it clean", async () => {
    const { peer, host } = await wire();
    host.routes.set("provider.fetch", async (_payload, id) => {
      await host.send({ t: "res", id, ok: true, payload: { status: 200 } });
      await host.send({
        t: "end",
        id,
        ok: false,
        error: { code: "stream_overflow", message: "host buffer full" },
      });
    });
    const answer = await peer.requestStream("provider.fetch", {});
    await expect(new Response(answer.body).text()).rejects.toThrow(/host buffer full/);
  });

  test("a chunk sequence gap is fatal", async () => {
    const { peer, host } = await wire();
    host.routes.set("provider.fetch", async (_payload, id) => {
      await host.send({ t: "res", id, ok: true, payload: { status: 200 } });
      await host.send({ t: "chunk", id, seq: 0 }, bytes("a"));
      await host.send({ t: "chunk", id, seq: 2 }, bytes("c"));
    });
    const closed = peer.waitForClose();
    await peer.requestStream("provider.fetch", {});
    await closed;
    expect(peer.isOpen).toBe(false);
  });
});

describe("cancel and liveness", () => {
  test("cancel aborts an in-flight inbound request", async () => {
    let aborted = false;
    const { host } = await wire({
      "test.slow": {
        principals: ["ui"],
        async handle(request) {
          await new Promise<void>((resolve) => {
            request.signal.addEventListener("abort", () => {
              aborted = true;
              resolve();
            });
          });
          return { kind: "unary", payload: {} };
        },
      },
    });
    const id = host.nextHostId();
    await host.send({ t: "req", id, principal: "ui", verb: "test.slow", payload: {} });
    await Bun.sleep(5);
    await host.send({ t: "cancel", id });
    await Bun.sleep(5);
    expect(aborted).toBe(true);
    // §2: a cancelled id gets no reply from this side.
    expect(host.received.some((f) => f.envelope.t === "res")).toBe(false);
  });

  test("cancelling an unknown id is a no-op, not an error", async () => {
    const { peer, host } = await wire({ "test.echo": echoRoute() });
    await host.send({ t: "cancel", id: 998 });
    await Bun.sleep(5);
    expect(peer.isOpen).toBe(true);
    expect((await host.call("test.echo", { still: "works" })).res.envelope).toMatchObject({
      ok: true,
    });
  });

  test("an inbound ping is answered with pong carrying the same id", async () => {
    const { host } = await wire();
    await host.send({ t: "ping", id: 40 });
    const pong = await host.waitFor(40, (f) => f.envelope.t === "pong");
    expect(pong.envelope).toEqual({ t: "pong", id: 40 });
  });

  test("close settles every waiter instead of leaving the process looking busy", async () => {
    const { peer } = await wire();
    const pending = peer.request("provider.fetch", {});
    peer.close(new Error("supervisor went away"));
    await expect(pending).rejects.toThrow(/supervisor went away/);
  });
});

describe("duplex (§6)", () => {
  test("a provider callback arriving mid-request does not deadlock the bridge", async () => {
    // The specific failure the spec names. The sidecar is serving an inbound
    // `agentkit.fetch` whose handler issues an outbound `provider.fetch`; the
    // host answers the provider call only AFTER it has seen it, so the only way
    // this resolves is if the reader kept draining while the handler awaited.
    let sawProviderCall = false;
    const toSidecar = new Pipe();
    const fromSidecar = new Pipe();
    const host = new HostMock(toSidecar, fromSidecar);
    const peerRef: { current?: BridgePeer } = {};
    const peer = new BridgePeer({
      input: toSidecar,
      write: fromSidecar.write,
      identity: { hostVersion: "0.1.0", sessionNonce: "n" },
      routes: {
        "agentkit.fetch": {
          principals: ["ui"],
          async handle() {
            const reply = (await peerRef.current!.request("provider.fetch", {
              providerId: "p",
            })) as { answer: string };
            return { kind: "unary", payload: { via: reply.answer } };
          },
        },
      },
    });
    peerRef.current = peer;
    open.push({ peer, host, toSidecar, fromSidecar });

    host.routes.set("provider.fetch", async (_payload, id) => {
      sawProviderCall = true;
      await host.send({ t: "res", id, ok: true, payload: { answer: "model said hi" } });
    });

    await peer.start();
    const answer = await host.call("agentkit.fetch", { method: "POST", path: "/v1/x" });
    expect(sawProviderCall).toBe(true);
    expect(answer.res.envelope).toMatchObject({ ok: true, payload: { via: "model said hi" } });
  });

  test("a ping is answered while an outbound request is still pending", async () => {
    const { peer, host } = await wire();
    let releaseProvider!: (id: number) => void;
    host.routes.set("provider.fetch", async (_payload, id) => {
      releaseProvider(id);
    });
    const pendingId = new Promise<number>((resolve) => {
      releaseProvider = resolve;
    });
    const pending = peer.request("provider.fetch", {});
    const id = await pendingId;

    await host.send({ t: "ping", id: 90 });
    const pong = await host.waitFor(90, (f) => f.envelope.t === "pong");
    expect(pong.envelope.t).toBe("pong");

    await host.send({ t: "res", id, ok: true, payload: { done: true } });
    expect(await pending).toEqual({ done: true });
  });
});

describe("teardown", () => {
  test("a malformed frame tears the connection down with no resync", async () => {
    const { peer, toSidecar } = await wire();
    const closed = peer.waitForClose();
    const good = encodeFrame({ t: "ping", id: 50 });
    const corrupt = new Uint8Array(good);
    corrupt[0] = 0x00;
    await toSidecar.write(corrupt);
    await closed;
    expect(peer.isOpen).toBe(false);
  });

  test("stdin closing ends the connection", async () => {
    const { peer, toSidecar } = await wire();
    const closed = peer.waitForClose();
    toSidecar.close();
    await closed;
    expect(peer.isOpen).toBe(false);
  });
});
