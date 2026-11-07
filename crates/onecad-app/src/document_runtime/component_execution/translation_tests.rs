use super::*;
use onecad_core::{
    document::{
        record::{OperationRecord, TransformBodyParams},
        refs::LocalFrame,
        variables::Scalar,
    },
    edit::AssemblyEdit,
    ids::RecordId,
    io::container::ContainerReader,
};

fn operation(record: u128, bodies: Vec<BodyId>, offset: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(record)),
        0,
        "Translate",
        Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
            targets: bodies,
            translate: [Scalar::new(offset), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
            copy: false,
            extra: Default::default(),
        })),
    )
}

fn proof() -> (DocumentRuntime, ComponentDefId, BodyId, usize) {
    let loaded = ContainerReader::open(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/qa_plate_hole.onecad"),
    )
    .unwrap();
    let source = loaded.document();
    let id = ComponentDefId(Uuid::from_u128(0xAD01));
    let mut def = ComponentDef::new(id, "Plate");
    def.variables = source.variables.clone();
    def.sketches = source.sketches.clone();
    def.sketch_visibility = source.sketch_visibility.clone();
    def.datum_planes = source.datum_planes.clone();
    def.timeline = source.timeline.clone();
    def.bodies = source.bodies.clone();
    def.elements = source.elements.clone();
    let prefix = def.timeline.len();
    let body = "88ba8696-36ea-4b00-b2de-d7eb24f39eb4".parse().unwrap();
    let mut doc = Document::new(DocumentId(Uuid::new_v4()));
    doc.definitions.insert(id, def);
    let engine = Arc::new(crate::worker::PendingBackend);
    let mut runtime = DocumentRuntime::from_document(
        doc,
        "assembly".into(),
        None,
        false,
        engine.clone(),
        engine.clone(),
        engine,
    );
    runtime.component_cache.insert(
        id,
        ComponentCache {
            fingerprint: fingerprint(&runtime.session.document().definitions[&id]).unwrap(),
            generation: 1,
            verified_head: Some(runtime.joint_head()),
            replay_context: replay_context(&runtime.session.document().settings, &runtime.imports)
                .unwrap(),
            identity_signature: BTreeMap::new(),
            meshes: BTreeMap::new(),
            budget: MeshBudget::default(),
        },
    );
    runtime
        .apply_assembly_edit(AssemblyEdit::EditDefinition {
            id,
            command: Box::new(EditCommand::AddOperation {
                record: operation(0xAD02, vec![body], 1.0),
                at_cursor: true,
            }),
        })
        .unwrap();
    (runtime, id, body, prefix)
}

#[test]
fn continuation_requires_exact_verified_head_and_context() {
    let (mut runtime, id, _, prefix) = proof();
    let def = runtime.session.document().definitions[&id].clone();
    assert_eq!(
        runtime
            .component_translation_prefix(id, &def)
            .unwrap()
            .prefix,
        prefix
    );
    runtime.component_cache.get_mut(&id).unwrap().replay_context[0] ^= 1;
    assert!(runtime.component_translation_prefix(id, &def).is_none());
    runtime.component_cache.get_mut(&id).unwrap().replay_context[0] ^= 1;
    runtime.fencing.set_epoch(runtime.fencing.get().1 .0 + 1);
    assert!(runtime.component_translation_prefix(id, &def).is_none());
}

#[test]
fn continuation_refuses_changed_opaque_context_or_prefix() {
    let (runtime, id, _, _) = proof();
    let mut def = runtime.session.document().definitions[&id].clone();
    def.extra
        .insert("future".into(), serde_json::json!({"meaning":2}));
    assert!(runtime.component_translation_prefix(id, &def).is_none());
    let mut def = runtime.session.document().definitions[&id].clone();
    let mut records = def.timeline.records().to_vec();
    records[0].name.push_str(" changed");
    def.timeline = onecad_core::history::Timeline::from_records(records);
    assert!(runtime.component_translation_prefix(id, &def).is_none());
}

#[test]
fn translation_composes_only_for_each_target_body() {
    let a = BodyId(Uuid::from_u128(1));
    let b = BodyId(Uuid::from_u128(2));
    let mut def = ComponentDef::new(ComponentDefId(Uuid::new_v4()), "part");
    def.timeline = onecad_core::history::Timeline::from_records(vec![
        operation(1, vec![a], 1.0),
        operation(2, vec![b], 5.0),
        operation(3, vec![a, b], 2.0),
    ]);
    let offsets = translations(&def, 0).unwrap();
    assert_eq!(offsets[&a], [3.0, 0.0, 0.0]);
    assert_eq!(offsets[&b], [7.0, 0.0, 0.0]);
    let original = def.timeline.records().to_vec();
    for which in 0..4 {
        let mut records = original.clone();
        let Operation::Known(KnownOperation::TransformBody(params)) = &mut records[0].op else {
            unreachable!()
        };
        match which {
            0 => params.copy = true,
            1 => params.rotate.angle_deg = Scalar::new(180.0),
            2 => params.translate[0] = Scalar::with_expr(1.0, "distance"),
            _ => params.translate[0].value = f64::INFINITY,
        }
        def.timeline = onecad_core::history::Timeline::from_records(records);
        assert!(
            translations(&def, 0).is_none(),
            "unsupported transform {which}"
        );
    }
}

#[test]
fn translated_anchor_preserves_uv_axes_and_unknown_intent() {
    let extra: onecad_core::document::refs::Extra =
        serde_json::from_value(serde_json::json!({"future":{"a":[1,2,3]}})).unwrap();
    let anchor = AnchorIntent {
        world_point: Vec3::new_unchecked(1.0, 2.0, 3.0),
        surface_uv: Some(onecad_core::math::Vec2::new_unchecked(0.2, 0.3)),
        local_frame: Some(LocalFrame {
            origin: Vec3::new_unchecked(2.0, 3.0, 4.0),
            x: Vec3::new_unchecked(1.0, 0.0, 0.0),
            y: Vec3::new_unchecked(0.0, 1.0, 0.0),
            z: Vec3::new_unchecked(0.0, 0.0, 1.0),
            extra: extra.clone(),
        }),
        adjacency_hint: Some("adjacency".into()),
        extra,
    };
    let entry = ElementEntry::new(BodyId(Uuid::new_v4()), ElementKind::Face)
        .with_evidence(Some(anchor.clone()), Some(serde_json::json!({"opaque":1})));
    let shifted = translated_anchor(&entry, [5.0, 6.0, 7.0]).unwrap().unwrap();
    let mut expected = anchor;
    expected.world_point = Vec3::new_unchecked(6.0, 8.0, 10.0);
    expected.local_frame.as_mut().unwrap().origin = Vec3::new_unchecked(7.0, 9.0, 11.0);
    assert_eq!(shifted, expected);
    assert_eq!(translated_anchor(&entry, [0.0; 3]).unwrap(), entry.anchor);
    assert!(translated_anchor(&entry, [f64::INFINITY, 0.0, 0.0]).is_err());
}

#[test]
fn streamed_fingerprint_matches_existing_bytes_and_context_changes() {
    let (_, id, _, _) = proof();
    let def = ComponentDef::new(id, "part");
    assert_eq!(
        fingerprint(&def).unwrap(),
        format!("{:x}", Sha256::digest(serde_json::to_vec(&def).unwrap()))
    );
    let imports = ImportBlobs::new();
    let mut settings = onecad_core::document::DocumentSettings::default();
    let before = replay_context(&settings, &imports);
    settings.tolerance_policy_hash = "different policy".into();
    assert_ne!(before, replay_context(&settings, &imports));
}

#[test]
fn historical_proof_refuses_when_its_own_reservation_is_denied() {
    struct Deny(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl crate::resource_admission::ArchiveAdmission for Deny {
        fn reserve(
            &self,
            bytes: usize,
            _: crate::resource_admission::ArchiveUse,
        ) -> Result<crate::resource_admission::Retention, String> {
            assert!(bytes > 0);
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err("historical proof denied".into())
        }
    }
    let (mut runtime, id, _, _) = proof();
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    runtime.set_archive_budget(crate::resource_admission::ArchiveBudget::new(Arc::new(
        Deny(attempts.clone()),
    )));
    let def = &runtime.session.document().definitions[&id];
    assert!(runtime.component_translation_prefix(id, def).is_none());
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn native_continuation_refuses_imported_frozen_or_certified_definitions() {
    use onecad_core::document::{
        import_source_identity::{ImportIdentityCodec, ImportSourceAddress, ImportSourceIdentity},
        record::{ImportSourceCodec, ImportStepParams, OpaqueOperation},
    };
    let (runtime, id, body, _) = proof();
    let native = runtime.session.document().definitions[&id].clone();
    assert!(native_definition(&native));
    let mut imported = native.clone();
    let import = Operation::Known(KnownOperation::ImportStep(ImportStepParams {
        source_sha256: "a".repeat(64),
        source_codec: ImportSourceCodec::Step,
        source_name: "source.step".into(),
        heal_policy: "occt-step-default-v1".into(),
        unit_scale: Scalar::new(1.0),
        brep_format: None,
        provenance_sha256: None,
        extra: Default::default(),
    }));
    imported.timeline.insert_at_cursor(OperationRecord::new(
        RecordId(Uuid::new_v4()),
        0,
        "Imported",
        import,
    ));
    assert!(!native_definition(&imported));
    let placed = Operation::Known(KnownOperation::PlaceComponent(
        onecad_core::document::record::PlaceComponentParams {
            component_id: "test.embedded".into(),
            component_version: "1.0.0".into(),
            component_revision: format!("sha256:{}", "a".repeat(64)),
            params: BTreeMap::new(),
            source: onecad_core::document::record::ComponentSourceRef::Embedded {
                sha256: "a".repeat(64),
                codec: ImportSourceCodec::Step,
                brep_format: None,
                extra: Default::default(),
            },
            mate: None,
            placement: onecad_core::document::record::FrozenPlacement {
                translate: std::array::from_fn(|_| Scalar::new(0.0)),
                rotate: Default::default(),
            },
            extra: Default::default(),
        },
    ));
    let mut embedded = native.clone();
    embedded.timeline.insert_at_cursor(OperationRecord::new(
        RecordId(Uuid::new_v4()),
        0,
        "Embedded source",
        placed,
    ));
    assert!(!native_definition(&embedded));
    let mut opaque = native.clone();
    opaque.timeline.insert_at_cursor(OperationRecord::new(
        RecordId(Uuid::new_v4()),
        0,
        "Frozen",
        Operation::Opaque(OpaqueOperation {
            raw: serde_json::from_value(serde_json::json!({"opType":"FutureSolid"})).unwrap(),
        }),
    ));
    assert!(!native_definition(&opaque));
    let mut certified = native;
    certified.elements.insert(
        ElementId::new("el_certified"),
        ElementEntry::new(body, ElementKind::Face).with_import_source_identity(Some(
            ImportSourceIdentity {
                version: 1,
                source_sha256: "a".repeat(64),
                codec: ImportIdentityCodec::Step,
                codec_version: "ISO-10303-21".into(),
                reader_policy: "occt-step-default-v1".into(),
                kernel_fingerprint: "0a6a1dce34181289".into(),
                producer_record_id: RecordId(Uuid::from_u128(1)),
                body_id: body,
                unit_scale_bits: format!("{:016x}", 1.0f64.to_bits()),
                source_address: ImportSourceAddress::Step {
                    root_entity_label: 1,
                    occurrence_path: vec![],
                    face_entity_label: 2,
                },
            },
        )),
    );
    assert!(!native_definition(&certified));
}
