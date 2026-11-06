use super::*;
use onecad_core::regen::ElementBinding;

fn cert(body: BodyId) -> Value {
    json!({"version":1,"sourceSha256":"a".repeat(64),"codec":"step",
        "codecVersion":"occt-step-stream-v1","readerPolicy":"step-v1","kernelFingerprint":"0a6a1dce34181289",
        "producerRecordId":"00000000-0000-0000-0000-000000000001", "bodyId":body_id_wire(body),
        "unitScaleBits":"3ff0000000000000", "sourceAddress":{"kind":"step",
            "rootEntityLabel":5850,"occurrencePath":[],"faceEntityLabel":3042}})
}

#[test]
fn acquired_identity_is_strict_and_survives_fenced_bind() {
    let body = BodyId(Uuid::from_u128(2));
    let input = json!({"ids":[{"topoKey":"f:2","bodyId":body_id_wire(body),"kind":"face","importSourceIdentity":cert(body)}]});
    let evidence = parse_acquire_evidence(&input, body).unwrap().remove(0);
    let request = BindElementIdsRequest {
        snapshot_id: SnapshotId(3),
        bindings: vec![ElementBinding {
            element_id: ElementId::new("el_test"),
            topo_key: evidence.topo_key,
            body,
            kind: evidence.kind,
            anchor: None,
            import_source_identity: evidence.import_source_identity,
        }],
    };
    let wire = bind_element_ids_args(&request);
    assert_eq!(wire["bindings"][0]["importSourceIdentity"], cert(body));
    assert!(validate_import_source_identities(&wire).is_ok());
    let mut invalid = input;
    invalid["ids"][0]["importSourceIdentity"] = Value::Null;
    assert!(parse_acquire_evidence(&invalid, body).is_err());
}
