use super::poison_tests::{read_mock_request, send_mock_frame, shared};
use super::*;

pub(super) async fn connection() -> (Arc<ProtocolClient>, tokio::io::DuplexStream) {
    let (io, mut server) = tokio::io::duplex(4096);
    send_mock_frame(&mut server, json!({"v":1,"t":"hello","seq":0,"result":{
        "protocolVersion":1,"workerVersion":"test","occt":{"version":"test","fingerprint":"0000000000000000"},
        "quantizationVersion":1,"solverPolicyVersion":5,"capabilities":[]}})).await;
    let (reader, writer) = tokio::io::split(io);
    (
        Arc::new(ProtocolClient::connect(reader, writer).await.unwrap()),
        server,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovery_publication_is_indivisible_at_every_barrier() {
    let shared = shared(1);
    let manager = WorkerManager {
        shared: shared.clone(),
    };
    let (old, _old_server) = connection().await;
    assert!(shared.publish_ready(old));
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    let transition = {
        let (shared, entered, resume) = (shared.clone(), entered.clone(), resume.clone());
        std::thread::spawn(move || {
            shared.disconnect_generation_with(|| {
                entered.wait();
                resume.wait();
            })
        })
    };
    for _ in 0..3 {
        entered.wait();
        assert!(
            shared.connection.try_read().is_err(),
            "no intermediate tuple is observable"
        );
        resume.wait();
    }
    assert_eq!(transition.join().unwrap(), 2);
    assert!(!manager.wait_ready(Duration::ZERO).await);
    assert!(manager.client_or_err().is_err());
    let waiter = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.wait_ready(Duration::from_secs(2)).await })
    };
    let (new, _new_server) = connection().await;
    let mut events = manager.subscribe();
    assert!(shared.publish_ready(new));
    shared.emit(WorkerLifecycle::Ready {
        epoch: 2,
        fingerprint: "0000000000000000".into(),
    });
    assert!(waiter.await.unwrap());
    assert!(matches!(
        events.recv().await.unwrap(),
        WorkerLifecycle::Ready { epoch: 2, .. }
    ));
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn late_solver_reply_cannot_cross_recovery_generation() {
    use onecad_core::sketch::WorldPlane;
    let shared = shared(1);
    let manager = WorkerManager {
        shared: shared.clone(),
    };
    let (old, mut server) = connection().await;
    assert!(shared.publish_ready(old));
    let id = SketchId(uuid::Uuid::from_u128(0x141));
    let request = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .sketch_upsert(&Sketch::on_world_plane(id, "test", WorldPlane::XY))
                .await
        })
    };
    let sent = read_mock_request(&mut server).await;
    shared.disconnect_generation();
    let (new, _new_server) = connection().await;
    assert!(shared.publish_ready(new));
    send_mock_frame(&mut server, json!({"v":1,"t":"resp","id":sent["id"],"ok":true,
        "documentRevision":0,"workerEpoch":1,"snapshotId":0,"seq":1,
        "result":{"upserted":true,"sketchId":id.to_string(),"sketchRevision":1,"dof":0,"state":"FullyConstrained"}})).await;
    assert!(request.await.unwrap().is_err());
    assert_eq!(manager.epoch(), WorkerEpoch(2));
}

#[tokio::test]
async fn failed_retired_and_closed_connections_never_report_ready() {
    let shared = shared(1);
    let manager = WorkerManager {
        shared: shared.clone(),
    };
    let (client, server) = connection().await;
    assert!(shared.publish_ready(client.clone()));
    drop(server);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !client.is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!manager.wait_ready(Duration::ZERO).await);
    shared.fail("relaunch-required".into());
    assert!(!manager.wait_ready(Duration::ZERO).await);
    manager.retire();
    assert!(!shared.publish_ready(client));
    assert!(!manager.wait_ready(Duration::ZERO).await);
}
