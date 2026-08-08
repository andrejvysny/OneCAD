#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use onecad_kernelbench::case::Case;
use onecad_kernelbench::result::Backend;
use onecad_kernelbench::runner;
use onecad_kernelbench::suite::{Variant, VariantName};
use serde_json::Value;
use tempfile::TempDir;

fn fixture() -> Case {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../bench/robustness/regressions/fillet-box-supported-single.json");
    Case::load(&path).expect("fixture must remain valid")
}

fn script(directory: &Path, body: &str) -> PathBuf {
    let path = directory.join("runner.sh");
    std::fs::write(&path, format!("#!/bin/sh\nread request\n{body}\n")).expect("write test runner");
    let mut permissions = path.metadata().expect("runner metadata").permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).expect("make test runner executable");
    path
}

fn execute(body: &str, configure: impl FnOnce(&mut Case)) -> Value {
    let directory = TempDir::new().expect("temporary directory");
    let runner_path = script(directory.path(), body);
    let mut case = fixture();
    configure(&mut case);
    let variant = Variant {
        name: VariantName::Base,
        translation: None,
        rotation: None,
    };
    runner::execute(
        &runner_path,
        &case.prepared(),
        Backend::RawOcct,
        &variant,
        directory.path(),
    )
}

fn assert_failure(result: &Value, execution: &str, class: &str) {
    assert_eq!(result["verdict"], "fail");
    assert_eq!(result["executionState"], execution);
    assert_eq!(result["failureClass"], class);
    assert_eq!(result["operationState"], "notRun");
}

#[test]
fn invalid_json_is_structured_and_campaign_can_continue() {
    let invalid = execute("printf 'not-json\\n'", |_| {});
    assert_failure(&invalid, "exception", "schema");
    let crash = execute("kill -11 $$", |_| {});
    assert_failure(&crash, "crash", "crash");
}

#[test]
fn timeout_is_killed_and_reaped() {
    let result = execute("sleep 1", |case| {
        case.limits.resources.wall_time_ms = 25;
    });
    assert_failure(&result, "timeout", "timeout");
}

#[test]
fn stdout_overflow_is_bounded() {
    let result = execute("printf '%0256d%0256d' 0 0", |case| {
        case.limits.resources.stdout_bytes = 256;
    });
    assert_failure(&result, "outputOverflow", "outputOverflow");
}

#[test]
fn supervisor_artifacts_respect_cumulative_cap() {
    let directory = TempDir::new().expect("temporary directory");
    let runner_path = script(directory.path(), "kill -11 $$");
    let mut case = fixture();
    case.limits.resources.artifact_bytes = 200;
    let variant = Variant {
        name: VariantName::Base,
        translation: None,
        rotation: None,
    };
    let result = runner::execute(
        &runner_path,
        &case.prepared(),
        Backend::RawOcct,
        &variant,
        directory.path(),
    );
    assert_failure(&result, "crash", "crash");
    // Case/request JSON dumps are each far larger than 200 bytes, so a
    // supervisor that truly enforces the cap while writing (not just after)
    // must refuse both rather than write them and only flip the verdict.
    assert!(result["artifacts"]["canonicalCase"].is_null());
    assert!(result["artifacts"]["request"].is_null());
    assert!(directory_bytes(directory.path()) <= 200);
}

fn directory_bytes(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .expect("read artifact directory")
        .map(|entry| {
            entry
                .expect("dir entry")
                .metadata()
                .expect("metadata")
                .len()
        })
        .sum()
}

#[cfg(target_os = "macos")]
#[test]
fn resident_memory_limit_is_enforced() {
    let result = execute(
        "exec python3 -c 'import time; data=bytearray(67108864); time.sleep(1)'",
        |case| case.limits.resources.address_space_bytes = 16 * 1024 * 1024,
    );
    assert_failure(&result, "memoryLimit", "memoryLimit");
}
