//! A bounded transient checkpoint for native component identity continuation.
use super::*;
use std::collections::BTreeSet;

pub(crate) const MAX_BINARY_BYTES: u32 = 32 << 20;

impl WorkerManager {
    pub(crate) async fn save_component_checkpoint(
        &self,
        step: usize,
    ) -> Result<CheckpointArtifacts, EngineError> {
        let client = self.client_or_err()?;
        let (resp, tail) = client
            .start_request_bounded(
                "SaveCheckpoint",
                wire::save_checkpoint_args(step),
                Lane::Control,
                crate::worker::MAX_PREVIEW_RESPONSE_JSON_BYTES,
                MAX_BINARY_BYTES,
            )
            .await
            .map_err(protocol_err)?
            .response_with_bin()
            .await
            .map_err(protocol_err)?;
        let sections = resp.bin.clone().unwrap_or_default();
        let result = ok_result(resp)?;
        validate_copy_budget(&result, &sections, &tail)
            .map_err(|message| EngineError::Protocol { message })?;
        wire::parse_save_checkpoint(&result, &sections, &tail, &self.shared.fingerprint())
            .map_err(|message| EngineError::Protocol { message })
    }
}

fn validate_copy_budget(
    result: &Value,
    sections: &[onecad_protocol::messages::BinSection],
    tail: &[u8],
) -> Result<(), String> {
    let artifacts = result
        .get("artifacts")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() <= 4096)
        .ok_or("invalid component checkpoint artifacts")?;
    if tail.len() > MAX_BINARY_BYTES as usize {
        return Err("component checkpoint tail exceeds budget".into());
    }
    let by_name = wire::validate_bin_sections("component checkpoint", sections, tail)?;
    let mut names = BTreeSet::new();
    let mut total = 0usize;
    let partition = result
        .get("elementMapPartition")
        .ok_or("missing component checkpoint partition")?;
    for row in artifacts.iter().chain(std::iter::once(partition)) {
        let name = row
            .get("bin")
            .and_then(Value::as_str)
            .ok_or("missing component checkpoint bin")?;
        if !names.insert(name) {
            return Err("duplicate component checkpoint bin".into());
        }
        let section = by_name
            .get(name)
            .ok_or("unknown component checkpoint bin")?;
        let end = section
            .off
            .checked_add(section.len)
            .ok_or("component checkpoint range overflow")?;
        if end as usize > tail.len() {
            return Err("component checkpoint range exceeds tail".into());
        }
        total = total
            .checked_add(section.len as usize)
            .filter(|n| *n <= MAX_BINARY_BYTES as usize)
            .ok_or("component checkpoint copies exceed budget")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_protocol::messages::BinSection;
    fn section(name: &str, off: u32, len: u32) -> BinSection {
        serde_json::from_value(json!({"name":name,"off":off,"len":len})).unwrap()
    }
    fn result() -> Value {
        json!({"artifacts":[{"bin":"a"}],"elementMapPartition":{"bin":"p"}})
    }
    #[test]
    fn duplicate_checkpoint_references_refuse_copy_amplification() {
        let result =
            json!({"artifacts":[{"bin":"a"},{"bin":"a"}],"elementMapPartition":{"bin":"p"}});
        assert!(
            validate_copy_budget(&result, &[section("a", 0, 1), section("p", 1, 1)], &[0; 2])
                .unwrap_err()
                .contains("duplicate component checkpoint bin")
        );
    }
    #[test]
    fn checkpoint_sections_reject_ambiguous_names_and_overlap() {
        assert!(validate_copy_budget(
            &result(),
            &[section("a", 0, 1), section("a", 1, 1), section("p", 2, 1)],
            &[0; 3]
        )
        .unwrap_err()
        .contains("duplicate binary section"));
        assert!(validate_copy_budget(
            &result(),
            &[section("a", 0, 2), section("p", 1, 1)],
            &[0; 2]
        )
        .unwrap_err()
        .contains("overlap"));
    }
    #[test]
    fn checkpoint_sections_refuse_overflow_or_out_of_tail() {
        assert!(validate_copy_budget(
            &result(),
            &[section("a", u32::MAX, 1), section("p", 1, 1)],
            &[0; 2]
        )
        .is_err());
        assert!(validate_copy_budget(
            &result(),
            &[section("a", 0, 3), section("p", 1, 1)],
            &[0; 2]
        )
        .is_err());
        assert!(validate_copy_budget(
            &result(),
            &[section("a", 0, 1), section("p", 1, 1)],
            &[0; 2]
        )
        .is_ok());
    }
}
