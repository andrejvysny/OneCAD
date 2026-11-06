use super::*;
use onecad_core::document::import_source_identity::ImportSourceIdentity;
use onecad_core::ids::{ElementId, TopoKey};

#[tokio::test]
async fn source_evidence_must_match_authored_import_and_existing_identity() {
    let body = BodyId(Uuid::from_u128(1));
    let producer = RecordId(Uuid::from_u128(1));
    let mut document = Document::new(DocumentId(Uuid::from_u128(9)));
    let params = onecad_core::document::record::ImportStepParams {
        source_sha256: "a".repeat(64),
        source_codec: onecad_core::document::record::ImportSourceCodec::Step,
        source_name: "source.step".into(),
        heal_policy: "v1".into(),
        unit_scale: Scalar::new(1.0),
        brep_format: None,
        provenance_sha256: None,
        extra: Default::default(),
    };
    document.timeline.insert_at_cursor(OperationRecord::new(
        producer,
        0,
        "Import",
        Operation::Known(KnownOperation::ImportStep(params)),
    ));
    document
        .bodies
        .register(onecad_core::document::body::BodyMeta::new(
            body, "Body", producer,
        ));
    let definition = onecad_core::document::component::ComponentDefId(Uuid::from_u128(11));
    document.definitions.insert(
        definition,
        onecad_core::document::component::ComponentDef::new(definition, "other"),
    );
    let backend = Arc::new(FakeBackend::with_overrides(HashMap::from([(
        0,
        vec![body],
    )])));
    let mut runtime = DocumentRuntime::from_document(
        document,
        "test".into(),
        None,
        false,
        backend.clone(),
        backend.clone(),
        backend,
    );
    let identity: ImportSourceIdentity = serde_json::from_value(serde_json::json!({
        "version":1,"sourceSha256":"a".repeat(64),"codec":"step","codecVersion":"occt-step-stream-v1",
        "readerPolicy":"step-v1","kernelFingerprint":"0a6a1dce34181289","producerRecordId":producer,
        "bodyId":body,"unitScaleBits":"3ff0000000000000","sourceAddress":{"kind":"step",
        "rootEntityLabel":5850,"occurrencePath":[],"faceEntityLabel":3042}})).unwrap();
    let mut evidence = WorkerElementEvidence {
        topo_key: TopoKey::new("f:1"),
        body,
        kind: onecad_core::document::refs::ElementKind::Face,
        anchor: None,
        descriptor: None,
        existing: None,
        import_source_identity: Some(identity.clone()),
    };
    let report = runtime
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_) | Outcome::NoOp),
        "{report:?}"
    );
    let snapshot = runtime.head_snapshot_id().unwrap();
    let ticket = runtime
        .prepare_face_entry(
            &runtime.runtime_session(),
            runtime.revision(),
            snapshot,
            runtime.head_epoch,
        )
        .unwrap();
    let fresh = runtime
        .face_entry_identity(&ticket, evidence.clone())
        .unwrap();
    let sketch_for = |id: ElementId| {
        onecad_core::sketch::Sketch::new(
            onecad_core::ids::SketchId(Uuid::from_u128(13)),
            "face entry",
            onecad_core::sketch::SketchAttachment::HostFace {
                face: onecad_core::document::refs::ElementRef {
                    primary: Some(onecad_core::document::refs::PrimaryRef {
                        body,
                        element: id,
                        kind: onecad_core::document::refs::ElementKind::Face,
                        extra: Default::default(),
                    }),
                    intent: None,
                    anchor: None,
                    import_source_identity: Some(identity.clone()),
                    extra: Default::default(),
                },
                projected_boundary_version: 0,
            },
        )
    };
    let before = runtime.revision();
    runtime.read_only = true;
    assert!(runtime
        .commit_face_entry(&ticket, sketch_for(fresh.0.clone()), Some(fresh.clone()))
        .is_err());
    assert!(!runtime.staged.contains_key(&fresh.0));
    assert_eq!(runtime.revision(), before);
    runtime.read_only = false;
    runtime
        .commit_face_entry(&ticket, sketch_for(fresh.0.clone()), Some(fresh.clone()))
        .unwrap();
    assert_eq!(
        runtime
            .regen
            .elements
            .get(&fresh.0)
            .unwrap()
            .import_source_identity
            .as_ref(),
        Some(&identity)
    );
    let mut identity = identity.clone();
    identity.source_address =
        onecad_core::document::import_source_identity::ImportSourceAddress::Step {
            root_entity_label: 5850,
            occurrence_path: Vec::new(),
            face_entity_label: 3043,
        };
    evidence.import_source_identity = Some(identity.clone());
    assert!(runtime.validate_import_evidence(&evidence).is_ok());
    evidence
        .import_source_identity
        .as_mut()
        .unwrap()
        .source_sha256 = "b".repeat(64);
    assert!(runtime.validate_import_evidence(&evidence).is_err());
    evidence.import_source_identity = Some(identity.clone());
    let id = ElementId::new("el_existing");
    runtime.regen.elements.insert(
        id.clone(),
        ElementEntry::new(body, evidence.kind).with_import_source_identity(Some(identity.clone())),
    );
    evidence.existing = Some(id.clone());
    evidence.import_source_identity = None;
    assert!(runtime.validate_import_evidence(&evidence).is_err());
    let sketch = onecad_core::sketch::Sketch::new(
        onecad_core::ids::SketchId(Uuid::from_u128(10)),
        "sketch",
        onecad_core::sketch::SketchAttachment::HostFace {
            face: onecad_core::document::refs::ElementRef {
                primary: Some(onecad_core::document::refs::PrimaryRef {
                    body,
                    element: id.clone(),
                    kind: onecad_core::document::refs::ElementKind::Face,
                    extra: Default::default(),
                }),
                intent: None,
                anchor: None,
                import_source_identity: None,
                extra: Default::default(),
            },
            projected_boundary_version: 0,
        },
    );
    let revision = runtime.revision();
    assert!(runtime.apply(EditCommand::AddSketch { sketch }).is_err());
    assert_eq!(runtime.revision(), revision);
    let make_sketch = |element: ElementId| {
        onecad_core::sketch::Sketch::new(
            onecad_core::ids::SketchId(Uuid::from_u128(12)),
            "certified",
            onecad_core::sketch::SketchAttachment::HostFace {
                face: onecad_core::document::refs::ElementRef {
                    primary: Some(onecad_core::document::refs::PrimaryRef {
                        body,
                        element,
                        kind: onecad_core::document::refs::ElementKind::Face,
                        extra: Default::default(),
                    }),
                    intent: None,
                    anchor: None,
                    import_source_identity: Some(identity.clone()),
                    extra: Default::default(),
                },
                projected_boundary_version: 0,
            },
        )
    };
    assert!(runtime
        .apply(EditCommand::AddSketch {
            sketch: make_sketch(ElementId::new("el_unknown"))
        })
        .is_err());
    assert_eq!(runtime.revision(), revision);
    let valid_evidence = WorkerElementEvidence {
        import_source_identity: Some(identity.clone()),
        ..evidence.clone()
    };
    assert!(runtime
        .validate_minted_import_identities(&[(id.clone(), valid_evidence.clone())])
        .is_ok());
    assert!(runtime
        .validate_minted_import_identities(&[(ElementId::new("el_other"), valid_evidence.clone())])
        .is_err());
    assert!(runtime
        .validate_minted_import_identities(&[
            (id.clone(), valid_evidence.clone()),
            (ElementId::new("el_other"), valid_evidence)
        ])
        .is_err());
    let prior_dirty = runtime.is_dirty();
    let prior_undo = runtime.undo_depth();
    let prior_body = runtime.body_meta(body).unwrap().name;
    assert!(runtime
        .apply_transaction(
            "invalid batch",
            vec![
                EditCommand::RenameBody {
                    body,
                    name: "changed".into()
                },
                EditCommand::AddSketch {
                    sketch: make_sketch(ElementId::new("el_unknown"))
                },
            ]
        )
        .is_err());
    assert_eq!(runtime.revision(), revision);
    assert_eq!(runtime.is_dirty(), prior_dirty);
    assert_eq!(runtime.undo_depth(), prior_undo);
    assert_eq!(runtime.body_meta(body).unwrap().name, prior_body);
    let edit = onecad_core::edit::AssemblyEdit::EditDefinition {
        id: definition,
        command: Box::new(EditCommand::AddSketch {
            sketch: make_sketch(id),
        }),
    };
    assert!(runtime.apply_assembly_edit(edit).is_err());
    assert_eq!(runtime.revision(), revision);
}
