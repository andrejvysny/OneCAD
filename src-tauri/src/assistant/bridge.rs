//! The duplex OCAK1 peer — `docs/assistant/wire-protocol.md` §2–§7, host side.
//!
//! Shape is dictated by §6: **both sides must keep draining inbound frames while
//! an outbound request awaits its reply.** One reader task, one writer task, a
//! correlation table of request ids, and an inbound-request router that answers
//! synchronously. A reader that awaited its own pending response would deadlock
//! the bridge the first time a `provider.fetch` callback landed mid-request —
//! which is to say on the first message the user ever sends. This mirrors
//! [`ProtocolClient`](onecad_protocol::client::ProtocolClient)'s structure over a
//! different protocol; it shares no code with it.
//!
//! Three rules this file exists to enforce:
//!
//! 1. **The `principal` on an inbound frame is never trusted** (§4). Every
//!    sidecar-originated `req` is stamped [`Principal::Host`] before anything
//!    looks at it, and only then checked against [`SIDECAR_TO_HOST_VERBS`]. A
//!    sidecar claiming `"ui"` therefore cannot reach a `ui`-only route: its claim
//!    is overwritten, not validated. The protocol crate deliberately does not
//!    stamp — a principal is asserted by the transport a request arrived on, and
//!    only this layer knows which transport that was.
//! 2. **A malformed frame is fatal** (§1). Bad magic, an over-cap length, an
//!    unknown envelope tag, a chunk `seq` gap, a `req` id with the wrong parity:
//!    every one tears the connection down with no resync, and the supervisor
//!    restarts the child.
//! 3. **A stream never drops a chunk silently** (§7). Each inbound stream holds a
//!    bounded byte buffer; on overflow the consumer receives a terminal
//!    `stream_overflow` and the producer is sent `cancel`. AgentKit's client
//!    resumes from `Last-Event-ID`, so an overflow costs a re-subscribe —
//!    whereas a dropped chunk would leave the client and the durable log
//!    disagreeing with nothing able to detect it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Notify};
use tokio::task::JoinHandle;
use tokio_util::codec::{Decoder, Encoder};
use tokio_util::sync::CancellationToken;

use onecad_assistant_protocol::{
    Accept, Cancel, Chunk, End, Envelope, ErrorObject, Hello, IdAllocator, OcakCodec, Ping, Pong,
    Principal, ProtocolError, RawFrame, Reject, Req, Res, HOST_TO_SIDECAR_VERBS, MAX_BIN_LEN,
    PROTOCOL_VERSION, SIDECAR_TO_HOST_VERBS,
};

use super::provider_gateway::{ProviderGateway, ProviderRegistry};

/// The one verb the sidecar may call on this host (§5).
const VERB_PROVIDER_FETCH: &str = "provider.fetch";

/// This host-side bridge implementation's own version, reported in `accept`.
/// Tracks `BRIDGE_VERSION` in `assistant-host/src/bridge/peer.ts`.
pub const BRIDGE_VERSION: &str = "0.1.0";

/// Default per-stream inbound buffer bound (§7), matching the sidecar's
/// `DEFAULT_STREAM_BUFFER_BYTES`. One `stream_overflow` beyond this.
pub const DEFAULT_STREAM_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Depth of the outbound frame queue. Bounded so a wedged child cannot make this
/// process buffer without limit; the reader never blocks on it (see
/// [`Inner::try_send`]).
const FRAME_QUEUE_DEPTH: usize = 64;

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

/// Everything that can go wrong on the assistant bridge.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// A framing or envelope violation. Fatal: the connection is torn down.
    #[error("assistant bridge protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    /// The handshake did not complete (no `hello` first, or a version this host
    /// cannot serve). The child is refused before any other frame is exchanged.
    #[error("assistant bridge handshake failed: {0}")]
    Handshake(String),
    /// The connection went away with the request in flight.
    #[error("assistant bridge closed: {0}")]
    Closed(String),
    /// The sidecar answered `res { ok: false }`. Carries its code verbatim so a
    /// caller can branch on `stream_overflow` rather than on a message.
    #[error("assistant host refused the request ({code}): {message}")]
    Refused {
        /// Machine-readable code from the sidecar.
        code: String,
        /// Human-readable detail.
        message: String,
    },
    /// No assistant host is running (never started, stopped, or failed).
    #[error("assistant host is not running: {0}")]
    NotRunning(String),
}

/// Transport IO folds into the protocol arm: an assistant bridge that cannot read
/// or write its pipe has failed at exactly the layer [`ProtocolError::Io`] names,
/// and a second IO variant here would make callers match on two things that mean
/// one thing.
impl From<std::io::Error> for BridgeError {
    fn from(e: std::io::Error) -> Self {
        BridgeError::Protocol(ProtocolError::Io(e))
    }
}

impl From<BridgeError> for crate::error::ApiError {
    /// Every assistant failure is `internal` at the webview boundary.
    ///
    /// The `ApiError` kind taxonomy is pinned by `src/ipc/types.ts` and its
    /// `worker` kind means the *geometry* worker — reusing it would make an
    /// assistant crash read as a kernel crash in every error surface the app
    /// already has. A dedicated kind is a frontend-visible contract change and
    /// belongs to the work package that gives the frontend somewhere to show it.
    fn from(e: BridgeError) -> Self {
        crate::error::ApiError::Internal(e.to_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Streams
// ─────────────────────────────────────────────────────────────────────────────

/// One event on a streaming response's body (§2 `chunk` / `end`).
#[derive(Debug)]
pub enum StreamEvent {
    /// Body bytes, in `seq` order.
    Chunk(Vec<u8>),
    /// The stream terminated. `Some(error)` for a failed `end` — including the
    /// locally-detected `stream_overflow`, which is delivered in exactly the
    /// shape a remote one would have.
    End(Option<ErrorObject>),
}

/// The head of a streaming response plus its body.
///
/// Dropping this before the terminal [`StreamEvent::End`] sends `cancel` for the
/// request (§2, best effort) and forgets the correlation slot: an abandoned
/// stream must not leave the sidecar pumping a body nobody will read.
pub struct StreamResponse {
    /// The OCAK1 request id this stream answers.
    pub id: u64,
    /// The `res` payload — for `agentkit.fetch`, the HTTP response head.
    pub head: Value,
    rx: mpsc::UnboundedReceiver<StreamEvent>,
    buffered: Arc<AtomicUsize>,
    inner: Arc<Inner>,
    ended: bool,
}

impl StreamResponse {
    /// The next body event, or `None` once the stream is finished.
    ///
    /// Receiving a chunk releases its bytes from the §7 buffer budget, so a
    /// consumer that keeps up never overflows and one that stalls does.
    pub async fn next(&mut self) -> Option<StreamEvent> {
        if self.ended {
            return None;
        }
        match self.rx.recv().await {
            Some(StreamEvent::Chunk(bytes)) => {
                self.buffered.fetch_sub(bytes.len(), Ordering::SeqCst);
                Some(StreamEvent::Chunk(bytes))
            }
            Some(StreamEvent::End(error)) => {
                self.ended = true;
                Some(StreamEvent::End(error))
            }
            None => {
                // The sender is gone without an `end`: the reader task tore down
                // between frames. Report it as a failed stream rather than a
                // clean finish, or a truncated body would look complete.
                self.ended = true;
                Some(StreamEvent::End(Some(ErrorObject {
                    code: "bridge_closed".into(),
                    message: "assistant bridge closed mid-stream".into(),
                })))
            }
        }
    }
}

impl Drop for StreamResponse {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        self.inner.forget(self.id);
        self.inner
            .try_send(&Envelope::Cancel(Cancel { id: self.id }));
    }
}

impl std::fmt::Debug for StreamResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamResponse")
            .field("id", &self.id)
            .field("ended", &self.ended)
            .finish_non_exhaustive()
    }
}

/// What the reader hands back when a streaming request's `res` head arrives.
struct StreamHeadDelivery {
    head: Value,
    rx: mpsc::UnboundedReceiver<StreamEvent>,
    buffered: Arc<AtomicUsize>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Correlation table
// ─────────────────────────────────────────────────────────────────────────────

enum Pending {
    /// Awaiting the single terminal `res`.
    Unary(oneshot::Sender<Result<Res, BridgeError>>),
    /// Awaiting the `res` that carries a streaming response's head.
    StreamHead(oneshot::Sender<Result<StreamHeadDelivery, BridgeError>>),
    /// Head delivered; `chunk`s and one `end` still to come.
    StreamBody {
        tx: mpsc::UnboundedSender<StreamEvent>,
        buffered: Arc<AtomicUsize>,
        next_seq: u64,
    },
}

/// The correlation table plus a **closed** latch.
///
/// Once the reader tears down it drains every waiter and latches `closed`, so a
/// request that races the teardown fails fast instead of registering a slot
/// nothing will ever fire — which would hang its caller forever. Registration and
/// closing take the same lock, so there is no lost-wakeup window. Pings live in
/// the same table for the same reason: one lock, one latch.
#[derive(Default)]
struct PendingTable {
    entries: HashMap<u64, Pending>,
    pings: HashMap<u64, oneshot::Sender<()>>,
    closed: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// The bridge
// ─────────────────────────────────────────────────────────────────────────────

/// Tunables for one connection.
#[derive(Debug, Clone)]
pub struct BridgeOptions {
    /// The OneCAD version reported in `accept`.
    pub app_version: String,
    /// Per-stream inbound buffer bound (§7).
    pub max_stream_buffer_bytes: usize,
    /// The holder the local-provider gateway that serves `provider.fetch` is read
    /// out of (ADR-0017).
    ///
    /// Read ONCE PER REQUEST, not captured at connect time: the user may edit
    /// their endpoint or model while the sidecar is running, and a gateway fixed
    /// at handshake would make that edit wait for a restart. An EMPTY holder
    /// means this connection has no provider to serve and answers the verb
    /// `unimplemented`.
    ///
    /// What is inside is Rust-owned and settings-populated; there is no frame on
    /// this bridge that can reach into it, because [`SIDECAR_TO_HOST_VERBS`] has
    /// exactly one entry and that entry names a provider, it does not define one.
    pub provider_registry: Arc<ProviderRegistry>,
}

impl Default for BridgeOptions {
    fn default() -> Self {
        BridgeOptions {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            max_stream_buffer_bytes: DEFAULT_STREAM_BUFFER_BYTES,
            provider_registry: Arc::new(ProviderRegistry::new()),
        }
    }
}

struct Inner {
    /// Host ids are EVEN (§3); the sidecar owns the odd ones.
    ids: IdAllocator,
    frames: mpsc::Sender<RawFrame>,
    pending: Mutex<PendingTable>,
    /// Cancellation tokens for the inbound requests this host is still serving.
    ///
    /// `provider.fetch` is the first inbound verb whose handler outlives its
    /// dispatch, which is what makes this table necessary: a `cancel` for a
    /// request already talking to a provider has to reach the task holding that
    /// response, or the upstream call keeps generating into a socket nobody will
    /// read.
    inbound: Mutex<HashMap<u64, CancellationToken>>,
    closed: AtomicBool,
    closed_signal: Notify,
    stream_cap: usize,
    provider: Arc<ProviderRegistry>,
}

impl Inner {
    /// Queues a frame without ever blocking the caller.
    ///
    /// Used by the reader task, which MUST keep draining (§6): awaiting a full
    /// outbound queue there would couple inbound progress to a child that has
    /// stopped reading its stdin, i.e. deadlock the bridge to avoid dropping a
    /// pong. The queue is 64 frames deep and a full one means the connection is
    /// already dead, so a dropped control frame changes nothing.
    fn try_send(&self, envelope: &Envelope) {
        let raw = match envelope.to_json_vec() {
            Ok(json) => RawFrame::json_only(json),
            Err(err) => {
                tracing::warn!(target: "assistant", error = %err, "bridge: failed to encode a control frame");
                return;
            }
        };
        if self.frames.try_send(raw).is_err() {
            tracing::warn!(target: "assistant", "bridge: outbound queue full or closed; control frame dropped");
        }
    }

    async fn send(&self, envelope: &Envelope) -> Result<(), BridgeError> {
        let raw = RawFrame::json_only(envelope.to_json_vec()?);
        self.frames
            .send(raw)
            .await
            .map_err(|_| BridgeError::Closed("writer task ended".into()))
    }

    /// Queues one outbound `chunk` with its binary tail.
    ///
    /// Awaits the bounded frame queue rather than `try_send`ing: this is a BODY
    /// frame, and dropping one would truncate a response silently. The wait is
    /// the backpressure that keeps a provider streaming faster than the sidecar
    /// reads from turning into unbounded memory here.
    async fn send_chunk(&self, id: u64, seq: u64, bin: Vec<u8>) -> Result<(), BridgeError> {
        let json = Envelope::Chunk(Chunk { id, seq }).to_json_vec()?;
        self.frames
            .send(RawFrame { json, bin })
            .await
            .map_err(|_| BridgeError::Closed("writer task ended".into()))
    }

    /// Registers an inbound request's cancellation token. Registered BEFORE the
    /// handler is spawned, so a `cancel` can never arrive in the window between
    /// the task starting and the table learning about it.
    fn track_inbound(&self, id: u64, token: CancellationToken) {
        let previous = self
            .inbound
            .lock()
            .expect("assistant inbound table poisoned")
            .insert(id, token);
        if let Some(previous) = previous {
            // §3 says ids are never reused, so this is a peer that has lost track
            // of its own counter. Cancel the handler it just orphaned.
            tracing::warn!(target: "assistant", id, "bridge: inbound request id reused; cancelling the older handler");
            previous.cancel();
        }
    }

    fn finish_inbound(&self, id: u64) {
        self.inbound
            .lock()
            .expect("assistant inbound table poisoned")
            .remove(&id);
    }

    fn cancel_inbound(&self, id: u64) {
        let token = self
            .inbound
            .lock()
            .expect("assistant inbound table poisoned")
            .remove(&id);
        if let Some(token) = token {
            token.cancel();
        }
    }

    /// Registers a correlation slot, refusing once the connection has closed.
    fn register(&self, id: u64, pending: Pending) -> Result<(), BridgeError> {
        let mut table = self
            .pending
            .lock()
            .expect("assistant pending table poisoned");
        if table.closed {
            return Err(BridgeError::Closed("connection closed".into()));
        }
        table.entries.insert(id, pending);
        Ok(())
    }

    fn forget(&self, id: u64) {
        self.pending
            .lock()
            .expect("assistant pending table poisoned")
            .entries
            .remove(&id);
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Drains every waiter and latches the connection closed. Idempotent.
    fn fail_all(&self, reason: &str) {
        {
            let mut table = self
                .pending
                .lock()
                .expect("assistant pending table poisoned");
            table.closed = true; // latch first: a racing request fails fast, never hangs.
            for (_, entry) in table.entries.drain() {
                match entry {
                    Pending::Unary(tx) => {
                        let _ = tx.send(Err(BridgeError::Closed(reason.to_string())));
                    }
                    Pending::StreamHead(tx) => {
                        let _ = tx.send(Err(BridgeError::Closed(reason.to_string())));
                    }
                    Pending::StreamBody { tx, .. } => {
                        let _ = tx.send(StreamEvent::End(Some(ErrorObject {
                            code: "bridge_closed".into(),
                            message: reason.to_string(),
                        })));
                    }
                }
            }
            table.pings.clear();
        }
        // A teardown must not leave a provider call running: the frames it would
        // produce have nowhere to go, and the upstream connection would stay open
        // until the model finished talking to nobody.
        for (_, token) in self
            .inbound
            .lock()
            .expect("assistant inbound table poisoned")
            .drain()
        {
            token.cancel();
        }
        self.closed.store(true, Ordering::SeqCst);
        self.closed_signal.notify_waiters();
    }
}

/// An open OCAK1 connection to the assistant host.
pub struct AssistantBridge {
    inner: Arc<Inner>,
    hello: Hello,
    reader_handle: JoinHandle<()>,
    writer_handle: JoinHandle<()>,
}

impl AssistantBridge {
    /// Completes the handshake over an already-connected transport and starts the
    /// reader and writer tasks.
    ///
    /// §2: the sidecar sends `hello` unsolicited as its FIRST frame. Anything
    /// else first is fatal, and a `protocolVersion` this host cannot serve is
    /// answered with `reject` and torn down **before any other frame is
    /// exchanged**, so an incompatible pair can never perform a partial
    /// operation.
    pub async fn connect<R, W>(
        mut reader: R,
        mut writer: W,
        options: BridgeOptions,
    ) -> Result<AssistantBridge, BridgeError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let mut codec = OcakCodec;
        let mut buf = BytesMut::new();
        let first = loop {
            if let Some(frame) = codec.decode(&mut buf)? {
                break frame;
            }
            if reader.read_buf(&mut buf).await? == 0 {
                return Err(BridgeError::Handshake("closed before hello".into()));
            }
        };

        let hello = match Envelope::from_json_slice(&first.json)? {
            Envelope::Hello(hello) => hello,
            other => {
                return Err(BridgeError::Handshake(format!(
                    "expected hello, got {}",
                    envelope_kind(&other)
                )))
            }
        };

        if hello.protocol_version != PROTOCOL_VERSION {
            let reason = format!(
                "unsupported protocol version {}; this host speaks {PROTOCOL_VERSION}",
                hello.protocol_version
            );
            // Written directly rather than through the writer task: the refusal
            // must reach the child before anything else exists, and there is no
            // connection to keep afterwards.
            write_one(
                &mut writer,
                &Envelope::Reject(Reject {
                    reason: reason.clone(),
                }),
            )
            .await?;
            return Err(BridgeError::Handshake(reason));
        }

        write_one(
            &mut writer,
            &Envelope::Accept(Accept {
                protocol_version: PROTOCOL_VERSION,
                app_version: options.app_version.clone(),
                bridge_version: BRIDGE_VERSION.to_string(),
            }),
        )
        .await?;

        let (frames_tx, frames_rx) = mpsc::channel::<RawFrame>(FRAME_QUEUE_DEPTH);
        let inner = Arc::new(Inner {
            ids: IdAllocator::host(),
            frames: frames_tx,
            pending: Mutex::new(PendingTable::default()),
            inbound: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            closed_signal: Notify::new(),
            stream_cap: options.max_stream_buffer_bytes,
            provider: options.provider_registry.clone(),
        });

        let writer_handle = tokio::spawn(writer_task(writer, frames_rx));
        let reader_handle = tokio::spawn(reader_task(reader, buf, inner.clone()));

        tracing::debug!(
            target: "assistant",
            pid = hello.pid,
            host_version = %hello.host_version,
            agentkit = %hello.agentkit_contract_version,
            "assistant bridge open"
        );

        Ok(AssistantBridge {
            inner,
            hello,
            reader_handle,
            writer_handle,
        })
    }

    /// The sidecar's `hello` (pid, versions, session nonce).
    #[must_use]
    pub fn hello(&self) -> &Hello {
        &self.hello
    }

    /// Whether the connection has torn down.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Resolves as soon as the connection tears down — immediately if it already
    /// has. The waiter is enabled BEFORE the flag is re-read, so the wakeup can
    /// never be lost (the naive check-then-`notified()` has exactly that race).
    pub async fn closed(&self) {
        loop {
            let notified = self.inner.closed_signal.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.inner.is_closed() {
                return;
            }
            notified.await;
        }
    }

    /// Sends a `req` and awaits its terminal `res` (§5 host → sidecar).
    ///
    /// `principal` is checked against [`HOST_TO_SIDECAR_VERBS`] before the frame
    /// goes out: this side refuses its own unauthorized call rather than relying
    /// on the peer to catch it.
    pub async fn request(
        &self,
        verb: &str,
        principal: Principal,
        payload: Value,
    ) -> Result<Value, BridgeError> {
        HOST_TO_SIDECAR_VERBS.check(verb, principal)?;
        let id = self.inner.ids.next();
        let (tx, rx) = oneshot::channel();
        self.inner.register(id, Pending::Unary(tx))?;
        self.send_req(id, principal, verb, payload).await?;
        let res = rx
            .await
            .map_err(|_| BridgeError::Closed("response channel dropped".into()))??;
        if res.ok {
            Ok(res.payload.unwrap_or(Value::Null))
        } else {
            Err(refused(res.error))
        }
    }

    /// Sends a `req` whose answer is a head plus a chunk stream (§2), so a caller
    /// can read a response's status before a single body byte exists.
    pub async fn request_stream(
        &self,
        verb: &str,
        principal: Principal,
        payload: Value,
    ) -> Result<StreamResponse, BridgeError> {
        HOST_TO_SIDECAR_VERBS.check(verb, principal)?;
        let id = self.inner.ids.next();
        let (tx, rx) = oneshot::channel();
        self.inner.register(id, Pending::StreamHead(tx))?;
        self.send_req(id, principal, verb, payload).await?;
        let delivery = rx
            .await
            .map_err(|_| BridgeError::Closed("response channel dropped".into()))??;
        Ok(StreamResponse {
            id,
            head: delivery.head,
            rx: delivery.rx,
            buffered: delivery.buffered,
            inner: self.inner.clone(),
            ended: false,
        })
    }

    /// Liveness probe (§2). Resolves when the matching `pong` returns.
    pub async fn ping(&self) -> Result<(), BridgeError> {
        let id = self.inner.ids.next();
        let (tx, rx) = oneshot::channel();
        {
            let mut table = self
                .inner
                .pending
                .lock()
                .expect("assistant pending table poisoned");
            if table.closed {
                return Err(BridgeError::Closed("connection closed".into()));
            }
            table.pings.insert(id, tx);
        }
        self.inner.send(&Envelope::Ping(Ping { id })).await?;
        rx.await
            .map_err(|_| BridgeError::Closed("ping never answered".into()))
    }

    /// Tears the connection down: every waiter fails, both tasks stop.
    pub fn close(&self, reason: &str) {
        self.inner.fail_all(reason);
        self.reader_handle.abort();
        self.writer_handle.abort();
    }

    async fn send_req(
        &self,
        id: u64,
        principal: Principal,
        verb: &str,
        payload: Value,
    ) -> Result<(), BridgeError> {
        let sent = self
            .inner
            .send(&Envelope::Req(Req {
                id,
                principal,
                verb: verb.to_string(),
                payload,
            }))
            .await;
        if sent.is_err() {
            self.inner.forget(id);
        }
        sent
    }
}

impl std::fmt::Debug for AssistantBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantBridge")
            .field("pid", &self.hello.pid)
            .field("host_version", &self.hello.host_version)
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

fn refused(error: Option<ErrorObject>) -> BridgeError {
    match error {
        Some(e) => BridgeError::Refused {
            code: e.code,
            message: e.message,
        },
        // A `res{ok:false}` with no error body: the peer is within its rights to
        // be terse, and the caller still needs a code to branch on.
        None => BridgeError::Refused {
            code: "unspecified".into(),
            message: "the assistant host reported a failure with no error body".into(),
        },
    }
}

fn envelope_kind(envelope: &Envelope) -> &'static str {
    match envelope {
        Envelope::Hello(_) => "hello",
        Envelope::Accept(_) => "accept",
        Envelope::Reject(_) => "reject",
        Envelope::Req(_) => "req",
        Envelope::Res(_) => "res",
        Envelope::Chunk(_) => "chunk",
        Envelope::End(_) => "end",
        Envelope::Cancel(_) => "cancel",
        Envelope::Ping(_) => "ping",
        Envelope::Pong(_) => "pong",
    }
}

async fn write_one<W: AsyncWrite + Unpin>(
    writer: &mut W,
    envelope: &Envelope,
) -> Result<(), BridgeError> {
    let mut codec = OcakCodec;
    let mut out = BytesMut::new();
    codec.encode(RawFrame::json_only(envelope.to_json_vec()?), &mut out)?;
    writer.write_all(&out).await?;
    writer.flush().await?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tasks
// ─────────────────────────────────────────────────────────────────────────────

async fn writer_task<W>(mut writer: W, mut frames: mpsc::Receiver<RawFrame>)
where
    W: AsyncWrite + Unpin,
{
    let mut codec = OcakCodec;
    let mut out = BytesMut::new();
    while let Some(frame) = frames.recv().await {
        out.clear();
        if let Err(err) = codec.encode(frame, &mut out) {
            tracing::warn!(target: "assistant", error = %err, "bridge writer: encode failed");
            break;
        }
        if writer.write_all(&out).await.is_err() || writer.flush().await.is_err() {
            break; // transport gone; the reader fails every pending request.
        }
    }
    let _ = writer.shutdown().await;
}

async fn reader_task<R>(mut reader: R, mut buf: BytesMut, inner: Arc<Inner>)
where
    R: AsyncRead + Unpin,
{
    let mut codec = OcakCodec;
    loop {
        // Drain every complete frame already buffered before reading again.
        loop {
            match codec.decode(&mut buf) {
                Ok(Some(frame)) => {
                    if let Err(err) = dispatch(&inner, frame) {
                        // §1/§2: fatal. No resync — the supervisor restarts the child.
                        tracing::warn!(target: "assistant", error = %err, "bridge reader: fatal frame");
                        inner.fail_all(&err.to_string());
                        return;
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!(target: "assistant", error = %err, "bridge reader: fatal framing error");
                    inner.fail_all(&err.to_string());
                    return;
                }
            }
        }
        match reader.read_buf(&mut buf).await {
            Ok(0) => {
                inner.fail_all("assistant host closed its stdout");
                return;
            }
            Ok(_) => {}
            Err(err) => {
                tracing::debug!(target: "assistant", error = %err, "bridge reader: io error");
                inner.fail_all("assistant bridge read error");
                return;
            }
        }
    }
}

/// Routes one decoded frame. `Err` is FATAL and tears the connection down.
fn dispatch(inner: &Arc<Inner>, frame: RawFrame) -> Result<(), BridgeError> {
    let envelope = Envelope::from_json_slice(&frame.json)?;
    match envelope {
        Envelope::Res(res) => on_res(inner, res),
        Envelope::Chunk(chunk) => on_chunk(inner, chunk, frame.bin),
        Envelope::End(end) => on_end(inner, end),
        Envelope::Req(req) => {
            on_req(inner, req);
            Ok(())
        }
        Envelope::Cancel(cancel) => {
            // §2: cancelling an unknown id is a no-op, not an error — the request
            // may have finished a frame ago. A known id reaches the task serving
            // it, which drops the provider response and with it the upstream
            // connection.
            inner.cancel_inbound(cancel.id);
            Ok(())
        }
        Envelope::Ping(ping) => {
            inner.try_send(&Envelope::Pong(Pong { id: ping.id }));
            Ok(())
        }
        Envelope::Pong(pong) => {
            let waiter = inner
                .pending
                .lock()
                .expect("assistant pending table poisoned")
                .pings
                .remove(&pong.id);
            if let Some(tx) = waiter {
                let _ = tx.send(());
            }
            Ok(())
        }
        // §2: the handshake is settled before any other frame is exchanged, so a
        // second one is a peer that has lost track of the connection's state.
        other @ (Envelope::Hello(_) | Envelope::Accept(_) | Envelope::Reject(_)) => {
            Err(BridgeError::Handshake(format!(
                "unexpected {} after the handshake",
                envelope_kind(&other)
            )))
        }
    }
}

fn on_res(inner: &Arc<Inner>, res: Res) -> Result<(), BridgeError> {
    let mut table = inner
        .pending
        .lock()
        .expect("assistant pending table poisoned");
    match table.entries.remove(&res.id) {
        Some(Pending::Unary(tx)) => {
            let _ = tx.send(Ok(res));
            Ok(())
        }
        Some(Pending::StreamHead(tx)) => {
            if !res.ok {
                let _ = tx.send(Err(refused(res.error)));
                return Ok(());
            }
            let (chunks_tx, chunks_rx) = mpsc::unbounded_channel();
            let buffered = Arc::new(AtomicUsize::new(0));
            table.entries.insert(
                res.id,
                Pending::StreamBody {
                    tx: chunks_tx,
                    buffered: buffered.clone(),
                    next_seq: 0,
                },
            );
            let delivery = StreamHeadDelivery {
                head: res.payload.unwrap_or(Value::Null),
                rx: chunks_rx,
                buffered,
            };
            if tx.send(Ok(delivery)).is_err() {
                // The caller went away between sending the request and reading
                // its head; drop the slot rather than buffering a body nobody
                // owns. The peer learns about it from the `cancel` below.
                table.entries.remove(&res.id);
                drop(table);
                inner.try_send(&Envelope::Cancel(Cancel { id: res.id }));
            }
            Ok(())
        }
        Some(entry @ Pending::StreamBody { .. }) => {
            table.entries.insert(res.id, entry);
            Err(BridgeError::Handshake(format!(
                "second res for streaming request {}",
                res.id
            )))
        }
        // §2: a `res` for a cancelled id may still arrive and MUST be discarded.
        None => Ok(()),
    }
}

fn on_chunk(inner: &Arc<Inner>, chunk: Chunk, bin: Vec<u8>) -> Result<(), BridgeError> {
    let mut table = inner
        .pending
        .lock()
        .expect("assistant pending table poisoned");
    let Some(entry) = table.entries.get_mut(&chunk.id) else {
        return Ok(()); // discarded: cancelled, or already ended.
    };
    let Pending::StreamBody {
        tx,
        buffered,
        next_seq,
    } = entry
    else {
        return Err(BridgeError::Handshake(format!(
            "chunk for request {}, which is not streaming",
            chunk.id
        )));
    };
    // §2: "seq starts at 0 and increases by one per chunk. A gap is fatal." A
    // receiver cannot tell a dropped chunk from a reordered one, and guessing
    // would let the stream and its durable log disagree undetectably.
    if chunk.seq != *next_seq {
        return Err(BridgeError::Handshake(format!(
            "chunk {} out of order on request {}, expected {}",
            chunk.seq, chunk.id, next_seq
        )));
    }
    *next_seq += 1;

    // §7: bounded, and loud on overflow. The buffer is charged here and released
    // in `StreamResponse::next`, so a consumer that keeps up never trips it.
    let pending_bytes = buffered.load(Ordering::SeqCst);
    if pending_bytes + bin.len() > inner.stream_cap {
        let cap = inner.stream_cap;
        let _ = tx.send(StreamEvent::End(Some(ErrorObject {
            code: "stream_overflow".into(),
            message: format!("stream {} exceeded its {cap}-byte buffer", chunk.id),
        })));
        table.entries.remove(&chunk.id);
        drop(table);
        // `cancel`, not `end`: on an INBOUND stream this host is the receiver,
        // and `end` belongs to the responder. Cancelling is what §2 gives the
        // originator to stop a producer it can no longer keep up with.
        inner.try_send(&Envelope::Cancel(Cancel { id: chunk.id }));
        tracing::warn!(target: "assistant", id = chunk.id, cap, "bridge: inbound stream overflowed its buffer");
        return Ok(());
    }
    buffered.fetch_add(bin.len(), Ordering::SeqCst);
    let _ = tx.send(StreamEvent::Chunk(bin));
    Ok(())
}

fn on_end(inner: &Arc<Inner>, end: End) -> Result<(), BridgeError> {
    let mut table = inner
        .pending
        .lock()
        .expect("assistant pending table poisoned");
    match table.entries.remove(&end.id) {
        Some(Pending::StreamBody { tx, .. }) => {
            let error = if end.ok {
                None
            } else {
                Some(end.error.unwrap_or(ErrorObject {
                    code: "stream_failed".into(),
                    message: "the assistant host ended the stream without a reason".into(),
                }))
            };
            let _ = tx.send(StreamEvent::End(error));
            Ok(())
        }
        Some(entry) => {
            table.entries.insert(end.id, entry);
            Err(BridgeError::Handshake(format!(
                "end for request {}, which is not streaming",
                end.id
            )))
        }
        None => Ok(()), // cancelled or already ended — the race is normal.
    }
}

/// The inbound-request router (§4/§5, sidecar → host).
///
/// **The stamp is the whole point.** `principal` as received is discarded and
/// replaced with [`Principal::Host`] before the verb table is consulted, so a
/// sidecar that claims `"ui"` gains nothing: a `ui`-only verb is not in the
/// sidecar→host table at all, and the one verb that is (`provider.fetch`) is
/// `host`-only. Data does not grant permission.
///
/// No verb registered here touches a document or the filesystem (ADR-0018). The
/// one that reaches the network, `provider.fetch`, reaches exactly the registered
/// loopback provider its payload names and nothing else (ADR-0017) — the payload
/// carries a provider id, and the gateway owns every part of the URL.
fn on_req(inner: &Arc<Inner>, req: Req) {
    // §3: the sidecar allocates ODD ids. An even one means the two counters have
    // diverged, and a diverged counter crosses responses silently. Refused rather
    // than answered, because there is no way to know whose request it is.
    if req.id.is_multiple_of(2) {
        tracing::warn!(
            target: "assistant",
            id = req.id,
            "bridge: inbound request id is not sidecar-allocated; refusing"
        );
        fail(
            inner,
            req.id,
            "bad_request",
            "inbound request id is not sidecar-allocated",
        );
        return;
    }

    let principal = Principal::Host; // STAMPED. Never read from the frame.
    if principal != req.principal {
        // Worth a line: a sidecar asserting another authority is either a bug in
        // the child or something wearing its skin. It changes nothing here.
        tracing::warn!(
            target: "assistant",
            id = req.id,
            verb = %req.verb,
            claimed = %req.principal,
            "bridge: inbound request claimed a principal it does not have; stamped host"
        );
    }

    match SIDECAR_TO_HOST_VERBS.check(&req.verb, principal) {
        Err(ProtocolError::UnknownVerb(verb)) => {
            fail(
                inner,
                req.id,
                "unknown_verb",
                &format!("no verb '{verb}' on this bridge"),
            );
        }
        Err(err) => {
            fail(inner, req.id, "forbidden", &err.to_string());
        }
        Ok(()) => route(inner, req),
    }
}

/// Dispatches an authorized inbound request to its host-side route.
fn route(inner: &Arc<Inner>, req: Req) {
    match req.verb.as_str() {
        VERB_PROVIDER_FETCH => {
            // Read here, per request: a provider installed since this bridge
            // handshook serves the very next `provider.fetch` (ADR-0017's
            // trusted settings path), and one cleared since then stops serving
            // just as promptly.
            let Some(gateway) = inner.provider.current() else {
                fail(
                    inner,
                    req.id,
                    "unimplemented",
                    "no local-provider gateway is configured on this bridge",
                );
                return;
            };
            let id = req.id;
            // Registered BEFORE the spawn, so a `cancel` that races the first
            // frame still finds something to cancel.
            let token = CancellationToken::new();
            inner.track_inbound(id, token.clone());
            let inner = inner.clone();
            // Spawned rather than awaited: §6 requires the reader to keep
            // draining, and a provider call can run for minutes. Serving it
            // inline would stop this host answering pings for the whole
            // completion.
            tokio::spawn(async move {
                tokio::select! {
                    // Dropping the serve future drops the `ProviderStream` inside
                    // it, which drops the upstream response and closes that
                    // connection. That is what makes a cancel an abort rather
                    // than an orphan.
                    () = token.cancelled() => {
                        tracing::debug!(target: "assistant", id, "bridge: provider.fetch cancelled");
                    }
                    () = serve_provider_fetch(&inner, &gateway, id, req.payload) => {}
                }
                inner.finish_inbound(id);
            });
        }
        other => fail(
            inner,
            req.id,
            "unimplemented",
            &format!("verb '{other}' has no host-side route in this build"),
        ),
    }
}

/// Serves one `provider.fetch`: the head as `res`, the body as `chunk`s, one
/// `end` (§2).
///
/// The head goes out before a single body byte exists, and every chunk is
/// forwarded as it arrives, so a completion is never held in memory here. A write
/// failure means the connection is gone and there is nothing left to report to.
async fn serve_provider_fetch(
    inner: &Arc<Inner>,
    gateway: &Arc<ProviderGateway>,
    id: u64,
    payload: Value,
) {
    let mut stream = match gateway.fetch(payload).await {
        Ok(stream) => stream,
        Err(err) => {
            tracing::debug!(target: "assistant", id, error = %err, "bridge: provider.fetch refused");
            let sent = inner
                .send(&Envelope::Res(Res {
                    id,
                    ok: false,
                    payload: None,
                    error: Some(ErrorObject {
                        code: err.code().to_string(),
                        message: err.to_string(),
                    }),
                }))
                .await;
            if sent.is_err() {
                tracing::debug!(target: "assistant", id, "bridge: could not report a provider refusal");
            }
            return;
        }
    };

    let head = stream.head().to_payload();
    if inner
        .send(&Envelope::Res(Res {
            id,
            ok: true,
            payload: Some(head),
            error: None,
        }))
        .await
        .is_err()
    {
        return;
    }

    let mut seq = 0u64;
    let error = loop {
        match stream.next_chunk().await {
            Ok(None) => break None,
            Ok(Some(bytes)) => {
                // §1 caps one binary tail at MAX_BIN_LEN. A provider that hands
                // back a larger buffer in one piece is split across frames rather
                // than failing a frame the codec would refuse to encode.
                for part in bytes.chunks(MAX_BIN_LEN as usize) {
                    if inner.send_chunk(id, seq, part.to_vec()).await.is_err() {
                        return;
                    }
                    seq += 1;
                }
            }
            Err(err) => {
                tracing::debug!(target: "assistant", id, error = %err, "bridge: provider.fetch body failed");
                break Some(ErrorObject {
                    code: err.code().to_string(),
                    message: err.to_string(),
                });
            }
        }
    };

    let _ = inner
        .send(&Envelope::End(End {
            id,
            ok: error.is_none(),
            error,
        }))
        .await;
}

fn fail(inner: &Arc<Inner>, id: u64, code: &str, message: &str) {
    inner.try_send(&Envelope::Res(Res {
        id,
        ok: false,
        payload: None,
        error: Some(ErrorObject {
            code: code.to_string(),
            message: message.to_string(),
        }),
    }));
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_assistant_protocol::{decode_frame, encode_frame};
    use serde_json::json;
    use tokio::io::DuplexStream;

    /// A scripted peer on the sidecar's end of an in-process duplex.
    ///
    /// The real compiled sidecar cannot be made to lie — it always sends a valid
    /// `hello` and never forges a principal — so the refusal paths are asserted
    /// against a peer this test controls byte for byte.
    struct ScriptedPeer {
        io: DuplexStream,
        buf: BytesMut,
    }

    impl ScriptedPeer {
        async fn send(&mut self, envelope: &Envelope) {
            self.send_with_bin(envelope, &[]).await;
        }

        async fn send_with_bin(&mut self, envelope: &Envelope, bin: &[u8]) {
            let bytes = encode_frame(&envelope.to_json_vec().unwrap(), bin).unwrap();
            self.io.write_all(&bytes).await.unwrap();
            self.io.flush().await.unwrap();
        }

        /// The next envelope the host sent, or `None` at EOF.
        async fn recv(&mut self) -> Option<Envelope> {
            loop {
                if let Some((frame, consumed)) = decode_frame(&self.buf).unwrap() {
                    let _ = self.buf.split_to(consumed);
                    return Some(Envelope::from_json_slice(&frame.json).unwrap());
                }
                if self.io.read_buf(&mut self.buf).await.unwrap() == 0 {
                    return None;
                }
            }
        }
    }

    fn hello(version: u32) -> Envelope {
        Envelope::Hello(Hello {
            protocol_version: version,
            host_version: "0.1.0".into(),
            agentkit_contract_version: "test".into(),
            pid: 4242,
            session_nonce: "nonce".into(),
        })
    }

    /// Wires a bridge to a scripted peer that has already sent `hello`.
    async fn connected(options: BridgeOptions) -> (AssistantBridge, ScriptedPeer) {
        let (host_io, peer_io) = tokio::io::duplex(64 * 1024);
        let mut peer = ScriptedPeer {
            io: peer_io,
            buf: BytesMut::new(),
        };
        peer.send(&hello(PROTOCOL_VERSION)).await;
        let (rx, tx) = tokio::io::split(host_io);
        let bridge = AssistantBridge::connect(rx, tx, options)
            .await
            .expect("handshake");
        // The host's `accept` is the first thing the peer reads back.
        match peer.recv().await {
            Some(Envelope::Accept(accept)) => {
                assert_eq!(accept.protocol_version, PROTOCOL_VERSION);
                assert_eq!(accept.bridge_version, BRIDGE_VERSION);
            }
            other => panic!("expected accept, got {other:?}"),
        }
        (bridge, peer)
    }

    #[tokio::test]
    async fn handshake_accepts_a_matching_version() {
        let (bridge, _peer) = connected(BridgeOptions::default()).await;
        assert_eq!(bridge.hello().pid, 4242);
        assert!(!bridge.is_closed());
    }

    #[tokio::test]
    async fn handshake_rejects_an_incompatible_version_before_any_other_frame() {
        let (host_io, peer_io) = tokio::io::duplex(64 * 1024);
        let mut peer = ScriptedPeer {
            io: peer_io,
            buf: BytesMut::new(),
        };
        peer.send(&hello(PROTOCOL_VERSION + 1)).await;
        let (rx, tx) = tokio::io::split(host_io);
        let err = AssistantBridge::connect(rx, tx, BridgeOptions::default())
            .await
            .expect_err("an incompatible version must be refused");
        assert!(matches!(err, BridgeError::Handshake(_)), "{err}");

        // The refusal is on the wire, and it is the ONLY frame the peer ever sees.
        match peer.recv().await {
            Some(Envelope::Reject(reject)) => {
                assert!(
                    reject.reason.contains("unsupported protocol version 2"),
                    "{reject:?}"
                );
            }
            other => panic!("expected reject, got {other:?}"),
        }
        assert!(peer.recv().await.is_none(), "nothing follows a reject");
    }

    #[tokio::test]
    async fn handshake_refuses_anything_but_hello_first() {
        let (host_io, peer_io) = tokio::io::duplex(64 * 1024);
        let mut peer = ScriptedPeer {
            io: peer_io,
            buf: BytesMut::new(),
        };
        peer.send(&Envelope::Ping(Ping { id: 1 })).await;
        let (rx, tx) = tokio::io::split(host_io);
        let err = AssistantBridge::connect(rx, tx, BridgeOptions::default())
            .await
            .expect_err("hello must be first");
        match err {
            BridgeError::Handshake(reason) => {
                assert!(reason.contains("expected hello"), "{reason}")
            }
            other => panic!("expected a handshake failure, got {other}"),
        }
    }

    /// THE authority test. A sidecar `req` that claims `principal: "ui"` must not
    /// reach a `ui`-only route: the claim is overwritten with `host`, and
    /// `agentkit.fetch` is not in the sidecar→host table at all.
    #[tokio::test]
    async fn a_sidecar_request_claiming_ui_is_refused() {
        let (_bridge, mut peer) = connected(BridgeOptions::default()).await;
        peer.send(&Envelope::Req(Req {
            id: 1, // odd: a legal sidecar id
            principal: Principal::Ui,
            verb: "agentkit.fetch".into(),
            payload: json!({"method": "GET", "path": "/v1/version"}),
        }))
        .await;

        match peer.recv().await {
            Some(Envelope::Res(res)) => {
                assert_eq!(res.id, 1);
                assert!(
                    !res.ok,
                    "a ui-only verb must never be served to the sidecar"
                );
                assert_eq!(res.error.expect("error body").code, "unknown_verb");
            }
            other => panic!("expected a refusing res, got {other:?}"),
        }
    }

    /// The stamp also applies to a verb the sidecar IS allowed to call: the
    /// request is accepted as `host`, and answered `unimplemented` because THIS
    /// bridge was built with no provider gateway. The route is absent, not
    /// disabled — a connection with no registry has no provider to name.
    #[tokio::test]
    async fn a_sidecar_provider_fetch_is_stamped_host_and_has_no_route_here() {
        let (_bridge, mut peer) = connected(BridgeOptions::default()).await;
        peer.send(&Envelope::Req(Req {
            id: 3,
            principal: Principal::Ui, // lying again; it changes nothing
            verb: "provider.fetch".into(),
            payload: json!({"providerId": "local"}),
        }))
        .await;

        match peer.recv().await {
            Some(Envelope::Res(res)) => {
                assert!(!res.ok);
                assert_eq!(res.error.expect("error body").code, "unimplemented");
            }
            other => panic!("expected a refusing res, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_inbound_request_with_a_host_allocated_id_is_refused() {
        let (_bridge, mut peer) = connected(BridgeOptions::default()).await;
        peer.send(&Envelope::Req(Req {
            id: 2, // even: the host's own id space (§3)
            principal: Principal::Host,
            verb: "provider.fetch".into(),
            payload: Value::Null,
        }))
        .await;
        match peer.recv().await {
            Some(Envelope::Res(res)) => {
                assert!(!res.ok);
                assert_eq!(res.error.expect("error body").code, "bad_request");
            }
            other => panic!("expected a refusing res, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_host_refuses_to_send_a_verb_its_principal_may_not_call() {
        let (bridge, _peer) = connected(BridgeOptions::default()).await;
        // `shutdown` is ui-only; the host principal may not call it.
        let err = bridge
            .request("shutdown", Principal::Host, Value::Null)
            .await
            .expect_err("a host-principal shutdown must be refused locally");
        assert!(
            matches!(
                err,
                BridgeError::Protocol(ProtocolError::VerbRefused { .. })
            ),
            "{err}"
        );
        // And an unknown verb never reaches the wire either.
        let err = bridge
            .request("fs.read", Principal::Ui, Value::Null)
            .await
            .expect_err("an unlisted verb must be refused locally");
        assert!(
            matches!(err, BridgeError::Protocol(ProtocolError::UnknownVerb(_))),
            "{err}"
        );
    }

    /// §6: the reader keeps draining while an outbound request is pending. The
    /// peer answers an inbound `ping` and an inbound `req` BEFORE it answers the
    /// in-flight `agentkit.fetch`; a lockstep reader would deadlock here.
    #[tokio::test]
    async fn the_reader_keeps_draining_while_a_request_is_in_flight() {
        let (bridge, mut peer) = connected(BridgeOptions::default()).await;
        let bridge = Arc::new(bridge);
        let caller = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .request("agentkit.fetch", Principal::Ui, json!({"method": "GET"}))
                    .await
            })
        };

        let req_id = match peer.recv().await {
            Some(Envelope::Req(req)) => {
                assert_eq!(req.principal, Principal::Ui);
                req.id
            }
            other => panic!("expected a req, got {other:?}"),
        };

        // Mid-request traffic in the other direction, answered immediately.
        peer.send(&Envelope::Ping(Ping { id: 9 })).await;
        assert!(matches!(
            peer.recv().await,
            Some(Envelope::Pong(Pong { id: 9 }))
        ));
        peer.send(&Envelope::Req(Req {
            id: 11,
            principal: Principal::Host,
            verb: "provider.fetch".into(),
            payload: Value::Null,
        }))
        .await;
        assert!(matches!(peer.recv().await, Some(Envelope::Res(_))));

        peer.send(&Envelope::Res(Res {
            id: req_id,
            ok: true,
            payload: Some(json!({"status": 200})),
            error: None,
        }))
        .await;
        let answer = caller.await.expect("task").expect("request");
        assert_eq!(answer["status"], 200);
    }

    #[tokio::test]
    async fn a_bounded_stream_round_trips_in_order() {
        let (bridge, mut peer) = connected(BridgeOptions::default()).await;
        let bridge = Arc::new(bridge);
        let caller = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .request_stream("agentkit.fetch", Principal::Ui, json!({"method": "GET"}))
                    .await
            })
        };
        let id = match peer.recv().await {
            Some(Envelope::Req(req)) => req.id,
            other => panic!("expected a req, got {other:?}"),
        };
        peer.send(&Envelope::Res(Res {
            id,
            ok: true,
            payload: Some(json!({"status": 200})),
            error: None,
        }))
        .await;
        let mut stream = caller.await.expect("task").expect("stream head");
        assert_eq!(stream.head["status"], 200);

        for (seq, text) in ["event: a\n", "data: {}\n"].iter().enumerate() {
            peer.send_with_bin(
                &Envelope::Chunk(Chunk {
                    id,
                    seq: seq as u64,
                }),
                text.as_bytes(),
            )
            .await;
        }
        peer.send(&Envelope::End(End {
            id,
            ok: true,
            error: None,
        }))
        .await;

        let mut body = Vec::new();
        loop {
            match stream.next().await {
                Some(StreamEvent::Chunk(bytes)) => body.extend_from_slice(&bytes),
                Some(StreamEvent::End(error)) => {
                    assert!(error.is_none(), "{error:?}");
                    break;
                }
                None => panic!("stream ended without a terminator"),
            }
        }
        assert_eq!(String::from_utf8(body).unwrap(), "event: a\ndata: {}\n");
    }

    #[tokio::test]
    async fn an_out_of_order_chunk_is_fatal() {
        let (bridge, mut peer) = connected(BridgeOptions::default()).await;
        let bridge = Arc::new(bridge);
        let caller = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .request_stream("agentkit.fetch", Principal::Ui, Value::Null)
                    .await
            })
        };
        let id = match peer.recv().await {
            Some(Envelope::Req(req)) => req.id,
            other => panic!("expected a req, got {other:?}"),
        };
        peer.send(&Envelope::Res(Res {
            id,
            ok: true,
            payload: Some(json!({})),
            error: None,
        }))
        .await;
        let mut stream = caller.await.expect("task").expect("stream head");
        // seq 1 with no seq 0: a gap, which §2 makes fatal.
        peer.send_with_bin(&Envelope::Chunk(Chunk { id, seq: 1 }), b"x")
            .await;

        match stream.next().await {
            Some(StreamEvent::End(Some(error))) => assert_eq!(error.code, "bridge_closed"),
            other => panic!("expected the stream to fail, got {other:?}"),
        }
        bridge.closed().await;
        assert!(bridge.is_closed());
    }

    /// §7: a consumer that does not drain gets one loud `stream_overflow`, never
    /// a quiet truncation — and the producer is sent `cancel`.
    #[tokio::test]
    async fn an_oversized_stream_overflows_loudly_and_cancels_the_producer() {
        let options = BridgeOptions {
            max_stream_buffer_bytes: 8,
            ..BridgeOptions::default()
        };
        let (bridge, mut peer) = connected(options).await;
        let bridge = Arc::new(bridge);
        let caller = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .request_stream("agentkit.fetch", Principal::Ui, Value::Null)
                    .await
            })
        };
        let id = match peer.recv().await {
            Some(Envelope::Req(req)) => req.id,
            other => panic!("expected a req, got {other:?}"),
        };
        peer.send(&Envelope::Res(Res {
            id,
            ok: true,
            payload: Some(json!({})),
            error: None,
        }))
        .await;
        let mut stream = caller.await.expect("task").expect("stream head");

        // Nothing is read between these two, so both are charged to the budget.
        peer.send_with_bin(&Envelope::Chunk(Chunk { id, seq: 0 }), b"12345")
            .await;
        peer.send_with_bin(&Envelope::Chunk(Chunk { id, seq: 1 }), b"67890")
            .await;

        // The bytes that DID arrive are delivered first; then the failure.
        match stream.next().await {
            Some(StreamEvent::Chunk(bytes)) => assert_eq!(bytes, b"12345"),
            other => panic!("expected the first chunk, got {other:?}"),
        }
        match stream.next().await {
            Some(StreamEvent::End(Some(error))) => {
                assert_eq!(error.code, "stream_overflow");
                assert!(error.message.contains("8-byte buffer"), "{}", error.message);
            }
            other => panic!("expected a stream_overflow end, got {other:?}"),
        }
        match peer.recv().await {
            Some(Envelope::Cancel(cancel)) => assert_eq!(cancel.id, id),
            other => panic!("expected a cancel for the overflowed stream, got {other:?}"),
        }
        // The connection itself survives: an overflow is a stream failure, not a
        // framing violation.
        assert!(!bridge.is_closed());
    }

    #[tokio::test]
    async fn dropping_a_stream_cancels_it() {
        let (bridge, mut peer) = connected(BridgeOptions::default()).await;
        let bridge = Arc::new(bridge);
        let caller = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .request_stream("agentkit.fetch", Principal::Ui, Value::Null)
                    .await
            })
        };
        let id = match peer.recv().await {
            Some(Envelope::Req(req)) => req.id,
            other => panic!("expected a req, got {other:?}"),
        };
        peer.send(&Envelope::Res(Res {
            id,
            ok: true,
            payload: Some(json!({})),
            error: None,
        }))
        .await;
        let stream = caller.await.expect("task").expect("stream head");
        drop(stream);

        match peer.recv().await {
            Some(Envelope::Cancel(cancel)) => assert_eq!(cancel.id, id),
            other => panic!("expected a cancel, got {other:?}"),
        }
        // §2: a late `end` for a cancelled id is discarded, not an error.
        peer.send(&Envelope::End(End {
            id,
            ok: true,
            error: None,
        }))
        .await;
        peer.send(&Envelope::Ping(Ping { id: 5 })).await;
        assert!(matches!(
            peer.recv().await,
            Some(Envelope::Pong(Pong { id: 5 }))
        ));
    }

    #[tokio::test]
    async fn a_pending_request_fails_when_the_peer_disappears() {
        let (bridge, peer) = connected(BridgeOptions::default()).await;
        let bridge = Arc::new(bridge);
        let caller = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .request("agentkit.fetch", Principal::Ui, Value::Null)
                    .await
            })
        };
        // Give the request time to register before the transport goes away.
        tokio::task::yield_now().await;
        drop(peer);
        let err = caller
            .await
            .expect("task")
            .expect_err("a dead peer fails the request");
        assert!(matches!(err, BridgeError::Closed(_)), "{err}");
        bridge.closed().await;
        // A request issued after the teardown fails fast rather than hanging.
        let err = bridge
            .request("agentkit.fetch", Principal::Ui, Value::Null)
            .await
            .expect_err("a closed bridge refuses new work");
        assert!(matches!(err, BridgeError::Closed(_)), "{err}");
    }

    #[tokio::test]
    async fn ping_resolves_on_its_pong() {
        let (bridge, mut peer) = connected(BridgeOptions::default()).await;
        let bridge = Arc::new(bridge);
        let caller = {
            let bridge = bridge.clone();
            tokio::spawn(async move { bridge.ping().await })
        };
        let id = match peer.recv().await {
            Some(Envelope::Ping(ping)) => ping.id,
            other => panic!("expected a ping, got {other:?}"),
        };
        assert_eq!(id % 2, 0, "host ids are even (§3)");
        peer.send(&Envelope::Pong(Pong { id })).await;
        caller.await.expect("task").expect("pong");
    }
}
