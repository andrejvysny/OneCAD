use super::*;
use crate::document::{body::BodyLifecycleEvent, refs::ElementKind};
use crate::edit::AssemblyEdit;
use crate::ids::{DocumentId, ElementId};
use uuid::Uuid;

fn definition(index: u128) -> ComponentDef {
    let mut part = ComponentDef::new(ComponentDefId(Uuid::from_u128(index)), "native");
    let body = BodyId(Uuid::from_u128(index + 100));
    part.bodies.fold(
        0,
        RecordId(Uuid::from_u128(index + 200)),
        BodyLifecycleEvent::Created { body },
    );
    part
}
fn session() -> DocumentSession {
    let mut doc = Document::new(DocumentId(Uuid::from_u128(1)));
    for index in [10, 20] {
        let part = definition(index);
        doc.definitions.insert(part.id, part);
    }
    DocumentSession::new(doc)
}
fn edit(s: &mut DocumentSession, index: u128) {
    s.apply(EditCommand::Assembly {
        edit: Box::new(AssemblyEdit::EditDefinition {
            id: ComponentDefId(Uuid::from_u128(index)),
            command: Box::new(EditCommand::RenameBody {
                body: BodyId(Uuid::from_u128(index + 100)),
                name: "edited".into(),
            }),
        }),
    })
    .unwrap();
}
fn enrich(s: &mut DocumentSession, index: u128) {
    let id = ComponentDefId(Uuid::from_u128(index));
    let mut part = s.document().definitions[&id].clone();
    part.elements.insert(
        ElementId::new(format!("el_{index}")),
        ElementEntry::new(BodyId(Uuid::from_u128(index + 100)), ElementKind::Face),
    );
    s.adopt_component_replay(id, part).unwrap();
}
fn has_id(s: &DocumentSession, index: u128) -> bool {
    s.document().definitions[&ComponentDefId(Uuid::from_u128(index))]
        .elements
        .contains(&ElementId::new(format!("el_{index}")))
}
#[test]
fn redo_restores_late_replay_identity_without_an_extra_history_step() {
    let mut s = session();
    edit(&mut s, 10);
    enrich(&mut s, 10);
    assert_eq!(s.undo_depth(), 1);
    for _ in 0..2 {
        s.undo().unwrap();
        assert!(!has_id(&s, 10));
        s.redo().unwrap().unwrap();
        assert!(has_id(&s, 10));
        assert_eq!(s.undo_depth(), 1);
    }
}
#[test]
fn changed_inputs_refuse_redo_atomically_and_preserve_new_opaque_state() {
    let mut s = session();
    edit(&mut s, 10);
    enrich(&mut s, 10);
    s.undo().unwrap();
    let id = ComponentDefId(Uuid::from_u128(10));
    let mut part = s.document().definitions[&id].clone();
    part.extra
        .insert("opaque".into(), serde_json::json!({"newer":true}));
    s.adopt_component_replay(id, part).unwrap();
    let before = serde_json::to_vec(s.document()).unwrap();
    assert!(s
        .redo()
        .unwrap_err()
        .to_string()
        .contains("component state changed while undone"));
    assert_eq!(serde_json::to_vec(s.document()).unwrap(), before);
    assert_eq!((s.undo_depth(), s.redo_depth()), (0, 1));
}
#[test]
fn a_closed_group_retains_both_definition_deltas() {
    let mut s = session();
    s.begin_transaction("two definitions");
    edit(&mut s, 10);
    edit(&mut s, 20);
    s.end_transaction().unwrap();
    enrich(&mut s, 10);
    enrich(&mut s, 20);
    s.undo().unwrap();
    assert!(!has_id(&s, 10) && !has_id(&s, 20));
    s.redo().unwrap().unwrap();
    assert!(has_id(&s, 10) && has_id(&s, 20));
    assert_eq!(s.undo_depth(), 1);
}
#[test]
fn replay_cannot_be_adopted_between_open_transaction_edits() {
    let mut s = session();
    s.begin_transaction("open");
    let part = s.document().definitions[&ComponentDefId(Uuid::from_u128(10))].clone();
    assert!(s.adopt_component_replay(part.id, part).is_err());
}

#[test]
fn replay_minting_while_undone_refuses_redo_without_duplicate_identity() {
    let mut s = session();
    edit(&mut s, 10);
    enrich(&mut s, 10);
    s.undo().unwrap();
    let id = ComponentDefId(Uuid::from_u128(10));
    let mut part = s.document().definitions[&id].clone();
    part.elements.insert(
        ElementId::new("el_newer"),
        ElementEntry::new(BodyId(Uuid::from_u128(110)), ElementKind::Face),
    );
    s.adopt_component_replay(id, part).unwrap();
    let before = serde_json::to_vec(s.document()).unwrap();
    assert!(s
        .redo()
        .unwrap_err()
        .to_string()
        .contains("component state changed while undone"));
    assert_eq!(serde_json::to_vec(s.document()).unwrap(), before);
    assert!(!has_id(&s, 10));
    assert_eq!((s.undo_depth(), s.redo_depth()), (0, 1));
}

#[test]
fn retained_delta_does_not_clone_unchanged_identity_entries() {
    let mut s = session();
    let id = ComponentDefId(Uuid::from_u128(10));
    let mut part = s.document().definitions[&id].clone();
    for index in 0..1000 {
        part.elements.insert(
            ElementId::new(format!("el_existing_{index}")),
            ElementEntry::new(BodyId(Uuid::from_u128(110)), ElementKind::Face),
        );
    }
    s.adopt_component_replay(id, part).unwrap();
    edit(&mut s, 10);
    enrich(&mut s, 10);
    let states = s
        .capture_component_identities(s.undo.history_txn(HistoryDirection::Undo).unwrap())
        .unwrap();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].upsert.len(), 1);
    assert!(states[0].removed.is_empty());
}

#[test]
fn changed_body_registry_while_undone_refuses_redo_atomically() {
    let mut s = session();
    edit(&mut s, 10);
    enrich(&mut s, 10);
    s.undo().unwrap();
    let id = ComponentDefId(Uuid::from_u128(10));
    let mut part = s.document().definitions[&id].clone();
    part.bodies = BodyRegistry::new();
    s.adopt_component_replay(id, part).unwrap();
    let before = serde_json::to_vec(s.document()).unwrap();
    assert!(s
        .redo()
        .unwrap_err()
        .to_string()
        .contains("component state changed while undone"));
    assert_eq!(serde_json::to_vec(s.document()).unwrap(), before);
    assert!(!has_id(&s, 10));
    assert_eq!((s.undo_depth(), s.redo_depth()), (0, 1));
}

#[test]
fn copied_definition_redo_refuses_new_source_identity_atomically() {
    copied_definition_change_refuses(false);
}
#[test]
fn copied_definition_redo_refuses_changed_source_body_atomically() {
    copied_definition_change_refuses(true);
}
fn copied_definition_change_refuses(change_body: bool) {
    use onecad_assembly::{
        math::RigidTransform,
        occurrence::{Occurrence, OccurrenceId},
    };
    let mut s = session();
    enrich(&mut s, 10);
    let occurrence = OccurrenceId(Uuid::from_u128(30));
    s.apply(EditCommand::Assembly {
        edit: Box::new(AssemblyEdit::InsertOccurrence {
            occurrence: Occurrence {
                id: occurrence,
                def: ComponentDefId(Uuid::from_u128(10)),
                parent: None,
                local: RigidTransform::IDENTITY,
                grounded: false,
                name: "repeat".into(),
            },
        }),
    })
    .unwrap();
    let copied = ComponentDefId(Uuid::from_u128(40));
    s.apply(EditCommand::Assembly {
        edit: Box::new(AssemblyEdit::MakeIndependent {
            occurrence,
            new_definition: copied,
            name: "copy".into(),
        }),
    })
    .unwrap();
    s.undo().unwrap();
    let source = ComponentDefId(Uuid::from_u128(10));
    let mut part = s.document().definitions[&source].clone();
    if change_body {
        part.bodies = BodyRegistry::new();
    } else {
        part.elements.insert(
            ElementId::new("el_new_source"),
            ElementEntry::new(BodyId(Uuid::from_u128(110)), ElementKind::Face),
        );
    }
    s.adopt_component_replay(source, part).unwrap();
    let before = serde_json::to_vec(s.document()).unwrap();
    assert!(s
        .redo()
        .unwrap_err()
        .to_string()
        .contains("copied component identities changed"));
    assert_eq!(serde_json::to_vec(s.document()).unwrap(), before);
    assert!(!s.document().definitions.contains_key(&copied));
    assert_eq!((s.undo_depth(), s.redo_depth()), (1, 1));
}

fn appended_translation_with_replay_outputs() -> DocumentSession {
    use crate::document::record::{
        KnownOperation, OpaqueOperation, Operation, TransformBodyParams,
    };
    use crate::document::variables::Scalar;
    let mut s = session();
    let id = ComponentDefId(Uuid::from_u128(10));
    let body = BodyId(Uuid::from_u128(110));
    let mut part = s.document().definitions[&id].clone();
    let mut producer = OperationRecord::new(
        RecordId(Uuid::from_u128(210)),
        0,
        "Frozen producer",
        Operation::Opaque(OpaqueOperation {
            raw: serde_json::from_value(
                serde_json::json!({"opType":"FutureSolid","future":{"keep":7}}),
            )
            .unwrap(),
        }),
    );
    producer.outputs = vec![body];
    part.timeline = crate::history::Timeline::from_records(vec![producer]);
    s.adopt_component_replay(id, part).unwrap();
    let record = OperationRecord::new(
        RecordId(Uuid::from_u128(211)),
        1,
        "Translate",
        Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
            targets: vec![body],
            translate: [Scalar::new(1.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
            copy: false,
            extra: Default::default(),
        })),
    );
    s.apply(EditCommand::Assembly {
        edit: Box::new(AssemblyEdit::EditDefinition {
            id,
            command: Box::new(EditCommand::AddOperation {
                record,
                at_cursor: true,
            }),
        }),
    })
    .unwrap();
    let mut replay = s.document().definitions[&id].clone();
    replay.timeline.set_record_outputs(1, vec![body]);
    replay.elements.insert(
        ElementId::new("el_output_test"),
        ElementEntry::new(body, ElementKind::Face),
    );
    s.adopt_component_replay(id, replay).unwrap();
    s
}

#[test]
fn redo_restores_observed_transform_outputs_and_opaque_prefix() {
    let mut s = appended_translation_with_replay_outputs();
    let expected = serde_json::to_value(s.document()).unwrap();
    s.undo().unwrap();
    s.redo().unwrap().unwrap();
    assert_eq!(serde_json::to_value(s.document()).unwrap(), expected);
    for _ in 0..2 {
        s.undo().unwrap();
        s.redo().unwrap().unwrap();
        assert_eq!(serde_json::to_value(s.document()).unwrap(), expected);
    }
}

#[test]
fn transform_output_redo_restores_without_an_element_delta() {
    let mut s = appended_translation_with_replay_outputs();
    let id = ComponentDefId(Uuid::from_u128(10));
    let mut part = s.document().definitions[&id].clone();
    part.elements = ElementIndex::new();
    s.adopt_component_replay(id, part).unwrap();
    let expected = serde_json::to_vec(s.document()).unwrap();
    s.undo().unwrap();
    s.redo().unwrap().unwrap();
    assert_eq!(serde_json::to_vec(s.document()).unwrap(), expected);
}

#[test]
fn transform_output_redo_rejects_changed_forward_outputs_atomically() {
    let mut s = appended_translation_with_replay_outputs();
    s.undo().unwrap();
    let (mut txn, states) = s.undo.pop_redo_with_components().unwrap();
    let EditCommand::Assembly { edit } = &mut txn.edits[0].forward else {
        unreachable!()
    };
    let AssemblyEdit::EditDefinition { command, .. } = edit.as_mut() else {
        unreachable!()
    };
    let EditCommand::AddOperation { record, .. } = command.as_mut() else {
        unreachable!()
    };
    record.outputs = vec![BodyId(Uuid::from_u128(999))];
    s.undo.push_undone_with_components(txn, states);
    let before = serde_json::to_vec(s.document()).unwrap();
    assert!(s.redo().is_err());
    assert_eq!(serde_json::to_vec(s.document()).unwrap(), before);
    assert_eq!((s.undo_depth(), s.redo_depth()), (0, 1));
}

#[test]
fn transform_output_redo_rejects_changed_owner_and_opaque_prefix() {
    for opaque in [false, true] {
        let mut s = appended_translation_with_replay_outputs();
        s.undo().unwrap();
        let id = ComponentDefId(Uuid::from_u128(10));
        let mut part = s.document().definitions[&id].clone();
        if opaque {
            let mut records = part.timeline.records().to_vec();
            let crate::document::record::Operation::Opaque(op) = &mut records[0].op else {
                unreachable!()
            };
            op.raw
                .insert("future".into(), serde_json::json!({"keep":99}));
            part.timeline = crate::history::Timeline::from_records(records);
        } else {
            let mut value = serde_json::to_value(&part.bodies).unwrap();
            value["bodies"][0]["createdBy"] = serde_json::json!(RecordId(Uuid::from_u128(999)));
            part.bodies = serde_json::from_value(value).unwrap();
        }
        s.adopt_component_replay(id, part).unwrap();
        let before = serde_json::to_vec(s.document()).unwrap();
        assert!(s.redo().is_err());
        assert_eq!(serde_json::to_vec(s.document()).unwrap(), before);
        assert_eq!((s.undo_depth(), s.redo_depth()), (0, 1));
    }
}
