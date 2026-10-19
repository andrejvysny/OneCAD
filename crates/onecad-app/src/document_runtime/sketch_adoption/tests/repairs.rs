use super::dimensions::{dimension_runtime, dimensions};
use super::*;
use onecad_core::document::variables::Scalar;

fn arc() -> EntityId {
    EntityId(Uuid::from_u128(23))
}

fn construction(construction: bool) -> SketchEditOp {
    SketchEditOp::SetEntityConstruction {
        entity: arc(),
        construction,
    }
}

#[tokio::test]
async fn explicit_repairs_admit_validated_remaining_conflicts_but_dimensions_stay_strict() {
    for mode in [Mode::Conflicting, Mode::OverConstrained] {
        let (mut runtime, solver) = dimension_runtime(dimensions().remove(0));
        runtime
            .apply(EditCommand::SketchEdit {
                sketch: sketch_id(),
                ops: vec![SketchEditOp::AddConstraint {
                    constraint: fixed(),
                }],
            })
            .unwrap();
        runtime.enter_sketch(sketch_id()).await.unwrap();
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let depth = runtime.undo_depth();
        *solver.mode.lock().unwrap() = mode;
        let (result, outcome) = runtime
            .repair_sketch_with_outcome(
                sketch_id(),
                vec![SketchEditOp::RemoveConstraint {
                    constraint: fixed().id(),
                }],
            )
            .await
            .unwrap();
        assert!(result.accepted && outcome.is_some());
        assert!(matches!(
            result.solve.status,
            SketchSolveStatus::Conflicting | SketchSolveStatus::OverConstrained
        ));
        assert_eq!(runtime.undo_depth(), depth + 1);
        let after = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        assert!(after.get_constraint(fixed().id()).is_none());
        assert_eq!(after.entities(), before.entities());
        assert_eq!(
            runtime.get_sketch(sketch_id()).unwrap().status,
            result.solve.status
        );
        let (strict, outcome) = runtime
            .set_sketch_dimension_with_outcome(
                sketch_id(),
                after.constraints()[0].id(),
                Scalar::new(20.0),
            )
            .await
            .unwrap();
        assert!(!strict.accepted && outcome.is_none());
        assert_eq!(runtime.sketch_snapshot(sketch_id(), "test").unwrap(), after);
    }
}

#[tokio::test]
async fn entity_deletion_cascades_references_and_construction_remains_solver_owned() {
    let (mut runtime, solver) = dimension_runtime(dimensions().remove(0));
    let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    let (result, outcome) = runtime
        .repair_sketch_with_outcome(sketch_id(), vec![construction(true)])
        .await
        .unwrap();
    assert!(result.accepted && outcome.is_some());
    assert_eq!(solver.calls.load(Ordering::SeqCst), 1);
    let after = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    assert!(after.get_entity(arc()).unwrap().is_construction());
    assert_eq!(after.constraints(), before.constraints());
    let (result, outcome) = runtime
        .repair_sketch_with_outcome(
            sketch_id(),
            vec![SketchEditOp::RemoveEntity { entity: entity() }],
        )
        .await
        .unwrap();
    assert!(result.accepted && outcome.is_some());
    let after = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    assert!(after.get_entity(entity()).is_none());
    assert!(after.get_entity(arc()).is_none());
    assert!(after.constraints().is_empty());
    assert_eq!(after.entities().len(), 1);
}

#[tokio::test]
async fn repair_preserves_locked_geometry_and_its_unlocked_dependencies() {
    let (mut runtime, solver) = dimension_runtime(dimensions().remove(0));
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: sketch_id(),
            ops: vec![SketchEditOp::SetEntityReferenceLocked {
                entity: arc(),
                locked: true,
            }],
        })
        .unwrap();
    let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    for op in [
        construction(true),
        SketchEditOp::RemoveEntity { entity: arc() },
        SketchEditOp::RemoveEntity { entity: entity() },
    ] {
        assert!(runtime
            .prepare_sketch_repair(sketch_id(), vec![op])
            .await
            .is_err());
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
    }
    assert_eq!(solver.calls.load(Ordering::SeqCst), 0);
    let constraint = before.constraints()[0].id();
    assert!(
        runtime
            .repair_sketch_with_outcome(
                sketch_id(),
                vec![SketchEditOp::RemoveConstraint { constraint }]
            )
            .await
            .unwrap()
            .0
            .accepted
    );
    assert_eq!(
        runtime
            .sketch_snapshot(sketch_id(), "test")
            .unwrap()
            .entities(),
        before.entities()
    );
}

#[tokio::test]
async fn repair_whitelist_missing_targets_bounds_and_noop_history_are_explicit() {
    let (mut runtime, solver) = dimension_runtime(dimensions().remove(0));
    let unsupported = vec![
        SketchEditOp::AddConstraint {
            constraint: fixed(),
        },
        SketchEditOp::AddEntity {
            entity: SketchEntity::point(
                EntityId::new(),
                Vec2::new(0.0, 0.0).unwrap(),
                false,
                false,
            ),
        },
        SketchEditOp::SetDimension {
            constraint: fixed().id(),
            value: Scalar::new(2.0),
        },
        moved_ops().remove(0),
        SketchEditOp::SetEntityReferenceLocked {
            entity: arc(),
            locked: false,
        },
        SketchEditOp::RemoveConstraint {
            constraint: fixed().id(),
        },
        SketchEditOp::RemoveEntity {
            entity: EntityId::new(),
        },
    ];
    for op in unsupported {
        assert!(runtime
            .prepare_sketch_repair(sketch_id(), vec![op])
            .await
            .is_err());
    }
    for ops in [vec![], vec![construction(false); 129]] {
        assert!(runtime
            .prepare_sketch_repair(sketch_id(), ops)
            .await
            .is_err());
    }
    assert_eq!(solver.calls.load(Ordering::SeqCst), 0);
    let state = (
        runtime.revision(),
        runtime.undo_depth(),
        runtime.redo_depth(),
    );
    let (result, outcome) = runtime
        .repair_sketch_with_outcome(sketch_id(), vec![construction(false)])
        .await
        .unwrap();
    assert!(result.accepted && outcome.is_none());
    assert_eq!(
        (
            runtime.revision(),
            runtime.undo_depth(),
            runtime.redo_depth()
        ),
        state
    );
}

#[tokio::test]
async fn repair_errors_unassessed_truth_and_stale_results_never_commit() {
    for mode in [
        Mode::Failure,
        Mode::Foreign,
        Mode::Nonfinite,
        Mode::NonfiniteTruth,
        Mode::Unassessed,
        Mode::StaleRevision,
        Mode::StaleEpoch,
        Mode::WorkerRestart,
        Mode::ReplayedRevision,
    ] {
        let (mut runtime, solver) = dimension_runtime(dimensions().remove(0));
        runtime.enter_sketch(sketch_id()).await.unwrap();
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let history = (runtime.undo_depth(), runtime.redo_depth());
        *solver.mode.lock().unwrap() = mode;
        assert!(runtime
            .repair_sketch_with_outcome(sketch_id(), vec![construction(true)])
            .await
            .is_err());
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
        assert_eq!((runtime.undo_depth(), runtime.redo_depth()), history);
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
}
