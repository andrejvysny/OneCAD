//! Reserved identities may remain only as evidence or explicit repair bindings.
use super::{component::JointBinding, element_index::ElementEntry, Document};
use crate::ids::ElementId;
use serde_json::Value;
use std::collections::HashSet;
mod closed_world;

pub(super) fn validate_consumers(document: &Document) -> Result<(), String> {
    let indices = std::iter::once((document.root_definition_id, &document.elements)).chain(
        document
            .definitions
            .iter()
            .map(|(id, definition)| (*id, &definition.elements)),
    );
    if !document.has_quarantined_elements() {
        return Ok(());
    }
    closed_world::validate(document)?;
    let source = serde_json::to_value(document).map_err(|error| error.to_string())?;
    for (definition, index) in indices {
        let reserved: HashSet<&str> = index
            .iter()
            .filter(|(_, entry)| entry.is_quarantined())
            .map(|(id, _)| id.as_str())
            .collect();
        if reserved.is_empty() {
            continue;
        }
        if reserved.iter().any(|id| !minted_identity(id)) {
            return Err("quarantine requires a minted persistent element identity".into());
        }
        let mut visible = scoped_view(document, &source, definition)?;
        let owner = if definition == document.root_definition_id {
            &mut visible["rootDefinition"]
        } else {
            &mut visible["definitions"][definition.to_string()]
        };
        if let Some(elements) = owner.get_mut("elements").and_then(Value::as_object_mut) {
            elements.retain(|id, _| !reserved.contains(id.as_str()));
        }
        mask_repair_bindings(document, &mut visible, definition, &reserved)?;
        reject_remaining(&visible, &reserved)?;
    }
    Ok(())
}

fn scoped_view(
    document: &Document,
    source: &Value,
    target: onecad_assembly::occurrence::ComponentDefId,
) -> Result<Value, String> {
    let mut visible = source.clone();
    // Definition-local identities may legitimately repeat after Copy/Make Independent.
    // Other definitions' ordinary fields cannot address this definition's topology.
    if target != document.root_definition_id {
        visible["rootDefinition"] = serde_json::to_value(&document.root_definition_extra)
            .map_err(|error| error.to_string())?;
    }
    for (id, definition) in &document.definitions {
        if *id != target {
            visible["definitions"][id.to_string()] =
                serde_json::to_value(&definition.extra).map_err(|error| error.to_string())?;
        }
    }
    Ok(visible)
}

fn minted_identity(id: &str) -> bool {
    id.len() == 35
        && id.starts_with("el_")
        && id.as_bytes()[3..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn mask_repair_bindings(
    document: &Document,
    visible: &mut Value,
    target: onecad_assembly::occurrence::ComponentDefId,
    reserved: &HashSet<&str>,
) -> Result<(), String> {
    for (index, binding) in document.assembly.joint_bindings.iter().enumerate() {
        if binding.definition != target {
            visible["assembly"]["jointBindings"][index]["element"] = Value::Null;
            continue;
        }
        let Some(primary) = &binding.element.primary else {
            continue;
        };
        if !reserved.contains(primary.element.as_str()) {
            continue;
        }
        let entries = if binding.definition == document.root_definition_id {
            &document.elements
        } else {
            &document
                .definitions
                .get(&binding.definition)
                .ok_or("quarantined binding definition is missing")?
                .elements
        };
        let entry = entries
            .get(&primary.element)
            .filter(|entry| entry.is_quarantined())
            .ok_or("quarantined binding crosses identity namespaces")?;
        if !binding.needs_repair || !binding_mirrors(binding, &primary.element, entry) {
            return Err(
                "quarantined identity requires its unchanged explicit repair binding".into(),
            );
        }
        if binding.path.0.len() != 1
            || !document.assembly.occurrences.iter().any(|occurrence| {
                occurrence.id == binding.path.0[0] && occurrence.def == binding.definition
            })
        {
            return Err("quarantined binding occurrence ownership is unavailable".into());
        }
        visible["assembly"]["jointBindings"][index]["element"] = Value::Null;
    }
    Ok(())
}

/// Compares every reference-bearing field before treating a repair binding as inactive.
pub fn binding_mirrors(binding: &JointBinding, id: &ElementId, entry: &ElementEntry) -> bool {
    let reference = &binding.element;
    let primary_matches = reference.primary.as_ref().is_some_and(|primary| {
        primary.element == *id
            && primary.body == entry.body
            && primary.kind == entry.kind
            && primary.extra.is_empty()
    });
    let intent_matches = match (&reference.intent, &entry.descriptor) {
        (Some(intent), Some(descriptor)) => {
            intent.kind == entry.kind
                && intent.version == crate::regen::PolicyVersions::default().descriptor
                && intent.descriptor == *descriptor
                && intent.extra.is_empty()
        }
        (None, None) => true,
        _ => false,
    };
    binding.body == entry.body
        && primary_matches
        && intent_matches
        && reference.anchor == entry.anchor
        && reference.extra.is_empty()
        && reference.import_source_identity.is_none()
        && reference
            .anchor
            .as_ref()
            .is_none_or(|anchor| anchor.extra.is_empty())
}

fn reject_remaining(value: &Value, reserved: &HashSet<&str>) -> Result<(), String> {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    reject_text(key, reserved)?;
                    pending.push(value);
                }
            }
            Value::Array(items) => pending.extend(items),
            Value::String(text) => reject_text(text, reserved)?,
            _ => {}
        }
    }
    Ok(())
}

fn reject_text(text: &str, reserved: &HashSet<&str>) -> Result<(), String> {
    for (start, _) in text.match_indices("el_") {
        if let Some(id) = text
            .get(start..start + 35)
            .filter(|id| reserved.contains(id))
        {
            return Err(format!(
                "quarantined identity {id} has a consumer outside an explicit repair binding"
            ));
        }
    }
    Ok(())
}
