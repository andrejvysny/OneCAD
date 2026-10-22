//! Pure sketch effects through actual PlaneGCS, the shared app and OCCT modeling.
//! Synthetic 30×20×5 mm data; this is automated service evidence, not native input.
#[allow(dead_code)]
#[path = "../../onecad-app/tests/native_plate_support/host.rs"]
mod host;

use host::Harness;
use onecad_app::{
    api,
    dto::{DocumentProjection, FinishSketchDto, SketchSolveStatus},
};
use onecad_core::{
    document::{
        record::{
            BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
        },
        refs::SketchRegionRef,
        variables::Scalar,
    },
    edit::EditCommand,
    ids::{ConstraintId, EntityId, RecordId, RegionId, SketchId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, WorldPlane},
};
use onecad_interaction::{sketch::*, DocumentContext};
use onecad_scene::{Camera, DVec3, Projection, SceneIdentity, ViewSnapshot, Viewport};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[path = "real_sketch_modeling/geometry.rs"]
mod geometry;
use geometry::assert_solid;

const SKETCH: u128 = 0x273b0;
const EXTRUDE: u128 = 0x273b1;
fn sketch_id() -> SketchId {
    SketchId(Uuid::from_u128(SKETCH))
}
fn extrude_id() -> RecordId {
    RecordId(Uuid::from_u128(EXTRUDE))
}

#[test]
fn emitted_rectangle_reaches_real_solver_and_editable_solid() {
    host::required_worker();
    if std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE").as_deref() == Ok("sketch") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(workflow(&root));
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("onecad-real-sketch-")
        .tempdir()
        .unwrap();
    host::run_isolated_phase(
        root.path(),
        "sketch",
        "emitted_rectangle_reaches_real_solver_and_editable_solid",
    );
}

async fn workflow(root: &Path) {
    let harness = Harness::new(&root.join("sketch-host")).await;
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    let worker = harness.state.live_worker().unwrap();
    assert!(worker.wait_ready(std::time::Duration::from_secs(15)).await);
    let mut interaction = begin(&harness, true).await;
    let primitive = rectangle(&harness, &mut interaction).await;
    constrain(&harness, &mut interaction, &primitive).await;
    let regions = finish(&harness, &mut interaction).await;
    let before = harness.projection().await;
    assert_eq!(before.features.len(), 1);
    assert!(before.bodies.is_empty());
    let committed = commit(&harness, extrude(&regions, 5.0), false).await;
    assert_eq!(committed.undo_depth, before.undo_depth + 1);
    assert_solid(&harness, &committed, 5.0).await;
    assert!(
        api::get_operation_record(&harness.state, extrude_id())
            .await
            .unwrap()
            .op
            == extrude(&regions, 5.0)
    );
    edit_and_history(&harness, &regions, &committed).await;
    let reentered = begin(&harness, false).await;
    let session = reentered.session.as_ref().unwrap();
    assert_eq!(session.phase, SketchPhase::Active);
    assert_eq!(session.authoritative.entities().len(), 8);
    assert_eq!(session.authoritative.constraints().len(), 7);
    let solve = api::get_sketch(&harness.state, sketch_id().to_string())
        .await
        .unwrap();
    assert_eq!(solve.dof, 0);
    assert!(solve.solve_current);
    api::cancel_sketch(
        &harness.state,
        harness.host.clone(),
        sketch_id().to_string(),
        Some(false),
    )
    .await
    .unwrap();
    harness.close().await;
}

async fn context(harness: &Harness) -> DocumentContext {
    let head = api::preview_context(&harness.state).await.unwrap();
    DocumentContext {
        document_id: head.document_id,
        runtime_session: head.runtime_session,
        revision: head.revision.0,
        snapshot_id: head.snapshot_id,
        scene: SceneIdentity {
            session: 1,
            scene: head.snapshot_id.0,
        },
    }
}

async fn snapshot(harness: &Harness) -> SketchSnapshot {
    SketchSnapshot {
        history: None,
        document: context(harness).await,
        sketch: Box::new(
            api::get_sketch_snapshot(&harness.state, sketch_id())
                .await
                .unwrap(),
        ),
    }
}

async fn begin(harness: &Harness, create: bool) -> SketchInteraction {
    let mut state = SketchInteraction::default();
    let document = context(harness).await;
    state.dispatch(SketchAction::ContextChanged(Some(document.clone())));
    let sketch = if create {
        Sketch::on_world_plane(sketch_id(), "Emitted 30 x 20 mm rectangle", WorldPlane::XY)
    } else {
        api::get_sketch_snapshot(&harness.state, sketch_id())
            .await
            .unwrap()
    };
    let mut effects = state.dispatch(SketchAction::Begin {
        sketch: Box::new(sketch),
        create,
        context: document,
    });
    assert_eq!(effects.len(), 1);
    if create {
        let SketchEffect::CreateSketch { tag, command } = effects.remove(0) else {
            panic!("expected authoring effect")
        };
        api::apply_edit_command(&harness.state, harness.host.clone(), *command)
            .await
            .unwrap();
        state.dispatch(SketchAction::ContextChanged(Some(context(harness).await)));
        effects = state.dispatch(SketchAction::Created { tag });
    }
    let SketchEffect::EnterSketch { tag, created_here } = effects.remove(0) else {
        panic!("expected enter effect")
    };
    let solve = api::enter_sketch(
        &harness.state,
        harness.host.clone(),
        tag.key.sketch_id.to_string(),
        Some(created_here),
    )
    .await
    .unwrap();
    let snapshot = snapshot(harness).await;
    let document = snapshot.document.clone();
    state.dispatch(SketchAction::ContextChanged(Some(document)));
    state.dispatch(SketchAction::Entered {
        tag,
        snapshot,
        solve: Box::new(solve),
    });
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
    state
}

fn ids(seed: u128) -> PrimitiveIds {
    PrimitiveIds {
        entities: std::array::from_fn(|i| EntityId(Uuid::from_u128(seed + i as u128))),
        constraints: std::array::from_fn(|i| ConstraintId(Uuid::from_u128(seed + 100 + i as u128))),
    }
}

fn captured(state: &mut SketchInteraction, point: Vec2) -> CapturedSketchPoint {
    let session = state.session.as_ref().unwrap();
    let view = ViewSnapshot::new(
        session.document.scene,
        1,
        Camera {
            target: DVec3::ZERO,
            offset: DVec3::new(0.0, 0.0, 100.0),
            projection: Projection::Orthographic,
            ..Camera::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 2.0,
        },
    )
    .unwrap();
    let world = session.plane.to_world(point);
    let logical = view
        .project(DVec3::new(world.x, world.y, world.z))
        .unwrap()
        .logical;
    let point = capture_sketch_pointer(session, &view, logical).unwrap();
    state.dispatch(SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view.identity(),
    });
    point
}

async fn rectangle(harness: &Harness, state: &mut SketchInteraction) -> PrimitiveIds {
    state.dispatch(SketchAction::SetTool(SketchTool::Rectangle));
    let first = captured(state, Vec2::new_unchecked(0.0, 0.0));
    assert!(state
        .dispatch(SketchAction::Click {
            point: first,
            ids: ids(0x273c0)
        })
        .is_empty());
    let opposite = captured(state, Vec2::new_unchecked(30.0, 20.0));
    state.dispatch(SketchAction::Move(opposite.clone()));
    assert!(matches!(
        state.session.as_ref().unwrap().preview,
        Some(DraftPrimitive::Rectangle { .. })
    ));
    assert!(
        api::get_sketch_snapshot(&harness.state, sketch_id())
            .await
            .unwrap()
            .entities()
            .is_empty(),
        "pointer preview is not authored geometry"
    );
    let primitive = ids(0x273d0);
    let effects = state.dispatch(SketchAction::Click {
        point: opposite,
        ids: primitive.clone(),
    });
    let solve = submit(harness, state, effects, 12).await;
    // Four shared 2D points give eight variables; four H/V constraints leave
    // translation X/Y, width and height free. This DOF expectation is analytic.
    assert_eq!(solve.dof, 4);
    assert_eq!(solve.status, SketchSolveStatus::UnderConstrained);
    let authoritative = &state.session.as_ref().unwrap().authoritative;
    assert_eq!(authoritative.entities().len(), 8);
    assert_eq!(authoritative.constraints().len(), 4);
    primitive
}

async fn submit(
    harness: &Harness,
    state: &mut SketchInteraction,
    mut effects: Vec<SketchEffect>,
    count: usize,
) -> onecad_app::dto::SketchUpsertDto {
    assert_eq!(effects.len(), 1);
    assert_eq!(
        state.session.as_ref().unwrap().phase,
        SketchPhase::Submitting
    );
    let (tag, solve) = match effects.remove(0) {
        SketchEffect::Upsert { tag, ops } => {
            assert_eq!(ops.len(), count);
            let solve = api::sketch_upsert(
                &harness.state,
                harness.host.clone(),
                tag.key.sketch_id.to_string(),
                ops,
            )
            .await
            .unwrap();
            (tag, solve)
        }
        SketchEffect::AddConstraints { tag, constraints } => {
            assert_eq!(constraints.len(), count);
            let receipt = api::add_sketch_constraints_with_completion(
                &harness.state,
                harness.host.clone(),
                tag.key.sketch_id.to_string(),
                constraints,
            )
            .await
            .unwrap();
            let admission = receipt.result.unwrap();
            assert!(admission.accepted);
            (tag, admission.solve)
        }
        _ => panic!("expected emitted sketch batch"),
    };
    assert!(solve.conflicting.is_empty());
    state.dispatch(SketchAction::BatchAccepted { tag: tag.clone() });
    let snapshot = snapshot(harness).await;
    state.dispatch(SketchAction::ContextChanged(Some(
        snapshot.document.clone(),
    )));
    state.dispatch(SketchAction::Solved {
        tag,
        snapshot,
        solve: Box::new(solve.clone()),
    });
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
    solve
}

async fn constrain(harness: &Harness, state: &mut SketchInteraction, primitive: &PrimitiveIds) {
    let position = CurvePosition::Arbitrary;
    let constraints = vec![
        Constraint::Fixed {
            id: ConstraintId(Uuid::from_u128(0x273e0)),
            point: primitive.entities[0],
            point_position: position,
            at: Vec2::new_unchecked(0.0, 0.0),
        },
        Constraint::HorizontalDistance {
            id: ConstraintId(Uuid::from_u128(0x273e1)),
            point1: primitive.entities[0],
            point1_position: position,
            point2: primitive.entities[1],
            point2_position: position,
            value: Scalar::new(30.0),
        },
        Constraint::VerticalDistance {
            id: ConstraintId(Uuid::from_u128(0x273e2)),
            point1: primitive.entities[1],
            point1_position: position,
            point2: primitive.entities[2],
            point2_position: position,
            value: Scalar::new(20.0),
        },
    ];
    let effects = state.dispatch(SketchAction::AddConstraints {
        key: state.session.as_ref().unwrap().key.clone(),
        constraints,
    });
    let solve = submit(harness, state, effects, 3).await;
    assert_eq!(solve.dof, 0);
    assert_eq!(solve.status, SketchSolveStatus::FullyConstrained);
}

async fn finish(harness: &Harness, state: &mut SketchInteraction) -> FinishSketchDto {
    let mut effects = state.dispatch(SketchAction::Finish);
    assert_eq!(effects.len(), 1);
    let SketchEffect::FinishSketch { tag, reason } = effects.remove(0) else {
        panic!("expected finish effect")
    };
    assert_eq!(reason, FinishReason::Explicit);
    api::cancel_sketch(
        &harness.state,
        harness.host.clone(),
        tag.key.sketch_id.to_string(),
        Some(false),
    )
    .await
    .unwrap();
    let regions = api::finish_sketch(
        &harness.state,
        harness.host.clone(),
        tag.key.sketch_id.to_string(),
    )
    .await
    .unwrap();
    harness.settled(&harness.projection().await).await;
    assert_eq!(regions.region_identity_version, 3);
    assert_eq!(regions.regions.len(), 1);
    assert!(regions.regions[0].holes.is_empty());
    assert!(!regions.regions[0]
        .preview_triangles
        .as_ref()
        .unwrap()
        .indices
        .is_empty());
    state.dispatch(SketchAction::Finished {
        warning: None,
        tag,
        result: Box::new(regions.clone()),
    });
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Closed);
    regions
}

fn extrude(regions: &FinishSketchDto, depth: f64) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch: sketch_id(),
            region: RegionId::new(&regions.regions[0].region_id),
            region_identity_version: Some(regions.region_identity_version),
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(depth),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }))
}

async fn commit(harness: &Harness, operation: Operation, edit: bool) -> DocumentProjection {
    let command = if edit {
        EditCommand::UpdateOperationParams {
            record: extrude_id(),
            op: operation,
        }
    } else {
        EditCommand::AddOperation {
            record: OperationRecord::new(extrude_id(), 1, "Emitted profile extrusion", operation),
            at_cursor: true,
        }
    };
    let effects = (!edit).then(|| api::OperationEffects {
        hide_sketch_ids: vec![sketch_id().to_string()],
    });
    let response =
        api::apply_operation(&harness.state, harness.host.clone(), vec![command], effects)
            .await
            .unwrap();
    harness.settled(&response.projection).await
}

async fn edit_and_history(
    harness: &Harness,
    regions: &FinishSketchDto,
    original: &DocumentProjection,
) {
    let edited = commit(harness, extrude(regions, 7.0), true).await;
    assert_eq!(edited.undo_depth, original.undo_depth + 1);
    assert_solid(harness, &edited, 7.0).await;
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    let undo = harness.settled(&undo).await;
    assert_eq!(undo.undo_depth, original.undo_depth);
    assert_solid(harness, &undo, 5.0).await;
    let redo = api::redo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    let redo = harness.settled(&redo).await;
    assert_solid(harness, &redo, 7.0).await;
}
