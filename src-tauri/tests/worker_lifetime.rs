//! `WorkerManager::retire` — the terminal lifecycle state, driven over the built
//! `onecad-worker-stub`.
//!
//! Before `retire` existed, `WorkerManager::shutdown` had **zero callers** and the
//! supervisor's only exits were `Failed` and "restart forever": every document open
//! spawned a sidecar and orphaned the previous one for the lifetime of the app. The
//! contract asserted here is what makes retirement safe to call from that path:
//!
//! * it is observable in **every** supervisor state — mid-handshake, in the
//!   ping/exit run loop, and inside a backoff sleep — so a retired manager never
//!   reconnects and never restarts;
//! * it emits **no further lifecycle transition** and does **not** bump the epoch
//!   (retirement is not a restart; a fence must never mistake it for one);
//! * it is terminal and idempotent, and every subsequent request fails FAST rather
//!   than waiting for a readiness that will never come.
//!
//! All drills gate on the stub binary existing (built by `cargo test --workspace`);
//! a missing binary skips cleanly, exactly like `worker_chaos.rs`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::variables::Scalar;
use onecad_core::history::{DependencyGraph, Timeline};
use onecad_core::ids::{DocumentRevision, JobId, RecordId, WorkerEpoch};
use onecad_core::regen::{
    GeometryEngine, Lod, PlanArtifacts, PlanContext, PlanEvent, PlanRequest, PolicyVersions,
    RegenPlan, RegenPlanner, RegenRequest, TessellateSpec,
};

use onecad_lib::worker::manager::{SupervisorConfig, WorkerLifecycle, WorkerState};
use onecad_lib::worker::WorkerManager;

// ─────────────────────────────────────────────────────────────────────────────
// Harness
// ─────────────────────────────────────────────────────────────────────────────

/// The built stub binary, derived from the test executable's target dir. `None`
/// ⇒ the crate was not built with `--workspace` ⇒ skip.
fn stub_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?; // <target>/debug/deps/<test>
    let deps = exe.parent()?;
    let debug = deps.parent()?;
    let bin = debug.join("onecad-worker-stub");
    bin.exists().then_some(bin)
}

/// A fast supervision policy (mirrors `worker_chaos::fast_config`): a generous ping
/// timeout so a healthy worker is never SIGKILLed under parallel-test CPU load.
fn fast_config(binary: PathBuf, envs: Vec<(String, String)>) -> SupervisorConfig {
    SupervisorConfig {
        binary,
        envs,
        ping_interval: Duration::from_millis(100),
        ping_timeout: Duration::from_millis(500),
        max_missed_pings: 2,
        backoff: vec![
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(30),
        ],
        max_rapid_deaths: 3,
        healthy_threshold: Duration::from_millis(300),
        poison_threshold: 3,
        auto_open_session: false,
    }
}

fn one_extrude_plan(epoch: u64) -> PlanRequest {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: None,
        distance: Scalar::new(25.0),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }));
    let mut tl = Timeline::new();
    tl.insert_at_cursor(OperationRecord::new(
        RecordId(Uuid::from_u128(0x10)),
        0,
        "Extrude",
        op,
    ));
    let ctx = PlanContext {
        policy_versions: PolicyVersions::default(),
        occt_fingerprint: "0000000000000000".into(),
    };
    let plan: RegenPlan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &[],
        RegenRequest::ToEnd { from: 0 },
        &ctx,
    );
    plan.into_request(
        JobId(Uuid::from_u128(1)),
        DocumentRevision(0),
        WorkerEpoch(epoch),
        PolicyVersions::default(),
        PlanArtifacts {
            tessellate: Some(TessellateSpec {
                lod: Lod::Coarse,
                include_edges: true,
            }),
        },
    )
}

/// Drains every lifecycle transition currently queued (pre-retire noise).
fn drain(rx: &mut tokio::sync::broadcast::Receiver<WorkerLifecycle>) {
    while rx.try_recv().is_ok() {}
}

/// Collects every lifecycle transition emitted over `window`. A retired supervisor
/// must produce an EMPTY vector — that is the "no further transitions" assertion.
async fn collect_for(
    rx: &mut tokio::sync::broadcast::Receiver<WorkerLifecycle>,
    window: Duration,
) -> Vec<String> {
    let deadline = Instant::now() + window;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(25), rx.recv()).await {
            Ok(Ok(ev)) => seen.push(format!("{ev:?}")),
            Ok(Err(_)) => break, // sender gone
            Err(_) => {}         // idle tick
        }
    }
    seen
}

/// Drives one `ExecutePlan`, returning how long the terminal event took. A retired
/// manager must resolve this without ever touching the wire.
async fn time_plan(wm: &WorkerManager, plan: PlanRequest) -> (Duration, Option<PlanEvent>) {
    let started = Instant::now();
    let mut rx = wm.execute_plan(plan).await;
    let mut terminal = None;
    while let Some(ev) = rx.recv().await {
        if !matches!(ev, PlanEvent::Step(_)) {
            terminal = Some(ev);
        }
    }
    (started.elapsed(), terminal)
}

// ─────────────────────────────────────────────────────────────────────────────
// Retire while Ready — the document-open path
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retire_while_ready_is_terminal_idempotent_and_silent() {
    let Some(bin) = stub_binary() else {
        eprintln!("skip: stub binary not built");
        return;
    };
    let wm = WorkerManager::spawn(fast_config(bin, vec![]));
    assert!(wm.wait_ready(Duration::from_secs(3)).await, "worker ready");
    let epoch_before = wm.epoch().0;
    let mut life = wm.subscribe();
    drain(&mut life);

    wm.retire();
    assert_eq!(wm.state(), WorkerState::Retired, "retire is immediate");
    assert!(wm.is_retired());
    assert!(
        wm.wait_torn_down(Duration::from_secs(2)).await,
        "the supervision task must END, with the child reaped — a latched flag alone \
         would leave the sidecar running exactly like the leak this replaces"
    );

    // The stub honours `Shutdown` and exits; the supervisor must treat that exit as
    // part of the retirement, NOT as a death to restart from. Well past both the
    // graceful window and a full backoff cycle:
    let seen = collect_for(&mut life, Duration::from_millis(900)).await;
    assert!(
        seen.is_empty(),
        "a retired supervisor emits no further lifecycle transitions, saw {seen:?}"
    );
    assert_eq!(
        wm.epoch().0,
        epoch_before,
        "retirement is not a restart — the epoch must NOT bump (fencing)"
    );
    assert_eq!(wm.state(), WorkerState::Retired, "Retired is sticky");
    assert!(
        !wm.wait_ready(Duration::from_millis(50)).await,
        "a retired manager never becomes Ready again"
    );

    // Idempotent.
    wm.retire();
    wm.retire();
    assert_eq!(wm.state(), WorkerState::Retired);
    assert_eq!(wm.epoch().0, epoch_before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retired_manager_fails_plans_fast() {
    let Some(bin) = stub_binary() else {
        return;
    };
    let wm = WorkerManager::spawn(fast_config(bin, vec![]));
    assert!(wm.wait_ready(Duration::from_secs(3)).await);
    wm.retire();

    let (elapsed, terminal) = time_plan(&wm, one_extrude_plan(1)).await;
    match terminal {
        Some(PlanEvent::Failed(err)) => {
            let message = err.to_string();
            assert!(
                message.contains("retired"),
                "the terminal must say WHY (retired, not a transient disconnect): {message}"
            );
        }
        other => panic!("a retired manager must fail the plan, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_millis(200),
        "fail-fast: no dispatch and no readiness wait, took {elapsed:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Retire mid-handshake — the connect path
// ─────────────────────────────────────────────────────────────────────────────

/// `/bin/cat` is a worker that accepts the pipes and never sends its OCW1 hello, so
/// the supervisor parks in `ProtocolClient::connect` indefinitely. Without a
/// cancellation arm on that await, `retire` could not be observed at all until the
/// handshake resolved — which for a wedged sidecar is never.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retire_during_the_handshake_never_connects_or_restarts() {
    let cat = Path::new("/bin/cat");
    if !cat.exists() {
        eprintln!("skip: /bin/cat not present");
        return;
    }
    let wm = WorkerManager::spawn(fast_config(cat.to_path_buf(), vec![]));
    let mut life = wm.subscribe();
    // Let the supervisor get into the handshake await.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        wm.state(),
        WorkerState::Starting,
        "a hello-less worker parks the supervisor in the handshake"
    );
    drain(&mut life);

    wm.retire();
    assert!(
        wm.wait_torn_down(Duration::from_secs(2)).await,
        "the handshake await must be CANCELLED — without a retire arm on it the \
         supervisor parks forever holding a live child"
    );
    let seen = collect_for(&mut life, Duration::from_millis(600)).await;
    assert!(
        seen.is_empty(),
        "retiring mid-handshake must not announce a restart or a failure, saw {seen:?}"
    );
    assert_eq!(wm.state(), WorkerState::Retired);
    assert_eq!(wm.epoch().0, 1, "no epoch bump on the connect path");
}

// ─────────────────────────────────────────────────────────────────────────────
// Retire during a backoff sleep — the reconnect path
// ─────────────────────────────────────────────────────────────────────────────

/// The budget below reaches `Failed` on the SECOND failed start, ~200 ms in. Retiring
/// at ~50 ms must cancel the sleeping supervisor outright: if the retire flag were
/// only checked between sleeps, `Failed` would still be emitted here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retire_during_backoff_cancels_the_pending_reconnect() {
    let mut config = fast_config(PathBuf::from("/nonexistent/onecad-worker"), vec![]);
    config.backoff = vec![Duration::from_millis(200)];
    config.max_rapid_deaths = 1;
    let wm = WorkerManager::spawn(config);

    // Wait for the first failed start to put the supervisor into its backoff sleep.
    let mut restarting = false;
    for _ in 0..100 {
        if wm.state() == WorkerState::Restarting {
            restarting = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(restarting, "a missing binary fails its first start");
    let mut life = wm.subscribe();
    drain(&mut life);

    wm.retire();
    assert!(
        wm.wait_torn_down(Duration::from_millis(150)).await,
        "the backoff sleep must wake on retirement, well inside its 200 ms delay"
    );
    let seen = collect_for(&mut life, Duration::from_millis(600)).await;
    assert!(
        seen.is_empty(),
        "the pending reconnect must be cancelled, not merely deferred — saw {seen:?}"
    );
    assert_eq!(
        wm.state(),
        WorkerState::Retired,
        "and the manager must NOT have fallen through to Failed"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Retire with a ping outstanding — the run-loop path
// ─────────────────────────────────────────────────────────────────────────────

/// `HANG_ON GetWorkerHead` wedges the stub inside the liveness ping and, because the
/// stub is single-threaded, its `Shutdown` frame is never answered either. So this
/// drill covers both halves of the run-loop contract: the select arm must win against
/// an in-flight ping, and the graceful window must fall through to a force-kill. A
/// generous `ping_timeout` keeps the supervisor's own SIGKILL out of the picture, so
/// what completes the teardown can only be the retirement.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retire_with_a_hung_ping_outstanding_force_kills_the_child() {
    let Some(bin) = stub_binary() else {
        return;
    };
    let mut config = fast_config(
        bin,
        vec![("ONECAD_STUB_HANG_ON".into(), "GetWorkerHead".into())],
    );
    config.ping_interval = Duration::from_millis(50);
    config.ping_timeout = Duration::from_secs(30);
    let wm = WorkerManager::spawn(config);
    assert!(wm.wait_ready(Duration::from_secs(3)).await);
    let epoch_before = wm.epoch().0;
    let mut life = wm.subscribe();
    // Give the ticker time to fire a ping that will now hang forever.
    tokio::time::sleep(Duration::from_millis(200)).await;
    drain(&mut life);

    wm.retire();
    assert!(
        wm.wait_torn_down(Duration::from_secs(3)).await,
        "the graceful window must fall through to SIGKILL — the stub can never answer \
         Shutdown while it is wedged in the ping"
    );
    // Longer than RETIRE_GRACE (500 ms) so the force-kill has landed.
    let seen = collect_for(&mut life, Duration::from_millis(1200)).await;
    assert!(
        seen.is_empty(),
        "an unanswerable worker must be reaped silently, saw {seen:?}"
    );
    assert_eq!(
        wm.epoch().0,
        epoch_before,
        "the forced kill of a retired worker is not a death to fence against"
    );
    assert_eq!(wm.state(), WorkerState::Retired);
}
