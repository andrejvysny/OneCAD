//! Controlled ordinary solver faults, distinct from physical crash qualification.
use super::failure::{has_test_hooks, wedge_config, BUSY_MS};
use super::*;
use crate::worker::SolverEngine;
use onecad_core::ids::SketchId;
use onecad_core::sketch::{Sketch, WorldPlane};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_upsert_and_regions_wedges_abandon_once_despite_healthy_pings() {
    let _gate = GATE.lock().await;
    if !kernel_linked() || !has_test_hooks() {
        return;
    }
    for verb in ["SketchUpsert", "SketchRegions"] {
        let before = in_process_counters();
        let journal = tempfile::tempdir().unwrap();
        let wm = spawn_ready(&embedded(Some(journal.path())), wedge_config()).await;
        let stale = wm.epoch();
        let busy = start_raw(
            &wm,
            verb,
            json!({
                "__testSolverDelayMs": BUSY_MS, "__testSolverIgnoreCancel": true
            }),
        )
        .await;
        let client = wm.client_or_err().unwrap();
        let head = client.request("GetWorkerHead", json!({})).await.unwrap();
        assert!(head.ok, "status remains responsive during {verb}");
        assert!(eventually(Duration::from_secs(5), || wm.state() == WorkerState::Failed).await);
        let reason = wm.failure_reason().unwrap();
        assert!(
            reason.contains("relaunch-required") && reason.contains(verb),
            "{reason}"
        );
        assert!(finish_raw(busy).await.is_err());
        assert_eq!(in_process_counters().launched, before.launched + 1);
        assert_eq!(in_process_counters().abandoned, before.abandoned + 1);
        assert_ne!(
            wm.epoch(),
            stale,
            "terminal abandonment fences the old generation"
        );
        assert!(
            wm.client_or_err().is_err(),
            "no dispatch after terminal abandonment"
        );
        let next = embedded(None).spawn_with(fast_config(), WorkerRole::Document);
        assert!(
            eventually(Duration::from_secs(2), || next.state()
                == WorkerState::Failed)
            .await
        );
        assert_eq!(in_process_counters().launched, before.launched + 1);
        retire(&wm).await;
        retire(&next).await;
        clear_relaunch_latch(before).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_solver_completion_and_cancel_leave_no_journal_or_replacement() {
    let _gate = GATE.lock().await;
    if !kernel_linked() || !has_test_hooks() {
        return;
    }
    let before = in_process_counters();
    let journal = tempfile::tempdir().unwrap();
    let wm = spawn_ready(&embedded(Some(journal.path())), fast_config()).await;
    let sketch = Sketch::on_world_plane(SketchId(uuid::Uuid::new_v4()), "Solver", WorldPlane::XY);
    wm.sketch_upsert(&sketch).await.unwrap();
    for verb in ["SketchUpsert", "SketchRegions"] {
        let mut args = if verb == "SketchUpsert" {
            crate::worker::wire::sketch_upsert_args(&sketch)
        } else {
            crate::worker::wire::sketch_regions_args(&sketch.id.to_string())
        };
        args["__testSolverDelayMs"] = json!(20);
        let completed = finish_raw(start_raw(&wm, verb, args).await).await;
        assert!(completed.is_ok(), "{verb}: {completed:?}");
        let client = wm.client_or_err().unwrap();
        let pending = start_raw(&wm, verb, json!({"__testSolverDelayMs":BUSY_MS})).await;
        client.cancel(pending.id).await.unwrap();
        let cancelled = tokio::time::timeout(Duration::from_secs(2), finish_raw(pending))
            .await
            .unwrap();
        assert!(cancelled.unwrap_err().contains("Cancelled"));
        assert_eq!(wm.state(), WorkerState::Ready);
    }
    retire(&wm).await;
    assert_eq!(in_process_counters().launched, before.launched + 1);
    assert_eq!(in_process_counters().abandoned, before.abandoned);
    assert!(!journal.path().join("inflight.json").exists());
}

#[cfg(feature = "sidecar")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sidecar_solver_wedges_restart_once_without_reissuing_request() {
    let _gate = GATE.lock().await;
    let Some(binary) = crate::worker::resolve_worker_path() else {
        assert!(
            std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
            "sidecar required"
        );
        return;
    };
    for verb in ["SketchUpsert", "SketchRegions"] {
        let mut config = wedge_config();
        config.binary = binary.clone();
        let wm = WorkerManager::spawn(config);
        assert!(wm.wait_ready(Duration::from_secs(10)).await);
        let epoch = wm.epoch();
        let busy = start_raw(
            &wm,
            verb,
            json!({
                "__testSolverDelayMs": BUSY_MS, "__testSolverIgnoreCancel":true
            }),
        )
        .await;
        let client = wm.client_or_err().unwrap();
        assert!(client.request("GetWorkerHead", json!({})).await.unwrap().ok);
        assert!(eventually(Duration::from_secs(5), || wm.epoch() != epoch).await);
        assert!(finish_raw(busy).await.is_err());
        assert!(wm.wait_ready(Duration::from_secs(5)).await);
        assert_eq!(wm.epoch().0, epoch.0 + 1);
        retire(&wm).await;
    }
}
