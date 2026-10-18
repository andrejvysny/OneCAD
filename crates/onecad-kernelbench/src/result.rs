use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::case::{ExpectedDomain, Generator};
use crate::digest::normalized_digest;
use crate::prepared::PreparedCase;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    RawOcct,
    Onecad,
}

impl Backend {
    pub fn wire(self) -> &'static str {
        match self {
            Self::RawOcct => "raw-occt",
            Self::Onecad => "onecad",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Pass,
    Fail,
    Characterization,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayEvidence {
    pub stable: bool,
    pub digest: String,
}

pub fn synthetic_result(
    case: &PreparedCase,
    backend: Backend,
    failure: &str,
    execution: Value,
    stderr: &str,
) -> Value {
    let verdict = failure_verdict(case.expected_domain, failure);
    let execution_state = execution_state(failure);
    let failure_class = failure_class(failure);
    let diagnostic = bounded_diagnostic(stderr);
    json!({
        "schemaVersion": 1,
        "caseId": case.case_id,
        "generator": case.generator,
        "backend": backend,
        "kernel": {"name":"unknown","version":"unknown","buildId":"unknown"},
        "modeler": {"name":"onecad-kernelbench-runner","version":"unknown","buildId":"unknown"},
        "expectedDomain": case.expected_domain,
        "verdict": verdict,
        "executionState": execution_state,
        "operationState": "notRun",
        "failureClass": failure_class,
        "diagnostics": [diagnostic],
        "inputAudit": Value::Null,
        "outputAudit": Value::Null,
        "validators": [],
        "selectionEvidence": [],
        "timing": {"wallMs":0.0,"operationMs":0.0,"auditMs":0.0},
        "resource": {
            "peakRssBytes": execution.get("peakRssBytes").cloned().unwrap_or(Value::Null),
            "exitCode": execution.get("exitCode").cloned().unwrap_or(Value::Null),
            "signal": execution.get("signal").cloned().unwrap_or(Value::Null),
            "stderrTruncated": false
        },
        "metamorph": Value::Null,
        "search": Value::Null,
        "shapeSignature": Value::Null,
        "artifacts": empty_artifacts()
    })
}

fn execution_state(failure: &str) -> &'static str {
    match failure {
        "timeout" => "timeout",
        "memory-limit" => "memoryLimit",
        "output-overflow" => "outputOverflow",
        "crash" => "crash",
        "spawn-error" | "stdin-error" | "wait-error" => "environmentError",
        _ => "exception",
    }
}

fn failure_class(failure: &str) -> &'static str {
    match failure {
        "timeout" => "timeout",
        "memory-limit" => "memoryLimit",
        "output-overflow" => "outputOverflow",
        "crash" => "crash",
        "invalid-json" => "schema",
        "spawn-error" | "stdin-error" | "wait-error" => "environment",
        _ => "internal",
    }
}

fn bounded_diagnostic(stderr: &str) -> Value {
    json!({
        "code":"runner.failure",
        "phase":"supervisor",
        "message":stderr.chars().take(4096).collect::<String>()
    })
}

fn empty_artifacts() -> Value {
    json!({
        "canonicalCase":Value::Null,
        "request":Value::Null,
        "stderr":Value::Null,
        "inputShape":Value::Null,
        "outputShape":Value::Null
    })
}

pub fn complete_result(
    mut value: Value,
    case: &PreparedCase,
    backend: Backend,
    input_digest: &str,
) -> Value {
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    object.insert("schemaVersion".into(), json!(1));
    object.insert("caseId".into(), json!(case.case_id));
    object.insert("generator".into(), json!(case.generator));
    object.insert("backend".into(), json!(backend));
    object.insert("expectedDomain".into(), json!(case.expected_domain));
    if !object.get("inputDigest").is_some_and(valid_digest) {
        object.insert("inputDigest".into(), json!(input_digest));
    }
    apply_policy(&mut value, case.expected_domain, backend);
    refresh_digest(&mut value);
    value
}

pub fn refresh_digest(value: &mut Value) {
    let digest = normalized_digest(value);
    value["normalizedDigest"] = json!(digest);
}

fn valid_digest(value: &Value) -> bool {
    value.as_str().is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

pub fn attach_replay(mut canonical: Value, replay: &Value) -> Value {
    let canonical_digest = canonical
        .get("normalizedDigest")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let replay_digest = replay
        .get("normalizedDigest")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let stable = !canonical_digest.is_empty() && canonical_digest == replay_digest;
    canonical["replay"] = json!(ReplayEvidence {
        stable,
        digest: replay_digest.into()
    });
    if !stable {
        canonical["verdict"] = json!(Verdict::Fail);
        canonical["failureClass"] = json!("nondeterministic");
    }
    refresh_digest(&mut canonical);
    canonical
}

pub fn is_red(value: &Value) -> bool {
    value.get("verdict").and_then(Value::as_str) == Some("fail")
}

pub fn generator(value: &Value) -> Option<Generator> {
    serde_json::from_value(value.get("generator")?.clone()).ok()
}

fn failure_verdict(domain: ExpectedDomain, failure: &str) -> Verdict {
    let unsafe_failure = matches!(
        failure,
        "crash"
            | "timeout"
            | "nondeterministic"
            | "invalid-json"
            | "output-overflow"
            | "memory-limit"
    );
    match domain {
        ExpectedDomain::Exploratory if !unsafe_failure => Verdict::Characterization,
        ExpectedDomain::OutsideProductDomain if !unsafe_failure => Verdict::Characterization,
        ExpectedDomain::ExpectedLimit if failure == "safe-refusal" => Verdict::Pass,
        _ => Verdict::Fail,
    }
}

fn apply_policy(value: &mut Value, domain: ExpectedDomain, backend: Backend) {
    let execution = value.get("executionState").and_then(Value::as_str);
    let operation = value.get("operationState").and_then(Value::as_str);
    if execution != Some("completed") {
        value["verdict"] = json!(Verdict::Fail);
        return;
    }
    match operation {
        Some("success") => apply_success_policy(value, backend),
        Some("rejected") => apply_refusal_policy(value, domain),
        Some("partial") => set_failure(value, "partialResult"),
        Some("badShape") => set_failure(value, "badShape"),
        _ => set_failure(value, "internal"),
    }
}

fn apply_success_policy(value: &mut Value, backend: Backend) {
    if !audit_publishable(value.get("outputAudit")) {
        let failure = if backend == Backend::Onecad {
            "publicationInvalid"
        } else {
            "auditFailed"
        };
        set_failure(value, failure);
    } else if !required_validators_pass(value.get("validators")) {
        set_failure(value, "semanticFailed");
    } else {
        value["verdict"] = json!(Verdict::Pass);
        value["failureClass"] = json!("none");
    }
}

fn apply_refusal_policy(value: &mut Value, domain: ExpectedDomain) {
    value["failureClass"] = json!("kernelRejected");
    value["verdict"] = match domain {
        ExpectedDomain::ExpectedLimit => json!(Verdict::Pass),
        ExpectedDomain::Exploratory | ExpectedDomain::OutsideProductDomain => {
            json!(Verdict::Characterization)
        }
        ExpectedDomain::Supported => json!(Verdict::Fail),
    };
}

fn set_failure(value: &mut Value, class: &str) {
    value["verdict"] = json!(Verdict::Fail);
    value["failureClass"] = json!(class);
}

fn audit_publishable(audit: Option<&Value>) -> bool {
    let Some(audit) = audit else {
        return false;
    };
    audit.get("productionAudit").and_then(Value::as_str) == Some("pass")
        && audit.get("exactValid").and_then(Value::as_bool) == Some(true)
        && audit.get("selfInterferenceFree").and_then(Value::as_bool) == Some(true)
        && audit.get("closedManifold").and_then(Value::as_bool) == Some(true)
}

fn required_validators_pass(validators: Option<&Value>) -> bool {
    validators.and_then(Value::as_array).is_some_and(|items| {
        items.iter().all(|item| {
            item.get("required").and_then(Value::as_bool) != Some(true)
                || item.get("status").and_then(Value::as_str) == Some("pass")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_wire_names_are_contractual() {
        assert_eq!(Backend::RawOcct.wire(), "raw-occt");
        assert_eq!(Backend::Onecad.wire(), "onecad");
    }
}
