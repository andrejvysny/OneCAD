//! Keep fresh-process children separate from the parent lifecycle inventory.
use regex::Regex;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};

pub(super) fn verify(directory: &Path) -> crate::Result<Value> {
    assess(&fs::read_to_string(
        directory.join("service-lifecycle.log"),
    )?)
}

fn assess(log: &str) -> crate::Result<Value> {
    let prefix = "document_services::tests::document_io_tests::";
    let expected = BTreeMap::from([
        (
            format!("{prefix}deleted_body_repair::deleted_body_repair_rejects_sibling_and_reopens"),
            1,
        ),
        (
            format!(
                "{prefix}deleted_source_repair::deleted_source_repair_rejects_sibling_and_reopens"
            ),
            2,
        ),
    ]);
    let pattern = Regex::new(r"(?m)^fresh-process evidence: test (\S+) \.\.\. ok$")?;
    let mut actual = BTreeMap::new();
    for record in pattern.captures_iter(log) {
        *actual.entry(record[1].to_owned()).or_insert(0) += 1;
    }
    let summaries = Regex::new(
        r"(?m)^fresh-process evidence: test result: ok\. 1 passed; 0 failed; 0 ignored;",
    )?
    .find_iter(log)
    .count();
    if actual != expected || summaries != 3 {
        return Err("fresh-process child inventory must execute exactly three successful children without skips".into());
    }
    Ok(
        json!({"children":actual,"successfulChildSummaries":summaries,
        "classification":"fresh-process child tests; excluded from parent lifecycle counts"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_inventory_rejects_missing_extra_failed_and_skipped_children() {
        let body = "document_services::tests::document_io_tests::deleted_body_repair::deleted_body_repair_rejects_sibling_and_reopens";
        let source = "document_services::tests::document_io_tests::deleted_source_repair::deleted_source_repair_rejects_sibling_and_reopens";
        let child = |name| {
            format!("fresh-process evidence: test {name} ... ok\nfresh-process evidence: test result: ok. 1 passed; 0 failed; 0 ignored;\n")
        };
        let log = format!("{}{}{}", child(body), child(source), child(source));
        assert!(assess(&log).is_ok());
        assert!(assess(&log.replace("0 ignored", "1 ignored")).is_err());
        assert!(assess(&log.replace(" ... ok", " ... FAILED")).is_err());
        assert!(assess(&format!("{log}{}", child(source))).is_err());
        assert!(assess(&child(body)).is_err());
    }
}
