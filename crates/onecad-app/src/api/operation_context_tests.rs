use super::*;
use onecad_core::document::record::OperationRecord;

#[test]
fn mesh_handoff_rejects_before_clone_and_reuses_admitted_allocation() {
    let mesh = std::sync::Arc::new(vec![7; 1025]);
    assert!(bounded_mesh(Some(mesh.clone()), 1024).is_err());
    assert_eq!(std::sync::Arc::strong_count(&mesh), 1);
    let admitted = bounded_mesh(Some(mesh.clone()), 1025).unwrap().unwrap();
    assert!(std::sync::Arc::ptr_eq(&mesh, &admitted));
}

#[tokio::test]
async fn prepared_apply_refuses_same_uuid_replacement_at_mutation_lock() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) =
        super::recovery_identity_tests::pending_copy(area.path(), false).await;
    open_document(
        &state,
        host.clone(),
        area.path()
            .join("original.onecad")
            .to_string_lossy()
            .into_owned(),
        Some("openSaved".into()),
    )
    .await
    .unwrap();
    let context = {
        let guard = state.runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        PreviewContext {
            document_id: rt.document_id(),
            runtime_session: rt.runtime_session(),
            revision: rt.revision(),
            worker_epoch: rt.fencing().get().1,
            snapshot_id: rt.head_snapshot_id().unwrap_or(SnapshotId(0)),
            expected_base_hash: onecad_core::regen::HistoryPrefixHash::empty(),
        }
    };
    open_document(
        &state,
        host.clone(),
        area.path()
            .join("original.onecad")
            .to_string_lossy()
            .into_owned(),
        Some("openSaved".into()),
    )
    .await
    .unwrap();
    let before = get_projection(&state).await.unwrap();
    assert_eq!(before.document_id, context.document_id);
    assert_ne!(before.runtime_session, context.runtime_session);
    let record = OperationRecord::new(
        RecordId::new(),
        0,
        "Future feature",
        serde_json::from_value(serde_json::json!({"type":"FutureFeature", "params":{}})).unwrap(),
    );
    let error = apply_operation_in_context(
        &state,
        host,
        vec![EditCommand::AddOperation {
            record,
            at_cursor: false,
        }],
        None,
        &context,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ApiError::StalePreview(_)));
    assert_eq!(get_projection(&state).await.unwrap(), before);
}

const DRAFT_BYTES: &[u8] = b"owned component geometry for fenced commit";

async fn captured_component_fixture(
    root: &Path,
) -> (
    AppState,
    HostContext,
    onecad_core::document::record::PlaceComponentParams,
    String,
) {
    let (state, host, _, _) = super::recovery_identity_tests::pending_copy(root, false).await;
    let (params, sha) = capture_component(root, &state).await;
    (state, host, params, sha)
}

async fn capture_component(
    root: &Path,
    state: &AppState,
) -> (onecad_core::document::record::PlaceComponentParams, String) {
    let library_root = root.join("component-library");
    let package_dir = library_root.join("example.component");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::write(package_dir.join("source.onecad"), b"source document").unwrap();
    let sha = onecad_library::blob::BlobStore::new(&library_root)
        .put(DRAFT_BYTES)
        .unwrap();
    let manifest = format!(
        "[identity]\nid = \"example.component\"\nversion = \"1.0.0\"\nrevision = \"sha256:{}\"\n\
         [metadata]\nname = \"Component\"\n\
         [source]\nkind = \"document\"\nfile = \"source.onecad\"\ngeometry = \"sha256:{sha}\"\ngeometry_codec = \"brep\"\ngeometry_format = 4\n\
         [parameters]\n",
        "0".repeat(64)
    );
    std::fs::write(package_dir.join("component.toml"), &manifest).unwrap();
    let package = onecad_library::package::parse(&manifest, &package_dir).unwrap();
    onecad_library::package::write_package(&package_dir, &package).unwrap();
    std::fs::copy(
        package_dir.join("component.toml"),
        root.join("expected-manifest.toml"),
    )
    .unwrap();
    crate::library::reindex_library_at(&library_root).unwrap();
    let params = crate::library::prepare_component_draft_at(
        &library_root,
        state,
        crate::library::PlaceComponentRequest {
            component_id: "example.component".into(),
            component_version: "1.0.0".into(),
            translate: [0.0; 3],
            rotate: None,
            params: Default::default(),
            mate: None,
            configuration: None,
        },
    )
    .await
    .unwrap();
    assert!(crate::library::captured_component_draft_bytes(state) > DRAFT_BYTES.len());
    std::fs::remove_dir_all(library_root).unwrap();
    (params, sha)
}

async fn local_context(state: &AppState) -> PreviewContext {
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    PreviewContext {
        document_id: runtime.document_id(),
        runtime_session: runtime.runtime_session(),
        revision: runtime.revision(),
        worker_epoch: runtime.fencing().get().1,
        snapshot_id: runtime.head_snapshot_id().unwrap_or(SnapshotId(0)),
        expected_base_hash: onecad_core::regen::HistoryPrefixHash::empty(),
    }
}

fn component_command(params: onecad_core::document::record::PlaceComponentParams) -> EditCommand {
    use onecad_core::document::record::{KnownOperation, Operation};
    EditCommand::AddOperation {
        record: OperationRecord::new(
            RecordId::new(),
            0,
            "Component",
            Operation::Known(KnownOperation::PlaceComponent(params)),
        ),
        at_cursor: false,
    }
}

#[tokio::test]
async fn fresh_component_commit_stages_exact_captured_bytes() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, params, sha) = captured_component_fixture(area.path()).await;
    let revision = params.component_revision.clone();
    let context = local_context(&state).await;
    let command = component_command(params);
    apply_operation_in_context(&state, host, vec![command], None, &context)
        .await
        .unwrap();
    let mut guard = state.runtime.lock().await;
    let runtime = guard.as_mut().unwrap();
    assert_eq!(runtime.import_blob_shas(), vec![sha.clone()]);
    assert_eq!(
        std::fs::read(runtime.import_workspace_dir().join(format!("{sha}.brep"))).unwrap(),
        DRAFT_BYTES
    );
    assert_eq!(crate::library::captured_component_draft_bytes(&state), 0);
    let first = area.path().join("with-component.onecad");
    let second = area.path().join("with-component-copy.onecad");
    runtime.save(&first, SaveMeta::default()).unwrap();
    runtime.save(&second, SaveMeta::default()).unwrap();
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );
    drop(guard);
    let (engine, meshes, solver) = state.make_backend();
    let reopened =
        crate::document_runtime::DocumentRuntime::open(&first, engine, meshes, solver).unwrap();
    let (entry, archive) = reopened
        .captured_library_archive("example.component", "1.0.0", &revision)
        .unwrap()
        .unwrap();
    let snapshot = crate::library_archive::decode(&entry, &revision, &archive, &Default::default())
        .unwrap()
        .0;
    assert_eq!(snapshot.package.identity.id, "example.component");
    assert_eq!(
        snapshot.manifest,
        std::fs::read_to_string(area.path().join("expected-manifest.toml")).unwrap()
    );
    assert_eq!(
        snapshot.document.as_deref(),
        Some(b"source document".as_slice())
    );
    let onecad_library::resolve::ResolvedSource::Document { blob, .. } = snapshot.source else {
        panic!("document source expected")
    };
    assert_eq!(blob.bytes, DRAFT_BYTES);
    *state.runtime.lock().await = Some(reopened);
    let detail =
        crate::library::component_detail_archived(&state, "example.component", "1.0.0", &revision)
            .await
            .unwrap()
            .expect("captured definition");
    assert_eq!(detail.component.name, "Component");
    assert_eq!(detail.component.revision, revision);
}

#[tokio::test]
async fn invalid_component_transaction_rolls_back_new_import_and_retains_draft() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, params, sha) = captured_component_fixture(area.path()).await;
    let context = local_context(&state).await;
    let command = component_command(params);
    let result =
        apply_operation_in_context(&state, host, vec![command.clone(), command], None, &context)
            .await;
    assert!(result.is_err());
    let runtime = state.runtime.lock().await;
    let runtime = runtime.as_ref().unwrap();
    assert!(runtime.import_blob_shas().is_empty());
    assert_eq!(
        std::fs::read(runtime.import_workspace_dir().join(format!("{sha}.brep"))).unwrap(),
        DRAFT_BYTES
    );
    assert!(crate::library::captured_component_draft_bytes(&state) > DRAFT_BYTES.len());
}

#[tokio::test]
async fn cancelled_component_draft_refuses_before_staging() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, params, _sha) = captured_component_fixture(area.path()).await;
    let context = local_context(&state).await;
    crate::library::clear_component_draft_capture(&state);
    let result = apply_operation_in_context(
        &state,
        host,
        vec![component_command(params)],
        None,
        &context,
    )
    .await;
    assert!(matches!(result, Err(ApiError::StalePreview(_))));
    let runtime = state.runtime.lock().await;
    let runtime = runtime.as_ref().unwrap();
    assert!(runtime.import_blob_shas().is_empty());
}

struct HeldPreview {
    started: tokio::sync::Notify,
    finish: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl crate::worker::PreviewEngine for HeldPreview {
    async fn preview_op(
        &self,
        _: onecad_core::document::record::Operation,
        _: String,
        _: Option<String>,
        _: Option<SnapshotId>,
        _: onecad_core::regen::Lod,
    ) -> Result<crate::dto::PreviewResultDto, onecad_core::regen::EngineError> {
        self.started.notify_one();
        self.finish.notified().await;
        Err(onecad_core::regen::EngineError::Cancelled)
    }
}

#[tokio::test]
async fn abandoned_preview_holds_draft_workspace_until_worker_finishes() {
    use std::sync::Arc;
    let area = tempfile::tempdir().unwrap();
    let engine = Arc::new(HeldPreview {
        started: Default::default(),
        finish: Default::default(),
    });
    let backend = engine.clone();
    let state = Arc::new(AppState::new(Arc::new(move || {
        let pending = Arc::new(crate::worker::PendingBackend);
        (
            pending.clone(),
            pending.clone(),
            pending.clone(),
            pending.clone(),
            pending.clone(),
            backend.clone(),
            pending.clone(),
            pending.clone(),
            pending.clone(),
            pending,
        )
    })));
    let (geometry, meshes, solver) = state.make_backend();
    *state.runtime.lock().await = Some(crate::document_runtime::DocumentRuntime::new_blank(
        geometry, meshes, solver,
    ));
    let (params, _) = capture_component(area.path(), &state).await;
    let operation = onecad_core::document::record::Operation::Known(
        onecad_core::document::record::KnownOperation::PlaceComponent(params),
    );
    let session = local_context(&state).await.runtime_session;
    let weak = Arc::downgrade(
        &crate::library::captured_component_draft_workspace(&state, &operation, &session).unwrap(),
    );
    let job_state = state.clone();
    let task = tokio::spawn(async move {
        preview_operation(
            &job_state,
            operation,
            RecordId::new(),
            None,
            None,
            onecad_core::regen::Lod::Coarse,
        )
        .await
    });
    engine.started.notified().await;
    task.abort();
    let _ = task.await;
    crate::library::clear_component_draft_capture(&state);
    assert!(
        weak.upgrade().is_some(),
        "worker must retain its workspace after cancellation"
    );
    engine.finish.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("workspace released after terminal worker response");
}

#[tokio::test]
async fn missing_archive_keeps_frozen_geometry_but_refuses_source_and_save() {
    use std::io::{Read, Write};
    let area = tempfile::tempdir().unwrap();
    let (state, host, params, sha) = captured_component_fixture(area.path()).await;
    let revision = params.component_revision.clone();
    let context = local_context(&state).await;
    apply_operation_in_context(
        &state,
        host,
        vec![component_command(params)],
        None,
        &context,
    )
    .await
    .unwrap();
    let good = area.path().join("good.onecad");
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(&good, SaveMeta::default())
        .unwrap();
    let bad = area.path().join("missing.onecad");
    let mut source = zip::ZipArchive::new(std::fs::File::open(good).unwrap()).unwrap();
    let mut target = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for index in 0..source.len() {
        let mut file = source.by_index(index).unwrap();
        let name = file.name().to_owned();
        if name.starts_with("library/") {
            continue;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        target.start_file(name, options).unwrap();
        target.write_all(&bytes).unwrap();
    }
    target.finish().unwrap();
    let (engine, meshes, solver) = state.make_backend();
    let mut reopened =
        crate::document_runtime::DocumentRuntime::open(&bad, engine, meshes, solver).unwrap();
    assert!(reopened.import_blob_shas().contains(&sha));
    assert!(reopened
        .captured_library_archive("example.component", "1.0.0", &revision)
        .is_err());
    assert!(reopened
        .save(
            &area.path().join("must-not-save.onecad"),
            SaveMeta::default()
        )
        .is_err());
}

#[tokio::test]
async fn generator_parameters_use_saved_archive_after_library_removal() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) =
        super::recovery_identity_tests::pending_copy(area.path(), false).await;
    let root = area.path().join("offline-library");
    let dir = root.join("example.generator");
    std::fs::create_dir_all(&dir).unwrap();
    let raw = format!("[identity]\nid = \"example.generator\"\nversion = \"1.0.0\"\nrevision = \"sha256:{}\"\n[metadata]\nname = \"Generator\"\n[source]\nkind = \"generator\"\ngenerator = \"example\"\ngenerator_version = 1\n", "0".repeat(64));
    std::fs::write(dir.join("component.toml"), &raw).unwrap();
    let package = onecad_library::package::parse(&raw, &dir).unwrap();
    onecad_library::package::write_package(&dir, &package).unwrap();
    crate::library::reindex_library_at(&root).unwrap();
    let params = crate::library::prepare_component_draft_at(
        &root,
        &state,
        crate::library::PlaceComponentRequest {
            component_id: "example.generator".into(),
            component_version: "1.0.0".into(),
            translate: [0.0; 3],
            rotate: None,
            params: Default::default(),
            mate: None,
            configuration: None,
        },
    )
    .await
    .unwrap();
    let command = component_command(params);
    let EditCommand::AddOperation { record, .. } = &command else {
        unreachable!()
    };
    let record_id = record.record_id.to_string();
    let context = local_context(&state).await;
    apply_operation_in_context(&state, host, vec![command], None, &context)
        .await
        .unwrap();
    let saved = area.path().join("offline.onecad");
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(&saved, SaveMeta::default())
        .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let (engine, meshes, solver) = state.make_backend();
    let reopened =
        crate::document_runtime::DocumentRuntime::open(&saved, engine, meshes, solver).unwrap();
    *state.runtime.lock().await = Some(reopened);
    let instances = crate::library_instances::component_instances_at(None, &state)
        .await
        .unwrap();
    assert!(
        matches!(&instances[0].status, crate::dto::ComponentStatusDto::ArchivedSnapshot { catalog } if matches!(catalog.as_ref(), crate::dto::ComponentStatusDto::DefinitionMissing))
    );
    crate::library::set_component_params_at(&root, &state, record_id, Default::default())
        .await
        .expect("offline edit uses verified archive");
}

#[tokio::test]
async fn undo_prunes_saved_archive_and_redo_restores_it() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, params, _) = captured_component_fixture(area.path()).await;
    let context = local_context(&state).await;
    apply_operation_in_context(
        &state,
        host,
        vec![component_command(params)],
        None,
        &context,
    )
    .await
    .unwrap();
    let mut guard = state.runtime.lock().await;
    let runtime = guard.as_mut().unwrap();
    let retained = runtime.library_archive_bytes();
    assert!(retained > 0);
    assert!(runtime.undo().is_some());
    let undone = area.path().join("undone.onecad");
    runtime.save(&undone, SaveMeta::default()).unwrap();
    assert!(onecad_core::io::container::ContainerReader::open(&undone)
        .unwrap()
        .library_archives()
        .is_empty());
    assert_eq!(
        runtime.library_archive_bytes(),
        retained,
        "redo still owns archive bytes"
    );
    assert!(runtime.redo().unwrap().is_some());
    let redone = area.path().join("redone.onecad");
    runtime.save(&redone, SaveMeta::default()).unwrap();
    assert_eq!(
        onecad_core::io::container::ContainerReader::open(&redone)
            .unwrap()
            .library_archives()
            .len(),
        1
    );
}

#[tokio::test]
async fn failed_commit_preserves_preexisting_unknown_archive_classification() {
    use std::sync::Arc;
    let area = tempfile::tempdir().unwrap();
    let (state, host, params, _) = captured_component_fixture(area.path()).await;
    let context = local_context(&state).await;
    apply_operation_in_context(
        &state,
        host.clone(),
        vec![component_command(params)],
        None,
        &context,
    )
    .await
    .unwrap();
    let first = area.path().join("first.onecad");
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(&first, SaveMeta::default())
        .unwrap();
    let loaded = onecad_core::io::container::ContainerReader::open(&first).unwrap();
    let sha = loaded.library_archives()[0].sha256.clone();
    let bytes = loaded.read_library_archive(&sha).unwrap();
    let orphan = area.path().join("unknown.onecad");
    let document = onecad_core::document::Document::new(onecad_core::ids::DocumentId::new());
    let archives = onecad_core::io::library::LibraryArchives::from([(
        sha.clone(),
        Arc::new(onecad_core::io::library::ArchiveBytes::untracked(bytes)),
    )]);
    onecad_core::io::container::ContainerWriter::save_with_imports_and_library(
        &orphan,
        &document,
        &onecad_core::io::container::ContainerCaches::none(),
        &onecad_core::io::imports::ImportBlobs::new(),
        &archives,
        &SaveMeta::default(),
    )
    .unwrap();
    crate::library::clear_component_draft_capture(&state);
    let (engine, meshes, solver) = state.make_backend();
    *state.runtime.lock().await = Some(
        crate::document_runtime::DocumentRuntime::open(&orphan, engine, meshes, solver).unwrap(),
    );
    let (params, _) = capture_component(area.path(), &state).await;
    let context = local_context(&state).await;
    let command = component_command(params);
    assert!(apply_operation_in_context(
        &state,
        host,
        vec![command.clone(), command],
        None,
        &context
    )
    .await
    .is_err());
    let saved = area.path().join("retained.onecad");
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(&saved, SaveMeta::default())
        .unwrap();
    let loaded = onecad_core::io::container::ContainerReader::open(&saved).unwrap();
    assert_eq!(loaded.library_archives().len(), 1);
    assert_eq!(loaded.library_archives()[0].sha256, sha);
}

#[tokio::test]
async fn admitted_archive_lifetime_follows_detached_save_payload_after_close() {
    use crate::resource_admission::{ArchiveAdmission, ArchiveBudget, ArchiveUse, Retention};
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    struct Admission(Arc<AtomicUsize>, Arc<AtomicBool>);
    struct Lease(Arc<AtomicUsize>, usize);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.fetch_sub(self.1, Ordering::SeqCst);
        }
    }
    impl ArchiveAdmission for Admission {
        fn reserve(&self, bytes: usize, _: ArchiveUse) -> Result<Retention, String> {
            if self.1.load(Ordering::SeqCst) {
                return Err("host closed".into());
            }
            self.0.fetch_add(bytes, Ordering::SeqCst);
            Ok(Arc::new(Lease(self.0.clone(), bytes)))
        }
    }
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) =
        super::recovery_identity_tests::pending_copy(area.path(), false).await;
    let total = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicBool::new(false));
    let budget = ArchiveBudget::new(Arc::new(Admission(total.clone(), closed.clone())));
    close_document(&state, host.clone()).await.unwrap();
    state.install_archive_budget(budget.clone()).unwrap();
    new_document(&state, host.clone()).await.unwrap();
    let (params, _) = capture_component(area.path(), &state).await;
    let context = local_context(&state).await;
    apply_operation_in_context(
        &state,
        host.clone(),
        vec![component_command(params)],
        None,
        &context,
    )
    .await
    .unwrap();
    let payload = state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .build_save_payload(
            SaveMeta::default(),
            crate::document_runtime::SaveCaches::none(),
        );
    let retained = total.load(Ordering::SeqCst);
    assert!(retained > 0);
    let second_root = area.path().join("second");
    std::fs::create_dir_all(&second_root).unwrap();
    let (second, second_host, _, _) =
        super::recovery_identity_tests::pending_copy(&second_root, false).await;
    close_document(&second, second_host.clone()).await.unwrap();
    second.install_archive_budget(budget).unwrap();
    new_document(&second, second_host.clone()).await.unwrap();
    let (second_params, _) = capture_component(&second_root, &second).await;
    let second_context = local_context(&second).await;
    apply_operation_in_context(
        &second,
        second_host.clone(),
        vec![component_command(second_params)],
        None,
        &second_context,
    )
    .await
    .unwrap();
    assert!(total.load(Ordering::SeqCst) > retained);
    close_document(&second, second_host).await.unwrap();
    assert_eq!(total.load(Ordering::SeqCst), retained);
    close_document(&state, host.clone()).await.unwrap();
    closed.store(true, Ordering::SeqCst);
    assert_eq!(total.load(Ordering::SeqCst), retained);
    crate::document_runtime::DocumentRuntime::write_payload(
        &area.path().join("retained.onecad"),
        &payload,
    )
    .unwrap();
    assert_eq!(total.load(Ordering::SeqCst), retained);
    closed.store(false, Ordering::SeqCst);
    open_document(
        &state,
        host.clone(),
        area.path()
            .join("retained.onecad")
            .to_string_lossy()
            .into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(total.load(Ordering::SeqCst) > retained);
    close_document(&state, host.clone()).await.unwrap();
    assert_eq!(total.load(Ordering::SeqCst), retained);
    closed.store(true, Ordering::SeqCst);
    let open = open_document(
        &state,
        host,
        area.path()
            .join("retained.onecad")
            .to_string_lossy()
            .into_owned(),
        None,
    )
    .await;
    assert!(open.is_err());
    assert!(state.runtime.lock().await.is_none());
    assert_eq!(total.load(Ordering::SeqCst), retained);
    drop(payload);
    assert_eq!(total.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn generic_component_entry_points_refuse_forgery_and_allow_pose_only_edit() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, params, _) = captured_component_fixture(area.path()).await;
    let command = component_command(params.clone());
    assert!(apply_edit_command(&state, host.clone(), command.clone())
        .await
        .is_err());
    let record_id = match &command {
        EditCommand::AddOperation { record, .. } => record.record_id,
        _ => unreachable!(),
    };
    let context = local_context(&state).await;
    apply_operation_in_context(&state, host.clone(), vec![command], None, &context)
        .await
        .unwrap();
    let before = local_context(&state).await;
    let mut forged = params.clone();
    if let onecad_core::document::record::ComponentSourceRef::Document { sha256, .. } =
        &mut forged.source
    {
        *sha256 = "b".repeat(64);
    } else {
        panic!("fixture is a document component");
    }
    let forged = EditCommand::UpdateOperationParams {
        record: record_id,
        op: Operation::Known(onecad_core::document::record::KnownOperation::PlaceComponent(forged)),
    };
    assert!(apply_edit_command(&state, host.clone(), forged.clone())
        .await
        .is_err());
    assert!(
        apply_operation_in_context(&state, host.clone(), vec![forged], None, &before)
            .await
            .is_err()
    );
    assert_eq!(local_context(&state).await.revision, before.revision);
    let mut moved = params;
    moved.placement.translate[0] = onecad_core::document::variables::Scalar::new(2.0);
    let moved = EditCommand::UpdateOperationParams {
        record: record_id,
        op: Operation::Known(onecad_core::document::record::KnownOperation::PlaceComponent(moved)),
    };
    assert!(
        apply_operation_in_context(&state, host, vec![moved], None, &before)
            .await
            .is_ok()
    );
}

async fn reject_stale_component_update(
    state: &AppState,
    record: RecordId,
    params: &onecad_core::document::record::PlaceComponentParams,
    command: &EditCommand,
) {
    use crate::library::commit_authorization::{authorize_update, check_existing_update};
    use onecad_core::document::record::KnownOperation;
    let receipt = authorize_update(state, command)
        .await
        .unwrap()
        .expect("parameter change requires authorization");
    let mut guard = state.runtime.lock().await;
    let runtime = guard.as_mut().unwrap();
    let before = (runtime.revision(), runtime.undo_depth(), runtime.is_dirty());
    let mut tampered = command.clone();
    let EditCommand::UpdateOperationParams {
        op: Operation::Known(KnownOperation::PlaceComponent(candidate)),
        ..
    } = &mut tampered
    else {
        panic!("component update required")
    };
    candidate.placement.translate[0] = onecad_core::document::variables::Scalar::new(5.0);
    assert!(check_existing_update(runtime, &tampered, Some(&receipt)).is_err());
    assert_eq!(
        (runtime.revision(), runtime.undo_depth(), runtime.is_dirty()),
        before
    );
    let mut moved = params.clone();
    moved.placement.translate[0] = onecad_core::document::variables::Scalar::new(2.0);
    runtime
        .apply(EditCommand::UpdateOperationParams {
            record,
            op: Operation::Known(KnownOperation::PlaceComponent(moved)),
        })
        .unwrap();
    let after = (runtime.revision(), runtime.undo_depth(), runtime.is_dirty());
    assert!(check_existing_update(runtime, command, Some(&receipt)).is_err());
    assert_eq!(
        (runtime.revision(), runtime.undo_depth(), runtime.is_dirty()),
        after
    );
}

#[tokio::test]
async fn existing_generator_parameter_edit_uses_pinned_archive() {
    use onecad_core::document::record::{ComponentParamValue, KnownOperation};
    use std::collections::BTreeMap;
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) =
        super::recovery_identity_tests::pending_copy(area.path(), false).await;
    let root = area.path().join("component-library");
    let directory = root.join("example.generator");
    std::fs::create_dir_all(&directory).unwrap();
    let manifest = format!(
        r#"
[identity]
id = "example.generator"
version = "1.0.0"
revision = "sha256:{}"
[metadata]
name = "Generator"
[source]
kind = "generator"
generator = "iso4762"
generator_version = 1
[parameters]
thread = {{ role = "free", value = "M3", domain = ["M3", "M6"] }}
"#,
        "0".repeat(64)
    );
    std::fs::write(directory.join("component.toml"), &manifest).unwrap();
    let package = onecad_library::package::parse(&manifest, &directory).unwrap();
    onecad_library::package::write_package(&directory, &package).unwrap();
    crate::library::reindex_library_at(&root).unwrap();
    let initial = BTreeMap::from([("thread".into(), ComponentParamValue::Text("M3".into()))]);
    let params = crate::library::prepare_component_draft_at(
        &root,
        &state,
        crate::library::PlaceComponentRequest {
            component_id: "example.generator".into(),
            component_version: "1.0.0".into(),
            translate: [0.0; 3],
            rotate: None,
            params: initial.clone(),
            mate: None,
            configuration: None,
        },
    )
    .await
    .unwrap();
    let command = component_command(params.clone());
    let record = match &command {
        EditCommand::AddOperation { record, .. } => record.record_id,
        _ => unreachable!(),
    };
    let context = local_context(&state).await;
    apply_operation_in_context(&state, host.clone(), vec![command], None, &context)
        .await
        .unwrap();
    let changed = BTreeMap::from([("thread".into(), ComponentParamValue::Text("M6".into()))]);
    let updated = params.with_free_params(changed).unwrap();
    let command = EditCommand::UpdateOperationParams {
        record,
        op: Operation::Known(KnownOperation::PlaceComponent(updated)),
    };
    reject_stale_component_update(&state, record, &params, &command).await;
    let context = local_context(&state).await;
    apply_operation_in_context(&state, host.clone(), vec![command], None, &context)
        .await
        .unwrap();
    let invalid = params
        .with_free_params(BTreeMap::from([(
            "thread".into(),
            ComponentParamValue::Text("M9".into()),
        )]))
        .unwrap();
    let invalid = EditCommand::UpdateOperationParams {
        record,
        op: Operation::Known(KnownOperation::PlaceComponent(invalid)),
    };
    let before = local_context(&state).await;
    assert!(apply_edit_command(&state, host, invalid).await.is_err());
    assert_eq!(local_context(&state).await.revision, before.revision);
    crate::library::set_component_params_at(&root, &state, record.to_string(), initial)
        .await
        .unwrap();
}
