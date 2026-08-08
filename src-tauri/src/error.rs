//! Frontend-facing API error type.
//!
//! Serialized into failed command results so the webview gets a typed error
//! (`{ kind, message }`). This mirrors the SCHEMA §8 taxonomy at the app boundary:
//! recoverable op failures keep the document editable; protocol/crash are fatal
//! (R-WP11 drives worker restart). `NeedsRepair` is never an error — it is
//! document *state* delivered in the projection.

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use onecad_core::error::DomainError;
use onecad_core::io::IoError;
use onecad_core::regen::{EngineError, OpFailureCode};

/// Error returned from Tauri commands to the webview (`{ kind, message }`).
#[derive(Debug, Clone)]
pub enum ApiError {
    /// No document is open (the command requires one).
    NoDocument(String),
    /// A command mutated the document invalidly (validation / anti-time-travel).
    InvalidCommand(String),
    /// A recoverable geometry-op failure — the document stays editable.
    OpFailed {
        message: String,
        diagnostics: Vec<onecad_core::regen::Diagnostic>,
    },
    /// The preview candidate targeted an older published snapshot. The caller
    /// should drop it and issue a fresh candidate, not surface a tool failure.
    StalePreview(String),
    /// A protocol / crash-class failure — R-WP11 restarts the worker.
    Worker(String),
    /// Filesystem / container IO failure (open/save).
    Io(String),
    /// A generic internal failure.
    Internal(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDocument(m) => write!(f, "no document open: {m}"),
            Self::InvalidCommand(m) => write!(f, "invalid command: {m}"),
            Self::OpFailed { message, .. } => write!(f, "operation failed: {message}"),
            Self::StalePreview(m) => write!(f, "stale preview: {m}"),
            Self::Worker(m) => write!(f, "worker error: {m}"),
            Self::Io(m) => write!(f, "io error: {m}"),
            Self::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for ApiError {}

impl Serialize for ApiError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let (kind, message, diagnostics) = match self {
            Self::NoDocument(message) => ("noDocument", message, None),
            Self::InvalidCommand(message) => ("invalidCommand", message, None),
            Self::OpFailed {
                message,
                diagnostics,
            } => ("opFailed", message, Some(diagnostics)),
            Self::StalePreview(message) => ("stalePreview", message, None),
            Self::Worker(message) => ("worker", message, None),
            Self::Io(message) => ("io", message, None),
            Self::Internal(message) => ("internal", message, None),
        };
        let include_diagnostics = diagnostics.is_some_and(|items| !items.is_empty());
        let mut state =
            serializer.serialize_struct("ApiError", if include_diagnostics { 3 } else { 2 })?;
        state.serialize_field("kind", kind)?;
        state.serialize_field("message", message)?;
        if let Some(items) = diagnostics.filter(|items| !items.is_empty()) {
            state.serialize_field("diagnostics", items)?;
        }
        state.end()
    }
}

impl From<DomainError> for ApiError {
    fn from(e: DomainError) -> Self {
        ApiError::InvalidCommand(e.to_string())
    }
}

impl From<IoError> for ApiError {
    fn from(e: IoError) -> Self {
        ApiError::Io(e.to_string())
    }
}

/// The engine→API conversion is the ONE choke point every `?`-propagated engine
/// failure crosses, so it is where the failure's structured detail is preserved:
/// the wire shape stays the pinned `{kind, message}`, but `code`/`recoverable`
/// reach the log.
///
/// `StalePreview` is deliberately DEMOTED to `debug`: it is the routine outcome of a
/// drag frame that raced a publish (and fires from unit tests), so warning on it
/// would make `'"level":"WARN"'` useless as a first-look grep.
impl From<EngineError> for ApiError {
    fn from(e: EngineError) -> Self {
        match &e {
            EngineError::OpFailed {
                code: OpFailureCode::StalePreview,
                message,
                ..
            } => tracing::debug!(message = %message, "engine: stale preview"),
            EngineError::OpFailed {
                code,
                recoverable,
                message,
                ..
            } => tracing::warn!(
                code = ?code,
                recoverable,
                message = %message,
                "engine: op failed"
            ),
            EngineError::Crashed { .. }
            | EngineError::NotConnected { .. }
            | EngineError::Protocol { .. }
            | EngineError::Timeout { .. } => {
                tracing::debug!(error = %e, "engine: worker-class failure → ApiError::Worker");
            }
            EngineError::Cancelled => {}
        }
        let message = e.to_string();
        match e {
            EngineError::OpFailed { diagnostics, .. } => ApiError::OpFailed {
                message,
                diagnostics,
            },
            EngineError::Crashed { .. }
            | EngineError::NotConnected { .. }
            | EngineError::Protocol { .. }
            | EngineError::Timeout { .. } => ApiError::Worker(message),
            EngineError::Cancelled => ApiError::OpFailed {
                message: "cancelled".into(),
                diagnostics: Vec::new(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_preview_is_a_distinct_frontend_error_kind() {
        let value = serde_json::to_value(ApiError::StalePreview("snapshot moved".into())).unwrap();
        assert_eq!(value["kind"], "stalePreview");
        assert_eq!(value["message"], "snapshot moved");
    }

    /// A never-connected worker is handled exactly like a crash (kind `"worker"`)
    /// but must not be REPORTED as one: the user-visible string used to read
    /// "worker crashed: worker not connected", which sent a startup race looking
    /// like a kernel crash.
    #[test]
    fn not_connected_is_a_worker_error_that_never_claims_a_crash() {
        let value = serde_json::to_value(ApiError::from(EngineError::NotConnected {
            message: "restarting or failed".into(),
        }))
        .unwrap();
        assert_eq!(value["kind"], "worker");
        let message = value["message"].as_str().expect("message is a string");
        assert!(message.contains("worker not connected"), "{message}");
        assert!(!message.contains("crashed"), "{message}");
    }

    #[test]
    fn op_failure_serializes_optional_structured_diagnostics_additively() {
        let error = ApiError::from(EngineError::OpFailed {
            code: OpFailureCode::GeometryInvalid,
            recoverable: true,
            message: "fillet failed".into(),
            diagnostics: vec![onecad_core::regen::Diagnostic {
                severity: onecad_core::regen::Severity::Error,
                code: "FILLET_WALKING_FAILED".into(),
                message: "fillet failed".into(),
                stage: Some("build".into()),
                evidence: Some(serde_json::json!({"metrics": {"requestedRadius": 11.0}})),
            }],
        });
        let value = serde_json::to_value(error).unwrap();
        assert_eq!(value["kind"], "opFailed");
        assert_eq!(value["diagnostics"][0]["code"], "FILLET_WALKING_FAILED");
        assert_eq!(value["diagnostics"][0]["stage"], "build");
    }
}
