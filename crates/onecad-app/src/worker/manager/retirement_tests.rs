use super::poison_tests::{read_mock_request, send_mock_frame, shared};
use super::*;

#[tokio::test]
async fn ready_retirement_sends_one_shutdown_after_terminal_latch_without_epoch_bump() {
    let shared = shared(1);
    let manager = WorkerManager {
        shared: shared.clone(),
    };
    let (client_io, mut server) = tokio::io::duplex(4096);
    send_mock_frame(
        &mut server,
        json!({"v":1,"t":"hello","seq":0,"result":{
        "protocolVersion":1,"workerVersion":"test",
        "occt":{"version":"test","fingerprint":"0000000000000000"},
        "quantizationVersion":1,"solverPolicyVersion":4,"capabilities":[]}}),
    )
    .await;
    let (reader, writer) = tokio::io::split(client_io);
    let client = Arc::new(ProtocolClient::connect(reader, writer).await.unwrap());
    assert!(shared.publish_ready(client));
    let epoch = manager.epoch();
    manager.retire();
    manager.retire();
    assert!(manager.is_retired());
    assert_eq!(manager.epoch(), epoch);
    assert!(manager.client_or_err().is_err());
    let request = tokio::time::timeout(Duration::from_secs(1), read_mock_request(&mut server))
        .await
        .expect("a ready retired worker must receive its graceful Shutdown");
    assert_eq!(request["verb"], "Shutdown");
    send_mock_frame(
        &mut server,
        json!({"v":1,"t":"resp","id":request["id"],"ok":true,
        "documentRevision":0,"workerEpoch":epoch.0,"snapshotId":0,"seq":1,"result":{}}),
    )
    .await;
    use tokio::io::AsyncReadExt;
    let mut byte = [0];
    assert!(
        tokio::time::timeout(Duration::from_millis(20), server.read(&mut byte))
            .await
            .is_err(),
        "retirement must send Shutdown only once"
    );
}
