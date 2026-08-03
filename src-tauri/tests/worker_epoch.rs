//! VF-B4 gate — epoch desync when the worker restarts with **no document open**.
//!
//! The restart hook (`state.rs`) can only bump the epoch of a runtime that already
//! exists; a crash on the start screen leaves nothing to notify. A [`DocumentRuntime`]
//! built afterwards must therefore seed its fencing cell from the engine's LIVE epoch,
//! or every plan it sends carries a stale one and the worker rejects it forever
//! (`PROTOCOL_ERROR: workerEpoch fencing mismatch`, SCHEMA §7.2 / D4).
//!
//! Driven over the `onecad-worker-stub`, whose D4 fencing mirrors the real worker's.
//! Gated on the stub binary existing (built by `cargo test --workspace`).

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::regen::{CancelToken, GeometryEngine, Outcome, RegenRequest};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::manager::{SupervisorConfig, WorkerState};
use onecad_lib::worker::{MeshProvider, SolverEngine, WorkerManager};

/// The built stub binary, derived from the test executable's target dir (mirrors
/// `worker_chaos.rs`). `None` ⇒ the crate was not built with `--workspace` ⇒ skip.
fn stub_binary() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?; // <target>/debug/deps/<test>
    let debug = exe.parent()?.parent()?;
    let bin = debug.join("onecad-worker-stub");
    bin.exists().then_some(bin)
}

/// Fast supervision so the restart drill runs in ~1 s. `auto_open_session` is ON —
/// that is what stamps the worker's session head with the manager's CURRENT epoch,
/// which is the whole point of this gate.
fn fast_config(binary: std::path::PathBuf) -> SupervisorConfig {
    SupervisorConfig {
        binary,
        envs: vec![],
        ping_interval: Duration::from_millis(100),
        ping_timeout: Duration::from_millis(500),
        max_missed_pings: 2,
        backoff: vec![Duration::from_millis(10), Duration::from_millis(20)],
        max_rapid_deaths: 3,
        healthy_threshold: Duration::from_millis(300),
        poison_threshold: 3,
        auto_open_session: true,
    }
}

fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

fn extrude_record(seed: u128, distance: f64) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: None,
        distance: Scalar::new(distance),
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
    OperationRecord::new(
        onecad_core::ids::RecordId(Uuid::from_u128(seed)),
        0,
        "Extrude",
        op,
    )
}

/// Kills the running worker and waits for the supervisor to bring a fresh one up
/// under a bumped epoch. Returns the new epoch.
async fn crash_and_wait_restart(wm: &WorkerManager) -> u64 {
    let before = wm.epoch().0;
    wm.shutdown().await;
    for _ in 0..200 {
        if wm.epoch().0 > before && wm.state() == WorkerState::Ready {
            return wm.epoch().0;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "worker did not restart (epoch still {before}, state {:?})",
        wm.state()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn document_opened_after_a_restart_fences_against_the_live_epoch() {
    let Some(bin) = stub_binary() else {
        eprintln!("skip: stub binary not built");
        return;
    };
    let wm = WorkerManager::spawn(fast_config(bin));
    assert!(wm.wait_ready(Duration::from_secs(3)).await, "worker ready");

    // The crash happens on the START SCREEN: no DocumentRuntime exists, so the restart
    // hook has nothing to notify and no cell to bump.
    let epoch = crash_and_wait_restart(&wm).await;
    assert!(epoch > 1, "the restart bumped the epoch, got {epoch}");

    // Only NOW is a document created. Its fencing cell must adopt the live epoch.
    let mut rt = runtime_over(&wm);
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(0x10, 25.0),
        at_cursor: true,
    })
    .expect("AddOperation");

    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    match report.outcome {
        Outcome::Published(snap) => assert_eq!(snap.bodies.len(), 1, "the extrude published"),
        other => panic!(
            "a document created after a worker restart must regen against the LIVE epoch \
             (worker head epoch {epoch}); got {other:?}"
        ),
    }
}
