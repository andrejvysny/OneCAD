use super::*;
use onecad_core::regen::ResolveOutcome;
use onecad_core::{
    document::record::{ImportSourceCodec, ImportStepParams, IMPORT_HEAL_POLICY_V1},
    io::{
        container::{ContainerCaches, ContainerWriter, SaveMeta},
        imports::{ImportBlob, ImportBlobs, OwnedSource},
    },
};
use sha2::{Digest, Sha256};

#[tokio::test]
async fn rebind_counts_only_matching_unchanged_identity_as_bound() {
    let id = ElementId::new("el_persisted");
    let pending = vec![(
        id.clone(),
        ElementEntry::new(BodyId(Uuid::from_u128(1)), ElementKind::Face),
    )];
    for (echo, expected) in [
        (Some(id), (1, 0)),
        (Some(ElementId::new("el_other")), (0, 1)),
        (None, (0, 1)),
    ] {
        let backend: Arc<dyn GeometryEngine> = Arc::new(FakeBackend {
            resolve_outcome: Mutex::new(Some(ResolveOutcome::Unchanged { element_id: echo })),
            bind_fails: true,
            ..FakeBackend::default()
        });
        assert_eq!(
            DocumentRuntime::rebind_entries_with(&backend, SnapshotId(7), &pending)
                .await
                .unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn real_worker_certified_bound_face_is_not_reported_unresolved() {
    let binary = crate::worker::resolve_worker_path();
    if binary.is_none() {
        assert_ne!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
        return;
    }
    let worker = crate::worker::WorkerManager::spawn(
        crate::worker::manager::SupervisorConfig::production(binary.unwrap()),
    );
    assert!(worker.wait_ready(std::time::Duration::from_secs(15)).await);
    let engine: Arc<dyn GeometryEngine> = Arc::new(worker.clone());
    let area = tempfile::tempdir().unwrap();
    let path = area.path().join("certified.onecad");
    write_source_fixture(&path);
    let mut runtime = DocumentRuntime::open(
        &path,
        engine.clone(),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    )
    .unwrap();
    let (snapshot, id, entry) = certified_entry(&mut runtime).await;
    assert_eq!(
        DocumentRuntime::rebind_entries_with(&engine, snapshot, &[(id, entry)])
            .await
            .unwrap(),
        (1, 0)
    );
    worker.shutdown().await;
}

fn source_operation(hash: &str) -> OperationRecord {
    let operation = KnownOperation::ImportStep(ImportStepParams {
        source_sha256: hash.into(),
        source_codec: ImportSourceCodec::Step,
        source_name: "lever.stp".into(),
        heal_policy: IMPORT_HEAL_POLICY_V1.into(),
        unit_scale: Scalar::new(1.0),
        brep_format: None,
        provenance_sha256: None,
        extra: Default::default(),
    });
    OperationRecord::new(
        RecordId::new(),
        0,
        "Imported source",
        Operation::Known(operation),
    )
}

fn write_source_fixture(path: &Path) {
    let bytes =
        include_bytes!("../../../../../fixtures/public-cad/prusa/MINI-fsenzor-lever.stp").to_vec();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let mut document = Document::new(DocumentId::new());
    document.timeline.insert_at_cursor(source_operation(&hash));
    let mut imports = ImportBlobs::new();
    imports.insert(
        hash,
        ImportBlob {
            codec: ImportSourceCodec::Step,
            bytes: Arc::new(OwnedSource::untracked(bytes)),
        },
    );
    ContainerWriter::save_with_imports(
        path,
        &document,
        &ContainerCaches::none(),
        &imports,
        &SaveMeta {
            app_version: "test".into(),
            occt_fingerprint: None,
            created: String::new(),
            modified: String::new(),
        },
    )
    .unwrap();
}

async fn certified_entry(runtime: &mut DocumentRuntime) -> (SnapshotId, ElementId, ElementEntry) {
    let report = runtime
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{report:?}"
    );
    let snapshot = runtime.head_snapshot_id().unwrap();
    let body = runtime.regen.bodies.bodies()[0].id;
    let promoted = runtime
        .promote_selection(snapshot, body, vec![(TopoKey::new("f:1"), None)])
        .await
        .unwrap();
    let id = ElementId::new(promoted[0].element_id.clone());
    let entry = runtime.regen.elements.get(&id).unwrap().clone();
    assert!(entry.import_source_identity.is_some());
    (snapshot, id, entry)
}
