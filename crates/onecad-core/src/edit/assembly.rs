//! Assembly edits applied by the document's single undoable session.
use onecad_assembly::{
    math::RigidTransform,
    occurrence::{ComponentDefId, Occurrence, OccurrenceId, MAX_OCCURRENCES},
};
use serde::{Deserialize, Serialize};

use super::{DocumentSession, EditCommand};
use crate::{
    document::{component::ComponentDef, Document},
    error::DomainError,
};

/// Flat embedded-definition and occurrence authoring operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AssemblyEdit {
    CreateDefinition {
        id: ComponentDefId,
        name: String,
    },
    /// Embeds a snapshot of the root part without rewriting its local IDs.
    CopyRoot {
        id: ComponentDefId,
        name: String,
    },
    EditDefinition {
        id: ComponentDefId,
        command: Box<EditCommand>,
    },
    InsertOccurrence {
        occurrence: Occurrence,
    },
    SetPose {
        occurrence: OccurrenceId,
        pose: RigidTransform,
    },
    SetVisibility {
        occurrence: OccurrenceId,
        visible: bool,
    },
    SetGrounded {
        occurrence: OccurrenceId,
        grounded: bool,
    },
    MakeIndependent {
        occurrence: OccurrenceId,
        new_definition: ComponentDefId,
        name: String,
    },
}

fn invalid(message: impl Into<String>) -> DomainError {
    DomainError::Validation(message.into())
}

pub(crate) fn memento(doc: &Document, edit: &AssemblyEdit) -> super::Inverse {
    let (definition, capture_assembly) = match edit {
        AssemblyEdit::CreateDefinition { id, .. }
        | AssemblyEdit::CopyRoot { id, .. }
        | AssemblyEdit::EditDefinition { id, .. } => (Some(*id), false),
        AssemblyEdit::MakeIndependent { new_definition, .. } => (Some(*new_definition), true),
        _ => (None, true),
    };
    super::Inverse::RestoreAssembly {
        definition: definition.map(|id| (id, doc.definitions.get(&id).cloned().map(Box::new))),
        assembly: capture_assembly.then(|| Box::new(doc.assembly.clone())),
    }
}

pub(crate) fn apply(doc: &mut Document, edit: &AssemblyEdit) -> Result<bool, DomainError> {
    match edit {
        AssemblyEdit::CreateDefinition { id, name } => {
            if *id == doc.root_definition_id || doc.definitions.contains_key(id) {
                return Err(invalid("component definition id already exists"));
            }
            if doc.definitions.len() >= MAX_OCCURRENCES {
                return Err(invalid("component definition limit reached"));
            }
            doc.definitions.insert(*id, ComponentDef::new(*id, name));
            Ok(true)
        }
        AssemblyEdit::CopyRoot { id, name } => copy_root(doc, *id, name),
        AssemblyEdit::EditDefinition { id, command } => edit_definition(doc, *id, command),
        AssemblyEdit::InsertOccurrence { occurrence } => {
            if occurrence.parent.is_some() || !doc.definitions.contains_key(&occurrence.def) {
                return Err(invalid(
                    "occurrence requires an embedded definition and no parent",
                ));
            }
            doc.assembly
                .occurrences
                .insert(occurrence.clone())
                .map_err(|error| invalid(error.to_string()))?;
            Ok(true)
        }
        AssemblyEdit::SetPose { occurrence, pose } => {
            let old = doc
                .assembly
                .occurrences
                .get(*occurrence)
                .ok_or_else(|| invalid("unknown occurrence"))?;
            if old.local == *pose {
                return Ok(false);
            }
            doc.assembly
                .occurrences
                .set_local(*occurrence, *pose)
                .map_err(|error| invalid(error.to_string()))?;
            Ok(true)
        }
        AssemblyEdit::SetVisibility {
            occurrence,
            visible,
        } => {
            if doc.assembly.occurrences.get(*occurrence).is_none() {
                return Err(invalid("unknown occurrence"));
            }
            if doc
                .assembly
                .visibility
                .get(occurrence)
                .copied()
                .unwrap_or(true)
                == *visible
            {
                return Ok(false);
            }
            if *visible {
                doc.assembly.visibility.remove(occurrence);
            } else {
                doc.assembly.visibility.insert(*occurrence, false);
            }
            Ok(true)
        }
        AssemblyEdit::SetGrounded {
            occurrence,
            grounded,
        } => {
            let old = doc
                .assembly
                .occurrences
                .get(*occurrence)
                .ok_or_else(|| invalid("unknown occurrence"))?;
            if old.grounded == *grounded {
                return Ok(false);
            }
            doc.assembly
                .occurrences
                .set_grounded(*occurrence, *grounded)
                .map_err(|error| invalid(error.to_string()))?;
            Ok(true)
        }
        AssemblyEdit::MakeIndependent {
            occurrence,
            new_definition,
            name,
        } => make_independent(doc, *occurrence, *new_definition, name),
    }
}

fn copy_root(doc: &mut Document, id: ComponentDefId, name: &str) -> Result<bool, DomainError> {
    if id == doc.root_definition_id
        || doc.definitions.contains_key(&id)
        || doc.definitions.len() >= MAX_OCCURRENCES
    {
        return Err(invalid(
            "component definition id unavailable or limit reached",
        ));
    }
    let mut def = ComponentDef::new(id, name);
    def.variables = doc.variables.clone();
    def.sketches = doc.sketches.clone();
    def.sketch_visibility = doc.sketch_visibility.clone();
    def.datum_planes = doc.datum_planes.clone();
    def.timeline = doc.timeline.clone();
    def.bodies = doc.bodies.clone();
    def.elements = doc.elements.clone();
    def.repair = doc.repair.clone();
    def.named_selections = doc.named_selections.clone();
    doc.definitions.insert(id, def);
    Ok(true)
}

fn make_independent(
    doc: &mut Document,
    id: OccurrenceId,
    new_id: ComponentDefId,
    name: &str,
) -> Result<bool, DomainError> {
    if new_id == doc.root_definition_id
        || doc.definitions.contains_key(&new_id)
        || doc.definitions.len() >= MAX_OCCURRENCES
    {
        return Err(invalid(
            "component definition id unavailable or limit reached",
        ));
    }
    let mut occurrence = doc
        .assembly
        .occurrences
        .get(id)
        .cloned()
        .ok_or_else(|| invalid("unknown occurrence"))?;
    let mut definition = doc
        .definitions
        .get(&occurrence.def)
        .cloned()
        .ok_or_else(|| invalid("unknown definition"))?;
    definition.id = new_id;
    definition.name = name.to_owned();
    occurrence.def = new_id;
    let mut tree = doc.assembly.occurrences.clone();
    tree.remove(id)
        .map_err(|error| invalid(error.to_string()))?;
    tree.insert(occurrence)
        .map_err(|error| invalid(error.to_string()))?;
    doc.definitions.insert(new_id, definition);
    doc.assembly.occurrences = tree;
    Ok(true)
}

fn edit_definition(
    doc: &mut Document,
    id: ComponentDefId,
    command: &EditCommand,
) -> Result<bool, DomainError> {
    if matches!(
        command,
        EditCommand::Assembly { .. } | EditCommand::SetModuleState { .. }
    ) {
        return Err(invalid(
            "nested assembly and document-module edits are not definition edits",
        ));
    }
    let definition = doc
        .definitions
        .get(&id)
        .cloned()
        .ok_or_else(|| invalid("unknown definition"))?;
    let mut part = Document::new(doc.id);
    part.variables = definition.variables;
    part.sketches = definition.sketches;
    part.sketch_visibility = definition.sketch_visibility;
    part.datum_planes = definition.datum_planes;
    part.timeline = definition.timeline;
    part.bodies = definition.bodies;
    part.elements = definition.elements;
    part.repair = definition.repair;
    part.named_selections = definition.named_selections;
    let mut session = DocumentSession::new(part);
    let (_, changed) = session.apply_with_change(command.clone())?;
    if !changed {
        return Ok(false);
    }
    let part = session.into_document();
    let target = doc
        .definitions
        .get_mut(&id)
        .expect("definition checked above");
    target.variables = part.variables;
    target.sketches = part.sketches;
    target.sketch_visibility = part.sketch_visibility;
    target.datum_planes = part.datum_planes;
    target.timeline = part.timeline;
    target.bodies = part.bodies;
    target.elements = part.elements;
    target.repair = part.repair;
    target.named_selections = part.named_selections;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::variables::{Scalar, Variable},
        ids::{DocumentId, VariableId},
    };
    use onecad_assembly::math::{Quat, Vec3d};
    use uuid::Uuid;

    fn edit(session: &mut DocumentSession, edit: AssemblyEdit) -> Result<(), DomainError> {
        session
            .apply(EditCommand::Assembly {
                edit: Box::new(edit),
            })
            .map(|_| ())
    }

    #[test]
    fn embedded_definitions_share_edits_and_independence_is_undoable() {
        let def = ComponentDefId(Uuid::from_u128(2));
        let independent = ComponentDefId(Uuid::from_u128(3));
        let a = OccurrenceId(Uuid::from_u128(4));
        let b = OccurrenceId(Uuid::from_u128(5));
        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        edit(
            &mut session,
            AssemblyEdit::CreateDefinition {
                id: def,
                name: "Part".into(),
            },
        )
        .unwrap();
        edit(
            &mut session,
            AssemblyEdit::EditDefinition {
                id: def,
                command: Box::new(EditCommand::AddVariable {
                    variable: Variable {
                        id: VariableId(Uuid::from_u128(6)),
                        name: "width".into(),
                        value: Scalar::new(12.0),
                        unit: Default::default(),
                    },
                }),
            },
        )
        .unwrap();
        for id in [a, b] {
            edit(
                &mut session,
                AssemblyEdit::InsertOccurrence {
                    occurrence: Occurrence {
                        id,
                        def,
                        parent: None,
                        local: RigidTransform::IDENTITY,
                        grounded: false,
                        name: id.to_string(),
                    },
                },
            )
            .unwrap();
        }
        let before_noop = session.undo_depth();
        edit(
            &mut session,
            AssemblyEdit::SetPose {
                occurrence: a,
                pose: RigidTransform::IDENTITY,
            },
        )
        .unwrap();
        assert_eq!(session.undo_depth(), before_noop);
        edit(
            &mut session,
            AssemblyEdit::SetVisibility {
                occurrence: b,
                visible: false,
            },
        )
        .unwrap();
        let moved = RigidTransform::new(Quat::IDENTITY, Vec3d::new(5.0, 0.0, 0.0));
        edit(
            &mut session,
            AssemblyEdit::SetPose {
                occurrence: b,
                pose: moved,
            },
        )
        .unwrap();
        assert_eq!(
            session
                .document()
                .assembly
                .occurrences
                .get(a)
                .unwrap()
                .local,
            RigidTransform::IDENTITY
        );
        assert_eq!(
            session
                .document()
                .assembly
                .occurrences
                .get(b)
                .unwrap()
                .local,
            moved
        );
        edit(
            &mut session,
            AssemblyEdit::MakeIndependent {
                occurrence: b,
                new_definition: independent,
                name: "Copy".into(),
            },
        )
        .unwrap();
        assert_eq!(
            session.document().assembly.occurrences.get(b).unwrap().def,
            independent
        );
        edit(
            &mut session,
            AssemblyEdit::EditDefinition {
                id: def,
                command: Box::new(EditCommand::SetVariable {
                    variable: VariableId(Uuid::from_u128(6)),
                    value: Scalar::new(23.0),
                }),
            },
        )
        .unwrap();
        assert_eq!(
            session.document().definitions[&def]
                .variables
                .get("width")
                .unwrap()
                .value
                .value,
            23.0
        );
        assert_eq!(
            session.document().definitions[&independent]
                .variables
                .get("width")
                .unwrap()
                .value
                .value,
            12.0
        );
        assert_eq!(
            session.document().definitions[&independent].variables.len(),
            1
        );
        session.undo().unwrap();
        assert_eq!(
            session.document().definitions[&def]
                .variables
                .get("width")
                .unwrap()
                .value
                .value,
            12.0
        );
        session.undo().unwrap();
        assert_eq!(
            session.document().assembly.occurrences.get(b).unwrap().def,
            def
        );
        assert!(!session.document().definitions.contains_key(&independent));
        session.redo().unwrap().unwrap();
        assert_eq!(
            session.document().assembly.occurrences.get(b).unwrap().def,
            independent
        );
    }

    #[test]
    fn invalid_pose_does_not_change_the_document_or_history() {
        let def = ComponentDefId(Uuid::from_u128(2));
        let occurrence = OccurrenceId(Uuid::from_u128(3));
        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        edit(
            &mut session,
            AssemblyEdit::CreateDefinition {
                id: def,
                name: "Part".into(),
            },
        )
        .unwrap();
        edit(
            &mut session,
            AssemblyEdit::InsertOccurrence {
                occurrence: Occurrence {
                    id: occurrence,
                    def,
                    parent: None,
                    local: RigidTransform::IDENTITY,
                    grounded: false,
                    name: "Part".into(),
                },
            },
        )
        .unwrap();
        let depth = session.undo_depth();
        let bad = RigidTransform::new(Quat::IDENTITY, Vec3d::new(f64::NAN, 0.0, 0.0));
        assert!(edit(
            &mut session,
            AssemblyEdit::SetPose {
                occurrence,
                pose: bad
            }
        )
        .is_err());
        assert_eq!(session.undo_depth(), depth);
        assert_eq!(
            session
                .document()
                .assembly
                .occurrences
                .get(occurrence)
                .unwrap()
                .local,
            RigidTransform::IDENTITY
        );
    }

    #[test]
    fn ten_definitions_and_twenty_occurrences_survive_serialization() {
        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        for n in 0..10 {
            let def = ComponentDefId(Uuid::from_u128(100 + n));
            edit(
                &mut session,
                AssemblyEdit::CreateDefinition {
                    id: def,
                    name: format!("Part {n}"),
                },
            )
            .unwrap();
            for copy in 0..2 {
                edit(
                    &mut session,
                    AssemblyEdit::InsertOccurrence {
                        occurrence: Occurrence {
                            id: OccurrenceId(Uuid::from_u128(1000 + n * 2 + copy)),
                            def,
                            parent: None,
                            local: RigidTransform::IDENTITY,
                            grounded: false,
                            name: format!("Part {n}.{copy}"),
                        },
                    },
                )
                .unwrap();
            }
        }
        let bytes = serde_json::to_vec(session.document()).unwrap();
        let reopened: Document = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reopened.definitions.len(), 10);
        assert_eq!(reopened.assembly.occurrences.len(), 20);
        assert!(reopened
            .assembly
            .validate(reopened.root_definition_id, &reopened.definitions)
            .is_ok());
    }
}
