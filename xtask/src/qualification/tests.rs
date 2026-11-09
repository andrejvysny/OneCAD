use super::{campaign::REQUIRED, manifest, PYTHON_RUNNER, PYTHON_SUITES};
use serde_json::json;
use std::{fs, path::Path, process::Command};

#[test]
fn python_suites_are_explicit_and_present() {
    assert_eq!(PYTHON_SUITES.len(), 6);
    for suite in PYTHON_SUITES {
        assert!(std::path::Path::new("..").join(suite).is_file(), "{suite}");
    }
}

#[test]
fn python_runner_rejects_empty_failed_skipped_and_missing_suites() {
    let dir = tempfile::tempdir().unwrap();
    for (body, pass) in [
        ("", false),
        ("import unittest\nclass T(unittest.TestCase):\n def test_ok(self): pass\n", true),
        ("import unittest\nclass T(unittest.TestCase):\n def test_bad(self): self.fail()\n", false),
        ("import unittest\nclass T(unittest.TestCase):\n @unittest.skip('no')\n def test_skip(self): pass\n", false),
    ] {
        let path = dir.path().join("test_suite.py");
        fs::write(&path, body).unwrap();
        let output = Command::new("python3").args(["-c", PYTHON_RUNNER]).arg(path)
            .env("PYTHONDONTWRITEBYTECODE", "1").output().unwrap();
        assert_eq!(output.status.success(), pass);
    }
    assert!(!Command::new("python3")
        .args(["-c", PYTHON_RUNNER])
        .arg(dir.path().join("missing.py"))
        .output()
        .unwrap()
        .status
        .success());
}

#[test]
fn incomplete_campaign_cannot_be_verified() {
    let dir = tempfile::tempdir().unwrap();
    let phases: Vec<_> = REQUIRED
        .iter()
        .map(|name| json!({"name":name,"passed":true}))
        .collect();
    let mut data = json!({"schemaVersion":1,"status":"incomplete","phases":phases});
    fs::write(
        dir.path().join("campaign.json"),
        serde_json::to_vec(&data).unwrap(),
    )
    .unwrap();
    assert!(manifest::verify(dir.path())
        .unwrap_err()
        .to_string()
        .contains("incomplete"));
    data["status"] = json!("passed");
    data["phases"].as_array_mut().unwrap().pop();
    fs::write(
        dir.path().join("campaign.json"),
        serde_json::to_vec(&data).unwrap(),
    )
    .unwrap();
    assert!(manifest::verify(dir.path())
        .unwrap_err()
        .to_string()
        .contains("incomplete"));
}

#[test]
fn artifact_inventory_detects_content_and_added_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("phase.log"), "passed").unwrap();
    let before = manifest::artifacts(dir.path()).unwrap();
    fs::write(dir.path().join("phase.log"), "failed").unwrap();
    assert_ne!(before, manifest::artifacts(dir.path()).unwrap());
    fs::write(dir.path().join("extra.log"), "extra").unwrap();
    assert_eq!(manifest::artifacts(dir.path()).unwrap().len(), 2);
}

#[test]
fn only_metal_phase_receives_capture_directory_even_with_inherited_override() {
    let out = Path::new("/temporary/qualification with spaces");
    for name in REQUIRED {
        let script = if *name == "metal" {
            "test \"$ONECAD_RENDER_CAPTURE_DIR\" = \"$1\""
        } else {
            "test -z \"${ONECAD_RENDER_CAPTURE_DIR+x}\""
        };
        let expected = out.join("metal");
        let args = super::campaign::phase_arguments(
            name,
            "sh",
            &["-c", script, "capture-check", expected.to_str().unwrap()],
            out,
        );
        assert!(
            Command::new("env")
                .args(args)
                .env("ONECAD_RENDER_CAPTURE_DIR", "/unrelated/captures")
                .status()
                .unwrap()
                .success(),
            "{name}"
        );
    }
}

#[test]
fn shipping_aggregate_requires_linux_only_for_trusted_events() {
    let workflow = include_str!("../../../.github/workflows/ci.yml");
    let script = workflow
        .split("python3 - <<'PYTHON'\n")
        .nth(1)
        .unwrap()
        .split("          PYTHON")
        .next()
        .unwrap()
        .lines()
        .map(|line| line.strip_prefix("          ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    for (trusted, hosted, linux, passed) in [
        ("true", "success", "success", true),
        ("true", "success", "skipped", false),
        ("true", "success", "cancelled", false),
        ("true", "skipped", "success", false),
        ("false", "success", "skipped", true),
        ("false", "success", "success", false),
        ("false", "failure", "skipped", false),
    ] {
        let jobs = json!({"architecture":{"result":hosted},"linux-worker":{"result":linux},"linux-kernelbench":{"result":linux}});
        let output = Command::new("python3")
            .args(["-c", &script])
            .env("REQUIRED_RESULTS", jobs.to_string())
            .env("REQUIRE_LINUX", trusted)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            passed,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
