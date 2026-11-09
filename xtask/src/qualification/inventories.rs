use regex::Regex;
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::Path};

pub(super) fn ctest(out: &Path) -> crate::Result<Value> {
    let inventory: Value = serde_json::from_slice(&fs::read(out.join("ctest-inventory.log"))?)?;
    let tests = inventory["tests"]
        .as_array()
        .ok_or("missing CTest inventory")?;
    let names: BTreeSet<_> = tests
        .iter()
        .filter_map(|test| test["name"].as_str())
        .collect();
    let log = fs::read_to_string(out.join("ctest.log"))?;
    let pattern = Regex::new(r"(?m)\d+/\d+ Test\s+#\d+: (\S+)")?;
    let actual: Vec<_> = pattern
        .captures_iter(&log)
        .map(|record| record[1].to_owned())
        .collect();
    let executed: BTreeSet<_> = actual.iter().map(String::as_str).collect();
    let summary = format!("100% tests passed, 0 tests failed out of {}", tests.len());
    if names.is_empty()
        || names.len() != tests.len()
        || names != executed
        || actual.len() != tests.len()
        || !log.contains(&summary)
        || log.contains("Not Run")
        || log.contains("***Skipped")
        || log.contains("Disabled")
    {
        return Err("CTest discovered/executed inventories differ, failed, or skipped".into());
    }
    Ok(json!({"discovered":names,"executed":executed,"testCount":tests.len(),"noSkips":true}))
}

pub(super) fn workspace(out: &Path) -> crate::Result<Value> {
    let list = fs::read_to_string(out.join("workspace-inventory.log"))?;
    let execution = fs::read_to_string(out.join("workspace.log"))?;
    let mut expected = std::collections::BTreeMap::<String, usize>::new();
    for name in list.lines().filter_map(|line| line.strip_suffix(": test")) {
        *expected.entry(name.into()).or_default() += 1;
    }
    // Child records can join parent summaries; unit names and doctest labels stay intact.
    let pattern = Regex::new(r"test (\S+(?: - [^\r\n]+? \(line [0-9]+\))?) \.\.\. ")?;
    let mut actual = std::collections::BTreeMap::<String, usize>::new();
    for record in pattern.captures_iter(&execution) {
        *actual.entry(record[1].into()).or_default() += 1;
    }
    if expected.is_empty()
        || expected
            .iter()
            .any(|(name, count)| actual.get(name).unwrap_or(&0) < count)
        || actual.keys().any(|name| !expected.contains_key(name))
    {
        return Err("workspace execution does not cover discovered inventory".into());
    }
    let children: std::collections::BTreeMap<_, _> = actual
        .iter()
        .filter_map(|(name, count)| {
            let extra = count - expected[name];
            (extra > 0).then_some((name, extra))
        })
        .collect();
    Ok(
        json!({"discovered":expected,"observedHarnessRecords":actual,"additionalChildRecords":children,
        "classification":"workspace includes declared ignored platform tiers; native and Metal execution are qualified separately with no skips"}),
    )
}

pub(super) fn metal(out: &Path) -> crate::Result<Value> {
    assess(
        &fs::read_to_string(out.join("metal-inventory.log"))?,
        &fs::read_to_string(out.join("metal.log"))?,
    )
}

fn assess(list: &str, execution: &str) -> crate::Result<Value> {
    let names: Vec<_> = list
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .collect();
    let expected: BTreeSet<_> = names.iter().copied().collect();
    let pattern = Regex::new(r"(?m)^test (\S+) \.\.\. ")?;
    let starts: Vec<_> = pattern.captures_iter(execution).collect();
    let actual: BTreeSet<_> = starts
        .iter()
        .map(|record| record.get(1).unwrap().as_str())
        .collect();
    if names.is_empty()
        || names.len() != expected.len()
        || expected != actual
        || starts.len() != names.len()
    {
        return Err(
            "Metal discovered and executed inventories differ or are empty/duplicated".into(),
        );
    }
    for (index, record) in starts.iter().enumerate() {
        let begin = record.get(0).unwrap().end();
        let end = starts
            .get(index + 1)
            .map_or(execution.len(), |next| next.get(0).unwrap().start());
        let output = execution[begin..end]
            .split("\ntest result:")
            .next()
            .unwrap()
            .trim();
        if output != "ok" && !output.ends_with("\nok") {
            return Err("Metal test did not terminate successfully".into());
        }
    }
    let summary = format!(
        "test result: ok. {} passed; 0 failed; 0 ignored;",
        names.len()
    );
    if execution.matches(&summary).count() != 1 || execution.matches("test result:").count() != 1 {
        return Err("Metal summary must contain every discovered test with no skips".into());
    }
    Ok(json!({"discovered":expected,"executed":actual,"testCount":names.len(),"noSkips":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metal_inventory_rejects_empty_skips_duplicates_and_missing_tests() {
        let log = "test a ... GPU log\nok\ntest b ... ok\ntest result: ok. 2 passed; 0 failed; 0 ignored;";
        assert!(assess("a: test\nb: test\n", log).is_ok());
        for list in ["", "a: test\n", "a: test\na: test\n"] {
            assert!(assess(list, log).is_err());
        }
        assert!(assess("a: test\nb: test\n", &log.replace("0 ignored", "1 ignored")).is_err());
    }

    #[test]
    fn ctest_inventory_rejects_missing_and_skipped_tests() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("ctest-inventory.log"),
            r#"{"tests":[{"name":"a"},{"name":"b"}]}"#,
        )
        .unwrap();
        let log = "1/2 Test #1: a ... Passed\n2/2 Test #2: b ... Passed\n100% tests passed, 0 tests failed out of 2";
        fs::write(dir.path().join("ctest.log"), log).unwrap();
        assert!(ctest(dir.path()).is_ok());
        fs::write(
            dir.path().join("ctest.log"),
            log.replace("Passed", "***Skipped"),
        )
        .unwrap();
        assert!(ctest(dir.path()).is_err());
        fs::write(
            dir.path().join("ctest.log"),
            log.replace("2/2 Test #2: b ... Passed", ""),
        )
        .unwrap();
        assert!(ctest(dir.path()).is_err());
    }

    #[test]
    fn workspace_records_children_but_refuses_missing_or_unknown_tests() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("workspace-inventory.log"),
            "a: test\nb: test\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("workspace.log"),
            "test a ... ok\ntest b ... ignored\ntest a ... ok\n",
        )
        .unwrap();
        let evidence = workspace(dir.path()).unwrap();
        assert_eq!(evidence["additionalChildRecords"]["a"], 1);
        fs::write(
            dir.path().join("workspace.log"),
            "test a ... test a ... ok\noktest b ... ok\n",
        )
        .unwrap();
        let interleaved = workspace(dir.path()).unwrap();
        assert_eq!(interleaved["additionalChildRecords"]["a"], 1);
        fs::write(
            dir.path().join("workspace.log"),
            "test a ... test a ... ok\ntest result: ok. 1 passed; 964 filtered outtest b ... ok\n",
        )
        .unwrap();
        assert_eq!(
            workspace(dir.path()).unwrap()["additionalChildRecords"]["a"],
            1
        );
        for log in [
            "test a ... ok\n",
            "test a ... ok\ntest b ... ok\ntest c ... ok\n",
        ] {
            fs::write(dir.path().join("workspace.log"), log).unwrap();
            assert!(workspace(dir.path()).is_err());
        }
    }

    #[test]
    fn workspace_preserves_marker_text_inside_names_and_doctest_labels() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("workspace-inventory.log"),
            "latest: test\ncrates/a.rs - module::latest (line 19): test\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("workspace.log"),
            "test latest ... oktest crates/a.rs - module::latest (line 19) ... ok\n",
        )
        .unwrap();
        let evidence = workspace(dir.path()).unwrap();
        assert_eq!(evidence["observedHarnessRecords"]["latest"], 1);
        assert_eq!(
            evidence["observedHarnessRecords"]["crates/a.rs - module::latest (line 19)"],
            1
        );
    }
}
