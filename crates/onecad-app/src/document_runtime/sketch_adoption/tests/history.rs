use super::*;
use crate::dto::SketchHistoryScope;
use onecad_core::edit::HistoryDirection;

fn scope(runtime: &DocumentRuntime) -> SketchHistoryScope {
    SketchHistoryScope {
        sketch_id: sketch_id(),
        edit_session: runtime
            .get_sketch(sketch_id())
            .unwrap()
            .edit_session
            .unwrap(),
        runtime_session: runtime.runtime_session().to_string(),
        revision: runtime.revision().0,
        snapshot_id: runtime.head_snapshot_id().unwrap_or(SnapshotId(0)),
    }
}

fn edit(runtime: &mut DocumentRuntime) {
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: sketch_id(),
            ops: vec![SketchEditOp::AddEntity {
                entity: SketchEntity::point(
                    EntityId(Uuid::from_u128(99)),
                    Vec2::new_unchecked(4.0, 0.0),
                    false,
                    false,
                ),
            }],
        })
        .unwrap();
}

#[tokio::test]
async fn scoped_history_boundary_empty_redo_and_foreign_model_replay_never_cross_visit() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    let context = scope(&runtime);
    assert!(runtime
        .prepare_sketch_history(context.clone(), HistoryDirection::Undo)
        .await
        .unwrap()
        .is_none());
    assert!(runtime
        .prepare_sketch_history(context, HistoryDirection::Redo)
        .await
        .unwrap()
        .is_none());
    edit(&mut runtime);
    runtime.undo().unwrap();
    assert!(runtime
        .prepare_sketch_history(scope(&runtime), HistoryDirection::Redo)
        .await
        .is_err());
}

#[tokio::test]
async fn failed_history_solve_or_stale_fences_preserve_document_and_both_stacks() {
    for mode in [
        Mode::Failure,
        Mode::Foreign,
        Mode::Nonfinite,
        Mode::Conflicting,
        Mode::OverConstrained,
        Mode::StaleEpoch,
        Mode::StaleRevision,
        Mode::WorkerRestart,
    ] {
        let (mut runtime, solver) = runtime(Mode::Unchanged);
        runtime.enter_sketch(sketch_id()).await.unwrap();
        edit(&mut runtime);
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let depth = (runtime.undo_depth(), runtime.redo_depth());
        let prepared = runtime
            .prepare_sketch_history(scope(&runtime), HistoryDirection::Undo)
            .await
            .unwrap()
            .unwrap();
        *solver.mode.lock().unwrap() = mode;
        let result = prepared
            .drive()
            .await
            .and_then(|driven| runtime.commit_sketch_history(driven));
        assert!(result.is_err());
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
        assert_eq!((runtime.undo_depth(), runtime.redo_depth()), depth);
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
}

#[tokio::test]
async fn cancel_new_visit_and_restart_before_prepare_reject_old_history_scope() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    let old = scope(&runtime);
    runtime.cancel_sketch(sketch_id(), false).await.unwrap();
    runtime.enter_sketch(sketch_id()).await.unwrap();
    assert_ne!(old.edit_session, scope(&runtime).edit_session);
    assert!(runtime
        .prepare_sketch_history(old, HistoryDirection::Undo)
        .await
        .is_err());
    edit(&mut runtime);
    let context = scope(&runtime);
    solver.backend.epoch.store(2, Ordering::SeqCst);
    runtime.fencing.set_epoch(2);
    assert!(runtime
        .prepare_sketch_history(context, HistoryDirection::Undo)
        .await
        .is_err());
}

#[tokio::test]
async fn changed_edit_visit_token_rejects_detached_history_and_constraint_solves() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    edit(&mut runtime);
    let prepared = runtime
        .prepare_sketch_history(scope(&runtime), HistoryDirection::Undo)
        .await
        .unwrap()
        .unwrap();
    let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    let depth = (runtime.undo_depth(), runtime.redo_depth());
    runtime.sketch_session.as_mut().unwrap().token = Uuid::new_v4();
    let driven = prepared.drive().await.unwrap();
    assert!(runtime.commit_sketch_history(driven).is_err());
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        before
    );
    assert_eq!((runtime.undo_depth(), runtime.redo_depth()), depth);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}

fn fill_model_history(runtime: &mut DocumentRuntime) {
    for index in 0..onecad_core::edit::undo::UNDO_CAP {
        runtime
            .apply(EditCommand::RenameSketch {
                sketch: sketch_id(),
                name: format!("History {index}"),
            })
            .unwrap();
    }
}

async fn history_step(runtime: &mut DocumentRuntime, direction: HistoryDirection) -> bool {
    let Some(prepared) = runtime
        .prepare_sketch_history(scope(runtime), direction)
        .await
        .unwrap()
    else {
        return false;
    };
    let driven = prepared.drive().await.unwrap();
    runtime.commit_sketch_history(driven).unwrap().0.moved
}

fn move_point(runtime: &mut DocumentRuntime, x: f64) {
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: sketch_id(),
            ops: vec![SketchEditOp::SetEntityPositions {
                positions: vec![(entity(), Vec2::new_unchecked(x, 1.0))],
            }],
        })
        .unwrap();
}

#[tokio::test]
async fn full_model_history_keeps_fresh_sketch_undo_redo_available() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    fill_model_history(&mut runtime);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    let baseline = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    edit(&mut runtime);
    assert_eq!(
        runtime
            .get_sketch(sketch_id())
            .unwrap()
            .history
            .unwrap()
            .undo_depth,
        1
    );
    for direction in [
        HistoryDirection::Undo,
        HistoryDirection::Redo,
        HistoryDirection::Undo,
    ] {
        let prepared = runtime
            .prepare_sketch_history(scope(&runtime), direction)
            .await
            .unwrap()
            .unwrap();
        let driven = prepared.drive().await.unwrap();
        runtime.commit_sketch_history(driven).unwrap();
    }
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        baseline
    );
    assert!(runtime
        .prepare_sketch_history(scope(&runtime), HistoryDirection::Undo)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn prefix_eviction_partial_undo_then_finish_squashes_only_applied_visit_edits() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    fill_model_history(&mut runtime);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    let baseline = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    edit(&mut runtime);
    let kept = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    move_point(&mut runtime, 8.0);
    assert!(history_step(&mut runtime, HistoryDirection::Undo).await);
    runtime.finish_sketch(sketch_id()).await.unwrap();
    assert_eq!(runtime.undo_depth(), 200);
    assert_eq!(runtime.redo_depth(), 0);
    assert_eq!(runtime.sketch_snapshot(sketch_id(), "test").unwrap(), kept);
    runtime.undo().unwrap(); // The minted sketch record is a separate model step.
    runtime.undo().unwrap();
    assert_eq!(runtime.undo_depth(), 198);
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        baseline
    );
    runtime.redo().unwrap().unwrap();
    assert_eq!(runtime.sketch_snapshot(sketch_id(), "test").unwrap(), kept);
}

#[tokio::test]
async fn prefix_eviction_partial_undo_then_discard_preserves_model_history() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    fill_model_history(&mut runtime);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    let baseline = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    edit(&mut runtime);
    move_point(&mut runtime, 8.0);
    assert!(history_step(&mut runtime, HistoryDirection::Undo).await);
    assert!(
        runtime
            .cancel_sketch(sketch_id(), true)
            .await
            .unwrap()
            .discarded
    );
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        baseline
    );
    assert_eq!((runtime.undo_depth(), runtime.redo_depth()), (198, 0));
    runtime.undo().unwrap();
    assert!(
        runtime
            .sketch_snapshot(sketch_id(), "test")
            .unwrap()
            .entities()
            == baseline.entities()
    );
}

#[tokio::test]
async fn created_sketch_discard_uses_shifted_creation_index_after_prefix_eviction() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    fill_model_history(&mut runtime);
    let original = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    let created = SketchId(Uuid::from_u128(500));
    runtime
        .apply(EditCommand::AddSketch {
            sketch: Sketch::on_world_plane(created, "Created at cap", WorldPlane::XY),
        })
        .unwrap();
    runtime.enter_sketch(created).await.unwrap();
    runtime.mark_open_sketch_created(created);
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: created,
            ops: vec![SketchEditOp::AddEntity {
                entity: SketchEntity::point(
                    EntityId(Uuid::from_u128(501)),
                    Vec2::new_unchecked(1.0, 1.0),
                    false,
                    false,
                ),
            }],
        })
        .unwrap();
    assert!(
        runtime
            .cancel_sketch(created, true)
            .await
            .unwrap()
            .discarded
    );
    assert!(runtime.get_sketch(created).is_err());
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        original
    );
    assert_eq!((runtime.undo_depth(), runtime.redo_depth()), (198, 0));
}

#[tokio::test]
async fn long_visit_finish_keeps_retained_steps_without_squashing_lost_visit_edits() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    for index in 1..=205 {
        move_point(&mut runtime, f64::from(index));
    }
    let edited = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    runtime.finish_sketch(sketch_id()).await.unwrap();
    assert!(runtime.sketch_session.is_none());
    assert_eq!((runtime.undo_depth(), runtime.redo_depth()), (200, 0));
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        edited
    );
    runtime.undo().unwrap();
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        edited
    );
    runtime.undo().unwrap();
    assert_ne!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        edited
    );
}

#[tokio::test]
async fn long_visit_history_rolls_without_crossing_retained_boundary_or_corrupting_discard() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    let mut retained_floor = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    for index in 1..=205 {
        move_point(&mut runtime, f64::from(index));
        if index == 5 {
            retained_floor = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        }
    }
    let edited = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    assert_eq!(
        runtime
            .get_sketch(sketch_id())
            .unwrap()
            .history
            .unwrap()
            .undo_depth,
        200
    );
    for _ in 0..200 {
        assert!(history_step(&mut runtime, HistoryDirection::Undo).await);
    }
    assert!(!history_step(&mut runtime, HistoryDirection::Undo).await);
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        retained_floor
    );
    for _ in 0..200 {
        assert!(history_step(&mut runtime, HistoryDirection::Redo).await);
    }
    assert!(!history_step(&mut runtime, HistoryDirection::Redo).await);
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        edited
    );
    let outcome = runtime.cancel_sketch(sketch_id(), true).await.unwrap();
    assert!(!outcome.discarded);
    assert_eq!(outcome.kept_reason, Some("history was trimmed"));
    assert!(runtime.sketch_session.is_none());
    assert_eq!((runtime.undo_depth(), runtime.redo_depth()), (200, 0));
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        edited
    );
}
