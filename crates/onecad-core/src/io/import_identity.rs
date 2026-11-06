//! Fail-closed admission for critical imported topology identity.
use serde_json::Value;

use crate::document::import_source_identity::ImportSourceIdentity;

fn certificate(value: &Value) -> Result<ImportSourceIdentity, String> {
    serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid importSourceIdentity: {error}"))
}

fn check_identity_owners(value: &Value) -> Result<(), String> {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(fields) => {
                if let Some(raw) = fields.get("importSourceIdentity") {
                    let cert = certificate(raw)?;
                    let owner = fields
                        .get("primary")
                        .and_then(Value::as_object)
                        .unwrap_or(fields);
                    let body = owner.get("bodyId").or_else(|| owner.get("body"));
                    let body: crate::ids::BodyId =
                        serde_json::from_value(body.cloned().ok_or("identity owner missing body")?)
                            .map_err(|error| format!("invalid identity owner body: {error}"))?;
                    if body != cert.body_id
                        || owner.get("kind").and_then(Value::as_str) != Some("face")
                    {
                        return Err("import source identity owner mismatch".into());
                    }
                }
                pending.extend(fields.values());
            }
            Value::Array(items) => pending.extend(items),
            _ => {}
        }
    }
    Ok(())
}

fn check_definition(definition: &Value) -> Result<(), String> {
    let Some(entries) = definition.get("elements").and_then(Value::as_object) else {
        return Ok(());
    };
    let mut certificates = Vec::new();
    for (id, entry) in entries {
        if let Some(raw) = entry.get("importSourceIdentity") {
            let cert = certificate(raw)?;
            if certificates
                .iter()
                .any(|(other_id, other)| other == &cert && *other_id != id)
            {
                return Err("competing element ids share import source identity".into());
            }
            certificates.push((id, cert));
        }
    }
    let mut pending = vec![definition];
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(fields) => {
                if let Some(primary) = fields.get("primary").and_then(Value::as_object) {
                    if let Some(entry) = primary
                        .get("elementId")
                        .and_then(Value::as_str)
                        .and_then(|id| entries.get(id))
                    {
                        if fields.get("importSourceIdentity") != entry.get("importSourceIdentity") {
                            return Err(
                                "element reference and index import source identities disagree"
                                    .into(),
                            );
                        }
                    }
                }
                pending.extend(
                    fields
                        .iter()
                        .filter(|(key, _)| *key != "elements")
                        .map(|(_, value)| value),
                );
            }
            Value::Array(items) => pending.extend(items),
            _ => {}
        }
    }
    Ok(())
}

fn check_joint_bindings(value: &Value) -> Result<(), String> {
    let Some(bindings) = value
        .get("assembly")
        .and_then(|a| a.get("jointBindings"))
        .and_then(Value::as_array)
    else {
        return Ok(());
    };
    for binding in bindings {
        let Some(definition) = binding
            .get("definition")
            .and_then(Value::as_str)
            .and_then(|id| value.get("definitions").and_then(|defs| defs.get(id)))
        else {
            continue;
        };
        let Some(reference) = binding.get("element") else {
            continue;
        };
        let entry = reference
            .get("primary")
            .and_then(|p| p.get("elementId"))
            .and_then(Value::as_str)
            .and_then(|id| {
                definition
                    .get("elements")
                    .and_then(|entries| entries.get(id))
            });
        if let Some(entry) = entry {
            if reference.get("importSourceIdentity") != entry.get("importSourceIdentity") {
                return Err("joint reference and index import source identities disagree".into());
            }
        }
    }
    Ok(())
}

pub(super) fn validate(value: &Value) -> Result<(), String> {
    check_identity_owners(value)?;
    check_joint_bindings(value)?;
    if let Some(root) = value.get("rootDefinition") {
        check_definition(root)?;
    }
    if let Some(definitions) = value.get("definitions").and_then(Value::as_object) {
        for definition in definitions.values() {
            check_definition(definition)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document() -> Value {
        let cert = json!({"version":1,"sourceSha256":"a".repeat(64),"codec":"step",
            "codecVersion":"occt-step-stream-v1","readerPolicy":"step-v1","kernelFingerprint":"0a6a1dce34181289",
            "producerRecordId":"00000000-0000-0000-0000-000000000001",
            "bodyId":"00000000-0000-0000-0000-000000000002","unitScaleBits":"3ff0000000000000",
            "sourceAddress":{"kind":"step","rootEntityLabel":5850,"occurrencePath":[],"faceEntityLabel":3042}});
        json!({"rootDefinition":{"elements":{"el_a":{"body":"00000000-0000-0000-0000-000000000002",
            "kind":"face","importSourceIdentity":cert}},"timeline":{"records":[{"face":{
            "primary":{"bodyId":"00000000-0000-0000-0000-000000000002","kind":"face","elementId":"el_a"},
            "importSourceIdentity":cert}}]}}})
    }

    #[test]
    fn critical_identity_is_required_and_consistent_with_element_index() {
        let valid = document();
        assert!(validate(&valid).is_ok());
        let mut missing = valid.clone();
        missing["rootDefinition"]["timeline"]["records"][0]["face"]
            .as_object_mut()
            .unwrap()
            .remove("importSourceIdentity");
        assert!(validate(&missing).is_err());
        let mut null = valid.clone();
        null["rootDefinition"]["elements"]["el_a"]["importSourceIdentity"] = Value::Null;
        assert!(validate(&null).is_err());
        let mut competing = valid;
        competing["rootDefinition"]["elements"]["el_b"] =
            competing["rootDefinition"]["elements"]["el_a"].clone();
        assert!(validate(&competing).is_err());
    }
}
