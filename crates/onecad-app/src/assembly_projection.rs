//! Definition-local assembly metadata, independent of mesh readiness.
use onecad_core::document::{body::BodyMeta, Document};

use crate::{
    dto::{AssemblyDefinitionDto, AssemblyOccurrenceDto, AssemblyProjectionDto, BodyDto},
    error::ApiError,
};

/// Projects authoritative definition and occurrence identities and placements.
///
/// # Errors
/// Returns an error if occurrence paths or references are invalid.
pub fn project(document: &Document) -> Result<AssemblyProjectionDto, ApiError> {
    document
        .assembly
        .validate(document.root_definition_id, &document.definitions)
        .map_err(ApiError::InvalidCommand)?;
    let mut definitions = vec![AssemblyDefinitionDto {
        id: document.root_definition_id.to_string(),
        name: document.root_definition_name.clone(),
        bodies: document.bodies.bodies().iter().map(body).collect(),
        mesh_generation: None,
    }];
    definitions.extend(
        document
            .definitions
            .values()
            .map(|def| AssemblyDefinitionDto {
                id: def.id.to_string(),
                name: def.name.clone(),
                bodies: def.bodies.bodies().iter().map(body).collect(),
                mesh_generation: None,
            }),
    );
    let mut occurrences = Vec::new();
    for occurrence in document.assembly.occurrences.iter() {
        let path = document
            .assembly
            .occurrences
            .path(occurrence.id)
            .map_err(|error| ApiError::InvalidCommand(error.to_string()))?;
        occurrences.push(AssemblyOccurrenceDto {
            id: occurrence.id.to_string(),
            definition: occurrence.def.to_string(),
            path: path.0.iter().map(ToString::to_string).collect(),
            local: occurrence.local,
            grounded: occurrence.grounded,
            visible: document
                .assembly
                .visibility
                .get(&occurrence.id)
                .copied()
                .unwrap_or(true),
        });
    }
    Ok(AssemblyProjectionDto {
        root_definition: document.root_definition_id.to_string(),
        definitions,
        occurrences,
    })
}

fn body(meta: &BodyMeta) -> BodyDto {
    BodyDto {
        id: meta.id.to_string(),
        created_by: meta.created_by.to_string(),
        name: meta.name.clone(),
        visible: meta.visible,
        health: meta.health,
        color: meta.color,
        face_colors: meta
            .face_colors
            .iter()
            .map(|(key, value)| (key.to_string(), *value))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::{
        document::{
            body::BodyMeta,
            component::{
                ComponentDef, ComponentDefId, Occurrence, OccurrenceId, Quat, RigidTransform, Vec3d,
            },
        },
        ids::{BodyId, DocumentId, RecordId},
    };
    use uuid::Uuid;

    #[test]
    fn repeated_occurrences_share_local_body_and_keep_independent_placement() {
        let mut doc = Document::new(DocumentId(Uuid::from_u128(1)));
        let def = ComponentDefId(Uuid::from_u128(2));
        let first = OccurrenceId(Uuid::from_u128(3));
        let second = OccurrenceId(Uuid::from_u128(4));
        let mut part = ComponentDef::new(def, "Part");
        let producer = RecordId(Uuid::from_u128(5));
        part.bodies
            .register(BodyMeta::new(BodyId(Uuid::from_u128(6)), "Child", producer));
        doc.definitions.insert(def, part);
        for (id, local, grounded) in [
            (first, RigidTransform::IDENTITY, true),
            (
                second,
                RigidTransform::new(Quat::IDENTITY, Vec3d::new(5.0, 0.0, 0.0)),
                false,
            ),
        ] {
            doc.assembly
                .occurrences
                .insert(Occurrence {
                    id,
                    def,
                    parent: None,
                    local,
                    grounded,
                    name: "Part".into(),
                })
                .unwrap();
        }
        doc.assembly.visibility.insert(second, false);
        let projection = project(&doc).unwrap();
        assert_eq!(projection.definitions.len(), 2);
        assert_eq!(
            projection.definitions[1].bodies[0].created_by,
            producer.to_string()
        );
        assert_eq!(projection.occurrences.len(), 2);
        assert_eq!(
            projection.occurrences[0].definition,
            projection.occurrences[1].definition
        );
        assert_ne!(
            projection.occurrences[0].local,
            projection.occurrences[1].local
        );
        assert!(!projection.occurrences[1].visible);
        assert_eq!(projection.occurrences[1].path, vec![second.to_string()]);
    }
}
