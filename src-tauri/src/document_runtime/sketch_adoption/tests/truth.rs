use super::*;

#[tokio::test]
async fn malformed_end_truth_refuses_document_history_and_redo_mutation() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    runtime
        .sketch_upsert_with_outcome(sketch_id(), moved_ops())
        .await
        .unwrap();
    runtime.undo().unwrap();
    let before = runtime.get_sketch(sketch_id()).unwrap().entities;
    let undo = runtime.undo_depth();
    let redo = runtime.redo_depth();
    runtime
        .begin_gesture(sketch_id(), entity(), wire::GestureTarget::point(entity()))
        .await
        .unwrap();
    *solver.mode.lock().unwrap() = Mode::EndMalformedTruth;
    assert!(runtime.end_gesture(None).await.is_err());
    assert_eq!(runtime.get_sketch(sketch_id()).unwrap().entities, before);
    assert_eq!(runtime.undo_depth(), undo);
    assert_eq!(runtime.redo_depth(), redo);
}

#[test]
fn static_sketch_read_preserves_truth_only_for_matching_geometry() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    let id = onecad_core::ids::ConstraintId(Uuid::from_u128(30));
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: sketch_id(),
            ops: vec![SketchEditOp::AddConstraint {
                constraint: onecad_core::sketch::Constraint::Fixed {
                    id,
                    point: entity(),
                    point_position: onecad_core::sketch::CurvePosition::Arbitrary,
                    at: Vec2::new(1.0, 1.0).unwrap(),
                },
            }],
        })
        .unwrap();
    let mut solved = response(sketch_id(), 1);
    solved.status = SketchSolveStatus::Conflicting;
    solved.conflicting = vec![id.to_string()];
    solved.truth.reasons.insert(
        id.to_string(),
        crate::dto::SketchConflictReason::RankConflict,
    );
    runtime.record_solve(sketch_id(), &solved);
    let cached = runtime.get_sketch(sketch_id()).unwrap();
    assert!(cached.solve_current);
    assert_eq!(cached.truth, solved.truth);
    assert_eq!(cached.conflicting, solved.conflicting);
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: sketch_id(),
            ops: moved_ops(),
        })
        .unwrap();
    let changed = runtime.get_sketch(sketch_id()).unwrap();
    assert!(!changed.solve_current);
    assert!(changed.truth.reasons.is_empty() && changed.conflicting.is_empty());
}
