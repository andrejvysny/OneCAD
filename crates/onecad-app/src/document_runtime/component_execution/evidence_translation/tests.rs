use super::*;
use onecad_core::{
    document::{
        body::BodyMeta, element_index::ElementEntry, refs::AnchorIntent, variables::Scalar,
    },
    edit::AssemblyEdit,
};
use serde_json::json;

fn descriptor() -> serde_json::Value {
    json!({"shapeType":4,"center":[1.0,2.0,3.0],"size":3.0,"magnitude":2.0,
        "surfaceType":0,"curveType":8,"normal":[0.0,0.0,1.0],"tangent":[0.0,0.0,0.0],
        "hasNormal":true,"hasTangent":false,"adjacencyHash":"0123456789abcdef","outward":[0.0,0.0,1.0]})
}

fn operation(record: u128, body: BodyId, distance: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(record)),
        0,
        "Translate",
        Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
            targets: vec![body],
            translate: [Scalar::new(distance), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
            copy: false,
            extra: Default::default(),
        })),
    )
}

fn fixture(distance: f64) -> (DocumentRuntime, ComponentDefId) {
    fixture_with_anchor(
        distance,
        serde_json::from_value(json!({"worldPoint":[1.0,2.0,3.0]})).unwrap(),
    )
}

fn fixture_with_anchor(distance: f64, anchor: AnchorIntent) -> (DocumentRuntime, ComponentDefId) {
    let body = BodyId(Uuid::from_u128(0x300));
    let id = ComponentDefId(Uuid::from_u128(0x301));
    let mut definition = ComponentDef::new(id, "native");
    definition.timeline = onecad_core::history::Timeline::from_records(vec![
        operation(1, body, 4.0),
        operation(2, body, 1.0),
    ]);
    definition
        .bodies
        .register(BodyMeta::new(body, "body", RecordId(Uuid::from_u128(1))));
    definition.elements.insert(
        ElementId::new("el_proven"),
        ElementEntry::new(body, ElementKind::Face).with_evidence(Some(anchor), Some(descriptor())),
    );
    let mut document = Document::new(DocumentId(Uuid::new_v4()));
    document.definitions.insert(id, definition);
    let backend = Arc::new(crate::worker::PendingBackend);
    let mut runtime = DocumentRuntime::from_document(
        document,
        "test".into(),
        None,
        false,
        backend.clone(),
        backend.clone(),
        backend,
    );
    runtime.component_cache.insert(
        id,
        ComponentCache {
            fingerprint: fingerprint(&runtime.session.document().definitions[&id]).unwrap(),
            generation: 7,
            verified_head: Some(runtime.joint_head()),
            replay_context: translation::replay_context(
                &runtime.session.document().settings,
                &runtime.imports,
            )
            .unwrap(),
            identity_signature: BTreeMap::new(),
            meshes: BTreeMap::new(),
            budget: MeshBudget::default(),
        },
    );
    runtime
        .apply_assembly_edit(AssemblyEdit::EditDefinition {
            id,
            command: Box::new(EditCommand::UpdateOperationParams {
                record: RecordId(Uuid::from_u128(2)),
                op: operation(2, body, distance).op,
            }),
        })
        .unwrap();
    (runtime, id)
}

#[test]
fn replacement_to_identity_uses_delta_without_enabling_continuation() {
    let (runtime, id) = fixture(0.0);
    let definition = &runtime.session.document().definitions[&id];
    assert!(runtime
        .component_translation_prefix(id, definition)
        .is_none());
    let proof = runtime
        .component_evidence_translation(id, definition)
        .unwrap();
    let next = proof.translated_elements(&definition.elements).unwrap();
    let entry = next.get(&ElementId::new("el_proven")).unwrap();
    assert_eq!(
        entry.descriptor.as_ref().unwrap()["center"],
        json!([0.0, 2.0, 3.0])
    );
    assert_eq!(
        <[f64; 3]>::from(entry.anchor.as_ref().unwrap().world_point),
        [0.0, 2.0, 3.0]
    );
    assert_eq!(
        entry.body,
        definition.elements.iter().next().unwrap().1.body
    );
    assert_eq!(entry.import_source_identity, None);
    assert_eq!(
        entry.descriptor.as_ref().unwrap()["outward"],
        descriptor()["outward"]
    );
}

#[test]
fn descriptor_transport_refuses_unknown_malformed_and_overflow_fields() {
    for which in 0..8 {
        let mut value = descriptor();
        match which {
            0 => value["futurePosition"] = json!([0.0, 0.0, 0.0]),
            1 => value["center"] = json!([1.0, 2.0]),
            2 => value["size"] = json!(-1.0),
            3 => value["shapeType"] = json!(6),
            4 => value["hasNormal"] = json!("true"),
            5 => value["adjacencyHash"] = json!("BAD"),
            6 => value["generatorSlot"] = json!({"recordId":Uuid::nil(),"slot":"face","future":1}),
            _ => value["center"] = json!([f64::MAX, 0.0, 0.0]),
        }
        let delta = if which == 7 {
            [f64::MAX, 0.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        assert!(
            super::descriptor::translated(&value, ElementKind::Face, delta).is_err(),
            "case {which}"
        );
    }
}

#[test]
fn stale_capture_refuses_generation_persisted_head_and_context_changes() {
    for which in 0..4 {
        let (mut runtime, id) = fixture(0.0);
        let proof = runtime
            .component_evidence_translation(id, &runtime.session.document().definitions[&id])
            .unwrap();
        match which {
            0 => runtime.component_cache.get_mut(&id).unwrap().generation += 1,
            1 => runtime.persisted_generation += 1,
            2 => runtime.fencing.set_epoch(runtime.fencing.get().1 .0 + 1),
            _ => runtime.component_cache.get_mut(&id).unwrap().replay_context[0] ^= 1,
        }
        assert!(runtime.validate_evidence_translation(id, &proof).is_err());
    }
}

#[test]
fn eligibility_refuses_changed_prefix_context_copy_rotation_certificate_or_descriptor() {
    for which in 0..9 {
        let (mut runtime, id) = fixture(0.0);
        let mut definition = runtime.session.document().definitions[&id].clone();
        match which {
            0 => {
                let mut records = definition.timeline.records().to_vec();
                records[0].name.push('x');
                definition.timeline = onecad_core::history::Timeline::from_records(records);
            }
            1 => {
                definition.extra.insert("future".into(), json!(true));
            }
            2 | 3 => {
                let mut records = definition.timeline.records().to_vec();
                let Operation::Known(KnownOperation::TransformBody(params)) = &mut records[1].op
                else {
                    unreachable!()
                };
                if which == 2 {
                    params.copy = true
                } else {
                    params.rotate.angle_deg.value = 1.0
                }
                definition.timeline = onecad_core::history::Timeline::from_records(records);
            }
            4 => {
                runtime.read_only = true;
            }
            5 => {
                let id = ElementId::new("el_proven");
                let mut entry = definition.elements.get(&id).unwrap().clone();
                entry.descriptor.as_mut().unwrap()["future"] = json!(1);
                definition.elements.insert(id, entry);
            }
            6 => {
                let id = ElementId::new("el_proven");
                let mut entry = definition.elements.get(&id).unwrap().clone();
                entry.import_source_identity = Some(serde_json::from_value(json!({
                    "version":1,"sourceSha256":"a".repeat(64),"codec":"step","codecVersion":"occt-step-stream-v1",
                    "readerPolicy":"step-v1","kernelFingerprint":"0a6a1dce34181289","producerRecordId":RecordId(Uuid::from_u128(1)),
                    "bodyId":entry.body,"unitScaleBits":"3ff0000000000000","sourceAddress":{"kind":"step",
                    "rootEntityLabel":5850,"occurrencePath":[],"faceEntityLabel":3042}})).unwrap());
                definition.elements.insert(id, entry);
            }
            7 => {
                runtime.component_cache.get_mut(&id).unwrap().verified_head = None;
            }
            _ => {
                let mut records = definition.timeline.records().to_vec();
                let Operation::Known(KnownOperation::TransformBody(params)) = &mut records[1].op
                else {
                    unreachable!()
                };
                params.translate[0].expr = Some("distance".into());
                definition.timeline = onecad_core::history::Timeline::from_records(records);
            }
        }
        assert!(
            runtime
                .component_evidence_translation(id, &definition)
                .is_none(),
            "case {which}"
        );
    }
}

fn output(input: ComponentReplayInput) -> ComponentReplayOutput {
    ComponentReplayOutput {
        id: input.id,
        runtime_session: input.runtime_session,
        revision: input.revision,
        epoch: WorkerEpoch(1),
        source_fingerprint: input.source_fingerprint,
        definition: input.definition,
        meshes: BTreeMap::new(),
        budget: MeshBudget::default(),
        snapshot: SnapshotId(1),
        minted: true,
        replay_context: translation::replay_context(&input.settings, &input.imports).unwrap(),
        identity_signature: Some(BTreeMap::new()),
        quarantine: None,
        evidence_translation: input.evidence_translation,
        _retention: input._retention,
    }
}

#[test]
fn publication_checks_receipt_before_changing_definition_or_cache() {
    for which in 0..5 {
        let (mut runtime, id) = fixture(0.0);
        let input = runtime
            .prepare_component_replay(id, &runtime.runtime_session())
            .unwrap();
        assert!(input.translation_prefix.is_none());
        assert!(input.evidence_translation.is_some());
        let mut output = output(input);
        match which {
            0 => runtime.component_cache.get_mut(&id).unwrap().generation += 1,
            1 => runtime.persisted_generation += 1,
            2 => runtime.fencing.set_epoch(runtime.fencing.get().1 .0 + 1),
            3 => output.runtime_session = "stale-session".into(),
            _ => {
                runtime.fencing.bump_revision();
            }
        }
        let before = serde_json::to_value(runtime.session.document()).unwrap();
        let state = (
            runtime.persisted_generation(),
            runtime.component_generation,
            runtime.undo_depth(),
            runtime.redo_depth(),
        );
        assert!(runtime
            .commit_component_replay(output, WorkerEpoch(1))
            .is_err());
        assert_eq!(
            serde_json::to_value(runtime.session.document()).unwrap(),
            before
        );
        assert_eq!(
            (
                runtime.persisted_generation(),
                runtime.component_generation,
                runtime.undo_depth(),
                runtime.redo_depth()
            ),
            state
        );
    }
}

#[test]
fn history_proof_reserves_each_prior_and_refuses_oversized_retained_input() {
    struct Deny(std::sync::atomic::AtomicUsize);
    impl crate::resource_admission::ArchiveAdmission for Deny {
        fn reserve(
            &self,
            _: usize,
            use_: crate::resource_admission::ArchiveUse,
        ) -> Result<crate::resource_admission::Retention, String> {
            assert_eq!(use_, crate::resource_admission::ArchiveUse::Scratch);
            self.0.fetch_add(1, Ordering::SeqCst);
            Err("prior proof budget denied".into())
        }
    }
    let (mut runtime, id) = fixture(0.0);
    let definition = runtime.session.document().definitions[&id].clone();
    let denied = Arc::new(Deny(std::sync::atomic::AtomicUsize::new(0)));
    runtime.set_archive_budget(crate::resource_admission::ArchiveBudget::new(
        denied.clone(),
    ));
    assert!(runtime
        .component_evidence_translation(id, &definition)
        .is_none());
    assert!(denied.0.load(Ordering::SeqCst) > 0);
    denied.0.store(0, Ordering::SeqCst);
    let mut retained = definition.clone();
    retained.extra.insert(
        "large retained input".into(),
        json!("x".repeat((1 << 20) + 1)),
    );
    assert!(runtime
        .admit_prior_offsets(
            &retained,
            &definition,
            &runtime.component_cache[&id],
            translation::context(&definition).unwrap()
        )
        .is_none());
    assert_eq!(
        denied.0.load(Ordering::SeqCst),
        0,
        "oversized prior must fail preflight before allocation admission or fingerprinting"
    );
}

#[test]
fn evidence_translation_refuses_unknown_anchor_spatial_extension() {
    let anchor = serde_json::from_value(
        json!({"worldPoint":[1.0,2.0,3.0],"futureWorldPoint":[1.0,2.0,3.0]}),
    )
    .unwrap();
    let (runtime, id) = fixture_with_anchor(0.0, anchor);
    assert!(runtime
        .component_evidence_translation(id, &runtime.session.document().definitions[&id])
        .is_none());
}

#[test]
fn evidence_translation_refuses_unknown_local_frame_spatial_extension() {
    let anchor = serde_json::from_value(json!({"worldPoint":[1.0,2.0,3.0],"localFrame":{
        "origin":[1.0,2.0,3.0],"x":[1.0,0.0,0.0],"y":[0.0,1.0,0.0],"z":[0.0,0.0,1.0],
        "futureOrigin":[1.0,2.0,3.0]}}))
    .unwrap();
    let (runtime, id) = fixture_with_anchor(0.0, anchor);
    assert!(runtime
        .component_evidence_translation(id, &runtime.session.document().definitions[&id])
        .is_none());
}

#[test]
fn evidence_translation_refuses_added_removed_or_reowned_final_identity() {
    let (runtime, id) = fixture(0.0);
    let source = &runtime.session.document().definitions[&id];
    let proof = runtime.component_evidence_translation(id, source).unwrap();
    for which in 0..4 {
        let mut actual = source.elements.clone();
        let id = ElementId::new("el_proven");
        let mut entry = actual.get(&id).unwrap().clone();
        match which {
            0 => actual.insert(ElementId::new("el_replacement"), entry),
            1 => {
                actual.remove(&id);
            }
            2 => {
                entry.body = BodyId(Uuid::from_u128(99));
                actual.insert(id, entry);
            }
            _ => {
                entry.kind = ElementKind::Edge;
                actual.insert(id, entry);
            }
        }
        assert!(proof.validate_inventory(&source.elements, &actual).is_err());
    }
}
