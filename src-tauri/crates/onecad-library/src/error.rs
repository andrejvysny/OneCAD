//! `LibraryError` — every failure mode a `Library` operation can produce.
//!
//! Mirrors `onecad-core::io::imports::ImportBlobError`'s discipline: a bad
//! package or blob is a per-entity failure the caller degrades gracefully
//! from (spec §4 — a revision mismatch becomes `NeedsRepair` at the op layer,
//! never a load-breaking `Err`), never a panic and never conflated with "the
//! whole library is corrupt."

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("component package not found: {0}")]
    NotFound(String),

    #[error("malformed component.toml at {path}: {message}")]
    MalformedPackage { path: PathBuf, message: String },

    #[error("component package identity invalid: {0}")]
    InvalidIdentity(String),

    #[error("blob {sha256} not found")]
    BlobMissing { sha256: String },

    #[error("blob {sha256} failed integrity check: expected sha256 {expected}, got {actual}")]
    BlobHashMismatch {
        sha256: String,
        expected: String,
        actual: String,
    },

    #[error("blob key {0:?} is not a well-formed sha256 hex string")]
    BadBlobKey(String),

    #[error("blob exceeds the {limit} byte cap ({actual} bytes)")]
    BlobTooLarge { limit: u64, actual: u64 },

    #[error(
        "component revision mismatch for {component_id}: recorded {recorded}, package now {current}"
    )]
    RevisionMismatch {
        component_id: String,
        recorded: String,
        current: String,
    },

    #[error("library index at {path} is malformed: {message}")]
    MalformedIndex { path: PathBuf, message: String },

    #[error("io error: {0}")]
    Io(String),
}

impl From<std::io::Error> for LibraryError {
    fn from(e: std::io::Error) -> Self {
        LibraryError::Io(e.to_string())
    }
}

pub type LibraryResult<T> = Result<T, LibraryError>;
