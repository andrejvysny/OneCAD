use super::*;
use onecad_core::sketch::{Constraint, CurvePosition};

#[path = "dimensions.rs"]
mod dimensions;
#[path = "primitives.rs"]
mod primitives;
#[path = "repairs.rs"]
mod repairs;

fn fixed() -> Constraint {
    Constraint::Fixed {
        id: onecad_core::ids::ConstraintId(Uuid::from_u128(900)),
        point: entity(),
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new(1.0, 1.0).unwrap(),
    }
}

#[tokio::test]
async fn strict_constraint_refusals_preserve_authoritative_state_and_cached_solve() {
    for mode in [Mode::OverConstrained, Mode::Conflicting] {
        let (mut runtime, solver) = runtime(Mode::Unchanged);
        runtime.enter_sketch(sketch_id()).await.unwrap();
        let before = runtime.get_sketch(sketch_id()).unwrap();
        let projection = runtime.projection();
        let undo = runtime.undo_depth();
        *solver.mode.lock().unwrap() = mode;
        let (result, outcome) = runtime
            .add_sketch_constraints_with_outcome(sketch_id(), vec![fixed()])
            .await
            .unwrap();
        assert!(!result.accepted && outcome.is_none());
        assert_eq!(runtime.projection(), projection);
        assert_eq!(runtime.undo_depth(), undo);
        assert_eq!(
            serde_json::to_value(runtime.get_sketch(sketch_id()).unwrap()).unwrap(),
            serde_json::to_value(before).unwrap()
        );
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
}

#[tokio::test]
async fn strict_constraint_solve_errors_stale_replies_and_nan_never_author() {
    for mode in [
        Mode::Failure,
        Mode::Foreign,
        Mode::Nonfinite,
        Mode::NonfiniteTruth,
        Mode::StaleRevision,
        Mode::StaleEpoch,
        Mode::WorkerRestart,
    ] {
        let (mut runtime, _) = runtime(mode);
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let depth = runtime.undo_depth();
        assert!(runtime
            .add_sketch_constraints_with_outcome(sketch_id(), vec![fixed()])
            .await
            .is_err());
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
        assert_eq!(runtime.undo_depth(), depth);
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
}

#[tokio::test]
async fn detached_constraint_claim_survives_drive_and_stale_refusal_is_not_returned() {
    for mode in [Mode::Unchanged, Mode::Conflicting] {
        let (mut runtime, _) = runtime(mode);
        let prepared = runtime
            .prepare_sketch_constraints(sketch_id(), vec![fixed()])
            .await
            .unwrap();
        assert!(runtime
            .prepare_sketch_constraints(sketch_id(), vec![fixed()])
            .await
            .is_err());
        assert!(runtime.prepare_sketch_regions(sketch_id()).is_err());
        let driven = prepared.drive().await.unwrap();
        assert!(runtime
            .prepare_sketch_constraints(sketch_id(), vec![fixed()])
            .await
            .is_err());
        assert!(runtime.prepare_sketch_regions(sketch_id()).is_err());
        runtime.fencing.bump_revision();
        assert!(runtime.commit_sketch_constraints(driven).is_err());
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
}

#[tokio::test]
async fn constraint_commit_rejects_runtime_replacement_and_canceled_prepare_releases_claim() {
    let (runtime, _) = runtime(Mode::Unchanged);
    let prepared = runtime
        .prepare_sketch_constraints(sketch_id(), vec![fixed()])
        .await
        .unwrap();
    drop(prepared);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    let driven = runtime
        .prepare_sketch_constraints(sketch_id(), vec![fixed()])
        .await
        .unwrap()
        .drive()
        .await
        .unwrap();
    let (mut replacement, _) = super::runtime(Mode::Unchanged);
    assert!(replacement.commit_sketch_constraints(driven).is_err());
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}

#[tokio::test]
async fn invalid_expression_read_only_and_replayed_solve_are_not_admitted() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    for expression in ["missing * 2", "45deg", "1 / 0"] {
        let constraint = Constraint::Radius {
            id: onecad_core::ids::ConstraintId(Uuid::from_u128(900)),
            entity: entity(),
            value: onecad_core::document::variables::Scalar::with_expr(5.0, expression),
        };
        assert!(runtime
            .prepare_sketch_constraints(sketch_id(), vec![constraint])
            .await
            .is_err());
    }
    assert_eq!(solver.calls.load(Ordering::SeqCst), 0);
    runtime.read_only = true;
    assert!(runtime
        .prepare_sketch_constraints(sketch_id(), vec![fixed()])
        .await
        .is_err());
    runtime.read_only = false;
    runtime.enter_sketch(sketch_id()).await.unwrap();
    *solver.mode.lock().unwrap() = Mode::ReplayedRevision;
    let before = runtime.projection();
    assert!(runtime
        .add_sketch_constraints_with_outcome(sketch_id(), vec![fixed()])
        .await
        .is_err());
    assert_eq!(runtime.projection(), before);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}

#[tokio::test]
async fn constraint_bounds_and_duplicate_ids_refuse_before_worker_execution() {
    let (runtime, solver) = runtime(Mode::Unchanged);
    for constraints in [vec![], vec![fixed(); 129], vec![fixed(), fixed()]] {
        assert!(runtime
            .prepare_sketch_constraints(sketch_id(), constraints)
            .await
            .is_err());
    }
    assert_eq!(solver.calls.load(Ordering::SeqCst), 0);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}

#[tokio::test]
async fn detached_constraints_cannot_outlive_cancel_or_a_new_visit_without_revision_change() {
    for discard in [false, true] {
        let (mut runtime, _) = runtime(Mode::Unchanged);
        runtime.enter_sketch(sketch_id()).await.unwrap();
        let revision = runtime.revision();
        let prepared = runtime
            .prepare_sketch_constraints(sketch_id(), vec![fixed()])
            .await
            .unwrap();
        runtime.cancel_sketch(sketch_id(), discard).await.unwrap();
        assert_eq!(runtime.revision(), revision);
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let depth = runtime.undo_depth();
        let driven = prepared.drive().await.unwrap();
        assert!(runtime.commit_sketch_constraints(driven).is_err());
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
        assert_eq!(runtime.undo_depth(), depth);
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
    let (mut runtime, _) = runtime(Mode::Unchanged);
    let old = runtime
        .enter_sketch(sketch_id())
        .await
        .unwrap()
        .edit_session;
    let prepared = runtime
        .prepare_sketch_constraints(sketch_id(), vec![fixed()])
        .await
        .unwrap();
    runtime.cancel_sketch(sketch_id(), false).await.unwrap();
    let driven = prepared.drive().await.unwrap();
    let revision = runtime.revision();
    let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    // New visits mint a token even when entering changes no geometry or revision.
    let new = runtime.enter_sketch(sketch_id()).await;
    assert!(new.is_err(), "the detached solve still owns its lane");
    assert!(runtime.commit_sketch_constraints(driven).is_err());
    let new = runtime
        .enter_sketch(sketch_id())
        .await
        .unwrap()
        .edit_session;
    assert_ne!(new, old);
    assert_eq!(runtime.revision(), revision);
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        before
    );
}

#[tokio::test]
async fn detached_constraint_commit_rejects_a_changed_visit_token() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    let prepared = runtime
        .prepare_sketch_constraints(sketch_id(), vec![fixed()])
        .await
        .unwrap();
    let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    let revision = runtime.revision();
    let depth = (runtime.undo_depth(), runtime.redo_depth());
    runtime.sketch_session.as_mut().unwrap().token = Uuid::new_v4();
    assert!(runtime
        .commit_sketch_constraints(prepared.drive().await.unwrap())
        .is_err());
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        before
    );
    assert_eq!(runtime.revision(), revision);
    assert_eq!((runtime.undo_depth(), runtime.redo_depth()), depth);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}
