//! Pure native primitive effects admitted by the real application/PlaneGCS services.
//! Fresh-process persistence is service evidence, never a physical native checkpoint.
#[path = "real_point_center/authorship.rs"]
mod authorship;
#[allow(dead_code)]
#[path = "../../onecad-app/tests/native_plate_support/host.rs"]
mod host;
#[path = "real_point_center/support.rs"]
mod support;
use host::Harness;
use onecad_app::{api, dto::SketchSolveStatus};
use onecad_core::{
    document::variables::Scalar,
    edit::{HistoryDirection, SketchEditOp},
    ids::{ConstraintId, EntityId, SketchId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane},
};
use onecad_interaction::{sketch::*, DocumentContext};
use onecad_scene::{Camera, DVec3, Projection, SceneIdentity, ViewSnapshot, Viewport};
use std::path::{Path, PathBuf};
use support::*;
use uuid::Uuid;

fn sketch_id() -> SketchId {
    SketchId(Uuid::from_u128(0x29000))
}
fn fixed_center() -> Constraint {
    Constraint::Fixed {
        id: ids(1000).constraints[11],
        point: ids(1000).entities[8],
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(10.0, 20.0),
    }
}
fn size_constraints() -> Vec<Constraint> {
    vec![
        Constraint::Distance {
            id: ids(1000).constraints[8],
            entity1: ids(1000).entities[0],
            entity1_position: CurvePosition::Arbitrary,
            entity2: ids(1000).entities[1],
            entity2_position: CurvePosition::Arbitrary,
            value: Scalar::new(80.0),
        },
        Constraint::Distance {
            id: ids(1000).constraints[9],
            entity1: ids(1000).entities[0],
            entity1_position: CurvePosition::Arbitrary,
            entity2: ids(1000).entities[3],
            entity2_position: CurvePosition::Arbitrary,
            value: Scalar::new(50.0),
        },
    ]
}

#[test]
fn center_rectangle_dof_edit_gestures_history_and_fresh_reopen() {
    isolated(
        "center_rectangle_dof_edit_gestures_history_and_fresh_reopen",
        "center",
        |root, phase| {
            Box::pin(async move {
                if phase == "center-reopen" {
                    reopened(&root).await;
                } else {
                    centered(&root).await;
                }
            })
        },
    );
}

#[test]
fn point_numeric_expressions_and_construction_are_real_solver_state() {
    isolated(
        "point_numeric_expressions_and_construction_are_real_solver_state",
        "point",
        |root, phase| {
            Box::pin(async move {
                if phase == "point-reopen" {
                    reopened_point(&root).await;
                } else {
                    points(&root).await;
                }
            })
        },
    );
}

fn isolated<F>(name: &str, phase: &str, run: F)
where
    F: FnOnce(PathBuf, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>,
{
    host::required_worker();
    if let Ok(current) = std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run(root, current));
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("onecad-point-center-")
        .tempdir()
        .unwrap();
    host::run_isolated_phase(root.path(), phase, name);
    host::run_isolated_phase(root.path(), &format!("{phase}-reopen"), name);
}

async fn new(root: &Path) -> Harness {
    let harness = Harness::new(&root.join("host")).await;
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(
        harness
            .state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    harness
}

async fn centered(root: &Path) {
    let harness = new(root).await;
    let mut state = begin(&harness, true).await;
    state.dispatch(SketchAction::SetTool(SketchTool::CenterRectangle));
    let first = captured(&mut state, Vec2::new_unchecked(10.0, 20.0));
    assert!(state
        .dispatch(SketchAction::Click {
            point: first,
            ids: ids(500)
        })
        .is_empty());
    let corner = captured(&mut state, Vec2::new_unchecked(50.0, 45.0));
    let effects = state.dispatch(SketchAction::Click {
        point: corner,
        ids: ids(1000),
    });
    let solve = submit(&harness, &mut state, effects).await;
    assert_eq!(solve.dof, 4);
    assert_eq!(solve.status, SketchSolveStatus::UnderConstrained);
    add(&harness, &mut state, vec![fixed_center()], 2).await;
    add(&harness, &mut state, size_constraints(), 0).await;
    regions(&harness, 1).await;
    width_edit(&harness, &mut state).await;
    eprintln!("center: DOF, size edit and local history passed");
    gesture_contracts(&harness).await;
    eprintln!("center: gesture contracts passed");
    close_visit(&harness).await;
    let finished = api::finish_sketch(
        &harness.state,
        harness.host.clone(),
        sketch_id().to_string(),
    )
    .await
    .unwrap();
    assert_eq!(finished.regions.len(), 1);
    let saved = api::save_document(
        &harness.state,
        harness.host.clone(),
        Some(root.join("center.onecad").to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean);
    assert!(!harness.projection().await.dirty);
    harness.close().await;
}

async fn add(
    harness: &Harness,
    state: &mut SketchInteraction,
    constraints: Vec<Constraint>,
    dof: u32,
) {
    let effects = state.dispatch(SketchAction::AddConstraints {
        key: state.session.as_ref().unwrap().key.clone(),
        constraints,
    });
    let solve = submit(harness, state, effects).await;
    assert_eq!(solve.dof, dof);
    assert_eq!(
        solve.status,
        if dof == 0 {
            SketchSolveStatus::FullyConstrained
        } else {
            SketchSolveStatus::UnderConstrained
        }
    );
}

async fn width_edit(harness: &Harness, state: &mut SketchInteraction) {
    let before = sketch(harness).await;
    let effects = state.dispatch(SketchAction::SetDimension {
        key: state.session.as_ref().unwrap().key.clone(),
        constraint: ids(1000).constraints[8],
        value: Scalar::new(90.0),
    });
    assert_eq!(submit(harness, state, effects).await.dof, 0);
    assert_width(&sketch(harness).await, 90.0);
    history(harness, state, HistoryDirection::Undo).await;
    assert_eq!(sketch(harness).await, before);
    history(harness, state, HistoryDirection::Redo).await;
    assert_width(&sketch(harness).await, 90.0);
}

fn assert_width(sketch: &Sketch, width: f64) {
    let p = ids(1000);
    assert_eq!(sketch.entities().len(), 10);
    assert_eq!(sketch.constraints().len(), 8);
    let entity_ids: std::collections::BTreeSet<_> =
        sketch.entities().iter().map(SketchEntity::id).collect();
    assert_eq!(entity_ids, p.entities[..10].iter().copied().collect());
    let constraint_ids: std::collections::BTreeSet<_> =
        sketch.constraints().iter().map(Constraint::id).collect();
    let expected = [0, 1, 2, 3, 8, 9, 10, 11].map(|index| p.constraints[index]);
    assert_eq!(constraint_ids, expected.into_iter().collect());
    symmetric(sketch, &p, Vec2::new_unchecked(10.0, 20.0));
    near(at(sketch, p.entities[0]), 10.0 - width * 0.5, -5.0);
    near(at(sketch, p.entities[2]), 10.0 + width * 0.5, 45.0);
    assert!(
        matches!(sketch.get_constraint(p.constraints[10]), Some(Constraint::Midpoint { point, line, .. }) if *point == p.entities[8] && *line == p.entities[9])
    );
    assert!(matches!(
        sketch.get_entity(p.entities[9]),
        Some(SketchEntity::Line {
            construction: true,
            ..
        })
    ));
}

async fn remove(harness: &Harness, constraints: &[ConstraintId]) {
    let ops = constraints
        .iter()
        .map(|id| SketchEditOp::RemoveConstraint { constraint: *id })
        .collect();
    let receipt = api::repair_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        sketch_id().to_string(),
        ops,
    )
    .await
    .unwrap();
    if receipt.regen_expected {
        harness.settled(&receipt.projection).await;
    }
    assert!(receipt.result.unwrap().accepted);
}

async fn drag(harness: &Harness, point: EntityId, target: [f64; 2]) {
    let before = harness.projection().await;
    api::begin_gesture(
        &harness.state,
        sketch_id().to_string(),
        point.to_string(),
        None,
    )
    .await
    .unwrap();
    let preview = api::solve_drag(&harness.state, target).await.unwrap();
    assert!(preview.conflicting.is_empty());
    api::end_gesture(&harness.state, harness.host.clone(), Some(target))
        .await
        .unwrap();
    assert_eq!(harness.projection().await.undo_depth, before.undo_depth + 1);
}

async fn gesture_contracts(harness: &Harness) {
    remove(harness, &[ids(1000).constraints[11]]).await;
    drag(harness, ids(1000).entities[8], [25.0, 35.0]).await;
    let translated = sketch(harness).await;
    symmetric(&translated, &ids(1000), Vec2::new_unchecked(25.0, 35.0));
    near(at(&translated, ids(1000).entities[0]), -20.0, 10.0);
    undo(harness).await;
    undo(harness).await;
    assert_width(&sketch(harness).await, 90.0);
    remove(
        harness,
        &[ids(1000).constraints[8], ids(1000).constraints[9]],
    )
    .await;
    drag(harness, ids(1000).entities[2], [60.0, 50.0]).await;
    let resized = sketch(harness).await;
    symmetric(&resized, &ids(1000), Vec2::new_unchecked(10.0, 20.0));
    near(at(&resized, ids(1000).entities[0]), -40.0, -10.0);
    undo(harness).await;
    undo(harness).await;
    assert_width(&sketch(harness).await, 90.0);
}

/// Opens a saved document in a fresh host and waits for its initial regen to publish.
///
/// `open_document` only schedules the initial regen. The worker head moves at
/// `AcceptPrepared` before Rust publishes that snapshot, so a `preview_context`
/// taken in between is correctly refused as `StalePreview` (ONECAD-441).
/// `RegenFinished` is emitted only after `finish_regen` publishes, so it is the
/// deterministic barrier; worker readiness alone is not.
async fn reopen(host: &Path, document: &Path) -> Harness {
    let harness = Harness::new(host).await;
    api::open_document(
        &harness.state,
        harness.host.clone(),
        document.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let opened = harness.projection().await;
    assert!(!opened.dirty);
    harness.settled(&opened).await;
    assert!(!harness.projection().await.dirty);
    harness
}

async fn reopened(root: &Path) {
    let harness = reopen(&root.join("reopen-host"), &root.join("center.onecad")).await;
    assert_width(&sketch(&harness).await, 90.0);
    let state = begin(&harness, false).await;
    let solve = api::get_sketch(&harness.state, sketch_id().to_string())
        .await
        .unwrap();
    assert!(solve.solve_current);
    assert_eq!(solve.dof, 0);
    assert_eq!(solve.status, SketchSolveStatus::FullyConstrained);
    assert_width(&state.session.as_ref().unwrap().authoritative, 90.0);
    regions(&harness, 1).await;
    close_visit(&harness).await;
    harness.close().await;
}

async fn points(root: &Path) {
    let harness = new(root).await;
    api::upsert_variable(&harness.state, harness.host.clone(), "offset".into(), -15.0)
        .await
        .unwrap();
    let mut state = begin(&harness, true).await;
    let accepted = numeric_point(&harness, &mut state).await;
    state.dispatch(SketchAction::SetConstructionMode {
        key: state.session.as_ref().unwrap().key.clone(),
        construction: true,
    });
    let point = captured(&mut state, Vec2::new_unchecked(5.0, 9.0));
    let effects = state.dispatch(SketchAction::Click {
        point,
        ids: ids(3000),
    });
    assert_eq!(submit(&harness, &mut state, effects).await.dof, 2);
    assert!(matches!(
        sketch(&harness).await.get_entity(ids(3000).entities[0]),
        Some(SketchEntity::Point {
            construction: true,
            ..
        })
    ));
    history(&harness, &mut state, HistoryDirection::Undo).await;
    assert_eq!(sketch(&harness).await, accepted);
    history(&harness, &mut state, HistoryDirection::Redo).await;
    regions(&harness, 0).await;
    close_visit(&harness).await;
    let saved = api::save_document(
        &harness.state,
        harness.host.clone(),
        Some(root.join("point.onecad").to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean);

    harness.close().await;
}

async fn numeric_point(harness: &Harness, state: &mut SketchInteraction) -> Sketch {
    state.dispatch(SketchAction::SetTool(SketchTool::Point));
    let _ = captured(state, Vec2::new_unchecked(0.0, 0.0));
    state.dispatch(SketchAction::BeginPointNumeric {
        key: state.session.as_ref().unwrap().key.clone(),
    });
    let session = state.session.as_ref().unwrap();
    let effects = state.dispatch(SketchAction::CreateNumeric {
        key: session.key.clone(),
        capture: session.accepted_capture.unwrap(),
        ids: ids(2000),
        dimensions: Box::new(CreationDimensions::Point {
            x: Scalar::with_expr(123.0, "offset"),
            y: Scalar::new(0.0),
        }),
    });
    let solve = submit(harness, state, effects).await;
    assert_eq!(solve.dof, 0);
    assert_eq!(solve.status, SketchSolveStatus::FullyConstrained);
    let accepted = sketch(harness).await;
    near(at(&accepted, ids(2000).entities[0]), -15.0, 0.0);
    assert!(
        matches!(accepted.get_constraint(ids(2000).constraints[8]), Some(Constraint::HorizontalDistance { value, .. }) if value.expr.as_deref() == Some("offset"))
    );
    accepted
}

async fn reopened_point(root: &Path) {
    let harness = reopen(&root.join("point-reopen-host"), &root.join("point.onecad")).await;
    let _ = begin(&harness, false).await;
    let accepted = sketch(&harness).await;
    near(at(&accepted, ids(2000).entities[0]), -15.0, 0.0);
    assert!(
        matches!(accepted.get_constraint(ids(2000).constraints[8]), Some(Constraint::HorizontalDistance { value, point1, point2, .. }) if value.expr.as_deref() == Some("offset") && *point1 == ids(2000).entities[1] && *point2 == ids(2000).entities[0])
    );
    assert!(matches!(
        accepted.get_entity(ids(3000).entities[0]),
        Some(SketchEntity::Point {
            construction: true,
            ..
        })
    ));
    let solve = api::get_sketch(&harness.state, sketch_id().to_string())
        .await
        .unwrap();
    assert!(solve.solve_current);
    assert_eq!(solve.dof, 2);
    regions(&harness, 0).await;
    close_visit(&harness).await;
    harness.close().await;
}

async fn undo(harness: &Harness) {
    let outcome = api::undo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    if outcome.regen_expected {
        harness.settled(&outcome.projection).await;
    }
}
