use super::*;
#[test]
fn closed_queue_rejects_racing_submit_and_releases_permit() {
    let permits = Arc::new(Semaphore::new(ADMISSION));
    let (commands, mut queue) = async_mpsc::channel(ADMISSION);
    admit(
        &commands,
        &permits,
        Command::Document(crate::document_flow::Request::Save { save_as: false }),
    )
    .unwrap();
    queue.close();
    assert!(
        admit(
            &commands,
            &permits,
            Command::Document(crate::document_flow::Request::Save { save_as: false })
        )
        .is_err()
    );
    assert_eq!(permits.available_permits(), ADMISSION - 1);
    drop(queue.try_recv().unwrap());
    assert_eq!(permits.available_permits(), ADMISSION);
}
#[test]
fn actual_admission_keeps_capacity_reserved_until_terminal_delivery() {
    let permits = Arc::new(Semaphore::new(ADMISSION));
    let (commands, mut queue) = async_mpsc::channel(ADMISSION);
    for _ in 0..ADMISSION {
        admit(
            &commands,
            &permits,
            Command::Document(crate::document_flow::Request::Save { save_as: false }),
        )
        .unwrap();
    }
    assert!(
        admit(
            &commands,
            &permits,
            Command::Document(crate::document_flow::Request::Save { save_as: false })
        )
        .is_err()
    );
    let (sender, receiver) = mpsc::channel();
    for _ in 0..ADMISSION {
        let envelope = queue.try_recv().unwrap();
        sender
            .send(Message::Terminal {
                command: "Save",
                tag: None,
                result: Ok(Default::default()),
                _permit: envelope.permit,
            })
            .unwrap();
    }
    assert!(
        admit(
            &commands,
            &permits,
            Command::Document(crate::document_flow::Request::Save { save_as: false })
        )
        .is_err()
    );
    drop(receiver.recv().unwrap());
    admit(
        &commands,
        &permits,
        Command::Document(crate::document_flow::Request::Save { save_as: false }),
    )
    .unwrap();
    assert!(
        admit(
            &commands,
            &permits,
            Command::Document(crate::document_flow::Request::Save { save_as: false })
        )
        .is_err()
    );
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn production_actor_captures_blank_start_and_repeated_new_documents() {
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
    let root = area.path().join("qa");
    let paths =
        crate::qa::QaPaths::from_args(["--qa-root".into(), root.into_os_string()].into_iter())
            .unwrap();
    let bridge = Bridge::start(
        paths,
        egui::Context::default(),
        renderer.preparation_budget(),
    )
    .unwrap();
    bridge.submit(Command::Open).unwrap();
    wait_actor_terminal(&bridge, false).await;
    let initial = bridge
        .take_display()
        .expect("blank startup must publish a usable captured document");
    assert!(
        bridge.valid(),
        "{:?}",
        bridge.shared.failure.lock().unwrap()
    );
    assert!(initial.projection.features.is_empty());
    let mut previous = initial.projection.runtime_session.clone();
    drop(initial);
    for _ in 0..3 {
        bridge
            .submit(Command::Document(crate::document_flow::Request::New))
            .unwrap();
        wait_actor_terminal(&bridge, true).await;
        let display = bridge
            .take_display()
            .expect("New must capture its replacement, not retain the old frame");
        assert_ne!(display.projection.runtime_session, previous);
        assert!(display.projection.features.is_empty());
        assert!(display.projection.bodies.is_empty());
        assert!(
            bridge.valid(),
            "{:?}",
            bridge.shared.failure.lock().unwrap()
        );
        previous = display.projection.runtime_session.clone();
    }
    bridge.request_shutdown();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if matches!(bridge.messages.try_recv(), Ok(Message::Stopped)) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actor must stop cleanly");
}

async fn wait_actor_terminal(bridge: &Bridge, document: bool) {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            match bridge.messages.try_recv() {
                Ok(Message::Document { result, .. }) if document => {
                    assert!(result.is_ok(), "{result:?}");
                    break;
                }
                Ok(Message::Terminal { result, .. }) if !document => {
                    assert!(result.is_ok(), "{result:?}");
                    break;
                }
                Ok(Message::StartupFailed(error)) => panic!("{error}"),
                _ => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("actor must deliver bounded completion");
}
