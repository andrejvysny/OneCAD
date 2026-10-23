//! Failure semantics of the in-process backend: identity/ABI refusal, the
//! wedge → abandon-once → `relaunch-required` rule, no resubmission of an
//! accept whose completion is lost, epoch fencing, and parity of the error
//! taxonomy with the sidecar.

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::variables::Scalar;
use onecad_core::history::{DependencyGraph, Timeline};
use onecad_core::ids::{DocumentId, DocumentRevision, JobId, RecordId, WorkerEpoch};
use onecad_core::regen::{
    EngineError, Fencing, GeometryEngine, OpenSessionRequest, PlanArtifacts, PlanContext,
    PlanEvent, PlanRequest, PolicyVersions, RegenPlanner, RegenRequest, SessionMode,
};

use super::*;

/// A fresh job id. Wire job ids are u64 counters (`wire::job_id_wire`).
pub(super) fn next_job() -> JobId {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);
    JobId(Uuid::from_u128(u128::from(
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    )))
}

/// A one-op plan: a profile-less Blind extrude (a recoverable op failure on
/// every worker) whose crash-circuit key is fixed by `record`.
pub(super) fn smoke_plan(record: u128, epoch: WorkerEpoch) -> PlanRequest {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: None,
        distance: Scalar::new(10.0),
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
    let mut timeline = Timeline::new();
    timeline.insert_at_cursor(OperationRecord::new(
        RecordId(Uuid::from_u128(record)),
        0,
        "Extrude",
        op,
    ));
    let ctx = PlanContext {
        policy_versions: PolicyVersions::default(),
        occt_fingerprint: String::new(),
    };
    RegenPlanner::plan(
        &timeline,
        &DependencyGraph::new(),
        &[],
        RegenRequest::ToEnd { from: 0 },
        &ctx,
    )
    .into_request(
        next_job(),
        DocumentRevision(0),
        epoch,
        PolicyVersions::default(),
        PlanArtifacts::default(),
    )
}

/// The terminal of one plan, reduced to what both backends must agree on.
#[derive(Debug, PartialEq)]
pub(super) enum PlanEnd {
    Prepared {
        stopped: String,
        steps: Vec<(String, String)>,
    },
    Failed(String),
}

pub(super) async fn drive(wm: &WorkerManager, plan: PlanRequest) -> PlanEnd {
    let job = plan.job_id;
    let mut events = wm.execute_plan(plan).await;
    while let Some(event) = events.recv().await {
        match event {
            PlanEvent::Step(_) => {}
            PlanEvent::Prepared(p) => {
                // A worker holds one prepared job; leave the session free.
                let _ = wm.discard_prepared(job).await;
                let steps = p
                    .per_step
                    .iter()
                    .map(|s| (format!("{:?}", s.status), s.message.clone()))
                    .collect();
                return PlanEnd::Prepared {
                    stopped: format!("{:?}", p.stopped_reason),
                    steps,
                };
            }
            PlanEvent::Failed(error) => return PlanEnd::Failed(error_kind(&error)),
        }
    }
    PlanEnd::Failed("event stream closed without a terminal".into())
}

/// The error class plus its message (both backends must say the same thing).
fn error_kind(error: &EngineError) -> String {
    match error {
        EngineError::Protocol { message } => format!("protocol: {message}"),
        EngineError::OpFailed {
            code,
            recoverable,
            message,
            ..
        } => {
            format!("opFailed {code:?} recoverable={recoverable}: {message}")
        }
        other => format!("{other}"),
    }
}

/// A result fenced to `stale` is refused locally and by the worker.
pub(super) async fn assert_stale_epoch_refused(wm: &WorkerManager, stale: WorkerEpoch) {
    let open = wm
        .open_session(OpenSessionRequest {
            document_id: DocumentId::from_uuid(Uuid::nil()),
            document_revision: DocumentRevision(0),
            worker_epoch: stale,
            mode: SessionMode::Determinism,
        })
        .await;
    assert!(
        matches!(open, Err(EngineError::Protocol { .. })),
        "stale OpenSession: {open:?}"
    );
    let plan = drive(wm, smoke_plan(0x51, stale)).await;
    assert!(
        matches!(&plan, PlanEnd::Failed(kind) if kind.starts_with("protocol:")),
        "a plan fenced to the replaced epoch is refused: {plan:?}"
    );
    let accept = wm
        .accept_prepared(
            next_job(),
            Fencing {
                document_revision: DocumentRevision(0),
                worker_epoch: stale,
            },
        )
        .await;
    assert!(
        accept.is_err(),
        "an accept fenced to the replaced epoch is refused: {accept:?}"
    );
}

fn bad_identity(edit: impl FnOnce(&mut Value)) -> WorkerBackend {
    let mut identity = identity();
    edit(&mut identity);
    WorkerBackend::Embedded(EmbeddedKernelConfig {
        identity: Some(identity.to_string()),
        ..EmbeddedKernelConfig::default()
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_identity_or_abi_fails_startup_without_leaking_a_thread() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    let before = in_process_counters();
    // ABI: refused before any thread exists.
    let wm = bad_identity(|id| id["abiVersion"] = json!(4))
        .spawn_with(fast_config(), WorkerRole::Document);
    assert!(
        eventually(Duration::from_secs(10), || wm.state()
            == WorkerState::Failed)
        .await
    );
    let reason = wm.failure_reason().unwrap();
    assert!(reason.contains("ABI 4"), "{reason}");
    assert_eq!(
        in_process_counters().launched,
        before.launched,
        "no thread for a refused ABI"
    );
    // Fingerprint: refused at the hello; every started run returns.
    let wm = bad_identity(|id| id["occt"]["fingerprint"] = json!("0000000000000000"))
        .spawn_with(fast_config(), WorkerRole::Document);
    assert!(
        eventually(Duration::from_secs(10), || wm.state()
            == WorkerState::Failed)
        .await
    );
    let reason = wm.failure_reason().unwrap();
    assert!(reason.contains("occt.fingerprint mismatch"), "{reason}");
    // Platform: an identity for another Apple platform is refused before a thread.
    let launched = in_process_counters().launched;
    let wm = bad_identity(|id| id["platform"] = json!("iphoneos"))
        .spawn_with(fast_config(), WorkerRole::Document);
    assert!(
        eventually(Duration::from_secs(10), || wm.state()
            == WorkerState::Failed)
        .await
    );
    let reason = wm.failure_reason().unwrap();
    assert!(reason.contains("platform \"iphoneos\""), "{reason}");
    assert_eq!(in_process_counters().launched, launched);
    assert!(
        eventually(Duration::from_secs(5), || in_process_counters().live
            == before.live)
        .await,
        "refused runs returned: {:?}",
        in_process_counters()
    );
    assert_eq!(in_process_counters().abandoned, before.abandoned);
}

/// Whether the linked bundle answers `Debug.Busy` (built with --test-hooks).
pub(super) fn has_test_hooks() -> bool {
    let hooks = identity()["testHooks"] == json!(true);
    if !hooks {
        assert!(
            std::env::var("ONECAD_REQUIRE_EMBEDDED_FAULTS").as_deref() != Ok("1"),
            "ONECAD_REQUIRE_EMBEDDED_FAULTS=1 but the linked kernel has no test hooks"
        );
        eprintln!("skip: the linked bundle has no test hooks (build-ios-worker.sh --test-hooks)");
    }
    hooks
}

/// Policy for wedge drills: a 300 ms op deadline on every verb.
pub(super) fn wedge_config() -> SupervisorConfig {
    SupervisorConfig {
        wedge_deadline_execute_plan: Duration::from_millis(300),
        wedge_deadline_readers: Duration::from_millis(300),
        wedge_deadline_default: Duration::from_millis(300),
        ..fast_config()
    }
}

pub(super) const BUSY_MS: u64 = 4000;

fn os_threads() -> i32 {
    // SAFETY: plain-old-data out parameter sized for PROC_PIDTASKINFO.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = i32::try_from(std::mem::size_of::<libc::proc_taskinfo>()).unwrap();
    // SAFETY: our own pid, a valid buffer of `size` bytes.
    let n = unsafe {
        libc::proc_pidinfo(
            std::process::id() as i32,
            libc::PROC_PIDTASKINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            size,
        )
    };
    assert_eq!(n, size, "proc_pidinfo");
    info.pti_threadnum
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wedge_abandons_once_and_fails_relaunch_required_without_a_new_thread() {
    let _gate = GATE.lock().await;
    if !kernel_linked() || !has_test_hooks() {
        return;
    }
    let threads = os_threads();
    let before = in_process_counters();
    let wm = spawn_ready(&embedded(None), wedge_config()).await;
    let busy = start_raw(&wm, "Debug.Busy", json!({ "durationMs": BUSY_MS })).await;
    let started = Instant::now();
    assert!(
        eventually(Duration::from_secs(10), || wm.state()
            == WorkerState::Failed)
        .await
    );
    assert!(
        started.elapsed() < Duration::from_millis(BUSY_MS),
        "bounded: failed before the op ended"
    );
    let reason = wm.failure_reason().unwrap();
    assert!(
        reason.contains("relaunch-required") && reason.contains("Debug.Busy"),
        "{reason}"
    );
    assert!(
        finish_raw(busy).await.is_err(),
        "the wedged request ends with the connection"
    );
    let at_failure = in_process_counters();
    assert_eq!(
        at_failure.launched,
        before.launched + 1,
        "no replacement thread"
    );
    assert_eq!(
        at_failure.abandoned,
        before.abandoned + 1,
        "abandoned exactly once"
    );
    assert_eq!(
        at_failure.live,
        before.live + 1,
        "one abandoned run thread left"
    );
    // Evidence only: the OS count also moves with whatever other tests run in
    // parallel. Serially it is the run thread plus its still-spinning kernel lane.
    eprintln!("os thread delta at abandonment: {}", os_threads() - threads);
    // The whole process is relaunch-required: a new supervisor fails at once.
    let latched = in_process_relaunch_required().expect("the abandonment latched");
    assert!(latched.contains("relaunch-required"), "{latched}");
    let next = embedded(None).spawn_with(fast_config(), WorkerRole::Document);
    assert!(
        eventually(Duration::from_secs(5), || next.state()
            == WorkerState::Failed)
        .await
    );
    assert_eq!(next.failure_reason(), Some(latched));
    // When the spin ends the abandoned run returns; nothing restarts.
    assert!(
        eventually(Duration::from_secs(10), || in_process_counters().live
            == before.live)
        .await
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(in_process_counters().launched, before.launched + 1);
    assert_eq!(wm.state(), WorkerState::Failed);
    retire(&wm).await;
    retire(&next).await;
    clear_relaunch_latch(before).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_accept_whose_completion_is_lost_is_never_resubmitted() {
    let _gate = GATE.lock().await;
    if !kernel_linked() || !has_test_hooks() {
        return;
    }
    let before = in_process_counters();
    let wm = spawn_ready(&embedded(None), wedge_config()).await;
    let plan = smoke_plan(0xacc, wm.epoch());
    let job = plan.job_id;
    let mut events = wm.execute_plan(plan).await;
    while let Some(event) = events.recv().await {
        assert!(!matches!(event, PlanEvent::Failed(_)), "prepare: {event:?}");
    }
    // The kernel lane is held, so the AcceptPrepared queues behind it and its
    // completion never reaches this side: the instance is abandoned meanwhile.
    let busy = start_raw(&wm, "Debug.Busy", json!({ "durationMs": BUSY_MS })).await;
    let fencing = Fencing {
        document_revision: DocumentRevision(0),
        worker_epoch: wm.epoch(),
    };
    let accepted = wm.accept_prepared(job, fencing).await;
    assert!(
        accepted.is_err(),
        "the accept's completion was lost: {accepted:?}"
    );
    assert!(finish_raw(busy).await.is_err());
    assert!(
        eventually(Duration::from_secs(10), || wm.state()
            == WorkerState::Failed)
        .await
    );
    // No path can carry the accept again: no connection, no relaunch.
    let again = wm.accept_prepared(job, fencing).await;
    assert!(again.is_err(), "{again:?}");
    assert!(
        eventually(Duration::from_secs(10), || in_process_counters().live
            == before.live)
        .await
    );
    assert_eq!(
        in_process_counters().launched,
        before.launched + 1,
        "no worker to resubmit to"
    );
    retire(&wm).await;
    clear_relaunch_latch(before).await;
}

#[cfg(feature = "sidecar")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sidecar_and_in_process_agree_on_results_and_errors() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    let Some(binary) = crate::worker::resolve_worker_path() else {
        assert!(
            std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
            "sidecar required"
        );
        eprintln!("skip: no sidecar binary for the parity comparison (ONECAD_WORKER_PATH)");
        return;
    };
    let sidecar = WorkerBackend::Sidecar
        .spawn_with(SupervisorConfig::production(binary), WorkerRole::Document);
    assert!(
        sidecar.wait_ready(Duration::from_secs(20)).await,
        "sidecar ready"
    );
    let in_process = spawn_ready(
        &embedded(None),
        SupervisorConfig::production(Default::default()),
    )
    .await;
    let mut ends = Vec::new();
    for wm in [&sidecar, &in_process] {
        let outcome = plate::plate_round(wm).await;
        plate::assert_plate(&outcome);
        let invalid = drive(wm, smoke_plan(0x1bad, wm.epoch())).await;
        let fenced = drive(wm, smoke_plan(0x2bad, WorkerEpoch(wm.epoch().0 + 7))).await;
        ends.push((outcome, invalid, fenced));
    }
    eprintln!("sidecar: {:?}\nin-process: {:?}", ends[0], ends[1]);
    assert_eq!(
        ends[0], ends[1],
        "both backends give the same results and error semantics"
    );
    assert!(matches!(&ends[1].2, PlanEnd::Failed(kind) if kind.starts_with("protocol:")));
    retire(&sidecar).await;
    retire(&in_process).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bundle_without_test_hooks_does_not_answer_debug_busy() {
    let _gate = GATE.lock().await;
    if !kernel_linked() || identity()["testHooks"] == json!(true) {
        return; // the hooks bundle answers it by design
    }
    let wm = spawn_ready(&embedded(None), fast_config()).await;
    let busy = finish_raw(start_raw(&wm, "Debug.Busy", json!({ "durationMs": 0 })).await).await;
    let error = busy.expect_err("Debug.Busy is compiled out of a release kernel");
    assert!(
        error.contains("ProtocolError") && error.contains("unknown verb"),
        "{error}"
    );
    assert_eq!(
        wm.state(),
        WorkerState::Ready,
        "an unknown verb is not a death"
    );
    retire(&wm).await;
}
