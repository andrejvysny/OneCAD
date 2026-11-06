//! Imported simulation meshes only; placements are source q=0, not physical qualification.
use async_trait::async_trait;
use onecad_app::{
    api,
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
            IMPORT_HEAL_POLICY_V1,
        },
        variables::Scalar,
        Document,
    },
    ids::{DocumentId, RecordId},
    io::{
        container::{ContainerCaches, ContainerWriter, SaveMeta},
        imports::{ImportBlob, ImportBlobs, OwnedSource},
    },
};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc, time::Duration};
use uuid::Uuid;

const HASHES: [&str; 1] = ["df23f34e1e57dc4861fa69f4b893916eb39b793948012a4856019170ca5e5b28"];
const POSE: [f64; 3] = [-7.062, 120.627, -152.066];
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
    ComponentDefId(Uuid::from_u128(0xB200 + i as u128))
}
fn occ(i: usize) -> OccurrenceId {
    OccurrenceId(Uuid::from_u128(0xB400 + i as u128))
}
fn record(i: usize) -> RecordId {
    RecordId(Uuid::from_u128(0xB300 + i as u128))
}
fn q0() -> RigidTransform {
    RigidTransform::new(Quat::IDENTITY, Vec3d::new(POSE[0], POSE[1], POSE[2]))
}
fn host(path: &Path) -> HostContext {
    HostContext {
        paths: HostPaths {
            app_data: Some(path.join("data")),
            app_config: Some(path.join("config")),
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    }
}

fn add_control(document: &mut Document, imports: &mut ImportBlobs) {
    let bytes = include_bytes!("fixtures/colored_boxes.step").to_vec();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let mut part = ComponentDef::new(def(2), "Control boxes");
    let params = ImportStepParams {
        source_sha256: hash.clone(),
        source_codec: ImportSourceCodec::Step,
        source_name: "colored_boxes.step".into(),
        heal_policy: IMPORT_HEAL_POLICY_V1.into(),
        unit_scale: Scalar::new(1.0),
        brep_format: None,
        provenance_sha256: None,
        extra: Default::default(),
    };
    part.timeline.insert_at_cursor(OperationRecord::new(
        record(2),
        0,
        "Control import",
        Operation::Known(KnownOperation::ImportStep(params)),
    ));
    document.definitions.insert(part.id, part);
    imports.insert(
        hash,
        ImportBlob {
            codec: ImportSourceCodec::Step,
            bytes: Arc::new(OwnedSource::untracked(bytes)),
        },
    );
    document
        .assembly
        .occurrences
        .insert(Occurrence {
            id: occ(4),
            def: def(2),
            parent: None,
            local: RigidTransform::IDENTITY,
            grounded: false,
            name: "Control".into(),
        })
        .unwrap();
}

fn write_fixture(path: &Path) {
    let mut document = Document::new(DocumentId(Uuid::from_u128(0xB100)));
    let mut imports = ImportBlobs::new();
    for (i, hash) in HASHES.iter().enumerate() {
        let name = format!("clamp_{}.brep", i + 1);
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/reference-derivatives/robonine")
            .join(&name);
        let bytes = std::fs::read(source).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), *hash);
        let mut part = ComponentDef::new(def(i), &name);
        let params = ImportStepParams {
            source_sha256: (*hash).into(),
            source_codec: ImportSourceCodec::Brep,
            source_name: name,
            heal_policy: IMPORT_HEAL_POLICY_V1.into(),
            unit_scale: Scalar::new(1.0),
            brep_format: Some(4),
            provenance_sha256: None,
            extra: Default::default(),
        };
        part.timeline.insert_at_cursor(OperationRecord::new(
            record(i),
            0,
            "Imported simulation mesh",
            Operation::Known(KnownOperation::ImportStep(params)),
        ));
        document.definitions.insert(part.id, part);
        imports.insert(
            (*hash).into(),
            ImportBlob {
                codec: ImportSourceCodec::Brep,
                bytes: Arc::new(OwnedSource::untracked(bytes)),
            },
        );
    }
    add_control(&mut document, &mut imports);
    for i in 0..2 {
        document
            .assembly
            .occurrences
            .insert(Occurrence {
                id: occ(i),
                def: def(0),
                parent: None,
                local: q0(),
                grounded: false,
                name: format!("clamp 1 instance {}", i + 1),
            })
            .unwrap();
    }
    let meta = SaveMeta {
        app_version: "robonine-static-test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save_with_imports(path, &document, &ContainerCaches::none(), &imports, &meta)
        .unwrap();
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
#[tokio::test]
async fn imported_static_jaw_topology_budget_refusal_preserves_document() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    let area = tempfile::tempdir().unwrap();
    let path = area.path().join("static-jaws.onecad");
    write_fixture(&path);
    let (state, host, session) = open(&path, area.path()).await;
    api::regenerate_component(&state, &session, def(2))
        .await
        .unwrap();
    let before = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert_eq!(before.occurrences.len(), 3);
    let control = before
        .definitions
        .iter()
        .find(|d| d.id == def(2).to_string())
        .unwrap();
    assert!(!control.bodies.is_empty());
    let generation = control.mesh_generation.unwrap();
    let body = control.bodies[0].id.parse().unwrap();
    let path = OccurrencePath(vec![occ(4)]);
    let accepted = api::component_mesh_for_path(&state, &session, &path, body, generation)
        .await
        .unwrap();
    let error = api::regenerate_component(&state, &session, def(0))
        .await
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("component topology budget exceeded"),
        "unexpected replay failure: {error:?}"
    );
    let after = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&after).unwrap()
    );
    let retained = api::component_mesh_for_path(&state, &session, &path, body, generation)
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&accepted, &retained));
    api::shutdown_host(&state, host, None).await.unwrap();
}
