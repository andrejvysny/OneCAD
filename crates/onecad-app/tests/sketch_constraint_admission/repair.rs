use super::*;

fn second_distance(id: u128, length: f64) -> Constraint {
    Constraint::Distance {
        id: constraint(id),
        entity1: entity(5),
        entity1_position: CurvePosition::Arbitrary,
        entity2: entity(6),
        entity2_position: CurvePosition::Arbitrary,
        value: Scalar::new(length),
    }
}

async fn independent_conflicts(runtime: &mut DocumentRuntime, sid: SketchId) {
    let mut ops = vec![
        SketchEditOp::AddEntity {
            entity: SketchEntity::point(entity(5), Vec2::new(0.0, 10.0).unwrap(), false, false),
        },
        SketchEditOp::AddEntity {
            entity: SketchEntity::point(entity(6), Vec2::new(10.0, 10.0).unwrap(), false, false),
        },
        SketchEditOp::AddEntity {
            entity: SketchEntity::line(entity(7), entity(5), entity(6), false),
        },
    ];
    let constraints = vec![
        distance(4, 25.0),
        Constraint::Fixed {
            id: constraint(5),
            point: entity(5),
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new(0.0, 10.0).unwrap(),
        },
        Constraint::Horizontal {
            id: constraint(6),
            line: entity(7),
        },
        second_distance(7, 10.0),
        second_distance(8, 25.0),
    ];
    ops.extend(
        constraints
            .into_iter()
            .map(|constraint| SketchEditOp::AddConstraint { constraint }),
    );
    assert_eq!(
        runtime.sketch_upsert(sid, ops).await.unwrap().status,
        SketchSolveStatus::Conflicting
    );
}

#[tokio::test]
async fn repair_removes_one_independent_conflict_before_the_other() {
    let Some((worker, mut runtime, sid)) = setup(true).await else {
        return;
    };
    independent_conflicts(&mut runtime, sid).await;
    let before = runtime.sketch_snapshot(sid, "test").unwrap();
    for (id, expected) in [
        (4, SketchSolveStatus::Conflicting),
        (8, SketchSolveStatus::FullyConstrained),
    ] {
        let depth = runtime.undo_depth();
        let (admission, outcome) = runtime
            .repair_sketch_with_outcome(
                sid,
                vec![SketchEditOp::RemoveConstraint {
                    constraint: constraint(id),
                }],
            )
            .await
            .unwrap();
        assert!(admission.accepted && outcome.is_some());
        assert_eq!(admission.solve.status, expected);
        assert_eq!(runtime.get_sketch(sid).unwrap().status, expected);
        assert_eq!(runtime.undo_depth(), depth + 1);
        let after = runtime.sketch_snapshot(sid, "test").unwrap();
        assert!(after.get_constraint(constraint(id)).is_none());
        for (point, x, y) in [
            (1, 0.0, 0.0),
            (2, 10.0, 0.0),
            (5, 0.0, 10.0),
            (6, 10.0, 10.0),
        ] {
            assert!(
                matches!(after.get_entity(entity(point)), Some(SketchEntity::Point {at,..}) if (at.x-x).abs()<1e-8 && (at.y-y).abs()<1e-8)
            );
        }
    }
    runtime.undo().unwrap();
    runtime.undo().unwrap();
    assert_eq!(runtime.sketch_snapshot(sid, "test").unwrap(), before);
    worker.shutdown().await;
}

#[tokio::test]
async fn repair_cannot_delete_a_locked_line_through_its_free_endpoint() {
    let Some((worker, mut runtime, sid)) = setup(true).await else {
        return;
    };
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: sid,
            ops: vec![SketchEditOp::SetEntityReferenceLocked {
                entity: entity(3),
                locked: true,
            }],
        })
        .unwrap();
    let before = runtime.sketch_snapshot(sid, "test").unwrap();
    let state = (
        runtime.revision(),
        runtime.undo_depth(),
        runtime.redo_depth(),
    );
    for op in [
        SketchEditOp::RemoveEntity { entity: entity(1) },
        SketchEditOp::SetEntityConstruction {
            entity: entity(3),
            construction: true,
        },
    ] {
        assert!(runtime.prepare_sketch_repair(sid, vec![op]).await.is_err());
        assert_eq!(runtime.sketch_snapshot(sid, "test").unwrap(), before);
        assert_eq!(
            (
                runtime.revision(),
                runtime.undo_depth(),
                runtime.redo_depth()
            ),
            state
        );
    }
    worker.shutdown().await;
}

#[tokio::test]
async fn construction_circle_is_solved_but_excluded_from_material_profiles() {
    let Some((worker, mut runtime, sid)) = setup(true).await else {
        return;
    };
    runtime
        .sketch_upsert(
            sid,
            vec![
                SketchEditOp::AddEntity {
                    entity: SketchEntity::point(
                        entity(9),
                        Vec2::new(30.0, 0.0).unwrap(),
                        false,
                        false,
                    ),
                },
                SketchEditOp::AddEntity {
                    entity: SketchEntity::circle(entity(8), entity(9), 5.0, false).unwrap(),
                },
            ],
        )
        .await
        .unwrap();
    let regions = runtime
        .prepare_sketch_regions(sid)
        .unwrap()
        .drive()
        .await
        .unwrap();
    assert_eq!(regions.regions.len(), 1);
    let (admission, _) = runtime
        .repair_sketch_with_outcome(
            sid,
            vec![SketchEditOp::SetEntityConstruction {
                entity: entity(8),
                construction: true,
            }],
        )
        .await
        .unwrap();
    assert!(admission.accepted);
    assert_eq!(admission.solve.status, SketchSolveStatus::UnderConstrained);
    let regions = runtime
        .prepare_sketch_regions(sid)
        .unwrap()
        .drive()
        .await
        .unwrap();
    assert!(regions.regions.is_empty());
    assert!(runtime
        .sketch_snapshot(sid, "test")
        .unwrap()
        .get_entity(entity(8))
        .unwrap()
        .is_construction());
    worker.shutdown().await;
}
