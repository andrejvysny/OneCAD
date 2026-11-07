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
    assert!(s.adopt_component_replay(id, part));
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
    assert!(s.adopt_component_replay(id, part));
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
    assert!(!s.adopt_component_replay(part.id, part));
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
    assert!(s.adopt_component_replay(id, part));
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
    assert!(s.adopt_component_replay(id, part));
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
    assert!(s.adopt_component_replay(id, part));
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
    assert!(s.adopt_component_replay(source, part));
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
