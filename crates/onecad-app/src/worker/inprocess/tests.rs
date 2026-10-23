//! Real-kernel tests of the in-process backend (`embedded-kernel`), on macOS
//! against the `macosx` bundle and on the iOS Simulator against its own
//! (docs/ios/KERNEL.md). They need `ONECAD_EMBEDDED_WORKER_DIR`; without a
//! linked kernel they skip, unless `ONECAD_REQUIRE_WORKER=1` makes that fail.
//! Process-global counters (instances, sockets, threads) are compared, so the
//! tests run one at a time behind [`GATE`].

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::*;
use crate::worker::backend::{EmbeddedKernelConfig, WorkerRole};
use crate::worker::{SupervisorConfig, WorkerBackend, WorkerManager, WorkerState};

mod abandon;
mod failure;
mod plate;
mod recovery;
mod services;
mod solver;

static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Whether the kernel is linked; panics when it is required and absent.
/// `cargo test --workspace` unifies `embedded-kernel` through onecad-ios but
/// links the stub, so the sidecar's `ONECAD_REQUIRE_WORKER` does not require it.
fn kernel_linked() -> bool {
    if onecad_embedded_worker::is_linked() {
        return true;
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_EMBEDDED_KERNEL").as_deref() != Ok("1")
            && std::env::var("ONECAD_REQUIRE_EMBEDDED_FAULTS").as_deref() != Ok("1"),
        "embedded kernel required but not linked (set ONECAD_EMBEDDED_WORKER_DIR)"
    );
    eprintln!("skip: embedded kernel not linked (set ONECAD_EMBEDDED_WORKER_DIR)");
    false
}

/// The linked bundle's identity, parsed.
fn identity() -> Value {
    serde_json::from_str(onecad_embedded_worker::identity_json().expect("linked identity")).unwrap()
}

fn fingerprint() -> String {
    identity()["occt"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Production policy with fast liveness, so failure drills converge in seconds.
fn fast_config() -> SupervisorConfig {
    SupervisorConfig {
        ping_interval: Duration::from_millis(100),
        ping_timeout: Duration::from_millis(1500),
        backoff: vec![Duration::from_millis(10)],
        max_rapid_deaths: 1,
        healthy_threshold: Duration::from_millis(200),
        ..SupervisorConfig::production(std::path::PathBuf::new())
    }
}

fn embedded(journal_dir: Option<&Path>) -> WorkerBackend {
    WorkerBackend::Embedded(EmbeddedKernelConfig {
        journal_dir: journal_dir.map(Path::to_path_buf),
        ..EmbeddedKernelConfig::default()
    })
}

async fn spawn_ready(backend: &WorkerBackend, config: SupervisorConfig) -> WorkerManager {
    let wm = backend.spawn_with(config, WorkerRole::Document);
    assert!(
        wm.wait_ready(Duration::from_secs(20)).await,
        "in-process worker ready: {:?} {:?}",
        wm.state(),
        wm.failure_reason()
    );
    wm
}

async fn retire(wm: &WorkerManager) {
    wm.retire();
    assert!(wm.wait_torn_down(Duration::from_secs(5)).await, "torn down");
}

/// An abandonment drill's cleanup: once every run started since `before` has
/// returned, clear the process-wide relaunch latch so later tests can launch
/// (production never clears it). Drills run one at a time behind [`GATE`].
async fn clear_relaunch_latch(before: InProcessCounters) {
    assert!(
        eventually(Duration::from_secs(10), || in_process_counters().live
            == before.live)
        .await,
        "the abandoned run returned: {:?}",
        in_process_counters()
    );
    reset_relaunch_latch();
}

/// Waits until `cond` holds, polling; `false` on timeout.
async fn eventually(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while !cond() {
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}

/// Unix-domain sockets open in this process (the in-process transport's fds).
fn open_sockets() -> usize {
    let Ok(entries) = std::fs::read_dir("/dev/fd") else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter(|fd| {
            // SAFETY: `stat` is plain old data, written by fstat before any read;
            // a closed or foreign fd just makes fstat fail.
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: valid pointer to `stat`; `fd` is only queried.
            let ok = unsafe { libc::fstat(*fd, &mut stat) } == 0;
            ok && (stat.st_mode & libc::S_IFMT) == libc::S_IFSOCK
        })
        .count()
}

/// Queues a raw request (test hooks like `Debug.Busy`) on the live connection.
/// The frame is on the writer queue when this returns, so every request sent
/// after it follows it on the wire (and on the worker's FIFO kernel lane).
async fn start_raw(
    wm: &WorkerManager,
    verb: &str,
    args: Value,
) -> onecad_protocol::client::InflightRequest {
    let client = wm.client_or_err().expect("a live connection");
    client
        .start_request(verb, args, onecad_protocol::messages::Lane::Control)
        .await
        .expect("request queued")
}

/// The terminal of a raw request: `Err` for a worker error or a lost connection.
async fn finish_raw(inflight: onecad_protocol::client::InflightRequest) -> Result<Value, String> {
    let resp = inflight.response().await.map_err(|e| e.to_string())?;
    if resp.ok {
        Ok(resp.result.unwrap_or(Value::Null))
    } else {
        Err(format!("{:?}", resp.error))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_process_plate_round() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    let before = in_process_counters();
    let wm = spawn_ready(
        &embedded(None),
        SupervisorConfig::production(Default::default()),
    )
    .await;
    let hello = wm.hello().expect("hello");
    assert_eq!(
        hello.occt.fingerprint,
        fingerprint(),
        "hello matches the bundled identity"
    );
    let outcome = plate::plate_round(&wm).await;
    eprintln!(
        "in-process plate: {outcome:?} (identity platform {})",
        identity()["platform"]
    );
    plate::assert_plate(&outcome);
    retire(&wm).await;
    assert!(
        eventually(Duration::from_secs(5), || in_process_counters().live
            == before.live)
        .await,
        "the run thread returned after retirement"
    );
    let after = in_process_counters();
    assert_eq!(
        after.launched,
        before.launched + 1,
        "exactly one run thread"
    );
    assert_eq!(
        after.abandoned, before.abandoned,
        "a graceful exit abandons nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_stop_cycles_release_every_socket_and_thread() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    // One warm-up cycle: the runtime's own lazily created fds are not ours.
    retire(&spawn_ready(&embedded(None), fast_config()).await).await;
    let sockets = open_sockets();
    let before = in_process_counters();
    for _ in 0..8 {
        let wm = spawn_ready(&embedded(None), fast_config()).await;
        retire(&wm).await;
    }
    assert!(
        eventually(Duration::from_secs(5), || in_process_counters().live
            == before.live)
        .await,
        "every run thread returned: {:?}",
        in_process_counters()
    );
    assert_eq!(in_process_counters().launched, before.launched + 8);
    assert_eq!(in_process_counters().abandoned, before.abandoned);
    assert_eq!(
        open_sockets(),
        sockets,
        "no socket fd leaked across 8 start/stop cycles"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_restarts_under_a_new_epoch_and_stale_results_are_refused() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    let wm = spawn_ready(&embedded(None), fast_config()).await;
    let first = wm.epoch();
    let before = in_process_counters();
    // A clean in-process exit (Shutdown) is a death like any other: the
    // supervisor relaunches under a bumped epoch.
    wm.shutdown().await;
    assert!(
        eventually(Duration::from_secs(10), || wm.epoch() > first
            && wm.state() == WorkerState::Ready)
        .await,
        "relaunched under a new epoch: {:?}",
        wm.state()
    );
    assert_eq!(in_process_counters().launched, before.launched + 1);
    failure::assert_stale_epoch_refused(&wm, first).await;
    retire(&wm).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_one_auxiliary_instance_runs_at_a_time() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    let backend = embedded(None);
    let rebake = backend.spawn_with(fast_config(), WorkerRole::Rebake);
    assert!(
        rebake.wait_ready(Duration::from_secs(20)).await,
        "first auxiliary ready"
    );
    let before = in_process_counters();
    let ingest = backend.spawn_with(fast_config(), WorkerRole::Ingest);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        ingest.state(),
        WorkerState::Starting,
        "the second auxiliary waits for the cap"
    );
    assert_eq!(
        in_process_counters().launched,
        before.launched,
        "no thread while capped"
    );
    // Document workers draw from their own slots, not this one.
    let document = spawn_ready(&backend, fast_config()).await;
    // A retired supervisor stops waiting for a slot at once and starts nothing.
    let abandoned_wait = backend.spawn_with(fast_config(), WorkerRole::Rebake);
    tokio::time::sleep(Duration::from_millis(100)).await;
    abandoned_wait.retire();
    assert!(
        abandoned_wait.wait_torn_down(Duration::from_secs(1)).await,
        "a launch waiting for a slot observes retirement"
    );
    assert_eq!(in_process_counters().launched, before.launched + 1);
    retire(&rebake).await;
    assert!(
        ingest.wait_ready(Duration::from_secs(20)).await,
        "the waiting auxiliary starts once the first one is gone"
    );
    retire(&ingest).await;
    retire(&document).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn at_most_three_document_runs_live_at_once() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    let backend = embedded(None);
    let mut open = Vec::new();
    for _ in 0..3 {
        open.push(spawn_ready(&backend, fast_config()).await);
    }
    let before = in_process_counters();
    let fourth = backend.spawn_with(fast_config(), WorkerRole::Document);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        fourth.state(),
        WorkerState::Starting,
        "a fourth document run waits for a slot"
    );
    assert_eq!(
        in_process_counters().launched,
        before.launched,
        "no thread while capped"
    );
    retire(&open.remove(0)).await;
    assert!(
        fourth.wait_ready(Duration::from_secs(20)).await,
        "it starts once a document run returned"
    );
    retire(&fourth).await;
    for wm in &open {
        retire(wm).await;
    }
}
