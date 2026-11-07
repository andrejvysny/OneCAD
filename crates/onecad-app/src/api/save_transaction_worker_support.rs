use super::*;
use onecad_core::{
    document::{
        record::{
            BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
            PlaneKind, SketchOpParams, SketchPlaneRef,
        },
        refs::SketchRegionRef,
        variables::Scalar,
    },
    edit::EditCommand,
    ids::{BodyId, EntityId, RecordId, RegionId, SketchId},
    math::{Vec2, Vec3},
    regen::{CancelToken, Outcome, RegenRequest},
    sketch::{Sketch, SketchEntity, WorldPlane},
};
use uuid::Uuid;

pub(super) fn host(root: &Path) -> HostContext {
    let quiet = Arc::new(super::super::recovery_identity_tests::Quiet);
    HostContext {
        paths: crate::host::HostPaths {
            app_data: Some(root.join("data")),
            app_config: Some(root.join("config")),
        },
        events: quiet.clone(),
        dialogs: quiet,
    }
}

pub(super) fn real_state(host: &HostContext) -> AppState {
    let state = AppState::default();
    state.start(host.clone()).unwrap();
    state
}

fn sketch_operation(sketch_id: SketchId) -> Operation {
    let mut sketch = Sketch::on_world_plane(sketch_id, "Circle", WorldPlane::XY);
    let center = EntityId(Uuid::from_u128(0x5a02));
    sketch
        .add_entity(SketchEntity::point(
            center,
            Vec2::new(0., 0.).unwrap(),
            false,
            false,
        ))
        .unwrap();
    sketch
        .add_entity(
            SketchEntity::circle(EntityId(Uuid::from_u128(0x5a03)), center, 5., false).unwrap(),
        )
        .unwrap();
    let (_, entities, constraints) = crate::worker::wire::sketch_wire(&sketch);
    let plane = SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: Vec3::new_unchecked(0., 0., 0.),
        x_axis: Vec3::new_unchecked(1., 0., 0.),
        y_axis: Vec3::new_unchecked(0., 1., 0.),
        normal: Vec3::new_unchecked(0., 0., 1.),
        extra: Default::default(),
    };
    Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch: sketch_id,
        plane,
        entities: entities.as_array().unwrap().clone(),
        constraints: constraints.as_array().unwrap().clone(),
        host_face: None,
        extra: Default::default(),
    }))
}

fn extrude_operation(sketch_id: SketchId) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch: sketch_id,
            region: RegionId::new(""),
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(3.),
        draft_angle_deg: Scalar::new(0.),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.),
        target_face2: None,
        extra: Default::default(),
    }))
}

pub(super) async fn create_body(state: &AppState) -> (BodyId, onecad_core::ids::SnapshotId) {
    let sketch_id = SketchId(Uuid::from_u128(0x5a01));
    let extrude_id = RecordId(Uuid::from_u128(0x5a05));
    let sketch_op = sketch_operation(sketch_id);
    let extrude = extrude_operation(sketch_id);
    let mut guard = state.runtime.lock().await;
    let rt = guard.as_mut().unwrap();
    for (id, op) in [
        (RecordId(Uuid::from_u128(0x5a04)), sketch_op),
        (extrude_id, extrude),
    ] {
        rt.apply(EditCommand::AddOperation {
            record: OperationRecord::new(id, 0, "Source", op),
            at_cursor: true,
        })
        .unwrap();
    }
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{report:?}"
    );
    (
        BodyId(extrude_id.0),
        onecad_core::ids::SnapshotId(report.snapshot_id),
    )
}

pub(super) fn add_variable(rt: &mut DocumentRuntime, name: &str, value: f64) {
    rt.apply(onecad_core::edit::EditCommand::AddVariable {
        variable: onecad_core::document::variables::Variable {
            id: onecad_core::ids::VariableId::new(),
            name: name.into(),
            value: onecad_core::document::variables::Scalar::new(value),
            unit: Default::default(),
        },
    })
    .unwrap();
}
