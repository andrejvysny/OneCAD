//! Abandonment on RETIREMENT (closing a tab whose kernel is stuck): never
//! silent, and terminal for the whole process — no further run thread starts
//! and every in-process supervisor reports `relaunch-required`.

use super::failure::{has_test_hooks, wedge_config, BUSY_MS};
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retiring_a_stuck_run_abandons_it_once_and_stops_every_kernel() {
    let _gate = GATE.lock().await;
    if !kernel_linked() || !has_test_hooks() {
        return;
    }
    let before = in_process_counters();
    let sibling = spawn_ready(&embedded(None), fast_config()).await;
    // wedge_config: every op deadline is 300 ms, so is the retire deadline.
    let stuck = spawn_ready(&embedded(None), wedge_config()).await;
    let busy = start_raw(&stuck, "Debug.Busy", json!({ "durationMs": BUSY_MS })).await;
    // Retire before the 300 ms wedge deadline: the retire path must abandon it.
    tokio::time::sleep(Duration::from_millis(100)).await;
    stuck.retire();
    assert!(
        stuck.wait_torn_down(Duration::from_secs(5)).await,
        "retirement is bounded by the retire deadline"
    );
    assert_eq!(
        in_process_counters().abandoned,
        before.abandoned + 1,
        "abandoned once"
    );
    let latched = in_process_relaunch_required().expect("a retire abandonment latches");
    assert!(
        latched.contains("relaunch-required") && latched.contains("retired"),
        "{latched}"
    );
    // The healthy sibling is stopped and fails relaunch-required too.
    assert!(
        eventually(Duration::from_secs(5), || sibling.state()
            == WorkerState::Failed)
        .await
    );
    assert_eq!(sibling.failure_reason().as_deref(), Some(latched.as_str()));
    // No new run thread anywhere in the process.
    let next = embedded(None).spawn_with(fast_config(), WorkerRole::Document);
    assert!(
        eventually(Duration::from_secs(5), || next.state()
            == WorkerState::Failed)
        .await
    );
    assert_eq!(
        in_process_counters().launched,
        before.launched + 2,
        "no thread after the latch"
    );
    assert!(
        finish_raw(busy).await.is_err(),
        "its request ends with the socket"
    );
    clear_relaunch_latch(before).await;
    assert_eq!(
        in_process_counters().abandoned,
        before.abandoned + 1,
        "the idle sibling returned when stopped: only the stuck run was abandoned"
    );
    retire(&sibling).await;
    retire(&next).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retiring_during_a_long_op_waits_for_it_instead_of_abandoning() {
    let _gate = GATE.lock().await;
    if !kernel_linked() || !has_test_hooks() {
        return;
    }
    let before = in_process_counters();
    // Production deadlines: a 1.5 s op is not stuck, whatever RETIRE_GRACE says.
    let wm = spawn_ready(&embedded(None), fast_config()).await;
    let busy = start_raw(&wm, "Debug.Busy", json!({ "durationMs": 1500 })).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    wm.retire();
    assert!(
        wm.wait_torn_down(Duration::from_secs(10)).await,
        "the run returned once its op ended"
    );
    assert!(finish_raw(busy).await.is_err());
    let after = in_process_counters();
    assert_eq!(after.abandoned, before.abandoned, "nothing abandoned");
    assert_eq!(after.live, before.live, "the run thread returned");
    assert!(in_process_relaunch_required().is_none(), "no latch");
}
