use serde::Deserialize;
use serde_json::Value;

use crate::case::{ExpectedDomain, Generator};
use crate::prepared::PreparedCase;
use crate::result::{Backend, Verdict};
use crate::result_validation_blocks as blocks;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunnerResult {
    schema_version: u32,
    case_id: String,
    generator: Generator,
    backend: Backend,
    kernel: Identity,
    modeler: Identity,
    expected_domain: ExpectedDomain,
    verdict: Verdict,
    execution_state: ExecutionState,
    operation_state: OperationState,
    failure_class: FailureClass,
    diagnostics: Vec<Diagnostic>,
    input_audit: Option<Value>,
    output_audit: Option<Value>,
    validators: Vec<Value>,
    selection_evidence: Vec<Value>,
    timing: Timing,
    resource: Resource,
    metamorph: Option<Value>,
    search: Option<Value>,
    shape_signature: Option<Value>,
    artifacts: Artifacts,
    input_digest: String,
    normalized_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Identity {
    name: String,
    version: String,
    build_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Diagnostic {
    code: String,
    phase: Phase,
    message: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    Request,
    Generation,
    Selection,
    Operation,
    Audit,
    Validation,
    Supervisor,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionState {
    Completed,
    Exception,
    Crash,
    Timeout,
    InvalidRequest,
    OutputOverflow,
    MemoryLimit,
    EnvironmentError,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationState {
    Success,
    Rejected,
    Partial,
    BadShape,
    NotRun,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FailureClass {
    None,
    Schema,
    Environment,
    Exception,
    Crash,
    Timeout,
    OutputOverflow,
    MemoryLimit,
    KernelRejected,
    PartialResult,
    BadShape,
    AuditFailed,
    SemanticFailed,
    Nondeterministic,
    PublicationInvalid,
    InputMismatch,
    Internal,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Timing {
    wall_ms: f64,
    operation_ms: f64,
    audit_ms: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Resource {
    peak_rss_bytes: Option<u64>,
    exit_code: Option<u8>,
    signal: Option<u8>,
    stderr_truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Artifacts {
    canonical_case: Option<String>,
    request: Option<String>,
    stderr: Option<String>,
    input_shape: Option<String>,
    output_shape: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct ValidatedState {
    pub execution: ExecutionState,
    pub operation: OperationState,
}

pub fn validate(
    value: &Value,
    case: &PreparedCase,
    backend: Backend,
) -> Result<ValidatedState, String> {
    let parsed: RunnerResult =
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    validate_identity(
        &parsed,
        &case.case_id,
        backend,
        Some(&case.generator),
        Some(case.expected_domain),
    )?;
    validate_common(&parsed)
}

pub fn validate_persisted(value: &Value) -> bool {
    persisted(value).is_ok()
}

fn persisted(value: &Value) -> Result<(), String> {
    let mut runner = value.clone();
    let object = runner
        .as_object_mut()
        .ok_or_else(|| "result is not object".to_string())?;
    let replay = object.remove("replay");
    let differential = object.remove("differential");
    let variant = object.remove("campaignVariant");
    let parsed: RunnerResult = serde_json::from_value(runner).map_err(|error| error.to_string())?;
    validate_identity(&parsed, &parsed.case_id, parsed.backend, None, None)?;
    validate_common(&parsed)?;
    replay.as_ref().map(validate_replay).transpose()?;
    differential
        .as_ref()
        .map(validate_differential)
        .transpose()?;
    variant.as_ref().map(validate_variant).transpose()?;
    Ok(())
}

fn validate_common(parsed: &RunnerResult) -> Result<ValidatedState, String> {
    validate_nested(parsed)?;
    let state = ValidatedState {
        execution: parsed.execution_state,
        operation: parsed.operation_state,
    };
    validate_state_coherence(state)?;
    Ok(state)
}

// The runner only ever reports `operationState: notRun` when the operation
// genuinely never ran (an exception, crash, timeout, or other synthetic
// supervisor failure), and always reports a real operation state when
// `executionState` is `completed` (see Execution.cpp's `operation_state()`
// and `result.rs`'s `synthetic_result()`). A record that violates this is
// internally inconsistent and must not be trusted.
fn validate_state_coherence(state: ValidatedState) -> Result<(), String> {
    let not_run = state.operation == OperationState::NotRun;
    let completed = state.execution == ExecutionState::Completed;
    if completed == not_run {
        return Err("executionState/operationState combination is incoherent".into());
    }
    Ok(())
}

fn validate_nested(parsed: &RunnerResult) -> Result<(), String> {
    validate_bounds(parsed)?;
    parsed.input_audit.as_ref().map(blocks::audit).transpose()?;
    parsed
        .output_audit
        .as_ref()
        .map(blocks::audit)
        .transpose()?;
    parsed.validators.iter().try_for_each(blocks::validator)?;
    parsed
        .selection_evidence
        .iter()
        .try_for_each(blocks::selection)?;
    parsed
        .metamorph
        .as_ref()
        .map(blocks::metamorph)
        .transpose()?;
    parsed.search.as_ref().map(blocks::search).transpose()?;
    parsed
        .shape_signature
        .as_ref()
        .map(blocks::shape_signature)
        .transpose()?;
    blocks::artifacts([
        &parsed.artifacts.canonical_case,
        &parsed.artifacts.request,
        &parsed.artifacts.stderr,
        &parsed.artifacts.input_shape,
        &parsed.artifacts.output_shape,
    ])
}

fn validate_identity(
    result: &RunnerResult,
    case_id: &str,
    backend: Backend,
    generator: Option<&Generator>,
    expected_domain: Option<ExpectedDomain>,
) -> Result<(), String> {
    if result.schema_version != 1 || result.case_id != case_id || result.backend != backend {
        return Err("runner identity mismatch".into());
    }
    if result.generator.version != 1 || !valid_seed(&result.generator.seed) {
        return Err("invalid generator identity".into());
    }
    if let Some(generator) = generator {
        if result.generator.name != generator.name
            || result.generator.version != generator.version
            || result.generator.seed != generator.seed
        {
            return Err("generator does not match requesting case".into());
        }
    }
    if let Some(domain) = expected_domain {
        if result.expected_domain != domain {
            return Err("expectedDomain does not match requesting case".into());
        }
    }
    for identity in [&result.kernel, &result.modeler] {
        if !bounded(&identity.name, 1, 64)
            || !bounded(&identity.version, 1, 64)
            || !bounded(&identity.build_id, 1, 256)
        {
            return Err("invalid kernel/modeler identity".into());
        }
    }
    if !valid_digest(&result.input_digest) || !valid_digest(&result.normalized_digest) {
        return Err("invalid digest".into());
    }
    Ok(())
}

fn validate_bounds(result: &RunnerResult) -> Result<(), String> {
    let _ = (
        result.expected_domain,
        result.verdict,
        result.failure_class,
        result.resource.stderr_truncated,
        result.resource.peak_rss_bytes,
        result.resource.exit_code,
        result.resource.signal,
    );
    if result.diagnostics.len() > 64
        || result.validators.len() > 64
        || result.selection_evidence.len() > 128
    {
        return Err("runner array exceeds limit".into());
    }
    for diagnostic in &result.diagnostics {
        let _ = diagnostic.phase;
        if !safe_id(&diagnostic.code) || diagnostic.message.len() > 4096 {
            return Err("invalid diagnostic".into());
        }
    }
    if ![
        result.timing.wall_ms,
        result.timing.operation_ms,
        result.timing.audit_ms,
    ]
    .iter()
    .all(|value| value.is_finite() && *value >= 0.0 && *value <= 1e15)
    {
        return Err("invalid timing".into());
    }
    Ok(())
}

fn validate_replay(value: &Value) -> Result<(), String> {
    blocks::exact(value, &["stable", "digest"])?;
    blocks::boolean(&value["stable"])?;
    value["digest"]
        .as_str()
        .filter(|digest| valid_digest(digest))
        .map(|_| ())
        .ok_or_else(|| "invalid replay digest".into())
}

fn validate_differential(value: &Value) -> Result<(), String> {
    blocks::exact(value, &["classification"])?;
    blocks::enumeration(
        &value["classification"],
        &[
            "rescued",
            "onecad-regression",
            "same-status",
            "input-mismatch",
            "status-difference",
        ],
    )
}

fn validate_variant(value: &Value) -> Result<(), String> {
    let _: crate::suite::Variant =
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    Ok(())
}

fn bounded(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len())
}
fn safe_id(value: &str) -> bool {
    bounded(value, 1, 128)
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._-".contains(&byte))
        })
}
fn valid_seed(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
