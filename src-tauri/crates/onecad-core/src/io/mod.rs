//! v2 `.onecad` container IO, per-section codecs, migration and crash recovery.
//!
//! # Layout (v2.0)
//!
//! A `.onecad` file is a ZIP archive with this section layout:
//!
//! ```text
//! manifest.json          index: magic, versions, documentId, opsHash, entry hashes
//! document.json          DocumentData — the AUTHORITATIVE payload
//! timeline/ops.jsonl     one OperationRecord per line — a DERIVED readable projection
//! geometry/<bodyId>.brep opaque BREP cache (from the worker)
//! meshes/<bodyId>.<lod>.mesh   MESH1 cache
//! checkpoints/<step>.json + .bin   optional checkpoint artifacts
//! imports/<sha256>.step|.brep      import source blobs — AUTHORITATIVE-FOR-A-RECORD
//! preview.png            optional thumbnail
//! ```
//!
//! ## Design decisions (flagged for orchestrator review)
//!
//! * **`document.json` is the single source of truth; `timeline/ops.jsonl` is a
//!   derived, human-readable projection.** On load both are read and
//!   cross-validated; on any divergence `document.json` wins and a `Warning`
//!   diagnostic is emitted (never an error). Rationale: a single authoritative
//!   payload avoids the dual-source reconciliation problem — the timeline records
//!   already live inside `document.json` (`DocumentData.timeline.records`), so
//!   splitting them into a second authoritative section would create two things
//!   that must agree. `ops.jsonl` exists only so the history is greppable/diffable
//!   outside the app.
//! * **Sketches stay INLINE in `document.json`; the v2.0 container has NO
//!   `sketches/` directory.** The plan sketched a `sketches/<uuid>.json` layout,
//!   but [`Document`](crate::document::Document) holds sketches inline in a
//!   `BTreeMap<SketchId, Sketch>`, so extracting them would (a) duplicate a source
//!   of truth and (b) churn the already-frozen `document.json` shape. Divergence
//!   from the plan is deliberate and recorded here + in [`sketch_io`]. Flag for
//!   orchestrator review.
//! * **`imports/` is a THIRD section class — "authoritative-for-a-record".** It
//!   is irreplaceable input (no regen can reconstruct a source STEP, so it is not
//!   a cache), yet its blast radius is one timeline step, so a missing or corrupt
//!   blob must not fail the open the way a corrupt `document.json` does. Rationale
//!   and the full contract live in [`imports`].
//!
//! ## File attack surface (Codex red-team; plan "File attack surface")
//!
//! Every entry read is bounded. The container is treated as adversarial input;
//! a malformed or hostile archive yields a typed [`IoError`], **never a panic**.
//! See [`container`] for the caps table (per-section decompression caps, total
//! container cap, entry-count cap) and the zip path-traversal guard. JSON nesting
//! depth is bounded by `serde_json`'s default recursion limit (128), which returns
//! an `Err` rather than overflowing the stack (verified by
//! `hostile_deeply_nested_json_errors`). Hostile STEP is out of scope here — it is
//! handled in the isolated worker.

pub mod container;
pub mod document_io;
pub mod history_io;
pub mod imports;
pub mod manifest;
pub mod migrate;
pub mod project_import;
pub mod recovery;
pub mod sketch_io;
pub mod threemf;

use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Typed failures raised by the container IO layer.
///
/// Hostile or malformed input maps to one of these variants — the layer never
/// panics on adversarial bytes (plan "File attack surface"; test
/// `hostile_*`). `NeedsRepair` is a document *state*, never an IO error (see
/// [`crate::error`]).
#[derive(Debug, Error)]
pub enum IoError {
    /// Filesystem / zip transport failure.
    #[error("io: {0}")]
    Io(String),

    /// The manifest magic string was absent or not `"ONECAD"`.
    #[error("bad magic: not a OneCAD container")]
    BadMagic,

    /// The container version is outside the supported range (v2 exact for now).
    #[error("unsupported container version {found} (this build reads {expected})")]
    UnsupportedVersion {
        /// The version found in the manifest.
        found: u32,
        /// The version this build reads.
        expected: u32,
    },

    /// The archive, an authoritative section, or an authoritative entry hash is
    /// corrupt / malformed.
    #[error("corrupt container: {0}")]
    Corrupt(String),

    /// A decompressed section, or the whole container, exceeded its size cap
    /// (decompression-bomb guard).
    #[error("too large: {0}")]
    TooLarge(String),

    /// A zip entry name escaped the archive root (`../`, absolute path, or a
    /// path the platform would resolve outside the extraction dir).
    #[error("path traversal blocked: {0}")]
    PathTraversal(String),
}

/// Convenience result alias for the container IO layer.
pub type IoResult<T> = Result<T, IoError>;

impl From<std::io::Error> for IoError {
    fn from(e: std::io::Error) -> Self {
        IoError::Io(e.to_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Diagnostics
// ─────────────────────────────────────────────────────────────────────────────

/// Severity of a non-fatal load [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Informational (e.g. a lossless migration ran).
    Info,
    /// Something is off but the load continues (e.g. `ops.jsonl` diverged, caches
    /// went stale, a low-confidence migration forced read-only).
    Warning,
}

/// A non-fatal observation surfaced during a load. Diagnostics never fail the
/// load; they are collected and reported to the app (plan §9 "guided migration
/// report" + the cache/ops.jsonl reconciliation warnings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// How serious the observation is.
    pub severity: Severity,
    /// A stable machine code (e.g. `"ops-jsonl-divergence"`) for tests / UI.
    pub code: &'static str,
    /// Human-facing detail.
    pub message: String,
}

impl Diagnostic {
    /// A `Warning`-severity diagnostic.
    #[must_use]
    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message: message.into(),
        }
    }

    /// An `Info`-severity diagnostic.
    #[must_use]
    pub fn info(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Info,
            code,
            message: message.into(),
        }
    }
}

/// Lowercase-hex-encodes bytes (SHA-256 digests, entry/ops hashes). Shared by the
/// IO codecs so the file format has one hex convention.
#[must_use]
pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

/// SHA-256 of `bytes` as a lowercase-hex string (the file format's content-hash
/// convention; SCHEMA §2).
#[must_use]
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(&hasher.finalize())
}

/// Durably writes `bytes` to `path`: a same-directory temp file, `fsync`, an
/// atomic `rename` over the target, then an `fsync` of the parent directory so the
/// rename itself survives a crash (durability of `rename` on macOS/Linux). The
/// temp file is removed on any failure along the way, so a caller never observes a
/// half-written `path`, nor an orphaned temp from the attempt itself.
///
/// Shared by [`recovery::write_marker`] and the app crate's `recents.json`
/// writer — anything persisting a small JSON blob outside the ZIP-container
/// writer ([`container::ContainerWriter`], which streams its archive straight
/// into the temp file and has its own copy of this same shape for that reason).
///
/// # Errors
/// [`IoError::Io`] on any filesystem failure.
pub fn durable_write(path: &Path, bytes: &[u8]) -> IoResult<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let tmp = temp_sibling(path);

    if let Err(e) = write_and_sync(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    fsync_dir(parent);
    Ok(())
}

/// Writes `bytes` to `path` and `fsync`s the file (durable content, before the
/// caller renames it into place).
fn write_and_sync(path: &Path, bytes: &[u8]) -> IoResult<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// A sibling temp path for `path` (`<name>.tmp`).
fn temp_sibling(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    path.with_file_name(format!("{name}.tmp"))
}

/// fsyncs a directory so a rename inside it is durable (macOS/Unix). A no-op on
/// other platforms (Windows renames are durable without this).
fn fsync_dir(dir: &Path) {
    #[cfg(unix)]
    {
        if let Ok(f) = std::fs::File::open(dir) {
            let _ = f.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}
