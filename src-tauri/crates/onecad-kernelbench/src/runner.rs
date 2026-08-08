use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::child;
use crate::digest::sha256_value;
use crate::prepared::PreparedCase;
use crate::result::{complete_result, refresh_digest, Backend};
use crate::suite::Variant;
use crate::{AppError, AppResult};

#[cfg(windows)]
pub fn resolve_runner(_explicit: Option<PathBuf>) -> AppResult<PathBuf> {
    Err(AppError::environment(
        "kernelbench process isolation is unsupported on Windows",
    ))
}

#[cfg(not(windows))]
pub fn resolve_runner(explicit: Option<PathBuf>) -> AppResult<PathBuf> {
    if let Some(path) = explicit {
        return validate_runner(path);
    }
    if let Some(path) = std::env::var_os("ONECAD_KERNELBENCH_RUNNER") {
        return validate_runner(PathBuf::from(path));
    }
    let executable = std::env::current_exe()
        .map_err(|e| AppError::environment(format!("current executable: {e}")))?;
    let sibling = executable.with_file_name(runner_name());
    validate_runner(sibling)
}

fn validate_runner(path: PathBuf) -> AppResult<PathBuf> {
    if path.is_file() && executable(&path) {
        Ok(path)
    } else {
        Err(AppError::environment(format!(
            "runner not found: {}",
            path.display()
        )))
    }
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}

fn runner_name() -> &'static str {
    if cfg!(windows) {
        "onecad-kernelbench-runner.exe"
    } else {
        "onecad-kernelbench-runner"
    }
}

pub fn execute(
    runner: &Path,
    case: &PreparedCase,
    backend: Backend,
    variant: &Variant,
    artifact_dir: &Path,
) -> Value {
    let input_value = json!({"case":case.json,"variant":variant});
    let input_digest = sha256_value(&input_value);
    let request = json!({"schemaVersion":1,"case":case.json,"backend":backend.wire(),"variant":variant,"artifactDir":artifact_dir});
    let mut value = match child::run(runner, case, backend, &request) {
        Ok(value) => complete_result(value, case, backend, &input_digest),
        Err(value) => complete_result(value, case, backend, &input_digest),
    };
    retain_supervisor_artifacts(&mut value, case, &request, artifact_dir);
    enforce_artifacts(&mut value, case, artifact_dir);
    refresh_digest(&mut value);
    value
}

fn enforce_artifacts(value: &mut Value, case: &PreparedCase, directory: &Path) {
    let safe_paths = value
        .get("artifacts")
        .and_then(Value::as_object)
        .is_some_and(|paths| paths.values().all(safe_relative_artifact));
    let bytes = directory_bytes(directory).unwrap_or(u64::MAX);
    if !safe_paths || bytes > case.limits.resources.artifact_bytes {
        value["verdict"] = json!("fail");
        value["failureClass"] = json!("internal");
    }
}

fn retain_supervisor_artifacts(
    value: &mut Value,
    case: &PreparedCase,
    request: &Value,
    directory: &Path,
) {
    if value.get("executionState").and_then(Value::as_str) == Some("completed") {
        return;
    }
    if std::fs::create_dir_all(directory).is_err() {
        return;
    }
    // Cumulative budget, not a per-file cap: whatever the runner already left
    // in this directory (e.g. an input.brep written before it was killed)
    // counts against the same configured artifactBytes ceiling.
    let cap = case.limits.resources.artifact_bytes;
    let mut remaining = cap.saturating_sub(directory_bytes(directory).unwrap_or(cap));
    write_json_artifact(
        directory,
        "case.json",
        &case.json,
        value,
        "canonicalCase",
        &mut remaining,
    );
    write_json_artifact(
        directory,
        "request.json",
        request,
        value,
        "request",
        &mut remaining,
    );
    let stderr = value
        .pointer("/diagnostics/0/message")
        .and_then(Value::as_str)
        .unwrap_or("");
    if write_bytes_artifact(directory, "stderr.txt", stderr.as_bytes(), &mut remaining) {
        value["artifacts"]["stderr"] = json!("stderr.txt");
    }
    for (field, name) in [("inputShape", "input.brep"), ("outputShape", "output.brep")] {
        if directory.join(name).is_file() {
            value["artifacts"][field] = json!(name);
        }
    }
}

fn write_json_artifact(
    directory: &Path,
    name: &str,
    payload: &Value,
    result: &mut Value,
    field: &str,
    remaining: &mut u64,
) {
    let Ok(bytes) = serde_json::to_vec_pretty(payload) else {
        return;
    };
    if write_bytes_artifact(directory, name, &bytes, remaining) {
        result["artifacts"][field] = json!(name);
    }
}

fn write_bytes_artifact(directory: &Path, name: &str, bytes: &[u8], remaining: &mut u64) -> bool {
    let size = bytes.len() as u64;
    if size > *remaining || std::fs::write(directory.join(name), bytes).is_err() {
        return false;
    }
    *remaining -= size;
    true
}

fn safe_relative_artifact(value: &Value) -> bool {
    if value.is_null() {
        return true;
    }
    let Some(path) = value.as_str().map(Path::new) else {
        return false;
    };
    !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
}

fn directory_bytes(path: &Path) -> std::io::Result<u64> {
    let mut total = 0_u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        total = total.saturating_add(if metadata.is_dir() {
            directory_bytes(&entry.path())?
        } else {
            metadata.len()
        });
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn non_executable_runner_path_is_environment_error() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let path = directory.path().join("not-executable");
        std::fs::write(&path, b"").expect("write stub file");
        let error = resolve_runner(Some(path)).expect_err("must reject non-executable file");
        assert_eq!(error.code, 3);
    }
}
