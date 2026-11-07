//! Bound pivot frames; bounds are authored test inputs, not physical travel.
use crate::{document, source_checks};
use onecad_app::{api, dto::AssemblyProjectionDto, joint_execution, state::AppState};
use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointLimits, JointOrigin},
    math::{Frame, RigidTransform, Vec3d},
    occurrence::{OccurrenceId, OccurrencePath},
};
use onecad_core::{
    document::{
        component::{JointBinding, JointSide},
        import_source_identity::ImportSourceAddress,
    },
    edit::AssemblyEdit,
    ids::BodyId,
};
use std::{
    f64::consts::PI,
    sync::{atomic::AtomicBool, Arc},
};
use uuid::Uuid;

pub fn hinge_id() -> JointId {
    JointId(Uuid::from_u128(0xA502))
}

pub fn stable(p: &AssemblyProjectionDto) -> serde_json::Value {
    serde_json::json!({"root":p.root_definition,"occurrences":p.occurrences,"joints":p.joints,"bindings":p.joint_bindings})
}

pub fn pose(p: &AssemblyProjectionDto, id: OccurrenceId) -> RigidTransform {
    p.occurrences
        .iter()
        .find(|o| o.id == id.to_string())
        .unwrap()
        .local
}

pub fn assert_pose_near(a: RigidTransform, b: RigidTransform) {
    const SOLVE_TOLERANCE: f64 = 1e-9;
    assert!((a.translation - b.translation).norm() < SOLVE_TOLERANCE);
    for axis in [Vec3d::X, Vec3d::Y, Vec3d::Z] {
        assert!((a.apply_vector(axis) - b.apply_vector(axis)).norm() < SOLVE_TOLERANCE);
    }
}

pub fn aligned(p: &AssemblyProjectionDto, joint: &Joint, angle: f64) {
    let a = pose(p, joint.a.occurrence).compose(&joint.a.frame.to_transform().unwrap());
    let b = pose(p, joint.b.occurrence).compose(&joint.b.frame.to_transform().unwrap());
    assert!((a.translation - b.translation).norm() < 1e-6);
    let (za, zb) = (a.apply_vector(Vec3d::Z), b.apply_vector(Vec3d::Z));
    assert!((za - zb).norm() < 1e-6);
    let (xa, xb) = (a.apply_vector(Vec3d::X), b.apply_vector(Vec3d::X));
    assert!((za.dot(xa.cross(xb)).atan2(xa.dot(xb)) - angle).abs() < 1e-6);
}

fn binding(
    joint: &Joint,
    side: JointSide,
    index: u128,
    body: BodyId,
    datum: api::JointOriginResolution,
) -> JointBinding {
    let x = match side {
        JointSide::A => joint.a.frame.x,
        JointSide::B => joint.b.frame.x,
    };
    JointBinding {
        joint: joint.id,
        side,
        path: OccurrencePath(vec![document::occurrence(index)]),
        definition: datum.definition,
        body,
        element: datum.reference,
        surface: datum.surface,
        radius: datum.radius,
        orientation_x: x,
        needs_repair: false,
    }
}

pub async fn fixtures(state: &AppState, session: &str) -> (Vec<Joint>, Vec<JointBinding>) {
    let p = api::get_assembly_projection(state, session).await.unwrap();
    assert_eq!(p.occurrences.len(), 3);
    let (_box_body, box_datum) = source_checks::resolved_face(
        state,
        session,
        &p,
        "box",
        document::definition(0),
        document::occurrence(0),
    )
    .await;
    let (_lever_body, lever_datum) = source_checks::resolved_face(
        state,
        session,
        &p,
        "lever",
        document::definition(2),
        document::occurrence(2),
    )
    .await;
    let certificate = box_datum
        .reference
        .import_source_identity
        .as_ref()
        .expect("box source certificate");
    assert_eq!(
        certificate.source_sha256,
        "c8b06ef5271166c367d27be13e9c8f93a7ae07707033e4d23bd1ed5997285cb6"
    );
    assert!(matches!(
        &certificate.source_address,
        ImportSourceAddress::Step {
            face_entity_label: 3042,
            ..
        }
    ));
    assert!(
        lever_datum.reference.import_source_identity.is_none(),
        "native lever must not masquerade as an import"
    );
    let lever_pose = pose(&p, document::occurrence(2));
    eprintln!(
        "Prusa pivot: box={:?}, lever_local={:?}, lever_world_axis={:?}, lever_pose={:?}",
        box_datum.frame,
        lever_datum.frame,
        lever_pose.apply_vector(lever_datum.frame.z),
        lever_pose
    );
    eprintln!(
        "Prusa world pivot: lever_origin={:?}, lever_x={:?}, z_dot={}, x_dot={}",
        lever_pose.apply_point(lever_datum.frame.origin),
        lever_pose.apply_vector(lever_datum.frame.x),
        box_datum
            .frame
            .z
            .dot(lever_pose.apply_vector(lever_datum.frame.z)),
        box_datum
            .frame
            .x
            .dot(lever_pose.apply_vector(lever_datum.frame.x))
    );
    source_checks::assert_axis_line(
        lever_pose.apply_point(lever_datum.frame.origin),
        lever_pose.apply_vector(lever_datum.frame.z),
        box_datum.frame.origin,
        box_datum.frame.z,
    );
    // A cylinder frame's axial origin and direction are parametrization choices.
    // Author a logical zero pose from the resolved frames before invoking the solver.
    let box_world =
        pose(&p, document::occurrence(0)).compose(&box_datum.frame.to_transform().unwrap());
    let authored = box_world.compose(&lever_datum.frame.to_transform().unwrap().inverse());
    let changed = api::apply_assembly_edit_at_generation(
        state,
        session,
        p.persisted_generation,
        AssemblyEdit::SetPose {
            occurrence: document::occurrence(2),
            pose: authored,
        },
    )
    .await
    .unwrap();
    assert!(changed.persisted_generation > p.persisted_generation);
    for i in 0..3 {
        api::regenerate_component(state, session, document::definition(i))
            .await
            .unwrap();
    }
    let p = api::get_assembly_projection(state, session).await.unwrap();
    let (box_body, box_datum) = source_checks::resolved_face(
        state,
        session,
        &p,
        "box",
        document::definition(0),
        document::occurrence(0),
    )
    .await;
    let (lever_body, lever_datum) = source_checks::resolved_face(
        state,
        session,
        &p,
        "lever",
        document::definition(2),
        document::occurrence(2),
    )
    .await;
    assert!(matches!(
        box_datum
            .reference
            .import_source_identity
            .as_ref()
            .map(|c| &c.source_address),
        Some(ImportSourceAddress::Step {
            face_entity_label: 3042,
            ..
        })
    ));
    assert!(lever_datum.reference.import_source_identity.is_none());
    let a = pose(&p, document::occurrence(0)).compose(&box_datum.frame.to_transform().unwrap());
    let b = pose(&p, document::occurrence(2)).compose(&lever_datum.frame.to_transform().unwrap());
    assert_pose_near(a, b);
    eprintln!(
        "Prusa authored zero pose: {authored:?}; refreshed generation={}",
        p.persisted_generation
    );
    let cover_pose = pose(&p, document::occurrence(1));
    let local = cover_pose
        .inverse()
        .compose(&box_datum.frame.to_transform().unwrap());
    let cover_frame = Frame {
        origin: local.translation,
        z: local.apply_vector(Vec3d::Z),
        x: local.apply_vector(Vec3d::X),
    };
    let rigid = Joint {
        id: JointId(Uuid::from_u128(0xA501)),
        name: "Authored rigid cover".into(),
        kind: JointKind::Rigid,
        a: JointOrigin {
            occurrence: document::occurrence(0),
            frame: box_datum.frame,
        },
        b: JointOrigin {
            occurrence: document::occurrence(1),
            frame: cover_frame,
        },
        limits: None,
        value: 0.,
        suppressed: false,
    };
    let hinge = Joint {
        id: hinge_id(),
        name: "Source-axis candidate hinge".into(),
        kind: JointKind::Revolute,
        a: JointOrigin {
            occurrence: document::occurrence(0),
            frame: box_datum.frame,
        },
        b: JointOrigin {
            occurrence: document::occurrence(2),
            frame: lever_datum.frame,
        },
        limits: Some(JointLimits {
            min: -PI / 6.,
            max: PI / 6.,
        }),
        value: 0.,
        suppressed: false,
    };
    let bindings = vec![
        binding(&hinge, JointSide::A, 0, box_body, box_datum),
        binding(&hinge, JointSide::B, 2, lever_body, lever_datum),
    ];
    (vec![rigid, hinge], bindings)
}

pub async fn solve(
    state: &AppState,
    session: &str,
    joints: Vec<Joint>,
    bindings: Vec<JointBinding>,
    drives: Vec<(JointId, f64)>,
) -> Result<joint_execution::JointSolveReport, onecad_app::error::ApiError> {
    let p = api::get_assembly_projection(state, session).await.unwrap();
    eprintln!(
        "Prusa solve input: generation={}, joints={joints:?}, drives={drives:?}",
        p.persisted_generation
    );
    joint_execution::solve_bound_joints_at_generation_cancellable(
        state,
        session,
        p.persisted_generation,
        joints,
        drives,
        bindings,
        Arc::new(AtomicBool::new(false)),
    )
    .await
}

pub async fn drive(
    state: &AppState,
    session: &str,
    angle: f64,
) -> Result<joint_execution::JointSolveReport, onecad_app::error::ApiError> {
    let p = api::get_assembly_projection(state, session).await.unwrap();
    let bindings = api::get_joint_bindings(state, session, p.persisted_generation)
        .await
        .unwrap();
    solve(
        state,
        session,
        p.joints,
        bindings,
        vec![(hinge_id(), angle)],
    )
    .await
}
