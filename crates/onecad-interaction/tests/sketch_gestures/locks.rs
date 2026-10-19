use super::*;

#[test]
fn free_endpoint_and_radial_channels_remain_usable_beside_locked_points() {
    let mut state = state();
    let sketch = &mut state.session.as_mut().unwrap().authoritative;
    for slot in [1, 2, 3] {
        let point = sketch
            .get_entity(eid(slot))
            .unwrap()
            .clone()
            .with_reference_locked(true);
        sketch.remove_entity(eid(slot));
        sketch.add_entity(point).unwrap();
    }
    assert_eq!(
        classify_target(sketch, point_target(5, CurvePosition::Start), xy(0.0, 0.0))
            .unwrap()
            .kind,
        GestureKind::Point
    );
    assert!(classify_target(sketch, point_target(5, CurvePosition::End), xy(0.0, 0.0)).is_err());
    assert!(classify_target(sketch, SketchTarget::Entity(eid(5)), xy(0.0, 0.0)).is_err());
    for target in [
        SketchTarget::Entity(eid(6)),
        SketchTarget::Entity(eid(7)),
        point_target(7, CurvePosition::Start),
        point_target(7, CurvePosition::End),
    ] {
        assert!(classify_target(sketch, target, xy(0.0, 0.0)).is_ok());
    }
    assert!(classify_target(sketch, point_target(6, CurvePosition::Center), xy(0.0, 0.0)).is_err());
    assert!(classify_target(sketch, point_target(7, CurvePosition::Center), xy(0.0, 0.0)).is_err());
}

#[test]
fn failed_terminal_cancel_releases_local_owner_and_keeps_error_without_preview() {
    let state = state();
    let mut controller = SketchGestureInteraction::default();
    let begin = begin(&mut controller, &state);
    let cancel =
        cancel_tag(controller.dispatch(GestureAction::Cancel { tag: begin.clone() }, &state));
    assert!(controller
        .dispatch(
            GestureAction::Failed {
                tag: cancel,
                message: "worker retired during cancellation".into()
            },
            &state
        )
        .is_empty());
    assert!(!controller.busy());
    assert!(controller.preview().is_none());
    assert_eq!(
        controller.error(),
        Some("worker retired during cancellation")
    );
    assert!(ready(&mut controller, &state, &begin).is_empty());
    assert!(!controller.busy());
}

#[test]
fn failed_update_cancels_and_invalid_worker_preview_cannot_move_locked_entities() {
    let mut state = state();
    let locked = SketchEntity::point(eid(10), xy(20.0, 30.0), false, true);
    state
        .session
        .as_mut()
        .unwrap()
        .authoritative
        .add_entity(locked)
        .unwrap();
    let mut controller = SketchGestureInteraction::default();
    let begin = begin(&mut controller, &state);
    ready(&mut controller, &state, &begin);
    let update = update_tag(movement(&mut controller, &state, &begin, 4.0), 4.0);
    let mut preview = shifted(&state, 4.0);
    preview.apply_solved_positions(&[(eid(10), xy(21.0, 30.0))]);
    cancel_tag(controller.dispatch(
        GestureAction::Updated {
            tag: update,
            worker_id: 7,
            worker_sequence: 1,
            preview,
        },
        &state,
    ));
    assert!(controller.preview().is_none());
    assert!(controller.error().unwrap().contains("preview"));
}

#[test]
fn failed_begin_update_and_end_cancel_once_and_preserve_authority() {
    for phase in 0..3 {
        let state = state();
        let accepted = baseline(&state);
        let mut controller = SketchGestureInteraction::default();
        let begin = begin(&mut controller, &state);
        let pending = match phase {
            0 => begin.clone(),
            1 => {
                ready(&mut controller, &state, &begin);
                update_tag(movement(&mut controller, &state, &begin, 5.0), 5.0)
            }
            _ => {
                ready(&mut controller, &state, &begin);
                end_tag(
                    controller.dispatch(
                        GestureAction::End {
                            tag: begin,
                            point: xy(6.0, 0.0),
                        },
                        &state,
                    ),
                    6.0,
                )
            }
        };
        let cancel = cancel_tag(controller.dispatch(
            GestureAction::Failed {
                tag: pending.clone(),
                message: "solver unavailable".into(),
            },
            &state,
        ));
        assert!(controller.preview().is_none());
        assert_eq!(controller.error(), Some("solver unavailable"));
        assert!(controller
            .dispatch(
                GestureAction::Failed {
                    tag: pending,
                    message: "late failure".into()
                },
                &state
            )
            .is_empty());
        assert_eq!(controller.error(), Some("solver unavailable"));
        controller.dispatch(GestureAction::Cancelled { tag: cancel }, &state);
        assert!(!controller.busy());
        assert_eq!(baseline(&state), accepted);
    }
}

#[test]
fn stale_receipts_on_every_scope_axis_leave_current_pending_request_intact() {
    for axis in 0..10 {
        let state = state();
        let mut controller = SketchGestureInteraction::default();
        let begin = begin(&mut controller, &state);
        let mut stale = begin.clone();
        match axis {
            0 => stale.scope.key.generation += 1,
            1 => stale.scope.document.revision += 1,
            2 => stale.scope.document.snapshot_id.0 += 1,
            3 => stale.scope.document.runtime_session.push_str("new"),
            4 => stale.scope.edit_session.push_str("new"),
            5 => stale.scope.capture.view += 1,
            6 => stale.gesture += 1,
            7 => stale.sequence += 1,
            8 => stale.scope.document.document_id.push_str("new"),
            _ => stale.scope.capture.scene.scene += 1,
        }
        assert!(ready(&mut controller, &state, &stale).is_empty());
        assert_eq!(controller.tag(), Some(&begin));
        assert_eq!(controller.phase(), Some(GesturePhase::PendingBegin));
        assert!(controller.preview().is_none());
        ready(&mut controller, &state, &begin);
        assert_eq!(controller.phase(), Some(GesturePhase::Ready));
    }
}

#[test]
fn pending_begin_coalesces_pointer_moves_until_ready_and_new_visits_never_reuse_token() {
    let state = state();
    let mut controller = SketchGestureInteraction::default();
    let first = begin(&mut controller, &state);
    for value in 0..100 {
        assert!(movement(&mut controller, &state, &first, value as f64).is_empty());
    }
    update_tag(ready(&mut controller, &state, &first), 99.0);
    let cancel =
        cancel_tag(controller.dispatch(GestureAction::Cancel { tag: first.clone() }, &state));
    controller.dispatch(GestureAction::Cancelled { tag: cancel }, &state);
    let next = begin(&mut controller, &state);
    assert!(next.gesture > first.gesture);
    assert!(ready(&mut controller, &state, &first).is_empty());
    assert_eq!(controller.phase(), Some(GesturePhase::PendingBegin));
    assert!(movement(&mut controller, &state, &first, 200.0).is_empty());
    assert_eq!(controller.tag(), Some(&next));
}
