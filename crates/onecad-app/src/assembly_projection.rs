//! Definition-local assembly metadata, independent of mesh readiness.
use onecad_core::document::{body::BodyMeta, Document};

use crate::{
    dto::{AssemblyDefinitionDto, AssemblyOccurrenceDto, AssemblyProjectionDto, BodyDto},
    error::ApiError,
};

/// Bounds every cloned assembly DTO field before projection allocates it.
pub(crate) fn preflight(
    document: &Document,
    root: &onecad_core::document::body::BodyRegistry,
    limit: usize,
) -> Result<usize, ApiError> {
    let mut used = 0usize;
    let mut charge = |amount: usize| -> Result<(), ApiError> {
        used = used
            .checked_add(amount)
            .filter(|v| *v <= limit)
            .ok_or_else(|| {
                ApiError::InvalidCommand("assembly projection metadata budget exceeded".into())
            })?;
        Ok(())
    };
    charge(1024 + document.root_definition_name.len())?;
    for definition in document.definitions.values() {
        charge(512 + definition.name.len())?;
        for body in definition.bodies.bodies() {
            charge_body(body, &mut charge)?;
        }
    }
    // Both registries are counted: authored metadata may overlay regen rows.
    for body in root.bodies().iter().chain(document.bodies.bodies()) {
        charge_body(body, &mut charge)?;
    }
    for occurrence in document.assembly.occurrences.iter() {
        charge(512 + occurrence.name.len())?;
    }
    for joint in &document.assembly.joints {
        charge(1024 + joint.name.len())?;
    }
    for _ in &document.assembly.joint_bindings {
        charge(128)?;
    }
    Ok(used)
}

fn charge_body(
    body: &BodyMeta,
    charge: &mut impl FnMut(usize) -> Result<(), ApiError>,
) -> Result<(), ApiError> {
    charge(512 + body.name.len())?;
    for id in body.face_colors.keys() {
        charge(128 + id.as_str().len())?;
    }
    Ok(())
}

/// Projects authoritative definition and occurrence identities and placements.
///
/// # Errors
/// Returns an error if occurrence paths or references are invalid.
pub fn project(document: &Document) -> Result<AssemblyProjectionDto, ApiError> {
    let root = document
        .bodies
        .bodies()
        .iter()
        .map(|meta| body(meta, None))
        .collect();
    project_with_root(document, root)
}

pub(crate) fn project_with_root(
    document: &Document,
    root: Vec<BodyDto>,
) -> Result<AssemblyProjectionDto, ApiError> {
    document
        .assembly
        .validate(document.root_definition_id, &document.definitions)
        .map_err(ApiError::InvalidCommand)?;
    let mut definitions = vec![AssemblyDefinitionDto {
        id: document.root_definition_id.to_string(),
        name: document.root_definition_name.clone(),
        bodies: root,
        mesh_generation: None,
    }];
    definitions.extend(document.definitions.values().map(|def| {
        AssemblyDefinitionDto {
            id: def.id.to_string(),
            name: def.name.clone(),
            bodies: def
                .bodies
                .bodies()
                .iter()
                .map(|meta| body(meta, None))
                .collect(),
            mesh_generation: None,
        }
    }));
    let mut occurrences = Vec::new();
    for occurrence in document.assembly.occurrences.iter() {
        let path = document
            .assembly
            .occurrences
            .path(occurrence.id)
            .map_err(|error| ApiError::InvalidCommand(error.to_string()))?;
        occurrences.push(AssemblyOccurrenceDto {
            name: occurrence.name.clone(),
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
        persisted_generation: 0,
        joints: document.assembly.joints.clone(),
        joint_bindings: document
            .assembly
            .joint_bindings
            .iter()
            .map(|binding| crate::dto::JointBindingStatusDto {
                joint: binding.joint,
                side: binding.side,
                needs_repair: binding.needs_repair,
                verified: false,
            })
            .collect(),
        root_definition: document.root_definition_id.to_string(),
        definitions,
        occurrences,
    })
}

pub(crate) fn body(meta: &BodyMeta, overlay: Option<&BodyMeta>) -> BodyDto {
    let authored = overlay.unwrap_or(meta);
    BodyDto {
        id: meta.id.to_string(),
        created_by: meta.created_by.to_string(),
        name: authored.name.clone(),
        visible: authored.visible,
        health: meta.health,
        color: authored.color,
        face_colors: authored
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
    fn oversized_invisible_definition_is_refused_before_projection() {
        let mut doc = Document::new(DocumentId(Uuid::from_u128(1)));
        let id = ComponentDefId(Uuid::from_u128(2));
        let mut definition = ComponentDef::new(id, "x".repeat(2 << 20));
        definition.bodies.register(BodyMeta::new(
            BodyId(Uuid::from_u128(3)),
            "hidden",
            RecordId(Uuid::from_u128(4)),
        ));
        doc.definitions.insert(id, definition);
        let error = preflight(&doc, &doc.bodies, 2 << 20).unwrap_err();
        assert!(error.to_string().contains("metadata budget"));
    }

    #[test]
    fn oversized_joint_names_are_refused_before_projection() {
        use onecad_assembly::joint::{Joint, JointId, JointKind, JointOrigin};
        use onecad_assembly::math::{Frame, Vec3d};
        use onecad_assembly::occurrence::OccurrenceId;
        let mut doc = Document::new(DocumentId(Uuid::from_u128(1)));
        let frame = Frame {
            origin: Vec3d::ZERO,
            x: Vec3d::X,
            z: Vec3d::Z,
        };
        doc.assembly.joints.push(Joint {
            id: JointId(Uuid::from_u128(2)),
            name: "x".repeat(2 << 20),
            kind: JointKind::Rigid,
            a: JointOrigin {
                occurrence: OccurrenceId(Uuid::from_u128(3)),
                frame,
            },
            b: JointOrigin {
                occurrence: OccurrenceId(Uuid::from_u128(4)),
                frame,
            },
            limits: None,
            value: 0.0,
            suppressed: false,
        });
        assert!(preflight(&doc, &doc.bodies, 2 << 20).is_err());
    }

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
