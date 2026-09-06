//! `onecad-worker-stub` — a real fake sidecar speaking OCW1 over stdio.
//!
//! It implements the handshake and the lifecycle verbs the protocol tests need
//! (Hello unsolicited, Shutdown, OpenSession/CloseSession, GetWorkerHead) and
//! replies to any unknown verb with a well-framed `PROTOCOL_ERROR` terminal
//! `resp` (SCHEMA §8 well-framed-illegal sub-case). IO is blocking — the wire
//! bytes are identical to a real worker's, and blocking IO keeps the chaos hooks
//! trivially deterministic.
//!
//! ## Lanes (kernel-hardening WP-H)
//!
//! The stub mirrors the real worker's thread shape, because the supervisor's
//! liveness rules are ABOUT that shape and a single-threaded fake cannot express
//! them: a READER thread parses frames and routes them, a KERNEL thread runs every
//! modelling verb, and a STATUS thread answers `GetWorkerHead` (SCHEMA §7.1) so a
//! long kernel job never delays the ping. The `GetWorkerHead` answer carries the
//! additive `inflight` + `hasRestoredBase` fields with the worker's field names
//! exactly, so a Rust wedge decision made against the stub is the decision it
//! would make against OCCT.
//!
//! `StubState` and stdout live behind ONE mutex, taken for the duration of a
//! handler. That is what keeps `seq` monotonic in the same order the bytes hit the
//! pipe (SCHEMA §2) across three writers — the real worker gets the same property
//! from its single `write_mu_`. The chaos sleeps happen OUTSIDE that lock, which is
//! the whole point: a hung kernel job must not be able to gag the status lane.
//!
//! Chaos hooks (env vars), for the client's crash/hang/garbage drills:
//! - `ONECAD_STUB_CRASH_ON=<verb>` — `abort()` when that verb arrives.
//! - `ONECAD_STUB_HANG_ON=<verb>`  — sleep forever on the KERNEL lane when that
//!   verb arrives. The process stays responsive (the status thread keeps answering
//!   `GetWorkerHead`, reporting the wedged job in `inflight`), so this is the
//!   WEDGED-OP drill: it must die by the §8 wall deadline, not by the ping rule.
//! - `ONECAD_STUB_IGNORE_DISCARD=1` — answer `DiscardPrepared` with
//!   `discarded: true` but KEEP the scratch. The control half of the dropped-prepare
//!   drill: it reproduces a worker with no Rust-side Drop guarantee, so the drill can
//!   show that a stranded scratch is observable at all before asserting there is none.
//! - `ONECAD_STUB_DEAF_ON=<verb>` — stop answering EVERYTHING when that verb
//!   arrives: the READER thread parks, so nothing is routed to either lane and the
//!   ping goes unanswered. The unresponsive-PROCESS drill, which the 5 s × 2 ping
//!   rule must still kill.
//! - `ONECAD_STUB_GARBAGE=1`       — emit one invalid-magic frame at startup
//!   (drives the client's `BadMagic` path), then continue.
//! - `ONECAD_STUB_CRASH_COUNTDOWN=<file>` — the R-WP11 convergence drill: the file
//!   holds a decimal counter; each `ExecutePlan` reads it, and while `> 0`
//!   decrements it (persisting across restarts) and `abort()`s **mid-plan** (after
//!   emitting the first `planStep`), so a fresh worker crashes N times then
//!   succeeds — the document must always converge to the last-valid snapshot.
//! - `ONECAD_STUB_CHUNKED_MESH=1` — `Tessellate` streams its MESH1 blob as a bulk
//!   chunk manifest + data frames (SCHEMA §5.2) instead of inlining it, exercising
//!   the client's chunk-assembly + credit path.
//! - `ONECAD_STUB_CHUNKED_MESH_GAP=<mode>` — with `ONECAD_STUB_CHUNKED_MESH=1`,
//!   corrupts the tiling of the streamed chunks (`gap` = leave a hole; `overlap` =
//!   overlap two chunks) so the client's StreamAcc gap-detection fires (F5).
//! - `ONECAD_STUB_EXIT_AFTER_HELLO=1` — exit 0 immediately after the unsolicited
//!   `hello` (connect-then-die), driving the supervisor's rapid-death restart cap
//!   (F2). No session/plan is ever served.
//! - `ONECAD_STUB_HELLO_DELAY_MS=<ms>` — sleep that long BEFORE the unsolicited
//!   `hello`, so the manager stays un-`Ready` for a known window: the cold-worker
//!   regen race (a `from: 0` plan dispatched before the handshake lands). Applied
//!   on every spawn, restarts included.
//! - `ONECAD_STUB_CRASH_ON_OP=<substr>` — `abort()` mid-plan when an op's `opId`
//!   contains `<substr>` (the F3 poison test: crash one specific plan's op so its
//!   crashing-op key poisons while a different plan still runs).
//! - `ONECAD_STUB_SLOW_ON=<verb>` + `ONECAD_STUB_SLOW_MS=<ms>` — a SLOW but LIVE
//!   op (WP-H): the verb takes `<ms>` on the kernel lane, streaming `progress`
//!   frames (SCHEMA §3.3) while it works, then replies normally. Distinct from
//!   `HANG_ON`, which never finishes.
//! - `ONECAD_STUB_PROGRESS_EVERY_MS=<ms>` — the `SLOW_ON` progress cadence
//!   (default 200 ms). **`0` means SILENT**: the op still takes its full time but
//!   emits no `progress` at all, so `sinceProgressMs` climbs with `ageMs` — the
//!   difference between an op the wedge rule must spare and one it must kill.
//! - `ONECAD_STUB_STALE_ON=<verb>` + `ONECAD_STUB_STALE_TIMES=<n>` — answer the
//!   first `<n>` requests for that verb with a `STALE_PREVIEW` error resp (SCHEMA
//!   §7.6 shape: `detail {requested, head}`), then behave normally. Drives Rust's
//!   re-read-the-head-once rule: `n = 1` must be invisible to the caller, `n = 2`
//!   must surface.
//!
//! Beyond the lifecycle verbs, the stub speaks a minimal `ExecutePlan`
//! (one `planStep` per op minting `body_<opId>`, terminal `PlanPrepared` echoing
//! the plan's opaque history-prefix token), `AcceptPrepared`/`DiscardPrepared`,
//! `ResetSession`, and `Tessellate` (a header-only valid MESH1 blob) — enough to
//! drive the real regen/mesh path end-to-end without OCCT.
//!
//! Fencing (D4/D5): the stub mirrors the real worker — `ExecutePlan` fences on
//! `workerEpoch` + `expectedBaseHash` ONLY (same PROTOCOL_ERROR shapes), never on
//! `documentRevision`; the head ADOPTS the plan's `documentRevision` + echoed
//! `historyPrefixHash` at `AcceptPrepared`. Per D5 a from-0 plan (no `baseCheckpoint`
//! AND `expectedBaseHash` == the empty anchor) is ALWAYS base-valid — the head-hash
//! comparison is skipped so sequential replay-from-0 regens keep preparing after the
//! head token advances (the epoch fence still applies). The one divergence from the
//! real worker (which requires `OpenSession`): when a plan arrives before any
//! `OpenSession` (the chaos drills), the stub adopts that first plan's epoch + base
//! hash as the fencing baseline and fences every plan thereafter. It also mirrors the
//! ONE-SCRATCH rule (§7.2): a second `ExecutePlan` with a different `jobId` while one
//! is prepared is a `PROTOCOL_ERROR`, and a re-sent same `jobId` re-returns the cached
//! `PlanPrepared` — without that, every drill about ordering a `DiscardPrepared` ahead
//! of the next plan is vacuously green.
//!
//! Logs go to stderr only; stdout carries frames exclusively (SCHEMA §1).

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};

use onecad_protocol::framing::{read_frame_blocking, write_frame_blocking, MAGIC_BYTES};
use onecad_protocol::messages::{
    BinSection, ChunkFrame, ChunkKind, CloseSessionResult, ErrorCode, ErrorObject, EventFrame,
    Frame, HelloFrame, HelloLimits, HelloResult, InflightResult, OcctInfo, OpenSessionArgs,
    OpenSessionResult, ProgressFrame, ReqFrame, RespFrame, ShutdownResult, Stamp, WorkerHeadBrief,
    WorkerHeadResult, PROTOCOL_VERSION,
};
use onecad_protocol::ProtocolError;

/// The SHA-256 of zero bytes — the empty-prefix `historyPrefixHash` anchor (the
/// base of a replay-from-0 plan). Mirrors the real worker's `kEmptyPrefixHash` and
/// onecad-core `HistoryPrefixHash::empty()`.
const EMPTY_PREFIX_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// One prepared-but-not-published scratch job (D4: carries the head token it would
/// adopt on accept + the plan's advisory documentRevision).
struct Prepared {
    job_id: u64,
    prepared_snapshot_id: u64,
    /// The `historyPrefixHash` this job would adopt as the head on accept.
    history_prefix_hash: String,
    /// The plan's Rust-owned documentRevision, ADOPTED as the head on accept (D4).
    plan_document_revision: u64,
    /// The terminal `PlanPrepared` result, cached for the §7.2 idempotency rule:
    /// a re-sent SAME `jobId` while prepared re-returns this verbatim.
    result: Value,
}

/// One stub sketch on the solver lane (SCHEMA §7.4): point positions + a deterministic
/// dof/state derived from `2·points − constraints`.
#[derive(Default, Clone)]
struct StubSketch {
    revision: u64,
    /// Point wire id → `[x, y]`.
    points: BTreeMap<String, [f64; 2]>,
    point_count: usize,
    constraint_count: usize,
}

impl StubSketch {
    fn dof(&self) -> i64 {
        (2 * self.point_count as i64 - self.constraint_count as i64).max(0)
    }
    fn state(&self) -> &'static str {
        if self.dof() == 0 {
            "FullyConstrained"
        } else {
            "UnderConstrained"
        }
    }
}

/// One in-flight drag gesture (SCHEMA §7.4). `max_seq` drives latest-wins: a `seq`
/// not newer than `max_seq` resolves `superseded`.
struct StubGesture {
    sketch_id: String,
    drag_point: String,
    max_seq: u64,
    baseline: BTreeMap<String, [f64; 2]>,
}

/// Mutable stub state across the request loop.
struct StubState {
    /// Worker output sequence number (SCHEMA §2: monotonic across every frame).
    seq: u64,
    session_open: bool,
    document_revision: u64,
    worker_epoch: u64,
    snapshot_id: u64,
    /// The head `historyPrefixHash` (fencing token; adopted from a plan's echo on
    /// accept). Mirrors the real worker's session head.
    history_prefix_hash: String,
    /// Whether the fencing baseline (epoch + head hash) has been established — set
    /// by OpenSession, or lazily by the first ExecutePlan when no session was
    /// opened (the chaos drills drive ExecutePlan directly, without OpenSession).
    fencing_baseline_set: bool,
    /// Prepared-but-not-published scratch jobs (D4-aware).
    prepared: Vec<Prepared>,
    /// Monotonic bulk-stream id allocator (SCHEMA §2 `streamId`).
    stream_id: u64,
    /// Solver-lane sketches by id (SCHEMA §7.4).
    sketches: BTreeMap<String, StubSketch>,
    /// In-flight drag gestures by gestureId (SCHEMA §7.4).
    gestures: BTreeMap<u64, StubGesture>,
}

impl StubState {
    fn new() -> Self {
        StubState {
            seq: 0,
            session_open: false,
            document_revision: 0,
            worker_epoch: 0,
            snapshot_id: 0,
            history_prefix_hash: EMPTY_PREFIX_HASH.to_string(),
            fencing_baseline_set: false,
            prepared: Vec::new(),
            stream_id: 700,
            sketches: BTreeMap::new(),
            gestures: BTreeMap::new(),
        }
    }

    /// Allocate the next output `seq`.
    fn next_seq(&mut self) -> u64 {
        let s = self.seq;
        self.seq += 1;
        s
    }

    fn stamp(&mut self) -> Stamp {
        self.stamp_job(None)
    }

    fn stamp_job(&mut self, job_id: Option<u64>) -> Stamp {
        Stamp {
            document_revision: self.document_revision,
            worker_epoch: self.worker_epoch,
            snapshot_id: self.snapshot_id,
            job_id,
            seq: self.next_seq(),
        }
    }
}

/// `StubState` + stdout behind ONE mutex (see the lane note in the module docs):
/// every writer stamps its `seq` and writes its bytes while holding it, so the
/// three lanes cannot produce a `seq` order that differs from the byte order.
struct StubCore {
    out: std::io::Stdout,
    state: StubState,
}

/// The kernel-lane job in flight, mirroring the worker's `Dispatcher::Inflight`
/// FIELD FOR FIELD (SCHEMA §7.1) — a Rust wedge decision taken against the stub
/// must be the decision it would take against the real worker.
struct InflightRec {
    verb: String,
    id: u64,
    job_id: Option<u64>,
    started: Instant,
    /// The last NON-TERMINAL frame this job emitted; == `started` until it emits
    /// one, which is what makes `sinceProgressMs == ageMs` for a silent op.
    last_progress: Instant,
}

/// Shared handle to the in-flight record. Deliberately NOT inside [`StubCore`]:
/// the status thread must be able to read it while the kernel thread is parked in
/// a chaos sleep holding nothing.
type InflightCell = Arc<Mutex<Option<InflightRec>>>;

fn main() {
    let code = run();
    std::process::exit(code);
}

fn run() -> i32 {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let core = Arc::new(Mutex::new(StubCore {
        out: std::io::stdout(),
        state: StubState::new(),
    }));
    let inflight: InflightCell = Arc::new(Mutex::new(None));

    // Chaos: emit one invalid-magic frame before anything else.
    if env_flag("ONECAD_STUB_GARBAGE") {
        let mut guard = core.lock().unwrap();
        if let Err(err) = emit_garbage(&mut guard.out) {
            eprintln!("stub: failed to emit garbage: {err}");
            return 1;
        }
    }

    // Chaos: stay silent for a known window so the parent's manager is spawned but
    // NOT yet `Ready` (the cold-worker regen race).
    if let Some(delay) = env_millis("ONECAD_STUB_HELLO_DELAY_MS") {
        eprintln!("stub: HELLO_DELAY_MS -> sleeping {delay:?} before hello");
        std::thread::sleep(delay);
    }

    // Unsolicited hello (SCHEMA §6): seq 0, written before any lane starts.
    {
        let mut guard = core.lock().unwrap();
        let hello_seq = guard.state.next_seq();
        debug_assert_eq!(hello_seq, 0);
        if let Err(err) = write_hello(&mut guard.out, hello_seq) {
            eprintln!("stub: failed to write hello: {err}");
            return 1;
        }
    }

    // Chaos: connect-then-die immediately after the hello (F2 rapid-death cap).
    if env_flag("ONECAD_STUB_EXIT_AFTER_HELLO") {
        eprintln!("stub: EXIT_AFTER_HELLO -> exit 0 right after hello");
        return 0;
    }

    let (kernel_tx, kernel_rx) = channel::<ReqFrame>();
    let (status_tx, status_rx) = channel::<ReqFrame>();
    let kernel = std::thread::spawn({
        let core = core.clone();
        let inflight = inflight.clone();
        move || kernel_loop(&core, &inflight, &kernel_rx)
    });
    let status = std::thread::spawn({
        let core = core.clone();
        let inflight = inflight.clone();
        move || status_loop(&core, &inflight, &status_rx)
    });

    let exit = reader_loop(&mut reader, &kernel_tx, &status_tx);

    // Drop the senders so both lanes drain and return, then join: a frame already
    // queued must still be answered before the process goes away.
    drop(kernel_tx);
    drop(status_tx);
    let _ = kernel.join();
    let _ = status.join();
    exit
}

/// Parses frames and ROUTES them: `GetWorkerHead` to the status lane, every other
/// request to the kernel lane. Nothing is executed here, which is what lets the
/// status lane answer while a kernel job runs.
fn reader_loop<R: std::io::Read>(
    reader: &mut R,
    kernel: &Sender<ReqFrame>,
    status: &Sender<ReqFrame>,
) -> i32 {
    loop {
        let raw = match read_frame_blocking(reader) {
            Ok(Some(raw)) => raw,
            Ok(None) => {
                eprintln!("stub: stdin closed; exiting");
                return 0;
            }
            Err(ProtocolError::ConnectionLost(reason)) => {
                eprintln!("stub: connection lost ({reason}); exiting");
                return 0;
            }
            Err(err) => {
                eprintln!("stub: fatal frame error: {err}");
                return 1;
            }
        };

        let frame = match Frame::from_json_slice(&raw.json) {
            Ok(frame) => frame,
            Err(err) => {
                // Malformed envelope: framing sub-case of PROTOCOL_ERROR, no id
                // to reply against -> tear down (SCHEMA §8).
                eprintln!("stub: malformed envelope: {err}");
                return 1;
            }
        };

        match frame {
            Frame::Req(req) => {
                // Chaos: an UNRESPONSIVE PROCESS. Parking the reader means nothing
                // reaches either lane — not even the liveness ping — which is the
                // one condition the 5 s x 2 ping rule is still there to catch.
                if env_matches("ONECAD_STUB_DEAF_ON", &req.verb) {
                    eprintln!("stub: DEAF_ON {} -> answering nothing, ever", req.verb);
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                    }
                }
                let lane = if req.verb == "GetWorkerHead" {
                    status
                } else {
                    kernel
                };
                if lane.send(req).is_err() {
                    eprintln!("stub: lane gone; exiting");
                    return 0;
                }
            }
            Frame::Cancel(c) => {
                // The stub answers every request on the lane that owns it, so by the
                // time a cancel could matter the terminal is already out (SCHEMA §3.5).
                eprintln!("stub: cancel for id {} (no-op)", c.id);
            }
            Frame::Credit(_) => {
                // The stub emits no bulk streams; credit is a no-op.
            }
            other => {
                eprintln!("stub: ignoring unexpected driver frame: {other:?}");
            }
        }
    }
}

/// The modelling lane. Chaos sleeps run BEFORE the core lock is taken, so a hung
/// or slow job never gags the status lane — the property the wedged-op drills
/// exist to measure.
fn kernel_loop(core: &Mutex<StubCore>, inflight: &InflightCell, rx: &Receiver<ReqFrame>) {
    while let Ok(req) = rx.recv() {
        {
            let now = Instant::now();
            *inflight.lock().unwrap() = Some(InflightRec {
                verb: req.verb.clone(),
                id: req.id,
                // Numbers only (SCHEMA §7.1): a verb without a numeric `jobId`
                // reports the key as ABSENT rather than as job zero.
                job_id: req.args.get("jobId").and_then(Value::as_u64),
                started: now,
                last_progress: now,
            });
        }
        let exit = run_kernel_job(core, inflight, req);
        *inflight.lock().unwrap() = None;
        if let Some(code) = exit {
            // The terminal frame was already written and flushed by the handler.
            std::process::exit(code);
        }
    }
}

/// One kernel job: the chaos hooks, then the handler.
fn run_kernel_job(core: &Mutex<StubCore>, inflight: &InflightCell, req: ReqFrame) -> Option<i32> {
    if env_matches("ONECAD_STUB_CRASH_ON", &req.verb) {
        eprintln!("stub: CRASH_ON {} -> abort()", req.verb);
        std::process::abort();
    }
    if env_matches("ONECAD_STUB_HANG_ON", &req.verb) {
        eprintln!(
            "stub: HANG_ON {} -> wedging the kernel lane forever (status lane stays live)",
            req.verb
        );
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    if env_matches("ONECAD_STUB_SLOW_ON", &req.verb) {
        if let Some(code) = run_slow_hook(core, inflight, &req) {
            return Some(code);
        }
    }
    if let Some(result) = stale_preview_hook(core, &req) {
        if let Err(err) = result {
            eprintln!("stub: write error for STALE_PREVIEW {}: {err}", req.verb);
            return Some(0);
        }
        return None;
    }
    let mut guard = core.lock().unwrap();
    let StubCore { out, state } = &mut *guard;
    handle_req(out, state, req)
}

/// `SLOW_ON`: hold the lane for `SLOW_MS`, emitting a `progress` frame every
/// `PROGRESS_EVERY_MS` (0 = silent). Each emitted frame notes progress against the
/// in-flight record, exactly as the worker's `inflight_note_progress` does.
fn run_slow_hook(core: &Mutex<StubCore>, inflight: &InflightCell, req: &ReqFrame) -> Option<i32> {
    let total = env_millis("ONECAD_STUB_SLOW_MS").unwrap_or(std::time::Duration::from_secs(1));
    // `0` is meaningful here (SILENT), so this cannot go through `env_millis`,
    // which treats 0 as "unset".
    let cadence = std::env::var("ONECAD_STUB_PROGRESS_EVERY_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(200);
    eprintln!(
        "stub: SLOW_ON {} -> answering in {total:?} (progress every {cadence} ms; 0 = silent)",
        req.verb
    );
    let started = Instant::now();
    if cadence == 0 {
        std::thread::sleep(total);
        return None;
    }
    let tick = std::time::Duration::from_millis(cadence);
    while started.elapsed() < total {
        std::thread::sleep(tick.min(total - started.elapsed()));
        let elapsed = started.elapsed();
        let fraction = (elapsed.as_secs_f64() / total.as_secs_f64()).min(1.0);
        let written = {
            let mut guard = core.lock().unwrap();
            let StubCore { out, state } = &mut *guard;
            let stamp = state.stamp();
            write_progress(out, req.id, "executing", fraction, stamp)
        };
        if let Err(err) = written {
            eprintln!("stub: failed to write progress: {err}");
            return Some(1);
        }
        if let Some(rec) = inflight.lock().unwrap().as_mut() {
            rec.last_progress = Instant::now();
        }
    }
    None
}

/// How many times `STALE_ON` has already fired (process-wide, like every other
/// counter-shaped hook here).
static STALE_FIRED: AtomicU64 = AtomicU64::new(0);

/// `STALE_ON`/`STALE_TIMES`: refuse the first N requests for a verb with the
/// §7.6 `STALE_PREVIEW` shape the real worker emits — a structured code plus
/// `detail {requested, head}`, so Rust never has to read the message text.
/// `None` ⇒ the hook did not fire and the handler runs normally.
fn stale_preview_hook(core: &Mutex<StubCore>, req: &ReqFrame) -> Option<Result<(), ProtocolError>> {
    if !env_matches("ONECAD_STUB_STALE_ON", &req.verb) {
        return None;
    }
    let times = std::env::var("ONECAD_STUB_STALE_TIMES")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(1);
    if STALE_FIRED.fetch_add(1, Ordering::SeqCst) >= times {
        return None;
    }
    let mut guard = core.lock().unwrap();
    let StubCore { out, state } = &mut *guard;
    let requested = req.args.get("snapshotId").and_then(Value::as_u64);
    let head = state.snapshot_id;
    let stamp = state.stamp();
    eprintln!(
        "stub: STALE_ON {} -> STALE_PREVIEW (requested {requested:?}, head {head})",
        req.verb
    );
    Some(write_resp_err(
        out,
        req.id,
        stamp,
        ErrorObject {
            code: ErrorCode::StalePreview,
            message: format!(
                "{}: request is against snapshot {}; head is {head}",
                req.verb,
                requested.unwrap_or(0)
            ),
            detail: Some(json!({ "requested": requested, "head": head })),
            retriable: false,
        },
    ))
}

/// The liveness lane (SCHEMA §7.1). It answers `GetWorkerHead` and nothing else,
/// takes only the core lock, and touches no chaos hook — so it keeps answering
/// while the kernel lane is wedged, which is what lets Rust tell a slow op from a
/// dead process.
fn status_loop(core: &Mutex<StubCore>, inflight: &InflightCell, rx: &Receiver<ReqFrame>) {
    while let Ok(req) = rx.recv() {
        let snapshot = inflight.lock().unwrap().as_ref().map(|rec| {
            let now = Instant::now();
            InflightResult {
                verb: rec.verb.clone(),
                id: rec.id,
                job_id: rec.job_id,
                age_ms: now.duration_since(rec.started).as_millis() as u64,
                since_progress_ms: now.duration_since(rec.last_progress).as_millis() as u64,
            }
        });
        let mut guard = core.lock().unwrap();
        let StubCore { out, state } = &mut *guard;
        let stamp = state.stamp();
        let result = write_resp_ok(
            out,
            req.id,
            stamp,
            &WorkerHeadResult {
                document_revision: state.document_revision,
                worker_epoch: state.worker_epoch,
                snapshot_id: state.snapshot_id,
                history_prefix_hash: state.history_prefix_hash.clone(),
                has_scratch: !state.prepared.is_empty(),
                // The stub owns no checkpoint slot, so this is always false — the
                // field is mirrored so its ABSENCE is never what a Rust reader is
                // being tested against.
                has_restored_base: false,
                inflight: snapshot,
            },
        );
        if let Err(err) = result {
            eprintln!("stub: write error for GetWorkerHead: {err}");
            return;
        }
    }
}

/// Handle one KERNEL-lane request. Returns `Some(exit_code)` if the process should
/// exit. The chaos hooks fire in [`run_kernel_job`], BEFORE the core lock this is
/// called under — a handler must never be able to hold the status lane hostage.
fn handle_req<W: Write>(writer: &mut W, state: &mut StubState, req: ReqFrame) -> Option<i32> {
    let result: Result<(), ProtocolError> = match req.verb.as_str() {
        "Shutdown" => {
            let stamp = state.stamp();
            let r = write_resp_ok(writer, req.id, stamp, &ShutdownResult { goodbye: true });
            if let Err(err) = r {
                eprintln!("stub: failed to write Shutdown resp: {err}");
            }
            // Graceful stop (SCHEMA §7.1): flush, reply, exit 0.
            return Some(0);
        }
        "OpenSession" => handle_open_session(writer, state, &req),
        "CloseSession" => {
            state.session_open = false;
            let stamp = state.stamp();
            write_resp_ok(
                writer,
                req.id,
                stamp,
                &CloseSessionResult {
                    session_closed: true,
                },
            )
        }
        "ResetSession" => {
            state.worker_epoch += 1;
            state.prepared.clear();
            let stamp = state.stamp();
            write_resp_value(
                writer,
                req.id,
                stamp,
                json!({ "reset": true, "workerEpoch": state.worker_epoch }),
                &[],
                &[],
            )
        }
        "ExecutePlan" => handle_execute_plan(writer, state, &req),
        "AcceptPrepared" => handle_accept_prepared(writer, state, &req),
        "DiscardPrepared" => handle_discard_prepared(writer, state, &req),
        "Tessellate" => handle_tessellate(writer, state, &req),
        // The stub has no BRep, but mirrors the real verb's strict addressing
        // and returns its canonical single-prism placeholder topology.
        "QueryBodyTopology" => handle_query_body_topology(writer, state, &req),
        // Same story: no BRep behind them, but the addressing, the not-found
        // convention and (for the bake) the on-disk side effect are real.
        "ClassifyElement" => handle_classify_element(writer, state, &req),
        "ExportGeometry" => handle_export_geometry(writer, state, &req),
        // --- solver lane (SCHEMA §7.4) ---
        "SketchUpsert" => handle_sketch_upsert(writer, state, &req),
        "BeginGesture" => handle_begin_gesture(writer, state, &req),
        "SolveDrag" => handle_solve_drag(writer, state, &req),
        "EndGesture" => handle_end_gesture(writer, state, &req),
        "SketchRegions" => handle_sketch_regions(writer, state, &req),
        // --- element identity (SCHEMA §7.5) ---
        "AcquireElementIds" => handle_acquire_element_ids(writer, state, &req),
        "BindElementIds" => handle_bind_element_ids(writer, state, &req),
        "ResolveRefs" => handle_resolve_refs(writer, state, &req),
        unknown => {
            // Well-framed but protocol-illegal: terminal error resp (SCHEMA §8).
            let stamp = state.stamp();
            write_resp_err(
                writer,
                req.id,
                stamp,
                ErrorObject {
                    code: ErrorCode::ProtocolError,
                    message: format!("unknown verb: {unknown}"),
                    detail: None,
                    retriable: false,
                },
            )
        }
    };

    if let Err(err) = result {
        eprintln!("stub: write error for verb {}: {err}", req.verb);
        // A broken stdout means the driver is gone; exit cleanly.
        return Some(0);
    }
    None
}

fn handle_query_body_topology<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let body_id = req.args.get("bodyId").and_then(Value::as_str).unwrap_or("");
    let stamp = state.stamp();
    if !body_id.starts_with("body_") {
        return write_resp_err(
            writer,
            req.id,
            stamp,
            ErrorObject {
                code: ErrorCode::RefUnresolved,
                message: format!("QueryBodyTopology: unknown bodyId '{body_id}'"),
                detail: None,
                retriable: false,
            },
        );
    }
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({ "solidCount": 1, "faceCount": 6 }),
        &[],
        &[],
    )
}

/// `ClassifyElement` (SCHEMA §7.5) — the placement solver's hover query.
///
/// The stub has no BRep to classify, so it answers for the same canonical prism
/// `QueryBodyTopology` reports: a planar face at the origin with a +Z normal.
/// What it DOES mirror faithfully is the addressing and the not-found
/// convention — either `elementId` or `topoKey` is required, a bad `bodyId` is
/// `REF_UNRESOLVED`, and an element the head does not hold is `present: false`,
/// an ANSWER rather than an error (the verb is re-issued every hover frame, so
/// a miss must not look like a failure).
fn handle_classify_element<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let body_id = req.args.get("bodyId").and_then(Value::as_str).unwrap_or("");
    let element_id = req
        .args
        .get("elementId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let topo_key = req
        .args
        .get("topoKey")
        .and_then(Value::as_str)
        .unwrap_or("");
    let stamp = state.stamp();
    if !body_id.starts_with("body_") {
        return write_resp_err(
            writer,
            req.id,
            stamp,
            ErrorObject {
                code: ErrorCode::RefUnresolved,
                message: format!("ClassifyElement: unknown bodyId '{body_id}'"),
                detail: None,
                retriable: false,
            },
        );
    }
    if element_id.is_empty() && topo_key.is_empty() {
        return write_resp_err(
            writer,
            req.id,
            stamp,
            ErrorObject {
                code: ErrorCode::ProtocolError,
                message: "ClassifyElement: one of elementId / topoKey is required".into(),
                detail: None,
                retriable: false,
            },
        );
    }
    // Only a face address resolves against the fiction; anything else is a miss.
    let present = topo_key.starts_with("f:") || (topo_key.is_empty() && !element_id.is_empty());
    if !present {
        return write_resp_value(writer, req.id, stamp, json!({ "present": false }), &[], &[]);
    }
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({
            "present": true,
            "kind": "face",
            "surfaceType": "plane",
            "curveType": "",
            "frame": { "origin": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0] },
        }),
        &[],
        &[],
    )
}

/// `ExportGeometry` (SCHEMA §7.8) — the component-authoring bake.
///
/// The stub writes a REAL file and reports its REAL byte count. A stub that
/// returned a plausible `bytes` without writing anything would make the one
/// thing this verb exists to do (produce a package payload on disk) untestable
/// through the stub lane, which is the failure mode the whole stub is meant to
/// avoid.
fn handle_export_geometry<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let path = req.args.get("path").and_then(Value::as_str).unwrap_or("");
    let codec = req
        .args
        .get("codec")
        .and_then(Value::as_str)
        .unwrap_or("brep");
    let bodies = req
        .args
        .get("bodyIds")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let union_solids = req
        .args
        .get("union")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stamp = state.stamp();
    if path.is_empty() || bodies == 0 {
        return write_resp_err(
            writer,
            req.id,
            stamp,
            ErrorObject {
                code: ErrorCode::ProtocolError,
                message: "ExportGeometry: path and a non-empty bodyIds are required".into(),
                detail: None,
                retriable: false,
            },
        );
    }
    // Refused for the same reason the real worker refuses it: a component that
    // bakes to more than one solid moves its failure to whoever places it.
    if bodies > 1 && !union_solids {
        return write_resp_err(
            writer,
            req.id,
            stamp,
            ErrorObject {
                code: ErrorCode::ProtocolError,
                message: format!("ExportGeometry: {bodies} solids without union"),
                detail: None,
                retriable: false,
            },
        );
    }
    let payload = format!("onecad-worker-stub {codec} bake\n");
    if let Err(error) = std::fs::write(path, payload.as_bytes()) {
        return write_resp_err(
            writer,
            req.id,
            stamp,
            ErrorObject {
                code: ErrorCode::ProtocolError,
                message: format!("ExportGeometry: cannot write '{path}': {error}"),
                detail: None,
                retriable: false,
            },
        );
    }
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({
            "codec": codec,
            "format": 1,
            "solidCount": 1,
            "bytes": payload.len(),
        }),
        &[],
        &[],
    )
}

fn handle_open_session<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    match serde_json::from_value::<OpenSessionArgs>(req.args.clone()) {
        Ok(args) => {
            state.session_open = true;
            state.document_revision = args.document_revision;
            state.worker_epoch = args.worker_epoch;
            state.snapshot_id = 0;
            // Fresh document ⇒ empty-prefix head hash; the fencing baseline is now set
            // (SCHEMA §7.1 / D4).
            state.history_prefix_hash = EMPTY_PREFIX_HASH.to_string();
            state.fencing_baseline_set = true;
            let stamp = state.stamp();
            write_resp_ok(
                writer,
                req.id,
                stamp,
                &OpenSessionResult {
                    session_open: true,
                    worker_head: WorkerHeadBrief {
                        document_revision: state.document_revision,
                        snapshot_id: state.snapshot_id,
                    },
                },
            )
        }
        Err(err) => {
            // Malformed args for a known verb: well-framed-illegal PROTOCOL_ERROR.
            let stamp = state.stamp();
            write_resp_err(
                writer,
                req.id,
                stamp,
                ErrorObject {
                    code: ErrorCode::ProtocolError,
                    message: format!("invalid OpenSession args: {err}"),
                    detail: None,
                    retriable: false,
                },
            )
        }
    }
}

/// Minimal `ExecutePlan` (SCHEMA §7.2): one `planStep` per op minting a
/// deterministic `body_<opId>`, then a terminal `PlanPrepared` echoing the plan's
/// opaque last-executed history-prefix token (the executor verifies that echo).
///
/// The convergence-drill hook `ONECAD_STUB_CRASH_COUNTDOWN` `abort()`s **mid-plan**
/// (after the first `planStep`) while its persisted counter is `> 0`.
fn handle_execute_plan<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let args = &req.args;
    let job_id = args.get("jobId").and_then(Value::as_u64);
    let ops = args
        .get("ops")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let prefix_hashes = args
        .get("prefixHashes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let expected_base = args
        .get("expectedBaseHash")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let plan_revision = args
        .get("documentRevision")
        .and_then(Value::as_u64)
        .unwrap_or(state.document_revision);
    let plan_epoch = args
        .get("workerEpoch")
        .and_then(Value::as_u64)
        .unwrap_or(state.worker_epoch);

    // D4 fencing — mirror the real worker: workerEpoch + expectedBaseHash ONLY,
    // never documentRevision (an advisory Rust-owned edit counter). Lazily adopt the
    // baseline from the first plan when no OpenSession established it (chaos drills).
    if !state.fencing_baseline_set {
        state.worker_epoch = plan_epoch;
        state.history_prefix_hash = expected_base.clone();
        state.fencing_baseline_set = true;
    }
    if plan_epoch != state.worker_epoch {
        let stamp = state.stamp_job(job_id);
        return write_resp_err(
            writer,
            req.id,
            stamp,
            ErrorObject {
                code: ErrorCode::ProtocolError,
                message: "ExecutePlan: workerEpoch fencing mismatch".into(),
                detail: Some(json!({ "headEpoch": state.worker_epoch, "planEpoch": plan_epoch })),
                retriable: false,
            },
        );
    }
    // D5: a from-0 plan — no `baseCheckpoint` AND expectedBaseHash == the empty anchor
    // — is ALWAYS base-valid: SKIP the head-hash comparison so a full-replay regen
    // still prepares after the head token advanced past the empty anchor (the
    // RegenPlanner always replays from 0; after the first accept the head is nonzero,
    // and the strict fence would reject every subsequent regen). workerEpoch fencing
    // (above) is unchanged; on accept the head is replaced wholesale. Incremental
    // plans (nonzero expectedBaseHash) keep the strict head-hash fence. Mirrors the
    // real worker's Session::fence_and_clone.
    // One scratch at a time (SCHEMA §7.2, mirroring `Session::fence_and_clone`): a
    // re-sent SAME jobId is idempotent (re-return the cached PlanPrepared), a
    // DIFFERENT jobId is a PROTOCOL_ERROR. Without this the stub silently accepts a
    // second prepare, and every Rust drill about ORDERING a `DiscardPrepared`
    // against the next plan is vacuously green.
    if let Some(p) = state.prepared.first() {
        if Some(p.job_id) == job_id {
            let result = p.result.clone();
            let stamp = state.stamp_job(job_id);
            return write_resp_value(writer, req.id, stamp, result, &[], &[]);
        }
        let prepared_job = p.job_id;
        let stamp = state.stamp_job(job_id);
        return write_resp_err(
            writer,
            req.id,
            stamp,
            ErrorObject {
                code: ErrorCode::ProtocolError,
                message: "ExecutePlan: a plan is already prepared; accept or discard it first"
                    .into(),
                detail: Some(json!({ "preparedJobId": prepared_job, "requestedJobId": job_id })),
                retriable: false,
            },
        );
    }

    let from_zero = expected_base == EMPTY_PREFIX_HASH && args.get("baseCheckpoint").is_none();
    if !from_zero && expected_base != state.history_prefix_hash {
        let stamp = state.stamp_job(job_id);
        return write_resp_err(
            writer,
            req.id,
            stamp,
            ErrorObject {
                code: ErrorCode::ProtocolError,
                message: "ExecutePlan: expectedBaseHash mismatch".into(),
                detail: Some(
                    json!({ "expected": expected_base, "actual": state.history_prefix_hash }),
                ),
                retriable: false,
            },
        );
    }

    let mut per_step: Vec<Value> = Vec::new();
    let mut last_valid: Option<u64> = None;
    for (i, op) in ops.iter().enumerate() {
        let step_index = op
            .get("stepIndex")
            .and_then(Value::as_u64)
            .unwrap_or(i as u64);
        let op_id = op
            .get("opId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // F3 poison test: crash (transport loss) when a specific op executes, so the
        // crash is attributed to THAT op's poison key (not the plan's last op). The
        // crash is before the planStep, so `steps_received` points at this op.
        if crash_on_op_matches(&op_id) {
            eprintln!("stub: CRASH_ON_OP {op_id} -> abort() mid-plan");
            std::process::abort();
        }
        let body_id = format!("body_{op_id}");
        let payload = json!({
            "stepIndex": step_index,
            "bodyEvents": [ { "kind": "created", "bodyId": body_id } ],
            "elementMapDelta": { "added": [], "removed": [], "relabeled": [] },
            "needsRepair": [],
            "signatures": {
                "geometry": format!("g{step_index}"),
                "bodyLifecycle": format!("b{step_index}"),
                "referencedBinding": format!("r{step_index}"),
            },
            "diagnostics": [],
        });
        let stamp = state.stamp_job(job_id);
        write_event(writer, req.id, "planStep", step_index, payload, stamp)?;
        // Mid-plan convergence-drill crash: after the first step is on the wire.
        if i == 0 {
            crash_countdown();
        }
        per_step.push(json!({ "stepIndex": step_index, "status": "ok", "bodyIds": [body_id] }));
        last_valid = Some(step_index);
    }

    let prepared_snapshot = state.snapshot_id + 1;
    // The head token this job would adopt on accept: prefixHashes[last] (or the base
    // hash for a base-only prepare) — the same opaque token the real worker echoes.
    let echo = prefix_hashes
        .last()
        .and_then(Value::as_str)
        .unwrap_or(&expected_base)
        .to_string();
    let result = json!({
        "planPrepared": true,
        "preparedSnapshotId": prepared_snapshot,
        "lastValidStep": last_valid,
        "stoppedReason": "completed",
        "perStepResults": per_step,
        "historyPrefixHash": echo,
    });
    if let Some(j) = job_id {
        state.prepared.push(Prepared {
            job_id: j,
            prepared_snapshot_id: prepared_snapshot,
            history_prefix_hash: echo.clone(),
            plan_document_revision: plan_revision,
            result: result.clone(),
        });
    }
    let stamp = state.stamp_job(job_id);
    write_resp_value(writer, req.id, stamp, result, &[], &[])
}

/// `AcceptPrepared` (SCHEMA §7.2 / D4): publish the scratch snapshot; ADOPT the
/// plan's advisory `documentRevision` + echoed `historyPrefixHash` as the head.
fn handle_accept_prepared<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let job_id = req.args.get("jobId").and_then(Value::as_u64);
    let prepared = job_id.and_then(|j| {
        let pos = state.prepared.iter().position(|p| p.job_id == j)?;
        Some(state.prepared.remove(pos))
    });
    match prepared {
        Some(p) => {
            state.snapshot_id = p.prepared_snapshot_id;
            // D4: adopt the plan's revision (not a worker-owned +1) + the head token.
            state.document_revision = p.plan_document_revision;
            state.history_prefix_hash = p.history_prefix_hash;
            let stamp = state.stamp_job(job_id);
            write_resp_value(
                writer,
                req.id,
                stamp,
                json!({ "accepted": true, "snapshotId": p.prepared_snapshot_id, "documentRevision": state.document_revision }),
                &[],
                &[],
            )
        }
        None => {
            let stamp = state.stamp_job(job_id);
            write_resp_err(
                writer,
                req.id,
                stamp,
                ErrorObject {
                    code: ErrorCode::ProtocolError,
                    message: "AcceptPrepared for unknown/absent job".into(),
                    detail: None,
                    retriable: false,
                },
            )
        }
    }
}

/// `DiscardPrepared` (SCHEMA §7.2): drop the scratch job; session unchanged.
fn handle_discard_prepared<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    // Chaos: answer `discarded: true` and keep the scratch anyway. The CONTROL half
    // of the dropped-prepare drill — it is what a worker looked like before Rust's
    // §7.2 Drop guarantee existed, and it is the only way to show that the drill's
    // observation (`hasScratch` at the head) can see a stranded scratch at all. With
    // the guard working, the real scratch lives for microseconds between the
    // prepare and the discard queued behind it, far too short to poll for.
    if env_flag("ONECAD_STUB_IGNORE_DISCARD") {
        eprintln!("stub: IGNORE_DISCARD -> answering ok but KEEPING the scratch");
    } else if let Some(j) = req.args.get("jobId").and_then(Value::as_u64) {
        state.prepared.retain(|p| p.job_id != j);
    }
    let stamp = state.stamp();
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({ "discarded": true }),
        &[],
        &[],
    )
}

// ── Solver lane (SCHEMA §7.4) — echo-style deterministic solve ───────────────

fn read_xy(v: &Value) -> Option<[f64; 2]> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

/// `SketchUpsert` (SCHEMA §7.4): store the point positions + report a deterministic
/// `dof = max(0, 2·points − constraints)` and derived state.
fn handle_sketch_upsert<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let args = &req.args;
    let sketch_id = args
        .get("sketchId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let entities = args
        .get("entities")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let constraint_count = args
        .get("constraints")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    let mut points = BTreeMap::new();
    let mut point_count = 0;
    for e in &entities {
        if e.get("type").and_then(Value::as_str) == Some("Point") {
            point_count += 1;
            if let (Some(id), Some(xy)) = (
                e.get("id").and_then(Value::as_str),
                e.get("at").and_then(read_xy),
            ) {
                points.insert(id.to_string(), xy);
            }
        }
    }
    let prev_rev = state.sketches.get(&sketch_id).map_or(0, |s| s.revision);
    let sk = StubSketch {
        revision: prev_rev + 1,
        points,
        point_count,
        constraint_count,
    };
    let (dof, st, rev) = (sk.dof(), sk.state(), sk.revision);
    state.sketches.insert(sketch_id.clone(), sk);
    let stamp = state.stamp();
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({ "upserted": true, "sketchId": sketch_id, "sketchRevision": rev, "dof": dof, "state": st }),
        &[],
        &[],
    )
}

/// `BeginGesture` (SCHEMA §7.4): snapshot the point baseline for change reporting.
fn handle_begin_gesture<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let args = &req.args;
    let gesture_id = args.get("gestureId").and_then(Value::as_u64).unwrap_or(0);
    let sketch_id = args
        .get("sketchId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let drag_point = args
        .get("drag")
        .and_then(|d| d.get("pointId"))
        .and_then(Value::as_str)
        .or_else(|| args.get("pointId").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let baseline = state
        .sketches
        .get(&sketch_id)
        .map(|s| s.points.clone())
        .unwrap_or_default();
    state.gestures.insert(
        gesture_id,
        StubGesture {
            sketch_id,
            drag_point,
            max_seq: 0,
            baseline,
        },
    );
    let stamp = state.stamp();
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({ "gestureId": gesture_id, "ready": true }),
        &[],
        &[],
    )
}

/// `SolveDrag` (SCHEMA §7.4): apply the drag delta to the dragged point. A `seq`
/// not newer than the gesture's `max_seq` resolves **superseded** (latest-wins).
fn handle_solve_drag<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let args = &req.args;
    let gesture_id = args.get("gestureId").and_then(Value::as_u64).unwrap_or(0);
    let seq = args.get("seq").and_then(Value::as_u64).unwrap_or(0);
    let target = args.get("target").and_then(read_xy).unwrap_or([0.0, 0.0]);

    let (sketch_id, drag_point, stale) = match state.gestures.get_mut(&gesture_id) {
        Some(g) => {
            let stale = g.max_seq > 0 && seq <= g.max_seq;
            if !stale {
                g.max_seq = seq;
            }
            (g.sketch_id.clone(), g.drag_point.clone(), stale)
        }
        None => {
            let stamp = state.stamp();
            return write_resp_err(
                writer,
                req.id,
                stamp,
                ErrorObject {
                    code: ErrorCode::RefUnresolved,
                    message: "SolveDrag: unknown or ended gesture".into(),
                    detail: None,
                    retriable: false,
                },
            );
        }
    };

    let mut positions = serde_json::Map::new();
    if !stale {
        if let Some(sk) = state.sketches.get_mut(&sketch_id) {
            sk.points.insert(drag_point.clone(), target);
        }
        positions.insert(drag_point, json!([target[0], target[1]]));
    }
    let dof = state.sketches.get(&sketch_id).map_or(0, StubSketch::dof);
    let status = if stale { "superseded" } else { "success" };
    let stamp = state.stamp();
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({
            "gestureId": gesture_id, "seq": seq, "status": status, "dof": dof,
            "conflicting": [], "positions": Value::Object(positions), "solveMicros": 42,
        }),
        &[],
        &[],
    )
}

/// `EndGesture` (SCHEMA §7.4): apply the final target, bump the sketch revision,
/// and report the points changed since the gesture began.
fn handle_end_gesture<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let args = &req.args;
    let gesture_id = args.get("gestureId").and_then(Value::as_u64).unwrap_or(0);
    let Some(g) = state.gestures.remove(&gesture_id) else {
        let stamp = state.stamp();
        return write_resp_err(
            writer,
            req.id,
            stamp,
            ErrorObject {
                code: ErrorCode::RefUnresolved,
                message: "EndGesture: unknown or ended gesture".into(),
                detail: None,
                retriable: false,
            },
        );
    };
    if let Some(ft) = args
        .get("commit")
        .and_then(|c| c.get("finalTarget"))
        .and_then(read_xy)
    {
        if let Some(sk) = state.sketches.get_mut(&g.sketch_id) {
            sk.points.insert(g.drag_point.clone(), ft);
        }
    }
    let (rev, dof, positions) = match state.sketches.get_mut(&g.sketch_id) {
        Some(sk) => {
            sk.revision += 1;
            let mut pos = serde_json::Map::new();
            for (k, v) in &sk.points {
                let changed = g
                    .baseline
                    .get(k)
                    .is_none_or(|b| (b[0] - v[0]).abs() > 1e-9 || (b[1] - v[1]).abs() > 1e-9);
                if changed {
                    pos.insert(k.clone(), json!([v[0], v[1]]));
                }
            }
            (sk.revision, sk.dof(), Value::Object(pos))
        }
        None => (0, 0, json!({})),
    };
    let stamp = state.stamp();
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({ "gestureId": gesture_id, "status": "success", "dof": dof, "positions": positions, "sketchRevision": rev }),
        &[],
        &[],
    )
}

/// `SketchRegions` (SCHEMA §7.4): the stub has no loop detector, so it returns an
/// empty region set (the flow is exercised; real regions need the C++ worker).
fn handle_sketch_regions<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let sketch_id = req
        .args
        .get("sketchId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let rev = state.sketches.get(&sketch_id).map_or(0, |s| s.revision);
    let stamp = state.stamp();
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({ "sketchId": sketch_id, "sketchRevision": rev, "regions": [] }),
        &[],
        &[],
    )
}

// ── Element identity (SCHEMA §7.5) — echo-style evidence ─────────────────────

/// `AcquireElementIds` (SCHEMA §7.5): echo one evidence entry per pick with an
/// empty `elementId` (Rust mints the id) + a stub descriptor.
fn handle_acquire_element_ids<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let args = &req.args;
    let body_id = args
        .get("bodyId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut ids = Vec::new();
    if let Some(picks) = args.get("picks").and_then(Value::as_array) {
        for p in picks {
            let topo = p.get("topoKey").and_then(Value::as_str).unwrap_or("");
            let kind = match topo.chars().next() {
                Some('e') => "edge",
                Some('v') => "vertex",
                _ => "face",
            };
            let mut entry = json!({
                "topoKey": topo, "kind": kind, "bodyId": body_id,
                "elementId": "", "descriptor": { "stub": true },
            });
            if let Some(a) = p.get("anchor") {
                entry["anchor"] = a.clone();
            }
            ids.push(entry);
        }
    }
    let stamp = state.stamp();
    write_resp_value(writer, req.id, stamp, json!({ "ids": ids }), &[], &[])
}

/// `BindElementIds` echo for the geometry-free stub. The real worker validates
/// shapes and installs atomically; this lane pins only the cross-language wire
/// contract so Rust promotion can complete against the fake sidecar.
fn handle_bind_element_ids<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let bound = req
        .args
        .get("bindings")
        .and_then(Value::as_array)
        .map(|bindings| {
            bindings
                .iter()
                .map(|binding| {
                    json!({
                        "bodyId": binding.get("bodyId").cloned().unwrap_or(Value::Null),
                        "topoKey": binding.get("topoKey").cloned().unwrap_or(Value::Null),
                        "elementId": binding.get("elementId").cloned().unwrap_or(Value::Null),
                        "kind": binding.get("kind").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let stamp = state.stamp();
    write_resp_value(writer, req.id, stamp, json!({ "bound": bound }), &[], &[])
}

/// `ResolveRefs` (SCHEMA §7.5): a deterministic dry run — an already-bound ref is
/// `unchanged`, anything else `autoBind`s with a canned high score.
fn handle_resolve_refs<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let mut resolutions = Vec::new();
    // SCHEMA §7.5 echo: the snapshot the resolution was computed against (the request's
    // when it names one, else the head) and the revision of that head. Rust validates
    // this echo fail-closed, so the stub has to speak it too or the whole stub lane
    // would fail for a reason that has nothing to do with what a test is asserting.
    let echoed_snapshot = req
        .args
        .get("snapshotId")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| state.stamp().snapshot_id);
    let echoed_revision = state.stamp().document_revision;
    if let Some(refs) = req.args.get("refs").and_then(Value::as_array) {
        for r in refs {
            let ref_id = r.get("refId").and_then(Value::as_str).unwrap_or("");
            let body_id = r
                .get("primary")
                .and_then(|p| p.get("bodyId"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let existing = r
                .get("primary")
                .and_then(|p| p.get("elementId"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let mut resolution = match (existing, body_id.is_empty()) {
                // SCHEMA §7.5: no body to enumerate candidates from. The real worker
                // takes its missing-body branch here, and that non-promotable
                // no-candidates shape is the ONLY resolution allowed to omit `bodyId`.
                (_, true) => json!({
                    "refId": ref_id,
                    "outcome": "needsRepair",
                    "needsRepair": {
                        "refId": ref_id,
                        "elementId": existing.unwrap_or(""),
                        "ladderFailed": "descriptor",
                        "reason": "no-candidates",
                        "candidates": [],
                        "uiLabel": "referenced body not found",
                    },
                }),
                (Some(eid), false) => {
                    json!({ "refId": ref_id, "outcome": "unchanged", "elementId": eid, "topoKey": "f:0" })
                }
                // SCHEMA §7.5: `elementId` slot (empty — the stub holds no partition,
                // so an autoBind resolves an unminted element); `topoKey` = evidence.
                (None, false) => {
                    json!({ "refId": ref_id, "outcome": "autoBind", "elementId": "", "topoKey": "f:0", "score": 0.95, "margin": 0.5 })
                }
            };
            resolution["snapshotId"] = json!(echoed_snapshot);
            resolution["revision"] = json!(echoed_revision);
            if !body_id.is_empty() {
                resolution["bodyId"] = json!(body_id);
            }
            resolutions.push(resolution);
        }
    }
    let stamp = state.stamp();
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({ "resolutions": resolutions }),
        &[],
        &[],
    )
}

/// `Tessellate` (SCHEMA §7.6): a header-only valid MESH1 blob per requested body,
/// inline in the resp tail — or streamed as a bulk chunk manifest + data frames
/// when `ONECAD_STUB_CHUNKED_MESH=1` (SCHEMA §5.2), exercising the chunk path.
fn handle_tessellate<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    req: &ReqFrame,
) -> Result<(), ProtocolError> {
    let lod = req
        .args
        .get("lod")
        .and_then(Value::as_str)
        .unwrap_or("coarse");
    let body_ids: Vec<String> = match req.args.get("bodyIds") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        // "all" (or unspecified) has no body registry in the stub → one synthetic.
        _ => vec!["body_all".to_string()],
    };

    let chunked = env_flag("ONECAD_STUB_CHUNKED_MESH");
    let mut meshes: Vec<Value> = Vec::new();
    let mut tail: Vec<u8> = Vec::new();
    let mut bin_sections: Vec<BinSection> = Vec::new();
    for body in &body_ids {
        let blob = mesh1_blob(lod);
        let sha = sha256_hex(&blob);
        if chunked {
            let stream_id = state.stream_id;
            state.stream_id += 1;
            stream_mesh(writer, state, req.id, stream_id, body, lod, &blob, &sha)?;
            meshes.push(json!({
                "bodyId": body, "streamId": stream_id, "format": "MESH1",
                "totalBytes": blob.len(), "sha256": sha, "snapshotId": state.snapshot_id,
            }));
        } else {
            let name = format!("mesh:{body}");
            let off = tail.len() as u32;
            tail.extend_from_slice(&blob);
            bin_sections.push(BinSection {
                name: name.clone(),
                off,
                len: blob.len() as u32,
            });
            meshes.push(json!({
                "bodyId": body, "bin": name, "format": "MESH1",
                "totalBytes": blob.len(), "sha256": sha, "snapshotId": state.snapshot_id,
            }));
        }
    }
    let stamp = state.stamp();
    write_resp_value(
        writer,
        req.id,
        stamp,
        json!({ "meshes": meshes }),
        &bin_sections,
        &tail,
    )
}

/// Stream one MESH1 blob as a manifest chunk + `count` data chunks (SCHEMA §5.2).
#[allow(clippy::too_many_arguments)]
fn stream_mesh<W: Write>(
    writer: &mut W,
    state: &mut StubState,
    id: u64,
    stream_id: u64,
    body: &str,
    lod: &str,
    blob: &[u8],
    sha: &str,
) -> Result<(), ProtocolError> {
    // Split into 2 data frames to prove multi-frame assembly by byteOffset. Each
    // frame is `(byteOffset, slice)`. The F5 gap hook perturbs the tiling so the
    // client's StreamAcc gap-detection must fire (a hole or an overlap in
    // [0, totalBytes)); `mode` ∈ "gap" (hole) | "overlap".
    let mid = blob.len().div_ceil(2);
    let gap_mode = std::env::var("ONECAD_STUB_CHUNKED_MESH_GAP").ok();
    let frames: Vec<(u64, &[u8])> = if mid == 0 || mid >= blob.len() {
        vec![(0, blob)]
    } else {
        match gap_mode.as_deref() {
            // Hole: the 2nd chunk starts 4 bytes past `mid`, leaving [mid, mid+4)
            // uncovered.
            Some("gap") => vec![(0, &blob[..mid]), (mid as u64 + 4, &blob[mid..])],
            // Overlap: the 2nd chunk starts 4 bytes before `mid`, re-covering
            // [mid-4, mid).
            Some("overlap") => vec![(0, &blob[..mid]), (mid as u64 - 4, &blob[mid - 4..])],
            _ => vec![(0, &blob[..mid]), (mid as u64, &blob[mid..])],
        }
    };
    let manifest = ChunkFrame {
        v: PROTOCOL_VERSION,
        id,
        stream_id,
        kind: ChunkKind::Manifest,
        purpose: Some("mesh".into()),
        count: Some(frames.len() as u32),
        total_bytes: Some(blob.len() as u64),
        sha256: Some(sha.to_string()),
        meta: Some(json!({ "bodyId": body, "lod": lod, "format": "MESH1" })),
        index: None,
        byte_offset: None,
        bin: None,
        document_revision: state.document_revision,
        worker_epoch: state.worker_epoch,
        snapshot_id: state.snapshot_id,
        job_id: None,
        seq: state.next_seq(),
    };
    write_frame(writer, &Frame::Chunk(manifest))?;
    for (index, (offset, part)) in frames.iter().enumerate() {
        let data = ChunkFrame {
            v: PROTOCOL_VERSION,
            id,
            stream_id,
            kind: ChunkKind::Data,
            purpose: None,
            count: None,
            total_bytes: None,
            sha256: None,
            meta: None,
            index: Some(index as u32),
            byte_offset: Some(*offset),
            bin: Some(vec![BinSection {
                name: "chunk".into(),
                off: 0,
                len: part.len() as u32,
            }]),
            document_revision: state.document_revision,
            worker_epoch: state.worker_epoch,
            snapshot_id: state.snapshot_id,
            job_id: None,
            seq: state.next_seq(),
        };
        let json = Frame::Chunk(data).to_json_vec()?;
        write_frame_blocking(writer, &json, part)?;
    }
    Ok(())
}

/// A minimal but valid MESH1 blob: the 64-byte header alone (`sectionCount = 0`)
/// passes `validate_mesh_blob`. Enough for the smoke path's "MESH1 validates".
fn mesh1_blob(lod: &str) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0x00..0x04].copy_from_slice(&0x4D45_5348u32.to_le_bytes()); // "MESH" magic (LE)
    b[0x04..0x06].copy_from_slice(&1u16.to_le_bytes()); // version
    let lod_v: u16 = match lod {
        "medium" => 1,
        "fine" => 2,
        _ => 0,
    };
    b[0x1C..0x1E].copy_from_slice(&lod_v.to_le_bytes());
    // flags/counts/sectionCount/bbox/reserved all zero.
    b
}

/// Lowercase-hex SHA-256 (SCHEMA §2 hash form).
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(data);
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The `ONECAD_STUB_CRASH_COUNTDOWN` mid-plan crash: if the env names a counter
/// file whose value is `> 0`, decrement it (persisting across restarts) and
/// `abort()`. Absent env / zero counter ⇒ no-op (the plan completes).
fn crash_countdown() {
    let Ok(path) = std::env::var("ONECAD_STUB_CRASH_COUNTDOWN") else {
        return;
    };
    let n: i64 = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    if n > 0 {
        let _ = std::fs::write(&path, (n - 1).to_string());
        eprintln!("stub: CRASH_COUNTDOWN {n} -> abort() mid-plan");
        std::process::abort();
    }
}

/// Write one non-terminal `event` frame (SCHEMA §3.4).
/// Write a non-terminal `progress` frame (SCHEMA §3.3) for an in-flight request.
fn write_progress<W: Write>(
    writer: &mut W,
    id: u64,
    phase: &str,
    fraction: f64,
    stamp: Stamp,
) -> Result<(), ProtocolError> {
    let frame = Frame::Progress(ProgressFrame {
        v: PROTOCOL_VERSION,
        id,
        phase: phase.to_string(),
        fraction: Some(fraction),
        message: None,
        document_revision: stamp.document_revision,
        worker_epoch: stamp.worker_epoch,
        snapshot_id: stamp.snapshot_id,
        job_id: stamp.job_id,
        seq: stamp.seq,
    });
    write_frame(writer, &frame)
}

fn write_event<W: Write>(
    writer: &mut W,
    id: u64,
    event: &str,
    step_index: u64,
    payload: Value,
    stamp: Stamp,
) -> Result<(), ProtocolError> {
    let frame = Frame::Event(EventFrame {
        v: PROTOCOL_VERSION,
        id,
        event: event.to_string(),
        step_index: Some(step_index),
        payload,
        document_revision: stamp.document_revision,
        worker_epoch: stamp.worker_epoch,
        snapshot_id: stamp.snapshot_id,
        job_id: stamp.job_id,
        seq: stamp.seq,
    });
    write_frame(writer, &frame)
}

/// Write a success `resp` with an arbitrary JSON result + optional binary tail.
fn write_resp_value<W: Write>(
    writer: &mut W,
    id: u64,
    stamp: Stamp,
    result: Value,
    bin_sections: &[BinSection],
    bin: &[u8],
) -> Result<(), ProtocolError> {
    let frame = Frame::Resp(RespFrame {
        v: PROTOCOL_VERSION,
        id,
        ok: true,
        result: Some(result),
        error: None,
        document_revision: stamp.document_revision,
        worker_epoch: stamp.worker_epoch,
        snapshot_id: stamp.snapshot_id,
        job_id: stamp.job_id,
        seq: stamp.seq,
        bin: if bin_sections.is_empty() {
            None
        } else {
            Some(bin_sections.to_vec())
        },
    });
    let json = frame.to_json_vec()?;
    write_frame_blocking(writer, &json, bin)
}

fn write_hello<W: Write>(writer: &mut W, seq: u64) -> Result<(), ProtocolError> {
    let hello = Frame::Hello(HelloFrame {
        v: PROTOCOL_VERSION,
        seq,
        result: HelloResult {
            protocol_version: 1,
            worker_version: "stub-0.1".into(),
            occt: OcctInfo {
                version: "stub".into(),
                // SCHEMA §2 + hello.ndjson require a 64-bit hex fingerprint
                // ($hex64). The task's literal "stub" would fail that matcher, so
                // the stub emits a valid all-zero hex fingerprint instead.
                fingerprint: "0000000000000000".into(),
            },
            quantization_version: 1,
            solver_policy_version: 1,
            capabilities: vec![],
            // SCHEMA §6 + the hello fixture require a `limits` object.
            limits: Some(HelloLimits {
                chunk_size: 1_048_576,
                initial_bulk_credit: 8_388_608,
            }),
        },
    });
    write_frame(writer, &hello)
}

fn write_resp_ok<W: Write, T: serde::Serialize>(
    writer: &mut W,
    id: u64,
    stamp: Stamp,
    result: &T,
) -> Result<(), ProtocolError> {
    let value = serde_json::to_value(result)?;
    let frame = Frame::Resp(RespFrame {
        v: PROTOCOL_VERSION,
        id,
        ok: true,
        result: Some(value),
        error: None,
        document_revision: stamp.document_revision,
        worker_epoch: stamp.worker_epoch,
        snapshot_id: stamp.snapshot_id,
        job_id: stamp.job_id,
        seq: stamp.seq,
        bin: None,
    });
    write_frame(writer, &frame)
}

fn write_resp_err<W: Write>(
    writer: &mut W,
    id: u64,
    stamp: Stamp,
    error: ErrorObject,
) -> Result<(), ProtocolError> {
    let frame = Frame::Resp(RespFrame {
        v: PROTOCOL_VERSION,
        id,
        ok: false,
        result: None,
        error: Some(error),
        document_revision: stamp.document_revision,
        worker_epoch: stamp.worker_epoch,
        snapshot_id: stamp.snapshot_id,
        job_id: stamp.job_id,
        seq: stamp.seq,
        bin: None,
    });
    write_frame(writer, &frame)
}

fn write_frame<W: Write>(writer: &mut W, frame: &Frame) -> Result<(), ProtocolError> {
    let json = frame.to_json_vec()?;
    write_frame_blocking(writer, &json, &[])
}

/// Emit one frame whose magic is NOT `OCW1`, to drive the client's `BadMagic`.
fn emit_garbage<W: Write>(writer: &mut W) -> std::io::Result<()> {
    // A plausible-looking header with a corrupted magic + zero lengths, so the
    // reader stops at the magic comparison (SCHEMA §1: bytes are authoritative).
    let mut bad = Vec::new();
    let mut magic = MAGIC_BYTES;
    magic[0] ^= 0xFF; // definitely not 'O'
    bad.extend_from_slice(&magic);
    bad.extend_from_slice(&0u32.to_le_bytes()); // jsonLen (well under MAX)
    bad.extend_from_slice(&0u32.to_le_bytes()); // binLen
    writer.write_all(&bad)?;
    writer.flush()
}

/// Read an environment flag as truthy (`1`).
fn env_flag(key: &str) -> bool {
    std::env::var(key).map(|v| v == "1").unwrap_or(false)
}

/// Read an environment variable as a millisecond duration. An unset, empty, or
/// unparsable value means "no delay" — a chaos knob must never make the stub die.
fn env_millis(key: &str) -> Option<std::time::Duration> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(std::time::Duration::from_millis)
}

/// Whether env var `key` equals `verb`.
fn env_matches(key: &str, verb: &str) -> bool {
    std::env::var(key).map(|v| v == verb).unwrap_or(false)
}

/// Whether `ONECAD_STUB_CRASH_ON_OP` names a (non-empty) substring of `op_id` — the
/// F3 hook that crashes the worker only on a specific plan's op.
fn crash_on_op_matches(op_id: &str) -> bool {
    std::env::var("ONECAD_STUB_CRASH_ON_OP")
        .ok()
        .filter(|v| !v.is_empty())
        .is_some_and(|v| op_id.contains(&v))
}
