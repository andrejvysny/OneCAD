use super::*;
use onecad_core::regen::WorkerElementEvidence;

fn certificate() -> onecad_core::document::import_source_identity::ImportSourceIdentity {
    serde_json::from_value(serde_json::json!({
        "version":1,"sourceSha256":"a".repeat(64),"codec":"step","codecVersion":"occt-step-stream-v1",
        "readerPolicy":"step-v1","kernelFingerprint":"0a6a1dce34181289",
        "producerRecordId":"00000000-0000-0000-0000-000000000001","bodyId":"00000000-0000-0000-0000-000000000002",
        "unitScaleBits":"3ff0000000000000","sourceAddress":{"kind":"step","rootEntityLabel":5850,"occurrencePath":[],"faceEntityLabel":3042}
    })).unwrap()
}

fn evidence() -> WorkerElementEvidence {
    WorkerElementEvidence {
        body: BodyId(Uuid::from_u128(2)),
        kind: ElementKind::Face,
        topo_key: TopoKey::new("f:1"),
        existing: Some(ElementId::new("el_same")),
        anchor: Some(anchor([1.0, 2.0, 3.0]).unwrap()),
        descriptor: Some(serde_json::json!({"current": true})),
        import_source_identity: None,
    }
}

#[test]
fn fills_only_missing_evidence_without_overwriting_frozen_intent() {
    let id = ElementId::new("el_same");
    let old = ElementEntry::new(evidence().body, ElementKind::Face)
        .with_evidence(None, Some(serde_json::json!({"frozen": [1, 2, 3]})));
    let next = merge_evidence(&id, &id, old.clone(), &evidence()).unwrap();
    assert_eq!(next.anchor, evidence().anchor);
    assert_eq!(next.descriptor, old.descriptor);
    let mut missing_descriptor = old.clone();
    missing_descriptor.descriptor = None;
    missing_descriptor.anchor = Some(anchor([7.0, 8.0, 9.0]).unwrap());
    let next = merge_evidence(&id, &id, missing_descriptor.clone(), &evidence()).unwrap();
    assert_eq!(next.anchor, missing_descriptor.anchor);
    assert_eq!(next.descriptor, evidence().descriptor);
}

#[test]
fn source_identity_cannot_appear_disappear_or_change_during_enrichment() {
    let id = ElementId::new("el_same");
    let plain = ElementEntry::new(evidence().body, ElementKind::Face);
    let mut certified = plain.clone();
    certified.import_source_identity = Some(certificate());
    let mut certified_evidence = evidence();
    certified_evidence.import_source_identity = Some(certificate());
    assert!(merge_evidence(&id, &id, plain, &certified_evidence).is_err());
    assert!(merge_evidence(&id, &id, certified.clone(), &evidence()).is_err());
    let matching = merge_evidence(&id, &id, certified.clone(), &certified_evidence).unwrap();
    assert_eq!(
        matching.import_source_identity,
        certified.import_source_identity
    );
    let mut altered = serde_json::to_value(certificate()).unwrap();
    altered["sourceSha256"] = serde_json::json!("b".repeat(64));
    certified_evidence.import_source_identity = Some(serde_json::from_value(altered).unwrap());
    assert!(merge_evidence(&id, &id, certified, &certified_evidence).is_err());
}

#[test]
fn wrong_identity_partition_kind_or_quarantine_refuses_evidence() {
    let id = ElementId::new("el_same");
    let entry = ElementEntry::new(evidence().body, ElementKind::Face);
    assert!(merge_evidence(&id, &ElementId::new("el_other"), entry.clone(), &evidence()).is_err());
    let mut wrong_body = evidence();
    wrong_body.body = BodyId(Uuid::from_u128(3));
    assert!(merge_evidence(&id, &id, entry.clone(), &wrong_body).is_err());
    let mut wrong_kind = evidence();
    wrong_kind.kind = ElementKind::Edge;
    assert!(merge_evidence(&id, &id, entry.clone(), &wrong_kind).is_err());
    let mut reserved = entry;
    reserved.binding_state =
        Some(onecad_core::document::element_index::ElementBindingState::Quarantined);
    assert!(merge_evidence(&id, &id, reserved, &evidence()).is_err());
    assert!(anchor([f64::NAN, 0.0, 0.0]).is_err());
}
