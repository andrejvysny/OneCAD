use super::*;
use onecad_core::document::{
    element_index::ElementEntry, import_source_identity::ImportSourceIdentity,
};
use onecad_core::ids::ElementId;

fn source(path: &std::path::Path) -> (ElementId, ElementEntry) {
    use onecad_core::io::{
        container::{ContainerCaches, ContainerWriter, SaveMeta},
        imports::{ImportBlob, ImportBlobs, OwnedSource},
    };
    use sha2::{Digest, Sha256};
    let bytes = format!("ISO-10303-21; source {}", path.display()).into_bytes();
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let body = BodyId(Uuid::from_u128(0x101));
    let producer = RecordId(Uuid::from_u128(0x100));
    let params = onecad_core::document::record::ImportStepParams {
        source_sha256: sha.clone(),
        source_codec: onecad_core::document::record::ImportSourceCodec::Step,
        source_name: "source.step".into(),
        heal_policy: "v1".into(),
        unit_scale: Scalar::new(1.0),
        brep_format: None,
        provenance_sha256: None,
        extra: Default::default(),
    };
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0x102)));
    doc.timeline.insert_at_cursor(OperationRecord::new(
        producer,
        0,
        "Import",
        Operation::Known(KnownOperation::ImportStep(params)),
    ));
    doc.bodies
        .register(onecad_core::document::body::BodyMeta::new(
            body, "body", producer,
        ));
    let identity: ImportSourceIdentity = serde_json::from_value(serde_json::json!({
        "version":1,"sourceSha256":sha,"codec":"step","codecVersion":"occt-step-stream-v1",
        "readerPolicy":"step-v1","kernelFingerprint":"0a6a1dce34181289","producerRecordId":producer,
        "bodyId":body,"unitScaleBits":"3ff0000000000000","sourceAddress":{"kind":"step",
        "rootEntityLabel":5850,"occurrencePath":[],"faceEntityLabel":3042}}))
    .unwrap();
    let element = ElementId::new("el_imported");
    let entry = ElementEntry::new(body, onecad_core::document::refs::ElementKind::Face)
        .with_import_source_identity(Some(identity.clone()));
    doc.elements.insert(element.clone(), entry.clone());
    doc.sketches.insert(
        SketchId(Uuid::from_u128(0x103)),
        Sketch::new(
            SketchId(Uuid::from_u128(0x103)),
            "hosted",
            SketchAttachment::HostFace {
                face: onecad_core::document::refs::ElementRef {
                    primary: Some(onecad_core::document::refs::PrimaryRef {
                        body,
                        element: element.clone(),
                        kind: onecad_core::document::refs::ElementKind::Face,
                        extra: Default::default(),
                    }),
                    intent: None,
                    anchor: None,
                    import_source_identity: Some(identity),
                    extra: Default::default(),
                },
                projected_boundary_version: 0,
            },
        ),
    );
    let imports = ImportBlobs::from([(
        sha,
        ImportBlob {
            codec: onecad_core::document::record::ImportSourceCodec::Step,
            bytes: Arc::new(OwnedSource::untracked(bytes)),
        },
    )]);
    ContainerWriter::save_with_imports(
        path,
        &doc,
        &ContainerCaches::none(),
        &imports,
        &SaveMeta::default(),
    )
    .unwrap();
    (element, entry)
}

#[test]
fn certified_project_import_keeps_evidence_in_undo_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.onecad");
    let (id, entry) = source(&path);
    let mut runtime = runtime_with(Arc::new(FakeBackend::new()));
    let before = runtime.undo_depth();
    runtime.import_project(&path).unwrap();
    assert_eq!(runtime.session.document().elements.get(&id), Some(&entry));
    assert_eq!(runtime.undo_depth(), before + 1);
    runtime.undo().unwrap();
    assert!(runtime.session.document().elements.get(&id).is_none());
    assert!(runtime.session.document().sketches.is_empty());
    assert!(runtime.session.document().timeline.is_empty());
    runtime.redo().unwrap();
    assert_eq!(runtime.session.document().elements.get(&id), Some(&entry));
}

#[test]
fn certified_import_refuses_identity_collision_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.onecad");
    let (id, entry) = source(&path);
    let mut doc = Document::new(DocumentId(Uuid::new_v4()));
    doc.elements.insert(id, entry);
    let backend = Arc::new(FakeBackend::new());
    let mut runtime = DocumentRuntime::from_document(
        doc,
        "target".into(),
        None,
        false,
        backend.clone(),
        backend.clone(),
        backend,
    );
    let before = serde_json::to_value(runtime.session.document()).unwrap();
    let generation = runtime.persisted_generation();
    assert!(runtime
        .import_project(&path)
        .unwrap_err()
        .to_string()
        .contains("source element id"));
    assert_eq!(
        serde_json::to_value(runtime.session.document()).unwrap(),
        before
    );
    assert_eq!(runtime.persisted_generation(), generation);
    assert_eq!(runtime.undo_depth(), 0);
    assert!(runtime.staged.is_empty());
    assert!(runtime.imports.is_empty());
}

#[test]
fn failed_project_transaction_releases_prepared_sources_and_identity() {
    use onecad_core::io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta};
    use onecad_core::io::project_import::read_project_for_import;
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.onecad");
    let source_path = dir.path().join("source.onecad");
    let (id, _) = source(&original);
    let loaded = ContainerReader::open(&original).unwrap();
    let mut doc = loaded.document().clone();
    let mut record = extrude_record(0x199, 1.0);
    let Operation::Known(KnownOperation::Extrude(params)) = &mut record.op else {
        unreachable!()
    };
    params.distance = Scalar::with_expr(1.0, "missing_target_variable");
    doc.timeline.insert_at_cursor(record);
    let imported = read_project_for_import(&original).unwrap();
    ContainerWriter::save_with_imports(
        &source_path,
        &doc,
        &ContainerCaches::none(),
        &imported.import_blobs,
        &SaveMeta::default(),
    )
    .unwrap();
    let sha = imported.import_blobs.keys().next().unwrap();
    assert!(crate::imports::resolve_blob_path(sha).is_empty());
    let mut runtime = runtime_with(Arc::new(FakeBackend::new()));
    let generation = runtime.persisted_generation();
    assert!(runtime.import_project(&source_path).is_err());
    assert!(crate::imports::resolve_blob_path(sha).is_empty());
    assert!(runtime.session.document().elements.get(&id).is_none());
    assert!(runtime.regen.elements.get(&id).is_none());
    assert!(runtime.session.document().timeline.is_empty());
    assert!(runtime.imports.is_empty());
    assert!(runtime.staged.is_empty());
    assert_eq!(runtime.persisted_generation(), generation);
    assert_eq!(runtime.undo_depth(), 0);
}

#[test]
fn nested_project_import_requires_source_admission() {
    use onecad_core::{document::component::ComponentDefId, edit::AssemblyEdit};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.onecad");
    source(&path);
    let imported = onecad_core::io::project_import::read_project_for_import(&path).unwrap();
    let mut runtime = runtime_with(Arc::new(FakeBackend::new()));
    let id = ComponentDefId(Uuid::from_u128(0x200));
    runtime
        .apply_assembly_edit(AssemblyEdit::CreateDefinition {
            id,
            name: "target".into(),
        })
        .unwrap();
    let generation = runtime.persisted_generation();
    let command = EditCommand::ImportProject {
        project: Box::new(onecad_core::edit::project_import::ProjectImportEdit {
            records: imported.records,
            sketches: imported.sketches,
            elements: imported.elements,
        }),
    };
    assert!(runtime
        .apply_assembly_edit(AssemblyEdit::EditDefinition {
            id,
            command: Box::new(command)
        })
        .is_err());
    assert_eq!(runtime.persisted_generation(), generation);
    assert!(runtime.session.document().definitions[&id]
        .timeline
        .is_empty());
    assert!(runtime.session.document().definitions[&id]
        .elements
        .is_empty());
    assert!(runtime.imports.is_empty());
}

#[test]
fn project_import_refuses_live_and_staged_element_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.onecad");
    let (id, entry) = source(&path);
    for staged in [false, true] {
        let mut runtime = runtime_with(Arc::new(FakeBackend::new()));
        if staged {
            runtime.staged.insert(id.clone(), entry.clone());
        } else {
            runtime.regen.elements.insert(id.clone(), entry.clone());
        }
        let generation = runtime.persisted_generation();
        assert!(runtime
            .import_project(&path)
            .unwrap_err()
            .to_string()
            .contains("source element id"));
        assert!(runtime.session.document().timeline.is_empty());
        assert!(runtime.imports.is_empty());
        assert_eq!(
            if staged {
                runtime.staged.get(&id)
            } else {
                runtime.regen.elements.get(&id)
            },
            Some(&entry)
        );
        assert_eq!(runtime.persisted_generation(), generation);
        assert_eq!(runtime.undo_depth(), 0);
    }
}

#[test]
fn project_import_certificates_must_match_imported_producers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.onecad");
    let (id, mut entry) = source(&path);
    let imported = onecad_core::io::project_import::read_project_for_import(&path).unwrap();
    let mut project = onecad_core::edit::project_import::ProjectImportEdit {
        records: imported.records,
        sketches: imported.sketches,
        elements: imported.elements,
    };
    project.validate().unwrap();
    entry.import_source_identity.as_mut().unwrap().source_sha256 = "b".repeat(64);
    project.elements.insert(id, entry);
    assert!(project.validate().is_err());
}

#[test]
fn import_undo_removes_later_imported_identities_and_redo_restores_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.onecad");
    let (_, entry) = source(&path);
    let mut runtime = runtime_with(Arc::new(FakeBackend::new()));
    runtime.import_project(&path).unwrap();
    let live_id = ElementId::new("el_later_live");
    let staged_id = ElementId::new("el_later_staged");
    let mut live = entry.clone();
    let mut staged = entry;
    let onecad_core::document::import_source_identity::ImportSourceAddress::Step {
        face_entity_label,
        ..
    } = &mut live.import_source_identity.as_mut().unwrap().source_address
    else {
        unreachable!()
    };
    *face_entity_label += 1;
    let onecad_core::document::import_source_identity::ImportSourceAddress::Step {
        face_entity_label,
        ..
    } = &mut staged
        .import_source_identity
        .as_mut()
        .unwrap()
        .source_address
    else {
        unreachable!()
    };
    *face_entity_label += 2;
    runtime.regen.elements.insert(live_id.clone(), live.clone());
    runtime.staged.insert(staged_id.clone(), staged.clone());
    let unrelated_id = ElementId::new("el_unrelated");
    let unrelated = ElementEntry::new(
        BodyId(Uuid::from_u128(0x777)),
        onecad_core::document::refs::ElementKind::Edge,
    );
    runtime
        .regen
        .elements
        .insert(unrelated_id.clone(), unrelated.clone());
    runtime.undo().unwrap();
    assert!(runtime.regen.elements.get(&live_id).is_none());
    assert!(!runtime.staged.contains_key(&staged_id));
    assert_eq!(runtime.regen.elements.get(&unrelated_id), Some(&unrelated));
    runtime.redo().unwrap().unwrap();
    assert_eq!(runtime.regen.elements.get(&live_id), Some(&live));
    assert!(runtime.regen.elements.get(&staged_id).is_none());
    assert!(!runtime.staged.contains_key(&staged_id));
    assert_eq!(runtime.regen.elements.get(&unrelated_id), Some(&unrelated));
}

#[test]
fn import_redo_refuses_new_live_or_staged_authority_without_publishing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.onecad");
    let (id, mut collision) = source(&path);
    collision.body = BodyId(Uuid::from_u128(0x999));
    collision.import_source_identity = None;
    for staged in [false, true] {
        let mut runtime = runtime_with(Arc::new(FakeBackend::new()));
        runtime.import_project(&path).unwrap();
        runtime.undo().unwrap();
        if staged {
            runtime.staged.insert(id.clone(), collision.clone());
        } else {
            runtime.regen.elements.insert(id.clone(), collision.clone());
        }
        let generation = runtime.persisted_generation();
        let redo = runtime.redo_depth();
        assert!(runtime.redo().is_err());
        assert_eq!(runtime.persisted_generation(), generation);
        assert_eq!(runtime.redo_depth(), redo);
        assert!(runtime.session.document().timeline.is_empty());
        assert_eq!(
            if staged {
                runtime.staged.get(&id)
            } else {
                runtime.regen.elements.get(&id)
            },
            Some(&collision)
        );
    }
}

#[test]
fn import_history_refuses_unproven_imported_producer_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.onecad");
    let (_, mut forged) = source(&path);
    let mut runtime = runtime_with(Arc::new(FakeBackend::new()));
    runtime.import_project(&path).unwrap();
    forged.body = BodyId(Uuid::from_u128(0x888));
    forged.import_source_identity.as_mut().unwrap().body_id = forged.body;
    let id = ElementId::new("el_forged_stage");
    runtime.staged.insert(id.clone(), forged.clone());
    let generation = runtime.persisted_generation();
    let depth = runtime.undo_depth();
    assert!(runtime.undo().is_none());
    assert_eq!(runtime.persisted_generation(), generation);
    assert_eq!(runtime.undo_depth(), depth);
    assert_eq!(runtime.staged.get(&id), Some(&forged));
    assert!(!runtime.session.document().timeline.is_empty());
}
