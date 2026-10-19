use super::*;
use onecad_core::{
    document::variables::{Scalar, Unit, Variable},
    ids::{ConstraintId, VariableId},
};

fn id() -> ConstraintId {
    ConstraintId(Uuid::from_u128(901))
}
fn other(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}

pub(super) fn dimensions() -> Vec<Constraint> {
    vec![
        Constraint::Distance {
            id: id(),
            entity1: entity(),
            entity1_position: CurvePosition::Arbitrary,
            entity2: other(23),
            entity2_position: CurvePosition::Start,
            value: Scalar::new(10.0),
        },
        Constraint::HorizontalDistance {
            id: id(),
            point1: entity(),
            point1_position: CurvePosition::Arbitrary,
            point2: other(23),
            point2_position: CurvePosition::End,
            value: Scalar::new(10.0),
        },
        Constraint::VerticalDistance {
            id: id(),
            point1: entity(),
            point1_position: CurvePosition::Arbitrary,
            point2: other(23),
            point2_position: CurvePosition::Start,
            value: Scalar::new(10.0),
        },
        Constraint::Angle {
            id: id(),
            line1: other(22),
            line2: other(24),
            value: Scalar::new(0.5),
        },
        Constraint::Radius {
            id: id(),
            entity: other(23),
            value: Scalar::new(10.0),
        },
        Constraint::Diameter {
            id: id(),
            entity: other(23),
            value: Scalar::new(10.0),
        },
    ]
}

pub(super) fn dimension_runtime(constraint: Constraint) -> (DocumentRuntime, Arc<Solver>) {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    let entities = vec![
        SketchEntity::point(other(21), Vec2::new(5.0, 1.0).unwrap(), false, false),
        SketchEntity::line(other(22), entity(), other(21), false),
        SketchEntity::arc(other(23), entity(), 10.0, 0.0, 1.0, false).unwrap(),
        SketchEntity::line(other(24), other(21), entity(), false),
    ];
    let mut ops: Vec<_> = entities
        .into_iter()
        .map(|entity| SketchEditOp::AddEntity { entity })
        .collect();
    ops.push(SketchEditOp::AddConstraint { constraint });
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: sketch_id(),
            ops,
        })
        .unwrap();
    (runtime, solver)
}

fn references(constraint: &Constraint) -> serde_json::Value {
    let mut value = serde_json::to_value(constraint).unwrap();
    value.as_object_mut().unwrap().remove("value");
    value
}

fn cached_solve(runtime: &DocumentRuntime) -> serde_json::Value {
    let mut value = serde_json::to_value(runtime.get_sketch(sketch_id()).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("history");
    value
}

#[tokio::test]
async fn dimension_edits_preserve_all_ids_roles_order_and_noop_history() {
    for constraint in dimensions() {
        let (mut runtime, _) = dimension_runtime(constraint.clone());
        runtime.enter_sketch(sketch_id()).await.unwrap();
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let depth = runtime.undo_depth();
        let (result, outcome) = runtime
            .set_sketch_dimension_with_outcome(sketch_id(), id(), Scalar::new(12.0))
            .await
            .unwrap();
        assert!(result.accepted && outcome.is_some());
        let after = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        assert_eq!(after.entities(), before.entities());
        assert_eq!(
            references(after.get_constraint(id()).unwrap()),
            references(&constraint)
        );
        assert_eq!(
            after
                .constraints()
                .iter()
                .map(Constraint::id)
                .collect::<Vec<_>>(),
            before
                .constraints()
                .iter()
                .map(Constraint::id)
                .collect::<Vec<_>>()
        );
        assert_eq!(runtime.undo_depth(), depth + 1);
        let revision = runtime.revision();
        assert!(runtime
            .set_sketch_dimension_with_outcome(sketch_id(), id(), Scalar::new(12.0))
            .await
            .unwrap()
            .1
            .is_none());
        assert_eq!(runtime.revision(), revision);
        assert_eq!(runtime.undo_depth(), depth + 1);
    }
}

#[tokio::test]
async fn dimension_expressions_use_current_variables_unbind_and_convert_angles_once() {
    let (mut runtime, _) = dimension_runtime(dimensions().remove(0));
    let variable = VariableId::new();
    runtime
        .apply(EditCommand::AddVariable {
            variable: Variable {
                id: variable,
                name: "width".into(),
                value: Scalar::new(7.0),
                unit: Unit::Mm,
            },
        })
        .unwrap();
    for (value, expected) in [
        (
            Scalar::with_expr(999.0, "width * 2"),
            Scalar::with_expr(14.0, "width * 2"),
        ),
        (Scalar::new(9.0), Scalar::new(9.0)),
        (
            Scalar::with_expr(0.0, "width * 3"),
            Scalar::with_expr(21.0, "width * 3"),
        ),
    ] {
        runtime
            .set_sketch_dimension_with_outcome(sketch_id(), id(), value)
            .await
            .unwrap();
        assert_eq!(
            runtime
                .sketch_snapshot(sketch_id(), "test")
                .unwrap()
                .get_constraint(id())
                .unwrap()
                .value(),
            Some(&expected)
        );
    }
    let (mut runtime, _) = dimension_runtime(dimensions().remove(3));
    for (text, expected) in [("90deg", std::f64::consts::FRAC_PI_2), ("-0.5rad", -0.5)] {
        runtime
            .set_sketch_dimension_with_outcome(sketch_id(), id(), Scalar::with_expr(999.0, text))
            .await
            .unwrap();
        let snapshot = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        assert!(
            (snapshot
                .get_constraint(id())
                .unwrap()
                .value()
                .unwrap()
                .value
                - expected)
                .abs()
                < 1e-12
        );
    }
}

#[tokio::test]
async fn signed_dimensions_accept_zero_negative_and_invalid_scalars_never_reach_solver() {
    for constraint in dimensions() {
        let signed = matches!(
            constraint,
            Constraint::HorizontalDistance { .. }
                | Constraint::VerticalDistance { .. }
                | Constraint::Angle { .. }
        );
        let (mut runtime, solver) = dimension_runtime(constraint);
        for number in [-2.0, 0.0] {
            let result = runtime
                .set_sketch_dimension_with_outcome(sketch_id(), id(), Scalar::new(number))
                .await;
            assert_eq!(result.is_ok(), signed);
        }
        let calls = solver.calls.load(Ordering::SeqCst);
        for value in [
            Scalar {
                value: f64::NAN,
                expr: None,
            },
            Scalar {
                value: f64::INFINITY,
                expr: None,
            },
            Scalar::with_expr(1.0, "missing * 2"),
            Scalar::with_expr(1.0, "1 / 0"),
        ] {
            assert!(runtime
                .prepare_sketch_dimension(sketch_id(), id(), value)
                .await
                .is_err());
        }
        assert_eq!(solver.calls.load(Ordering::SeqCst), calls);
        assert!(runtime
            .prepare_sketch_dimension(sketch_id(), ConstraintId::new(), Scalar::new(1.0))
            .await
            .is_err());
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
}

#[tokio::test]
async fn dimension_failed_conflicting_stale_and_malformed_solves_preserve_accepted_state() {
    for mode in [
        Mode::Failure,
        Mode::Foreign,
        Mode::Nonfinite,
        Mode::NonfiniteTruth,
        Mode::Conflicting,
        Mode::OverConstrained,
        Mode::StaleRevision,
        Mode::StaleEpoch,
        Mode::WorkerRestart,
        Mode::ReplayedRevision,
    ] {
        let (mut runtime, solver) = dimension_runtime(dimensions().remove(0));
        runtime.enter_sketch(sketch_id()).await.unwrap();
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let cached = cached_solve(&runtime);
        let history = (runtime.undo_depth(), runtime.redo_depth());
        let dirty = runtime.is_dirty();
        *solver.mode.lock().unwrap() = mode;
        let result = runtime
            .set_sketch_dimension_with_outcome(sketch_id(), id(), Scalar::new(22.0))
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
        assert_eq!(cached_solve(&runtime), cached);
        assert_eq!((runtime.undo_depth(), runtime.redo_depth()), history);
        assert_eq!(runtime.is_dirty(), dirty);
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
}

#[tokio::test]
async fn dimension_candidates_cannot_outlive_variable_geometry_visit_or_runtime_changes() {
    for race in 0..4 {
        let (mut runtime, _) = dimension_runtime(dimensions().remove(0));
        let variable = VariableId::new();
        runtime
            .apply(EditCommand::AddVariable {
                variable: Variable {
                    id: variable,
                    name: "width".into(),
                    value: Scalar::new(5.0),
                    unit: Unit::Mm,
                },
            })
            .unwrap();
        runtime.enter_sketch(sketch_id()).await.unwrap();
        let prepared = runtime
            .prepare_sketch_dimension(sketch_id(), id(), Scalar::with_expr(0.0, "width * 2"))
            .await
            .unwrap();
        let driven = prepared.drive().await.unwrap();
        match race {
            0 => {
                runtime
                    .apply(EditCommand::SetVariable {
                        variable,
                        value: Scalar::new(12.0),
                    })
                    .unwrap();
            }
            1 => {
                runtime
                    .session
                    .apply(EditCommand::SketchEdit {
                        sketch: sketch_id(),
                        ops: moved_ops(),
                    })
                    .unwrap();
            }
            2 => runtime.sketch_session.as_mut().unwrap().token = Uuid::new_v4(),
            _ => runtime.instance = Uuid::new_v4(),
        }
        let before = runtime.sketch_snapshot(sketch_id(), "test").unwrap();
        let history = (runtime.undo_depth(), runtime.redo_depth());
        let revision = runtime.revision();
        assert!(runtime.commit_sketch_constraints(driven).is_err());
        assert_eq!(
            runtime.sketch_snapshot(sketch_id(), "test").unwrap(),
            before
        );
        assert_eq!((runtime.undo_depth(), runtime.redo_depth()), history);
        assert_eq!(runtime.revision(), revision);
        assert!(runtime.solver_lane.owner(sketch_id()).is_none());
    }
}

#[tokio::test]
async fn dimension_edits_reject_readonly_nondimensional_and_mismatched_expression_units() {
    let (mut runtime, solver) = dimension_runtime(dimensions().remove(0));
    runtime.read_only = true;
    assert!(runtime
        .prepare_sketch_dimension(sketch_id(), id(), Scalar::new(2.0))
        .await
        .is_err());
    runtime.read_only = false;
    assert!(runtime
        .prepare_sketch_dimension(sketch_id(), id(), Scalar::with_expr(0.0, "45deg"))
        .await
        .is_err());
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: sketch_id(),
            ops: vec![SketchEditOp::AddConstraint {
                constraint: fixed(),
            }],
        })
        .unwrap();
    assert!(runtime
        .prepare_sketch_dimension(sketch_id(), fixed().id(), Scalar::new(2.0))
        .await
        .is_err());
    assert_eq!(solver.calls.load(Ordering::SeqCst), 0);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}
