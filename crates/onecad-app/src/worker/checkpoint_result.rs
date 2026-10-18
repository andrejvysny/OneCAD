use onecad_core::ids::BodyId;
use onecad_core::regen::{CheckpointEnvelope, HistoryPrefixHash};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Axes {
    artifact_schema_version: u32,
    occt_fingerprint: String,
    descriptor_version: u32,
    resolver_version: u32,
    quantization_version: u32,
    solver_policy_version: u32,
    signature_version: u32,
}

pub(super) fn parse_envelope(
    artifact: &Value,
    body: BodyId,
    step: usize,
    history_prefix_hash: &HistoryPrefixHash,
    bytes_len: usize,
    expected_fingerprint: &str,
) -> Result<CheckpointEnvelope, String> {
    let axes: Axes = serde_json::from_value(
        artifact
            .get("envelope")
            .cloned()
            .ok_or("checkpoint artifact missing envelope")?,
    )
    .map_err(|e| format!("checkpoint envelope: {e}"))?;
    if axes.occt_fingerprint != expected_fingerprint {
        return Err("checkpoint envelope fingerprint differs from worker hello".into());
    }
    let size = artifact
        .get("size")
        .and_then(Value::as_u64)
        .ok_or("checkpoint artifact missing size")?;
    if size != bytes_len as u64 {
        return Err("checkpoint artifact size differs from binary section".into());
    }
    let content_hash = artifact
        .get("contentHash")
        .and_then(Value::as_str)
        .ok_or("checkpoint artifact missing contentHash")?
        .to_string();
    let codec = artifact
        .get("codec")
        .and_then(Value::as_str)
        .ok_or("checkpoint artifact missing codec")?
        .to_string();
    Ok(CheckpointEnvelope {
        artifact_schema_version: axes.artifact_schema_version,
        body,
        step,
        history_prefix_hash: history_prefix_hash.clone(),
        brep_content_hash: content_hash.clone(),
        occt_fingerprint: axes.occt_fingerprint,
        descriptor_version: axes.descriptor_version,
        resolver_version: axes.resolver_version,
        quantization_version: axes.quantization_version,
        solver_policy_version: axes.solver_policy_version,
        signature_version: axes.signature_version,
        codec,
        size,
        content_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn artifact() -> Value {
        json!({"size": 3,"codec":"brep-bintools","contentHash":"hash",
            "envelope":{"artifactSchemaVersion":7,"occtFingerprint":"fingerprint",
                "descriptorVersion":8,"resolverVersion":9,"quantizationVersion":10,
                "solverPolicyVersion":1,"signatureVersion":11,"future":true}})
    }

    fn read(value: &Value) -> Result<CheckpointEnvelope, String> {
        parse_envelope(
            value,
            BodyId(uuid::Uuid::from_u128(1)),
            4,
            &HistoryPrefixHash::new("prefix"),
            3,
            "fingerprint",
        )
    }

    #[test]
    fn parses_actual_axes_without_stamping_current_policy() {
        let envelope = read(&artifact()).unwrap();
        assert_eq!(envelope.artifact_schema_version, 7);
        assert_eq!(envelope.descriptor_version, 8);
        assert_eq!(envelope.resolver_version, 9);
        assert_eq!(envelope.quantization_version, 10);
        assert_eq!(envelope.solver_policy_version, 1);
        assert_eq!(envelope.signature_version, 11);
        assert_eq!(envelope.step, 4);
    }

    #[test]
    fn missing_axes_wrong_fingerprint_and_size_are_refused() {
        let original = artifact();
        for key in [
            "artifactSchemaVersion",
            "occtFingerprint",
            "descriptorVersion",
            "resolverVersion",
            "quantizationVersion",
            "solverPolicyVersion",
            "signatureVersion",
        ] {
            let mut value = original.clone();
            value["envelope"].as_object_mut().unwrap().remove(key);
            assert!(read(&value).is_err(), "missing {key}");
        }
        let mut wrong = original.clone();
        wrong["envelope"]["occtFingerprint"] = json!("foreign");
        assert!(read(&wrong).is_err());
        wrong = original;
        wrong["size"] = json!(4);
        assert!(read(&wrong).is_err());
    }
}
