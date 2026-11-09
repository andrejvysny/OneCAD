//! Native histories retain reservations without treating imported geometry as history.
use onecad_core::{
    document::{
        component::{ComponentDef, ComponentDefId},
        element_index::{ElementBindingState, ElementEntry},
        refs::ElementKind,
        Document,
    },
    ids::ElementId,
    io::{container::ContainerReader, document_io::document_from_value},
};
use serde_json::{json, Value};
use uuid::Uuid;

const RESERVED: &str = "el_00000000000000000000000000000001";

fn standalone(seed: u128, reserved: bool) -> Document {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/qualification/assembly-public-10x20/native-compact-10x20.onecad");
    let loaded = ContainerReader::open(&source).unwrap();
    let id = ComponentDefId(Uuid::from_u128(seed));
    let mut definition: ComponentDef = loaded.document().definitions[&id].clone();
    let element = ElementId::new(RESERVED);
    assert!(definition.elements.get(&element).is_none());
    if reserved {
        // Synthetic orphan metadata tests the closed consumer proof; actual creation
        // still requires a complete current-worker NeedsRepair answer in onecad-app.
        let mut marker = ElementEntry::new(definition.bodies.bodies()[0].id, ElementKind::Face)
            .with_evidence(None, Some(json!({"syntheticOrphanEvidence": true})));
        marker.binding_state = Some(ElementBindingState::Quarantined);
        definition.elements.insert(element, marker);
    }
    let mut document = Document::new(loaded.document().id);
    document.definitions.insert(id, definition);
    document
}

fn record<'a>(value: &'a mut Value, operation: &str) -> &'a mut Value {
    value["definitions"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()["timeline"]["records"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|record| record["opType"] == operation)
        .unwrap()
}

fn arc(value: &mut Value) -> &mut Value {
    value["definitions"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()["timeline"]["records"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .filter(|record| record["opType"] == "Sketch")
        .flat_map(|record| record["params"]["entities"].as_array_mut().unwrap())
        .find(|entity| entity["type"] == "Arc")
        .unwrap()
}

#[test]
fn native_prusa_arc_and_revolve_histories_admit_retained_orphan_evidence() {
    for seed in [0xd205, 0xd206] {
        let document = standalone(seed, true);
        let before = serde_json::to_value(&document).unwrap();
        document.validate_topology_binding_states().unwrap();
        let reopened = document_from_value(before.clone()).unwrap();
        assert_eq!(serde_json::to_value(reopened).unwrap(), before);
    }
}

#[test]
fn native_arc_refuses_opaque_fields_and_malformed_values() {
    for (key, bad) in [
        ("future", json!({"encodedRef": "opaque"})),
        ("startAngle", json!("NaN")),
        ("endAngle", Value::Null),
        ("radius", json!([])),
        ("center", json!([0])),
        ("centerRef", json!(RESERVED)),
        ("construction", json!(0)),
        ("type", json!("FutureArc")),
    ] {
        let mut value = serde_json::to_value(standalone(0xd205, true)).unwrap();
        arc(&mut value)[key] = bad;
        assert!(
            document_from_value(value).is_err(),
            "accepted Arc field {key}"
        );
    }
}

#[test]
fn native_revolve_refuses_opaque_fields_and_nonlocal_axes() {
    for (path, bad) in [
        ("/axis", Value::Null),
        (
            "/axis",
            json!({"kind":"edge", "bodyId":Uuid::from_u128(1), "edgeId":RESERVED}),
        ),
        ("/axis", json!({"kind":"futureAxis", "encodedRef":"opaque"})),
        ("/axis/future", json!({"encodedRef":"opaque"})),
        ("/axis/lineId", json!("invalid-uuid")),
        ("/profile/future", json!({"encodedRef":"opaque"})),
        ("/future", json!({"encodedRef":"opaque"})),
    ] {
        let mut value = serde_json::to_value(standalone(0xd206, true)).unwrap();
        let params = &mut record(&mut value, "Revolve")["params"];
        let (parent, key) = path.rsplit_once('/').unwrap();
        params.pointer_mut(parent).unwrap()[key] = bad;
        assert!(
            document_from_value(value).is_err(),
            "accepted Revolve path {path}"
        );
    }
}

#[test]
fn native_revolve_cannot_consume_reserved_identity() {
    let mut value = serde_json::to_value(standalone(0xd205, true)).unwrap();
    record(&mut value, "Revolve")["params"]["profile"]["regionId"] = json!(RESERVED);
    let error = document_from_value(value).unwrap_err();
    assert!(error.to_string().contains("consumer outside"), "{error}");
}

#[test]
fn schema5_revolve_ingress_refuses_discarded_data_before_any_reservation() {
    let mut value = serde_json::to_value(standalone(0xd205, false)).unwrap();
    record(&mut value, "Revolve")["params"]["angleDeg"]["future"] = json!({"encodedRef":"opaque"});
    let error = document_from_value(value).unwrap_err();
    assert!(error.to_string().contains("would be discarded"), "{error}");
}

fn imported_sibling(document: &mut Document) {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/qualification/assembly-public-10x20/native-compact-10x20.onecad");
    let loaded = ContainerReader::open(&source).unwrap();
    let id = ComponentDefId(Uuid::from_u128(0xd200));
    document
        .definitions
        .insert(id, loaded.document().definitions[&id].clone());
}

#[test]
fn native_reservation_allows_valid_independent_imported_sibling() {
    let mut document = standalone(0xd205, true);
    imported_sibling(&mut document);
    let value = serde_json::to_value(document).unwrap();
    let reopened = document_from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(reopened).unwrap(), value);
}

#[test]
fn native_reservation_refuses_imports_in_owner_even_when_suppressed() {
    use onecad_core::document::record::{KnownOperation, Operation};
    for suppressed in [false, true] {
        let mut document = standalone(0xd205, true);
        imported_sibling(&mut document);
        let import = document
            .definitions
            .remove(&ComponentDefId(Uuid::from_u128(0xd200)))
            .unwrap();
        let mut record = import
            .timeline
            .records()
            .iter()
            .find(|record| matches!(record.op, Operation::Known(KnownOperation::ImportStep(_))))
            .unwrap()
            .clone();
        record.suppressed = suppressed;
        document
            .definitions
            .get_mut(&ComponentDefId(Uuid::from_u128(0xd205)))
            .unwrap()
            .timeline
            .insert_at_cursor(record);
        assert!(document.validate_topology_binding_states().is_err());
    }
}

#[test]
fn imported_sibling_cannot_hide_opaque_or_invalid_parameters() {
    for bad in [
        json!({"future":{"encodedRef":"opaque"}}),
        json!({"sourceSha256":"invalid"}),
        json!({"unitScale":{"value":0}}),
    ] {
        let mut document = standalone(0xd205, true);
        imported_sibling(&mut document);
        let mut value = serde_json::to_value(document).unwrap();
        let sibling =
            &mut value["definitions"][ComponentDefId(Uuid::from_u128(0xd200)).to_string()];
        let records = sibling["timeline"]["records"].as_array_mut().unwrap();
        let import = records
            .iter_mut()
            .find(|record| record["opType"] == "ImportStep")
            .unwrap();
        import["params"]
            .as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        assert!(document_from_value(value).is_err());
    }
    let mut document = standalone(0xd205, true);
    imported_sibling(&mut document);
    let mut value = serde_json::to_value(document).unwrap();
    let sibling = &mut value["definitions"][ComponentDefId(Uuid::from_u128(0xd200)).to_string()];
    sibling["timeline"]["records"][0]["opType"] = json!("FutureImport");
    assert!(document_from_value(value).is_err());
}

#[test]
fn unreserved_root_import_stays_distinct_from_native_reserved_owner() {
    let mut document = standalone(0xd205, true);
    imported_sibling(&mut document);
    let imported = document
        .definitions
        .remove(&ComponentDefId(Uuid::from_u128(0xd200)))
        .unwrap();
    document.timeline = imported.timeline;
    document.bodies = imported.bodies;
    document.elements = imported.elements;
    let value = serde_json::to_value(document).unwrap();
    let reopened = document_from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(reopened).unwrap(), value);
}
