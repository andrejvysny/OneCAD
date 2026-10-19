use super::*;
use std::sync::atomic::Ordering;

pub(super) async fn accepted_capture_failure(mut host: Host, ids: ArcIds) {
    let before = host.sketch().clone();
    let depth = undo_depth(&host);
    let model_depth = host.app.display.as_ref().unwrap().projection.undo_depth;
    let old_key = host.app.sketch.session.as_ref().unwrap().key.clone();
    let old_capture = host.app.view.unwrap().identity();
    let old_revision = host.app.document_state.as_ref().unwrap().revision;
    let (release, gate) = tokio::sync::oneshot::channel();
    *host.app.bridge.shared.gesture_end_gate.lock().unwrap() = Some(gate);
    host.app
        .bridge
        .shared
        .fail_next_gesture_capture
        .store(true, Ordering::Release);
    // Keep recovery captures refused through the real bounded handoff budget as well.
    let mut reservations = Vec::new();
    for _ in 0..12 {
        match host.app.bridge.shared.host_budget.reserve(64 << 20) {
            Ok(lease) => reservations.push(lease),
            Err(_) => break,
        }
    }
    assert!(!reservations.is_empty());
    drag(
        &mut host,
        [0.0, 0.0],
        [5.0, 6.0],
        GestureKind::EntityBody,
        Some(CurvePosition::Center),
    );
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.bridge
            .shared
            .gesture_end_arrived
            .load(Ordering::Acquire)
    })
    .await;
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.bridge
            .shared
            .gesture_cancel_observed
            .load(Ordering::Acquire)
    })
    .await;
    assert_eq!(
        host.app.sketch_gesture.phase(),
        Some(GesturePhase::Cancelling)
    );
    assert!(
        host.app.sketch_gesture.busy(),
        "Cancel ack must not release UI ownership before accepted End delivery"
    );
    release.send(()).unwrap();
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.sketch_gesture.busy()
    })
    .await;
    assert!(host.app.document_dirty());
    assert!(host.app.document_state.as_ref().unwrap().revision > old_revision);
    assert!(
        !host.app.bridge.valid(),
        "failed capture must not authorize the old scene"
    );
    assert!(
        host.app
            .sketch
            .session
            .as_ref()
            .unwrap()
            .accepted_capture
            .is_none()
    );
    let accepted = inspect(&host, old_key.sketch_id).await;
    assert_arc(
        &accepted.sketch,
        ids,
        [5.0, 6.0],
        10.0,
        Some([0.0, std::f64::consts::FRAC_PI_2]),
    );
    if let Some(history) = &accepted.history {
        assert_eq!(history.undo_depth, depth + 1);
    } else {
        assert_eq!(
            accepted.undo_depth,
            model_depth + 1,
            "authority may have kept and closed the edit visit for saving"
        );
    }
    host.app.sketch_dispatch(SketchAction::Click {
        point: onecad_interaction::sketch::CapturedSketchPoint {
            key: old_key.clone(),
            capture: old_capture,
            point: Vec2::new(50.0, 50.0).unwrap(),
        },
        ids: super::super::dimension_delivery_tests::ids(),
    });
    let after = inspect(&host, old_key.sketch_id).await;
    assert_eq!(after.sketch, accepted.sketch);
    assert_eq!(after.history, accepted.history);
    assert_eq!(after.undo_depth, accepted.undo_depth);
    assert_ne!(
        after.sketch, before,
        "accepted drag geometry must survive failed presentation"
    );
    assert!(
        crate::bars::save_allowed(&host.app),
        "accepted state must remain saveable: {}",
        host.app.status
    );
    for request in [
        Request::Save { save_as: false },
        Request::Save { save_as: true },
    ] {
        assert!(
            !crate::document_flow::tool_blocks_request(&host.app, &request),
            "recovery Save request must pass the actual frontend guard"
        );
    }
    for request in [Request::New, Request::Import] {
        assert!(
            crate::document_flow::tool_blocks_request(&host.app, &request),
            "recovery saving must not authorize document replacement"
        );
    }
    drop(reservations);
    host.stop().await;
}

async fn inspect(host: &Host, sketch: SketchId) -> crate::gesture_services::testing::Inspection {
    let mut expected = host.app.display.as_ref().unwrap().context.clone();
    expected.revision =
        onecad_core::ids::DocumentRevision(host.app.document_state.as_ref().unwrap().revision);
    let (reply, receive) = tokio::sync::oneshot::channel();
    host.app
        .bridge
        .submit(crate::bridge::Command::InspectSketch(Box::new(
            crate::gesture_services::testing::InspectionRequest {
                expected,
                sketch,
                reply,
            },
        )))
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(8), receive)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}
