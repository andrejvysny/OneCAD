use super::*;
use onecad_core::{
    document::variables::{Scalar, Unit, Variable},
    ids::{ConstraintId, VariableId},
};
use std::{
    future::{poll_fn, Future},
    task::Poll,
};

fn fresh(number: u128) -> EntityId {
    EntityId(Uuid::from_u128(number))
}
fn dimension() -> ConstraintId {
    ConstraintId(Uuid::from_u128(901))
}
fn primitive(value: Scalar) -> Vec<SketchEditOp> {
    vec![
        SketchEditOp::AddEntity {
            entity: SketchEntity::point(fresh(21), Vec2::new(4.0, 1.0).unwrap(), false, false),
        },
        SketchEditOp::AddEntity {
            entity: SketchEntity::line(fresh(22), entity(), fresh(21), false),
        },
        SketchEditOp::AddConstraint {
            constraint: Constraint::Distance {
                id: dimension(),
                entity1: entity(),
                entity1_position: CurvePosition::Arbitrary,
                entity2: fresh(21),
                entity2_position: CurvePosition::Arbitrary,
                value,
            },
        },
    ]
}
fn variable(runtime: &mut DocumentRuntime) -> VariableId {
    let id = VariableId::new();
    runtime
        .apply(EditCommand::AddVariable {
            variable: Variable {
                id,
                name: "width".into(),
                value: Scalar::new(7.0),
                unit: Unit::Mm,
            },
        })
        .unwrap();
    id
}

#[tokio::test]
async fn primitive_resolves_current_expression_and_commits_one_atomic_history_step() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    variable(&mut runtime);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    let depth = runtime.undo_depth();
    let revision = runtime.revision();
    let (result, outcome) = runtime
        .author_sketch_primitive_with_outcome(
            sketch_id(),
            primitive(Scalar::with_expr(999.0, "width * 2")),
        )
        .await
        .unwrap();
    assert!(result.accepted && outcome.is_some());
    assert_eq!(solver.calls.load(Ordering::SeqCst), 2);
    let after = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    assert_eq!(
        after.get_constraint(dimension()).unwrap().value(),
        Some(&Scalar::with_expr(14.0, "width * 2"))
    );
    assert_eq!(runtime.undo_depth(), depth + 1);
    assert_eq!(runtime.revision().0, revision.0 + 1);
    runtime.undo().unwrap();
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        before
    );
    runtime.redo().unwrap().unwrap();
    assert_eq!(runtime.sketch_snapshot(sketch_id(), "test").unwrap(), after);
}

#[tokio::test]
async fn primitive_bounds_freshness_locks_whitelist_and_missing_refs_refuse_before_worker() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: sketch_id(),
            ops: vec![SketchEditOp::AddConstraint {
                constraint: fixed(),
            }],
        })
        .unwrap();
    let base = primitive(Scalar::new(3.0));
    let mut duplicate_entity = base.clone();
    duplicate_entity.push(base[0].clone());
    let mut duplicate_constraint = base.clone();
    duplicate_constraint.push(base[2].clone());
    let mut existing_constraint = base.clone();
    existing_constraint.push(SketchEditOp::AddConstraint {
        constraint: fixed(),
    });
    let rejected = vec![
        vec![],
        vec![base[2].clone()],
        vec![base[0].clone(); 129],
        duplicate_entity,
        duplicate_constraint,
        existing_constraint,
        vec![SketchEditOp::AddEntity {
            entity: SketchEntity::point(entity(), Vec2::new(2.0, 1.0).unwrap(), false, false),
        }],
        vec![SketchEditOp::AddEntity {
            entity: SketchEntity::point(fresh(21), Vec2::new(2.0, 1.0).unwrap(), false, true),
        }],
        vec![SketchEditOp::AddEntity {
            entity: SketchEntity::line(fresh(22), entity(), entity(), false)
                .with_reference_locked(true),
        }],
        vec![
            base[0].clone(),
            SketchEditOp::AddEntity {
                entity: SketchEntity::line(fresh(22), entity(), fresh(99), false),
            },
        ],
    ];
    for ops in rejected {
        assert!(runtime
            .prepare_sketch_primitive(sketch_id(), ops)
            .await
            .is_err());
    }
    for op in [
        moved_ops().remove(0),
        SketchEditOp::RemoveEntity { entity: entity() },
        SketchEditOp::RemoveConstraint {
            constraint: fixed().id(),
        },
        SketchEditOp::SetDimension {
            constraint: fixed().id(),
            value: Scalar::new(2.0),
        },
        SketchEditOp::SetEntityReferenceLocked {
            entity: entity(),
            locked: false,
        },
        SketchEditOp::SetEntityConstruction {
            entity: entity(),
            construction: true,
        },
    ] {
        assert!(runtime
            .prepare_sketch_primitive(sketch_id(), vec![base[0].clone(), op])
            .await
            .is_err());
    }
    assert_eq!(solver.calls.load(Ordering::SeqCst), 0);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}

#[tokio::test]
async fn primitive_invalid_expressions_or_scalars_preserve_clean_document_without_solving() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    let area = tempfile::tempdir().unwrap();
    assert!(runtime.mark_saved(&area.path().join("clean.onecad"), runtime.revision()));
    let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    let projection = runtime.projection();
    for value in [
        Scalar::with_expr(10.0, "missing * 2"),
        Scalar::with_expr(10.0, "45deg"),
        Scalar::with_expr(10.0, "1 / 0"),
        Scalar {
            value: f64::NAN,
            expr: None,
        },
        Scalar::new(-1.0),
    ] {
        assert!(runtime
            .prepare_sketch_primitive(sketch_id(), primitive(value))
            .await
            .is_err());
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
        assert_eq!(runtime.projection(), projection);
        assert!(!runtime.is_dirty());
    }
    assert_eq!(solver.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn primitive_conflicts_errors_and_stale_worker_results_never_publish_candidate_state() {
    for mode in [
        Mode::Conflicting,
        Mode::OverConstrained,
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
        let (mut runtime, solver) = runtime(Mode::Unchanged);
        runtime.enter_sketch(sketch_id()).await.unwrap();
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let cached = runtime
            .sketch_solve
            .get(&sketch_id())
            .unwrap()
            .token
            .clone();
        let history = (runtime.undo_depth(), runtime.redo_depth());
        let area = tempfile::tempdir().unwrap();
        assert!(runtime.mark_saved(&area.path().join("clean.onecad"), runtime.revision()));
        *solver.mode.lock().unwrap() = mode;
        let result = runtime
            .author_sketch_primitive_with_outcome(sketch_id(), primitive(Scalar::new(3.0)))
            .await;
        match mode {
            Mode::Conflicting | Mode::OverConstrained => {
                let (result, outcome) = result.unwrap();
                assert!(!result.accepted && outcome.is_none());
            }
            _ => assert!(result.is_err()),
        }
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
        assert_eq!((runtime.undo_depth(), runtime.redo_depth()), history);
        assert!(!runtime.is_dirty());
        assert_eq!(
            runtime.sketch_solve.get(&sketch_id()).unwrap().token,
            cached
        );
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
}

#[tokio::test]
async fn primitive_detached_completion_rejects_variable_visit_runtime_and_geometry_changes() {
    for race in 0..4 {
        let (mut runtime, _) = runtime(Mode::Unchanged);
        let variable = variable(&mut runtime);
        runtime.enter_sketch(sketch_id()).await.unwrap();
        let prepared = runtime
            .prepare_sketch_primitive(sketch_id(), primitive(Scalar::with_expr(0.0, "width * 2")))
            .await
            .unwrap();
        let driven = prepared.drive().await.unwrap();
        match race {
            0 => {
                runtime
                    .apply(EditCommand::SetVariable {
                        variable,
                        value: Scalar::new(20.0),
                    })
                    .unwrap();
            }
            1 => runtime.sketch_session.as_mut().unwrap().token = Uuid::new_v4(),
            2 => runtime.instance = Uuid::new_v4(),
            _ => {
                runtime
                    .session
                    .apply(EditCommand::SketchEdit {
                        sketch: sketch_id(),
                        ops: moved_ops(),
                    })
                    .unwrap();
            }
        }
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let history = (runtime.undo_depth(), runtime.redo_depth());
        assert!(runtime.commit_sketch_constraints(driven).is_err());
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
        assert_eq!((runtime.undo_depth(), runtime.redo_depth()), history);
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
}

#[tokio::test]
async fn primitive_constraints_may_reference_locked_sources_but_cannot_move_them() {
    for mode in [Mode::Unchanged, Mode::Correct] {
        let (mut runtime, _) = runtime(mode);
        runtime
            .apply(EditCommand::SketchEdit {
                sketch: sketch_id(),
                ops: vec![SketchEditOp::SetEntityReferenceLocked {
                    entity: entity(),
                    locked: true,
                }],
            })
            .unwrap();
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let result = runtime
            .author_sketch_primitive_with_outcome(sketch_id(), primitive(Scalar::new(3.0)))
            .await;
        if mode == Mode::Unchanged {
            assert!(result.unwrap().0.accepted);
        } else {
            assert!(result.is_err());
            assert_eq!(
                runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
                before
            );
        }
        assert_eq!(
            runtime
                .sketch_snapshot(sketch_id(), "test")
                .unwrap()
                .get_entity(entity()),
            before.get_entity(entity())
        );
    }
}

struct Quiet;
impl crate::host::EventSink for Quiet {
    fn emit(&self, _: crate::host::AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait]
impl crate::host::DialogPort for Quiet {
    async fn pick(&self, _: crate::host::FileDialog) -> Vec<String> {
        Vec::new()
    }
}

#[tokio::test]
async fn public_primitive_api_releases_application_lock_during_worker_await_and_cancel() {
    let (runtime, solver) = runtime(Mode::Pending);
    let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    let depth = runtime.undo_depth();
    let state = crate::state::AppState::default();
    *state.runtime.lock().await = Some(runtime);
    let host = crate::host::HostContext {
        paths: Default::default(),
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    };
    {
        let operation = crate::api::author_sketch_primitive_with_completion(
            &state,
            host,
            sketch_id().to_string(),
            primitive(Scalar::new(3.0)),
        );
        tokio::pin!(operation);
        assert!(matches!(
            poll_fn(|context| Poll::Ready(operation.as_mut().poll(context))).await,
            Poll::Pending
        ));
        assert_eq!(solver.calls.load(Ordering::SeqCst), 1);
        let guard = state
            .runtime
            .try_lock()
            .expect("native primitive worker I/O must not hold the application lock");
        let runtime = guard.as_ref().unwrap();
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
        assert!(runtime.solver_lane.owner(sketch_id()).is_some());
    }
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    assert_eq!(
        runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
        before
    );
    assert_eq!(runtime.undo_depth(), depth);
}

#[tokio::test]
async fn primitive_nonfinite_geometry_and_readonly_authoring_refuse_before_worker() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
    let entities = [
        SketchEntity::point(fresh(21), Vec2::new_unchecked(f64::NAN, 0.0), false, false),
        SketchEntity::Circle {
            id: fresh(21),
            center: entity(),
            radius: f64::INFINITY,
            construction: false,
            reference_locked: false,
        },
        SketchEntity::Arc {
            id: fresh(21),
            center: entity(),
            radius: 2.0,
            start_angle: f64::NAN,
            end_angle: 1.0,
            construction: false,
            reference_locked: false,
        },
        SketchEntity::Ellipse {
            id: fresh(21),
            center: entity(),
            major_r: 2.0,
            minor_r: 1.0,
            rotation: f64::NAN,
            construction: false,
            reference_locked: false,
        },
    ];
    for entity in entities {
        assert!(runtime
            .prepare_sketch_primitive(sketch_id(), vec![SketchEditOp::AddEntity { entity }])
            .await
            .is_err());
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
    }
    let mut invalid_fixed = primitive(Scalar::new(3.0));
    invalid_fixed.push(SketchEditOp::AddConstraint {
        constraint: Constraint::Fixed {
            id: ConstraintId::new(),
            point: fresh(21),
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new_unchecked(0.0, f64::NAN),
        },
    });
    assert!(runtime
        .prepare_sketch_primitive(sketch_id(), invalid_fixed)
        .await
        .is_err());
    runtime.read_only = true;
    assert!(runtime
        .prepare_sketch_primitive(sketch_id(), primitive(Scalar::new(3.0)))
        .await
        .is_err());
    assert_eq!(solver.calls.load(Ordering::SeqCst), 0);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}
