//! Imported analytic source parts; placements are test inputs, not kinematic qualification.
mod public_assembly_artifacts;
mod robonine_analytic_support;
use async_trait::async_trait;
use onecad_app::{
    api,
    dto::AssemblyProjectionDto,
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
    state::AppState,
};
use onecad_assembly::{
    math::{Quat, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId, OccurrencePath},
};
use onecad_core::{
    document::{
        component::ComponentDef,
        record::{
            ImportSourceCodec, ImportStepParams, KnownOperation, Operation, OperationRecord,
            TransformBodyParams, IMPORT_HEAL_POLICY_V1,
        },
        variables::Scalar,
        Document,
    },
    edit::{AssemblyEdit, EditCommand},
    ids::{BodyId, DocumentId, RecordId},
    io::{
        container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta},
        imports::{ImportBlob, ImportBlobs, OwnedSource},
    },
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use robonine_analytic_support::write_fixture;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::Path, sync::Arc, time::Duration};
use uuid::Uuid;

const FILES: [&str; 4] = [
    "part-010.xbf",
    "part-020.xbf",
    "part-030.xbf",
    "part-040.xbf",
];
const HASHES: [&str; 4] = [
    "f643cfdf9ec480a23d14888bf896cad30e02bd05be079a0baf11f7fc88bd1992",
    "932cc0452078850d9059ed6ad0716dd3658d35b8117c5598c2f0699e2b14b21c",
    "d9880bc3f2d82b2e9cf49919603ada70af7e3f4c7d9d60caeb3f81a26fc28d8e",
    "2e780afa05c59f10fb3a8c4acf1cc0c2289d45312dedc0172bc7fa83ecbeffc0",
];
const FACES: [u32; 4] = [181, 168, 108, 96];
const SIZES: [usize; 4] = [118968, 154964, 58366, 85170];
const MAP: [usize; 6] = [0, 1, 2, 3, 1, 2];
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
fn def(i: usize) -> ComponentDefId {
    ComponentDefId(Uuid::from_u128(0xC200 + i as u128))
}
fn occ(i: usize) -> OccurrenceId {
    OccurrenceId(Uuid::from_u128(0xC400 + i as u128))
}
fn record(i: usize) -> RecordId {
    RecordId(Uuid::from_u128(0xC300 + i as u128))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn observation() -> serde_json::Value {
    serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-derivatives/robonine/analytic/observations.json"
    ))
    .unwrap()
}
fn params(i: usize, scale: f64) -> ImportStepParams {
    ImportStepParams {
        source_sha256: HASHES[i].into(),
        source_codec: ImportSourceCodec::Xbf,
        source_name: FILES[i].into(),
        heal_policy: IMPORT_HEAL_POLICY_V1.into(),
        unit_scale: Scalar::new(scale),
        brep_format: Some(12),
        provenance_sha256: None,
        extra: Default::default(),
    }
}
fn host(root: &Path) -> HostContext {
    HostContext {
        paths: HostPaths {
            app_data: Some(root.join("data")),
            app_config: Some(root.join("config")),
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    }
}
async fn open(path: &Path, area: &Path) -> (AppState, HostContext, String) {
    let host = host(area);
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
    (state, host, opened.runtime_session)
}
async fn edit(state: &AppState, session: &str, change: AssemblyEdit) -> AssemblyProjectionDto {
    let current = api::get_assembly_projection(state, session).await.unwrap();
    api::apply_assembly_edit_at_generation(state, session, current.persisted_generation, change)
        .await
        .unwrap()
}
async fn mesh(
    state: &AppState,
    session: &str,
    occurrence: usize,
    definition: usize,
) -> Arc<Vec<u8>> {
    let projection = api::get_assembly_projection(state, session).await.unwrap();
    let part = projection
        .definitions
        .iter()
        .find(|part| part.id == def(definition).to_string())
        .unwrap();
    assert_eq!(part.bodies.len(), 1);
    let body: BodyId = part.bodies[0].id.parse().unwrap();
    api::component_mesh_for_path(
        state,
        session,
        &OccurrencePath(vec![occ(occurrence)]),
        body,
        part.mesh_generation.unwrap(),
    )
    .await
    .unwrap()
}
fn check_mesh(bytes: &[u8], index: usize, offset: f64) {
    let mesh = MeshView::decode(bytes, MeshLimits::default()).unwrap();
    assert_eq!(mesh.header().face_count, FACES[index]);
    let mut ids = HashSet::new();
    for i in 0..FACES[index] as usize {
        let id = mesh.face_id(i).unwrap();
        assert!(id.starts_with("el_"), "unpromoted face: {id}");
        assert!(ids.insert(id.to_owned()), "duplicate face: {id}");
    }
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for i in 0..mesh.header().triangle_count as usize {
        for vertex in mesh.triangle(i).unwrap() {
            let p = mesh.position(vertex as usize).unwrap();
            for axis in 0..3 {
                low[axis] = low[axis].min(p[axis] as f64);
                high[axis] = high[axis].max(p[axis] as f64);
            }
        }
    }
    let expected = &observation()["parts"][index]["before"]["bounds_mm"];
    for axis in 0..3 {
        assert!(
            (low[axis]
                - expected[0][axis].as_f64().unwrap()
                - if axis == 0 { offset } else { 0.0 })
            .abs()
                <= 0.15
        );
        assert!(
            (high[axis]
                - expected[1][axis].as_f64().unwrap()
                - if axis == 0 { offset } else { 0.0 })
            .abs()
                <= 0.15
        );
    }
}
async fn translate(
    state: &AppState,
    session: &str,
    definition: usize,
    operation: u128,
    delta: f64,
) {
    let projection = api::get_assembly_projection(state, session).await.unwrap();
    let part = projection
        .definitions
        .iter()
        .find(|part| part.id == def(definition).to_string())
        .unwrap();
    let targets = part
        .bodies
        .iter()
        .map(|body| body.id.parse().unwrap())
        .collect();
    let op = Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
        targets,
        translate: [Scalar::new(delta), Scalar::new(0.0), Scalar::new(0.0)],
        rotate: Default::default(),
        copy: false,
        extra: Default::default(),
    }));
    edit(
        state,
        session,
        AssemblyEdit::EditDefinition {
            id: def(definition),
            command: Box::new(EditCommand::AddOperation {
                record: OperationRecord::new(
                    RecordId(Uuid::from_u128(operation)),
                    0,
                    "Translate shared geometry",
                    op,
                ),
                at_cursor: true,
            }),
        },
    )
    .await;
}
fn stable_projection(projection: &AssemblyProjectionDto) -> serde_json::Value {
    serde_json::json!({ "root": projection.root_definition, "occurrences": projection.occurrences, "definitions": projection.definitions.iter().map(|d| serde_json::json!({"id": d.id, "name": d.name, "bodies": d.bodies})).collect::<Vec<_>>() })
}
fn verify_sources(path: &Path) {
    let loaded = ContainerReader::open(path).unwrap();
    for i in 0..4 {
        let bytes = loaded.read_import_blob(HASHES[i]).unwrap();
        assert_eq!(bytes.len(), SIZES[i]);
        assert_eq!(digest(&bytes), HASHES[i]);
        let definition = loaded.document().definitions.get(&def(i)).unwrap();
        let op = &definition.timeline.records()[0].op;
        let Operation::Known(KnownOperation::ImportStep(p)) = op else {
            panic!("missing source import")
        };
        assert_eq!(p.source_sha256, HASHES[i]);
        assert_eq!(p.source_name, FILES[i]);
        assert_eq!(p.source_codec, ImportSourceCodec::Xbf);
        assert_eq!(p.brep_format, Some(12));
    }
}
async fn replay_all(state: &AppState, session: &str, count: usize) {
    for i in 0..count {
        api::regenerate_component(state, session, def(i))
            .await
            .unwrap();
    }
}
async fn child(path: &Path, area: &Path) {
    verify_sources(path);
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(area.join("expected.json")).unwrap()).unwrap();
    let (state, host, session) = open(path, area).await;
    replay_all(&state, &session, 5).await;
    let projection = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert_eq!(stable_projection(&projection), expected["projection"]);
    for (occurrence, definition) in [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)] {
        let bytes = mesh(&state, &session, occurrence, definition).await;
        assert_eq!(
            digest(&bytes),
            expected["meshes"][definition].as_str().unwrap()
        );
    }
    let original = projection
        .occurrences
        .iter()
        .find(|o| o.id == occ(4).to_string())
        .unwrap()
        .local;
    let changed = RigidTransform::new(
        original.rotation,
        original.translation + Vec3d::new(2.0, 0.0, 0.0),
    );
    edit(
        &state,
        &session,
        AssemblyEdit::SetPose {
            occurrence: occ(4),
            pose: changed,
        },
    )
    .await;
    api::undo(&state, host.clone()).await.unwrap();
    let restored = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert_eq!(stable_projection(&restored), expected["projection"]);
    api::shutdown_host(&state, host, None).await.unwrap();
}
#[tokio::test]
async fn analytic_assembly_shared_services_cold_reopen() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    if let Some(area) = std::env::var_os("ONECAD_ROBONINE_CHILD") {
        let area = Path::new(&area);
        child(&area.join("analytic.onecad"), area).await;
        return;
    }
    let area = tempfile::tempdir().unwrap();
    let path = area.path().join("analytic.onecad");
    write_fixture(&path);
    let (state, host, session) = open(&path, area.path()).await;
    replay_all(&state, &session, 4).await;
    let initial = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert_eq!(initial.occurrences.len(), 6);
    for (i, definition) in MAP.iter().enumerate() {
        let row = initial
            .occurrences
            .iter()
            .find(|row| row.id == occ(i).to_string())
            .unwrap();
        assert_eq!(row.definition, def(*definition).to_string());
        assert_eq!(row.grounded, i == 0);
        assert_eq!(
            row.local,
            if i < 4 {
                RigidTransform::IDENTITY
            } else {
                RigidTransform::new(
                    Quat::from_axis_angle(Vec3d::Z, std::f64::consts::PI).unwrap(),
                    Vec3d::ZERO,
                )
            }
        );
    }
    for i in 0..4 {
        check_mesh(&mesh(&state, &session, i, i).await, i, 0.0);
    }
    let clamp = mesh(&state, &session, 1, 1).await;
    let rack = mesh(&state, &session, 2, 2).await;
    assert!(Arc::ptr_eq(&clamp, &mesh(&state, &session, 4, 1).await));
    assert!(Arc::ptr_eq(&rack, &mesh(&state, &session, 5, 2).await));
    let hidden = edit(
        &state,
        &session,
        AssemblyEdit::SetVisibility {
            occurrence: occ(4),
            visible: false,
        },
    )
    .await;
    assert!(
        !hidden
            .occurrences
            .iter()
            .find(|o| o.id == occ(4).to_string())
            .unwrap()
            .visible
    );
    assert!(
        hidden
            .occurrences
            .iter()
            .find(|o| o.id == occ(1).to_string())
            .unwrap()
            .visible
    );
    edit(
        &state,
        &session,
        AssemblyEdit::SetVisibility {
            occurrence: occ(4),
            visible: true,
        },
    )
    .await;
    let sibling = initial
        .occurrences
        .iter()
        .find(|o| o.id == occ(1).to_string())
        .unwrap()
        .local;
    let turn = Quat::from_axis_angle(Vec3d::Z, std::f64::consts::PI).unwrap();
    let moved = edit(
        &state,
        &session,
        AssemblyEdit::SetPose {
            occurrence: occ(4),
            pose: RigidTransform::new(turn, Vec3d::new(5.0, 0.0, 0.0)),
        },
    )
    .await;
    assert_eq!(
        moved
            .occurrences
            .iter()
            .find(|o| o.id == occ(1).to_string())
            .unwrap()
            .local,
        sibling
    );
    let invalid = AssemblyEdit::SetPose {
        occurrence: occ(4),
        pose: RigidTransform::new(turn, Vec3d::new(f64::NAN, 0.0, 0.0)),
    };
    assert!(api::apply_assembly_edit_at_generation(
        &state,
        &session,
        moved.persisted_generation,
        invalid
    )
    .await
    .is_err());
    assert_eq!(
        serde_json::to_value(&moved).unwrap(),
        serde_json::to_value(
            api::get_assembly_projection(&state, &session)
                .await
                .unwrap()
        )
        .unwrap()
    );
    translate(&state, &session, 1, 0xC501, 3.0).await;
    api::regenerate_component(&state, &session, def(1))
        .await
        .unwrap();
    let scaled = mesh(&state, &session, 1, 1).await;
    check_mesh(&scaled, 1, 3.0);
    assert_ne!(scaled.as_slice(), clamp.as_slice());
    assert!(Arc::ptr_eq(&scaled, &mesh(&state, &session, 4, 1).await));
    api::undo(&state, host.clone()).await.unwrap();
    api::regenerate_component(&state, &session, def(1))
        .await
        .unwrap();
    check_mesh(&mesh(&state, &session, 1, 1).await, 1, 0.0);
    api::redo(&state, host.clone()).await.unwrap();
    api::regenerate_component(&state, &session, def(1))
        .await
        .unwrap();
    check_mesh(&mesh(&state, &session, 1, 1).await, 1, 3.0);
    edit(
        &state,
        &session,
        AssemblyEdit::MakeIndependent {
            occurrence: occ(4),
            new_definition: def(4),
            name: "Independent clamp".into(),
        },
    )
    .await;
    api::regenerate_component(&state, &session, def(4))
        .await
        .unwrap();
    let independent = mesh(&state, &session, 4, 4).await;
    translate(&state, &session, 1, 0xC502, 2.0).await;
    api::regenerate_component(&state, &session, def(1))
        .await
        .unwrap();
    let changed = mesh(&state, &session, 1, 1).await;
    check_mesh(&changed, 1, 5.0);
    assert_ne!(changed.as_slice(), independent.as_slice());
    assert!(Arc::ptr_eq(
        &independent,
        &mesh(&state, &session, 4, 4).await
    ));
    check_mesh(&independent, 1, 3.0);
    let projection = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    let mut meshes = Vec::new();
    for (occurrence, definition) in [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)] {
        meshes.push(digest(
            &mesh(&state, &session, occurrence, definition).await,
        ));
    }
    let expected =
        serde_json::json!({ "projection": stable_projection(&projection), "meshes": meshes });
    std::fs::write(
        area.path().join("expected.json"),
        serde_json::to_vec(&expected).unwrap(),
    )
    .unwrap();
    assert!(
        api::save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    api::shutdown_host(&state, host, None).await.unwrap();
    public_assembly_artifacts::retain(
        &path,
        "robonine-static",
        include_bytes!("robonine_analytic_assembly.rs"),
        "imported source parts; not native feature-history qualification",
    );
    verify_sources(&path);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "analytic_assembly_shared_services_cold_reopen",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ONECAD_ROBONINE_CHILD", area.path())
        .status()
        .unwrap();
    assert!(status.success(), "cold host process failed: {status}");
}
