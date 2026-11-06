//! Imported Prusa source geometry in an explicitly authored test assembly.
//! The placements and drive are test inputs, not measured physical assembly data.
use async_trait::async_trait;
use onecad_app::{
    api,
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
    joint_execution,
    state::AppState,
};
use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointOrigin},
    math::{Frame, Quat, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId, OccurrencePath},
};
use onecad_core::{
    document::{
        component::{ComponentDef, JointBinding, JointSide},
        record::{
            ImportSourceCodec, ImportStepParams, KnownOperation, Operation, OperationRecord,
            IMPORT_HEAL_POLICY_V1,
        },
        variables::Scalar,
        Document,
    },
    edit::{AssemblyEdit, EditCommand},
    ids::{BodyId, DocumentId, RecordId},
    io::{
        container::{ContainerCaches, ContainerWriter, SaveMeta},
        imports::{ImportBlob, ImportBlobs, OwnedSource},
    },
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use uuid::Uuid;

struct Quiet;
impl EventSink for Quiet {
    fn emit(&self, _: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait]
impl DialogPort for Quiet {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn definition(n: u128) -> ComponentDefId {
    ComponentDefId(id(n))
}
fn occurrence(n: u128) -> OccurrenceId {
    OccurrenceId(id(n))
}
fn record(n: u128) -> RecordId {
    RecordId(id(n))
}

fn oracle() -> serde_json::Value {
    serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap()
}

fn source_document(root: &Path, path: &Path) {
    let oracle = oracle();
    let mut document = Document::new(DocumentId(id(0xA100)));
    let mut imports = ImportBlobs::new();
    for (index, name) in ["box", "cover", "lever"].into_iter().enumerate() {
        let source = root.join(format!("MINI-fsenzor-{name}.stp"));
        let bytes = std::fs::read(&source).unwrap();
        let sha = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(sha, oracle["parts"][name]["sha256"].as_str().unwrap());
        let mut part = ComponentDef::new(definition(0xA200 + index as u128), name);
        let parameters = ImportStepParams {
            source_sha256: sha.clone(),
            source_codec: ImportSourceCodec::Step,
            source_name: format!("MINI-fsenzor-{name}.stp"),
            heal_policy: IMPORT_HEAL_POLICY_V1.into(),
            unit_scale: Scalar::new(1.0),
            brep_format: None,
            provenance_sha256: None,
            extra: Default::default(),
        };
        part.timeline.insert_at_cursor(OperationRecord::new(
            record(0xA300 + index as u128),
            0,
            format!("Imported {name}"),
            Operation::Known(KnownOperation::ImportStep(parameters)),
        ));
        document.definitions.insert(part.id, part);
        imports.insert(
            sha,
            ImportBlob {
                codec: ImportSourceCodec::Step,
                bytes: Arc::new(OwnedSource::untracked(bytes)),
            },
        );
    }
    let turn = Quat::from_axis_angle(Vec3d::X, std::f64::consts::FRAC_PI_2).unwrap();
    let box_axis = vector(&oracle["parts"]["box"]["candidate"]["originMm"]);
    let lever_axis = vector(&oracle["parts"]["lever"]["candidate"]["originMm"]);
    let placements = [
        RigidTransform::IDENTITY,
        RigidTransform::new(turn, Vec3d::ZERO),
        RigidTransform::new(turn, box_axis - turn.rotate(lever_axis)),
    ];
    for (index, local) in placements.into_iter().enumerate() {
        document
            .assembly
            .occurrences
            .insert(Occurrence {
                id: occurrence(0xA400 + index as u128),
                def: definition(0xA200 + index as u128),
                parent: None,
                local,
                grounded: index == 0,
                name: ["Box", "Cover", "Lever"][index].into(),
            })
            .unwrap();
    }
    let meta = SaveMeta {
        app_version: "prusa-test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save_with_imports(path, &document, &ContainerCaches::none(), &imports, &meta)
        .unwrap();
}

fn vector(value: &serde_json::Value) -> Vec3d {
    Vec3d::new(
        value[0].as_f64().unwrap(),
        value[1].as_f64().unwrap(),
        value[2].as_f64().unwrap(),
    )
}

fn near(actual: f64, expected: f64, tolerance: f64, label: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: actual={actual}, expected={expected}, tolerance={tolerance}"
    );
}

fn mesh_properties(mesh: &MeshView<'_>, expected: &serde_json::Value) {
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    let mut six_volume = 0.0;
    for index in 0..mesh.header().triangle_count as usize {
        let triangle = mesh.triangle(index).unwrap();
        let points = triangle.map(|vertex| {
            let value = mesh.position(vertex as usize).unwrap();
            [value[0] as f64, value[1] as f64, value[2] as f64]
        });
        for point in points {
            for axis in 0..3 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
        let [a, b, c] = points;
        six_volume += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    let volume = six_volume.abs() / 6.0;
    let target = expected["volumeMm3"].as_f64().unwrap();
    near(
        volume,
        target,
        target * 0.02,
        "tessellated volume vs source reader",
    );
    for axis in 0..3 {
        near(
            low[axis],
            expected["boundsMm"][0][axis].as_f64().unwrap(),
            0.15,
            "mesh minimum",
        );
        near(
            high[axis],
            expected["boundsMm"][1][axis].as_f64().unwrap(),
            0.15,
            "mesh maximum",
        );
    }
}

fn face_bounds(mesh: &MeshView<'_>, face: usize) -> ([f64; 3], [f64; 3]) {
    let span = mesh.face_range(face).unwrap();
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for triangle in span.first..span.first + span.count {
        for vertex in mesh.triangle(triangle as usize).unwrap() {
            let point = mesh.position(vertex as usize).unwrap();
            for axis in 0..3 {
                low[axis] = low[axis].min(point[axis] as f64);
                high[axis] = high[axis].max(point[axis] as f64);
            }
        }
    }
    (low, high)
}

fn candidate_id(mesh: &MeshView<'_>, expected: &serde_json::Value) -> String {
    let mut candidates = Vec::new();
    for face in 0..mesh.header().face_count as usize {
        let (low, high) = face_bounds(mesh, face);
        if (0..3).all(|axis| {
            (low[axis] - expected[0][axis].as_f64().unwrap()).abs() < 0.15
                && (high[axis] - expected[1][axis].as_f64().unwrap()).abs() < 0.15
        }) {
            candidates.push(mesh.face_id(face).unwrap().to_owned());
        }
    }
    assert_eq!(
        candidates.len(),
        1,
        "source face bounds must select exactly one mesh face"
    );
    assert!(candidates[0].starts_with("el_"));
    candidates.remove(0)
}

async fn resolved_face(
    state: &AppState,
    session: &str,
    projection: &onecad_app::dto::AssemblyProjectionDto,
    part: &str,
    def: ComponentDefId,
    instance: OccurrenceId,
) -> (BodyId, onecad_app::api::JointOriginResolution) {
    let source = oracle();
    let expected = &source["parts"][part];
    let item = projection
        .definitions
        .iter()
        .find(|d| d.id == def.to_string())
        .unwrap();
    assert_eq!(item.bodies.len(), 1, "{part}: imported source solid count");
    let body: BodyId = item.bodies[0].id.parse().unwrap();
    let generation = item.mesh_generation.unwrap();
    let path = OccurrencePath(vec![instance]);
    let bytes = api::component_mesh_for_path(state, session, &path, body, generation)
        .await
        .unwrap();
    let mesh = MeshView::decode(&bytes, MeshLimits::default()).unwrap();
    mesh_properties(&mesh, expected);
    let element = candidate_id(&mesh, &expected["candidate"]["faceBoundsMm"]);
    let datum = api::component_joint_origin(
        state,
        api::JointOriginRequest {
            session,
            generation: projection.persisted_generation,
            path: &path,
            body,
            mesh_generation: generation,
            element: &element,
            x: Vec3d::X,
        },
    )
    .await
    .unwrap();
    near(
        datum.radius.unwrap(),
        expected["candidate"]["radiusMm"].as_f64().unwrap(),
        1e-6,
        "source cylinder radius",
    );
    assert!((datum.frame.origin - vector(&expected["candidate"]["originMm"])).norm() < 1e-5);
    assert!((datum.frame.z - vector(&expected["candidate"]["axis"])).norm() < 1e-8);
    (body, datum)
}

fn pose(projection: &onecad_app::dto::AssemblyProjectionDto, id: OccurrenceId) -> RigidTransform {
    projection
        .occurrences
        .iter()
        .find(|item| item.id == id.to_string())
        .unwrap()
        .local
}

fn aligned(projection: &onecad_app::dto::AssemblyProjectionDto, joint: &Joint, angle: f64) {
    let a = pose(projection, joint.a.occurrence).compose(&joint.a.frame.to_transform().unwrap());
    let b = pose(projection, joint.b.occurrence).compose(&joint.b.frame.to_transform().unwrap());
    assert!(
        (a.translation - b.translation).norm() < 1e-6,
        "joint origins differ"
    );
    let za = a.apply_vector(Vec3d::Z);
    let zb = b.apply_vector(Vec3d::Z);
    assert!((za - zb).norm() < 1e-6, "joint axes differ");
    let xa = a.apply_vector(Vec3d::X);
    let xb = b.apply_vector(Vec3d::X);
    near(
        za.dot(xa.cross(xb)).atan2(xa.dot(xb)),
        angle,
        1e-6,
        "authored drive angle",
    );
}

#[tokio::test]
async fn prusa_imported_sensor_candidate_hinge_survives_drive_and_reopen() {
    let _log = onecad_app::logging::init();
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/public-cad/prusa");
    let area = tempfile::tempdir().unwrap();
    let path = area.path().join("prusa-test-assembly.onecad");
    source_document(&root, &path);
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(area.path().join("data")),
            app_config: Some(area.path().join("config")),
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    };
    let state = AppState::default();
    state.start(host.clone()).unwrap();
    let opened = api::open_assembly_document(
        &state,
        host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(state.readiness().wait_ready(Duration::from_secs(15)).await);
    let session = opened.runtime_session;
    for index in 0..3 {
        let name = ["box", "cover", "lever"][index as usize];
        eprintln!("replaying Prusa {name}");
        api::regenerate_component(&state, &session, definition(0xA200 + index))
            .await
            .unwrap_or_else(|error| panic!("Prusa {name} replay failed: {error:?}"));
    }
    let projection = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    let (box_body, box_datum) = resolved_face(
        &state,
        &session,
        &projection,
        "box",
        definition(0xA200),
        occurrence(0xA400),
    )
    .await;
    let (lever_body, lever_datum) = resolved_face(
        &state,
        &session,
        &projection,
        "lever",
        definition(0xA202),
        occurrence(0xA402),
    )
    .await;
    let cover = projection
        .definitions
        .iter()
        .find(|d| d.id == definition(0xA201).to_string())
        .unwrap();
    assert_eq!(cover.bodies.len(), 1);
    let cover_body: BodyId = cover.bodies[0].id.parse().unwrap();
    let cover_mesh = api::component_mesh_for_path(
        &state,
        &session,
        &OccurrencePath(vec![occurrence(0xA401)]),
        cover_body,
        cover.mesh_generation.unwrap(),
    )
    .await
    .unwrap();
    mesh_properties(
        &MeshView::decode(&cover_mesh, MeshLimits::default()).unwrap(),
        &oracle()["parts"]["cover"],
    );
    let cover_pose = pose(&projection, occurrence(0xA401));
    let cover_local = cover_pose
        .inverse()
        .compose(&box_datum.frame.to_transform().unwrap());
    let cover_frame = Frame {
        origin: cover_local.translation,
        z: cover_local.apply_vector(Vec3d::Z),
        x: cover_local.apply_vector(Vec3d::X),
    };
    let rigid = Joint {
        id: JointId(id(0xA501)),
        name: "Authored cover fixture".into(),
        kind: JointKind::Rigid,
        a: JointOrigin {
            occurrence: occurrence(0xA400),
            frame: box_datum.frame,
        },
        b: JointOrigin {
            occurrence: occurrence(0xA401),
            frame: cover_frame,
        },
        limits: None,
        value: 0.0,
        suppressed: false,
    };
    let hinge = Joint {
        id: JointId(id(0xA502)),
        name: "Candidate lever hinge".into(),
        kind: JointKind::Revolute,
        a: JointOrigin {
            occurrence: occurrence(0xA400),
            frame: box_datum.frame,
        },
        b: JointOrigin {
            occurrence: occurrence(0xA402),
            frame: lever_datum.frame,
        },
        limits: None,
        value: 0.0,
        suppressed: false,
    };
    let bindings = vec![
        JointBinding {
            joint: hinge.id,
            side: JointSide::A,
            path: OccurrencePath(vec![occurrence(0xA400)]),
            definition: box_datum.definition,
            body: box_body,
            element: box_datum.reference,
            surface: box_datum.surface,
            radius: box_datum.radius,
            orientation_x: hinge.a.frame.x,
            needs_repair: false,
        },
        JointBinding {
            joint: hinge.id,
            side: JointSide::B,
            path: OccurrencePath(vec![occurrence(0xA402)]),
            definition: lever_datum.definition,
            body: lever_body,
            element: lever_datum.reference,
            surface: lever_datum.surface,
            radius: lever_datum.radius,
            orientation_x: hinge.b.frame.x,
            needs_repair: false,
        },
    ];
    let joints = vec![rigid, hinge.clone()];
    let report = joint_execution::solve_bound_joints_at_generation_cancellable(
        &state,
        &session,
        projection.persisted_generation,
        joints.clone(),
        vec![],
        bindings.clone(),
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    assert_eq!(report.remaining_dof, 1);
    let rest = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    aligned(&rest, &hinge, 0.0);
    let report = joint_execution::solve_bound_joints_at_generation_cancellable(
        &state,
        &session,
        rest.persisted_generation,
        joints,
        vec![(hinge.id, 0.1)],
        bindings,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    assert_eq!(report.remaining_dof, 0);
    let driven = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    aligned(&driven, &hinge, 0.1);
    assert_eq!(pose(&driven, occurrence(0xA400)), RigidTransform::IDENTITY);
    api::undo(&state, host.clone()).await.unwrap();
    aligned(
        &api::get_assembly_projection(&state, &session)
            .await
            .unwrap(),
        &hinge,
        0.0,
    );
    api::redo(&state, host.clone()).await.unwrap();
    aligned(
        &api::get_assembly_projection(&state, &session)
            .await
            .unwrap(),
        &hinge,
        0.1,
    );
    assert!(
        api::save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    api::close_document(&state, host.clone()).await.unwrap();
    let reopened = api::open_assembly_document(
        &state,
        host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    for index in 0..3 {
        api::regenerate_component(
            &state,
            &reopened.runtime_session,
            definition(0xA200 + index),
        )
        .await
        .unwrap();
    }
    let restored = api::get_assembly_projection(&state, &reopened.runtime_session)
        .await
        .unwrap();
    assert_eq!(restored.joint_bindings.len(), 2);
    assert!(restored
        .joint_bindings
        .iter()
        .all(|binding| binding.verified && !binding.needs_repair));
    aligned(&restored, &hinge, 0.1);
    let changed = api::apply_assembly_edit_at_generation(
        &state,
        &reopened.runtime_session,
        restored.persisted_generation,
        AssemblyEdit::EditDefinition {
            id: definition(0xA202),
            command: Box::new(EditCommand::SetOperationSuppression {
                record: record(0xA302),
                suppressed: true,
                cascade: false,
            }),
        },
    )
    .await
    .unwrap();
    assert!(changed
        .joint_bindings
        .iter()
        .any(|binding| binding.side == JointSide::B && binding.needs_repair));
    assert_eq!(
        pose(&changed, occurrence(0xA402)),
        pose(&restored, occurrence(0xA402))
    );
    api::shutdown_host(&state, host, None).await.unwrap();
}
