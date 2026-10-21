use onecad_app::dto::CancelSketchDto;
use onecad_core::{
    edit::{EditCommand, SketchEditOp},
    ids::SketchId,
    math::Vec2,
    sketch::{Constraint, SketchEntity, SketchPlane},
};
use onecad_interaction::{sketch::*, UndoRoute};
use onecad_scene::{Camera, DVec2, SceneIdentity, ViewSnapshot};
use uuid::Uuid;
#[path = "../src/sketch/capture_tests.rs"]
mod capture_tests;
#[path = "../src/sketch/test_support.rs"]
mod support;
use support::*;

#[test]
fn rectangle_preview_is_local_then_one_closed_shared_endpoint_batch_is_accepted() {
    let mut state = active(SketchPlane::xy(), SketchTool::Rectangle);
    assert!(click(&mut state, 0.0, 0.0, 1000).is_empty());
    let cursor = point(&state, 30.0, 20.0);
    assert!(state.dispatch(SketchAction::Move(cursor)).is_empty());
    assert!(matches!(
        state.session.as_ref().unwrap().preview,
        Some(DraftPrimitive::Rectangle { .. })
    ));
    assert!(state
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .entities()
        .is_empty());
    let (tag, ops) = upsert(click(&mut state, 30.0, 20.0, 2000));
    assert_eq!(ops.len(), 12);
    let snapshot = applied(&state, &ops);
    assert_eq!(snapshot.sketch.entities().len(), 8);
    assert_eq!(snapshot.sketch.constraints().len(), 4);
    let lines: Vec<_> = snapshot
        .sketch
        .entities()
        .iter()
        .filter_map(|e| match e {
            SketchEntity::Line { start, end, .. } => Some((*start, *end)),
            _ => None,
        })
        .collect();
    for i in 0..4 {
        assert_eq!(lines[i].1, lines[(i + 1) % 4].0);
    }
    state.dispatch(SketchAction::BatchAccepted { tag: tag.clone() });
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Accepted);
    assert!(state
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .entities()
        .is_empty());
    complete(&mut state, tag, snapshot);
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
    assert!(state.session.as_ref().unwrap().draft.is_none());
}

#[test]
fn line_chain_reuses_solved_endpoints_and_closes_by_point_identity() {
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, snapshot) = primitive(&mut state, 10.0, 0.0, 2000);
    complete(&mut state, tag, snapshot);
    let first = state
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .first_id
        .unwrap();
    let tail = state
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .anchor_id
        .unwrap();
    let (tag, ops) = upsert(click(&mut state, 10.0, 10.0, 3000));
    assert_eq!(ops.len(), 2);
    assert!(
        matches!(&ops[1],SketchEditOp::AddEntity {entity:SketchEntity::Line{start,..}} if *start==tail)
    );
    let snapshot = applied(&state, &ops);
    complete(&mut state, tag, snapshot);
    let (tag, ops) = upsert(click(&mut state, 0.0, 0.0, 4000));
    assert_eq!(ops.len(), 1);
    assert!(
        matches!(&ops[0],SketchEditOp::AddEntity {entity:SketchEntity::Line{end,..}} if *end==first)
    );
    let snapshot = applied(&state, &ops);
    complete(&mut state, tag, snapshot);
    assert_eq!(
        state
            .session
            .as_ref()
            .unwrap()
            .authoritative
            .entities()
            .len(),
        6
    );
    assert!(state.session.as_ref().unwrap().draft.is_none());
}

#[test]
fn line_anchor_comes_from_authoritative_solve_not_original_pointer() {
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, mut snapshot) = primitive(&mut state, 10.0, 0.0, 2000);
    let end = ids(2000).entities[1];
    snapshot
        .sketch
        .apply_solved_positions(&[(end, Vec2::new(12.0, 3.0).unwrap())]);
    complete(&mut state, tag, snapshot);
    assert_eq!(
        state
            .session
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .anchor,
        Vec2::new(12.0, 3.0).unwrap()
    );
    let (tag, ops) = upsert(click(&mut state, 20.0, 3.0, 3000));
    assert_eq!(
        tag.key.sketch_id,
        state.session.as_ref().unwrap().key.sketch_id
    );
    assert!(
        matches!(&ops[1],SketchEditOp::AddEntity{entity:SketchEntity::Line{start,..}} if *start==end)
    );
}

#[test]
fn circle_uses_geometric_radius_and_one_two_entity_batch() {
    let mut state = active(SketchPlane::xz(), SketchTool::Circle);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, ops) = upsert(click(&mut state, 3.0, 4.0, 2000));
    assert_eq!(ops.len(), 2);
    assert!(
        matches!(&ops[1],SketchEditOp::AddEntity{entity:SketchEntity::Circle{center,radius,..}} if *center==ids(2000).entities[0] && (*radius-5.0).abs()<1e-10)
    );
    let snapshot = applied(&state, &ops);
    complete(&mut state, tag, snapshot);
    assert!(state.session.as_ref().unwrap().draft.is_none());
}

#[test]
fn degenerate_primitives_do_not_author_geometry() {
    for tool in [SketchTool::Rectangle, SketchTool::Circle] {
        let mut state = active(SketchPlane::xy(), tool);
        click(&mut state, 0.0, 0.0, 1000);
        assert!(click(&mut state, 0.0, 0.0, 2000).is_empty());
        assert!(matches!(
            state.session.as_ref().unwrap().status,
            SketchStatus::Refused(_)
        ));
        assert!(state.session.as_ref().unwrap().draft.is_some());
        assert!(state
            .session
            .as_ref()
            .unwrap()
            .authoritative
            .entities()
            .is_empty());
    }
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    click(&mut state, 0.0, 0.0, 1000);
    assert!(click(&mut state, 0.0, 0.0, 2000).is_empty());
    assert!(state.session.as_ref().unwrap().draft.is_none());
}

#[test]
fn escape_unwinds_to_idle_and_only_enter_finishes_without_upserting_preview() {
    let mut state = active(SketchPlane::xy(), SketchTool::Rectangle);
    click(&mut state, 0.0, 0.0, 1000);
    assert!(state.dispatch(SketchAction::Enter).is_empty());
    assert!(state.session.as_ref().unwrap().draft.is_none());
    assert!(state.dispatch(SketchAction::Escape).is_empty());
    assert_eq!(state.session.as_ref().unwrap().tool, SketchTool::Select);
    assert!(state.dispatch(SketchAction::Escape).is_empty());
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
    let effects = state.dispatch(SketchAction::Enter);
    assert!(matches!(
        effects.as_slice(),
        [SketchEffect::FinishSketch {
            reason: FinishReason::Explicit,
            ..
        }]
    ));
    assert!(state.dispatch(SketchAction::Finish).is_empty());
    assert!(state.dispatch(SketchAction::Enter).is_empty());
}

#[test]
fn duplicate_confirmation_and_discard_wait_for_real_batch_then_do_not_finish() {
    let mut state = active(SketchPlane::xy(), SketchTool::Circle);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, snapshot) = primitive(&mut state, 3.0, 4.0, 2000);
    assert!(click(&mut state, 3.0, 4.0, 3000).is_empty());
    assert!(state.dispatch(SketchAction::Discard).is_empty());
    let effects = complete(&mut state, tag, snapshot);
    let [SketchEffect::DiscardSketch { tag }] = effects.as_slice() else {
        panic!()
    };
    assert!(state
        .dispatch(SketchAction::Discarded {
            tag: tag.clone(),
            result: CancelSketchDto {
                discarded: true,
                kept_reason: None
            }
        })
        .is_empty());
    assert!(matches!(
        state.session.as_ref().unwrap().status,
        SketchStatus::Discarded(_)
    ));
}

#[test]
fn discard_refusal_keeps_authoring_and_finishes_with_exact_reason() {
    let mut state = active(SketchPlane::xy(), SketchTool::Select);
    let effects = state.dispatch(SketchAction::Discard);
    let [SketchEffect::DiscardSketch { tag }] = effects.as_slice() else {
        panic!()
    };
    let effects = state.dispatch(SketchAction::Discarded {
        tag: tag.clone(),
        result: CancelSketchDto {
            discarded: false,
            kept_reason: Some("guard changed".into()),
        },
    });
    assert!(matches!(
        effects.as_slice(),
        [SketchEffect::FinishSketch {
            reason: FinishReason::DiscardRefused,
            ..
        }]
    ));
}

#[test]
fn stale_session_and_new_document_drop_old_pointer_and_solver_reply() {
    let mut state = active(SketchPlane::xy(), SketchTool::Circle);
    let old_point = point(&state, 0.0, 0.0);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, snapshot) = primitive(&mut state, 3.0, 4.0, 2000);
    let mut next = context();
    next.runtime_session = "runtime-2".into();
    state.dispatch(SketchAction::ContextChanged(Some(next)));
    assert!(state
        .dispatch(SketchAction::Solved {
            tag,
            snapshot,
            solve: Box::new(solve_dto(SketchId(Uuid::from_u128(1))))
        })
        .is_empty());
    assert!(state
        .dispatch(SketchAction::Click {
            point: old_point,
            ids: ids(3000)
        })
        .is_empty());
    assert!(state.session.is_none());
}

#[test]
fn solver_before_acceptance_is_buffered_and_old_revision_snapshot_is_not_adopted() {
    let mut state = active(SketchPlane::xy(), SketchTool::Circle);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, snapshot) = primitive(&mut state, 3.0, 4.0, 2000);
    let solve = Box::new(solve_dto(snapshot.sketch.id));
    state.dispatch(SketchAction::Solved {
        tag: tag.clone(),
        snapshot,
        solve,
    });
    assert!(state
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .entities()
        .is_empty());
    let mut next = context();
    next.revision += 1;
    state.dispatch(SketchAction::ContextChanged(Some(next)));
    state.dispatch(SketchAction::BatchAccepted { tag });
    assert!(matches!(
        state.session.as_ref().unwrap().status,
        SketchStatus::Failed {
            accepted: Some(true),
            ..
        }
    ));
}

#[test]
fn constraints_are_one_typed_batch_and_invalid_refs_are_explicit_refusals() {
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, snapshot) = primitive(&mut state, 10.0, 0.0, 2000);
    complete(&mut state, tag, snapshot);
    let key = state.session.as_ref().unwrap().key.clone();
    let bad = Constraint::Horizontal {
        id: ids(4000).constraints[0],
        line: ids(4000).entities[2],
    };
    assert!(state
        .dispatch(SketchAction::AddConstraints {
            key: key.clone(),
            constraints: vec![bad]
        })
        .is_empty());
    assert!(matches!(
        state.session.as_ref().unwrap().status,
        SketchStatus::Refused(_)
    ));
    let good = Constraint::Horizontal {
        id: ids(4000).constraints[0],
        line: ids(2000).entities[2],
    };
    let effects = state.dispatch(SketchAction::AddConstraints {
        key,
        constraints: vec![good.clone()],
    });
    let [SketchEffect::AddConstraints { tag, constraints }] = effects.as_slice() else {
        panic!("strict constraint effect required")
    };
    assert_eq!(constraints, &[good]);
    let tag = tag.clone();
    let before = state.session.as_ref().unwrap().authoritative.clone();
    let selection = state.session.as_ref().unwrap().selection.clone();
    state.dispatch(SketchAction::ConstraintRefused {
        tag: tag.clone(),
        message: "Conflicting".into(),
    });
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.phase, SketchPhase::Active);
    assert_eq!(session.authoritative, before);
    assert_eq!(session.selection, selection);
    assert!(matches!(&session.status, SketchStatus::Refused(message) if message == "Conflicting"));
    state.dispatch(SketchAction::ConstraintRefused {
        tag,
        message: "stale".into(),
    });
    assert!(
        matches!(&state.session.as_ref().unwrap().status, SketchStatus::Refused(message) if message == "Conflicting")
    );
}

#[test]
fn scoped_selection_and_text_undo_do_not_escape_sketch_session() {
    let mut state = active(SketchPlane::xy(), SketchTool::Circle);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, snapshot) = primitive(&mut state, 3.0, 4.0, 2000);
    complete(&mut state, tag, snapshot);
    let key = state.session.as_ref().unwrap().key.clone();
    let document = state.session.as_ref().unwrap().document.clone();
    let capture = view(&state).identity();
    state.dispatch(SketchAction::AcceptView {
        key: key.clone(),
        document: document.clone(),
        capture,
    });
    state.dispatch(SketchAction::Select {
        key: key.clone(),
        document: document.clone(),
        capture,
        targets: vec![SketchTarget::Entity(ids(2000).entities[1])],
        mode: SelectionMode::Replace,
    });
    let mut stale = key;
    stale.generation += 1;
    state.dispatch(SketchAction::Select {
        key: stale,
        document,
        capture,
        targets: vec![],
        mode: SelectionMode::Replace,
    });
    assert_eq!(state.session.as_ref().unwrap().selection.len(), 1);
    assert_eq!(state.text_undo_route(true), UndoRoute::Text);
    assert_eq!(state.text_undo_route(false), UndoRoute::Sketch);
    closed(&mut state);
    assert_eq!(state.text_undo_route(false), UndoRoute::Model);
}

#[test]
fn captured_pointer_matches_onecad_non_xy_and_skewed_plane_coordinates() {
    let mut skew = SketchPlane::xy();
    skew.x_axis = onecad_core::math::Vec3::new(0.0, 2.0, 0.0).unwrap();
    skew.y_axis = onecad_core::math::Vec3::new(-3.0, 1.0, 0.0).unwrap();
    for plane in [
        SketchPlane::xy(),
        SketchPlane::xz(),
        SketchPlane::yz(),
        skew,
    ] {
        let state = active(plane, SketchTool::Line);
        let p = point(&state, 12.5, -7.25);
        assert!((p.point.x - 12.5).abs() < 1e-10);
        assert!((p.point.y + 7.25).abs() < 1e-10);
        let other = view(&state);
        let wrong = ViewSnapshot::new(
            SceneIdentity {
                session: 999,
                scene: 1,
            },
            1,
            Camera::default(),
            other.viewport(),
        )
        .unwrap();
        assert_eq!(
            capture_sketch_pointer(state.session.as_ref().unwrap(), &wrong, DVec2::ZERO)
                .unwrap_err(),
            SketchPointerError::CaptureMismatch
        );
    }
}

#[test]
fn create_then_enter_and_reenter_keep_entity_ids_but_change_session_generation() {
    let mut state = SketchInteraction::default();
    state.dispatch(SketchAction::ContextChanged(Some(context())));
    let sketch = empty(SketchPlane::xy());
    let effects = state.dispatch(SketchAction::Begin {
        sketch: Box::new(sketch.clone()),
        create: true,
        context: context(),
    });
    let [SketchEffect::CreateSketch { tag, command }] = effects.as_slice() else {
        panic!()
    };
    assert!(
        matches!(command.as_ref(),EditCommand::AddSketch{sketch:created} if created.id==sketch.id)
    );
    let old = tag.clone();
    let effects = state.dispatch(SketchAction::Created { tag: old.clone() });
    let [SketchEffect::EnterSketch {
        tag,
        created_here: true,
    }] = effects.as_slice()
    else {
        panic!()
    };
    state.dispatch(SketchAction::Entered {
        tag: tag.clone(),
        snapshot: SketchSnapshot {
            history: None,
            document: context(),
            sketch: Box::new(sketch.clone()),
        },
        solve: Box::new(enter_dto(sketch.id)),
    });
    closed(&mut state);
    let effects = state.dispatch(SketchAction::Begin {
        sketch: Box::new(sketch),
        create: false,
        context: context(),
    });
    let [SketchEffect::EnterSketch { tag, .. }] = effects.as_slice() else {
        panic!()
    };
    assert_ne!(tag.key.generation, old.key.generation);
    assert!(state
        .dispatch(SketchAction::Created { tag: old })
        .is_empty());
}

#[test]
fn sketch_history_cancels_only_transient_draft_before_requesting_authoritative_history() {
    use onecad_core::edit::HistoryDirection;
    let mut state = active(SketchPlane::xy(), SketchTool::Circle);
    click(&mut state, 1.0, 2.0, 1000);
    let before = state.session.as_ref().unwrap().authoritative.clone();
    assert!(state
        .dispatch(SketchAction::History(HistoryDirection::Undo))
        .is_empty());
    assert!(state.session.as_ref().unwrap().draft.is_none());
    assert_eq!(state.session.as_ref().unwrap().authoritative, before);
    let effects = state.dispatch(SketchAction::History(HistoryDirection::Undo));
    assert!(
        matches!(effects.as_slice(), [SketchEffect::History { edit_session, direction: HistoryDirection::Undo, .. }] if edit_session == "edit-visit")
    );
    assert_eq!(
        state.session.as_ref().unwrap().phase,
        SketchPhase::Submitting
    );
    assert!(state
        .dispatch(SketchAction::History(HistoryDirection::Redo))
        .is_empty());
}

#[test]
fn sketch_history_reply_preserves_scope_and_empty_history_is_explicit() {
    use onecad_core::edit::HistoryDirection;
    let mut state = active(SketchPlane::xy(), SketchTool::Select);
    let effect = state.dispatch(SketchAction::History(HistoryDirection::Undo));
    let SketchEffect::History { tag, .. } = &effect[0] else {
        panic!()
    };
    let tag = tag.clone();
    let session = state.session.as_ref().unwrap();
    let snapshot = SketchSnapshot {
        history: Some(onecad_app::dto::SketchHistoryAvailability {
            undo_depth: 0,
            redo_depth: 0,
        }),
        document: session.document.clone(),
        sketch: Box::new(session.authoritative.clone()),
    };
    state.dispatch(SketchAction::HistoryResolved {
        tag: tag.clone(),
        snapshot: snapshot.clone(),
        solve: Box::new(enter_dto(tag.key.sketch_id)),
        moved: false,
        direction: HistoryDirection::Undo,
    });
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
    assert!(
        matches!(&state.session.as_ref().unwrap().status, SketchStatus::Refused(message) if message == "No sketch edits to undo")
    );
    state.dispatch(SketchAction::ContextChanged(None));
    state.dispatch(SketchAction::ContextChanged(Some(context())));
    let mut sketch = empty(SketchPlane::xy());
    sketch.name = "New visit".into();
    let effects = state.dispatch(SketchAction::Begin {
        sketch: Box::new(sketch),
        create: false,
        context: context(),
    });
    assert!(!effects.is_empty());
    state.dispatch(SketchAction::HistoryResolved {
        tag,
        snapshot,
        solve: Box::new(enter_dto(SketchId(Uuid::from_u128(1)))),
        moved: true,
        direction: HistoryDirection::Undo,
    });
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Entering);
    assert_eq!(
        state.session.as_ref().unwrap().authoritative.name,
        "New visit"
    );
}

#[test]
fn application_created_sketch_enters_without_repeating_creation_and_keeps_discard_scope() {
    let mut state = SketchInteraction::default();
    state.dispatch(SketchAction::ContextChanged(Some(context())));
    let sketch = empty(SketchPlane::xy());
    let effects = state.dispatch(SketchAction::BeginCreated {
        sketch: Box::new(sketch.clone()),
        context: context(),
    });
    let [SketchEffect::EnterSketch {
        tag,
        created_here: true,
    }] = effects.as_slice()
    else {
        panic!("already-created sketch must enter without another AddSketch");
    };
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Entering);
    state.dispatch(SketchAction::Entered {
        tag: tag.clone(),
        snapshot: SketchSnapshot {
            history: None,
            document: context(),
            sketch: Box::new(sketch.clone()),
        },
        solve: Box::new(enter_dto(sketch.id)),
    });
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
    assert!(matches!(
        state.dispatch(SketchAction::Discard).as_slice(),
        [SketchEffect::DiscardSketch { .. }]
    ));
}
