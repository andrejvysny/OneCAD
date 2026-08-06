//! The **cold-worker regen race** — regression pins for the bug where opening a
//! project rendered nothing and reported "Geometry rebuild failed: worker crashed:
//! worker not connected".
//!
//! Shape of the original defect: `open_document` enqueued its `ToEnd { from: 0 }`
//! regen the instant the command returned, which could beat the freshly spawned
//! sidecar's OCW1 handshake. `WorkerManager::execute_plan` fails FAST when there is
//! no connection (by design — a caller must never sit on a readiness wait it did not
//! ask for), and a from-0 plan has no retry path, so the whole document's first
//! rebuild died on a worker that was merely still starting.
//!
//! The fix is at the API layer: `api::spawn_gated_regen` awaits
//! [`WorkerReadiness::wait_ready`] before firing that first request (unit-pinned in
//! `src/api/mod.rs`'s `readiness_gate` tests). These drills pin the two halves of
//! that contract against a REAL sidecar process whose handshake is deliberately slow
//! (`ONECAD_STUB_HELLO_DELAY_MS`):
//!
//! * the manager still fails fast, with the honest [`EngineError::NotConnected`]
//!   (never `Crashed`) — the API-layer gate is the load-bearing fix, not a change of
//!   manager semantics;
//! * gating on readiness first makes the same plan, over the same slow worker,
//!   publish.
//!
//! **These cannot be red-first** — the gate already landed. They are regression pins:
//! they fail if the readiness seam is removed or if the taxonomy is re-conflated.
//!
//! Gates on the stub binary existing (built by `cargo test --workspace`); a missing
//! binary skips cleanly, exactly like `worker_chaos.rs`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::variables::Scalar;
use onecad_core::history::{DependencyGraph, Timeline};
use onecad_core::ids::{DocumentRevision, JobId, RecordId, WorkerEpoch};
use onecad_core::regen::{
    CancelToken, EngineError, GeometryEngine, Lod, Outcome, PlanArtifacts, PlanContext, PlanEvent,
    PlanRequest, PolicyVersions, RegenExecutor, RegenPlan, RegenPlanner, RegenRequest,
    RegenSession, SnapshotPublisher, TessellateSpec,
};

use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{AdoptingEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors `worker_chaos.rs` / `worker_lifetime.rs`)
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

/// How long the stub withholds its `hello`. Long enough that the "dispatch before
/// the handshake" window cannot be missed even on a loaded CI box, short enough that
/// the gated drill pays it once.
const HELLO_DELAY: Duration = Duration::from_millis(300);

/// A supervision policy with a generous ping timeout (a healthy worker must never be
/// SIGKILLed under parallel-test CPU load) and the slow handshake wired in.
fn slow_hello_config(binary: PathBuf, hello_delay: Duration) -> SupervisorConfig {
    SupervisorConfig {
        binary,
        envs: vec![(
            "ONECAD_STUB_HELLO_DELAY_MS".into(),
            hello_delay.as_millis().to_string(),
        )],
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

fn one_extrude_timeline() -> Timeline {
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
    tl
}

/// The document's FIRST regen exactly as `open_document` schedules it:
/// `ToEnd { from: 0 }`, no checkpoint, revision 0.
fn from_zero_plan(tl: &Timeline, epoch: u64) -> PlanRequest {
    let ctx = PlanContext {
        policy_versions: PolicyVersions::default(),
        occt_fingerprint: "0000000000000000".into(),
    };
    let plan: RegenPlan = RegenPlanner::plan(
        tl,
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

/// Drives one plan through the executor (atomic prepare/accept) over the WM —
/// exactly the app's regen path.
async fn drive(wm: &WorkerManager, tl: &Timeline, epoch: u64) -> Outcome {
    let plan = from_zero_plan(tl, epoch);
    let engine = AdoptingEngine::new(Arc::new(wm.clone()), HashSet::new());
    let executor = RegenExecutor::new(engine);
    let mut session = RegenSession {
        timeline: tl.clone(),
        ..Default::default()
    };
    let publisher = SnapshotPublisher::new();
    let cancel = CancelToken::new();
    let gate = move || (DocumentRevision(0), WorkerEpoch(epoch));
    executor
        .run(plan, &mut session, &gate, &cancel, &publisher)
        .await
}

// ─────────────────────────────────────────────────────────────────────────────
// The bug shape: dispatch before the handshake
// ─────────────────────────────────────────────────────────────────────────────

/// The manager's UNGATED behavior, pinned deliberately: a plan dispatched before
/// the handshake fails immediately rather than blocking on a readiness the caller
/// never asked to wait for.
///
/// The regression this guards is the REPORT, not the timing: the terminal is
/// [`EngineError::NotConnected`], so nothing downstream can tell the user a worker
/// that had not connected yet "crashed".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ungated_plan_dispatched_before_the_handshake_fails_fast_as_not_connected() {
    let Some(bin) = stub_binary() else {
        eprintln!("skip: stub binary not built");
        return;
    };
    // A full second of silence: the dispatch below cannot accidentally land on a
    // connected worker, even if this task is descheduled for a while first.
    let wm = WorkerManager::spawn(slow_hello_config(bin, Duration::from_millis(1000)));

    let started = Instant::now();
    let mut rx = wm
        .execute_plan(from_zero_plan(&one_extrude_timeline(), 1))
        .await;
    let mut terminal = None;
    while let Some(ev) = rx.recv().await {
        if !matches!(ev, PlanEvent::Step(_)) {
            terminal = Some(ev);
        }
    }
    let elapsed = started.elapsed();

    match terminal {
        Some(PlanEvent::Failed(EngineError::NotConnected { message })) => {
            let rendered = EngineError::NotConnected { message }.to_string();
            assert!(
                !rendered.contains("crash"),
                "THE mislabel: a never-connected worker must not report a crash: {rendered}"
            );
        }
        other => panic!(
            "a plan dispatched before the handshake must fail as NotConnected, got {other:?}"
        ),
    }
    assert!(
        elapsed < Duration::from_millis(300),
        "fail-fast: no wire traffic and no readiness wait, took {elapsed:?}"
    );
    wm.retire();
}

/// The fix: wait on readiness FIRST (what `api::spawn_gated_regen` does), and the
/// very same from-0 plan over the very same slow worker publishes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_readiness_gate_lets_the_first_regen_publish_over_a_slow_worker() {
    let Some(bin) = stub_binary() else {
        eprintln!("skip: stub binary not built");
        return;
    };
    let wm = WorkerManager::spawn(slow_hello_config(bin, HELLO_DELAY));

    // The gate, verbatim: `spawn_gated_regen` awaits exactly this before requesting
    // `ToEnd { from: 0 }`.
    let started = Instant::now();
    assert!(
        wm.wait_ready(Duration::from_secs(8)).await,
        "the slow worker must still become ready inside the production window"
    );
    let waited = started.elapsed();
    assert!(
        waited >= HELLO_DELAY / 2,
        "the gate must have actually waited on the delayed handshake, waited {waited:?} \
         — a ~0 wait means the stub ignored ONECAD_STUB_HELLO_DELAY_MS and this drill \
         is no longer exercising the race (a stale stub binary does exactly that: \
         `--test worker_connect_race` alone does NOT rebuild it, run \
         `cargo build -p onecad-worker-stub` or `cargo test --workspace`)"
    );

    let tl = one_extrude_timeline();
    match drive(&wm, &tl, 1).await {
        Outcome::Published(snap) => assert_eq!(
            snap.bodies.len(),
            1,
            "the document's first regen publishes its body"
        ),
        other => panic!("the gated first regen must publish, got {other:?}"),
    }
    wm.retire();
}
