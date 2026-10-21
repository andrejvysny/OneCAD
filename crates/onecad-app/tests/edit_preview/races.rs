//! Cancellation, preemption, supersession and worker death. Each test pins a
//! WORKER-side effect (the kernel lane frees, the commit does not queue behind
//! the replay) — a Rust-side fence alone would pass with a broken cancel path.

use std::time::Instant;

use onecad_core::regen::WorkerHead;

use super::super::*;

/// Twenty fused bosses: long enough that a cancel lands mid-replay.
fn heavy_part() -> Vec<OperationRecord> {
    let mut records = vec![
        sketch_record(SKETCH_A, 0.0, 0.0, 40.0, 40.0),
        extrude_record(EXTRUDE_A, extrude_op(SKETCH_A, "", 10.0, None)),
    ];
    for i in 0..20u128 {
        let sketch = 0x700 + 2 * i;
        let x = 1.0 + (i % 5) as f64 * 7.5;
        let y = 1.0 + (i / 5) as f64 * 9.0;
        records.push(sketch_record(sketch, x, y, 5.0, 5.0));
        records.push(extrude_record(
            sketch + 1,
            extrude_op(sketch, "", 12.0 + i as f64, Some(EXTRUDE_A)),
        ));
    }
    records
}

fn depth(distance: f64) -> Operation {
    extrude_op(SKETCH_A, "", distance, None)
}

/// The wall time of one uncancelled preview of the heavy part. It must be long
/// enough that "mid-flight" below is really mid-flight.
async fn full_replay_time(state: &AppState) -> Duration {
    let started = Instant::now();
    let calibration = preview(state, EXTRUDE_A, depth(5.0)).await;
    assert!(
        matches!(calibration, Ok(ref p) if p.outcome == EditPreviewOutcome::Ready),
        "{calibration:?}"
    );
    let full = started.elapsed();
    eprintln!("heavy preview replay took {full:?}");
    assert!(
        full >= Duration::from_millis(160),
        "replay too short to cancel mid-flight: {full:?}"
    );
    full
}

async fn worker_head(wm: &WorkerManager) -> WorkerHead {
    GeometryEngine::get_worker_head(wm)
        .await
        .expect("GetWorkerHead")
}

/// Polls until the worker reports a `PreviewPlan` on its kernel lane.
async fn wait_for_preview_inflight(wm: &WorkerManager) {
    wait_for_preview_after(wm, None).await;
}

/// Polls until the worker runs a `PreviewPlan` request other than `after`;
/// returns its OCW1 request id.
async fn wait_for_preview_after(wm: &WorkerManager, after: Option<u64>) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let head = worker_head(wm).await;
        if let Some(job) = head
            .inflight
            .as_ref()
            .filter(|j| j.verb == "PreviewPlan" && Some(j.id) != after)
        {
            return job.id;
        }
        assert!(
            Instant::now() < deadline,
            "no PreviewPlan ever ran: {head:?}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Polls until the kernel lane is idle and no scratch is held; returns how long.
async fn wait_lane_idle(wm: &WorkerManager) -> Duration {
    let started = Instant::now();
    loop {
        let head = worker_head(wm).await;
        if head.inflight.is_none() && !head.has_scratch {
            return started.elapsed();
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the kernel lane never went idle: {head:?}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

fn spawn_preview(
    state: &Arc<AppState>,
    distance: f64,
) -> tokio::task::JoinHandle<Result<EditPreview, ApiError>> {
    let state = state.clone();
    tokio::spawn(async move { preview(&state, EXTRUDE_A, depth(distance)).await })
}

/// Abandoning the caller's future (the UI dropping a superseded candidate) must
/// stop the WORKER job, not just the Rust wait: the kernel lane frees within a
/// step or two, far inside the remaining replay time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abandoning_a_preview_frees_the_kernel_lane() {
    let Some((wm, state)) = document(heavy_part()).await else {
        return;
    };
    let state = Arc::new(state);
    let full = full_replay_time(&state).await;
    let task = spawn_preview(&state, 6.0);
    wait_for_preview_inflight(&wm).await;
    task.abort();
    let _ = task.await;
    let drained = wait_lane_idle(&wm).await;
    eprintln!("lane idle {drained:?} after abandoning (full replay {full:?})");
    assert!(
        drained < full / 2,
        "the worker kept replaying after the cancel: idle after {drained:?}, full replay {full:?}"
    );
    let again = preview(&state, EXTRUDE_A, depth(8.0)).await;
    assert!(
        matches!(again, Ok(ref p) if p.outcome == EditPreviewOutcome::Ready),
        "{again:?}"
    );
}

/// Apply during a preview: `begin_regen` preempts it (`Cancelled`, never a
/// stale-fence `StalePreview`), the worker drops the replay, and the commit does
/// not queue behind it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apply_preempts_an_inflight_preview_without_queueing_behind_it() {
    let Some((wm, state)) = document(heavy_part()).await else {
        return;
    };
    let state = Arc::new(state);
    let full = full_replay_time(&state).await;
    let started = Instant::now();
    commit(&state, EXTRUDE_A, depth(9.0)).await;
    let commit_alone = started.elapsed();

    let task = spawn_preview(&state, 6.0);
    wait_for_preview_inflight(&wm).await;
    let started = Instant::now();
    let snap = commit(&state, EXTRUDE_A, depth(7.0)).await;
    let commit_time = started.elapsed();
    eprintln!(
        "commit alone {commit_alone:?}, under a preview {commit_time:?}, full replay {full:?}"
    );
    let result = task.await.unwrap();
    assert!(
        matches!(result, Err(ApiError::Cancelled(_))),
        "a preempted preview reports Cancelled: {result:?}"
    );
    assert!(
        commit_time < commit_alone + full / 2,
        "the commit queued behind the preempted replay: {commit_time:?} vs alone {commit_alone:?} + full {full:?}"
    );
    let head = worker_head(&wm).await;
    assert!(!head.has_scratch, "a cancelled preview leaves no scratch");
    assert_eq!(head.snapshot_id, snap.id, "the commit published");
    let again = preview(&state, EXTRUDE_A, depth(8.0)).await;
    assert!(
        matches!(again, Ok(ref p) if p.outcome == EditPreviewOutcome::Ready),
        "{again:?}"
    );
}

/// Typing through a field: every draft supersedes the previous one, so only the
/// last is `Ready` and it shows the last value.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_supersede_storm_leaves_only_the_last_draft_ready() {
    let Some((wm, state)) = document(heavy_part()).await else {
        return;
    };
    let state = Arc::new(state);
    let drafts = [4.0, 5.0, 6.0, 7.0, 8.0];
    let mut tasks = Vec::new();
    let mut running = None;
    for distance in drafts {
        tasks.push(spawn_preview(&state, distance));
        // Each draft reaches the worker before the next one is typed.
        running = Some(wait_for_preview_after(&wm, running).await);
    }
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.unwrap());
    }
    let (last, earlier) = results.split_last().unwrap();
    for (distance, result) in drafts.iter().zip(earlier) {
        assert!(
            matches!(result, Err(ApiError::StalePreview(_))),
            "draft {distance} was superseded: {result:?}"
        );
    }
    let last = last.as_ref().expect("the last draft previews");
    assert_eq!(last.outcome, EditPreviewOutcome::Ready);
    wait_lane_idle(&wm).await;

    let snap = commit(&state, EXTRUDE_A, depth(8.0)).await;
    assert_preview_equals_commit(&state, last, &snap).await;
}

/// The worker binary behind a unique symlink, so exactly this test's worker can
/// be killed by command line while other tests run their own.
fn private_worker(dir: &tempfile::TempDir) -> Option<PathBuf> {
    let bin = real_worker()?;
    let link = dir
        .path()
        .join(format!("onecad-worker-crash-{}", Uuid::new_v4().simple()));
    std::os::unix::fs::symlink(std::fs::canonicalize(bin).unwrap(), &link).unwrap();
    Some(link)
}

/// A crash mid-preview (not a retire): the preview is a worker-class error, the
/// supervisor restarts the worker, the restart replay reproduces the pre-preview
/// head exactly, and a fresh preview runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_crash_mid_preview_restarts_to_the_unedited_head() {
    let dir = tempfile::tempdir().unwrap();
    let Some(bin) = private_worker(&dir) else {
        return;
    };
    let (wm, state) = document_on(bin.clone(), heavy_part()).await;
    let state = Arc::new(state);
    let signatures = |rt: &DocumentRuntime| {
        rt.subscribe_snapshots()
            .borrow()
            .clone()
            .expect("published head")
            .signatures
            .clone()
    };
    let before = signatures(state.runtime.lock().await.as_ref().unwrap());
    let epoch = wm.epoch();

    let task = spawn_preview(&state, 6.0);
    wait_for_preview_inflight(&wm).await;
    let killed = std::process::Command::new("pkill")
        .args(["-KILL", "-f"])
        .arg(&bin)
        .status()
        .unwrap();
    assert!(killed.success(), "pkill found the worker");
    let result = task.await.unwrap();
    assert!(matches!(result, Err(ApiError::Worker(_))), "{result:?}");

    let deadline = Instant::now() + Duration::from_secs(20);
    while wm.epoch() == epoch || !wm.wait_ready(Duration::from_millis(100)).await {
        assert!(
            Instant::now() < deadline,
            "the supervisor never restarted the worker"
        );
    }
    // What the app's restart hook does (state.rs `install_restart_hook`).
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        rt.on_worker_restart(wm.epoch());
        let report = rt
            .run_regen(RegenRequest::RevertToEnd { from: 0 }, CancelToken::new())
            .await;
        assert!(
            matches!(report.outcome, Outcome::Published(_)),
            "{:?}",
            report.outcome
        );
        assert_eq!(
            signatures(rt),
            before,
            "the restart replay is the pre-preview head"
        );
    }
    let again = preview(&state, EXTRUDE_A, depth(8.0)).await;
    assert!(
        matches!(again, Ok(ref p) if p.outcome == EditPreviewOutcome::Ready),
        "{again:?}"
    );
}
