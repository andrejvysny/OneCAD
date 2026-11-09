//! Quarantine is critical persisted identity state, never an ignorable hint.
use onecad_core::{
    document::{element_index::ElementEntry, refs::ElementKind, Document},
    ids::{BodyId, DocumentId, ElementId},
    io::document_io::{document_from_value, serialize_document},
};
use serde_json::{json, Value};
use uuid::Uuid;

const ELEMENT: &str = "el_00000000000000000000000000000001";
const DEFINITION: &str = "00000000-0000-0000-0000-00000000000a";

fn value(version: u32, state: Value) -> Value {
    let mut document = Document::new(DocumentId(Uuid::from_u128(1)));
    let mut definition = onecad_core::document::component::ComponentDef::new(
        onecad_core::document::component::ComponentDefId(Uuid::from_u128(10)),
        "part",
    );
    definition.elements.insert(
        ElementId::new(ELEMENT),
        ElementEntry::new(BodyId(Uuid::from_u128(2)), ElementKind::Face)
            .with_evidence(None, Some(json!({"opaque": "retained"}))),
    );
    document.definitions.insert(definition.id, definition);
    let mut value = serde_json::to_value(document).unwrap();
    value["schemaVersion"] = json!(version);
    value["definitions"][DEFINITION]["elements"][ELEMENT]["topologyBindingState"] = state;
    value
}

#[test]
fn schema_five_quarantine_retains_evidence_without_rebinding() {
    let source = value(5, json!("quarantined"));
    let document = document_from_value(source.clone()).unwrap();
    let entry = document
        .definitions
        .values()
        .next()
        .unwrap()
        .elements
        .get(&ElementId::new(ELEMENT))
        .unwrap();
    assert!(!entry.has_rebind_evidence());
    assert_eq!(entry.descriptor, Some(json!({"opaque": "retained"})));
    let roundtrip: Value = serde_json::from_slice(&serialize_document(&document).unwrap()).unwrap();
    assert_eq!(roundtrip, source);
}

#[test]
fn legacy_schema_cannot_silently_ignore_quarantine() {
    let source = value(4, json!("quarantined"));
    assert!(document_from_value(source.clone()).is_err());
    assert!(serde_json::from_value::<Document>(source).is_err());
}

#[test]
fn unknown_or_null_state_is_refused() {
    for state in [
        Value::Null,
        json!("active"),
        json!("future-state"),
        json!({}),
        json!(42),
    ] {
        assert!(document_from_value(value(5, state)).is_err());
    }
}

#[test]
fn schema_five_still_checks_critical_import_fields_in_opaque_data() {
    let mut source = value(5, json!("quarantined"));
    source["settings"]["opaqueFuture"] = json!({"importSourceIdentity": null});
    assert!(document_from_value(source).is_err());
}

#[test]
fn quarantine_requires_retained_uncertified_face_evidence() {
    for key in ["kind", "descriptor"] {
        let mut source = value(5, json!("quarantined"));
        let entry = &mut source["definitions"][DEFINITION]["elements"][ELEMENT];
        entry[key] = if key == "kind" {
            json!("edge")
        } else {
            Value::Null
        };
        assert!(document_from_value(source).is_err());
    }
}

#[test]
fn discarded_unknown_field_is_refused_before_a_consumer_can_disappear() {
    let mut source = value(5, json!("quarantined"));
    source["settings"]["futureConsumer"] = json!({"encoded": "not-a-literal-element-id"});
    let error = document_from_value(source).unwrap_err().to_string();
    assert!(error.contains("futureConsumer"), "{error}");
}

#[test]
fn ordinary_legacy_explicit_defaults_keep_existing_decode_behavior() {
    let mut source = value(4, json!("quarantined"));
    source["definitions"][DEFINITION]["elements"][ELEMENT]
        .as_object_mut()
        .unwrap()
        .remove("topologyBindingState");
    source["settings"]["occtFingerprint"] = Value::Null;
    source["rootDefinition"]["namedSelections"] = json!([]);
    let mut migrated = source.clone();
    onecad_core::io::migrate::MigrationRegistry::current()
        .apply(&mut migrated, 4, 5)
        .unwrap();
    let document = document_from_value(migrated).unwrap();
    assert_eq!(document.schema_version, 5);
    assert!(document.settings.occt_fingerprint.is_none());
    assert_eq!(source["schemaVersion"], 4);
}

#[test]
fn root_quarantine_refuses_decode_and_save_without_replacing_original() {
    use onecad_core::io::container::{ContainerCaches, ContainerWriter, SaveMeta};
    let mut document = document_from_value(value(5, json!("quarantined"))).unwrap();
    document.elements =
        std::mem::take(&mut document.definitions.values_mut().next().unwrap().elements);
    document.definitions.clear();
    let source = serde_json::to_value(&document).unwrap();
    assert!(document_from_value(source.clone()).is_err());
    assert!(serde_json::from_value::<Document>(source).is_err());
    assert!(serialize_document(&document).is_err());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original.onecad");
    let meta = SaveMeta {
        app_version: "test".into(),
        occt_fingerprint: None,
        created: "2026-11-09T00:00:00Z".into(),
        modified: "2026-11-09T00:00:00Z".into(),
    };
    ContainerWriter::save(
        &path,
        &Document::new(document.id),
        &ContainerCaches::none(),
        &meta,
    )
    .unwrap();
    let original = std::fs::read(&path).unwrap();
    let error =
        ContainerWriter::save(&path, &document, &ContainerCaches::none(), &meta).unwrap_err();
    assert!(error.to_string().contains("embedded definitions"));
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn marker_free_decode_cannot_discard_fields_before_a_later_repair() {
    let mut source = value(5, json!("quarantined"));
    source["definitions"][DEFINITION]["elements"][ELEMENT]
        .as_object_mut()
        .unwrap()
        .remove("topologyBindingState");
    source["settings"]["occtFingerprint"] = Value::Null;
    source["definitions"][DEFINITION]["namedSelections"] = json!([]);
    document_from_value(source.clone()).unwrap();
    for unknown in [Value::Null, json!({}), json!([]), json!(false)] {
        let mut unsupported = source.clone();
        unsupported["settings"]["futureConsumer"] = unknown;
        let error = document_from_value(unsupported).unwrap_err().to_string();
        assert!(error.contains("futureConsumer"), "{error}");
    }
    source["settings"]["referenceLocked"] = json!(false);
    assert!(document_from_value(source).is_err());
}

#[test]
fn known_entity_default_does_not_allow_an_unknown_neighbour() {
    use onecad_core::{
        ids::SketchId,
        math::Vec2,
        sketch::{Sketch, SketchAttachment, SketchEntity, WorldPlane},
    };
    let mut document = Document::new(DocumentId(Uuid::from_u128(1)));
    let mut sketch = Sketch::new(
        SketchId(Uuid::from_u128(20)),
        "sketch",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    sketch
        .add_entity(SketchEntity::Point {
            id: onecad_core::ids::EntityId(Uuid::from_u128(21)),
            at: Vec2::new_unchecked(0.0, 0.0),
            construction: false,
            reference_locked: false,
        })
        .unwrap();
    let id = sketch.id.to_string();
    document.sketches.insert(sketch.id, sketch);
    let mut source = serde_json::to_value(document).unwrap();
    source["rootDefinition"]["sketches"][&id]["entities"][0]["referenceLocked"] = json!(false);
    document_from_value(source.clone()).unwrap();
    for unknown in [Value::Null, json!({}), json!([]), json!(false)] {
        let mut unsupported = source.clone();
        unsupported["rootDefinition"]["sketches"][&id]["entities"][0]["futureConsumer"] = unknown;
        assert!(document_from_value(unsupported).is_err());
    }
}
