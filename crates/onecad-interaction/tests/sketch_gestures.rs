#[allow(dead_code)]
#[path = "../src/sketch/test_support.rs"]
mod sketch_support;
use onecad_core::{
    ids::{EntityId, SketchId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::{gesture::*, *};
use sketch_support::*;
#[path = "sketch_gestures/support.rs"]
mod support;
use support::*;

#[test]
fn classifier_keeps_whole_curves_and_point_roles_distinct_without_ghost_ids() {
    use CurvePosition::{Arbitrary, Center, End, Start};
    use GestureKind::{ArcEnd, EntityBody, Point, Radius};
    let state = state();
    let sketch = &state.session.as_ref().unwrap().authoritative;
    for (target, kind, entity, role, raw) in [
        (point_target(0, Arbitrary), Point, 0, None, 0),
        (point_target(5, Start), Point, 0, None, 0),
        (point_target(5, End), Point, 1, None, 1),
        (point_target(6, Center), Point, 2, None, 2),
        (point_target(8, Center), Point, 4, None, 4),
        (point_target(7, Center), EntityBody, 7, Some(Center), 3),
        (point_target(7, Start), ArcEnd, 7, Some(Start), 3),
        (point_target(7, End), ArcEnd, 7, Some(End), 3),
        (SketchTarget::Entity(eid(5)), EntityBody, 5, None, 0),
        (SketchTarget::Entity(eid(6)), Radius, 6, None, 2),
        (SketchTarget::Entity(eid(7)), Radius, 7, None, 3),
    ] {
        let intent = classify_target(sketch, target, xy(9.0, 8.0)).unwrap();
        assert_eq!(
            (intent.kind, intent.entity, intent.role, intent.drag_point),
            (kind, eid(entity), role, eid(raw))
        );
        assert_eq!(intent.target, target);
        assert_eq!(intent.grab, xy(9.0, 8.0));
    }
}

#[test]
fn classifier_refuses_unsupported_ambiguous_invalid_and_locked_references() {
    let mut state = state();
    let sketch = &state.session.as_ref().unwrap().authoritative;
    for target in [
        SketchTarget::Entity(eid(8)),
        SketchTarget::Entity(eid(0)),
        point_target(5, CurvePosition::Center),
        point_target(6, CurvePosition::Start),
    ] {
        assert!(classify_target(sketch, target, xy(0.0, 0.0)).is_err());
    }
    assert!(classify_candidates(
        sketch,
        &[
            point_target(0, CurvePosition::Arbitrary),
            point_target(5, CurvePosition::Start)
        ],
        xy(0.0, 0.0)
    )
    .unwrap_err()
    .contains("ambiguous"));
    assert!(classify_candidates(sketch, &[], xy(0.0, 0.0)).is_err());
    let sketch = &mut state.session.as_mut().unwrap().authoritative;
    let locked = SketchEntity::line(eid(9), eid(0), eid(1), false).with_reference_locked(true);
    sketch.add_entity(locked).unwrap();
    for target in [
        SketchTarget::Entity(eid(9)),
        point_target(0, CurvePosition::Arbitrary),
        point_target(5, CurvePosition::End),
    ] {
        assert!(classify_target(sketch, target, xy(0.0, 0.0))
            .unwrap_err()
            .contains("Locked"));
    }
}

#[test]
fn begin_requires_current_active_select_capture_without_draft_marquee_or_closed_visit() {
    for invalid in 0..7 {
        let mut state = state();
        let scope = scope(&state);
        let session = state.session.as_mut().unwrap();
        match invalid {
            0 => session.tool = SketchTool::Line,
            1 => session.phase = SketchPhase::NeedsRefresh,
            2 => session.accepted_capture = None,
            3 => session.selection_dragging = true,
            4 => session.edit_session = None,
            5 => session.key.generation += 1,
            _ => state.context.as_mut().unwrap().revision += 1,
        }
        let mut controller = SketchGestureInteraction::default();
        assert!(controller
            .dispatch(
                GestureAction::Begin {
                    scope,
                    target: point_target(0, CurvePosition::Arbitrary),
                    grab: xy(0.0, 0.0)
                },
                &state
            )
            .is_empty());
        assert!(!controller.busy());
        assert!(controller.error().is_some());
    }
}

#[test]
fn ready_and_updates_store_only_transient_checked_preview() {
    let state = state();
    let accepted = baseline(&state);
    let mut controller = SketchGestureInteraction::default();
    let begin = begin(&mut controller, &state);
    assert_eq!(controller.phase(), Some(GesturePhase::PendingBegin));
    assert!(controller.preview().is_none());
    assert!(ready(&mut controller, &state, &begin).is_empty());
    let update = update_tag(movement(&mut controller, &state, &begin, 5.0), 5.0);
    assert!(updated(&mut controller, &state, &update, 1, 5.0).is_empty());
    assert_ne!(controller.preview().unwrap(), &*accepted);
    assert_eq!(baseline(&state), accepted);
    assert!(controller.active() && controller.busy());
}

#[test]
fn only_one_update_is_inflight_and_latest_pointer_is_bounded_replaceable() {
    let state = state();
    let mut controller = SketchGestureInteraction::default();
    let begin = begin(&mut controller, &state);
    ready(&mut controller, &state, &begin);
    let first = update_tag(movement(&mut controller, &state, &begin, 1.0), 1.0);
    for x in 2..10000 {
        assert!(movement(&mut controller, &state, &begin, x as f64).is_empty());
    }
    let latest = update_tag(updated(&mut controller, &state, &first, 1, 1.0), 9999.0);
    assert_eq!(latest.sequence, first.sequence + 1);
    assert!(updated(&mut controller, &state, &first, 2, -99.0).is_empty());
    assert_eq!(controller.tag(), Some(&latest));
    assert!(updated(&mut controller, &state, &latest, 2, 9999.0).is_empty());
    assert_eq!(controller.phase(), Some(GesturePhase::Ready));
}

#[test]
fn pointer_up_before_ready_ends_at_exact_final_target_without_extra_update() {
    let state = state();
    let mut controller = SketchGestureInteraction::default();
    let begin = begin(&mut controller, &state);
    assert!(movement(&mut controller, &state, &begin, 7.0).is_empty());
    assert!(controller
        .dispatch(
            GestureAction::End {
                tag: begin.clone(),
                point: xy(8.5, 0.0)
            },
            &state
        )
        .is_empty());
    assert!(movement(&mut controller, &state, &begin, 99.0).is_empty());
    let end = end_tag(ready(&mut controller, &state, &begin), 8.5);
    assert_eq!(end.sequence, 1);
    assert_eq!(controller.phase(), Some(GesturePhase::PendingEnd));
}

#[test]
fn pointer_up_during_update_flushes_exact_final_target_and_completion_clears_preview() {
    let state = state();
    let mut controller = SketchGestureInteraction::default();
    let begin = begin(&mut controller, &state);
    ready(&mut controller, &state, &begin);
    let first = update_tag(movement(&mut controller, &state, &begin, 5.0), 5.0);
    movement(&mut controller, &state, &begin, 6.0);
    assert!(controller
        .dispatch(
            GestureAction::End {
                tag: begin.clone(),
                point: xy(8.0, 0.0)
            },
            &state
        )
        .is_empty());
    let end = end_tag(updated(&mut controller, &state, &first, 1, 5.0), 8.0);
    controller.dispatch(
        GestureAction::Completed {
            tag: end,
            worker_id: 7,
            moved: true,
        },
        &state,
    );
    assert!(!controller.busy());
    assert!(controller.preview().is_none());
    assert_eq!(controller.moved(), Some(true));
    assert_eq!(
        baseline(&state),
        Box::new(state.session.as_ref().unwrap().authoritative.clone())
    );
}

#[test]
fn cancel_wins_during_begin_update_and_end_without_late_resurrection() {
    for phase in 0..3 {
        let state = state();
        let accepted = baseline(&state);
        let mut controller = SketchGestureInteraction::default();
        let begin = begin(&mut controller, &state);
        let pending = match phase {
            0 => begin.clone(),
            1 => {
                ready(&mut controller, &state, &begin);
                update_tag(movement(&mut controller, &state, &begin, 3.0), 3.0)
            }
            _ => {
                ready(&mut controller, &state, &begin);
                end_tag(
                    controller.dispatch(
                        GestureAction::End {
                            tag: begin.clone(),
                            point: xy(4.0, 0.0),
                        },
                        &state,
                    ),
                    4.0,
                )
            }
        };
        let cancel =
            cancel_tag(controller.dispatch(GestureAction::Cancel { tag: begin.clone() }, &state));
        assert_eq!(controller.phase(), Some(GesturePhase::Cancelling));
        assert!(controller.preview().is_none());
        assert!(ready(&mut controller, &state, &begin).is_empty());
        assert!(updated(&mut controller, &state, &pending, 1, 3.0).is_empty());
        controller.dispatch(
            GestureAction::Completed {
                tag: pending,
                worker_id: 7,
                moved: true,
            },
            &state,
        );
        assert_eq!(controller.phase(), Some(GesturePhase::Cancelling));
        controller.dispatch(GestureAction::Cancelled { tag: cancel }, &state);
        assert!(!controller.busy());
        assert_eq!(controller.moved(), None);
        assert_eq!(baseline(&state), accepted);
    }
}

#[test]
fn every_scope_axis_and_accepted_geometry_change_cancel_once() {
    for axis in 0..10 {
        let mut state = state();
        let mut controller = SketchGestureInteraction::default();
        let begin = begin(&mut controller, &state);
        ready(&mut controller, &state, &begin);
        let session = state.session.as_mut().unwrap();
        match axis {
            0 => session.key.generation += 1,
            1 => session.document.revision += 1,
            2 => session.document.snapshot_id.0 += 1,
            3 => session.document.runtime_session.push_str("new"),
            4 => session.edit_session = Some("new-visit".into()),
            5 => session.accepted_capture.as_mut().unwrap().view += 1,
            6 => {
                session
                    .authoritative
                    .apply_solved_positions(&[(eid(0), xy(1.0, 2.0))]);
            }
            7 => {
                state.context = None;
            }
            8 => {
                session.phase = SketchPhase::Closed;
            }
            _ => {
                session.document.scene.scene += 1;
            }
        };
        let cancel = cancel_tag(controller.dispatch(GestureAction::Sync, &state));
        assert!(controller.preview().is_none());
        assert!(controller.dispatch(GestureAction::Sync, &state).is_empty());
        controller.dispatch(GestureAction::Cancelled { tag: cancel }, &state);
        assert!(!controller.busy());
    }
}

#[test]
fn invalid_preview_identity_plane_topology_and_locked_motion_cancel_without_adoption() {
    for axis in 0..7 {
        let state = state();
        let accepted = baseline(&state);
        let mut controller = SketchGestureInteraction::default();
        let begin = begin(&mut controller, &state);
        let mut preview = baseline(&state);
        match axis {
            0 => preview.id = SketchId::new(),
            1 => preview.plane.origin.x += 1.0,
            2 => {
                preview.remove_entity(eid(1));
            }
            3 => {
                preview
                    .add_entity(SketchEntity::point(eid(10), xy(0.0, 0.0), false, false))
                    .unwrap();
            }
            4 => {
                preview.remove_entity(eid(5));
                preview
                    .add_entity(SketchEntity::line(eid(5), eid(0), eid(2), false))
                    .unwrap();
            }
            5 => {
                preview.apply_solved_positions(&[(eid(0), xy(f64::NAN, 0.0))]);
            }
            _ => {
                preview
                    .add_constraint(Constraint::Fixed {
                        id: ids(1000).constraints[0],
                        point: eid(0),
                        point_position: CurvePosition::Arbitrary,
                        at: xy(0.0, 0.0),
                    })
                    .unwrap();
            }
        }
        let effects = controller.dispatch(
            GestureAction::Ready {
                tag: begin,
                worker_id: 7,
                preview,
            },
            &state,
        );
        cancel_tag(effects);
        assert!(controller.preview().is_none());
        assert_eq!(baseline(&state), accepted);
    }
}

#[test]
fn stale_worker_sequence_and_wrong_receipt_tags_cannot_replace_preview_or_release_pending() {
    let state = state();
    let mut controller = SketchGestureInteraction::default();
    let begin = begin(&mut controller, &state);
    ready(&mut controller, &state, &begin);
    let update = update_tag(movement(&mut controller, &state, &begin, 3.0), 3.0);
    for mut tag in [update.clone(), update.clone(), update.clone()] {
        tag.sequence += 1;
        assert!(updated(&mut controller, &state, &tag, 1, 4.0).is_empty());
    }
    assert!(updated(&mut controller, &state, &update, 0, 4.0).is_empty());
    assert_eq!(controller.phase(), Some(GesturePhase::PendingUpdate));
    assert!(controller
        .dispatch(
            GestureAction::Updated {
                tag: update.clone(),
                worker_id: 8,
                worker_sequence: 1,
                preview: shifted(&state, 4.0)
            },
            &state
        )
        .is_empty());
    assert!(updated(&mut controller, &state, &update, 1, 3.0).is_empty());
    assert_eq!(controller.phase(), Some(GesturePhase::Ready));
}

#[test]
fn no_motion_completion_reports_worker_outcome_without_synthetic_edit_or_history() {
    let state = state();
    let accepted = baseline(&state);
    let mut controller = SketchGestureInteraction::default();
    let begin = begin(&mut controller, &state);
    ready(&mut controller, &state, &begin);
    let effects = controller.dispatch(
        GestureAction::End {
            tag: begin,
            point: xy(1.0, 2.0),
        },
        &state,
    );
    let [GestureEffect::End { tag: end, .. }] = effects.as_slice() else {
        panic!("End")
    };
    let end = end.clone();
    controller.dispatch(
        GestureAction::Completed {
            tag: end,
            worker_id: 7,
            moved: false,
        },
        &state,
    );
    assert_eq!(controller.moved(), Some(false));
    assert!(!controller.active() && !controller.busy());
    assert!(controller.preview().is_none());
    assert_eq!(baseline(&state), accepted);
}

#[path = "sketch_gestures/locks.rs"]
mod locks;
