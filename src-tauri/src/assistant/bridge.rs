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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::BytesMut;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Notify, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::codec::{Decoder, Encoder};
use tokio_util::sync::CancellationToken;

use onecad_assistant_protocol::{
    Accept, Cancel, Chunk, End, Envelope, ErrorObject, Hello, IdAllocator, OcakCodec, Ping, Pong,
    Principal, ProtocolError, RawFrame, Reject, Req, Res, HOST_TO_SIDECAR_VERBS, MAX_BIN_LEN,
    MAX_JSON_LEN, PROTOCOL_VERSION, SIDECAR_TO_HOST_VERBS,
};

use super::provider_gateway::{ProviderGateway, ProviderRegistry};

/// The one verb the sidecar may call on this host (§5).
const VERB_PROVIDER_FETCH: &str = "provider.fetch";

/// This host-side bridge implementation's own version, reported in `accept`.
/// Tracks `BRIDGE_VERSION` in `assistant-host/src/bridge/peer.ts`.
pub const BRIDGE_VERSION: &str = "0.1.0";

/// The AgentKit contract versions this host can serve (§2, `hello`).
///
/// `protocolVersion` gates the FRAMING; this gates the DTOs that ride inside it.
/// A sidecar built against a different contract speaks the same envelopes and a
/// different `agentkit.fetch` payload, so the handshake is the last point at
/// which the mismatch is still one clear refusal rather than a decode error in
/// the middle of somebody's first message.
///
/// Exact match against a list, not a range: AgentKit is pre-1.0, where a minor
/// bump is allowed to break anything. Two places move together when it does —
/// this constant, and the `agentkitContractVersion` field
/// `scripts/build-assistant-host.sh` records for the shipped binary. A mismatch
/// between them is a packaging bug the handshake will report on first use.
pub const SUPPORTED_AGENTKIT_CONTRACTS: &[&str] = &["0.5.0"];

/// Default per-stream inbound buffer bound (§7), matching the sidecar's
/// `DEFAULT_STREAM_BUFFER_BYTES`. One `stream_overflow` beyond this.
pub const DEFAULT_STREAM_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Default per-stream inbound item bound (§9). A second axis beside the byte
/// bound: a flood of one-byte chunks charges almost nothing against
/// [`DEFAULT_STREAM_BUFFER_BYTES`] while costing a queue slot each.
pub const DEFAULT_STREAM_BUFFER_ITEMS: usize = 4096;

/// Depth of the CONTROL frame queue (§9). Control frames are small and never
/// carry a tail, so this is a count bound and needs no byte budget.
///
/// Separate from the body queue on purpose: a control frame that cannot be
/// delivered fails the connection, and sharing one queue with 8 MiB body frames
/// would make that failure a routine consequence of a busy stream rather than
/// the signal it is meant to be.
const CONTROL_QUEUE_DEPTH: usize = 64;

/// Depth of the outbound BODY frame queue, in frames (§9). The byte budget
/// below is the real bound; this only keeps the queue from holding many small
/// frames' worth of per-frame overhead.
const BODY_QUEUE_DEPTH: usize = 32;

/// Aggregate bytes this process will hold in the outbound body queue (§9).
///
/// Frame COUNT is not a budget: 32 near-cap frames is a quarter of a gigabyte.
/// Every queued body frame takes credits proportional to its size and returns
/// them once the writer has flushed it, so the queue's memory is bounded by this
/// number rather than by how large the frames happen to be.
const MAX_QUEUED_BODY_BYTES: usize = 16 * 1024 * 1024;

/// One outbound-body credit, in bytes. A frame takes `ceil(len / CREDIT)` of
/// them, so the accounting overshoots by at most one credit per frame.
const QUEUE_CREDIT_BYTES: usize = 64 * 1024;

/// Concurrent outbound requests this host will have in flight on one connection
/// (§9). Reached only by a caller looping without bound; refused rather than
/// allowed to grow the correlation table forever.
const MAX_ACTIVE_OUTBOUND: usize = 64;

/// Concurrent inbound requests this host will serve on one connection (§9).
/// Beyond this the sidecar is refused with `too_many_requests`, which is a
/// backpressure signal it can act on — unlike an unbounded task spawn.
const MAX_ACTIVE_INBOUND: usize = 64;

/// How long an outbound request waits for its response HEAD before giving up
/// (§9).
///
/// A head, not a body: `agentkit.fetch` is answered out of the sidecar's memory,
/// so a head that has not arrived in this long means the child is wedged, not
/// slow. The BODY that follows is deliberately not on a deadline — an SSE
/// subscription is long-lived by design and a finite RPC timeout applied to it
/// would cancel healthy runs.
pub const REQUEST_HEAD_DEADLINE: Duration = Duration::from_secs(30);

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
    /// A peer broke the connection or per-request state machine of contract
    /// §2a — a second response head, a chunk before one, an id nobody issued.
    /// Fatal: the connection is torn down and the supervisor restarts the child.
    #[error("assistant bridge state violation: {0}")]
    StateViolation(String),
    /// The response head did not arrive within [`REQUEST_HEAD_DEADLINE`]. The
    /// correlation slot is reclaimed and the peer is sent `cancel`; a request
    /// with no deadline pins its slot for as long as the child stays silent.
    #[error("assistant bridge request timed out after {seconds}s waiting for a response head")]
    Timeout {
        /// The deadline that expired, in seconds.
        seconds: u64,
    },
    /// A §9 budget refused the request: too many in flight on this connection.
    /// Distinct from [`BridgeError::Closed`] because the connection is healthy
    /// and the caller may retry once something finishes.
    #[error("assistant bridge is at capacity: {0}")]
    Busy(String),
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
    budget: Arc<StreamBudget>,
    inner: Arc<Inner>,
    ended: bool,
}

/// What one inbound stream currently holds against its §9 bounds.
///
/// Two axes, because one does not imply the other: bytes bound a fast producer
/// and items bound a producer sending many tiny chunks, which charges almost
/// nothing in bytes while costing a queue slot and a wakeup each. Charged by the
/// reader as a chunk is queued and released by [`StreamResponse::next`] as the
/// consumer takes it, so the budget measures what is actually held rather than
/// what has passed through.
#[derive(Debug, Default)]
struct StreamBudget {
    bytes: AtomicUsize,
    items: AtomicUsize,
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
                self.budget.bytes.fetch_sub(bytes.len(), Ordering::SeqCst);
                self.budget.items.fetch_sub(1, Ordering::SeqCst);
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
        // Settled locally and detached WITHOUT waiting for the peer (§2a): a
        // sidecar that never answers must not pin this registration forever.
        self.inner.forget(self.id);
        self.inner.cancel_outbound(self.id);
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
    budget: Arc<StreamBudget>,
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
        budget: Arc<StreamBudget>,
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
    /// Per-stream inbound buffer bound, in bytes (§7, §9).
    pub max_stream_buffer_bytes: usize,
    /// Per-stream inbound buffer bound, in chunks (§9). The second axis: a
    /// thousand one-byte chunks cost a thousand queue slots and almost no bytes.
    pub max_stream_buffer_items: usize,
    /// How long an outbound request waits for its response head (§9). Defaults
    /// to [`REQUEST_HEAD_DEADLINE`]; a test shortens it rather than sleeping
    /// through the production budget.
    pub request_head_deadline: Duration,
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
            max_stream_buffer_items: DEFAULT_STREAM_BUFFER_ITEMS,
            request_head_deadline: REQUEST_HEAD_DEADLINE,
            provider_registry: Arc::new(ProviderRegistry::new()),
        }
    }
}

/// One frame on its way out, holding the queue credits it took (§9).
///
/// The credits ride WITH the frame rather than being released at `send` time:
/// they are what bounds the queue's memory, so they must be returned by the
/// writer once the bytes are gone, not by the producer once they are handed over.
struct Outgoing {
    frame: RawFrame,
    credit: Option<OwnedSemaphorePermit>,
}

impl Outgoing {
    fn control(frame: RawFrame) -> Self {
        Outgoing {
            frame,
            credit: None,
        }
    }
}

/// Why an inbound `req` could not be registered (§2a, §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InboundRefusal {
    /// The id is already in flight. The existing registration is untouched.
    DuplicateId,
    /// [`MAX_ACTIVE_INBOUND`] handlers are already running.
    TooMany,
}

impl InboundRefusal {
    const fn code(self) -> &'static str {
        match self {
            InboundRefusal::DuplicateId => "duplicate_id",
            InboundRefusal::TooMany => "too_many_requests",
        }
    }
}

/// Encodes one control envelope and checks its §1 cap BEFORE it is queued.
///
/// The writer fails the whole connection now, which makes an avoidable encode
/// failure expensive: a caller that hands over an over-cap payload would take
/// down a healthy child rather than getting told no. Checked here, the refusal
/// belongs to the request that caused it.
fn control_frame(envelope: &Envelope) -> Result<RawFrame, BridgeError> {
    let json = envelope.to_json_vec()?;
    if json.len() > MAX_JSON_LEN as usize {
        return Err(BridgeError::Protocol(ProtocolError::TooLarge {
            what: "json",
            len: u32::try_from(json.len()).unwrap_or(u32::MAX),
            cap: MAX_JSON_LEN,
        }));
    }
    Ok(RawFrame::json_only(json))
}

/// Outbound-body credits a frame of `len` bytes takes, rounded up (§9).
fn credits_for(len: usize) -> u32 {
    let credits = len.div_ceil(QUEUE_CREDIT_BYTES).max(1);
    u32::try_from(credits).unwrap_or(u32::MAX)
}

struct Inner {
    /// Host ids are EVEN (§3); the sidecar owns the odd ones.
    ids: IdAllocator,
    /// Small control frames: `req`, `res`, `end`, `cancel`, `ping`, `pong`.
    control: mpsc::Sender<Outgoing>,
    /// `chunk` frames and their tails, bounded by [`Inner::body_credit`].
    body: mpsc::Sender<Outgoing>,
    body_credit: Arc<Semaphore>,
    pending: Mutex<PendingTable>,
    /// Cancellation tokens for the inbound requests this host is still serving.
    ///
    /// `provider.fetch` is the first inbound verb whose handler outlives its
    /// dispatch, which is what makes this table necessary: a `cancel` for a
    /// request already talking to a provider has to reach the task holding that
    /// response, or the upstream call keeps generating into a socket nobody will
    /// read.
    ///
    /// Keyed by id and TICKETED: cleanup names the ticket it registered, never
    /// the bare integer, so a handler retiring late cannot remove an entry that
    /// now belongs to something else (§2a).
    inbound: Mutex<HashMap<u64, (u64, CancellationToken)>>,
    next_ticket: AtomicU64,
    closed: AtomicBool,
    closed_signal: Notify,
    /// Cancelled by [`Inner::fail_all`]. **The one connection-failure authority**:
    /// whichever half notices first, both IO tasks stop on this token, every
    /// waiter is settled, and `closed_signal` wakes the supervisor. A writer that
    /// merely returned would leave a half-open peer looking Ready forever.
    shutdown: CancellationToken,
    stream_cap: usize,
    stream_item_cap: usize,
    head_deadline: Duration,
    provider: Arc<ProviderRegistry>,
}

impl Inner {
    /// Queues a control frame without ever blocking the caller.
    ///
    /// Used by the reader task, which MUST keep draining (§6): awaiting a full
    /// outbound queue there would couple inbound progress to a child that has
    /// stopped reading its stdin, i.e. deadlock the bridge to avoid dropping a
    /// pong.
    ///
    /// **A control frame that cannot be delivered fails the connection.** The
    /// control queue is reserved — body frames cannot occupy it — so a full one
    /// is not a busy stream, it is a child that has stopped reading its stdin
    /// for [`CONTROL_QUEUE_DEPTH`] frames. Logging and continuing was the old
    /// behaviour, and it silently dropped the `cancel` that stops a runaway
    /// producer and the `res` that settles a waiting request.
    fn try_send(&self, envelope: &Envelope) -> Result<(), BridgeError> {
        let raw = control_frame(envelope)?;
        match self.control.try_send(Outgoing::control(raw)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                let reason = format!(
                    "control frame '{}' could not be delivered: outbound control queue full",
                    envelope.tag()
                );
                tracing::warn!(target: "assistant", tag = envelope.tag(), "bridge: {reason}");
                self.fail_all(&reason);
                Err(BridgeError::Closed(reason))
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(BridgeError::Closed("writer task ended".into()))
            }
        }
    }

    /// Best-effort control send from a `Drop` or a cleanup path, where there is
    /// no caller left to report to. The connection-failure authority still runs.
    fn try_send_detached(&self, envelope: &Envelope) {
        let _ = self.try_send(envelope);
    }

    /// Settles an outbound request locally and tells the peer to stop (§2a).
    fn cancel_outbound(&self, id: u64) {
        self.try_send_detached(&Envelope::Cancel(Cancel { id }));
    }

    async fn send(&self, envelope: &Envelope) -> Result<(), BridgeError> {
        let raw = control_frame(envelope)?;
        self.control
            .send(Outgoing::control(raw))
            .await
            .map_err(|_| BridgeError::Closed("writer task ended".into()))
    }

    /// Queues one outbound `chunk` with its binary tail.
    ///
    /// Awaits both the byte credits and the bounded frame queue rather than
    /// `try_send`ing: this is a BODY frame, and dropping one would truncate a
    /// response silently. The wait is the backpressure that keeps a provider
    /// streaming faster than the sidecar reads from turning into unbounded
    /// memory here — and the credits are what make that bound a byte count
    /// rather than a frame count, which for 8 MiB frames is not a bound at all.
    ///
    /// An empty chunk is never queued: §2a makes a tailless `chunk` fatal at the
    /// receiver, so producing one would be this side inventing a frame the peer
    /// must tear the connection down over.
    async fn send_chunk(&self, id: u64, seq: u64, bin: Vec<u8>) -> Result<(), BridgeError> {
        debug_assert!(!bin.is_empty(), "an empty chunk is not a legal frame");
        // A tail past the frame cap would ask for more credits than the budget
        // holds, and an unsatisfiable acquire waits forever. Callers split by
        // `MAX_BIN_LEN` before they get here; this refuses rather than hangs if
        // one ever stops.
        if bin.len() > MAX_BIN_LEN as usize {
            return Err(BridgeError::Protocol(ProtocolError::TooLarge {
                what: "bin",
                len: u32::try_from(bin.len()).unwrap_or(u32::MAX),
                cap: MAX_BIN_LEN,
            }));
        }
        let credit = self
            .body_credit
            .clone()
            .acquire_many_owned(credits_for(bin.len()))
            .await
            .map_err(|_| BridgeError::Closed("writer task ended".into()))?;
        let json = Envelope::Chunk(Chunk { id, seq }).to_json_vec()?;
        self.body
            .send(Outgoing {
                frame: RawFrame { json, bin },
                credit: Some(credit),
            })
            .await
            .map_err(|_| BridgeError::Closed("writer task ended".into()))
    }

    /// Queues a frame that must stay BEHIND this request's body.
    ///
    /// The writer drains control before body, which is what keeps a `cancel`
    /// from queueing behind the flood it is meant to stop — and it is also why a
    /// stream's `end` cannot travel on the control lane. `end` is the terminator
    /// of a chunk sequence (§2), so sending it there lets it overtake chunks
    /// that are still queued and puts `res → end → chunk` on the wire. Per-request
    /// order is total precisely because every frame after the head rides the one
    /// lane, in the order it was produced.
    async fn send_after_body(&self, envelope: &Envelope) -> Result<(), BridgeError> {
        let frame = control_frame(envelope)?;
        let credit = self
            .body_credit
            .clone()
            .acquire_many_owned(1)
            .await
            .map_err(|_| BridgeError::Closed("writer task ended".into()))?;
        self.body
            .send(Outgoing {
                frame,
                credit: Some(credit),
            })
            .await
            .map_err(|_| BridgeError::Closed("writer task ended".into()))
    }

    /// Registers an inbound request's cancellation token, returning the TICKET
    /// that owns it. Registered BEFORE the handler is spawned, so a `cancel` can
    /// never arrive in the window between the task starting and the table
    /// learning about it.
    ///
    /// **An id that is still in flight is refused, and the existing registration
    /// is not touched** (§2a). Replacing it was the old behaviour and it created
    /// an unsafe ownership transfer: the displaced handler's unconditional
    /// `finish_inbound(id)` then removed its replacement's token, after which a
    /// `cancel` for that id reached nothing at all. Ownership is bound to the
    /// ticket, never to the bare integer.
    fn track_inbound(&self, id: u64, token: CancellationToken) -> Result<u64, InboundRefusal> {
        let mut table = self
            .inbound
            .lock()
            .expect("assistant inbound table poisoned");
        if table.contains_key(&id) {
            return Err(InboundRefusal::DuplicateId);
        }
        if table.len() >= MAX_ACTIVE_INBOUND {
            return Err(InboundRefusal::TooMany);
        }
        let ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        table.insert(id, (ticket, token));
        Ok(ticket)
    }

    /// Retires an inbound request, but only if `ticket` still owns the slot.
    fn finish_inbound(&self, id: u64, ticket: u64) {
        let mut table = self
            .inbound
            .lock()
            .expect("assistant inbound table poisoned");
        if table.get(&id).is_some_and(|(owner, _)| *owner == ticket) {
            table.remove(&id);
        }
    }

    fn cancel_inbound(&self, id: u64) {
        let token = self
            .inbound
            .lock()
            .expect("assistant inbound table poisoned")
            .remove(&id);
        if let Some((_, token)) = token {
            token.cancel();
        }
    }

    /// Registers a correlation slot, refusing once the connection has closed or
    /// the §9 in-flight bound is reached.
    fn register(&self, id: u64, pending: Pending) -> Result<(), BridgeError> {
        let mut table = self
            .pending
            .lock()
            .expect("assistant pending table poisoned");
        if table.closed {
            return Err(BridgeError::Closed("connection closed".into()));
        }
        if table.entries.len() >= MAX_ACTIVE_OUTBOUND {
            return Err(BridgeError::Busy(format!(
                "{MAX_ACTIVE_OUTBOUND} requests are already in flight on this connection"
            )));
        }
        table.entries.insert(id, pending);
        Ok(())
    }

    /// Whether `id` is one this host allocated and has since settled.
    ///
    /// §2a's id ladder needs three outcomes from a correlation miss, not two: an
    /// id in our space below the allocation cursor is a late frame for a request
    /// we cancelled and is discarded, while an id of the wrong parity or one
    /// nobody has allocated yet is a peer inventing correlation and is fatal.
    /// Treating both as "unknown, ignore" is how a diverged allocator goes
    /// unnoticed until it crosses two responses.
    fn is_settled_outbound(&self, id: u64) -> bool {
        id.is_multiple_of(2) && id < self.ids.peek()
    }

    fn forget(&self, id: u64) {
        self.pending
            .lock()
            .expect("assistant pending table poisoned")
            .entries
            .remove(&id);
    }

    /// The §9 in-flight counters: `(outbound, inbound)`.
    ///
    /// Read by the tests that assert a request's registration comes back to
    /// baseline after a timeout, a drop or a duplicate refusal — "it settled" is
    /// not the same claim as "nothing is still pinned".
    #[cfg(test)]
    fn in_flight(&self) -> (usize, usize) {
        let outbound = self
            .pending
            .lock()
            .expect("assistant pending table poisoned")
            .entries
            .len();
        let inbound = self
            .inbound
            .lock()
            .expect("assistant inbound table poisoned")
            .len();
        (outbound, inbound)
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// **The connection-failure authority.** Drains every waiter, cancels every
    /// inbound handler, stops both IO tasks and notifies the supervisor.
    /// Idempotent, and called from either half: whichever notices first, the
    /// whole connection ends in one place.
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
        for (_, (_, token)) in self
            .inbound
            .lock()
            .expect("assistant inbound table poisoned")
            .drain()
        {
            token.cancel();
        }
        self.closed.store(true, Ordering::SeqCst);
        // Stops BOTH IO tasks, whichever half noticed the failure. A writer that
        // returned quietly used to leave the reader parked on a half-open pipe,
        // and the supervisor reading Ready off a connection nothing could use.
        self.shutdown.cancel();
        // Wakes anything blocked on outbound-body credits so it fails rather
        // than waiting for a writer that has stopped returning them.
        self.body_credit.close();
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

        // §2: an incompatible pair is refused BEFORE any other frame is
        // exchanged, so it can never perform a partial operation. The contract
        // version was advertised and then ignored, which let a sidecar built
        // against DTOs this host does not understand activate a connection and
        // fail later, one request at a time.
        if !SUPPORTED_AGENTKIT_CONTRACTS.contains(&hello.agentkit_contract_version.as_str()) {
            let reason = format!(
                "unsupported AgentKit contract version {}; this host serves {}",
                hello.agentkit_contract_version,
                SUPPORTED_AGENTKIT_CONTRACTS.join(", ")
            );
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

        let (control_tx, control_rx) = mpsc::channel::<Outgoing>(CONTROL_QUEUE_DEPTH);
        let (body_tx, body_rx) = mpsc::channel::<Outgoing>(BODY_QUEUE_DEPTH);
        let inner = Arc::new(Inner {
            ids: IdAllocator::host(),
            control: control_tx,
            body: body_tx,
            body_credit: Arc::new(Semaphore::new(MAX_QUEUED_BODY_BYTES / QUEUE_CREDIT_BYTES)),
            pending: Mutex::new(PendingTable::default()),
            inbound: Mutex::new(HashMap::new()),
            next_ticket: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            closed_signal: Notify::new(),
            shutdown: CancellationToken::new(),
            stream_cap: options.max_stream_buffer_bytes,
            stream_item_cap: options.max_stream_buffer_items,
            head_deadline: options.request_head_deadline,
            provider: options.provider_registry.clone(),
        });

        let writer_handle = tokio::spawn(writer_task(writer, control_rx, body_rx, inner.clone()));
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
        // Armed from here on: every early return, every `?`, and a caller that
        // drops this future mid-await all retire the slot and tell the peer.
        let mut guard = PendingGuard::arm(&self.inner, id);
        self.send_req(id, principal, verb, payload).await?;
        let res = await_head(self.inner.head_deadline, rx).await??;
        guard.disarm();
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
        let mut guard = PendingGuard::arm(&self.inner, id);
        self.send_req(id, principal, verb, payload).await?;
        let delivery = await_head(self.inner.head_deadline, rx).await??;
        // Ownership of the cancellation passes to the `StreamResponse`, whose
        // own `Drop` sends `cancel` if the body is abandoned before its `end`.
        guard.disarm();
        Ok(StreamResponse {
            id,
            head: delivery.head,
            rx: delivery.rx,
            budget: delivery.budget,
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
        // Interrupted by retirement, not merely by a `pong` that will never come:
        // `fail_all` drops the ping's sender, so this resolves as soon as the
        // connection fails instead of holding the supervisor's probe open.
        tokio::select! {
            biased;
            () = self.inner.shutdown.cancelled() => {
                Err(BridgeError::Closed("connection closed while pinging".into()))
            }
            answered = rx => {
                answered.map_err(|_| BridgeError::Closed("ping never answered".into()))
            }
        }
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

/// Awaits a response head under [`REQUEST_HEAD_DEADLINE`].
///
/// A request with no deadline pins its correlation slot for as long as the child
/// stays silent, and the supervisor's own ping can keep succeeding the whole
/// time — a wedged route is not a wedged process.
async fn await_head<T>(
    deadline: Duration,
    rx: oneshot::Receiver<Result<T, BridgeError>>,
) -> Result<Result<T, BridgeError>, BridgeError> {
    match tokio::time::timeout(deadline, rx).await {
        Ok(Ok(answer)) => Ok(answer),
        Ok(Err(_)) => Err(BridgeError::Closed("response channel dropped".into())),
        Err(_) => Err(BridgeError::Timeout {
            seconds: deadline.as_secs(),
        }),
    }
}

/// RAII cleanup for one outbound correlation slot.
///
/// The slot is released and the peer is sent `cancel` unless the request
/// completed — including when the caller's future is dropped mid-await, which no
/// amount of care on the success path can cover. §2a: a request is settled
/// locally and detached WITHOUT waiting for the peer, so a sidecar that never
/// answers cannot pin a registration forever.
struct PendingGuard<'a> {
    inner: &'a Arc<Inner>,
    id: u64,
    armed: bool,
}

impl<'a> PendingGuard<'a> {
    fn arm(inner: &'a Arc<Inner>, id: u64) -> Self {
        PendingGuard {
            inner,
            id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.inner.forget(self.id);
        self.inner.cancel_outbound(self.id);
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

/// The next frame to write, control first.
///
/// `biased` is load-bearing: a `cancel` that stops a runaway producer must not
/// queue behind the megabytes that producer has already handed over.
///
/// The cost of that bias is that a frame on the control lane may overtake one on
/// the body lane, so **no request may split its post-head frames across the two**.
/// A stream's `chunk`s and its terminating `end` all ride the body lane
/// ([`Inner::send_after_body`]); the head goes out on control before any of them
/// exist, so it cannot be overtaken by its own body. Per-request order is
/// therefore total, and only unrelated requests interleave — which §2 allows.
async fn next_outgoing(
    control: &mut mpsc::Receiver<Outgoing>,
    body: &mut mpsc::Receiver<Outgoing>,
) -> Option<Outgoing> {
    tokio::select! {
        biased;
        Some(frame) = control.recv() => Some(frame),
        Some(frame) = body.recv() => Some(frame),
        else => None,
    }
}

/// Drains the outbound queues onto the child's stdin.
///
/// **Every exit from this loop fails the whole connection.** The old version
/// returned on a write error and assumed the reader would notice, which is only
/// true of a peer that closes both halves: a child holding its stdout open with
/// a broken stdin left every pending request waiting and the supervisor reading
/// Ready off a connection that could no longer carry a frame (F05).
async fn writer_task<W>(
    mut writer: W,
    mut control: mpsc::Receiver<Outgoing>,
    mut body: mpsc::Receiver<Outgoing>,
    inner: Arc<Inner>,
) where
    W: AsyncWrite + Unpin,
{
    let mut codec = OcakCodec;
    let mut out = BytesMut::new();
    let reason = loop {
        let next = tokio::select! {
            biased;
            () = inner.shutdown.cancelled() => break None,
            next = next_outgoing(&mut control, &mut body) => next,
        };
        let Some(outgoing) = next else {
            break Some("assistant bridge outbound queue closed".to_string());
        };
        out.clear();
        if let Err(err) = codec.encode(outgoing.frame, &mut out) {
            break Some(format!(
                "assistant bridge writer could not encode a frame: {err}"
            ));
        }
        if let Err(err) = writer.write_all(&out).await {
            break Some(format!("assistant bridge writer failed: {err}"));
        }
        if let Err(err) = writer.flush().await {
            break Some(format!("assistant bridge writer could not flush: {err}"));
        }
        // Returned only now: the credits bound what is HELD, and it is held
        // until the bytes are actually gone.
        drop(outgoing.credit);
    };
    if let Some(reason) = reason {
        tracing::warn!(target: "assistant", "bridge writer: {reason}");
        inner.fail_all(&reason);
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
        // Selected against the shutdown token so a WRITER failure stops this
        // half too. Without it the reader parks on a half-open pipe that will
        // never produce another frame, and nothing retires the connection.
        let read = tokio::select! {
            biased;
            () = inner.shutdown.cancelled() => return,
            read = reader.read_buf(&mut buf) => read,
        };
        match read {
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
    // §2a: the framing layer yields the tail uninterpreted by design, so the
    // pairing check belongs here. A tail on a control frame is a peer writing a
    // frame shape this protocol does not have; an EMPTY chunk is a queue slot
    // charging nothing against any byte budget (§9).
    envelope.check_binary_tail(frame.bin.len())?;
    match envelope {
        Envelope::Res(res) => on_res(inner, res),
        Envelope::Chunk(chunk) => on_chunk(inner, chunk, frame.bin),
        Envelope::End(end) => on_end(inner, end),
        Envelope::Req(req) => on_req(inner, req),
        Envelope::Cancel(cancel) => {
            // §2: cancelling an unknown id is a no-op, not an error — the request
            // may have finished a frame ago. A known id reaches the task serving
            // it, which drops the provider response and with it the upstream
            // connection.
            inner.cancel_inbound(cancel.id);
            Ok(())
        }
        Envelope::Ping(ping) => inner.try_send(&Envelope::Pong(Pong { id: ping.id })),
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

/// §2a's correlation-miss ladder, shared by `res`, `chunk` and `end`.
///
/// `Ok(())` means the frame is a late answer to something this host cancelled
/// and must be discarded; `Err` means the peer named an id it could not have
/// been answering, which is fatal.
fn discard_or_fatal(inner: &Arc<Inner>, tag: &str, id: u64) -> Result<(), BridgeError> {
    if inner.is_settled_outbound(id) {
        return Ok(());
    }
    Err(BridgeError::StateViolation(format!(
        "{tag} names request id {id}, which this host has never issued"
    )))
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
            let budget = Arc::new(StreamBudget::default());
            table.entries.insert(
                res.id,
                Pending::StreamBody {
                    tx: chunks_tx,
                    budget: budget.clone(),
                    next_seq: 0,
                },
            );
            let delivery = StreamHeadDelivery {
                head: res.payload.unwrap_or(Value::Null),
                rx: chunks_rx,
                budget,
            };
            if tx.send(Ok(delivery)).is_err() {
                // The caller went away between sending the request and reading
                // its head; drop the slot rather than buffering a body nobody
                // owns. The peer learns about it from the `cancel` below.
                table.entries.remove(&res.id);
                drop(table);
                inner.cancel_outbound(res.id);
            }
            Ok(())
        }
        // §2a: a second head is FATAL, and so is a failed `res` after a
        // successful one. It is not a replacement — the body is already owned by
        // a consumer that would be left waiting forever while the chunks flowed
        // to a sink nobody reads.
        Some(entry @ Pending::StreamBody { .. }) => {
            table.entries.insert(res.id, entry);
            Err(BridgeError::StateViolation(format!(
                "a second res arrived for request {}, which already has a head",
                res.id
            )))
        }
        None => {
            drop(table);
            discard_or_fatal(inner, "res", res.id)
        }
    }
}

fn on_chunk(inner: &Arc<Inner>, chunk: Chunk, bin: Vec<u8>) -> Result<(), BridgeError> {
    let mut table = inner
        .pending
        .lock()
        .expect("assistant pending table poisoned");
    let Some(entry) = table.entries.get_mut(&chunk.id) else {
        drop(table);
        return discard_or_fatal(inner, "chunk", chunk.id);
    };
    let Pending::StreamBody {
        tx,
        budget,
        next_seq,
    } = entry
    else {
        return Err(BridgeError::StateViolation(format!(
            "chunk for request {}, which has not been answered with a head",
            chunk.id
        )));
    };
    // §2: "seq starts at 0 and increases by one per chunk. A gap is fatal." A
    // receiver cannot tell a dropped chunk from a reordered one, and guessing
    // would let the stream and its durable log disagree undetectably.
    if chunk.seq != *next_seq {
        return Err(BridgeError::StateViolation(format!(
            "chunk {} out of order on request {}, expected {}",
            chunk.seq, chunk.id, next_seq
        )));
    }
    *next_seq += 1;

    // §7/§9: bounded on BOTH axes, and loud on overflow. The buffer is charged
    // here and released in `StreamResponse::next`, so a consumer that keeps up
    // never trips it. Bytes alone are not a bound — an item cap is what stops a
    // producer whose chunks are one byte each from costing a queue slot apiece.
    let held_bytes = budget.bytes.load(Ordering::SeqCst);
    let held_items = budget.items.load(Ordering::SeqCst);
    let overflow = if held_bytes + bin.len() > inner.stream_cap {
        Some(format!(
            "stream {} exceeded its {}-byte buffer",
            chunk.id, inner.stream_cap
        ))
    } else if held_items + 1 > inner.stream_item_cap {
        Some(format!(
            "stream {} exceeded its {}-chunk buffer",
            chunk.id, inner.stream_item_cap
        ))
    } else {
        None
    };
    if let Some(message) = overflow {
        let _ = tx.send(StreamEvent::End(Some(ErrorObject {
            code: "stream_overflow".into(),
            message: message.clone(),
        })));
        table.entries.remove(&chunk.id);
        drop(table);
        // `cancel`, not `end`: on an INBOUND stream this host is the receiver,
        // and `end` belongs to the responder. Cancelling is what §2 gives the
        // originator to stop a producer it can no longer keep up with.
        inner.cancel_outbound(chunk.id);
        tracing::warn!(target: "assistant", id = chunk.id, "bridge: {message}");
        return Ok(());
    }
    budget.bytes.fetch_add(bin.len(), Ordering::SeqCst);
    budget.items.fetch_add(1, Ordering::SeqCst);
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
            Err(BridgeError::StateViolation(format!(
                "end for request {}, which has not been answered with a head",
                end.id
            )))
        }
        None => {
            drop(table);
            discard_or_fatal(inner, "end", end.id)
        }
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
fn on_req(inner: &Arc<Inner>, req: Req) -> Result<(), BridgeError> {
    // §2a/§3: the sidecar allocates ODD ids, and a wrong-parity inbound request
    // is FATAL on both sides. Answering it, as this host used to, assumes the
    // peer's allocator is merely confused — but a diverged allocator crosses
    // responses, and an even id here is one this host may itself allocate, so
    // there is no reply that cannot be mistaken for the answer to something else.
    if req.id.is_multiple_of(2) {
        return Err(BridgeError::StateViolation(format!(
            "inbound request id {} is host-allocated; the sidecar's allocator has diverged",
            req.id
        )));
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
        Err(ProtocolError::UnknownVerb(verb)) => fail(
            inner,
            req.id,
            "unknown_verb",
            &format!("no verb '{verb}' on this bridge"),
        ),
        Err(err) => fail(inner, req.id, "forbidden", &err.to_string()),
        Ok(()) => route(inner, req),
    }
}

/// Dispatches an authorized inbound request to its host-side route.
fn route(inner: &Arc<Inner>, req: Req) -> Result<(), BridgeError> {
    match req.verb.as_str() {
        VERB_PROVIDER_FETCH => {
            // Read here, per request: a provider installed since this bridge
            // handshook serves the very next `provider.fetch` (ADR-0017's
            // trusted settings path), and one cleared since then stops serving
            // just as promptly.
            let Some(gateway) = inner.provider.current() else {
                return fail(
                    inner,
                    req.id,
                    "unimplemented",
                    "no local-provider gateway is configured on this bridge",
                );
            };
            let id = req.id;
            // Registered BEFORE the spawn, so a `cancel` that races the first
            // frame still finds something to cancel — and refused, WITHOUT
            // touching what is already there, if this id is still in flight
            // (§2a). The ticket is what the handler retires; a bare id would let
            // a late handler retire a registration that is no longer its own.
            let token = CancellationToken::new();
            let ticket = match inner.track_inbound(id, token.clone()) {
                Ok(ticket) => ticket,
                Err(refusal) => {
                    tracing::warn!(target: "assistant", id, code = refusal.code(), "bridge: inbound request refused");
                    return fail(
                        inner,
                        id,
                        refusal.code(),
                        match refusal {
                            InboundRefusal::DuplicateId => {
                                "an inbound request with this id is still in flight"
                            }
                            InboundRefusal::TooMany => {
                                "too many inbound requests are in flight on this connection"
                            }
                        },
                    );
                }
            };
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
                inner.finish_inbound(id, ticket);
            });
            Ok(())
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

    // On the BODY lane: this `end` terminates the chunks above it and must never
    // be written before one of them.
    let _ = inner
        .send_after_body(&Envelope::End(End {
            id,
            ok: error.is_none(),
            error,
        }))
        .await;
}

fn fail(inner: &Arc<Inner>, id: u64, code: &str, message: &str) -> Result<(), BridgeError> {
    inner.try_send(&Envelope::Res(Res {
        id,
        ok: false,
        payload: None,
        error: Some(ErrorObject {
            code: code.to_string(),
            message: message.to_string(),
        }),
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use onecad_assistant_protocol::{decode_frame, encode_frame};
    use serde_json::json;
    use tokio::io::DuplexStream;
    use tokio::time::timeout;

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
            agentkit_contract_version: SUPPORTED_AGENTKIT_CONTRACTS[0].into(),
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

    /// §2: the contract version gates the DTOs the same way `protocolVersion`
    /// gates the framing, and both are settled before any other frame.
    ///
    /// It was advertised and then ignored, so a sidecar built against DTOs this
    /// host does not understand could activate a connection and fail later, one
    /// request at a time, in a way no caller could distinguish from a real error.
    #[tokio::test]
    async fn handshake_rejects_an_unsupported_agentkit_contract() {
        let (host_io, peer_io) = tokio::io::duplex(64 * 1024);
        let mut peer = ScriptedPeer {
            io: peer_io,
            buf: BytesMut::new(),
        };
        peer.send(&Envelope::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            host_version: "0.1.0".into(),
            agentkit_contract_version: "0.4.0".into(),
            pid: 4242,
            session_nonce: "nonce".into(),
        }))
        .await;
        let (rx, tx) = tokio::io::split(host_io);
        let err = AssistantBridge::connect(rx, tx, BridgeOptions::default())
            .await
            .expect_err("an unservable contract must be refused");
        assert!(matches!(err, BridgeError::Handshake(_)), "{err}");

        match peer.recv().await {
            Some(Envelope::Reject(reject)) => {
                assert!(
                    reject.reason.contains("AgentKit contract version 0.4.0"),
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

    /// §2a: a wrong-parity inbound `req` is FATAL, not answered.
    ///
    /// This used to be a refusal, and the TypeScript peer has always torn down.
    /// The two behaviours are not both defensible: an even id here is one this
    /// host may itself allocate, so a reply to it can be mistaken for the answer
    /// to a request this host is waiting on.
    #[tokio::test]
    async fn an_inbound_request_with_a_host_allocated_id_is_fatal() {
        let (bridge, mut peer) = connected(BridgeOptions::default()).await;
        peer.send(&Envelope::Req(Req {
            id: 2, // even: the host's own id space (§3)
            principal: Principal::Host,
            verb: "provider.fetch".into(),
            payload: Value::Null,
        }))
        .await;
        bridge.closed().await;
        assert!(bridge.is_closed());
        assert!(
            peer.recv().await.is_none(),
            "a diverged allocator gets no reply at all"
        );
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

    // ─────────────────────────────────────────────────────────────────────
    // §2a: the shared adverse-wire fixture corpus
    // ─────────────────────────────────────────────────────────────────────
    //
    // The cases live in `docs/assistant/wire-fixtures/` and are executed by BOTH
    // implementations — this runner and `assistant-host/tests/wireFixtures.test.ts`.
    // That is the point: F09 was not one bug but two peers that had each decided
    // something reasonable and different, and no test either of them owned could
    // have noticed. A fixture the two run against their own real dispatchers is
    // the only artefact that can.
    //
    // Ids in a fixture are symbolic. `self:N` is the Nth id in the RECEIVING
    // peer's own space (even here, odd in the sidecar) and `peer:N` is the Nth in
    // the sender's. One corpus therefore describes both directions without a
    // single hard-coded integer to keep in sync.

    /// The id a `self:N` / `peer:N` token names, seen from this host.
    fn resolve_token(token: &str) -> u64 {
        let (space, index) = token
            .split_once(':')
            .unwrap_or_else(|| panic!("malformed id token {token:?}"));
        let index: u64 = index.parse().expect("id token index");
        match space {
            // The host allocates EVEN ids; the sidecar the odd ones (§3).
            "self" => index * 2,
            "peer" => index * 2 + 1,
            other => panic!("unknown id space {other:?}"),
        }
    }

    /// One fixture frame → the bytes to put on the wire.
    fn fixture_frame(spec: &Value) -> Vec<u8> {
        if let Some(raw) = spec.get("json").and_then(Value::as_str) {
            // A literal: duplicate keys and out-of-range ids cannot be expressed
            // as structured JSON, so those cases name their bytes exactly.
            return encode_frame(raw.as_bytes(), &[]).expect("encode literal frame");
        }
        let mut object = spec.as_object().expect("fixture frame object").clone();
        let bin = match object.remove("bin") {
            Some(Value::String(encoded)) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .expect("fixture bin is base64"),
            _ => Vec::new(),
        };
        if let Some(Value::String(token)) = object.get("id").cloned() {
            object.insert("id".into(), json!(resolve_token(&token)));
        }
        // `@inflight` is the corpus's placeholder for "whatever verb this peer
        // serves with a handler that does not finish". Each side substitutes its
        // own, because the two do not serve the same verbs (§5).
        if object.get("verb").and_then(Value::as_str) == Some("@inflight") {
            object.insert("verb".into(), json!(VERB_PROVIDER_FETCH));
            object.insert("payload".into(), inflight_payload());
        }
        let json = serde_json::to_vec(&Value::Object(object)).expect("fixture json");
        encode_frame(&json, &bin).expect("encode fixture frame")
    }

    /// The payload the `@inflight` placeholder carries on this side.
    fn inflight_payload() -> Value {
        json!({ "providerId": "p", "method": "GET", "path": "/models", "headers": {} })
    }

    /// A provider whose endpoint accepts the connection and never answers, so a
    /// `provider.fetch` stays genuinely in flight for the duplicate-id case.
    async fn hanging_provider() -> (Arc<ProviderRegistry>, tokio::net::TcpListener) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let gateway = crate::assistant::provider_gateway::ProviderGateway::new(
            vec![crate::assistant::provider_gateway::ProviderConfig {
                id: "p".into(),
                base_url: format!("http://127.0.0.1:{port}/v1"),
                model: "m".into(),
                api_key: None,
            }],
            crate::assistant::provider_gateway::GatewayLimits::default(),
        )
        .expect("gateway");
        let registry = Arc::new(ProviderRegistry::new());
        registry.install(Some(Arc::new(gateway)));
        (registry, listener)
    }

    async fn run_wire_fixture(case: &Value) {
        let name = case["name"].as_str().expect("fixture name");
        let setup = case["setup"].as_str().expect("fixture setup");
        let expect = case["expect"].as_str().expect("fixture expectation");

        let mut options = BridgeOptions::default();
        let mut listener = None;
        if setup == "inboundInFlight" {
            let (registry, socket) = hanging_provider().await;
            options.provider_registry = registry;
            listener = Some(socket);
        }
        let (bridge, mut peer) = connected(options).await;
        let bridge = Arc::new(bridge);

        // A task that issues one streaming request and then HOLDS the response,
        // so the per-request state machine has something to be in a state about.
        let mut streaming: Option<JoinHandle<()>> = None;
        match setup {
            "none" => {}
            "outboundStream" => {
                let held = bridge.clone();
                streaming = Some(tokio::spawn(async move {
                    let stream = held
                        .request_stream("agentkit.fetch", Principal::Ui, json!({}))
                        .await;
                    // Held, never read: releasing the budget is not what these
                    // cases are about, and dropping it would send `cancel`.
                    let _held = stream;
                    std::future::pending::<()>().await;
                }));
                match timeout(Duration::from_secs(2), peer.recv()).await {
                    Ok(Some(Envelope::Req(req))) => assert_eq!(req.id, 0, "{name}: first host id"),
                    other => panic!("{name}: expected the host's req, got {other:?}"),
                }
            }
            "inboundInFlight" => {
                peer.send(&Envelope::Req(Req {
                    id: resolve_token("peer:0"),
                    principal: Principal::Host,
                    verb: "provider.fetch".into(),
                    payload: inflight_payload(),
                }))
                .await;
                // The gateway connecting is the proof the handler is running.
                let socket = listener.as_ref().expect("listener");
                let (held, _) = timeout(Duration::from_secs(5), socket.accept())
                    .await
                    .expect("the provider fetch must reach the endpoint")
                    .expect("accept");
                std::mem::forget(held); // held open: the request must not settle
            }
            other => panic!("{name}: unknown setup {other:?}"),
        }

        for spec in case["deliver"].as_array().expect("deliver") {
            peer.io
                .write_all(&fixture_frame(spec))
                .await
                .expect("write fixture frame");
            peer.io.flush().await.expect("flush");
        }

        // The liveness probe. A connection that is still open answers it; one
        // that tore down never will, and that difference is the whole assertion.
        let probe = resolve_token("peer:9");
        peer.send(&Envelope::Ping(Ping { id: probe })).await;

        match expect {
            "close" => {
                timeout(Duration::from_secs(2), bridge.closed())
                    .await
                    .unwrap_or_else(|_| panic!("{name}: the connection must tear down"));
                while let Ok(Some(frame)) = timeout(Duration::from_secs(1), peer.recv()).await {
                    assert!(
                        !matches!(frame, Envelope::Pong(Pong { id }) if id == probe),
                        "{name}: a torn-down connection must not answer the probe"
                    );
                }
            }
            "refuse" => {
                let refusal = &case["refusal"];
                let want_id = resolve_token(refusal["id"].as_str().expect("refusal id"));
                let want_code = refusal["code"].as_str().expect("refusal code");
                let mut refused = false;
                let mut ponged = false;
                while !(refused && ponged) {
                    match timeout(Duration::from_secs(2), peer.recv()).await {
                        Ok(Some(Envelope::Res(res))) if res.id == want_id && !res.ok => {
                            assert_eq!(
                                res.error.expect("error body").code,
                                want_code,
                                "{name}: refusal code"
                            );
                            refused = true;
                        }
                        Ok(Some(Envelope::Pong(Pong { id }))) if id == probe => ponged = true,
                        Ok(Some(_)) => {}
                        other => panic!("{name}: expected a refusal and a pong, got {other:?}"),
                    }
                }
                assert!(!bridge.is_closed(), "{name}: a refusal is not a teardown");
            }
            "discard" => {
                loop {
                    match timeout(Duration::from_secs(2), peer.recv()).await {
                        Ok(Some(Envelope::Pong(Pong { id }))) if id == probe => break,
                        Ok(Some(_)) => {}
                        other => panic!("{name}: the probe must be answered, got {other:?}"),
                    }
                }
                assert!(
                    !bridge.is_closed(),
                    "{name}: a discarded frame is not fatal"
                );
            }
            other => panic!("{name}: unknown expectation {other:?}"),
        }

        if let Some(handle) = streaming {
            handle.abort();
        }
        bridge.close("fixture over");
    }

    #[tokio::test]
    async fn every_shared_wire_fixture_produces_the_contracted_outcome() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../docs/assistant/wire-fixtures");
        let mut paths: Vec<_> = std::fs::read_dir(&dir)
            .expect("the shared fixture corpus must exist")
            .map(|entry| entry.expect("dir entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();
        assert!(
            paths.len() >= 14,
            "the corpus lost cases: found {}",
            paths.len()
        );
        for path in paths {
            let text = std::fs::read_to_string(&path).expect("read fixture");
            let case: Value = serde_json::from_str(&text).expect("parse fixture");
            run_wire_fixture(&case).await;
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // §9 budgets and the connection-failure authority
    // ─────────────────────────────────────────────────────────────────────

    /// Wires a bridge whose read and write halves are SEPARATE pipes, so one can
    /// be broken while the other stays open — the half-open peer F05 is about.
    async fn connected_split() -> (AssistantBridge, ScriptedPeer, DuplexStream) {
        let (host_rx, peer_tx) = tokio::io::duplex(64 * 1024);
        let (host_tx, peer_rx) = tokio::io::duplex(64 * 1024);
        let mut peer = ScriptedPeer {
            io: peer_tx,
            buf: BytesMut::new(),
        };
        peer.send(&hello(PROTOCOL_VERSION)).await;
        let bridge = AssistantBridge::connect(host_rx, host_tx, BridgeOptions::default())
            .await
            .expect("handshake");
        (bridge, peer, peer_rx)
    }

    /// IO-01. A writer that fails while the reader stays open must still retire
    /// the whole connection.
    ///
    /// The old writer returned quietly and assumed the reader would notice. That
    /// holds only for a peer that closes both halves: a child keeping stdout open
    /// with a broken stdin left every pending request waiting forever and the
    /// supervisor reading `Ready` off a connection that could not carry a frame.
    #[tokio::test]
    async fn a_writer_failure_alone_retires_the_connection() {
        let (bridge, _peer, peer_rx) = connected_split().await;
        // Only the host's WRITE side is broken. Its read side is still open and
        // the scripted peer is still holding it.
        drop(peer_rx);

        let err = bridge
            .request("shutdown", Principal::Ui, Value::Null)
            .await
            .expect_err("a request cannot succeed over a dead writer");
        assert!(
            matches!(err, BridgeError::Closed(_) | BridgeError::Timeout { .. }),
            "{err}"
        );
        timeout(Duration::from_secs(2), bridge.closed())
            .await
            .expect("the connection must retire when either half fails");
        assert!(bridge.is_closed());
        assert_eq!(bridge.inner.in_flight(), (0, 0), "no slot is left pinned");
    }

    /// IO-02. A control frame that cannot be delivered fails the connection
    /// explicitly rather than being logged and dropped.
    ///
    /// A dropped control frame is not a harmless one: it is the `cancel` that
    /// stops a runaway producer or the `res` that settles a waiting caller.
    #[tokio::test]
    async fn an_undeliverable_control_frame_fails_the_connection() {
        // A pipe small enough that the writer parks almost at once, and a peer
        // that never reads what it asked for.
        let (host_io, peer_io) = tokio::io::duplex(512);
        let mut peer = ScriptedPeer {
            io: peer_io,
            buf: BytesMut::new(),
        };
        peer.send(&hello(PROTOCOL_VERSION)).await;
        let (rx, tx) = tokio::io::split(host_io);
        let bridge = AssistantBridge::connect(rx, tx, BridgeOptions::default())
            .await
            .expect("handshake");

        // Every ping demands a pong. The peer never drains, so the reserved
        // control queue fills and the next pong has nowhere to go.
        //
        // Spawned, not looped inline: once the host retires, the peer's own
        // writes block against a buffer nobody is reading, and the assertion
        // below is about the host's behaviour rather than the flood's.
        let flood = tokio::spawn(async move {
            for id in 0..5_000u64 {
                peer.send(&Envelope::Ping(Ping { id: id * 2 + 1 })).await;
            }
        });
        let outcome = timeout(Duration::from_secs(10), bridge.closed()).await;
        flood.abort();
        outcome.expect("an undeliverable control frame must fail the connection");
    }

    /// PRO-01. A second head is fatal, and the ORIGINAL consumer is told.
    ///
    /// Tearing the connection down is only half the requirement: the reader that
    /// already owns the body must settle, or the caller waits forever on a
    /// connection that no longer exists. The fixture corpus asserts the
    /// teardown; this asserts what the consumer sees.
    #[tokio::test]
    async fn a_second_head_settles_the_original_consumer() {
        let (bridge, mut peer) = connected(BridgeOptions::default()).await;
        let request = bridge.request_stream("agentkit.fetch", Principal::Ui, json!({}));
        let serve = async {
            let Some(Envelope::Req(req)) = peer.recv().await else {
                panic!("expected a req");
            };
            for _ in 0..2 {
                peer.send(&Envelope::Res(Res {
                    id: req.id,
                    ok: true,
                    payload: Some(json!({ "status": 200 })),
                    error: None,
                }))
                .await;
            }
        };
        let (stream, ()) = tokio::join!(request, serve);
        let mut stream = stream.expect("the first head is delivered");

        match timeout(Duration::from_secs(2), stream.next()).await {
            Ok(Some(StreamEvent::End(Some(error)))) => {
                assert_eq!(
                    error.code, "bridge_closed",
                    "the original body is failed, never abandoned"
                );
            }
            other => panic!("the original consumer must settle, got {other:?}"),
        }
        assert!(bridge.is_closed());
    }

    /// §2: "a streaming request is answered by a `res` carrying the head, then
    /// zero or more `chunk`s, then exactly one `end`" — in that order, always.
    ///
    /// The writer drains control before body so a `cancel` cannot queue behind a
    /// flood. `end` used to ride the control lane, so for any body that finished
    /// quickly the terminator overtook its own chunks and the wire carried
    /// `res → end → chunk`. It was timing-dependent, which is the worst kind of
    /// wrong: a fixture with an extra EOF round trip, or a sleep between events,
    /// hid it completely.
    ///
    /// **This test contains no sleep.** The provider answers with
    /// `content-length` and then HOLDS the socket open, so the body completes
    /// immediately with no EOF to slow it down — the exact shape that exposed
    /// the inversion.
    #[tokio::test]
    async fn a_stream_end_never_overtakes_its_own_chunks() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut scratch = [0u8; 2048];
            let _ = socket.read(&mut scratch).await;
            let body = r#"{"data":[{"id":"m"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.expect("write");
            socket.flush().await.expect("flush");
            // Held open: the client must finish on `content-length` alone.
            std::future::pending::<()>().await;
        });

        let gateway = crate::assistant::provider_gateway::ProviderGateway::new(
            vec![crate::assistant::provider_gateway::ProviderConfig {
                id: "p".into(),
                base_url: format!("http://127.0.0.1:{port}/v1"),
                model: "m".into(),
                api_key: None,
            }],
            crate::assistant::provider_gateway::GatewayLimits::default(),
        )
        .expect("gateway");
        let registry = Arc::new(ProviderRegistry::new());
        registry.install(Some(Arc::new(gateway)));
        let options = BridgeOptions {
            provider_registry: registry,
            ..BridgeOptions::default()
        };
        let (bridge, mut peer) = connected(options).await;

        peer.send(&Envelope::Req(Req {
            id: 1,
            principal: Principal::Host,
            verb: "provider.fetch".into(),
            payload: inflight_payload(),
        }))
        .await;

        // The frames, in the order they reached the wire.
        let mut order: Vec<&'static str> = Vec::new();
        let mut seqs: Vec<u64> = Vec::new();
        loop {
            match timeout(Duration::from_secs(10), peer.recv()).await {
                Ok(Some(Envelope::Res(res))) => {
                    assert!(res.ok, "the provider answered 200: {res:?}");
                    order.push("res");
                }
                Ok(Some(Envelope::Chunk(chunk))) => {
                    order.push("chunk");
                    seqs.push(chunk.seq);
                }
                Ok(Some(Envelope::End(end))) => {
                    assert!(end.ok, "{end:?}");
                    order.push("end");
                    break;
                }
                Ok(Some(_)) => {}
                other => panic!("expected the stream's frames, got {other:?}"),
            }
        }
        assert_eq!(order.first().copied(), Some("res"), "the head comes first");
        assert_eq!(order.last().copied(), Some("end"), "and the end comes last");
        assert!(
            order[1..order.len() - 1].iter().all(|tag| *tag == "chunk"),
            "nothing may come between the head and the terminator: {order:?}"
        );
        assert!(
            !seqs.is_empty(),
            "the provider sent a body, so at least one chunk must precede the end"
        );
        assert_eq!(
            seqs,
            (0..seqs.len() as u64).collect::<Vec<_>>(),
            "chunk seqs are contiguous from zero"
        );

        server.abort();
        bridge.close("test over");
    }

    /// F05. An over-cap request this host generated itself is refused at its own
    /// call site, not by poisoning a healthy connection.
    ///
    /// The writer now fails the whole connection on an encode error, which makes
    /// an avoidable one expensive: without this check a caller handing over two
    /// megabytes of payload would take down a child that was working perfectly.
    #[tokio::test]
    async fn an_over_cap_request_is_refused_without_touching_the_connection() {
        let (bridge, _peer) = connected(BridgeOptions::default()).await;
        let payload = json!({ "blob": "x".repeat(2 * 1024 * 1024) });
        let err = bridge
            .request("agentkit.fetch", Principal::Ui, payload)
            .await
            .expect_err("an over-cap envelope cannot be sent");
        assert!(
            matches!(
                err,
                BridgeError::Protocol(ProtocolError::TooLarge { what: "json", .. })
            ),
            "{err}"
        );
        assert!(!bridge.is_closed(), "the connection is untouched");
        assert_eq!(bridge.inner.in_flight(), (0, 0), "and so is the table");
    }

    /// IO-03. A head that never arrives expires and gives its slot back, while
    /// the connection itself stays healthy — a wedged route is not a wedged
    /// process, and the supervisor's ping would have kept saying so.
    #[tokio::test]
    async fn a_head_that_never_arrives_expires_and_reclaims_its_slot() {
        let options = BridgeOptions {
            request_head_deadline: Duration::from_millis(50),
            ..BridgeOptions::default()
        };
        let (bridge, mut peer) = connected(options).await;

        let err = bridge
            .request("shutdown", Principal::Ui, Value::Null)
            .await
            .expect_err("an unanswered request must expire");
        assert!(matches!(err, BridgeError::Timeout { .. }), "{err}");
        assert_eq!(
            bridge.inner.in_flight(),
            (0, 0),
            "the correlation slot is reclaimed without waiting for the peer"
        );
        assert!(
            !bridge.is_closed(),
            "one dead route is not a dead connection"
        );

        // The peer is told, and the connection still answers.
        let mut cancelled = false;
        let mut ponged = false;
        peer.send(&Envelope::Ping(Ping { id: 9 })).await;
        while !(cancelled && ponged) {
            match timeout(Duration::from_secs(2), peer.recv()).await {
                Ok(Some(Envelope::Cancel(Cancel { id: 0 }))) => cancelled = true,
                Ok(Some(Envelope::Pong(Pong { id: 9 }))) => ponged = true,
                Ok(Some(_)) => {}
                other => panic!("expected cancel and pong, got {other:?}"),
            }
        }
    }

    /// §2a. Inbound cleanup is bound to the ticket issued at registration, never
    /// to the bare integer.
    ///
    /// This is the ownership transfer that made the Rust side's duplicate-id
    /// handling unsafe: the displaced handler's unconditional `finish_inbound`
    /// removed its REPLACEMENT's token, after which a `cancel` for that id
    /// reached nothing at all.
    #[tokio::test]
    async fn inbound_cleanup_is_bound_to_its_ticket() {
        let (bridge, _peer) = connected(BridgeOptions::default()).await;
        let inner = &bridge.inner;
        let first = CancellationToken::new();
        let ticket = inner.track_inbound(3, first.clone()).expect("registers");

        // While it is in flight, the id is refused and the original is untouched.
        assert_eq!(
            inner.track_inbound(3, CancellationToken::new()),
            Err(InboundRefusal::DuplicateId)
        );
        assert_eq!(inner.in_flight().1, 1);
        assert!(
            !first.is_cancelled(),
            "the in-flight handler is not disturbed"
        );

        // A stale ticket retires nothing.
        inner.finish_inbound(3, ticket + 1000);
        assert_eq!(inner.in_flight().1, 1);

        inner.finish_inbound(3, ticket);
        assert_eq!(inner.in_flight().1, 0);

        // And a LATE retirement by the first owner cannot remove the second.
        let second = inner
            .track_inbound(3, CancellationToken::new())
            .expect("re-registers");
        inner.finish_inbound(3, ticket);
        assert_eq!(inner.in_flight().1, 1, "the replacement keeps its slot");
        inner.finish_inbound(3, second);
        assert_eq!(inner.in_flight().1, 0);
    }

    /// MEM-02. A flood of tiny chunks trips the ITEM bound, which byte
    /// accounting alone cannot see.
    #[tokio::test]
    async fn a_flood_of_tiny_chunks_overflows_on_the_item_bound() {
        let options = BridgeOptions {
            max_stream_buffer_bytes: 8 * 1024 * 1024,
            max_stream_buffer_items: 4,
            ..BridgeOptions::default()
        };
        let (bridge, mut peer) = connected(options).await;
        let request = async {
            bridge
                .request_stream("agentkit.fetch", Principal::Ui, json!({}))
                .await
        };
        let serve = async {
            match peer.recv().await {
                Some(Envelope::Req(req)) => {
                    peer.send(&Envelope::Res(Res {
                        id: req.id,
                        ok: true,
                        payload: Some(json!({ "status": 200 })),
                        error: None,
                    }))
                    .await;
                    // Five one-byte chunks: 5 bytes against an 8 MiB byte budget,
                    // and one chunk past a four-item one.
                    for seq in 0..5u64 {
                        peer.send_with_bin(&Envelope::Chunk(Chunk { id: req.id, seq }), &[7])
                            .await;
                    }
                    req.id
                }
                other => panic!("expected a req, got {other:?}"),
            }
        };
        let (stream, id) = tokio::join!(request, serve);
        let mut stream = stream.expect("head");
        let mut delivered = 0usize;
        let error = loop {
            match stream.next().await {
                Some(StreamEvent::Chunk(bytes)) => delivered += bytes.len(),
                Some(StreamEvent::End(error)) => break error,
                None => panic!("the stream must terminate"),
            }
        };
        assert_eq!(
            error.expect("overflow is loud, never a silent drop").code,
            "stream_overflow"
        );
        assert!(delivered <= 4, "at most the item budget was ever queued");
        // §2: the producer is told to stop.
        loop {
            match timeout(Duration::from_secs(2), peer.recv()).await {
                Ok(Some(Envelope::Cancel(cancel))) if cancel.id == id => break,
                Ok(Some(_)) => {}
                other => panic!("expected a cancel, got {other:?}"),
            }
        }
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
