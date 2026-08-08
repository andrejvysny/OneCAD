//! Import-blob materialization: the bridge between the container's
//! content-addressed [`imports/`](onecad_core::io::imports) section and the
//! filesystem path the C++ worker actually reads.
//!
//! # Why a filesystem hop exists at all
//!
//! A `ImportStep` record's params are a **pointer** (`sourceSha256`), never a
//! payload — see [`ImportStepParams`]. The worker, on the other side of an OCW1
//! stdio pipe, needs *bytes*. Pushing megabytes of BinTools through the frame
//! layer on every regen would put the whole blob in every `ExecutePlan`; instead
//! Rust writes the blob ONCE to a temp file and lowers its path as the wire-only,
//! non-hashed `params.path` (SCHEMA §7.3 / §7.8 "Rust-provided temp paths").
//!
//! # Lifetime
//!
//! A materialized blob must outlive:
//!
//! * the **unlocked** `PreparedRegen::drive` window (the executor runs without the
//!   `DocumentRuntime` lock, so the runtime could be mutated meanwhile), and
//! * a **worker restart replay** (a crash re-executes the whole timeline from 0,
//!   which re-reads every import path).
//!
//! So the files are owned by an [`ImportWorkspace`] that lives on the
//! `DocumentRuntime` and is cleaned on drop (close / document swap), plus a
//! best-effort [`sweep_stale_workspaces`] at startup for directories a crash left
//! behind. No daemon, no refcount file — just mtime.
//!
//! # The path registry
//!
//! Lowering happens in [`wire`](crate::worker::wire), a pure function with no
//! access to the runtime, so the `sha → path` mapping is process-global (the same
//! shape as that module's split-id interner). This is sound precisely BECAUSE the
//! key is a content address: two documents holding the same digest hold
//! byte-identical bytes, so either path is equally correct. Each sha maps to a
//! LIST of paths and resolution takes the first that still exists, so one
//! document's teardown can never invalidate another's still-live materialization.
//!
//! [`ImportStepParams`]: onecad_core::document::record::ImportStepParams

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use onecad_core::document::record::{ImportSourceCodec, ImportStepParams, IMPORT_HEAL_POLICY_V1};
use onecad_core::document::variables::Scalar;
use onecad_core::ids::DocumentId;
use onecad_core::io::imports::{ImportBlob, MAX_IMPORT_BLOB_BYTES};

use crate::error::ApiError;
use crate::worker::{StepImport, StepInspection};

/// Hard cap on a source file this build will import, mirroring the container's
/// per-blob cap ([`MAX_IMPORT_BLOB_BYTES`], 256 MiB). Rejected BEFORE the bytes are
/// read, so a hostile file never costs more than a `metadata()` call.
pub const MAX_IMPORT_FILE_BYTES: u64 = MAX_IMPORT_BLOB_BYTES;

/// Above this the import still proceeds, but the returned diagnostics carry a
/// heads-up: the blob is held in memory by the runtime and re-written into the
/// container on every save.
pub const LARGE_IMPORT_HINT_BYTES: u64 = 64 * 1024 * 1024;

/// How long a leftover workspace directory survives before the startup sweep
/// removes it. Generous on purpose — the cost of keeping one is disk, the cost of
/// deleting a LIVE one is a broken regen.
pub const STALE_WORKSPACE_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The parent directory of every per-document workspace.
const WORKSPACE_ROOT: &str = "onecad-imports";

// ─────────────────────────────────────────────────────────────────────────────
// Process-wide `sha → materialized paths` registry (read by wire lowering)
// ─────────────────────────────────────────────────────────────────────────────

fn registry() -> &'static RwLock<HashMap<String, Vec<PathBuf>>> {
    static REGISTRY: OnceLock<RwLock<HashMap<String, Vec<PathBuf>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Records that `sha`'s bytes are readable at `path`.
fn register(sha: &str, path: &Path) {
    if let Ok(mut map) = registry().write() {
        let paths = map.entry(sha.to_string()).or_default();
        if !paths.iter().any(|p| p == path) {
            paths.push(path.to_path_buf());
        }
    }
}

/// Drops `path` from `sha`'s list (its owning workspace is going away). Other
/// workspaces' copies of the same digest stay resolvable.
fn unregister(sha: &str, path: &Path) {
    if let Ok(mut map) = registry().write() {
        if let Some(paths) = map.get_mut(sha) {
            paths.retain(|p| p != path);
            if paths.is_empty() {
                map.remove(sha);
            }
        }
    }
}

/// The filesystem path a materialized import blob is readable at, or `""` when the
/// digest has no live materialization.
///
/// The empty string is a deliberate, load-bearing answer: the wire layer lowers it
/// verbatim and the worker fails that ONE step with `OP_FAILED` (SCHEMA §7.3
/// "failure is recoverable"), which is the correct blast radius for a missing
/// source. See [`crate::worker::wire`]'s `inject_import_path`.
#[must_use]
pub fn resolve_blob_path(sha256: &str) -> String {
    let Ok(map) = registry().read() else {
        return String::new();
    };
    map.get(sha256)
        .and_then(|paths| paths.iter().find(|p| p.is_file()))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-document workspace
// ─────────────────────────────────────────────────────────────────────────────

/// Process-unique suffix so two runtimes over the SAME document id (reopen while
/// the old one is still alive — the save/reopen test does exactly this) never
/// share a directory, and so one's teardown never deletes the other's files.
static WORKSPACE_SEQ: AtomicU64 = AtomicU64::new(0);

/// The temp directory holding one document's materialized import blobs, laid out
/// as `<tmp>/onecad-imports/<documentId>-<pid>-<seq>/<sha256>.<codec>`.
///
/// Cleaned on [`Drop`] (document close / swap). Every materialized file is also
/// registered in the process-wide path registry and unregistered here, so a
/// dropped workspace cannot leave the wire layer pointing at a deleted file.
#[derive(Debug)]
pub struct ImportWorkspace {
    dir: PathBuf,
    /// `(sha, path)` for everything this workspace put in the registry.
    entries: Vec<(String, PathBuf)>,
}

impl ImportWorkspace {
    /// A workspace for `document`. The directory is created lazily on the first
    /// [`materialize`](Self::materialize), so a document with no imports (the
    /// overwhelming majority) touches the filesystem zero times.
    #[must_use]
    pub fn new(document: DocumentId) -> Self {
        let seq = WORKSPACE_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(WORKSPACE_ROOT)
            .join(format!("{document}-{}-{seq}", std::process::id()));
        Self {
            dir,
            entries: Vec::new(),
        }
    }

    /// This workspace's directory (may not exist until the first materialization).
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes `bytes` to `<dir>/<sha256>.<codec>` and registers the path for wire
    /// lowering. Re-materializing an already-present digest is a no-op.
    ///
    /// # Errors
    /// [`std::io::Error`] on a filesystem failure. The caller treats this as
    /// non-fatal for OPEN (the record then fails loudly at its own step) and as
    /// fatal for IMPORT (authoring a record whose source cannot be written would
    /// persist a document that never regenerates).
    pub fn materialize(
        &mut self,
        sha256: &str,
        codec: ImportSourceCodec,
        bytes: &[u8],
    ) -> std::io::Result<PathBuf> {
        let path = self.dir.join(format!("{sha256}.{}", codec.extension()));
        if self.entries.iter().any(|(s, p)| s == sha256 && p == &path) {
            return Ok(path);
        }
        std::fs::create_dir_all(&self.dir)?;
        // Content-addressed, so an existing file with this name already holds these
        // exact bytes; rewriting is harmless and covers a truncated leftover.
        std::fs::write(&path, bytes)?;
        register(sha256, &path);
        self.entries.push((sha256.to_string(), path.clone()));
        tracing::debug!(sha = %sha256, path = %path.display(), bytes = bytes.len(), "import blob materialized");
        Ok(path)
    }
}

impl Drop for ImportWorkspace {
    fn drop(&mut self) {
        for (sha, path) in &self.entries {
            unregister(sha, path);
        }
        if self.dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&self.dir) {
                tracing::warn!(dir = %self.dir.display(), error = %e, "import workspace cleanup failed");
            }
        }
    }
}

/// Removes `onecad-imports/*` directories older than `max_age` — workspaces a crash
/// (or a kill -9) left behind, since [`ImportWorkspace::drop`] never ran.
///
/// Best-effort and cheap: one directory listing plus an mtime stat each, no
/// recursion into live directories, every failure ignored. Safe to call at every
/// startup. Returns the number of directories removed (tests / diagnostics).
pub fn sweep_stale_workspaces(max_age: Duration) -> usize {
    let root = std::env::temp_dir().join(WORKSPACE_ROOT);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return 0; // never created — nothing to sweep
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() {
            continue;
        }
        let stale = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > max_age);
        if stale && std::fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(removed, root = %root.display(), "swept stale import workspaces");
    }
    removed
}

// ─────────────────────────────────────────────────────────────────────────────
// The import preflight (read → probe → params)
// ─────────────────────────────────────────────────────────────────────────────

/// Everything an `ImportStep` record needs, produced WITHOUT touching the document:
/// the params to author plus the two blobs to persist. Built by [`prepare_import`].
///
/// **Converted-primary replay policy** (W0 decision, codec chosen by the worker):
/// `params.source_sha256` addresses the healed kernel blob so replay never
/// re-parses STEP, while the user's original file is co-stored under
/// `params.provenance_sha256` for re-export / re-heal / audit. `blobs` carries
/// both, and `referenced_import_shas` pins both at save.
#[derive(Debug, Clone)]
pub struct PreparedImport {
    /// The record params (already `validate`d).
    pub params: ImportStepParams,
    /// `(sha256, blob)` for the converted replay blob AND the provenance STEP, in
    /// that order.
    pub blobs: Vec<(String, ImportBlob)>,
    /// How many bodies this import will mint (SCHEMA §7.3 ordinal children).
    pub solid_count: usize,
    /// STEP product names in ordinal order, for body naming (best effort).
    pub product_names: Vec<String>,
    /// Human-facing notes for the caller's hint text (`(code, message)`).
    pub diagnostics: Vec<(String, String)>,
}

/// Reads `path`, probes it through the worker, and builds the record + blobs.
///
/// The document is NOT touched: on any failure the caller authors no record and
/// reports the error, so a bad STEP file leaves the open document exactly as it
/// was. The worker round-trip means the caller must NOT hold the `DocumentRuntime`
/// lock across this call.
///
/// # Errors
/// * [`ApiError::Io`] — unreadable file, or one over [`MAX_IMPORT_FILE_BYTES`].
/// * [`ApiError::OpFailed`] — the worker could not read the file, or recovered no
///   solid from it.
/// * [`ApiError::Worker`] — a protocol/transport failure.
pub async fn prepare_import(
    importer: &dyn StepImport,
    path: &Path,
) -> Result<PreparedImport, ApiError> {
    let meta = std::fs::metadata(path)
        .map_err(|e| ApiError::Io(format!("cannot read {}: {e}", path.display())))?;
    if meta.len() > MAX_IMPORT_FILE_BYTES {
        return Err(ApiError::Io(format!(
            "{} is {:.1} MiB — over the {} MiB import limit",
            path.display(),
            meta.len() as f64 / (1024.0 * 1024.0),
            MAX_IMPORT_FILE_BYTES / (1024 * 1024),
        )));
    }
    let source_bytes = std::fs::read(path)
        .map_err(|e| ApiError::Io(format!("cannot read {}: {e}", path.display())))?;
    let source_sha = sha256_hex(&source_bytes);

    // The probe IS the conversion lane: one read of the user's file yields both the
    // metadata and the canonical replay bytes the record will use (SCHEMA §7.8).
    let inspection: StepInspection = importer.inspect_step(path, true).await?;
    if inspection.solid_count == 0 {
        return Err(ApiError::OpFailed {
            message: format!(
                "no solid recovered from {} — nothing to import",
                path.display()
            ),
            diagnostics: Vec::new(),
        });
    }
    // The WORKER names its preferred replay codec; Rust does not hardcode one. An
    // unknown value is a hard stop rather than a fallback: authoring a record under
    // the wrong codec would persist a document whose replay reads the bytes with the
    // wrong reader.
    let codec = ImportSourceCodec::from_extension(&inspection.geometry_codec).ok_or_else(|| {
        ApiError::Worker(format!(
            "InspectStep reported geometryCodec `{}`, which this build cannot store",
            inspection.geometry_codec
        ))
    })?;
    if !codec.is_converted() {
        return Err(ApiError::Worker(format!(
            "InspectStep reported geometryCodec `{}` for the conversion lane — expected a \
             converted replay form",
            inspection.geometry_codec
        )));
    }
    let geometry_bytes = inspection.geometry_bytes.clone().ok_or_else(|| {
        ApiError::Worker("InspectStep returned no geometry bytes for the conversion lane".into())
    })?;
    let geometry_sha = sha256_hex(&geometry_bytes);

    let mut diagnostics = inspection.diagnostics.clone();
    if meta.len() > LARGE_IMPORT_HINT_BYTES {
        diagnostics.push((
            "STEP_LARGE_SOURCE".into(),
            format!(
                "{:.0} MiB source: it is stored in the document and kept in memory while the \
                 document is open",
                meta.len() as f64 / (1024.0 * 1024.0)
            ),
        ));
    }

    let source_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "import.step".to_string());

    // Degenerate but possible: a STEP whose converted blob hashes to the same digest
    // as the source file. `provenanceSha256` MUST differ from `sourceSha256` (core
    // validation), and it would address the same blob anyway, so drop it.
    let provenance = (geometry_sha != source_sha).then(|| source_sha.clone());

    let params = ImportStepParams {
        source_sha256: geometry_sha.clone(),
        source_codec: codec,
        source_name,
        heal_policy: IMPORT_HEAL_POLICY_V1.to_string(),
        unit_scale: Scalar::new(1.0),
        brep_format: Some(inspection.geometry_format),
        provenance_sha256: provenance,
        extra: Default::default(),
    };
    params.validate().map_err(ApiError::InvalidCommand)?;

    let mut blobs = vec![(
        geometry_sha,
        ImportBlob {
            codec,
            bytes: Arc::new(geometry_bytes),
        },
    )];
    if params.provenance_sha256.is_some() {
        blobs.push((
            source_sha,
            ImportBlob {
                codec: ImportSourceCodec::Step,
                bytes: Arc::new(source_bytes),
            },
        ));
    }

    tracing::info!(
        source = %params.source_name,
        solids = inspection.solid_count,
        unit = %inspection.source_unit,
        codec = %inspection.geometry_codec,
        geometry_format = inspection.geometry_format,
        sha = %params.source_sha256,
        "importStep: prepared"
    );

    Ok(PreparedImport {
        params,
        blobs,
        solid_count: inspection.solid_count,
        product_names: inspection.product_names,
        diagnostics,
    })
}

/// Lowercase-hex SHA-256 of `bytes` — the container's import-blob key form.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_is_the_container_key_form() {
        // Anchor: SHA-256 of the empty input, lowercase, 64 chars.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn workspace_materializes_registers_and_cleans_up() {
        let doc = DocumentId::new();
        let sha = sha256_hex(b"solid bytes");
        let path = {
            let mut ws = ImportWorkspace::new(doc);
            let path = ws
                .materialize(&sha, ImportSourceCodec::Brep, b"solid bytes")
                .expect("materialize");
            assert!(path.is_file(), "the blob must be readable by the worker");
            assert_eq!(
                resolve_blob_path(&sha),
                path.to_string_lossy(),
                "lowering must resolve the materialized path"
            );
            // Idempotent: the same digest twice is one file, one registration.
            let again = ws
                .materialize(&sha, ImportSourceCodec::Brep, b"solid bytes")
                .expect("re-materialize");
            assert_eq!(again, path);
            path
        };
        assert!(!path.exists(), "drop must remove the workspace directory");
        assert_eq!(
            resolve_blob_path(&sha),
            "",
            "drop must unregister the path so lowering reports MISSING, not a dead path"
        );
    }

    #[test]
    fn two_workspaces_over_one_digest_survive_each_others_teardown() {
        let sha = sha256_hex(b"shared bytes");
        let mut keeper = ImportWorkspace::new(DocumentId::new());
        let kept = keeper
            .materialize(&sha, ImportSourceCodec::Brep, b"shared bytes")
            .expect("materialize keeper");
        {
            let mut doomed = ImportWorkspace::new(DocumentId::new());
            let other = doomed
                .materialize(&sha, ImportSourceCodec::Brep, b"shared bytes")
                .expect("materialize doomed");
            assert_ne!(other, kept, "each runtime gets its OWN directory");
        }
        assert_eq!(
            resolve_blob_path(&sha),
            kept.to_string_lossy(),
            "a dropped workspace must not invalidate a live one's materialization"
        );
    }

    /// A [`StepImport`] that fails the test if it is ever called — the size cap must
    /// reject BEFORE any worker round-trip (and before the bytes are read).
    struct NeverProbed;

    #[async_trait::async_trait]
    impl StepImport for NeverProbed {
        async fn inspect_step(
            &self,
            _path: &Path,
            _include_geometry: bool,
        ) -> Result<StepInspection, onecad_core::regen::EngineError> {
            panic!("an over-cap file must be rejected before the worker is asked to read it");
        }
    }

    #[tokio::test]
    async fn an_over_cap_source_is_rejected_by_size_alone() {
        let dir = std::env::temp_dir().join(format!("onecad-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("huge.step");
        // Sparse: `set_len` past the cap costs no disk and no read.
        let file = std::fs::File::create(&path).expect("create");
        file.set_len(MAX_IMPORT_FILE_BYTES + 1).expect("set_len");
        drop(file);

        let err = prepare_import(&NeverProbed, &path)
            .await
            .expect_err("over-cap must be rejected");
        let message = err.to_string();
        assert!(
            matches!(err, ApiError::Io(_)) && message.contains("256"),
            "the rejection must NAME the cap so the user knows the limit, got {message:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sweep_leaves_fresh_workspaces_alone() {
        let mut ws = ImportWorkspace::new(DocumentId::new());
        let sha = sha256_hex(b"fresh");
        let path = ws
            .materialize(&sha, ImportSourceCodec::Step, b"fresh")
            .expect("materialize");
        sweep_stale_workspaces(STALE_WORKSPACE_AGE);
        assert!(path.is_file(), "a just-created workspace is never stale");
    }
}
