//! Production-worker service qualification; prepared geometry, not native authoring.
use std::{path::PathBuf, sync::Arc, time::Duration};

use onecad_app::{
    document_runtime::DocumentRuntime,
    dto::SketchHistoryScope,
    worker::{resolve_worker_path, SupervisorConfig, WorkerManager},
};
use onecad_core::{
    document::variables::Scalar,
    edit::{EditCommand, HistoryDirection},
    ids::{ConstraintId, EntityId, SketchId, SnapshotId},
    io::container::SaveMeta,
    math::Vec2,
    sketch::{Constraint, CurvePosition as Role, Sketch, SketchEntity, WorldPlane},
};
use uuid::Uuid;

fn eid(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}
fn cid(n: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(n))
}
fn point(sketch: &mut Sketch, n: u128, x: f64, y: f64) {
    sketch
        .add_entity(SketchEntity::point(
            eid(n),
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .unwrap();
}
fn tangent(id: u128, a: u128, b: u128, ra: Role, rb: Role) -> Constraint {
    Constraint::TangentAtPoint {
        id: cid(id),
        entity1: eid(a),
        entity2: eid(b),
        entity1_position: ra,
        entity2_position: rb,
    }
}
fn weld(id: u128, a: u128, b: u128, ra: Role, rb: Role) -> Constraint {
    Constraint::Coincident {
        id: cid(id),
        point1: eid(a),
        point2: eid(b),
        point1_position: ra,
        point2_position: rb,
    }
}
fn fixture(arc_arc: bool) -> Sketch {
    let mut sketch = Sketch::on_world_plane(SketchId::new(), "Endpoint tangency", WorldPlane::XY);
    point(&mut sketch, 1, 0., 5.);
    sketch
        .add_entity(
            SketchEntity::arc(eid(10), eid(1), 5., -std::f64::consts::FRAC_PI_2, 0., false)
                .unwrap(),
        )
        .unwrap();
    if arc_arc {
        point(&mut sketch, 2, 0., -5.);
        sketch
            .add_entity(
                SketchEntity::arc(
                    eid(11),
                    eid(2),
                    5.,
                    std::f64::consts::FRAC_PI_2,
                    std::f64::consts::PI,
                    false,
                )
                .unwrap(),
            )
            .unwrap();
    } else {
        point(&mut sketch, 2, 0., 0.);
        point(&mut sketch, 3, 20., 0.);
        sketch
            .add_entity(SketchEntity::line(eid(11), eid(2), eid(3), false))
            .unwrap();
    }
    sketch
        .add_constraint(weld(20, 10, 11, Role::Start, Role::Start))
        .unwrap();
    sketch
}
async fn worker() -> Option<WorkerManager> {
    let path = std::env::var("ONECAD_WORKER_PATH")
        .ok()
        .map(PathBuf::from)
        .or_else(resolve_worker_path);
    assert!(
        path.as_ref().is_none_or(|p| p.is_file()),
        "configured worker missing"
    );
    assert!(
        path.is_some() || std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "required worker missing"
    );
    let wm = WorkerManager::spawn(SupervisorConfig::production(path?));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "production worker handshake"
    );
    Some(wm)
}
fn runtime(wm: &WorkerManager) -> DocumentRuntime {
    DocumentRuntime::new_blank(
        Arc::new(wm.clone()),
        Arc::new(wm.clone()),
        Arc::new(wm.clone()),
    )
}
fn point_at(sketch: &Sketch, id: EntityId) -> [f64; 2] {
    let Some(SketchEntity::Point { at, .. }) = sketch.get_entity(id) else {
        panic!("point missing")
    };
    [at.x, at.y]
}
fn endpoint(sketch: &Sketch, id: EntityId, role: Role) -> ([f64; 2], [f64; 2], f64) {
    match sketch.get_entity(id).unwrap() {
        SketchEntity::Line { start, end, .. } => {
            let a = point_at(sketch, *start);
            let b = point_at(sketch, *end);
            (
                if role == Role::Start { a } else { b },
                [b[0] - a[0], b[1] - a[1]],
                (b[0] - a[0]).hypot(b[1] - a[1]),
            )
        }
        SketchEntity::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => {
            let c = point_at(sketch, *center);
            let angle = if role == Role::Start {
                *start_angle
            } else {
                *end_angle
            };
            (
                [c[0] + radius * angle.cos(), c[1] + radius * angle.sin()],
                [-angle.sin(), angle.cos()],
                *radius,
            )
        }
        _ => panic!("unsupported tangent operand"),
    }
}
fn assert_truth(sketch: &Sketch) -> f64 {
    let (a, u, l1) = endpoint(sketch, eid(10), Role::Start);
    let (b, v, l2) = endpoint(sketch, eid(11), Role::Start);
    assert!((a[0] - b[0]).hypot(a[1] - b[1]) <= 0.0001, "endpoint weld");
    let cross = u[0] * v[1] - u[1] * v[0];
    let dot = u[0] * v[0] + u[1] * v[1];
    let angular = cross.atan2(dot).sin().abs().asin();
    assert!(angular <= 0.0001 / l1.max(l2), "angular residual {angular}");
    assert_eq!(
        sketch.get_constraint(cid(21)).unwrap(),
        &tangent(21, 10, 11, Role::Start, Role::Start)
    );
    dot.signum()
}
fn assert_sketch_close(actual: &Sketch, expected: &Sketch) {
    assert_eq!(actual.constraints(), expected.constraints());
    fn compare(a: &serde_json::Value, b: &serde_json::Value) {
        match (a, b) {
            (serde_json::Value::Number(a), serde_json::Value::Number(b)) => {
                assert!((a.as_f64().unwrap() - b.as_f64().unwrap()).abs() <= 1e-12)
            }
            (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
                assert_eq!(a.len(), b.len());
                for (a, b) in a.iter().zip(b) {
                    compare(a, b);
                }
            }
            (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
                assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
                for (key, a) in a {
                    compare(a, &b[key]);
                }
            }
            _ => assert_eq!(a, b),
        }
    }
    // History re-solves can normalize the welded point by floating-point roundoff.
    compare(
        &serde_json::to_value(actual).unwrap(),
        &serde_json::to_value(expected).unwrap(),
    );
}

async fn history(rt: &mut DocumentRuntime, sid: SketchId, direction: HistoryDirection) {
    let scope = SketchHistoryScope {
        sketch_id: sid,
        edit_session: rt.get_sketch(sid).unwrap().edit_session.unwrap(),
        runtime_session: rt.runtime_session().to_string(),
        revision: rt.revision().0,
        snapshot_id: rt.head_snapshot_id().unwrap_or(SnapshotId(0)),
    };
    let prepared = rt
        .prepare_sketch_history(scope, direction)
        .await
        .unwrap()
        .unwrap();
    assert!(
        rt.commit_sketch_history(prepared.drive().await.unwrap())
            .unwrap()
            .0
            .moved
    );
}
fn meta() -> SaveMeta {
    SaveMeta {
        app_version: "endpoint-tangency-test".into(),
        occt_fingerprint: None,
        created: "2026-10-19T00:00:00Z".into(),
        modified: "2026-10-19T00:00:00Z".into(),
    }
}
async fn author(rt: &mut DocumentRuntime, sid: SketchId, before: u32) {
    let result = rt
        .add_sketch_constraints_with_outcome(
            sid,
            vec![tangent(21, 10, 11, Role::Start, Role::Start)],
        )
        .await
        .unwrap()
        .0;
    assert!(result.accepted);
    assert_eq!(result.solve.dof, before - 1);
    let result = rt
        .add_sketch_constraints_with_outcome(
            sid,
            vec![Constraint::Radius {
                id: cid(22),
                entity: eid(10),
                value: Scalar::new(5.),
            }],
        )
        .await
        .unwrap()
        .0;
    assert!(result.accepted);
    assert_eq!(result.solve.dof, before - 2);
}

async fn assert_rejected_edit(rt: &mut DocumentRuntime, sid: SketchId) {
    let accepted = rt.sketch_snapshot(sid, "test").unwrap();
    let depth = rt.get_sketch(sid).unwrap().history.unwrap().undo_depth;
    let invalid = tangent(23, 10, 11, Role::Center, Role::Start);
    assert!(rt
        .add_sketch_constraints_with_outcome(sid, vec![invalid])
        .await
        .is_err());
    assert_eq!(rt.sketch_snapshot(sid, "rejected").unwrap(), accepted);
    assert_eq!(
        rt.get_sketch(sid).unwrap().history.unwrap().undo_depth,
        depth
    );
}

async fn edit_and_history(rt: &mut DocumentRuntime, sid: SketchId) -> Sketch {
    let baseline = rt.sketch_snapshot(sid, "test").unwrap();
    let branch = assert_truth(&baseline);
    let result = rt
        .set_sketch_dimension_with_outcome(sid, cid(22), Scalar::new(7.))
        .await
        .unwrap()
        .0;
    assert!(result.accepted);
    let edited = rt.sketch_snapshot(sid, "test").unwrap();
    assert_eq!(assert_truth(&edited), branch);
    assert_eq!(
        edited
            .entities()
            .iter()
            .map(SketchEntity::id)
            .collect::<Vec<_>>(),
        baseline
            .entities()
            .iter()
            .map(SketchEntity::id)
            .collect::<Vec<_>>()
    );
    assert!(
        (edited
            .get_constraint(cid(22))
            .unwrap()
            .value()
            .unwrap()
            .value
            - 7.)
            .abs()
            < 1e-12
    );
    let Some(SketchEntity::Arc { radius, .. }) = edited.get_entity(eid(10)) else {
        panic!("arc")
    };
    assert!((radius - 7.).abs() <= 0.0001, "actual solved radius");
    history(rt, sid, HistoryDirection::Undo).await;
    assert_eq!(rt.sketch_snapshot(sid, "test").unwrap(), baseline);
    history(rt, sid, HistoryDirection::Redo).await;
    assert_sketch_close(&rt.sketch_snapshot(sid, "test").unwrap(), &edited);
    edited
}

async fn reopen(path: &std::path::Path, sid: SketchId, edited: &Sketch, dof: u32) {
    let wm2 = worker().await.expect("fresh production worker");
    let mut rt = DocumentRuntime::open(
        path,
        Arc::new(wm2.clone()),
        Arc::new(wm2.clone()),
        Arc::new(wm2.clone()),
    )
    .unwrap();
    assert_sketch_close(&rt.sketch_snapshot(sid, "reopen").unwrap(), edited);
    let solved = rt.enter_sketch(sid).await.unwrap();
    assert_eq!(solved.dof, dof);
    let reopened = rt.sketch_snapshot(sid, "reopen").unwrap();
    assert_eq!(assert_truth(&reopened), assert_truth(edited));
    assert_eq!(reopened.constraints(), edited.constraints());
    wm2.shutdown().await;
}

async fn exercise(arc_arc: bool) {
    let Some(wm) = worker().await else {
        return;
    };
    let mut rt = runtime(&wm);
    let sketch = fixture(arc_arc);
    let sid = sketch.id;
    rt.apply(EditCommand::AddSketch { sketch }).unwrap();
    let before = rt.enter_sketch(sid).await.unwrap().dof;
    author(&mut rt, sid, before).await;
    assert_rejected_edit(&mut rt, sid).await;
    let edited = edit_and_history(&mut rt, sid).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("endpoint.onecad");
    rt.save(&path, meta()).unwrap();
    drop(rt);
    wm.shutdown().await;
    reopen(&path, sid, &edited, before - 2).await;
}

#[tokio::test]
async fn line_arc_radius_edit_history_and_fresh_worker_reopen() {
    exercise(false).await;
}
#[tokio::test]
async fn arc_arc_radius_edit_history_and_fresh_worker_reopen() {
    exercise(true).await;
}

#[path = "endpoint_tangency/no_motion.rs"]
mod no_motion;
