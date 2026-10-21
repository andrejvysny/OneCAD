//! The `plan.preview` capability gate against an OLD worker (the Rust stub,
//! whose hello advertises no capabilities). The stub is told to `abort()` if a
//! `PreviewPlan` ever arrives, so "no frame was sent" is observable as "the
//! worker never died".

use onecad_app::worker::manager::WorkerState;

use super::super::*;

/// The built stub, derived from the test executable's target dir. `None` ⇒ it
/// was not built (`cargo build -p onecad-worker-stub`) ⇒ skip.
fn stub_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("onecad-worker-stub");
    bin.exists().then_some(bin)
}

fn blind_extrude(seed: u128, distance: f64) -> OperationRecord {
    let mut op = extrude_op(SKETCH_A, "", distance, None);
    if let Operation::Known(KnownOperation::Extrude(params)) = &mut op {
        params.profile = None;
    }
    extrude_record(seed, op)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_old_worker_is_unsupported_and_never_sent_the_verb() {
    let Some(stub) = stub_binary() else {
        eprintln!("skip: onecad-worker-stub not built");
        return;
    };
    let mut config = SupervisorConfig::production(stub);
    config.envs = vec![("ONECAD_STUB_CRASH_ON".into(), "PreviewPlan".into())];
    let wm = WorkerManager::spawn(config);
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "stub handshake"
    );
    let state = app_state_over(&wm);
    let (engine, meshes, solver) = state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    for record in [
        blind_extrude(EXTRUDE_A, 10.0),
        blind_extrude(EXTRUDE_C, 4.0),
    ] {
        rt.apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .expect("AddOperation");
    }
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{:?}",
        report.outcome
    );
    *state.runtime.lock().await = Some(rt);
    let epoch = wm.epoch();

    let result = preview(&state, EXTRUDE_A, blind_extrude(EXTRUDE_A, 5.0).op).await;
    assert!(
        matches!(result, Err(ApiError::Unsupported(ref m)) if m.contains("plan.preview")),
        "{result:?}"
    );
    // A sent frame would have aborted the stub: the supervisor would restart it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(wm.state(), WorkerState::Ready);
    assert_eq!(wm.epoch(), epoch, "the stub never received PreviewPlan");
}

/// A worker with `plan.preview` but not `op.boolean.tools` must refuse an
/// array-form Boolean preview as the ExecutePlan gate does, before the verb is
/// sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn array_form_boolean_preview_needs_boolean_tools_capability() {
    use onecad_app::worker::PreviewPlanRequest;
    use onecad_core::document::record::{BooleanOp, BooleanParams};
    use onecad_core::history::{DependencyGraph, Timeline};
    use onecad_core::ids::{DocumentRevision, JobId, SnapshotId};
    use onecad_core::regen::{
        EngineError, PlanArtifacts, PlanContext, PolicyVersions, RegenPlanner,
    };

    let Some(stub) = stub_binary() else {
        eprintln!("skip: onecad-worker-stub not built");
        return;
    };
    let mut config = SupervisorConfig::production(stub);
    config.envs = vec![
        ("ONECAD_STUB_CAPABILITIES".into(), "plan.preview".into()),
        ("ONECAD_STUB_CRASH_ON".into(), "PreviewPlan".into()),
    ];
    let wm = WorkerManager::spawn(config);
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "stub handshake"
    );
    let capabilities = wm.hello().expect("hello").capabilities;
    assert!(capabilities.iter().any(|c| c == "plan.preview"));
    assert!(!capabilities.iter().any(|c| c == "op.boolean.tools"));
    let epoch = wm.epoch();

    let boolean = Operation::Known(KnownOperation::Boolean(BooleanParams::new(
        BooleanOp::Cut,
        BodyId::new(),
        vec![BodyId::new(), BodyId::new()],
        false,
    )));
    let mut timeline = Timeline::new();
    timeline.insert_at_cursor(OperationRecord::new(
        RecordId(Uuid::from_u128(0x10)),
        0,
        "Boolean",
        boolean,
    ));
    let ctx = PlanContext {
        policy_versions: PolicyVersions::default(),
        occt_fingerprint: "0000000000000000".into(),
    };
    let plan = RegenPlanner::plan(
        &timeline,
        &DependencyGraph::new(),
        &[],
        RegenRequest::ToEnd { from: 0 },
        &ctx,
    )
    .into_request(
        JobId(Uuid::from_u128(1)),
        DocumentRevision(0),
        epoch,
        PolicyVersions::default(),
        PlanArtifacts { tessellate: None },
    );
    let request = PreviewPlanRequest {
        plan,
        expected_snapshot: SnapshotId(0),
        lod: Lod::Coarse,
    };

    let result = wm.preview_plan(request, CancelToken::new()).await;
    assert!(
        matches!(result, Err(EngineError::Protocol { ref message }) if message.contains("op.boolean.tools")),
        "{result:?}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(wm.state(), WorkerState::Ready);
    assert_eq!(wm.epoch(), epoch, "the stub never received PreviewPlan");
    wm.shutdown().await;
}
