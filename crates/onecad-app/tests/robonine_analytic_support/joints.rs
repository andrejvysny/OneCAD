use super::*;

fn source_element(path: &Path, part: usize, label: &str) -> (BodyId, String) {
    let loaded = ContainerReader::open(path).unwrap();
    let definition = loaded.document().definitions.get(&def(part)).unwrap();
    let matches: Vec<_> = definition
        .elements
        .iter()
        .filter_map(|(id, entry)| {
            let cert = entry.import_source_identity.as_ref()?;
            let ImportSourceAddress::Xbf { face_label, .. } = &cert.source_address else {
                return None;
            };
            (cert.source_sha256 == HASHES[part] && face_label == label)
                .then(|| (entry.body, id.to_string()))
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "source certificate must select exactly one face"
    );
    matches[0].clone()
}
async fn datum(
    state: &AppState,
    session: &str,
    path: &Path,
    part: usize,
    instance: usize,
    label: &str,
    x: Vec3d,
) -> (BodyId, api::JointOriginResolution) {
    let (body, element) = source_element(path, part, label);
    let projection = api::get_assembly_projection(state, session).await.unwrap();
    let item = projection
        .definitions
        .iter()
        .find(|d| d.id == def(part).to_string())
        .unwrap();
    let instance_path = OccurrencePath(vec![occ(instance)]);
    let result = api::component_joint_origin(
        state,
        api::JointOriginRequest {
            session,
            generation: projection.persisted_generation,
            path: &instance_path,
            body,
            mesh_generation: item.mesh_generation.unwrap(),
            element: &element,
            x,
        },
    )
    .await
    .unwrap();
    let cert = result.reference.import_source_identity.as_ref().unwrap();
    assert_eq!(cert.source_sha256, HASHES[part]);
    let ImportSourceAddress::Xbf { face_label, .. } = &cert.source_address else {
        panic!("expected XBF source identity")
    };
    assert_eq!(face_label, label);
    (body, result)
}
fn world_frame(frame: Frame, pose: RigidTransform) -> Frame {
    Frame {
        origin: pose.apply_point(frame.origin),
        z: pose.apply_vector(frame.z),
        x: pose.apply_vector(frame.x),
    }
}
fn joint(
    id: usize,
    kind: JointKind,
    a: usize,
    b: usize,
    af: Frame,
    bf: Frame,
    limits: Option<JointLimits>,
) -> Joint {
    Joint {
        id: JointId(Uuid::from_u128(0xC600 + id as u128)),
        name: format!("test joint {id}"),
        kind,
        a: JointOrigin {
            occurrence: occ(a),
            frame: af,
        },
        b: JointOrigin {
            occurrence: occ(b),
            frame: bf,
        },
        limits,
        value: 0.0,
        suppressed: false,
    }
}
fn bind(
    joint: &Joint,
    instance: usize,
    body: BodyId,
    datum: api::JointOriginResolution,
) -> JointBinding {
    JointBinding {
        joint: joint.id,
        side: JointSide::B,
        path: OccurrencePath(vec![occ(instance)]),
        definition: datum.definition,
        body,
        element: datum.reference,
        surface: datum.surface,
        radius: datum.radius,
        orientation_x: joint.b.frame.x,
        needs_repair: false,
    }
}
fn near_vec(a: Vec3d, b: Vec3d, tolerance: f64) {
    assert!((a - b).norm() <= tolerance, "{a:?} != {b:?}");
}
pub(super) fn pose(projection: &AssemblyProjectionDto, i: usize) -> RigidTransform {
    projection
        .occurrences
        .iter()
        .find(|o| o.id == occ(i).to_string())
        .unwrap()
        .local
}
pub(super) async fn fixtures(
    state: &AppState,
    session: &str,
    path: &Path,
) -> (Vec<Joint>, Vec<JointBinding>) {
    let (body1, clamp1) = datum(state, session, path, 1, 1, "0:1:1:1:9", Vec3d::Z).await;
    let (body2, clamp2) = datum(state, session, path, 1, 4, "0:1:1:1:9", Vec3d::Z).await;
    let (body3, gear) = datum(state, session, path, 3, 3, "0:1:1:1:6", Vec3d::X).await;
    for c in [&clamp1, &clamp2] {
        near_vec(
            c.frame.origin,
            Vec3d::new(24.888015895285545, -16.0, 11.0),
            1e-6,
        );
        near_vec(c.frame.z, Vec3d::X, 1e-8);
        assert!((c.radius.unwrap() - 2.85).abs() < 1e-6);
    }
    near_vec(
        gear.frame.origin,
        Vec3d::new(0.0, 0.0, 6.202235200000838),
        1e-6,
    );
    near_vec(gear.frame.z, -Vec3d::Z, 1e-8);
    assert!((gear.radius.unwrap() - 3.0).abs() < 1e-6);
    let projection = api::get_assembly_projection(state, session).await.unwrap();
    let a1 = world_frame(clamp1.frame, pose(&projection, 1));
    let a2 = world_frame(clamp2.frame, pose(&projection, 4));
    let ag = world_frame(gear.frame, pose(&projection, 3));
    near_vec(a1.z, Vec3d::X, 1e-8);
    near_vec(a2.z, -Vec3d::X, 1e-8);
    let range = Some(JointLimits {
        min: -42.0,
        max: 0.0,
    });
    let s1 = joint(1, JointKind::Slider, 0, 1, a1, clamp1.frame, range);
    let s2 = joint(2, JointKind::Slider, 0, 4, a2, clamp2.frame, range);
    let r = joint(
        3,
        JointKind::Revolute,
        0,
        3,
        ag,
        gear.frame,
        Some(JointLimits {
            min: -std::f64::consts::PI,
            max: std::f64::consts::PI,
        }),
    );
    let common = Frame {
        origin: Vec3d::ZERO,
        z: Vec3d::Z,
        x: Vec3d::X,
    };
    let r1 = joint(4, JointKind::Rigid, 1, 2, common, common, None);
    let r2 = joint(5, JointKind::Rigid, 4, 5, common, common, None);
    let bindings = vec![
        bind(&s1, 1, body1, clamp1),
        bind(&s2, 4, body2, clamp2),
        bind(&r, 3, body3, gear),
    ];
    (vec![s1, s2, r, r1, r2], bindings)
}
