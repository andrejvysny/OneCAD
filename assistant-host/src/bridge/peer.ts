/**
 * The duplex OCAK1 peer — `docs/assistant/wire-protocol.md` §2–§7, sidecar side.
 *
 * The shape of this class is dictated by §6. The reader loop decodes a frame
 * and dispatches it **synchronously**; every handler that needs to await runs
 * detached, and every outbound request is a promise resolved later by that same
 * loop. Nothing in the read path ever awaits a reply. A peer that awaited its
 * own pending response would deadlock the first time a `provider.fetch`
 * callback landed while an `agentkit.fetch` was in flight — which is to say,
 * on the first message the user ever sends.
 *
 * The second structural rule is §3's id allocation: this process owns the ODD
 * ids and the host owns the even ones, one monotonic counter each. Because the
 * two ranges cannot collide, "whose request is this?" never needs a direction
 * field, and an id arriving with the wrong parity is a desynchronised peer
 * rather than a request to answer.
 */
import { CONTRACT_VERSION } from "agentkit/contracts";
import type { Logger } from "agentkit/host";
import {
  FrameDecoder,
  FrameError,
  encodeFrame,
  MAX_BIN_LEN,
  type AcceptEnvelope,
  type BridgeErrorBody,
  type Envelope,
  type Frame,
  type Principal,
} from "./frame.js";

export const OCAK_PROTOCOL_VERSION = 1;

/** Default per-stream buffer bound (§7). One `end{stream_overflow}` beyond this. */
export const DEFAULT_STREAM_BUFFER_BYTES = 8 * 1024 * 1024;

/**
 * Default per-stream ITEM bound (§9).
 *
 * The second axis, and not a redundant one: a producer sending one-byte chunks
 * charges 1/8388608th of the byte budget per frame while costing a full array
 * slot, a wakeup and a map lookup each. A byte-only bound calls that stream
 * healthy right up to the point the process dies.
 */
export const DEFAULT_STREAM_BUFFER_ITEMS = 4096;

/**
 * Aggregate bytes this process will hold in its outbound write chain (§9).
 *
 * Per-stream bounds do not bound arbitrarily many streams, and the write chain
 * is where every one of them converges. A frame that would take the chain past
 * this waits for room rather than being queued; the producer is a pump that can
 * be slowed, and slowing it is the whole point of a budget.
 */
export const MAX_QUEUED_WRITE_BYTES = 16 * 1024 * 1024;

/** Concurrent outbound requests one connection will carry (§9). */
export const MAX_ACTIVE_OUTBOUND = 64;

/** Concurrent inbound requests one connection will serve (§9). */
export const MAX_ACTIVE_INBOUND = 64;

/** The `hello` payload, minus the tag and the two values the peer mints itself. */
export interface HelloIdentity {
  hostVersion: string;
  sessionNonce: string;
}

/**
 * An error a verb handler throws to choose the `res{ok:false}` error code the
 * originator sees. Anything else becomes `handler_error`, because inventing a
 * specific code for an unclassified throw would tell the host something this
 * process does not actually know.
 */
export class BridgeRequestError extends Error {
  override readonly name = "BridgeRequestError";
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

/** Raised locally when an inbound stream outruns its bound (§7). */
export class StreamOverflowError extends Error {
  override readonly name = "StreamOverflowError";
}

export interface InboundRequest {
  id: number;
  principal: Principal;
  payload: unknown;
  /** Aborted when the originator sends `cancel` for this id, or on teardown. */
  signal: AbortSignal;
}

/**
 * `onSent` runs after the LAST frame of this answer has left the process. It
 * exists for `shutdown`, whose contract is "answer, then stop": a handler that
 * tore the peer down itself would race its own `res` off the wire and leave the
 * supervisor unable to tell a clean exit from a crash.
 */
export type HandlerResult =
  | { kind: "unary"; payload?: unknown; onSent?: () => void }
  | {
      /** `payload` is the response head; the body follows as `chunk`s then `end`. */
      kind: "stream";
      payload?: unknown;
      body: ReadableStream<Uint8Array> | null;
      onSent?: () => void;
    };

/**
 * One row of §4's verb table. `principals` is explicit and there is no wildcard
 * entry: a verb that any principal may call is a verb that says so by naming
 * both of them.
 */
export interface VerbRoute {
  principals: readonly Principal[];
  handle(request: InboundRequest): Promise<HandlerResult>;
}

export interface StreamResponse {
  /** Whatever the responder put in its `res` payload — for `provider.fetch`, the head. */
  head: unknown;
  body: ReadableStream<Uint8Array>;
}

export interface BridgePeerOptions {
  input: AsyncIterable<Uint8Array>;
  write(bytes: Uint8Array): Promise<void>;
  identity: HelloIdentity;
  routes: Readonly<Record<string, VerbRoute>>;
  logger?: Logger;
  maxStreamBufferBytes?: number;
  maxStreamBufferItems?: number;
}

type Phase = "idle" | "handshake" | "open" | "closed";

interface PendingUnary {
  kind: "unary";
  resolve(payload: unknown): void;
  reject(err: Error): void;
}

interface PendingStream {
  kind: "stream";
  resolve(response: StreamResponse): void;
  reject(err: Error): void;
  sink?: ChunkSink;
  nextSeq: number;
}

/**
 * A peer that has broken the connection or per-request state machine of §2a.
 * Distinct from {@link FrameError} only in what it reports; both are fatal, and
 * both reach the same `close` in the read loop.
 */
export class StateViolationError extends Error {
  override readonly name = "StateViolationError";
}

type Pending = PendingUnary | PendingStream;

interface Inflight {
  controller: AbortController;
  /** Set once the handler answered with a stream, so `cancel` can stop the pump. */
  cancelPump?: () => void;
}

export class BridgePeer {
  private readonly decoder = new FrameDecoder();
  private readonly pending = new Map<number, Pending>();
  private readonly pendingPings = new Map<number, { resolve(): void; reject(err: Error): void }>();
  private readonly inflight = new Map<number, Inflight>();
  private readonly streamLimit: number;
  private readonly streamItemLimit: number;
  private readonly logger: Logger | undefined;
  /** §9: bytes handed to the write chain and not yet flushed. */
  private queuedWriteBytes = 0;
  private readonly writeAdmission: (() => void)[] = [];

  private phase: Phase = "idle";
  private nextId = 1; // §3: odd ids belong to the sidecar.
  private writeChain: Promise<void> = Promise.resolve();
  private closedError: Error | undefined;
  private handshake:
    | { resolve(accept: AcceptEnvelope): void; reject(err: Error): void }
    | undefined;
  private readonly finished: Promise<void>;
  private finish: (() => void) | undefined;

  constructor(private readonly options: BridgePeerOptions) {
    this.streamLimit = options.maxStreamBufferBytes ?? DEFAULT_STREAM_BUFFER_BYTES;
    this.streamItemLimit = options.maxStreamBufferItems ?? DEFAULT_STREAM_BUFFER_ITEMS;
    this.logger = options.logger;
    this.finished = new Promise<void>((resolve) => {
      this.finish = resolve;
    });
  }

  /**
   * Send `hello`, then resolve when the host answers `accept`.
   *
   * §2: `hello` is unsolicited, exactly once, and first — so the reader loop is
   * started BEFORE the frame goes out, or a host that replies inside one
   * scheduler tick would have its `accept` decoded by nobody.
   *
   * It is also this process's readiness signal. `main.ts` builds the whole
   * AgentKit object graph before calling `start`, which means a host that has
   * seen `hello` may send `agentkit.fetch` immediately and be served, rather
   * than being told to try again.
   */
  async start(): Promise<AcceptEnvelope> {
    if (this.phase !== "idle") throw new Error("BridgePeer.start called twice");
    this.phase = "handshake";
    const accepted = new Promise<AcceptEnvelope>((resolve, reject) => {
      this.handshake = { resolve, reject };
    });
    void this.readLoop();
    await this.send({
      t: "hello",
      protocolVersion: OCAK_PROTOCOL_VERSION,
      hostVersion: this.options.identity.hostVersion,
      agentkitContractVersion: CONTRACT_VERSION,
      pid: process.pid,
      sessionNonce: this.options.identity.sessionNonce,
    });
    return accepted;
  }

  /** Resolves when the connection has ended, for whatever reason. */
  waitForClose(): Promise<void> {
    return this.finished;
  }

  get isOpen(): boolean {
    return this.phase === "open";
  }

  /** §5, sidecar → host. Every request this process originates is `host`. */
  async request(verb: string, payload: unknown, signal?: AbortSignal): Promise<unknown> {
    this.admitRequest();
    const id = this.allocateId();
    const answered = new Promise<unknown>((resolve, reject) => {
      this.pending.set(id, { kind: "unary", resolve, reject });
    });
    this.watchAbort(id, signal);
    await this.send({ t: "req", id, principal: "host", verb, payload });
    return answered;
  }

  /**
   * A request answered by a head plus a body stream. The head arrives as the
   * `res` payload, so a caller can inspect a provider's status code before a
   * single body byte exists.
   */
  async requestStream(
    verb: string,
    payload: unknown,
    signal?: AbortSignal,
  ): Promise<StreamResponse> {
    this.admitRequest();
    const id = this.allocateId();
    const answered = new Promise<StreamResponse>((resolve, reject) => {
      this.pending.set(id, { kind: "stream", resolve, reject, nextSeq: 0 });
    });
    this.watchAbort(id, signal);
    await this.send({ t: "req", id, principal: "host", verb, payload });
    return answered;
  }

  async ping(): Promise<void> {
    const id = this.allocateId();
    const ponged = new Promise<void>((resolve, reject) => {
      this.pendingPings.set(id, { resolve, reject });
    });
    await this.send({ t: "ping", id });
    return ponged;
  }

  /**
   * Tear the connection down. Every waiter is settled with `err` rather than
   * left hanging: a promise that never resolves turns a crashed bridge into a
   * process that looks busy forever.
   */
  close(err?: Error): void {
    if (this.phase === "closed") return;
    this.phase = "closed";
    const failure = err ?? new Error("assistant bridge closed");
    this.closedError = failure;

    this.handshake?.reject(failure);
    this.handshake = undefined;
    for (const [, waiter] of this.pendingPings) waiter.reject(failure);
    this.pendingPings.clear();
    for (const [, entry] of this.pending) {
      if (entry.kind === "stream" && entry.sink) entry.sink.fail(failure);
      else entry.reject(failure);
    }
    this.pending.clear();
    for (const [, entry] of this.inflight) {
      entry.cancelPump?.();
      entry.controller.abort(failure);
    }
    this.inflight.clear();
    const waiters = this.writeAdmission.splice(0, this.writeAdmission.length);
    for (const wake of waiters) wake();
    this.finish?.();
  }

  // -- reading ------------------------------------------------------------

  private async readLoop(): Promise<void> {
    try {
      for await (const chunk of this.options.input) {
        if (this.phase === "closed") return;
        let frames: Frame[];
        try {
          frames = this.decoder.push(toBytes(chunk));
        } catch (err) {
          // §1: malformed is fatal. No resync, no skip — the supervisor restarts us.
          this.close(err as Error);
          return;
        }
        for (const frame of frames) {
          try {
            this.dispatch(frame);
          } catch (err) {
            this.close(err as Error);
            return;
          }
        }
      }
      this.close(new Error("assistant bridge stdin closed"));
    } catch (err) {
      this.close(err as Error);
    }
  }

  /**
   * Synchronous by contract. Anything that can block is started with `void` and
   * settles through the maps above — see this class's header for why (§6).
   */
  private dispatch(frame: Frame): void {
    const envelope = frame.envelope;
    if (this.phase === "handshake") {
      if (envelope.t === "accept") {
        if (envelope.protocolVersion !== OCAK_PROTOCOL_VERSION) {
          throw new FrameError(
            `host accepted protocol version ${envelope.protocolVersion}, this host speaks ${OCAK_PROTOCOL_VERSION}`,
          );
        }
        this.phase = "open";
        this.handshake?.resolve(envelope);
        this.handshake = undefined;
        return;
      }
      if (envelope.t === "reject") {
        throw new FrameError(`host rejected the bridge: ${envelope.reason}`);
      }
      // §2: the versions are settled before any other frame is exchanged, so an
      // incompatible pair can never perform a partial operation.
      throw new FrameError(`expected accept or reject, got ${envelope.t}`);
    }

    switch (envelope.t) {
      case "req":
        this.onRequest(envelope.id, envelope.principal, envelope.verb, envelope.payload);
        return;
      case "res":
        this.onResponse(envelope);
        return;
      case "chunk":
        this.onChunk(envelope.id, envelope.seq, frame.bin);
        return;
      case "end":
        this.onEnd(envelope);
        return;
      case "cancel":
        this.onCancel(envelope.id);
        return;
      case "ping":
        void this.send({ t: "pong", id: envelope.id });
        return;
      case "pong": {
        const waiter = this.pendingPings.get(envelope.id);
        this.pendingPings.delete(envelope.id);
        waiter?.resolve();
        return;
      }
      case "hello":
      case "accept":
      case "reject":
        throw new FrameError(`unexpected ${envelope.t} after the handshake`);
    }
  }

  private onRequest(id: number, principal: Principal, verb: string, payload: unknown): void {
    // §3: the host allocates even ids. An odd one means the two counters have
    // diverged, and a diverged counter crosses responses silently.
    if (id % 2 !== 0) {
      throw new StateViolationError(
        `inbound request id ${id} is sidecar-allocated; the host's allocator has diverged`,
      );
    }
    // §2a: refused WITHOUT touching the in-flight registration. Ownership is
    // bound to the entry that is already there, so nothing a late handler does
    // can retire a replacement's slot.
    if (this.inflight.has(id)) {
      void this.fail(id, {
        code: "duplicate_id",
        message: `an inbound request with id ${id} is still in flight`,
      });
      return;
    }
    if (this.inflight.size >= MAX_ACTIVE_INBOUND) {
      void this.fail(id, {
        code: "too_many_requests",
        message: `${MAX_ACTIVE_INBOUND} inbound requests are already in flight`,
      });
      return;
    }

    const route = this.options.routes[verb];
    if (!route) {
      void this.fail(id, { code: "unknown_verb", message: `no verb '${verb}'` });
      return;
    }
    if (!route.principals.includes(principal)) {
      void this.fail(id, {
        code: "forbidden",
        message: `principal '${principal}' may not call '${verb}'`,
      });
      return;
    }

    const controller = new AbortController();
    const entry: Inflight = { controller };
    this.inflight.set(id, entry);
    void this.serve(id, entry, route, { id, principal, payload, signal: controller.signal });
  }

  private async serve(
    id: number,
    entry: Inflight,
    route: VerbRoute,
    request: InboundRequest,
  ): Promise<void> {
    let result: HandlerResult;
    try {
      result = await route.handle(request);
    } catch (err) {
      this.retire(id, entry);
      if (request.signal.aborted) return; // §2: a cancelled id's reply is discarded anyway.
      const failure =
        err instanceof BridgeRequestError
          ? { code: err.code, message: err.message }
          : { code: "handler_error", message: (err as Error).message };
      await this.fail(id, failure);
      return;
    }

    if (request.signal.aborted) {
      this.retire(id, entry);
      if (result.kind === "stream") void result.body?.cancel();
      return;
    }

    await this.send({ t: "res", id, ok: true, payload: result.payload });
    if (result.kind === "unary") {
      this.retire(id, entry);
      result.onSent?.();
      return;
    }
    if (!result.body) {
      this.retire(id, entry);
      await this.send({ t: "end", id, ok: true });
      result.onSent?.();
      return;
    }
    await this.pumpBody(id, entry, result.body);
    result.onSent?.();
  }

  /**
   * Writes a response body as `chunk` frames, bounded by §7.
   *
   * The source is read WITHOUT awaiting each frame's flush, and the bytes in
   * flight are counted instead. That is what makes the bound reachable: a host
   * that has stopped draining stdout gets one loud
   * `end{ok:false, stream_overflow}` and the client re-subscribes from
   * `Last-Event-ID`. Parking the reader on the write instead would look fine
   * and quietly pin the run's whole event backlog in this process's heap.
   */
  private async pumpBody(
    id: number,
    entry: Inflight,
    body: ReadableStream<Uint8Array>,
  ): Promise<void> {
    const reader = body.getReader();
    let seq = 0;
    let outstanding = 0;
    let cancelled = false;
    entry.cancelPump = () => {
      cancelled = true;
      void reader.cancel().catch(() => {});
    };

    try {
      for (;;) {
        const next = await reader.read();
        if (cancelled || entry.controller.signal.aborted) return;
        if (next.done) break;
        for (const slice of split(next.value, MAX_BIN_LEN)) {
          if (outstanding + slice.byteLength > this.streamLimit) {
            void reader.cancel().catch(() => {});
            await this.send({
              t: "end",
              id,
              ok: false,
              error: {
                code: "stream_overflow",
                message: `stream ${id} exceeded its ${this.streamLimit}-byte buffer`,
              },
            });
            return;
          }
          outstanding += slice.byteLength;
          // Awaited HERE, in the producer, so the frames stay in `seq` order and
          // the aggregate budget is charged before the bytes are handed over.
          await this.admitWrite(slice.byteLength);
          const settle = (): void => {
            outstanding -= slice.byteLength;
            this.releaseWrite(slice.byteLength);
          };
          void this.send({ t: "chunk", id, seq }, slice).then(settle, settle);
          seq += 1;
        }
      }
      await this.send({ t: "end", id, ok: true });
    } catch (err) {
      await this.send({
        t: "end",
        id,
        ok: false,
        error: { code: "stream_failed", message: (err as Error).message },
      });
    } finally {
      this.retire(id, entry);
    }
  }

  /**
   * Retires an inbound request, but only if `entry` still owns the slot.
   *
   * The identity check is the fix for the half of F09 that bit the Rust side:
   * an unconditional `delete(id)` lets a handler that is finishing late remove
   * whatever now holds its id, after which a `cancel` for that id reaches
   * nothing. Cleanup is bound to the registration, never to the bare integer.
   */
  private retire(id: number, entry: Inflight): void {
    if (this.inflight.get(id) === entry) this.inflight.delete(id);
  }

  private onResponse(envelope: Extract<Envelope, { t: "res" }>): void {
    const entry = this.pending.get(envelope.id);
    if (!entry) {
      this.discardOrThrow("res", envelope.id);
      return;
    }
    // §2a: a request is `Settled` exactly once. A second head — successful or
    // not — is FATAL and never a replacement. Overwriting the sink resolved an
    // already-resolved promise and left the ORIGINAL reader waiting forever
    // while the chunks flowed to a sink nobody was reading (F09/R06).
    if (entry.kind === "stream" && entry.sink) {
      throw new StateViolationError(
        `a second res arrived for request ${envelope.id}, which already has a head`,
      );
    }
    if (!envelope.ok) {
      this.pending.delete(envelope.id);
      entry.reject(new BridgeRequestError(envelope.error.code, envelope.error.message));
      return;
    }
    if (entry.kind === "unary") {
      this.pending.delete(envelope.id);
      entry.resolve(envelope.payload);
      return;
    }
    // A streaming reply keeps its pending entry until `end`: `res` is only the head.
    const sink = new ChunkSink(this.streamLimit, this.streamItemLimit, () => {
      this.pending.delete(envelope.id);
      void this.send({ t: "cancel", id: envelope.id });
    });
    entry.sink = sink;
    entry.resolve({ head: envelope.payload, body: sink.readable });
  }

  /**
   * §2a's correlation-miss ladder, shared by `res`, `chunk` and `end`.
   *
   * Three outcomes, not two. An id in THIS peer's space that it has already
   * allocated is a late answer to something it cancelled, and §2 says to discard
   * it. An id of the wrong parity, or one past this peer's allocation cursor, is
   * the other side inventing correlation — and a peer whose allocator has
   * diverged crosses responses, which is precisely what a discarded frame would
   * hide until it did.
   */
  private discardOrThrow(tag: string, id: number): void {
    if (id % 2 === 1 && id < this.nextId) return;
    throw new StateViolationError(
      `${tag} names request id ${id}, which this host has never issued`,
    );
  }

  private onChunk(id: number, seq: number, bin: Uint8Array): void {
    const entry = this.pending.get(id);
    if (!entry) {
      this.discardOrThrow("chunk", id);
      return;
    }
    if (entry.kind !== "stream" || !entry.sink) {
      throw new StateViolationError(
        `chunk for request ${id}, which has not been answered with a head`,
      );
    }
    // §2: "seq starts at 0 and increases by one per chunk. A gap is fatal."
    if (seq !== entry.nextSeq) {
      throw new StateViolationError(
        `chunk ${seq} out of order on request ${id}, expected ${entry.nextSeq}`,
      );
    }
    entry.nextSeq = seq + 1;
    if (!entry.sink.push(bin)) {
      // The consumer is not keeping up. Say so loudly and stop the producer;
      // never drop the chunk, because the reader would then be silently wrong.
      this.pending.delete(id);
      entry.sink.fail(
        new StreamOverflowError(
          `inbound stream ${id} exceeded its buffer (${this.streamLimit} bytes / ${this.streamItemLimit} chunks)`,
        ),
      );
      void this.send({ t: "cancel", id });
    }
  }

  private onEnd(envelope: Extract<Envelope, { t: "end" }>): void {
    const entry = this.pending.get(envelope.id);
    if (!entry) {
      this.discardOrThrow("end", envelope.id);
      return;
    }
    this.pending.delete(envelope.id);
    if (entry.kind !== "stream" || !entry.sink) {
      throw new StateViolationError(
        `end for request ${envelope.id}, which has not been answered with a head`,
      );
    }
    if (envelope.ok) entry.sink.finish();
    else entry.sink.fail(new BridgeRequestError(envelope.error.code, envelope.error.message));
  }

  private onCancel(id: number): void {
    // §2: cancelling an unknown id is a no-op, not an error — the race is normal.
    const entry = this.inflight.get(id);
    if (!entry) return;
    this.inflight.delete(id);
    entry.cancelPump?.();
    entry.controller.abort(new Error(`request ${id} cancelled by peer`));
  }

  // -- writing ------------------------------------------------------------

  private allocateId(): number {
    if (this.phase === "closed") throw this.closedError ?? new Error("assistant bridge closed");
    const id = this.nextId;
    this.nextId += 2;
    return id;
  }

  /** §9: the correlation table is not allowed to grow without bound. */
  private admitRequest(): void {
    if (this.pending.size >= MAX_ACTIVE_OUTBOUND) {
      throw new BridgeRequestError(
        "too_many_requests",
        `${MAX_ACTIVE_OUTBOUND} requests are already in flight on this connection`,
      );
    }
  }

  /**
   * §9: waits until `size` bytes fit in the write chain's aggregate budget.
   *
   * Only body frames go through here. A control frame is small, already bounded
   * by the in-flight request caps, and is the thing that settles a request or
   * stops a runaway producer — queueing it behind a stream's megabytes is how a
   * `cancel` arrives after the flood it was meant to stop.
   */
  private async admitWrite(size: number): Promise<void> {
    while (
      this.queuedWriteBytes > 0 &&
      this.queuedWriteBytes + size > MAX_QUEUED_WRITE_BYTES &&
      this.phase !== "closed"
    ) {
      await new Promise<void>((resolve) => {
        this.writeAdmission.push(resolve);
      });
    }
    this.queuedWriteBytes += size;
  }

  private releaseWrite(size: number): void {
    this.queuedWriteBytes -= size;
    const waiters = this.writeAdmission.splice(0, this.writeAdmission.length);
    for (const wake of waiters) wake();
  }

  private watchAbort(id: number, signal: AbortSignal | undefined): void {
    if (!signal) return;
    const onAbort = (): void => {
      const entry = this.pending.get(id);
      if (!entry) return;
      this.pending.delete(id);
      const reason = new Error(`request ${id} aborted`);
      if (entry.kind === "stream" && entry.sink) entry.sink.fail(reason);
      else entry.reject(reason);
      void this.send({ t: "cancel", id });
    };
    if (signal.aborted) queueMicrotask(onAbort);
    else signal.addEventListener("abort", onAbort, { once: true });
  }

  /**
   * Answers `res{ok:false}` for an id this peer is NOT serving — an unknown
   * verb, a forbidden principal, a duplicate id, a full table.
   *
   * It deliberately does not touch `inflight`: on the duplicate-id path the
   * entry under that key belongs to a request that is still running, and
   * deleting it here is exactly the ownership transfer §2a forbids. A handler
   * retires its own entry, in `serve`.
   */
  private fail(id: number, error: BridgeErrorBody): Promise<void> {
    return this.send({ t: "res", id, ok: false, error });
  }

  /**
   * Serialised through one chain, because two interleaved writes on a
   * length-prefixed stream produce a frame that decodes as garbage. The chain
   * is repaired after a failure so one dead write does not poison every
   * subsequent one — the connection is being torn down anyway.
   */
  private send(envelope: Envelope, bin?: Uint8Array): Promise<void> {
    if (this.phase === "closed") return Promise.resolve();
    let bytes: Uint8Array;
    try {
      bytes = encodeFrame(envelope, bin);
    } catch (err) {
      this.logger?.error("assistant bridge failed to encode a frame", {
        tag: envelope.t,
        error: (err as Error).message,
      });
      return Promise.reject(err as Error);
    }
    const write = this.writeChain.then(() => this.options.write(bytes));
    this.writeChain = write.catch((err: unknown) => {
      this.close(err as Error);
    });
    return write;
  }
}

/**
 * A bounded byte queue behind a `ReadableStream`.
 *
 * `ReadableStream`'s own queuing strategy is not usable for §7: it makes `pull`
 * stop being called, which does nothing about a producer that pushes anyway,
 * and it has no way to express "this is now a failure". This does — `push`
 * returns `false` and the caller ends the stream loudly.
 */
export class ChunkSink {
  private readonly chunks: Uint8Array[] = [];
  private queued = 0;
  private ended = false;
  private failure: Error | undefined;
  private wake: (() => void) | undefined;
  readonly readable: ReadableStream<Uint8Array>;

  constructor(
    private readonly limit: number,
    private readonly itemLimit: number = DEFAULT_STREAM_BUFFER_ITEMS,
    onCancel?: () => void,
  ) {
    this.readable = new ReadableStream<Uint8Array>({
      pull: async (controller) => {
        for (;;) {
          const next = this.chunks.shift();
          if (next) {
            this.queued -= next.byteLength;
            controller.enqueue(next);
            return;
          }
          if (this.failure) {
            controller.error(this.failure);
            return;
          }
          if (this.ended) {
            controller.close();
            return;
          }
          await new Promise<void>((resolve) => {
            this.wake = resolve;
          });
        }
      },
      cancel: () => {
        this.ended = true;
        this.chunks.length = 0;
        this.queued = 0;
        onCancel?.();
      },
    });
  }

  /**
   * Queues one chunk, or returns `false` so the caller can end the stream loudly.
   *
   * Bounded on BOTH axes (§9). Byte-only accounting accepted an unbounded number
   * of tiny chunks — each one a live array slot and a wakeup — while reporting
   * the stream as well inside its budget (F10/R07). An empty chunk never reaches
   * here: §2a makes a tailless `chunk` frame fatal at the decoder.
   */
  push(bytes: Uint8Array): boolean {
    if (this.ended || this.failure) return true;
    if (this.queued + bytes.byteLength > this.limit) return false;
    if (this.chunks.length + 1 > this.itemLimit) return false;
    this.chunks.push(bytes);
    this.queued += bytes.byteLength;
    this.signal();
    return true;
  }

  /** §9 instrumentation: what this sink is currently holding. */
  get held(): { bytes: number; items: number } {
    return { bytes: this.queued, items: this.chunks.length };
  }

  finish(): void {
    this.ended = true;
    this.signal();
  }

  fail(err: Error): void {
    if (this.failure) return;
    this.failure = err;
    this.signal();
  }

  private signal(): void {
    const wake = this.wake;
    this.wake = undefined;
    wake?.();
  }
}

/** A body chunk larger than one frame's bin cap is split, never truncated. */
function* split(bytes: Uint8Array, max: number): Generator<Uint8Array> {
  if (bytes.byteLength <= max) {
    if (bytes.byteLength > 0) yield bytes;
    return;
  }
  for (let offset = 0; offset < bytes.byteLength; offset += max) {
    yield bytes.subarray(offset, Math.min(offset + max, bytes.byteLength));
  }
}

function toBytes(chunk: Uint8Array | string): Uint8Array {
  return typeof chunk === "string" ? new TextEncoder().encode(chunk) : chunk;
}
