use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::{AppError, AppResult};

const MAX_RESULTS_BYTES: u64 = 512 * 1024 * 1024;

pub fn report(results: &Path, summary_path: &Path) -> AppResult<i32> {
    let records = load_jsonl(results)?;
    let summary = summarize(&records);
    let bytes =
        serde_json::to_vec_pretty(&summary).map_err(|e| AppError::cli(format!("summary: {e}")))?;
    std::fs::write(summary_path, bytes)
        .map_err(|e| AppError::environment(format!("summary: {e}")))?;
    print_table(&summary);
    Ok(if summary["gatingFailures"].as_u64().unwrap_or(0) > 0 {
        1
    } else {
        0
    })
}

fn load_jsonl(path: &Path) -> AppResult<Vec<Value>> {
    let file = std::fs::File::open(path).map_err(|e| AppError::cli(format!("results: {e}")))?;
    let size = file
        .metadata()
        .map_err(|e| AppError::cli(format!("results: {e}")))?
        .len();
    if size > MAX_RESULTS_BYTES {
        return Err(AppError::cli("results exceed 512 MiB"));
    }
    let mut records = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| AppError::cli(format!("results line {}: {e}", index + 1)))?;
        if line.len() > 16 * 1024 * 1024 {
            return Err(AppError::cli("result line exceeds 16 MiB"));
        }
        if line.trim().is_empty() {
            continue;
        }
        records.push(parse_record(&line, index + 1)?);
    }
    if records.is_empty() {
        return Err(AppError::cli("results contain no records"));
    }
    Ok(records)
}

fn parse_record(line: &str, line_number: usize) -> AppResult<Value> {
    let value: Value = serde_json::from_str(line)
        .map_err(|e| AppError::cli(format!("results line {line_number}: {e}")))?;
    if !crate::result_validation::validate_persisted(&value) {
        return Err(AppError::cli(format!(
            "results line {line_number}: result schema"
        )));
    }
    Ok(value)
}

fn summarize(records: &[Value]) -> Value {
    let mut verdict = BTreeMap::new();
    let mut domain = BTreeMap::new();
    let mut generator = BTreeMap::new();
    let mut backend = BTreeMap::new();
    let mut failure = BTreeMap::new();
    let mut differential = BTreeMap::new();
    let mut timing = Vec::new();
    let mut quality = Vec::new();
    for record in records {
        increment(&mut verdict, text(record, "/verdict"));
        increment(&mut domain, text(record, "/expectedDomain"));
        increment(&mut generator, text(record, "/generator/name"));
        increment(&mut backend, text(record, "/backend"));
        increment(&mut failure, text(record, "/failureClass"));
        increment(
            &mut differential,
            text(record, "/differential/classification"),
        );
        push_number(record, "/timing/wallMs", &mut timing);
        push_tolerance(record, &mut quality);
    }
    let transitions = unique_transitions(records);
    json!({
        "schemaVersion":1,
        "records":records.len(),
        "gatingFailures":verdict.get("fail").copied().unwrap_or(0),
        "counts":{"verdict":verdict,"domain":domain,"generator":generator,"backend":backend,"failureClass":failure},
        "differential":differential,
        "replay":{"stable":records.iter().filter(|value| value.pointer("/replay/stable").and_then(Value::as_bool)==Some(true)).count(),"unstable":records.iter().filter(|value| value.pointer("/replay/stable").and_then(Value::as_bool)==Some(false)).count()},
        "metamorph":{"passed":records.iter().filter(|value| value.pointer("/metamorph/status").and_then(Value::as_str)==Some("pass")).count(),"failed":records.iter().filter(|value| value.pointer("/metamorph/status").and_then(Value::as_str)==Some("fail")).count()},
        "criticalTransitions":transitions,
        "timingMs":{"p50":percentile(&mut timing,0.50),"p95":percentile(&mut timing,0.95)},
        "auditQuality":{"maxToleranceP50":percentile(&mut quality,0.50),"maxToleranceP95":percentile(&mut quality,0.95)}
    })
}

fn unique_transitions(records: &[Value]) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for record in records {
        let Some(search) = record.get("search").filter(|value| !value.is_null()) else {
            continue;
        };
        let key = format!(
            "{}|{}|{}",
            text(record, "/caseId"),
            text(record, "/backend"),
            search
        );
        if seen.insert(key) {
            result.push(search.clone());
        }
    }
    result
}

fn push_tolerance(value: &Value, out: &mut Vec<f64>) {
    for kind in ["vertex", "edge", "face"] {
        push_number(
            value,
            &format!("/outputAudit/tolerances/{kind}/maximum"),
            out,
        );
    }
}

fn increment(map: &mut BTreeMap<String, usize>, key: String) {
    if !key.is_empty() {
        *map.entry(key).or_default() += 1;
    }
}

fn text(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn push_number(value: &Value, pointer: &str, out: &mut Vec<f64>) {
    if let Some(number) = value
        .pointer(pointer)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
    {
        out.push(number);
    }
}

fn percentile(values: &mut [f64], fraction: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let index = (fraction * values.len() as f64).ceil() as usize - 1;
    values.get(index).copied()
}

fn print_table(summary: &Value) {
    eprintln!("records fail pass char");
    eprintln!(
        "{} {} {} {}",
        count(summary, "/records"),
        count(summary, "/counts/verdict/fail"),
        count(summary, "/counts/verdict/pass"),
        count(summary, "/counts/verdict/characterization")
    );
    eprintln!(
        "rescued={} regressions={} replay-unstable={}",
        count(summary, "/differential/rescued"),
        count(summary, "/differential/onecad-regression"),
        count(summary, "/replay/unstable")
    );
}

fn count(value: &Value, pointer: &str) -> u64 {
    value.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::{refresh_digest, synthetic_result, Backend};

    #[test]
    fn percentile_is_deterministic() {
        let mut values = vec![3.0, 1.0, 2.0, 4.0];
        assert_eq!(percentile(&mut values, 0.5), Some(2.0));
    }

    fn valid_record() -> Value {
        let case = crate::suite::t0().into_iter().next().unwrap().case;
        let mut record = synthetic_result(
            &case,
            Backend::RawOcct,
            "timeout",
            json!({"state": "timeout"}),
            "",
        );
        record["inputDigest"] = json!("0".repeat(64));
        refresh_digest(&mut record);
        record
    }

    fn write_lines(directory: &Path, lines: &[String]) -> std::path::PathBuf {
        let path = directory.join("results.jsonl");
        std::fs::write(&path, lines.join("\n")).expect("write jsonl fixture");
        path
    }

    #[test]
    fn report_rejects_empty_jsonl() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let results = write_lines(directory.path(), &["".into(), "  ".into()]);
        let summary = directory.path().join("summary.json");
        assert!(report(&results, &summary).is_err());
    }

    #[test]
    fn report_rejects_scalar_json_line() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let results = write_lines(directory.path(), &["42".into()]);
        let summary = directory.path().join("summary.json");
        assert!(report(&results, &summary).is_err());
    }

    #[test]
    fn report_rejects_unknown_field() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let mut record = valid_record();
        record["bogusField"] = json!(1);
        let results = write_lines(directory.path(), &[record.to_string()]);
        let summary = directory.path().join("summary.json");
        assert!(report(&results, &summary).is_err());
    }

    #[test]
    fn report_accepts_valid_campaign_and_search_records() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let mut campaign_record = valid_record();
        campaign_record["campaignVariant"] = json!({"name": "base"});
        campaign_record["replay"] = json!({"stable": true, "digest": "a".repeat(64)});
        campaign_record["differential"] = json!({"classification": "same-status"});

        let mut search_record = valid_record();
        search_record["search"] = json!({
            "parameter": "operation.radius",
            "probeRadius": 1.5,
            "probeCount": 4,
            "monotonicObserved": true,
            "transitions": []
        });
        refresh_digest(&mut search_record);

        let results = write_lines(
            directory.path(),
            &[campaign_record.to_string(), search_record.to_string()],
        );
        let summary = directory.path().join("summary.json");
        // A synthetic timeout record is deliberately a gating failure (exit
        // 1); what this test proves is that schema-valid campaign/search
        // wrapper fields are accepted at all, not the gate outcome.
        report(&results, &summary).expect("valid records must pass schema validation");
    }
}
