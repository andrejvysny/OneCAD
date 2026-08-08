use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use crate::case::Case;
use crate::metamorph;
use crate::result::{attach_replay, is_red, refresh_digest, Backend};
use crate::runner;
use crate::suite::{GeneratedCase, Variant};
use crate::{AppError, AppResult};

pub fn run_cases(
    runner_path: &Path,
    generated: &[GeneratedCase],
    backends: &[Backend],
    out_dir: &Path,
) -> AppResult<i32> {
    std::fs::create_dir_all(out_dir).map_err(|e| AppError::environment(format!("out dir: {e}")))?;
    let mut records = Vec::with_capacity(generated.len() * backends.len());
    for item in generated {
        for backend in backends {
            records.push(run_repeated(
                runner_path,
                &item.case,
                *backend,
                &item.variant,
                out_dir,
            ));
        }
    }
    apply_metamorph(&mut records, generated);
    apply_differential(&mut records);
    records.iter_mut().for_each(refresh_digest);
    write_jsonl(&out_dir.join("results.jsonl"), &records)?;
    Ok(if records.iter().any(is_red) { 1 } else { 0 })
}

fn apply_metamorph(records: &mut [Value], generated: &[GeneratedCase]) {
    let tolerances: BTreeMap<&str, f64> = generated
        .iter()
        .map(|item| {
            (
                item.case.case_id.as_str(),
                metamorph::point_tolerance(&item.case),
            )
        })
        .collect();
    let mut groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (index, record) in records.iter().enumerate() {
        groups
            .entry((text(record, "caseId"), text(record, "backend")))
            .or_default()
            .push(index);
    }
    for indices in groups.values() {
        let Some(base) = indices
            .iter()
            .copied()
            .find(|index| variant_name(&records[*index]) == "base")
        else {
            continue;
        };
        let tolerance = tolerances
            .get(text(&records[base], "caseId").as_str())
            .copied()
            .unwrap_or(1e-6);
        for index in indices.iter().copied().filter(|index| *index != base) {
            attach_metamorph(records, base, index, tolerance);
        }
    }
}

fn attach_metamorph(records: &mut [Value], base: usize, variant: usize, tolerance: f64) {
    let status_match =
        text(&records[base], "operationState") == text(&records[variant], "operationState");
    let properties_match = invariant_number_match(
        &records[base],
        &records[variant],
        &[
            "/outputAudit/massProperties/volume",
            "/outputAudit/massProperties/area",
        ],
    );
    let semantic_match =
        validator_signature(&records[base]) == validator_signature(&records[variant]);
    let evidence = metamorph::compare(&records[base], &records[variant], tolerance);
    let (surface_samples_match, point_classification_match) = evidence
        .as_ref()
        .map(|result| {
            (
                result.surface_samples_match,
                result.point_classification_match,
            )
        })
        // Absence of geometry on both sides (e.g. both variants correctly
        // refused an expected-limit case) is not evidence of a mismatch.
        .unwrap_or((true, true));
    let shape_ok = evidence
        .as_ref()
        .is_none_or(|result| result.surface_samples_match && result.point_classification_match);
    let passed = status_match && properties_match && semantic_match && shape_ok;
    let status = if evidence.is_none() {
        "notRun"
    } else if passed {
        "pass"
    } else {
        "fail"
    };
    let name = variant_name(&records[variant]);
    records[variant]["metamorph"] = json!({
        "variant": name,
        "status": status,
        "normalizedPropertiesMatch": properties_match,
        "semanticEvidenceMatch": semantic_match,
        "surfaceSamplesMatch": surface_samples_match,
        "pointClassificationMatch": point_classification_match,
    });
    if !passed
        && matches!(
            text(&records[variant], "expectedDomain").as_str(),
            "supported" | "expectedLimit"
        )
    {
        records[variant]["verdict"] = json!("fail");
        records[variant]["failureClass"] = json!("semanticFailed");
    }
}

fn invariant_number_match(left: &Value, right: &Value, pointers: &[&str]) -> bool {
    pointers.iter().all(|pointer| {
        let a = left.pointer(pointer).and_then(Value::as_f64);
        let b = right.pointer(pointer).and_then(Value::as_f64);
        match (a, b) {
            (Some(a), Some(b)) => (a - b).abs() <= 1e-7_f64.max(a.abs() * 1e-9),
            (None, None) => true,
            _ => false,
        }
    })
}

fn validator_signature(value: &Value) -> Vec<(String, String)> {
    value
        .get("validators")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            let kind = item
                .get("kind")
                .or_else(|| item.get("type"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    item.get("passed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        .to_string()
                });
            (kind, status)
        })
        .collect()
}

pub fn run_one(
    runner_path: &Path,
    case: Case,
    variant: Variant,
    backends: &[Backend],
    out_dir: &Path,
) -> AppResult<i32> {
    run_cases(
        runner_path,
        &[GeneratedCase { case, variant }],
        backends,
        out_dir,
    )
}

fn run_repeated(
    runner_path: &Path,
    case: &Case,
    backend: Backend,
    variant: &Variant,
    out_dir: &Path,
) -> Value {
    let root = artifact_dir(out_dir, case, backend, variant);
    let canonical_dir = root.join("canonical");
    let replay_dir = root.join("replay");
    let _ = std::fs::create_dir_all(&canonical_dir);
    let _ = std::fs::create_dir_all(&replay_dir);
    let mut canonical = runner::execute(runner_path, case, backend, variant, &canonical_dir);
    let mut replay = runner::execute(runner_path, case, backend, variant, &replay_dir);
    rebase_artifacts(&mut canonical, out_dir, &canonical_dir);
    rebase_artifacts(&mut replay, out_dir, &replay_dir);
    canonical["campaignVariant"] = json!(variant);
    replay["campaignVariant"] = json!(variant);
    refresh_digest(&mut canonical);
    refresh_digest(&mut replay);
    attach_replay(canonical, &replay)
}

pub(crate) fn artifact_dir(
    out_dir: &Path,
    case: &Case,
    backend: Backend,
    variant: &Variant,
) -> std::path::PathBuf {
    let variant_name = serde_json::to_value(variant.name)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "base".into());
    out_dir
        .join("artifacts")
        .join(&case.case_id)
        .join(variant_name)
        .join(backend.wire())
}

pub(crate) fn rebase_artifacts(value: &mut Value, out_dir: &Path, artifact_dir: &Path) {
    let Some(artifacts) = value.get_mut("artifacts").and_then(Value::as_object_mut) else {
        return;
    };
    for path in artifacts.values_mut() {
        let Some(relative) = path.as_str() else {
            continue;
        };
        let absolute = artifact_dir.join(relative);
        if let Ok(rebased) = absolute.strip_prefix(out_dir) {
            *path = json!(portable_path(rebased));
        }
    }
}

fn portable_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn apply_differential(records: &mut [Value]) {
    let mut pairs: BTreeMap<(String, String), (Option<usize>, Option<usize>)> = BTreeMap::new();
    for (index, record) in records.iter().enumerate() {
        let key = (text(record, "caseId"), variant_name(record));
        let entry = pairs.entry(key).or_default();
        if text(record, "backend") == "raw-occt" {
            entry.0 = Some(index);
        } else {
            entry.1 = Some(index);
        }
    }
    for (_, (raw, onecad)) in pairs {
        let (Some(raw), Some(onecad)) = (raw, onecad) else {
            continue;
        };
        let input_match =
            text(&records[raw], "inputDigest") == text(&records[onecad], "inputDigest");
        let raw_pass = verdict(&records[raw]) == "pass";
        let onecad_pass = verdict(&records[onecad]) == "pass";
        let states_match =
            text(&records[raw], "operationState") == text(&records[onecad], "operationState");
        let supported = text(&records[onecad], "expectedDomain") == "supported";
        let classification = if !input_match {
            "input-mismatch"
        } else if !raw_pass && onecad_pass {
            "rescued"
        } else if supported && raw_pass && !onecad_pass {
            "onecad-regression"
        } else if !states_match || raw_pass != onecad_pass {
            "status-difference"
        } else {
            "same-status"
        };
        records[raw]["differential"] = json!({"classification":classification});
        records[onecad]["differential"] = json!({"classification":classification});
        if classification == "input-mismatch" {
            mark_input_mismatch(&mut records[raw]);
            mark_input_mismatch(&mut records[onecad]);
        } else if classification == "onecad-regression" {
            records[onecad]["verdict"] = json!("fail");
            records[onecad]["failureClass"] = json!("semanticFailed");
        }
    }
}

fn mark_input_mismatch(record: &mut Value) {
    record["verdict"] = json!("fail");
    record["failureClass"] = json!("inputMismatch");
}

fn variant_name(value: &Value) -> String {
    value
        .pointer("/campaignVariant/name")
        .and_then(Value::as_str)
        .unwrap_or("base")
        .to_string()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn verdict(value: &Value) -> &str {
    value
        .get("verdict")
        .and_then(Value::as_str)
        .unwrap_or("fail")
}

pub fn write_jsonl(path: &Path, records: &[Value]) -> AppResult<()> {
    let mut file =
        std::fs::File::create(path).map_err(|e| AppError::environment(format!("results: {e}")))?;
    for record in records {
        serde_json::to_writer(&mut file, record)
            .map_err(|e| AppError::environment(format!("results: {e}")))?;
        file.write_all(b"\n")
            .map_err(|e| AppError::environment(format!("results: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(backend: &str, verdict: &str) -> Value {
        json!({
            "caseId":"case-01",
            "backend":backend,
            "verdict":verdict,
            "failureClass":if verdict == "pass" {"none"} else {"kernelRejected"},
            "expectedDomain":"supported",
            "operationState":if verdict == "pass" {"success"} else {"rejected"},
            "inputDigest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "campaignVariant":{"name":"base"}
        })
    }

    #[test]
    fn differential_classifies_rescue() {
        let mut records = vec![record("raw-occt", "fail"), record("onecad", "pass")];
        apply_differential(&mut records);
        assert_eq!(
            records[1].pointer("/differential/classification"),
            Some(&json!("rescued"))
        );
        assert_eq!(records[1]["verdict"], "pass");
    }

    #[test]
    fn differential_gates_onecad_regression() {
        let mut records = vec![record("raw-occt", "pass"), record("onecad", "fail")];
        apply_differential(&mut records);
        assert_eq!(
            records[1].pointer("/differential/classification"),
            Some(&json!("onecad-regression"))
        );
        assert_eq!(records[1]["failureClass"], "semanticFailed");
    }

    #[test]
    fn exploratory_status_difference_does_not_create_regression() {
        let mut records = vec![record("raw-occt", "pass"), record("onecad", "fail")];
        records[0]["expectedDomain"] = json!("exploratory");
        records[1]["expectedDomain"] = json!("exploratory");
        records[0]["inputDigest"] = json!("a");
        records[1]["inputDigest"] = json!("a");
        apply_differential(&mut records);
        assert_eq!(
            records[1].pointer("/differential/classification"),
            Some(&json!("status-difference"))
        );
    }

    #[test]
    fn differential_gates_input_mismatch() {
        let mut records = vec![record("raw-occt", "pass"), record("onecad", "pass")];
        records[0]["inputDigest"] = json!("a");
        records[1]["inputDigest"] = json!("b");
        apply_differential(&mut records);
        assert!(records
            .iter()
            .all(|value| value["failureClass"] == "inputMismatch"));
    }
}
