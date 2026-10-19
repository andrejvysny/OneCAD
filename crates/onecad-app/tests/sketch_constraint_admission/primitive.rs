use super::*;
use onecad_core::{
    document::variables::{Unit, Variable},
    ids::VariableId,
};

fn line(value: Scalar) -> Vec<SketchEditOp> {
    vec![
        SketchEditOp::AddEntity {
            entity: SketchEntity::point(entity(4), Vec2::new(30.0, 10.0).unwrap(), false, false),
        },
        SketchEditOp::AddEntity {
            entity: SketchEntity::line(entity(5), entity(2), entity(4), false),
        },
        SketchEditOp::AddConstraint {
            constraint: Constraint::Horizontal {
                id: constraint(4),
                line: entity(5),
            },
        },
        SketchEditOp::AddConstraint {
            constraint: Constraint::Distance {
                id: constraint(5),
                entity1: entity(2),
                entity1_position: CurvePosition::Arbitrary,
                entity2: entity(4),
                entity2_position: CurvePosition::Arbitrary,
                value,
            },
        },
    ]
}

#[tokio::test]
async fn native_primitive_numeric_expression_resolves_current_variables_before_atomic_commit() {
    let Some((worker, mut runtime, sid)) = setup(true).await else {
        return;
    };
    let variable = VariableId::new();
    runtime
        .apply(EditCommand::AddVariable {
            variable: Variable {
                id: variable,
                name: "span".into(),
                value: Scalar::new(15.0),
                unit: Unit::Mm,
            },
        })
        .unwrap();
    let ops = line(Scalar::with_expr(999.0, "span * 2"));
    runtime
        .apply(EditCommand::SetVariable {
            variable,
            value: Scalar::new(20.0),
        })
        .unwrap();
    let before = runtime.sketch_snapshot(sid, "test").unwrap();
    let depth = runtime.undo_depth();
    let revision = runtime.revision();
    let (admission, outcome) = runtime
        .author_sketch_primitive_with_outcome(sid, ops)
        .await
        .unwrap();
    assert!(admission.accepted && outcome.is_some());
    assert_eq!(admission.solve.status, SketchSolveStatus::FullyConstrained);
    assert_eq!(runtime.undo_depth(), depth + 1);
    assert_eq!(runtime.revision().0, revision.0 + 1);
    let after = runtime.sketch_snapshot(sid, "test").unwrap();
    assert_eq!(
        after.get_constraint(constraint(5)).unwrap().value(),
        Some(&Scalar::with_expr(40.0, "span * 2"))
    );
    assert!(
        matches!(after.get_entity(entity(4)),Some(SketchEntity::Point {at,..}) if (at.x-50.0).abs()<1e-8&&at.y.abs()<1e-8)
    );
    for original in before.entities() {
        assert_eq!(after.get_entity(original.id()), Some(original));
    }
    runtime.undo().unwrap();
    assert_eq!(runtime.sketch_snapshot(sid, "test").unwrap(), before);
    runtime.redo().unwrap().unwrap();
    assert_eq!(runtime.sketch_snapshot(sid, "test").unwrap(), after);
    worker.shutdown().await;
}

#[tokio::test]
async fn undefined_numeric_expression_and_infeasible_winning_inference_never_author() {
    let Some((worker, mut runtime, sid)) = setup(true).await else {
        return;
    };
    let area = tempfile::tempdir().unwrap();
    assert!(runtime.mark_saved(&area.path().join("clean.onecad"), runtime.revision()));
    let before = runtime.sketch_snapshot(sid, "test").unwrap();
    let projection = runtime.projection();
    let solve = serde_json::to_value(runtime.get_sketch(sid).unwrap()).unwrap();
    let history = (runtime.undo_depth(), runtime.redo_depth());
    assert!(runtime
        .prepare_sketch_primitive(sid, line(Scalar::with_expr(40.0, "missing * 2")))
        .await
        .is_err());
    let ops = vec![
        SketchEditOp::AddEntity {
            entity: SketchEntity::point(entity(4), Vec2::new(15.0, 0.0).unwrap(), false, false),
        },
        SketchEditOp::AddConstraint {
            constraint: Constraint::Fixed {
                id: constraint(4),
                point: entity(4),
                point_position: CurvePosition::Arbitrary,
                at: Vec2::new(15.0, 0.0).unwrap(),
            },
        },
        SketchEditOp::AddConstraint {
            constraint: Constraint::Coincident {
                id: constraint(5),
                point1: entity(4),
                point1_position: CurvePosition::Arbitrary,
                point2: entity(1),
                point2_position: CurvePosition::Arbitrary,
            },
        },
    ];
    let (admission, outcome) = runtime
        .author_sketch_primitive_with_outcome(sid, ops)
        .await
        .unwrap();
    assert!(!admission.accepted && outcome.is_none());
    assert_eq!(admission.solve.status, SketchSolveStatus::Conflicting);
    assert_eq!(runtime.sketch_snapshot(sid, "test").unwrap(), before);
    assert_eq!(runtime.projection(), projection);
    assert_eq!(
        serde_json::to_value(runtime.get_sketch(sid).unwrap()).unwrap(),
        solve
    );
    assert_eq!((runtime.undo_depth(), runtime.redo_depth()), history);
    assert!(!runtime.is_dirty());
    worker.shutdown().await;
}

#[tokio::test]
async fn detached_real_primitive_cannot_commit_after_its_sketch_visit_is_closed() {
    let Some((worker, mut runtime, sid)) = setup(true).await else {
        return;
    };
    let before = runtime.sketch_snapshot(sid, "test").unwrap();
    let driven = runtime
        .prepare_sketch_primitive(sid, line(Scalar::new(40.0)))
        .await
        .unwrap()
        .drive()
        .await
        .unwrap();
    runtime.cancel_sketch(sid, false).await.unwrap();
    let history = (runtime.undo_depth(), runtime.redo_depth());
    let revision = runtime.revision();
    assert!(runtime.commit_sketch_constraints(driven).is_err());
    assert_eq!(runtime.sketch_snapshot(sid, "test").unwrap(), before);
    assert_eq!((runtime.undo_depth(), runtime.redo_depth()), history);
    assert_eq!(runtime.revision(), revision);
    worker.shutdown().await;
}
