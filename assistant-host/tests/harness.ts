/**
 * A host stand-in for the sidecar's peer: the other end of the pipe, speaking
 * OCAK1 by hand out of `frame.ts` rather than through `BridgePeer`.
 *
 * Written against the frame codec directly and NOT against a second
 * `BridgePeer`, because a test in which both ends share an implementation
 * cannot catch the bugs that matter here — a wrong id parity, a `res` the other
 * side does not expect, a `hello` that never goes out. The Rust host will not
 * share this implementation either.
 */
import {
  FrameDecoder,
  encodeFrame,
  type Envelope,
  type Frame,
  type Principal,
} from "../src/bridge/frame.js";

/** A one-directional byte pipe that is also an async iterable of chunks. */
export class Pipe {
  private readonly queue: Uint8Array[] = [];
  private wake: (() => void) | undefined;
  private closed = false;

  readonly write = async (bytes: Uint8Array): Promise<void> => {
    if (this.closed) return;
    this.queue.push(bytes);
    const wake = this.wake;
    this.wake = undefined;
    wake?.();
  };

  close(): void {
    this.closed = true;
    const wake = this.wake;
    this.wake = undefined;
    wake?.();
  }

  async *[Symbol.asyncIterator](): AsyncGenerator<Uint8Array> {
    for (;;) {
      const next = this.queue.shift();
      if (next) {
        yield next;
        continue;
      }
      if (this.closed) return;
      await new Promise<void>((resolve) => {
        this.wake = resolve;
      });
    }
  }
}

export interface HostRoute {
  (payload: unknown, id: number): Promise<void>;
}

export class HostMock {
  private readonly decoder = new FrameDecoder();
  private nextId = 0; // §3: the host owns the EVEN ids.
  private readonly waiters = new Map<number, (frame: Frame) => void>();
  readonly received: Frame[] = [];
  /** Answers a sidecar-originated `req`, keyed by verb. */
  readonly routes = new Map<string, HostRoute>();
  helloSeen: Envelope | undefined;
  /** Set to false to test what happens when the host never accepts. */
  autoAccept = true;

  constructor(
    private readonly toSidecar: Pipe,
    fromSidecar: Pipe,
  ) {
    void this.read(fromSidecar);
  }

  private async read(source: Pipe): Promise<void> {
    for await (const bytes of source) {
      for (const frame of this.decoder.push(bytes)) {
        this.received.push(frame);
        const envelope = frame.envelope;
        // Waiters run for EVERY frame, including `hello`, so a test can assert
        // on the handshake as well as on a reply.
        this.waiters.get(idOf(envelope))?.(frame);
        if (envelope.t === "hello") {
          this.helloSeen = envelope;
          if (this.autoAccept) {
            await this.send({
              t: "accept",
              protocolVersion: 1,
              appVersion: "test",
              bridgeVersion: "0.1.0",
            });
          }
        } else if (envelope.t === "req") {
          const route = this.routes.get(envelope.verb);
          if (route) void route(envelope.payload, envelope.id);
        }
      }
    }
  }

  send(envelope: Envelope, bin?: Uint8Array): Promise<void> {
    return this.toSidecar.write(encodeFrame(envelope, bin));
  }

  /** Resolves on the next frame carrying `id` that satisfies `match`. */
  waitFor(id: number, match: (frame: Frame) => boolean): Promise<Frame> {
    const existing = this.received.find((f) => idOf(f.envelope) === id && match(f));
    if (existing) return Promise.resolve(existing);
    return new Promise<Frame>((resolve) => {
      const previous = this.waiters.get(id);
      this.waiters.set(id, (frame) => {
        previous?.(frame);
        if (match(frame)) resolve(frame);
      });
    });
  }

  /**
   * Issue a host → sidecar request. `stream` says whether to keep collecting
   * `chunk`s until `end` — the caller knows, because the verb decides: a unary
   * verb sends no `end` and waiting for one would just hang.
   */
  async call(
    verb: string,
    payload: unknown,
    options: { principal?: Principal; stream?: boolean } = {},
  ): Promise<{ res: Frame; chunks: Uint8Array[]; end: Frame | undefined }> {
    const id = this.nextId;
    this.nextId += 2;
    const chunks: Uint8Array[] = [];
    let end: Frame | undefined;
    const res = this.waitFor(id, (f) => f.envelope.t === "res");
    const done = new Promise<void>((resolve) => {
      const previous = this.waiters.get(id);
      this.waiters.set(id, (frame) => {
        previous?.(frame);
        if (frame.envelope.t === "chunk") chunks.push(frame.bin);
        if (frame.envelope.t === "end") {
          end = frame;
          resolve();
        }
      });
    });
    await this.send({ t: "req", id, principal: options.principal ?? "ui", verb, payload });
    const head = await res;
    if (head.envelope.t === "res" && !head.envelope.ok) return { res: head, chunks, end };
    if (options.stream === true) await done;
    return { res: head, chunks, end };
  }

  nextHostId(): number {
    const id = this.nextId;
    this.nextId += 2;
    return id;
  }
}

function idOf(envelope: Envelope): number {
  return "id" in envelope ? envelope.id : -1;
}

export function bytes(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

export function text(chunks: Uint8Array[]): string {
  const total = chunks.reduce((sum, c) => sum + c.byteLength, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return new TextDecoder().decode(out);
}
