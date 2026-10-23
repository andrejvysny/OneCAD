use super::*;
use crate::worker::manager::poison_tests::{read_mock_request, send_mock_frame, shared};

struct StartingInstance {
    stopped: Arc<AtomicU64>,
    outcome: KillOutcome,
}

#[async_trait]
impl WorkerInstance for StartingInstance {
    async fn exited(&mut self) {
        std::future::pending().await
    }
    async fn terminate(&mut self, _: Duration) {
        self.stopped.fetch_add(1, Ordering::SeqCst);
    }
    async fn force_kill(&mut self) -> KillOutcome {
        tokio::task::yield_now().await;
        self.stopped.fetch_add(1, Ordering::SeqCst);
        self.outcome
    }
}

async fn launch(outcome: KillOutcome) -> (Launched, tokio::io::DuplexStream, Arc<AtomicU64>) {
    let (io, mut server) = tokio::io::duplex(4096);
    send_mock_frame(&mut server, json!({"v":1,"t":"hello","seq":0,"result":{
        "protocolVersion":1,"workerVersion":"test","occt":{"version":"test","fingerprint":"0000000000000000"},
        "quantizationVersion":1,"solverPolicyVersion":onecad_core::regen::engine::SOLVER_POLICY_VERSION,
        "capabilities":[]}})).await;
    let (reader, writer) = tokio::io::split(io);
    let stopped = Arc::new(AtomicU64::new(0));
    let launched = Launched {
        reader: Box::new(reader),
        writer: Box::new(writer),
        expected: None,
        instance: Box::new(StartingInstance {
            stopped: stopped.clone(),
            outcome,
        }),
    };
    (launched, server, stopped)
}

#[tokio::test]
async fn unanswered_open_session_has_bounded_cleanup_without_ready_or_epoch_change() {
    for outcome in [KillOutcome::Reaped, KillOutcome::Abandoned] {
        let mut shared = shared(1);
        let config = &mut Arc::get_mut(&mut shared).unwrap().config;
        config.auto_open_session = true;
        config.ping_timeout = Duration::from_millis(40);
        let (launched, mut server, stopped) = launch(outcome).await;
        let startup = {
            let shared = shared.clone();
            tokio::spawn(async move { initialize(&shared, launched).await })
        };
        assert_eq!(read_mock_request(&mut server).await["verb"], "OpenSession");
        let result = tokio::time::timeout(Duration::from_secs(1), startup)
            .await
            .unwrap()
            .unwrap();
        let Err(reason) = result else {
            panic!("startup unexpectedly succeeded");
        };
        assert!(reason.contains("startup deadline exceeded"));
        assert_eq!(
            stopped.load(Ordering::SeqCst),
            1,
            "stop completed before returning failure"
        );
        assert_eq!(shared.connection.read().unwrap().epoch, 1);
        assert!(shared.client().is_none());
        if outcome == KillOutcome::Abandoned {
            assert_eq!(shared.connection.read().unwrap().state, WorkerState::Failed);
            assert!(shared
                .failure_reason
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("relaunch-required"));
        }
    }
}

#[tokio::test]
async fn retirement_interrupts_startup_and_stops_instance_before_return() {
    let mut shared = shared(1);
    Arc::get_mut(&mut shared).unwrap().config.auto_open_session = true;
    let manager = WorkerManager {
        shared: shared.clone(),
    };
    let (launched, mut server, stopped) = launch(KillOutcome::Reaped).await;
    let startup = {
        let shared = shared.clone();
        tokio::spawn(async move { initialize(&shared, launched).await })
    };
    assert_eq!(read_mock_request(&mut server).await["verb"], "OpenSession");
    manager.retire();
    let Err(reason) = startup.await.unwrap() else {
        panic!("retired startup succeeded");
    };
    assert!(reason.contains("retired"));
    assert_eq!(stopped.load(Ordering::SeqCst), 1);
    assert_eq!(manager.epoch(), WorkerEpoch(1));
    assert!(!manager.wait_ready(Duration::ZERO).await);
}
