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
