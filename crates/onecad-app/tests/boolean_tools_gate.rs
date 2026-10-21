//! `op.boolean.tools` capability gate (SCHEMA §7.3 Boolean, ONECAD-362), driven
//! through the `WorkerManager` over the `onecad-worker-stub`, whose Hello
//! advertises no capabilities — i.e. an old worker.
//!
//! An array-form Boolean (several tools, or a retained tool) must fail as an
//! attributable `EngineError::Protocol` naming the capability, for both
//! `ExecutePlan` and `PreviewOp`, before anything is sent. A legacy single-tool
//! Boolean needs no capability and passes the gate.
//!
//! Gated on the stub binary existing (built by `cargo test --workspace`); a missing
//! binary skips cleanly, exactly like `worker_lifetime.rs`.

use std::path::PathBuf;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanOp, BooleanParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::history::{DependencyGraph, Timeline};
use onecad_core::ids::{BodyId, DocumentRevision, JobId, RecordId, WorkerEpoch};
use onecad_core::regen::{
    EngineError, GeometryEngine, Lod, PlanArtifacts, PlanContext, PlanEvent, PlanRequest,
    PolicyVersions, RegenPlanner, RegenRequest,
};

use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::{PreviewEngine, WorkerManager};

const CAPABILITY: &str = "op.boolean.tools";

fn stub_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?; // <target>/debug/deps/<test>
    let debug = exe.parent()?.parent()?;
    let bin = debug.join("onecad-worker-stub");
    bin.exists().then_some(bin)
}

async fn stub_manager() -> Option<WorkerManager> {
    let Some(bin) = stub_binary() else {
        eprintln!("skip: stub binary not built");
        return None;
    };
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(wm.wait_ready(Duration::from_secs(5)).await, "stub ready");
    let hello = wm.hello().expect("hello");
    assert!(
        !hello.capabilities.iter().any(|c| c == CAPABILITY),
        "the stub stands in for a worker without {CAPABILITY}"
    );
    Some(wm)
}

fn boolean(tools: usize, keep: bool) -> Operation {
    let tools = (0..tools).map(|_| BodyId::new()).collect();
    Operation::Known(KnownOperation::Boolean(BooleanParams::new(
        BooleanOp::Cut,
        BodyId::new(),
        tools,
        keep,
    )))
}

fn plan_with(op: Operation) -> PlanRequest {
    let mut tl = Timeline::new();
    tl.insert_at_cursor(OperationRecord::new(
        RecordId(Uuid::from_u128(0x10)),
        0,
        "Boolean",
        op,
    ));
    let ctx = PlanContext {
        policy_versions: PolicyVersions::default(),
        occt_fingerprint: "0000000000000000".into(),
    };
    RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &[],
        RegenRequest::ToEnd { from: 0 },
        &ctx,
    )
    .into_request(
        JobId(Uuid::from_u128(1)),
        DocumentRevision(0),
        WorkerEpoch(1),
        PolicyVersions::default(),
        PlanArtifacts { tessellate: None },
    )
}

/// The terminal event of a plan, skipping per-step results.
async fn terminal(wm: &WorkerManager, request: PlanRequest) -> PlanEvent {
    let mut rx = wm.execute_plan(request).await;
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(PlanEvent::Step(_))) => continue,
            Ok(Some(event)) => return event,
            Ok(None) => panic!("plan channel closed without a terminal"),
            Err(_) => panic!("no terminal within 10 s"),
        }
    }
}

fn assert_gate_error(error: &EngineError, what: &str) {
    match error {
        EngineError::Protocol { message } => {
            assert!(message.contains(CAPABILITY), "{what}: {message}")
        }
        other => panic!("{what}: expected a protocol error naming {CAPABILITY}, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn array_form_plan_and_preview_refuse_without_capability() {
    let Some(wm) = stub_manager().await else {
        return;
    };
    for (tools, keep) in [(2, false), (1, true), (3, true)] {
        let what = format!("tools={tools} keep={keep}");
        match terminal(&wm, plan_with(boolean(tools, keep))).await {
            PlanEvent::Failed(error) => assert_gate_error(&error, &format!("plan {what}")),
            other => panic!("plan {what}: expected Failed, got {other:?}"),
        }
        let error = wm
            .preview_op(
                boolean(tools, keep),
                "op_preview".into(),
                None,
                None,
                Lod::Coarse,
            )
            .await
            .expect_err("preview refuses");
        assert_gate_error(&error, &format!("preview {what}"));
    }

    // The legacy form needs no capability: whatever the stub answers, it is not
    // the gate's refusal.
    if let PlanEvent::Failed(EngineError::Protocol { message }) =
        terminal(&wm, plan_with(boolean(1, false))).await
    {
        assert!(
            !message.contains(CAPABILITY),
            "legacy plan gated: {message}"
        );
    }
    if let Err(EngineError::Protocol { message }) = wm
        .preview_op(
            boolean(1, false),
            "op_legacy".into(),
            None,
            None,
            Lod::Coarse,
        )
        .await
    {
        assert!(
            !message.contains(CAPABILITY),
            "legacy preview gated: {message}"
        );
    }
    wm.shutdown().await;
}
