//! An import-free Component Model sandbox. Extensions return requests; only the
//! application service layer may execute them against a captured document target.
mod package;
mod policy;
mod registry;
mod sandbox;

pub use onecad_extension_api as api;
pub use registry::{Approval, ExtensionRegistry, PackageState, PackageSummary, RegisteredCommand};
pub use sandbox::SandboxLimits;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("extension I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid extension data: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Manifest(#[from] api::ManifestError),
    #[error("invalid package: {0}")]
    InvalidPackage(String),
    #[error("extension not found: {0}")]
    NotFound(String),
    #[error("extension permission denied: {0}")]
    PermissionDenied(String),
    #[error("extension approval expired because package content changed")]
    ApprovalChanged,
    #[error("extension command unavailable: {0}")]
    Unavailable(String),
    #[error("extension sandbox failed: {0}")]
    Sandbox(String),
    #[error(
        "native extensions require explicit trust and are not supported by this portable host"
    )]
    NativeUnsupported,
}

pub type Result<T> = std::result::Result<T, HostError>;
