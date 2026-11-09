use onecad_core::{
    document::{
        component::{ComponentDef, ComponentDefId},
        element_index::{ElementBindingState, ElementEntry},
        modules::{ModuleId, ModuleState},
        refs::ElementKind,
        Document, NamedSelection,
    },
    edit::{DocumentSession, EditCommand},
    ids::{BodyId, DocumentId, ElementId},
};
use serde_json::json;
use uuid::Uuid;

const ID: &str = "el_00000000000000000000000000000001";

fn part(index: u128, quarantined: bool) -> ComponentDef {
    let mut part = ComponentDef::new(ComponentDefId(Uuid::from_u128(index)), "part");
    let mut entry = ElementEntry::new(BodyId(Uuid::from_u128(2)), ElementKind::Face)
        .with_evidence(None, Some(json!({"opaque": "retained"})));
    if quarantined {
        entry.binding_state = Some(ElementBindingState::Quarantined);
    }
    part.elements.insert(ElementId::new(ID), entry);
    part.bodies.fold(
        0,
        onecad_core::ids::RecordId(Uuid::from_u128(3)),
        onecad_core::document::body::BodyLifecycleEvent::Created {
            body: BodyId(Uuid::from_u128(2)),
        },
    );
    part
}

#[test]
fn embedded_edit_adapter_retains_reservations_and_rejects_a_new_colour_consumer() {
    use onecad_core::edit::AssemblyEdit;
    let mut session = session();
    mark(&mut session);
    let command = EditCommand::Assembly {
        edit: Box::new(AssemblyEdit::EditDefinition {
            id: part(10, true).id,
            command: Box::new(EditCommand::RenameBody {
                body: BodyId(Uuid::from_u128(2)),
                name: "renamed".into(),
            }),
        }),
    };
    session.apply(command).unwrap();
    session
        .document()
        .validate_topology_binding_states()
        .unwrap();
    let saved = serde_json::to_value(session.document()).unwrap();
    session.undo().unwrap();
    session.redo().unwrap();
    assert_eq!(serde_json::to_value(session.document()).unwrap(), saved);
    let error = session
        .apply(EditCommand::Assembly {
            edit: Box::new(AssemblyEdit::EditDefinition {
                id: part(10, true).id,
                command: Box::new(EditCommand::SetFaceColor {
                    body: BodyId(Uuid::from_u128(2)),
                    element_id: ElementId::new(ID),
                    color: Some([1, 2, 3, 255]),
                }),
            }),
        })
        .unwrap_err();
    assert!(error.to_string().contains("consumer outside"), "{error}");
    assert_eq!(serde_json::to_value(session.document()).unwrap(), saved);
    assert_eq!((session.undo_depth(), session.redo_depth()), (1, 0));
}

#[test]
fn make_independent_keeps_reserved_ids_in_each_definition_namespace() {
    use onecad_core::document::component::{Occurrence, OccurrenceId, RigidTransform};
    use onecad_core::edit::AssemblyEdit;
    let mut session = session();
    mark(&mut session);
    let occurrence = OccurrenceId(Uuid::from_u128(30));
    session
        .apply(EditCommand::Assembly {
            edit: Box::new(AssemblyEdit::InsertOccurrence {
                occurrence: Occurrence {
                    id: occurrence,
                    def: part(10, true).id,
                    parent: None,
                    local: RigidTransform::IDENTITY,
                    grounded: true,
                    name: "instance".into(),
                },
            }),
        })
        .unwrap();
    session
        .apply(EditCommand::Assembly {
            edit: Box::new(AssemblyEdit::MakeIndependent {
                occurrence,
                new_definition: part(20, true).id,
                name: "independent".into(),
            }),
        })
        .unwrap();
    let definitions = &session.document().definitions;
    assert_eq!(
        definitions[&part(10, true).id].elements,
        definitions[&part(20, true).id].elements
    );
    let saved = serde_json::to_value(session.document()).unwrap();
    session.undo().unwrap();
    assert!(!session
        .document()
        .definitions
        .contains_key(&part(20, true).id));
    session.redo().unwrap();
    assert_eq!(serde_json::to_value(session.document()).unwrap(), saved);
    let reopened = onecad_core::io::document_io::document_from_value(saved).unwrap();
    reopened.validate_topology_binding_states().unwrap();
    assert!(reopened.definitions[&part(20, true).id]
        .elements
        .has_quarantined());
}

fn session() -> DocumentSession {
    let mut document = Document::new(DocumentId(Uuid::from_u128(1)));
    document
        .definitions
        .insert(part(10, false).id, part(10, false));
    DocumentSession::new(document)
}

fn module(present: bool) -> EditCommand {
    EditCommand::SetModuleState {
        module: ModuleId::parse("org.example.future").unwrap(),
        state: present.then(|| ModuleState::new(1, json!({"indirect": 42}))),
    }
}

fn mark(session: &mut DocumentSession) {
    let candidate = part(10, true);
    session
        .adopt_component_replay(candidate.id, candidate)
        .unwrap();
}

#[test]
fn independent_definition_can_reuse_local_identity_but_owner_cannot_consume_it() {
    let mut document = session().into_document();
    document
        .definitions
        .insert(part(10, true).id, part(10, true));
    let mut independent = part(20, false);
    independent.named_selections.push(NamedSelection {
        name: "independent face".into(),
        members: vec![ElementId::new(ID)],
        extra: Default::default(),
    });
    document.definitions.insert(independent.id, independent);
    document.validate_topology_binding_states().unwrap();
    document
        .definitions
        .get_mut(&part(10, true).id)
        .unwrap()
        .named_selections
        .push(NamedSelection {
            name: "stale face".into(),
            members: vec![ElementId::new(ID)],
            extra: Default::default(),
        });
    assert!(document.validate_topology_binding_states().is_err());
}

#[test]
fn unknown_state_without_literal_identity_refuses_and_rolls_back_edit() {
    let mut session = session();
    mark(&mut session);
    let before = serde_json::to_value(session.document()).unwrap();
    assert!(session.apply(module(true)).is_err());
    assert_eq!(serde_json::to_value(session.document()).unwrap(), before);
    assert_eq!((session.undo_depth(), session.redo_depth()), (0, 0));
}

#[test]
fn undo_cannot_restore_hidden_consumer_after_derived_quarantine() {
    let mut session = session();
    session.apply(module(true)).unwrap();
    session.apply(module(false)).unwrap();
    mark(&mut session);
    let before = serde_json::to_value(session.document()).unwrap();
    assert!(session.undo().is_none());
    assert_eq!(serde_json::to_value(session.document()).unwrap(), before);
    assert_eq!((session.undo_depth(), session.redo_depth()), (2, 0));
    session
        .document()
        .validate_topology_binding_states()
        .unwrap();
}

#[test]
fn redo_cannot_restore_hidden_consumer_after_derived_quarantine() {
    let mut session = session();
    session.apply(module(true)).unwrap();
    session.undo().unwrap();
    mark(&mut session);
    let before = serde_json::to_value(session.document()).unwrap();
    assert!(session.redo().is_err());
    assert_eq!(serde_json::to_value(session.document()).unwrap(), before);
    assert_eq!((session.undo_depth(), session.redo_depth()), (0, 1));
    session
        .document()
        .validate_topology_binding_states()
        .unwrap();
}

#[test]
fn mixed_batch_adoption_is_atomic_when_one_identity_has_a_consumer() {
    let mut session = session();
    let mut candidate = part(10, true);
    let second = ElementId::new("el_00000000000000000000000000000002");
    candidate.elements.insert(
        second.clone(),
        candidate.elements.get(&ElementId::new(ID)).unwrap().clone(),
    );
    candidate.named_selections.push(NamedSelection {
        name: "active consumer".into(),
        members: vec![second],
        extra: Default::default(),
    });
    let before = serde_json::to_value(session.document()).unwrap();
    assert!(session
        .adopt_component_replay(candidate.id, candidate)
        .is_err());
    assert_eq!(serde_json::to_value(session.document()).unwrap(), before);
    assert_eq!(session.undo_depth(), 0);
}

#[test]
fn region_and_repair_extensions_cannot_hide_indirect_consumers() {
    use onecad_core::{
        ids::{RegionId, SketchId},
        sketch::{RegionInfo, Sketch, SketchAttachment, WorldPlane},
    };
    let mut document = session().into_document();
    let definition = part(10, true);
    document.definitions.insert(definition.id, definition);
    document.validate_topology_binding_states().unwrap();
    let mut sketch = Sketch::new(
        SketchId(Uuid::from_u128(40)),
        "sketch",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    sketch.regions.push(RegionInfo {
        id: RegionId::new("region"),
        outer: vec![],
        holes: vec![],
        extra: serde_json::from_value(json!({"vendorEncodedRef": "indirect-payload"})).unwrap(),
    });
    document
        .definitions
        .get_mut(&part(10, true).id)
        .unwrap()
        .sketches
        .insert(sketch.id, sketch);
    assert!(document.validate_topology_binding_states().is_err());
    document
        .definitions
        .get_mut(&part(10, true).id)
        .unwrap()
        .sketches
        .clear();
    for extension in [
        json!({"candidates": [{"topoKey":"f:1","score":1.0,"margin":1.0,"worldPos":[0,0,0],"summary":"candidate","vendorEncodedRef":"indirect-payload"}]}),
        json!({"anchor":{"worldPoint":[0,0,0],"vendorEncodedRef":"indirect-payload"}}),
    ] {
        let mut repair = json!({"stepIndex":0,"refId":"future","ladderFailed":"descriptor","reason":"no-candidates"});
        repair
            .as_object_mut()
            .unwrap()
            .extend(extension.as_object().unwrap().clone());
        document
            .definitions
            .get_mut(&part(10, true).id)
            .unwrap()
            .repair = serde_json::from_value(json!([repair])).unwrap();
        assert!(document.validate_topology_binding_states().is_err());
    }
}
