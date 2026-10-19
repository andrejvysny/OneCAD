//! Native constraint candidates must never author a refused solve.
use std::{path::PathBuf, sync::Arc, time::Duration};

use onecad_app::{
    document_runtime::DocumentRuntime,
    dto::SketchSolveStatus,
    worker::{resolve_worker_path, SupervisorConfig, WorkerManager},
};
use onecad_core::{
    document::variables::Scalar,
    edit::{EditCommand, SketchEditOp},
    ids::{ConstraintId, EntityId, SketchId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane},
};
use uuid::Uuid;

fn entity(id: u128) -> EntityId {
    EntityId(Uuid::from_u128(id))
}
fn constraint(id: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(id))
}
fn distance(id: u128, length: f64) -> Constraint {
    Constraint::Distance {
        id: constraint(id),
        entity1: entity(1),
        entity2: entity(2),
        entity1_position: CurvePosition::Arbitrary,
        entity2_position: CurvePosition::Arbitrary,
        value: Scalar::new(length),
    }
}
fn worker_path() -> Option<PathBuf> {
    let path = std::env::var("ONECAD_WORKER_PATH")
        .ok()
        .map(PathBuf::from)
        .or_else(resolve_worker_path);
    assert!(path.as_ref().is_none_or(|p| p.is_file()));
    assert!(path.is_some() || std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"));
    path
}
async fn setup(with_length: bool) -> Option<(WorkerManager, DocumentRuntime, SketchId)> {
    let worker = WorkerManager::spawn(SupervisorConfig::production(worker_path()?));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    );
    let sid = SketchId::new();
    let mut sketch = Sketch::on_world_plane(sid, "Constraint admission", WorldPlane::XY);
    for (id, x) in [(1, 0.0), (2, 10.0)] {
        sketch
            .add_entity(SketchEntity::point(
                entity(id),
                Vec2::new(x, 0.0).unwrap(),
                false,
                false,
            ))
            .unwrap();
    }
    sketch
        .add_entity(SketchEntity::line(entity(3), entity(1), entity(2), false))
        .unwrap();
    sketch
        .add_constraint(Constraint::Fixed {
            id: constraint(1),
            point: entity(1),
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new(0.0, 0.0).unwrap(),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Horizontal {
            id: constraint(2),
            line: entity(3),
        })
        .unwrap();
    if with_length {
        sketch.add_constraint(distance(3, 10.0)).unwrap();
    }
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    assert_eq!(
        runtime.enter_sketch(sid).await.unwrap().dof,
        u32::from(!with_length)
    );
    Some((worker, runtime, sid))
}

#[tokio::test]
async fn conflicting_candidate_preserves_authoritative_document_and_history() {
    let Some((worker, mut runtime, sid)) = setup(true).await else {
        return;
    };
    let before = runtime.sketch_snapshot(sid, "test").unwrap();
    let revision = runtime.revision();
    let undo = runtime.undo_depth();
    let (admission, outcome) = runtime
        .add_sketch_constraints_with_outcome(sid, vec![distance(4, 25.0)])
        .await
        .unwrap();
    assert!(!admission.accepted && outcome.is_none());
    let result = admission.solve;
    assert_eq!(result.status, SketchSolveStatus::Conflicting);
    eprintln!(
        "candidate {:?}; document constraints {} -> {}; undo {} -> {}",
        result.status,
        before.constraints().len(),
        runtime
            .sketch_snapshot(sid, "test")
            .unwrap()
            .constraints()
            .len(),
        undo,
        runtime.undo_depth()
    );
    assert_eq!(runtime.sketch_snapshot(sid, "test").unwrap(), before);
    assert_eq!(runtime.revision(), revision);
    assert_eq!(runtime.undo_depth(), undo);
    worker.shutdown().await;
}

#[tokio::test]
async fn redundant_candidate_preserves_redo_clean_state_and_authoritative_solve() {
    let Some((worker, mut runtime, sid)) = setup(true).await else {
        return;
    };
    runtime
        .sketch_upsert(
            sid,
            vec![SketchEditOp::AddEntity {
                entity: SketchEntity::point(entity(99), Vec2::new(8.0, 8.0).unwrap(), false, false),
            }],
        )
        .await
        .unwrap();
    runtime.undo().unwrap();
    runtime.enter_sketch(sid).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert!(runtime.mark_saved(&directory.path().join("qa.onecad"), runtime.revision()));
    let projection = runtime.projection();
    let solve = serde_json::to_value(runtime.get_sketch(sid).unwrap()).unwrap();
    let history = (runtime.undo_depth(), runtime.redo_depth());
    assert!(history.1 > 0);
    let extra = Constraint::Horizontal {
        id: constraint(4),
        line: entity(3),
    };
    let (admission, outcome) = runtime
        .add_sketch_constraints_with_outcome(sid, vec![extra])
        .await
        .unwrap();
    assert!(!admission.accepted && outcome.is_none());
    assert_eq!(admission.solve.status, SketchSolveStatus::OverConstrained);
    assert_eq!(runtime.projection(), projection);
    assert!(!runtime.is_dirty());
    assert_eq!((runtime.undo_depth(), runtime.redo_depth()), history);
    assert_eq!(
        serde_json::to_value(runtime.get_sketch(sid).unwrap()).unwrap(),
        solve
    );
    worker.shutdown().await;
}

#[tokio::test]
async fn accepted_expression_uses_current_variables_and_commits_one_undo_entry() {
    use onecad_core::{
        document::variables::{Unit, Variable},
        ids::VariableId,
    };
    let Some((worker, mut runtime, sid)) = setup(false).await else {
        return;
    };
    runtime
        .apply(EditCommand::AddVariable {
            variable: Variable {
                id: VariableId::new(),
                name: "width".into(),
                value: Scalar::new(15.0),
                unit: Unit::Mm,
            },
        })
        .unwrap();
    let before = runtime.sketch_snapshot(sid, "test").unwrap();
    let revision = runtime.revision();
    let depth = runtime.undo_depth();
    let mut dimension = distance(4, 999.0);
    if let Constraint::Distance { value, .. } = &mut dimension {
        *value = Scalar::with_expr(999.0, "width * 2");
    }
    let (admission, outcome) = runtime
        .add_sketch_constraints_with_outcome(sid, vec![dimension])
        .await
        .unwrap();
    assert!(admission.accepted && outcome.is_some());
    assert_eq!(admission.solve.dof, 0);
    assert_eq!(runtime.revision().0, revision.0 + 1);
    assert_eq!(runtime.undo_depth(), depth + 1);
    let after = runtime.sketch_snapshot(sid, "test").unwrap();
    let value = after
        .get_constraint(constraint(4))
        .unwrap()
        .value()
        .unwrap();
    assert_eq!(value.value, 30.0);
    assert_eq!(value.expr.as_deref(), Some("width * 2"));
    assert!(
        matches!(after.get_entity(entity(2)), Some(SketchEntity::Point {at,..}) if (at.x-30.0).abs()<1e-9)
    );
    runtime.undo().unwrap();
    assert_eq!(runtime.sketch_snapshot(sid, "test").unwrap(), before);
    runtime.redo().unwrap().unwrap();
    assert_eq!(runtime.sketch_snapshot(sid, "test").unwrap(), after);
    worker.shutdown().await;
}

#[tokio::test]
async fn generic_upsert_still_retains_explicit_conflicting_edits() {
    let Some((worker, mut runtime, sid)) = setup(true).await else {
        return;
    };
    let depth = runtime.undo_depth();
    let solve = runtime
        .sketch_upsert(
            sid,
            vec![SketchEditOp::AddConstraint {
                constraint: distance(4, 25.0),
            }],
        )
        .await
        .unwrap();
    assert_eq!(solve.status, SketchSolveStatus::Conflicting);
    assert_eq!(
        runtime
            .sketch_snapshot(sid, "test")
            .unwrap()
            .constraints()
            .len(),
        4
    );
    assert_eq!(runtime.undo_depth(), depth + 1);
    worker.shutdown().await;
}
