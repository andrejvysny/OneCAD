//! Authoritative definition-local modeling data for embedded components.

use std::collections::{BTreeMap, BTreeSet};

use onecad_assembly::joint::Joint;
pub use onecad_assembly::math::{Quat, RigidTransform, Vec3d};
pub use onecad_assembly::occurrence::{
    ComponentDefId, Occurrence, OccurrenceId, OccurrencePath, OccurrenceTree,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    document::{
        body::BodyRegistry, datum::DatumPlane, element_index::ElementIndex, refs::Extra,
        repair::RepairState, variables::VariableTable, NamedSelection, TimelineWire,
    },
    history::Timeline,
    ids::{DatumPlaneId, DocumentId, SketchId},
    sketch::Sketch,
};

/// Derives a stable root namespace for a legacy document without reauthoring IDs.
#[must_use]
pub fn root_definition_id(document: DocumentId) -> ComponentDefId {
    let mut hash = Sha256::new();
    hash.update(b"onecad.root-definition.v1\0");
    hash.update(document.0.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    ComponentDefId(uuid::Uuid::from_bytes(bytes))
}

/// One definition's authoritative history and local identity namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDef {
    pub id: ComponentDefId,
    pub name: String,
    #[serde(default)]
    pub variables: VariableTable,
    #[serde(default)]
    pub sketches: BTreeMap<SketchId, Sketch>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sketch_visibility: BTreeMap<SketchId, bool>,
    #[serde(default)]
    pub datum_planes: BTreeMap<DatumPlaneId, DatumPlane>,
    #[serde(default, with = "timeline_serde")]
    pub timeline: Timeline,
    #[serde(default)]
    pub bodies: BodyRegistry,
    #[serde(default, skip_serializing_if = "ElementIndex::is_empty")]
    pub elements: ElementIndex,
    #[serde(default)]
    pub repair: RepairState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub named_selections: Vec<NamedSelection>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl ComponentDef {
    #[must_use]
    pub fn new(id: ComponentDefId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            variables: VariableTable::new(),
            sketches: BTreeMap::new(),
            sketch_visibility: BTreeMap::new(),
            datum_planes: BTreeMap::new(),
            timeline: Timeline::new(),
            bodies: BodyRegistry::new(),
            elements: ElementIndex::new(),
            repair: RepairState::new(),
            named_selections: Vec::new(),
            extra: Extra::new(),
        }
    }
}

mod timeline_serde {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(timeline: &Timeline, serializer: S) -> Result<S::Ok, S::Error> {
        TimelineWire::from(timeline).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timeline, D::Error> {
        Ok(TimelineWire::deserialize(deserializer)?.into())
    }
}

pub(crate) fn deserialize_definitions<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<ComponentDefId, ComponentDef>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct DefinitionsVisitor;
    impl<'de> serde::de::Visitor<'de> for DefinitionsVisitor {
        type Value = BTreeMap<ComponentDefId, ComponentDef>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded component definitions")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut values = BTreeMap::new();
            while let Some(key) = map.next_key()? {
                if values.len() >= onecad_assembly::occurrence::MAX_OCCURRENCES {
                    return Err(serde::de::Error::custom(
                        "component definition limit exceeded",
                    ));
                }
                let value = map.next_value()?;
                if values.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate component definition"));
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(DefinitionsVisitor)
}

/// Root-owned assembly state. Visibility is per occurrence, not per definition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssemblyState {
    #[serde(default)]
    pub occurrences: OccurrenceTree,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub visibility: BTreeMap<OccurrenceId, bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joints: Vec<Joint>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl AssemblyState {
    /// Validates the supported flat embedded assembly without changing it.
    pub fn validate(
        &self,
        root: ComponentDefId,
        definitions: &BTreeMap<ComponentDefId, ComponentDef>,
    ) -> Result<(), String> {
        if definitions.len() > onecad_assembly::occurrence::MAX_OCCURRENCES {
            return Err("component definition limit exceeded".into());
        }
        for (id, definition) in definitions {
            if *id == root || *id != definition.id {
                return Err(format!("invalid component definition identity {id}"));
            }
            if definition.extra.contains_key("assembly")
                || definition.extra.contains_key("occurrences")
            {
                return Err(format!(
                    "nested assembly state in definition {id} is unsupported"
                ));
            }
        }
        for occurrence in self.occurrences.iter() {
            if occurrence.parent.is_some() {
                return Err("nested occurrence authoring is not supported".into());
            }
            if occurrence.def == root || !definitions.contains_key(&occurrence.def) {
                return Err(format!(
                    "occurrence {} references an unavailable definition",
                    occurrence.id
                ));
            }
        }
        if self
            .visibility
            .keys()
            .any(|id| self.occurrences.get(*id).is_none())
        {
            return Err("visibility references an unknown occurrence".into());
        }
        let mut joint_ids = BTreeSet::new();
        for joint in &self.joints {
            if !joint_ids.insert(joint.id) {
                return Err(format!("duplicate joint {}", joint.id.0));
            }
            if self.occurrences.get(joint.a.occurrence).is_none()
                || self.occurrences.get(joint.b.occurrence).is_none()
            {
                return Err(format!(
                    "joint {} references an unknown occurrence",
                    joint.id.0
                ));
            }
            joint
                .validate()
                .map_err(|error| format!("joint {}: {error}", joint.id.0))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_assembly::{math::RigidTransform, occurrence::Occurrence};
    use uuid::Uuid;

    #[test]
    fn oversized_definitions_refused_during_deserialization() {
        let mut doc = serde_json::to_value(crate::document::Document::new(DocumentId(
            Uuid::from_u128(42),
        )))
        .unwrap();
        let mut definitions = serde_json::Map::new();
        for n in 0..=onecad_assembly::occurrence::MAX_OCCURRENCES {
            let id = ComponentDefId(Uuid::from_u128(n as u128 + 100));
            definitions.insert(
                id.to_string(),
                serde_json::to_value(ComponentDef::new(id, "part")).unwrap(),
            );
        }
        doc["definitions"] = serde_json::Value::Object(definitions);
        let error = serde_json::from_value::<crate::document::Document>(doc).unwrap_err();
        assert!(error
            .to_string()
            .contains("component definition limit exceeded"));
    }

    #[test]
    fn checked_flat_assembly_accepts_repeated_definitions_and_rejects_unknown_or_nested() {
        let root = ComponentDefId(Uuid::from_u128(1));
        let def = ComponentDefId(Uuid::from_u128(2));
        let first = OccurrenceId(Uuid::from_u128(3));
        let second = OccurrenceId(Uuid::from_u128(4));
        let definitions = BTreeMap::from([(def, ComponentDef::new(def, "part"))]);
        let mut state = AssemblyState::default();
        for id in [first, second] {
            state
                .occurrences
                .insert(Occurrence {
                    id,
                    def,
                    parent: None,
                    local: RigidTransform::IDENTITY,
                    grounded: false,
                    name: "part".into(),
                })
                .unwrap();
        }
        assert!(state.validate(root, &definitions).is_ok());
        assert!(state.validate(root, &BTreeMap::new()).is_err());
        state
            .visibility
            .insert(OccurrenceId(Uuid::from_u128(9)), false);
        assert!(state.validate(root, &definitions).is_err());
        state.visibility.clear();
        state
            .occurrences
            .insert(Occurrence {
                id: OccurrenceId(Uuid::from_u128(5)),
                def,
                parent: Some(first),
                local: RigidTransform::IDENTITY,
                grounded: false,
                name: "nested".into(),
            })
            .unwrap();
        assert!(state.validate(root, &definitions).is_err());
    }
}
