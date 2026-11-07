//! Retain replay-minted topology identities with the edit that produced them.
use super::*;
use crate::document::{
    component::ComponentDef,
    element_index::{ElementEntry, ElementIndex},
};
use onecad_assembly::occurrence::ComponentDefId;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub(crate) struct ComponentRedoState {
    id: ComponentDefId,
    inputs: [u8; 32],
    baseline: Option<[u8; 32]>,
    lifecycle: [u8; 32],
    upsert: Vec<(crate::ids::ElementId, ElementEntry)>,
    removed: Vec<crate::ids::ElementId>,
}

#[derive(serde::Serialize)]
struct TimelineView<'a> {
    records: &'a [OperationRecord],
    cursor: usize,
}

#[derive(serde::Serialize)]
struct ComponentView<'a> {
    id: ComponentDefId,
    name: &'a str,
    variables: &'a VariableTable,
    sketches: &'a BTreeMap<SketchId, Sketch>,
    sketch_visibility: &'a BTreeMap<SketchId, bool>,
    datum_planes: &'a BTreeMap<crate::ids::DatumPlaneId, DatumPlane>,
    timeline: TimelineView<'a>,
    named_selections: &'a [crate::document::NamedSelection],
    extra: &'a crate::document::refs::Extra,
    bodies: Option<&'a BodyRegistry>,
    elements: Option<&'a ElementIndex>,
    repair: Option<&'a crate::document::repair::RepairState>,
}
struct HashWriter(Sha256);
impl std::io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn component_hash(definition: &ComponentDef, derived: bool) -> Result<[u8; 32], DomainError> {
    let view = ComponentView {
        id: definition.id,
        name: &definition.name,
        variables: &definition.variables,
        sketches: &definition.sketches,
        sketch_visibility: &definition.sketch_visibility,
        datum_planes: &definition.datum_planes,
        timeline: TimelineView {
            records: definition.timeline.records(),
            cursor: definition.timeline.cursor(),
        },
        named_selections: &definition.named_selections,
        extra: &definition.extra,
        bodies: derived.then_some(&definition.bodies),
        elements: derived.then_some(&definition.elements),
        repair: derived.then_some(&definition.repair),
    };
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, &view)
        .map_err(|error| DomainError::Validation(error.to_string()))?;
    Ok(writer.0.finalize().into())
}

fn lifecycle_hash(definition: &ComponentDef) -> Result<[u8; 32], DomainError> {
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, &(&definition.bodies, &definition.repair))
        .map_err(|error| DomainError::Validation(error.to_string()))?;
    Ok(writer.0.finalize().into())
}

fn prior_definitions<'a>(
    inverse: &'a Inverse,
    out: &mut BTreeMap<ComponentDefId, Option<&'a ComponentDef>>,
) {
    match inverse {
        Inverse::RestoreAssembly {
            definition: Some((id, definition)),
            ..
        } => {
            out.entry(*id).or_insert(definition.as_deref());
        }
        Inverse::Composite(items) => {
            for item in items {
                prior_definitions(item, out);
            }
        }
        _ => {}
    }
}

impl DocumentSession {
    pub(super) fn capture_component_identities(
        &self,
        txn: &Txn,
    ) -> Result<Vec<ComponentRedoState>, DomainError> {
        let mut prior = BTreeMap::new();
        for edit in &txn.edits {
            prior_definitions(&edit.inverse, &mut prior);
        }
        let mut states = Vec::new();
        for (id, before) in prior {
            let Some(after) = self.document.definitions.get(&id) else {
                continue;
            };
            let empty = ElementIndex::new();
            let old = before.map_or(&empty, |definition| &definition.elements);
            let upsert: Vec<_> = after
                .elements
                .iter()
                .filter(|(id, entry)| old.get(id) != Some(*entry))
                .map(|(id, entry)| (id.clone(), entry.clone()))
                .collect();
            let removed: Vec<_> = old
                .iter()
                .filter(|(id, _)| !after.elements.contains(id))
                .map(|(id, _)| id.clone())
                .collect();
            if !upsert.is_empty() || !removed.is_empty() {
                states.push(ComponentRedoState {
                    id,
                    inputs: component_hash(after, false)?,
                    baseline: before
                        .map(|definition| component_hash(definition, true))
                        .transpose()?,
                    lifecycle: lifecycle_hash(after)?,
                    upsert,
                    removed,
                });
            }
        }
        Ok(states)
    }

    pub(super) fn check_undone_component_identities(
        &self,
        states: &[ComponentRedoState],
    ) -> Result<(), DomainError> {
        for state in states {
            let current = self
                .document
                .definitions
                .get(&state.id)
                .map(|definition| component_hash(definition, true))
                .transpose()?;
            if current != state.baseline {
                return Err(DomainError::Validation(
                    "redo component state changed while undone".into(),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn restore_component_identities(
        &mut self,
        states: &[ComponentRedoState],
    ) -> Result<(), DomainError> {
        for state in states {
            let definition = self
                .document
                .definitions
                .get(&state.id)
                .ok_or_else(|| DomainError::Validation("redo component is missing".into()))?;
            if component_hash(definition, false)? != state.inputs {
                return Err(DomainError::Validation(
                    "redo component inputs changed before identity restoration".into(),
                ));
            }
            // A copied definition may inherit newer replay identities from its source.
            // Never combine those with evidence captured before the copy was undone.
            if state.baseline.is_none()
                && (lifecycle_hash(definition)? != state.lifecycle
                    || definition.elements.iter().any(|(id, entry)| {
                        state
                            .upsert
                            .binary_search_by(|(saved_id, _)| saved_id.cmp(id))
                            .map_or(true, |index| state.upsert[index].1 != *entry)
                    }))
            {
                return Err(DomainError::Validation(
                    "redo copied component identities changed while undone".into(),
                ));
            }
        }
        // Only topology evidence is restored; authored fields and opaque metadata remain current.
        for state in states {
            let definition = self
                .document
                .definitions
                .get_mut(&state.id)
                .expect("preflighted component");
            for id in &state.removed {
                definition.elements.remove(id);
            }
            for (id, entry) in &state.upsert {
                definition.elements.insert(id.clone(), entry.clone());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "component_history_tests.rs"]
mod tests;
