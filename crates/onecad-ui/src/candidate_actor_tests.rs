//! Deterministic service gates exercise the production actor without kernel timing guesses.
use super::*;
use tokio::sync::oneshot;

struct Gate {
    started: oneshot::Receiver<()>,
    release: oneshot::Sender<()>,
    finished: oneshot::Receiver<()>,
}

fn gate(bridge: &Bridge, sequence: u64) -> Gate {
    let (started_tx, started) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let (finished_tx, finished) = oneshot::channel();
    bridge.candidate(Candidate::Gate {
        tag: RequestTag {
            context: onecad_interaction::DocumentContext {
                document_id: "actor-test".into(),
                runtime_session: "actor-test".into(),
                revision: 0,
                snapshot_id: onecad_core::ids::SnapshotId(0),
                scene: onecad_scene::SceneIdentity {
                    session: 1,
                    scene: 1,
                },
            },
            record_id: onecad_core::ids::RecordId::new(),
            gesture: 1,
            sequence,
        },
        started: started_tx,
        release: release_rx,
        finished: finished_tx,
    });
    Gate {
        started,
        release,
        finished,
    }
}

async fn message(bridge: &Bridge, wanted: fn(&Message) -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match bridge.messages.try_recv() {
                Ok(message) if wanted(&message) => break,
                Ok(Message::StartupFailed(error)) => panic!("{error}"),
                _ => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("actor control must remain responsive");
}

#[tokio::test]
#[ignore = "requires Metal preparation budget; no kernel work"]
async fn active_candidate_keeps_control_responsive_coalesces_pending_and_drains_shutdown() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_features: onecad_render::required_device_features(adapter.features()),
            ..Default::default()
        })
        .await
        .unwrap();
    let renderer =
        onecad_render::Renderer::new(&device, &queue, [0, 0], Default::default()).unwrap();
    let area = tempfile::tempdir().unwrap();
    let paths = QaPaths::from_args(
        ["--qa-root".into(), area.path().join("qa").into_os_string()].into_iter(),
    )
    .unwrap();
    let bridge = Bridge::start(
        paths,
        egui::Context::default(),
        renderer.preparation_budget(),
    )
    .unwrap();
    let first = gate(&bridge, 1);
    first.started.await.unwrap();
    bridge.submit(Command::Undo).unwrap();
    bridge.submit(Command::DescribeChanges).unwrap();
    message(&bridge, |message| {
        matches!(message, Message::Changes { .. })
    })
    .await;
    let replaced = gate(&bridge, 2);
    let mut latest = gate(&bridge, 3);
    assert!(replaced.started.await.is_err());
    assert!(matches!(
        latest.started.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    first.release.send(()).unwrap();
    first.finished.await.unwrap();
    latest.started.await.unwrap();
    message(&bridge, |message| {
        matches!(
            message,
            Message::Terminal {
                command: "Undo",
                ..
            }
        )
    })
    .await;
    bridge.submit(Command::Redo).unwrap();
    bridge.submit(Command::DescribeChanges).unwrap();
    message(&bridge, |message| {
        matches!(message, Message::Changes { .. })
    })
    .await;
    bridge.request_shutdown();
    assert!(bridge.submit(Command::DescribeChanges).is_err());
    assert!(matches!(
        latest.finished.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    latest.release.send(()).unwrap();
    latest.finished.await.unwrap();
    message(&bridge, |message| {
        matches!(
            message,
            Message::Terminal {
                command: "Redo",
                result: Err(_),
                ..
            }
        )
    })
    .await;
    message(&bridge, |message| matches!(message, Message::Stopped)).await;
}
