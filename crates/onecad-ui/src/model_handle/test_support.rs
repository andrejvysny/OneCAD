use super::*;
use onecad_core::{
    document::{
        record::*,
        refs::{ElementKind, ElementRef, PrimaryRef},
        variables::Scalar,
    },
    ids::{BodyId, ElementId, SnapshotId},
    math::Vec3,
};
use onecad_interaction::{DocumentContext, Interaction, NumericParameter as N, modeling_records};
use onecad_scene::{
    Camera, DVec3, Projection, SceneBody, SceneIdentity, SceneLimits, SceneSnapshot, Viewport,
};
use std::sync::Arc;

pub(super) fn context() -> DocumentContext {
    DocumentContext {
        document_id: "handles".into(),
        runtime_session: "handles-runtime".into(),
        revision: 1,
        snapshot_id: SnapshotId(1),
        scene: SceneIdentity {
            session: 1,
            scene: 1,
        },
    }
}
pub(super) fn view() -> ViewSnapshot {
    ViewSnapshot::new(
        context().scene,
        1,
        Camera {
            offset: DVec3::new(70.0, -110.0, 90.0),
            projection: Projection::Perspective,
            ..Default::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        },
    )
    .unwrap()
}
pub(super) fn rect() -> Rect {
    Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 600.0))
}
pub(super) fn scene() -> SceneSnapshot {
    let bytes = Arc::from(
        include_bytes!(
            "../../../../fixtures/render/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
        )
        .as_slice(),
    );
    let body = SceneBody::new(BodyId::new(), 1, bytes, Default::default()).unwrap();
    SceneSnapshot::new(context().scene, vec![body], SceneLimits::default()).unwrap()
}
fn reference(body: BodyId, kind: ElementKind, id: &str) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            kind,
            element: ElementId::new(id),
            extra: Default::default(),
        }),
        intent: None,
        anchor: None,
        extra: Default::default(),
    }
}
fn record(op: KnownOperation) -> OperationRecord {
    OperationRecord::new(RecordId::new(), 0, "Handle", Operation::Known(op))
}
pub(super) fn records(scene: &SceneSnapshot) -> Vec<(OperationRecord, Vec<N>)> {
    let body = &scene.bodies()[0];
    let id = body.body_id();
    let mesh = body.mesh().unwrap();
    let face = reference(id, ElementKind::Face, mesh.face_id(0).unwrap());
    let edge_i = (0..mesh.header().edge_count as usize)
        .find(|&i| {
            let r = mesh.edge_range(i).unwrap();
            r.count == 2
        })
        .unwrap();
    let edge = reference(id, ElementKind::Edge, mesh.edge_id(edge_i).unwrap());
    let edge_id = edge.primary.as_ref().unwrap().element.clone();
    let hole = record(KnownOperation::Hole(HoleParams {
        target_body: id,
        face: face.clone(),
        point: Vec3::new_unchecked(0.0, 0.0, 0.0),
        points: None,
        hole_type: HoleType::Simple,
        diameter: Scalar::new(4.0),
        depth: Some(Scalar::new(12.0)),
        cb_diameter: None,
        cb_depth: None,
        cs_diameter: None,
        cs_angle_deg: None,
        thread: None,
        result_policy_version: Some(2),
        extra: Default::default(),
    }));
    vec![
        (
            record(KnownOperation::Fillet(FilletParams {
                radius: Scalar::new(2.0),
                edge_ids: vec![edge_id.clone()],
                edges: vec![edge.clone()],
                chain_tangent_edges: false,
                tangent_closure_version: None,
                extra: Default::default(),
            })),
            vec![N::FilletRadius],
        ),
        (
            record(KnownOperation::Chamfer(ChamferParams {
                radius: Scalar::new(2.0),
                distance2: None,
                angle_deg: None,
                edge_ids: vec![edge_id],
                edges: vec![edge],
                reference_faces: vec![],
                reference_face_refs: vec![],
                chain_tangent_edges: false,
                tangent_closure_version: None,
                extra: Default::default(),
            })),
            vec![N::ChamferDistance],
        ),
        (
            record(KnownOperation::Shell(ShellParams {
                thickness: Scalar::new(1.0),
                open_faces: vec![face.primary.as_ref().unwrap().element.clone()],
                faces: vec![face.clone()],
                target_body: Some(id),
                extra: Default::default(),
            })),
            vec![N::ShellThickness],
        ),
        (hole, vec![N::HoleDiameter, N::HoleDepth]),
        (
            record(KnownOperation::OffsetFace(OffsetFaceParams {
                face_ids: vec![face.primary.as_ref().unwrap().element.clone()],
                primary_face_ids: vec![],
                faces: vec![face],
                distance: Scalar::new(3.0),
                distance_type: OffsetDistanceType::Offset,
                chain_tangent_faces: false,
                opposite_face_id: None,
                opposite_face: None,
                target_body: id,
                result_policy_version: None,
                extra: Default::default(),
            })),
            vec![N::OffsetFaceDistance],
        ),
        (
            modeling_records::linear_pattern(id, 10.0, RecordId::new(), 0),
            vec![N::LinearPatternSpacing],
        ),
        (
            modeling_records::circular_pattern(id, RecordId::new(), 0),
            vec![N::CircularPatternAngle],
        ),
        (
            modeling_records::gear(RecordId::new(), 0),
            vec![N::GearHeight, N::GearModule],
        ),
        (modeling_records::mirror(id, RecordId::new(), 0), vec![]),
    ]
}
pub(super) fn begin(record: OperationRecord) -> Interaction {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let fields = onecad_interaction::declared_keys(&record.op);
    // Existing-edit setup admits historical records without imposing new authoring staging.
    state.dispatch(Action::BeginOperationEdit {
        record: Box::new(record),
        gesture: 1,
        context: context(),
    });
    assert!(state.tool.is_some(), "{:?}", state.status);
    if let Some(key) = fields.first() {
        let tool = state.tool.as_ref().unwrap();
        let value = onecad_interaction::numeric_parameters(&tool.record.op)
            .iter()
            .find(|d| d.parameter == key.parameter())
            .unwrap()
            .scalar
            .value;
        state.dispatch(Action::Evaluated {
            tag: tool.tag.clone(),
            result: Ok(onecad_interaction::EvaluatedExpressionDto {
                value,
                dimension: key.site().into(),
                error: None,
            }),
        });
    }
    state
}
pub(super) fn evaluate(state: &mut Interaction, value: f64) {
    let tool = state.tool.as_ref().unwrap();
    let key = tool
        .params
        .fields()
        .iter()
        .find(|f| matches!(f.status, onecad_interaction::FieldStatus::Pending))
        .unwrap()
        .key;
    state.dispatch(Action::Evaluated {
        tag: tool.tag.clone(),
        result: Ok(onecad_interaction::EvaluatedExpressionDto {
            value,
            dimension: key.site().into(),
            error: None,
        }),
    });
}
