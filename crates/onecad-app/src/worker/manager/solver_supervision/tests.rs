use super::*;
use crate::worker::manager::poison_tests::{read_mock_request, send_mock_frame, shared};
use crate::worker::manager::readiness_tests::connection;

struct WedgedInstance;
#[async_trait]
impl WorkerInstance for WedgedInstance {
    async fn exited(&mut self) {
        std::future::pending().await
    }
    async fn terminate(&mut self, _: Duration) {}
    async fn force_kill(&mut self) -> KillOutcome {
        KillOutcome::Abandoned
    }
}

#[tokio::test]
async fn host_deadline_supervises_queued_solver_even_with_healthy_pings() {
    let mut shared = shared(1);
    let config = &mut Arc::get_mut(&mut shared).unwrap().config;
    config.wedge_deadline_default = Duration::from_millis(80);
    config.ping_interval = Duration::from_millis(5);
    let (client, mut server) = connection().await;
    assert!(shared.publish_ready(client.clone()));
    let pings = Arc::new(AtomicU64::new(0));
    let server_task = {
        let pings = pings.clone();
        tokio::spawn(async move {
            let request = read_mock_request(&mut server).await;
            assert_eq!(request["verb"], "SketchRegions");
            loop {
                let request = read_mock_request(&mut server).await;
                assert_eq!(
                    request["verb"], "GetWorkerHead",
                    "ordinary request must not retry"
                );
                pings.fetch_add(1, Ordering::SeqCst);
                send_mock_frame(
                    &mut server,
                    json!({"v":1,"t":"resp","id":request["id"],
                "ok":true,"documentRevision":0,"workerEpoch":1,"snapshotId":0,"seq":1,
                "result":{"inflightJobs":[]}}),
                )
                .await;
            }
        })
    };
    let request = shared.solver_round_trip(&client, "SketchRegions", json!({}));
    let mut instance: Box<dyn WorkerInstance> = Box::new(WedgedInstance);
    let (result, death) = tokio::join!(request, run_until_death(&shared, &client, &mut instance));
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("completion unknown"));
    assert!(matches!(death, Death::Abandoned { reason } if reason.contains("SketchRegions")));
    assert!(pings.load(Ordering::SeqCst) > 0);
    assert!(shared.client().is_none());
    server_task.abort();
}

#[tokio::test]
async fn stale_timeout_cannot_stop_replacement_or_retry_original() {
    let mut shared = shared(1);
    Arc::get_mut(&mut shared)
        .unwrap()
        .config
        .wedge_deadline_default = Duration::from_millis(40);
    let (old, mut server) = connection().await;
    assert!(shared.publish_ready(old.clone()));
    let request = {
        let shared = shared.clone();
        tokio::spawn(async move {
            shared
                .solver_round_trip(&old, "SketchUpsert", json!({}))
                .await
        })
    };
    assert_eq!(read_mock_request(&mut server).await["verb"], "SketchUpsert");
    shared.disconnect_generation();
    let (new, _new_server) = connection().await;
    assert!(shared.publish_ready(new.clone()));
    assert!(request.await.unwrap().is_err());
    assert!(shared.take_expired_solver(&new).is_none());
    assert!(shared.client().is_some());
    use tokio::io::AsyncReadExt;
    let mut byte = [0];
    let next = tokio::time::timeout(Duration::from_millis(20), server.read(&mut byte)).await;
    assert!(
        matches!(next, Ok(Ok(0)) | Err(_)),
        "old connection sent another request: {next:?}"
    );
}

#[tokio::test]
async fn completed_job_in_stale_status_snapshot_cannot_trigger_wedge() {
    let shared = shared(1);
    let (client, mut server) = connection().await;
    assert!(shared.publish_ready(client.clone()));
    let pending = client
        .start_request("SketchUpsert", json!({}), Lane::Control)
        .await
        .unwrap();
    let request = read_mock_request(&mut server).await;
    let status = json!({"inflightJobs":[{"kind":"solver","verb":"SketchUpsert",
        "id":pending.id,"ageMs":60000,"sinceProgressMs":60000}]});
    assert!(wedged_inflight(&shared, &client, Some(&status))
        .unwrap()
        .is_some());
    send_mock_frame(
        &mut server,
        json!({"v":1,"t":"resp","id":request["id"],"ok":true,
        "documentRevision":0,"workerEpoch":1,"snapshotId":0,"seq":1,"result":{}}),
    )
    .await;
    pending.response().await.unwrap();
    assert!(wedged_inflight(&shared, &client, Some(&status))
        .unwrap()
        .is_none());
}
