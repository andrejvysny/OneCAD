//! The v2 `.onecad` container (a ZIP archive): atomic writer + guarded reader.
//!
//! # Sections
//!
//! | path | class | cap | codec |
//! |------|-------|-----|-------|
//! | `manifest.json` | index (identity-validated) | 64 MB | Deflated |
//! | `document.json` | **authoritative** (hash → `Corrupt`) | 64 MB | Deflated |
//! | `timeline/ops.jsonl` | derived projection (mismatch → warn) | 256 MB | Deflated |
//! | `geometry/<bodyId>.brep` | cache (mismatch → stale) | 1 GB | Stored |
//! | `meshes/<bodyId>.<lod>.mesh` | cache | 1 GB | Stored |
//! | `checkpoints/<step>.json` / `.bin` | cache | 1 GB | Stored |
//! | `imports/<sha256>.step`/`.brep` | **authoritative-for-a-record** (see [`super::imports`]) | 256 MB | Deflated |
//! | `preview.png` | cache | 1 GB | Stored |
//!
//! Whole-container caps: **4 GB** total (declared uncompressed) and **10 000**
//! entries. Text sections are Deflated (compressible JSON); opaque binary caches
//! are Stored (already compact — deflating burns CPU for ~nothing). Entries under
//! 64 bytes are Stored regardless (deflate framing exceeds any saving).
//!
//! What the app actually WRITES today: `document.json`, `timeline/ops.jsonl`,
//! `imports/`, and — on an **explicit** save only — `meshes/<bodyId>.coarse.mesh`
//! plus `preview.png` (the "paint last-saved geometry at open" caches). An
//! autosave writes no cache section at all, and `checkpoints/` is never written
//! (SCHEMA §7.7 V2 policy: worker restore is in-session-map-only).
//!
//! # Mesh cache entry names
//!
//! [`mesh_cache_path`] / [`parse_mesh_cache_path`] are an exact inverse pair.
//! Both components are percent-escaped for `%` and `:` — `:` is illegal in a
//! Windows filename, and the SCHEMA §2 N-body wire form (`body_<opId>:<k>`) puts
//! one in the id family this path is named after. A core [`BodyId`] is a bare
//! UUID today, so the escape never fires for the body component; it is symmetric
//! anyway so a colon-bearing component can never mint an unopenable container.
//! `/` and `..` are deliberately NOT escaped — [`guard_authored_name`] must stay
//! the traversal gate, and escaping them would turn a rejection into a silent
//! rename.
//!
//! # Atomic save
//!
//! [`ContainerWriter::save`] writes to a sibling temp file (`.<name>.tmp-<nonce>`),
//! `fsync`s it, atomically `rename`s over the target, then `fsync`s the parent
//! directory (durability of the rename on macOS). The original target is never
//! touched until the rename, and a leftover temp from a crash-before-rename is
//! cleaned up on the next save. So a crash at any point leaves *either* the intact
//! previous file *or* the fully-written new one — never a torn container.
//!
//! # Attack surface
//!
//! [`ContainerReader::open`] treats the archive as adversarial: it enforces the
//! entry-count + total-size caps, rejects path-traversal entry names (`../`,
//! absolute paths, symlink-ish names) via [`zip`]'s `enclosed_name`, and bounds
//! every decompressed read (`Take`-limited — a zip bomb hits its per-section cap
//! and yields [`IoError::TooLarge`] rather than exhausting memory). No hostile
//! input path panics.

use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use zip::result::ZipError;
use zip::{CompressionMethod, ZipArchive};

use crate::document::record::{KnownOperation, Operation};
use crate::document::Document;
use crate::ids::BodyId;

use super::imports::{
    import_sections, parse_import_blob_path, verify_blob, ImportBlobError, ImportBlobInfo,
    ImportBlobResult, ImportBlobs, IMPORTS_DIR, MAX_IMPORT_BLOB_BYTES,
};
use super::manifest::{
    Manifest, ManifestEntry, ModuleDescriptor, CONTAINER_VERSION, GLOBAL_SCHEMA_VERSION, MAGIC,
};
use super::migrate::{LoadOutcome, MigrationRegistry};
use super::{document_io, history_io, sha256_hex, Diagnostic, IoError, IoResult};

// ── Section paths ────────────────────────────────────────────────────────────

/// The manifest index path.
pub const MANIFEST_PATH: &str = "manifest.json";
/// The authoritative document body path.
pub const DOCUMENT_PATH: &str = "document.json";
/// The derived timeline projection path.
pub const OPS_PATH: &str = "timeline/ops.jsonl";
/// Geometry (BREP) cache directory prefix.
pub const GEOMETRY_DIR: &str = "geometry/";
/// Mesh (MESH1) cache directory prefix.
pub const MESHES_DIR: &str = "meshes/";
/// Checkpoint artifact directory prefix.
pub const CHECKPOINTS_DIR: &str = "checkpoints/";
/// Optional preview thumbnail path.
pub const PREVIEW_PATH: &str = "preview.png";
/// Mesh cache entry suffix (`meshes/<bodyId>.<lod>.mesh`).
const MESH_EXT: &str = ".mesh";

/// The archive path for a mesh cache blob: `meshes/<bodyId>.<lod>.mesh`, with both
/// components percent-escaped (see the module docs). The exact inverse of
/// [`parse_mesh_cache_path`].
#[must_use]
pub fn mesh_cache_path(body: BodyId, lod: &str) -> String {
    format!(
        "{MESHES_DIR}{}.{}{MESH_EXT}",
        escape_component(&body.to_string()),
        escape_component(lod)
    )
}

/// Splits a `meshes/<bodyId>.<lod>.mesh` entry name into its `(body, lod)`.
///
/// `None` for anything else — a foreign, misnamed or nested entry under `meshes/`
/// is IGNORED rather than guessed at (the [`parse_import_blob_path`] precedent),
/// so a container written by a future build still opens here.
#[must_use]
pub fn parse_mesh_cache_path(path: &str) -> Option<(BodyId, String)> {
    let rest = path.strip_prefix(MESHES_DIR)?;
    if rest.contains('/') {
        return None;
    }
    let stem = rest.strip_suffix(MESH_EXT)?;
    // The body component is a UUID (never contains `.`), so the FIRST dot splits.
    let (body, lod) = stem.split_once('.')?;
    if lod.is_empty() {
        return None;
    }
    let body = BodyId::from_str(&unescape_component(body)).ok()?;
    Some((body, unescape_component(lod)))
}

/// Percent-escapes the two characters an entry-name component may not carry
/// verbatim: `%` (the escape marker itself — encoded FIRST so unescaping is exact)
/// and `:` (illegal in a Windows filename).
fn escape_component(s: &str) -> String {
    if !s.contains(['%', ':']) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '%' => out.push_str("%25"),
            ':' => out.push_str("%3A"),
            _ => out.push(c),
        }
    }
    out
}

/// The exact inverse of [`escape_component`]. An escape this writer never emits is
/// left VERBATIM rather than decoded: a foreign name must not silently become a
/// different id (it will simply fail the `BodyId` parse and be skipped).
fn unescape_component(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &s[i..];
        if let Some(tail) = rest.strip_prefix("%25") {
            out.push('%');
            i = s.len() - tail.len();
        } else if let Some(tail) = rest
            .strip_prefix("%3A")
            .or_else(|| rest.strip_prefix("%3a"))
        {
            out.push(':');
            i = s.len() - tail.len();
        } else {
            let c = rest.chars().next().unwrap_or('%');
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

// ── Caps (decompression-bomb / DoS guards) ───────────────────────────────────

const MB: u64 = 1024 * 1024;
/// Per-section cap for `manifest.json`.
pub const MAX_MANIFEST_BYTES: u64 = 64 * MB;
/// Per-section cap for `document.json`.
pub const MAX_DOCUMENT_BYTES: u64 = 64 * MB;
/// Per-section cap for `timeline/ops.jsonl`.
pub const MAX_OPS_BYTES: u64 = 256 * MB;
/// Per-section cap for any cache blob (geometry / mesh / checkpoint / preview).
pub const MAX_BLOB_BYTES: u64 = 1024 * MB;
/// Whole-container cap on total declared uncompressed size.
pub const MAX_TOTAL_BYTES: u64 = 4096 * MB;
/// Whole-container cap on entry count.
pub const MAX_ENTRIES: usize = 10_000;
/// Cap for the standalone [`read_preview`] path. Far below [`MAX_BLOB_BYTES`]: a
/// start-screen thumbnail is a small viewport capture, and this read runs 10× per
/// listing with no document open to justify the memory.
pub const MAX_PREVIEW_BYTES: u64 = 2 * MB;
/// Below this size, entries are Stored (deflate framing is not worth it).
const STORE_THRESHOLD: usize = 64;

// ─────────────────────────────────────────────────────────────────────────────
// Writer inputs
// ─────────────────────────────────────────────────────────────────────────────

/// A mesh cache blob (`meshes/<bodyId>.<lod>.mesh`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshCache {
    /// The body the mesh belongs to.
    pub body: BodyId,
    /// The level-of-detail tag (e.g. `"coarse"`) — used verbatim in the filename.
    pub lod: String,
    /// The MESH1 bytes (opaque to the core).
    ///
    /// Behind an `Arc` for the same reason
    /// [`ImportBlob::bytes`](super::imports::ImportBlob::bytes) is (VF-B7): the app
    /// assembles this carrier while holding its single-writer lock, from blobs it
    /// already holds behind an `Arc` in its mesh LRU. A `Vec<u8>` here would turn
    /// that assembly into a `memcpy` of the whole persisted mesh budget under the
    /// lock. The writer's own copy into a [`Section`] still happens — but off the
    /// lock, on the blocking thread, where it belongs.
    pub bytes: Arc<Vec<u8>>,
}

/// A checkpoint artifact pair (`checkpoints/<step>.json` + optional `.bin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointCache {
    /// The timeline step the checkpoint represents.
    pub step: usize,
    /// The envelope/metadata JSON bytes.
    pub json: Vec<u8>,
    /// The payload (BREP etc.) bytes, if any.
    pub bin: Option<Vec<u8>>,
}

/// Optional cache artifacts to embed alongside the document. Caches degrade
/// performance, never correctness — a reader may ignore all of them (Invariant 7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerCaches {
    /// Per-body BREP blobs (`geometry/<bodyId>.brep`).
    pub geometry: BTreeMap<BodyId, Vec<u8>>,
    /// Per-body/LOD mesh blobs.
    pub meshes: Vec<MeshCache>,
    /// Per-step checkpoint artifacts.
    pub checkpoints: Vec<CheckpointCache>,
    /// Optional preview thumbnail (`preview.png`).
    pub preview_png: Option<Vec<u8>>,
}

impl ContainerCaches {
    /// An empty cache set (a document-only container).
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// True iff there are no cache artifacts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.geometry.is_empty()
            && self.meshes.is_empty()
            && self.checkpoints.is_empty()
            && self.preview_png.is_none()
    }
}

/// Caller-supplied manifest metadata (provenance). Timestamps are RFC3339 strings
/// passed IN — the pure core never reads the wall clock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveMeta {
    /// The authoring app version.
    pub app_version: String,
    /// The OCCT fingerprint the caches were produced under, if any.
    pub occt_fingerprint: Option<String>,
    /// RFC3339 creation timestamp.
    pub created: String,
    /// RFC3339 last-modified timestamp.
    pub modified: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Writer
// ─────────────────────────────────────────────────────────────────────────────

/// One in-memory section to write: its path and uncompressed bytes.
struct Section {
    path: String,
    bytes: Vec<u8>,
}

/// Global temp-file nonce counter (uniqueness across concurrent saves in-process).
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes v2 `.onecad` containers atomically.
#[derive(Debug, Default)]
pub struct ContainerWriter;

impl ContainerWriter {
    /// Atomically writes `document` (+ optional `caches`) to `path`.
    ///
    /// # Errors
    /// [`IoError`] on a serialization or filesystem failure. On any failure the
    /// target file is left untouched and the temp file is removed.
    pub fn save(
        path: &Path,
        document: &Document,
        caches: &ContainerCaches,
        meta: &SaveMeta,
    ) -> IoResult<()> {
        Self::save_inner(path, document, caches, &ImportBlobs::new(), meta, true).map(|_| ())
    }

    /// Atomically writes `document` (+ optional `caches`) to `path`, additionally
    /// persisting the `imports` blobs its `ImportStep` records reference.
    ///
    /// Only REFERENCED blobs are written and orphans are dropped — see
    /// [`referenced_import_shas`](super::imports::referenced_import_shas) for the
    /// refcount rule. A document with no `ImportStep` record writes no `imports/`
    /// entry at all and is byte-identical to the same
    /// [`save`](ContainerWriter::save).
    ///
    /// # Errors
    /// [`IoError`] on a serialization or filesystem failure, on a malformed /
    /// mis-keyed blob key, or on a blob over
    /// [`MAX_IMPORT_BLOB_BYTES`]. On any failure the target file is left
    /// untouched and the temp file is removed.
    pub fn save_with_imports(
        path: &Path,
        document: &Document,
        caches: &ContainerCaches,
        imports: &ImportBlobs,
        meta: &SaveMeta,
    ) -> IoResult<()> {
        Self::save_inner(path, document, caches, imports, meta, true).map(|_| ())
    }

    /// Crash-simulation hook: writes + fsyncs the temp file but **skips** the
    /// rename, leaving the target untouched and the temp on disk. Returns the temp
    /// path. Used to prove atomicity (a crash between temp-write and rename leaves
    /// the previous container intact); the leftover is cleaned up by the next
    /// [`save`](ContainerWriter::save). Test-only (compiled only under `cfg(test)`,
    /// so it is never dead code in release/consumer builds).
    #[cfg(test)]
    pub(crate) fn save_leaving_temp(
        path: &Path,
        document: &Document,
        caches: &ContainerCaches,
        meta: &SaveMeta,
    ) -> IoResult<PathBuf> {
        Self::save_inner(path, document, caches, &ImportBlobs::new(), meta, false)
    }

    fn save_inner(
        path: &Path,
        document: &Document,
        caches: &ContainerCaches,
        imports: &ImportBlobs,
        meta: &SaveMeta,
        commit: bool,
    ) -> IoResult<PathBuf> {
        let sections = build_sections(document, caches, imports)?;
        let manifest = build_manifest(document, meta, &sections);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| IoError::Corrupt(format!("manifest serialize: {e}")))?;

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let tmp = temp_path(path);

        // Write the archive to the temp file; on any error, clean the temp up.
        if let Err(e) = write_archive(&tmp, &manifest_bytes, &sections) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }

        if !commit {
            return Ok(tmp);
        }

        // Best-effort cleanup of stale temps from a prior crash (not our own).
        cleanup_stale_temps(path, &tmp);
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            IoError::Io(format!("rename temp → target: {e}"))
        })?;
        fsync_dir(parent);
        Ok(path.to_path_buf())
    }
}

/// Builds the section list (authoritative + derived + imports + caches) with
/// validated names.
fn build_sections(
    document: &Document,
    caches: &ContainerCaches,
    imports: &ImportBlobs,
) -> IoResult<Vec<Section>> {
    let mut sections = Vec::new();
    sections.push(Section {
        path: DOCUMENT_PATH.to_string(),
        bytes: document_io::serialize_document(document)?,
    });
    sections.push(Section {
        path: OPS_PATH.to_string(),
        bytes: history_io::serialize_ops_jsonl(document.timeline.records())?,
    });
    // Authoritative-for-a-record import blobs, refcounted against live records.
    for (path, bytes) in import_sections(document, imports)? {
        guard_authored_name(&path)?;
        sections.push(Section {
            path,
            bytes: bytes.to_vec(),
        });
    }
    for (body, bytes) in &caches.geometry {
        sections.push(Section {
            path: format!("{GEOMETRY_DIR}{body}.brep"),
            bytes: bytes.clone(),
        });
    }
    for mesh in &caches.meshes {
        let path = mesh_cache_path(mesh.body, &mesh.lod);
        guard_authored_name(&path)?;
        sections.push(Section {
            path,
            bytes: mesh.bytes.as_ref().clone(),
        });
    }
    for cp in &caches.checkpoints {
        sections.push(Section {
            path: format!("{CHECKPOINTS_DIR}{}.json", cp.step),
            bytes: cp.json.clone(),
        });
        if let Some(bin) = &cp.bin {
            sections.push(Section {
                path: format!("{CHECKPOINTS_DIR}{}.bin", cp.step),
                bytes: bin.clone(),
            });
        }
    }
    if let Some(png) = &caches.preview_png {
        sections.push(Section {
            path: PREVIEW_PATH.to_string(),
            bytes: png.clone(),
        });
    }
    Ok(sections)
}

/// Rejects an authored (writer-side) entry name that would escape the archive
/// root — defense in depth against a hostile `lod`/component leaking `/` or `..`.
fn guard_authored_name(name: &str) -> IoResult<()> {
    if name.contains("..") || name.contains('\\') || name.starts_with('/') {
        return Err(IoError::PathTraversal(name.to_string()));
    }
    // Exactly one path component beyond the known prefix (no injected separators).
    let component_count = name.split('/').count();
    if component_count > 2 {
        return Err(IoError::PathTraversal(name.to_string()));
    }
    Ok(())
}

/// Assembles the manifest from the sections (hashing each entry's bytes).
fn build_manifest(document: &Document, meta: &SaveMeta, sections: &[Section]) -> Manifest {
    let entries = sections
        .iter()
        .map(|s| ManifestEntry {
            path: s.path.clone(),
            sha256: sha256_hex(&s.bytes),
        })
        .collect();
    Manifest {
        magic: MAGIC.to_string(),
        container_version: CONTAINER_VERSION,
        global_schema_version: document.schema_version,
        app_version: meta.app_version.clone(),
        occt_fingerprint: meta.occt_fingerprint.clone(),
        document_id: document.id,
        created: meta.created.clone(),
        modified: meta.modified.clone(),
        ops_hash: history_io::ops_hash(document.timeline.records()),
        entries,
        // Derived, never authored: the document's own module table is the single
        // source of truth, so the manifest can never disagree with it.
        modules: document
            .modules
            .iter()
            .map(|(id, state)| {
                (
                    id.clone(),
                    ModuleDescriptor {
                        schema_version: state.schema_version,
                        extra: Default::default(),
                    },
                )
            })
            .collect(),
        extra: Default::default(),
    }
}

/// Writes the manifest + sections into a fresh zip at `tmp`, fsyncing the file.
fn write_archive(tmp: &Path, manifest_bytes: &[u8], sections: &[Section]) -> IoResult<()> {
    let file = std::fs::File::create(tmp)?;
    let mut zip = zip::ZipWriter::new(file);

    write_zip_entry(&mut zip, MANIFEST_PATH, manifest_bytes)?;
    for s in sections {
        write_zip_entry(&mut zip, &s.path, &s.bytes)?;
    }
    let file = zip
        .finish()
        .map_err(|e| IoError::Io(format!("zip finish: {e}")))?;
    file.sync_all()?;
    Ok(())
}

/// Writes one zip entry with the size/type-appropriate compression method and a
/// fixed timestamp (deterministic output).
fn write_zip_entry<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    name: &str,
    bytes: &[u8],
) -> IoResult<()> {
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(compression_for(name, bytes.len()))
        .last_modified_time(zip::DateTime::default());
    zip.start_file(name, options)
        .map_err(|e| IoError::Io(format!("zip start_file {name}: {e}")))?;
    zip.write_all(bytes)
        .map_err(|e| IoError::Io(format!("zip write {name}: {e}")))?;
    Ok(())
}

/// Deflate compressible text sections; Store opaque binary caches and tiny files.
fn compression_for(name: &str, len: usize) -> CompressionMethod {
    if len < STORE_THRESHOLD {
        return CompressionMethod::Stored;
    }
    // Import blobs are STEP/BREP text — highly compressible, and the one section
    // family where a user routinely ships tens of MB.
    let is_text = name == MANIFEST_PATH
        || name == DOCUMENT_PATH
        || name == OPS_PATH
        || name.starts_with(IMPORTS_DIR)
        || name.ends_with(".json");
    if is_text {
        CompressionMethod::Deflated
    } else {
        CompressionMethod::Stored
    }
}

/// A unique sibling temp path for `path` (`.<name>.tmp-<pid>-<nanos>-<ctr>`).
fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "container".into());
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let ctr = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.tmp-{pid}-{nanos}-{ctr}"))
}

/// The temp-file prefix for a target (used to find/clean stale temps).
fn temp_prefix(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "container".into());
    format!(".{name}.tmp-")
}

/// Removes leftover temp files for `target` (from a prior crash), except `keep`.
fn cleanup_stale_temps(target: &Path, keep: &Path) {
    let Some(parent) = target.parent().or_else(|| Some(Path::new("."))) else {
        return;
    };
    let prefix = temp_prefix(target);
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p == keep {
            continue;
        }
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(prefix.as_str())
        {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// fsyncs a directory so a rename inside it is durable (macOS/Unix). A no-op / best
/// effort elsewhere.
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

// ─────────────────────────────────────────────────────────────────────────────
// Reader
// ─────────────────────────────────────────────────────────────────────────────

/// The result of reading a cache blob on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheRead {
    /// The blob was present and its hash matched the manifest.
    Present(Vec<u8>),
    /// The blob is stale (container-level `opsHash` mismatch, or its own hash
    /// no longer matches the manifest) — must not be used (Invariant 7).
    Stale,
    /// No such cache entry in this container.
    Missing,
}

/// One `meshes/<bodyId>.<lod>.mesh` entry present in a container — its identity
/// and archive path, **without reading any bytes**. Read the blob with
/// [`LoadedContainer::read_caches`] (one archive open for the whole batch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshCacheEntry {
    /// The body the mesh belongs to.
    pub body: BodyId,
    /// The level-of-detail tag, decoded from the filename.
    pub lod: String,
    /// The full archive entry path (the key for [`LoadedContainer::read_caches`]).
    pub path: String,
}

/// An opened container: the manifest, the load outcome (document + policy), and
/// lazy access to cache blobs.
#[derive(Debug)]
pub struct LoadedContainer {
    /// The parsed, identity-validated manifest.
    pub manifest: Manifest,
    /// The load outcome: the (migrated) document, read-only flag, migration report,
    /// stale-caches flag and diagnostics.
    pub outcome: LoadOutcome,
    /// Import blobs present in the archive, indexed from the ZIP directory at
    /// open (names + declared sizes only — **no bytes are read**).
    imports: Vec<ImportBlobInfo>,
    /// The container path (reopened for lazy cache reads).
    source: PathBuf,
}

impl LoadedContainer {
    /// The loaded document.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.outcome.document
    }

    /// The cache entries listed in the manifest — everything but the
    /// authoritative document, the derived ops projection, and the
    /// authoritative-for-a-record `imports/` blobs (an import blob is NOT a
    /// cache: it can never be regenerated, so it must never be reported stale
    /// and discarded; see [`LoadedContainer::import_blobs`]).
    #[must_use]
    pub fn cache_entries(&self) -> Vec<&ManifestEntry> {
        self.manifest
            .entries
            .iter()
            .filter(|e| {
                e.path != DOCUMENT_PATH && e.path != OPS_PATH && !e.path.starts_with(IMPORTS_DIR)
            })
            .collect()
    }

    /// The import blobs present in this container — digest, codec and declared
    /// size, **without reading any bytes**.
    ///
    /// Indexed from the ZIP central directory during the open-time cap walk, so
    /// this is free. Entries under `imports/` that do not parse as
    /// `<sha256>.<known codec>` are omitted (a future codec is ignored, never
    /// guessed at) and never fail the open.
    #[must_use]
    pub fn import_blobs(&self) -> &[ImportBlobInfo] {
        &self.imports
    }

    /// Reads one import blob by its content hash, verifying the decompressed
    /// bytes against that hash.
    ///
    /// Unlike [`read_cache`](LoadedContainer::read_cache) this ignores
    /// `stale_caches` entirely: an import blob is source input, not a derived
    /// artifact, so a stale `opsHash` says nothing about it.
    ///
    /// # Errors
    /// * [`ImportBlobError::BadKey`] — `sha256` is not a 64-char lowercase hex digest.
    /// * [`ImportBlobError::Missing`] — no such blob in this container.
    /// * [`ImportBlobError::HashMismatch`] — the bytes were tampered with / rotted.
    /// * [`ImportBlobError::Io`] — transport failure, or the entry inflates past
    ///   [`MAX_IMPORT_BLOB_BYTES`] ([`IoError::TooLarge`]).
    pub fn read_import_blob(&self, sha256: &str) -> ImportBlobResult<Vec<u8>> {
        let info = self
            .imports
            .iter()
            .find(|i| i.sha256 == sha256)
            .ok_or_else(|| {
                if crate::document::record::is_sha256_hex(sha256) {
                    ImportBlobError::Missing(sha256.to_string())
                } else {
                    ImportBlobError::BadKey(sha256.to_string())
                }
            })?;
        let file = std::fs::File::open(&self.source).map_err(IoError::from)?;
        let mut archive = ZipArchive::new(std::io::BufReader::new(file))
            .map_err(|e| IoError::Corrupt(format!("reopen archive: {e}")))?;
        let bytes = read_named(&mut archive, &info.path, MAX_IMPORT_BLOB_BYTES)?
            .ok_or_else(|| ImportBlobError::Missing(sha256.to_string()))?;
        verify_blob(sha256, bytes)
    }

    /// Reads a cache blob by its archive path, on demand.
    ///
    /// Returns [`CacheRead::Stale`] without touching the archive when the
    /// container's caches are known stale (`opsHash` mismatch / post-migration), or
    /// when the blob's own hash no longer matches the manifest. Genuine IO / path
    /// / size failures are [`IoError`].
    ///
    /// # Errors
    /// [`IoError`] on a filesystem, traversal or size-cap failure.
    pub fn read_cache(&self, path: &str) -> IoResult<CacheRead> {
        if self.outcome.stale_caches {
            return Ok(CacheRead::Stale);
        }
        let mut archive = self.reopen()?;
        self.read_cache_from(&mut archive, path)
    }

    /// Reads MANY cache blobs with **one** archive open — the open-time batch form
    /// of [`read_cache`](LoadedContainer::read_cache), whose per-call reopen costs a
    /// full central-directory parse each time.
    ///
    /// Returns one `(path, CacheRead)` per input, in input order. A container whose
    /// caches are known stale short-circuits to all-[`Stale`](CacheRead::Stale)
    /// without touching the archive at all.
    ///
    /// # Errors
    /// [`IoError`] on a filesystem, traversal or size-cap failure.
    pub fn read_caches<S: AsRef<str>>(&self, paths: &[S]) -> IoResult<Vec<(String, CacheRead)>> {
        if self.outcome.stale_caches {
            return Ok(paths
                .iter()
                .map(|p| (p.as_ref().to_string(), CacheRead::Stale))
                .collect());
        }
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut archive = self.reopen()?;
        let mut out = Vec::with_capacity(paths.len());
        for p in paths {
            let path = p.as_ref();
            out.push((path.to_string(), self.read_cache_from(&mut archive, path)?));
        }
        Ok(out)
    }

    /// Every `meshes/<bodyId>.<lod>.mesh` entry the manifest lists, parsed to its
    /// identity. **No bytes are read** — pair it with
    /// [`read_caches`](LoadedContainer::read_caches). An entry whose name does not
    /// parse is omitted (never an error).
    #[must_use]
    pub fn mesh_cache_entries(&self) -> Vec<MeshCacheEntry> {
        self.cache_entries()
            .into_iter()
            .filter_map(|e| {
                parse_mesh_cache_path(&e.path).map(|(body, lod)| MeshCacheEntry {
                    body,
                    lod,
                    path: e.path.clone(),
                })
            })
            .collect()
    }

    /// Reopens the source archive for a lazy read.
    fn reopen(&self) -> IoResult<ZipArchive<std::io::BufReader<std::fs::File>>> {
        let file = std::fs::File::open(&self.source)?;
        ZipArchive::new(std::io::BufReader::new(file))
            .map_err(|e| IoError::Corrupt(format!("reopen archive: {e}")))
    }

    /// One cache read against an ALREADY-open archive. Callers must have checked
    /// `stale_caches` first (this does not).
    fn read_cache_from<R: Read + Seek>(
        &self,
        archive: &mut ZipArchive<R>,
        path: &str,
    ) -> IoResult<CacheRead> {
        let Some(entry) = self.manifest.entry(path) else {
            return Ok(CacheRead::Missing);
        };
        if path == DOCUMENT_PATH || path == OPS_PATH || path.starts_with(IMPORTS_DIR) {
            return Ok(CacheRead::Missing); // not a cache
        }
        match read_named(archive, path, MAX_BLOB_BYTES)? {
            Some(bytes) => {
                if sha256_hex(&bytes) == entry.sha256 {
                    Ok(CacheRead::Present(bytes))
                } else {
                    Ok(CacheRead::Stale)
                }
            }
            None => Ok(CacheRead::Missing),
        }
    }
}

/// Reads ONLY the `preview.png` cache out of a container, for a start-screen
/// listing — deliberately **not** [`ContainerReader::open`].
///
/// A recents listing renders up to ten cards and needs one small PNG from each.
/// `open` would, per file, parse + hash-verify `document.json`, run the migration
/// chain, and cross-validate `timeline/ops.jsonl` — the whole document pipeline,
/// for a thumbnail. This walks the ZIP directory (entry-count guard), reads the
/// manifest, and reads the one entry, bounded at [`MAX_PREVIEW_BYTES`].
///
/// **Integrity:** the bytes are verified against the manifest's own
/// `entries[preview.png].sha256` — the same content hash
/// [`read_cache`](LoadedContainer::read_cache) checks. Freshness (`opsHash`) is
/// deliberately NOT consulted: a preview is a picture of the last save, and a
/// document whose caches went stale still had that picture taken. A tampered or
/// truncated PNG fails the hash and yields `None`.
///
/// **Every failure mode is `None`.** Missing entry, missing manifest row,
/// oversized blob, hash mismatch, corrupt/absent/hostile archive — all of it. The
/// return type says so directly: there is no error a caller could act on, and a
/// thumbnail must never break a listing.
#[must_use]
pub fn read_preview(path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = ZipArchive::new(std::io::BufReader::new(file)).ok()?;
    if archive.len() > MAX_ENTRIES {
        return None;
    }
    let manifest_bytes = read_named(&mut archive, MANIFEST_PATH, MAX_MANIFEST_BYTES).ok()??;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).ok()?;
    manifest.validate_identity().ok()?;
    let expected = &manifest.entry(PREVIEW_PATH)?.sha256;
    let bytes = read_named(&mut archive, PREVIEW_PATH, MAX_PREVIEW_BYTES).ok()??;
    (sha256_hex(&bytes) == *expected).then_some(bytes)
}

/// Reads v2 `.onecad` containers with attack-surface guards.
#[derive(Debug, Default)]
pub struct ContainerReader;

impl ContainerReader {
    /// Opens a container with the default (empty v2.0) migration registry.
    ///
    /// # Errors
    /// [`IoError`] on a malformed / hostile / corrupt archive.
    pub fn open(path: &Path) -> IoResult<LoadedContainer> {
        Self::open_with_registry(path, &MigrationRegistry::new())
    }

    /// Opens a container, applying `registry`'s migration chain to older documents.
    ///
    /// # Errors
    /// [`IoError`] on a malformed / hostile / corrupt archive, or a hard migration
    /// failure. A newer-schema or low-confidence document is *not* an error — it
    /// opens read-only (see [`LoadOutcome`]).
    pub fn open_with_registry(
        path: &Path,
        registry: &MigrationRegistry,
    ) -> IoResult<LoadedContainer> {
        let file = std::fs::File::open(path)?;
        let mut archive = ZipArchive::new(std::io::BufReader::new(file))
            .map_err(|e| IoError::Corrupt(format!("open archive: {e}")))?;

        // Caps + traversal guard, and (free, same walk) the import-blob index.
        let imports = guard_directory(&mut archive)?;

        // Manifest (index) — identity-validated.
        let manifest_bytes = read_named(&mut archive, MANIFEST_PATH, MAX_MANIFEST_BYTES)?
            .ok_or_else(|| IoError::Corrupt("missing manifest.json".into()))?;
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| IoError::Corrupt(format!("manifest parse: {e}")))?;
        manifest.validate_identity()?;

        // Authoritative document body.
        let doc_bytes = read_named(&mut archive, DOCUMENT_PATH, MAX_DOCUMENT_BYTES)?
            .ok_or_else(|| IoError::Corrupt("missing document.json".into()))?;
        verify_authoritative(&manifest, DOCUMENT_PATH, &doc_bytes)?;
        let doc_value = document_io::parse_value(&doc_bytes)?;

        // Version-aware migration + typed decode.
        let (document, migrated) = super::migrate::migrate_and_decode(
            registry,
            doc_value,
            manifest.global_schema_version,
            GLOBAL_SCHEMA_VERSION,
        )?;
        validate_persisted_shells(&document)?;

        let mut diagnostics = migrated.diagnostics;

        // Derived ops.jsonl projection: verify integrity + cross-validate content.
        cross_validate_ops(&mut archive, &manifest, &document, &mut diagnostics)?;

        // Cache staleness: opsHash mismatch, or a migration changed the records.
        let recomputed = history_io::ops_hash(document.timeline.records());
        let stale_caches = migrated.records_changed || recomputed != manifest.ops_hash;
        if stale_caches {
            diagnostics.push(Diagnostic::warning(
                "stale-caches",
                "container caches are stale (opsHash mismatch); ignoring geometry/mesh/checkpoint caches",
            ));
        }

        let outcome = LoadOutcome {
            document,
            read_only: migrated.read_only,
            migration_report: migrated.report,
            stale_caches,
            diagnostics,
        };
        Ok(LoadedContainer {
            manifest,
            outcome,
            imports,
            source: path.to_path_buf(),
        })
    }
}

/// Rejects typed Shell records whose dependency view and worker input view differ.
fn validate_persisted_shells(document: &Document) -> IoResult<()> {
    for record in document.timeline.records() {
        let Operation::Known(KnownOperation::Shell(params)) = &record.op else {
            continue;
        };
        params.validate().map_err(|message| {
            IoError::Corrupt(format!("Shell operation {}: {message}", record.record_id))
        })?;
    }
    Ok(())
}

/// Validates the archive directory up front: entry count, path traversal, and the
/// total declared-uncompressed-size cap. Runs before any decompression.
///
/// Also returns the **import-blob index** built from the same walk — names and
/// declared sizes only, no decompression. An `imports/` entry that does not parse
/// as `<sha256>.<known codec>` is skipped, and an oversized one is indexed rather
/// than rejected: a bad import blob is a one-record failure surfaced at
/// [`LoadedContainer::read_import_blob`], never an open failure (see
/// [`super::imports`]). Import blobs DO count against the whole-container caps
/// enforced here.
fn guard_directory<R: Read + Seek>(archive: &mut ZipArchive<R>) -> IoResult<Vec<ImportBlobInfo>> {
    if archive.len() > MAX_ENTRIES {
        return Err(IoError::TooLarge(format!(
            "{} entries exceeds the {MAX_ENTRIES} cap",
            archive.len()
        )));
    }
    let mut imports = Vec::new();
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| IoError::Corrupt(format!("entry {i}: {e}")))?;
        if file.is_dir() {
            continue;
        }
        // `enclosed_name` returns None for any name that would escape the root
        // (absolute, `..`, drive-relative). A None here is a traversal attempt.
        if file.enclosed_name().is_none() {
            return Err(IoError::PathTraversal(file.name().to_string()));
        }
        if file.name().contains('\\') {
            return Err(IoError::PathTraversal(file.name().to_string()));
        }
        total = total.saturating_add(file.size());
        if total > MAX_TOTAL_BYTES {
            return Err(IoError::TooLarge(format!(
                "declared uncompressed size exceeds the {MAX_TOTAL_BYTES}-byte cap"
            )));
        }
        if let Some((sha256, codec)) = parse_import_blob_path(file.name()) {
            imports.push(ImportBlobInfo {
                sha256,
                codec,
                declared_size: file.size(),
                path: file.name().to_string(),
            });
        }
    }
    Ok(imports)
}

/// Reads a named entry, bounding the decompressed size to `cap`. `Ok(None)` when
/// absent.
fn read_named<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    cap: u64,
) -> IoResult<Option<Vec<u8>>> {
    match archive.by_name(name) {
        Ok(file) => {
            if file.enclosed_name().is_none() {
                return Err(IoError::PathTraversal(name.to_string()));
            }
            Ok(Some(read_capped(file, cap, name)?))
        }
        Err(ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(IoError::Corrupt(format!("read {name}: {e}"))),
    }
}

/// Reads at most `cap` bytes from `r`; more than `cap` is [`IoError::TooLarge`].
fn read_capped<R: Read>(r: R, cap: u64, what: &str) -> IoResult<Vec<u8>> {
    let mut buf = Vec::new();
    // Read one past the cap: if we get cap+1 bytes, the source is over budget.
    r.take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|e| IoError::Corrupt(format!("{what} read: {e}")))?;
    if buf.len() as u64 > cap {
        return Err(IoError::TooLarge(format!(
            "{what} exceeds its {cap}-byte cap"
        )));
    }
    Ok(buf)
}

/// Verifies an authoritative entry's bytes against the manifest hash. A mismatch
/// (or a missing manifest entry) is [`IoError::Corrupt`].
fn verify_authoritative(manifest: &Manifest, path: &str, bytes: &[u8]) -> IoResult<()> {
    let entry = manifest
        .entry(path)
        .ok_or_else(|| IoError::Corrupt(format!("manifest missing entry for {path}")))?;
    if sha256_hex(bytes) != entry.sha256 {
        return Err(IoError::Corrupt(format!("{path} hash mismatch (tampered)")));
    }
    Ok(())
}

/// Reads + integrity-checks + content-cross-validates the derived `ops.jsonl`
/// against the authoritative document, appending a `Warning` on any problem.
/// `document.json` always wins — this never fails the load.
fn cross_validate_ops<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    manifest: &Manifest,
    document: &Document,
    diagnostics: &mut Vec<Diagnostic>,
) -> IoResult<()> {
    let Some(ops_bytes) = read_named(archive, OPS_PATH, MAX_OPS_BYTES)? else {
        return Ok(()); // optional projection absent
    };
    // Integrity of the derived section (bit-rot / tamper). Not authoritative → warn.
    if let Some(entry) = manifest.entry(OPS_PATH) {
        if sha256_hex(&ops_bytes) != entry.sha256 {
            diagnostics.push(Diagnostic::warning(
                "ops-jsonl-integrity",
                "timeline/ops.jsonl hash mismatch; using document.json (authoritative)",
            ));
            return Ok(());
        }
    }
    match history_io::parse_ops_jsonl(&ops_bytes) {
        Ok(lines) => {
            if let Some(diag) = history_io::cross_validate(document.timeline.records(), &lines) {
                diagnostics.push(diag);
            }
        }
        Err(_) => diagnostics.push(Diagnostic::warning(
            "ops-jsonl-integrity",
            "timeline/ops.jsonl is unparseable; using document.json (authoritative)",
        )),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::modules::{ModuleId, ModuleState};
    use crate::document::record::{
        BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, ShellParams,
    };
    use crate::document::refs::{ElementKind, ElementRef, PrimaryRef};
    use crate::document::variables::Scalar;
    use crate::ids::{BodyId, DocumentId, ElementId, RecordId};
    use uuid::Uuid;

    fn extrude(seed: u128, distance: f64) -> crate::document::record::OperationRecord {
        let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: None,
            distance: Scalar::new(distance),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        }));
        crate::document::record::OperationRecord::new(
            RecordId(Uuid::from_u128(seed)),
            0,
            "Extrude",
            op,
        )
    }

    fn doc(seed: u128, dist: f64) -> Document {
        let mut d = Document::new(DocumentId(Uuid::from_u128(seed)));
        d.timeline.insert_at_cursor(extrude(0x10, dist));
        d.timeline.insert_at_cursor(extrude(0x11, dist + 1.0));
        d
    }

    fn meta() -> SaveMeta {
        SaveMeta {
            app_version: "0.1.0-test".into(),
            occt_fingerprint: Some("occt-7.9.3".into()),
            created: "2026-07-16T00:00:00Z".into(),
            modified: "2026-07-16T00:00:00Z".into(),
        }
    }

    #[test]
    fn save_then_open_preserves_document() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let d = doc(1, 10.0);
        ContainerWriter::save(&path, &d, &ContainerCaches::none(), &meta()).unwrap();

        let loaded = ContainerReader::open(&path).unwrap();
        assert!(!loaded.outcome.read_only);
        assert!(!loaded.outcome.stale_caches);
        // Document JSON equality (the strongest structural check).
        assert_eq!(
            serde_json::to_value(loaded.document()).unwrap(),
            serde_json::to_value(&d).unwrap()
        );
    }

    #[test]
    fn open_rejects_shell_typed_face_from_foreign_body() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foreign-shell-face.onecad");
        let target = BodyId(Uuid::from_u128(0x20));
        let foreign = BodyId(Uuid::from_u128(0x21));
        let face = ElementId::new("el_face");
        let op = Operation::Known(KnownOperation::Shell(ShellParams {
            thickness: Scalar::new(2.0),
            open_faces: vec![face.clone()],
            faces: vec![ElementRef {
                primary: Some(PrimaryRef {
                    body: foreign,
                    element: face,
                    kind: ElementKind::Face,
                    extra: Default::default(),
                }),
                intent: None,
                anchor: None,
                extra: Default::default(),
            }],
            target_body: Some(target),
            extra: Default::default(),
        }));
        let mut document = Document::new(DocumentId(Uuid::from_u128(0x22)));
        document
            .timeline
            .insert_at_cursor(crate::document::record::OperationRecord::new(
                RecordId(Uuid::from_u128(0x23)),
                0,
                "Shell",
                op,
            ));
        ContainerWriter::save(&path, &document, &ContainerCaches::none(), &meta()).unwrap();

        let err = ContainerReader::open(&path).unwrap_err();
        assert!(err.to_string().contains("typed ref body"));
    }

    /// W0 baseline for the Platform refactor: state this build cannot interpret
    /// must survive a full edit cycle, not merely a single serde round trip.
    ///
    /// This is the generalized form of the guarantee module/addon namespaces will
    /// rely on — the writer re-emits what the reader could not understand, across
    /// open → modify → save → reopen. Pinned now so a later change to the document
    /// mirror cannot quietly drop it.
    #[test]
    fn unknown_document_state_survives_open_modify_save() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");

        let mut authored = doc(1, 10.0);
        let foreign = serde_json::json!({ "schemaVersion": 3, "payload": { "note": "keep me" } });
        authored
            .extra
            .insert("comExampleFoo".into(), foreign.clone());
        ContainerWriter::save(&path, &authored, &ContainerCaches::none(), &meta()).unwrap();

        // Open, make a real modeling change, save again through the normal path.
        let opened = ContainerReader::open(&path).unwrap();
        let mut edited = opened.document().clone();
        assert_eq!(edited.extra.get("comExampleFoo"), Some(&foreign));
        edited.timeline.insert_at_cursor(extrude(0x12, 25.0));
        ContainerWriter::save(&path, &edited, &ContainerCaches::none(), &meta()).unwrap();

        let reopened = ContainerReader::open(&path).unwrap();
        assert_eq!(
            reopened.document().extra.get("comExampleFoo"),
            Some(&foreign),
            "unknown document state must round-trip verbatim through an edit"
        );
        assert_eq!(
            reopened.document().timeline.len(),
            3,
            "the modeling edit must still have been persisted"
        );
    }

    /// ADR-0004: adding module state must not change what an existing document
    /// serializes to. A stray `"modules": {}` would be a silent format change on
    /// every file the user re-saves.
    #[test]
    fn a_document_without_module_state_serializes_no_module_keys() {
        let d = doc(1, 10.0);
        let json = serde_json::to_string(&d).unwrap();
        assert!(
            !json.contains("\"modules\""),
            "an empty module table must not be written: {json}"
        );

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        ContainerWriter::save(&path, &d, &ContainerCaches::none(), &meta()).unwrap();
        let loaded = ContainerReader::open(&path).unwrap();
        assert!(loaded.manifest.modules.is_empty());
        assert!(!serde_json::to_string(&loaded.manifest)
            .unwrap()
            .contains("\"modules\""));
    }

    /// The manifest's descriptor table is what makes "this project uses an addon
    /// you do not have" answerable without parsing the payload.
    #[test]
    fn the_manifest_describes_every_module_the_document_carries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");

        let mut d = doc(1, 10.0);
        d.modules.insert(
            ModuleId::parse("com.example.foo").unwrap(),
            ModuleState::new(3, serde_json::json!({ "a": 1 })),
        );
        d.modules.insert(
            ModuleId::parse("onecad.modeling").unwrap(),
            ModuleState::new(1, serde_json::json!({})),
        );
        ContainerWriter::save(&path, &d, &ContainerCaches::none(), &meta()).unwrap();

        let loaded = ContainerReader::open(&path).unwrap();
        let modules = &loaded.manifest.modules;
        assert_eq!(modules.len(), 2);
        assert_eq!(
            modules[&ModuleId::parse("com.example.foo").unwrap()].schema_version,
            3
        );
        assert_eq!(
            modules[&ModuleId::parse("onecad.modeling").unwrap()].schema_version,
            1
        );
    }

    /// ADR-0005, the guarantee third-party document state rests on: state whose
    /// module is not installed survives a real editing cycle untouched.
    #[test]
    fn unknown_module_state_survives_open_modify_save() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let foreign = ModuleId::parse("com.example.absent").unwrap();
        let payload = serde_json::json!({
            "cost": { "currency": "EUR", "value": 12.5 },
            "refs": { "body-1": ["a", "b"] },
            "unicode": "påyload"
        });

        let mut authored = doc(1, 10.0);
        authored
            .modules
            .insert(foreign.clone(), ModuleState::new(7, payload.clone()));
        ContainerWriter::save(&path, &authored, &ContainerCaches::none(), &meta()).unwrap();

        // Open with NOTHING that understands `com.example.absent`, make a real
        // modeling change, and save through the ordinary path.
        let opened = ContainerReader::open(&path).unwrap();
        let mut edited = opened.document().clone();
        edited.timeline.insert_at_cursor(extrude(0x12, 25.0));
        ContainerWriter::save(&path, &edited, &ContainerCaches::none(), &meta()).unwrap();

        let reopened = ContainerReader::open(&path).unwrap();
        let survived = reopened
            .document()
            .modules
            .get(&foreign)
            .expect("the absent module's state must still be there");
        assert_eq!(survived.schema_version, 7);
        assert_eq!(survived.payload, payload, "payload must be byte-equal");
        assert_eq!(
            reopened.document().timeline.len(),
            3,
            "and the modeling edit must have persisted"
        );
        assert_eq!(reopened.manifest.modules[&foreign].schema_version, 7);
    }

    #[test]
    fn a_module_id_this_build_would_refuse_still_round_trips() {
        // A document written by a newer build (or a laxer tool) must not become
        // unloadable — refusing it would destroy the data preservation protects.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let odd = ModuleId::from_stored("Not.A.Valid.Id!".into());
        assert!(ModuleId::parse(odd.as_str()).is_err());

        let mut d = doc(1, 10.0);
        d.modules.insert(
            odd.clone(),
            ModuleState::new(1, serde_json::json!({ "x": 1 })),
        );
        ContainerWriter::save(&path, &d, &ContainerCaches::none(), &meta()).unwrap();

        let reopened = ContainerReader::open(&path).unwrap();
        assert_eq!(
            reopened.document().modules[&odd].payload,
            serde_json::json!({ "x": 1 })
        );
    }

    #[test]
    fn atomic_save_crash_leaves_original_intact_and_cleans_temp() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");

        // v1 committed.
        let v1 = doc(1, 10.0);
        ContainerWriter::save(&path, &v1, &ContainerCaches::none(), &meta()).unwrap();

        // Simulate a crash between temp-write and rename: temp written, not renamed.
        let v2 = doc(1, 999.0);
        let leftover =
            ContainerWriter::save_leaving_temp(&path, &v2, &ContainerCaches::none(), &meta())
                .unwrap();
        assert!(
            leftover.exists(),
            "temp file must exist after simulated crash"
        );

        // Original target is untouched: still opens as v1.
        let still_v1 = ContainerReader::open(&path).unwrap();
        assert_eq!(
            serde_json::to_value(still_v1.document()).unwrap(),
            serde_json::to_value(&v1).unwrap()
        );

        // Next real save cleans the stale temp and commits v3.
        let v3 = doc(1, 42.0);
        ContainerWriter::save(&path, &v3, &ContainerCaches::none(), &meta()).unwrap();
        assert!(
            !leftover.exists(),
            "stale temp must be cleaned on next save"
        );
        let now_v3 = ContainerReader::open(&path).unwrap();
        assert_eq!(
            serde_json::to_value(now_v3.document()).unwrap(),
            serde_json::to_value(&v3).unwrap()
        );
    }

    #[test]
    fn ops_hash_change_reports_stale_caches_but_still_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let d = doc(1, 10.0);
        let mut caches = ContainerCaches::none();
        caches
            .geometry
            .insert(crate::ids::BodyId(Uuid::from_u128(0xB0)), vec![1, 2, 3, 4]);
        ContainerWriter::save(&path, &d, &caches, &meta()).unwrap();

        // Tamper the manifest's opsHash so it no longer matches the records.
        rewrite_manifest_ops_hash(&path, "deadbeef");

        let loaded = ContainerReader::open(&path).unwrap();
        assert!(loaded.outcome.stale_caches);
        assert!(loaded
            .outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "stale-caches"));
        // Document still loads fine.
        assert_eq!(loaded.document().timeline.len(), 2);
        // Cache reads are reported stale (never used).
        let entry = loaded.cache_entries()[0].path.clone();
        assert_eq!(loaded.read_cache(&entry).unwrap(), CacheRead::Stale);
    }

    #[test]
    fn cache_round_trips_when_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let d = doc(1, 10.0);
        let body = crate::ids::BodyId(Uuid::from_u128(0xB0));
        let mut caches = ContainerCaches::none();
        caches.geometry.insert(body, vec![9, 8, 7, 6, 5]);
        caches.preview_png = Some(vec![0x89, 0x50, 0x4e, 0x47]);
        ContainerWriter::save(&path, &d, &caches, &meta()).unwrap();

        let loaded = ContainerReader::open(&path).unwrap();
        assert!(!loaded.outcome.stale_caches);
        let geo_path = format!("{GEOMETRY_DIR}{body}.brep");
        assert_eq!(
            loaded.read_cache(&geo_path).unwrap(),
            CacheRead::Present(vec![9, 8, 7, 6, 5])
        );
        assert_eq!(
            loaded.read_cache(PREVIEW_PATH).unwrap(),
            CacheRead::Present(vec![0x89, 0x50, 0x4e, 0x47])
        );
        assert_eq!(
            loaded.read_cache("geometry/nope.brep").unwrap(),
            CacheRead::Missing
        );
    }

    /// Rewrites the manifest.json inside a container, replacing its opsHash. Used
    /// to simulate stale caches / tamper without rebuilding the whole archive.
    fn rewrite_manifest_ops_hash(path: &Path, new_hash: &str) {
        let bytes = std::fs::read(path).unwrap();
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        // Read every entry out.
        let mut entries: Vec<(String, Vec<u8>, bool)> = Vec::new();
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).unwrap();
            let name = f.name().to_string();
            let is_dir = f.is_dir();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            entries.push((name, buf, is_dir));
        }
        drop(archive);
        // Rewrite manifest.json bytes.
        let out = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(out);
        for (name, buf, is_dir) in entries {
            if is_dir {
                continue;
            }
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored);
            zip.start_file(&name, opts).unwrap();
            if name == MANIFEST_PATH {
                let mut m: Manifest = serde_json::from_slice(&buf).unwrap();
                m.ops_hash = new_hash.to_string();
                zip.write_all(&serde_json::to_vec(&m).unwrap()).unwrap();
            } else {
                zip.write_all(&buf).unwrap();
            }
        }
        zip.finish().unwrap();
    }

    #[test]
    fn mesh_and_checkpoint_caches_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let d = doc(1, 10.0);
        let body = BodyId(Uuid::from_u128(0xB0));
        let caches = ContainerCaches {
            geometry: BTreeMap::new(),
            meshes: vec![MeshCache {
                body,
                lod: "coarse".into(),
                bytes: Arc::new(b"MESH1....".to_vec()),
            }],
            checkpoints: vec![CheckpointCache {
                step: 3,
                json: b"{\"envelope\":1}".to_vec(),
                bin: Some(vec![0xDE, 0xAD, 0xBE, 0xEF]),
            }],
            preview_png: None,
        };
        ContainerWriter::save(&path, &d, &caches, &meta()).unwrap();

        let loaded = ContainerReader::open(&path).unwrap();
        assert!(!loaded.outcome.stale_caches);
        assert_eq!(
            loaded
                .read_cache(&format!("{MESHES_DIR}{body}.coarse.mesh"))
                .unwrap(),
            CacheRead::Present(b"MESH1....".to_vec())
        );
        assert_eq!(
            loaded
                .read_cache(&format!("{CHECKPOINTS_DIR}3.json"))
                .unwrap(),
            CacheRead::Present(b"{\"envelope\":1}".to_vec())
        );
        assert_eq!(
            loaded
                .read_cache(&format!("{CHECKPOINTS_DIR}3.bin"))
                .unwrap(),
            CacheRead::Present(vec![0xDE, 0xAD, 0xBE, 0xEF])
        );
    }

    #[test]
    fn writer_rejects_traversal_in_mesh_lod() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let d = doc(1, 10.0);
        let caches = ContainerCaches {
            meshes: vec![MeshCache {
                body: BodyId(Uuid::from_u128(0xB0)),
                lod: "../../evil".into(), // hostile LOD tag
                bytes: Arc::new(vec![1, 2, 3]),
            }],
            ..ContainerCaches::none()
        };
        assert!(matches!(
            ContainerWriter::save(&path, &d, &caches, &meta()),
            Err(IoError::PathTraversal(_))
        ));
        // The target was never created (failure before any commit).
        assert!(!path.exists());
    }

    // ── Mesh cache entry naming ──────────────────────────────────────────────

    #[test]
    fn mesh_cache_path_and_parse_are_inverses() {
        let body = BodyId(Uuid::from_u128(0xB0));
        let path = mesh_cache_path(body, "coarse");
        assert_eq!(path, format!("{MESHES_DIR}{body}.coarse{MESH_EXT}"));
        assert_eq!(
            parse_mesh_cache_path(&path),
            Some((body, "coarse".to_string()))
        );
    }

    /// A `:`-bearing component (SCHEMA §2's `body_<opId>:<k>` split-child family)
    /// must round-trip through `%3A` — a literal colon is an illegal Windows
    /// filename and would make the container unopenable on that platform.
    #[test]
    fn colon_in_a_component_round_trips_as_percent_3a() {
        let body = BodyId(Uuid::from_u128(0xB0));
        let path = mesh_cache_path(body, "coarse:0");
        assert!(
            path.ends_with("coarse%3A0.mesh"),
            "the colon must be escaped, got {path}"
        );
        assert!(!path.contains(':'), "no literal colon survives: {path}");
        assert_eq!(
            parse_mesh_cache_path(&path),
            Some((body, "coarse:0".to_string())),
            "and it decodes back to the exact component"
        );
        // The escape marker itself round-trips (so escaping is injective).
        let literal = mesh_cache_path(body, "a%3Ab");
        assert_eq!(
            parse_mesh_cache_path(&literal),
            Some((body, "a%3Ab".to_string())),
            "a literal `%3A` in the component is NOT confused with an escaped colon"
        );
    }

    #[test]
    fn parse_mesh_cache_path_rejects_foreign_names() {
        for bad in [
            "meshes/not-a-uuid.coarse.mesh",
            "meshes/nested/dir.coarse.mesh",
            "meshes/00000000-0000-0000-0000-0000000000b0.coarse.brep",
            "meshes/00000000-0000-0000-0000-0000000000b0.mesh", // no lod component
            "geometry/00000000-0000-0000-0000-0000000000b0.coarse.mesh",
            "meshes/../escape.coarse.mesh",
        ] {
            assert_eq!(parse_mesh_cache_path(bad), None, "{bad} must not parse");
        }
    }

    // ── Batch cache reads ────────────────────────────────────────────────────

    #[test]
    fn mesh_entries_and_batch_read_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let d = doc(1, 10.0);
        let a = BodyId(Uuid::from_u128(0xB0));
        let b = BodyId(Uuid::from_u128(0xB1));
        let caches = ContainerCaches {
            meshes: vec![
                MeshCache {
                    body: a,
                    lod: "coarse".into(),
                    bytes: Arc::new(b"MESH1-a".to_vec()),
                },
                MeshCache {
                    body: b,
                    lod: "coarse".into(),
                    bytes: Arc::new(b"MESH1-b".to_vec()),
                },
            ],
            preview_png: Some(vec![0x89, 0x50, 0x4e, 0x47]),
            ..ContainerCaches::none()
        };
        ContainerWriter::save(&path, &d, &caches, &meta()).unwrap();

        let loaded = ContainerReader::open(&path).unwrap();
        let mut entries = loaded.mesh_cache_entries();
        entries.sort_by_key(|x| x.body);
        assert_eq!(entries.len(), 2, "preview.png is not a mesh entry");
        assert_eq!(entries[0].body, a);
        assert_eq!(entries[0].lod, "coarse");
        assert_eq!(entries[1].body, b);

        let paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
        let read = loaded.read_caches(&paths).unwrap();
        assert_eq!(
            read,
            vec![
                (paths[0].clone(), CacheRead::Present(b"MESH1-a".to_vec())),
                (paths[1].clone(), CacheRead::Present(b"MESH1-b".to_vec())),
            ],
            "batch read returns one result per input, in input order"
        );
        // The preview is still reachable through the generic single read.
        assert_eq!(
            loaded.read_cache(PREVIEW_PATH).unwrap(),
            CacheRead::Present(vec![0x89, 0x50, 0x4e, 0x47])
        );
    }

    #[test]
    fn batch_read_reports_every_entry_stale_when_the_container_is() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let caches = ContainerCaches {
            meshes: vec![MeshCache {
                body: BodyId(Uuid::from_u128(0xB0)),
                lod: "coarse".into(),
                bytes: Arc::new(b"MESH1-a".to_vec()),
            }],
            ..ContainerCaches::none()
        };
        ContainerWriter::save(&path, &doc(1, 10.0), &caches, &meta()).unwrap();
        rewrite_manifest_ops_hash(&path, "deadbeef");

        let loaded = ContainerReader::open(&path).unwrap();
        let paths: Vec<String> = loaded
            .mesh_cache_entries()
            .iter()
            .map(|e| e.path.clone())
            .collect();
        assert_eq!(paths.len(), 1);
        assert_eq!(
            loaded.read_caches(&paths).unwrap(),
            vec![(paths[0].clone(), CacheRead::Stale)]
        );
    }

    // ── read_preview (the start-screen lane) ─────────────────────────────────

    #[test]
    fn read_preview_returns_the_verified_png() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        let caches = ContainerCaches {
            preview_png: Some(png.clone()),
            ..ContainerCaches::none()
        };
        ContainerWriter::save(&path, &doc(1, 10.0), &caches, &meta()).unwrap();
        assert_eq!(read_preview(&path), Some(png));
    }

    /// A stale `opsHash` invalidates the GEOMETRY caches, not the picture of the
    /// last save. `read_preview` must still serve it.
    #[test]
    fn read_preview_ignores_cache_staleness() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let caches = ContainerCaches {
            preview_png: Some(vec![1, 2, 3, 4]),
            ..ContainerCaches::none()
        };
        ContainerWriter::save(&path, &doc(1, 10.0), &caches, &meta()).unwrap();
        rewrite_manifest_ops_hash(&path, "deadbeef");
        assert_eq!(read_preview(&path), Some(vec![1, 2, 3, 4]));
    }

    #[test]
    fn read_preview_is_none_for_absent_missing_and_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        // A container with NO preview section.
        let plain = tmp.path().join("plain.onecad");
        ContainerWriter::save(&plain, &doc(1, 10.0), &ContainerCaches::none(), &meta()).unwrap();
        assert_eq!(read_preview(&plain), None, "no preview.png entry");

        // A path that is not a file at all.
        assert_eq!(read_preview(&tmp.path().join("nope.onecad")), None);

        // Not a ZIP.
        let garbage = tmp.path().join("garbage.onecad");
        std::fs::write(&garbage, b"definitely not a zip archive").unwrap();
        assert_eq!(read_preview(&garbage), None, "corrupt archive");
    }

    #[test]
    fn read_preview_rejects_a_tampered_png() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let caches = ContainerCaches {
            preview_png: Some(vec![0x89, 0x50, 0x4e, 0x47]),
            ..ContainerCaches::none()
        };
        ContainerWriter::save(&path, &doc(1, 10.0), &caches, &meta()).unwrap();
        rewrite_entry(&path, PREVIEW_PATH, b"hostile replacement bytes");
        assert_eq!(
            read_preview(&path),
            None,
            "the manifest sha no longer matches"
        );
    }

    /// The 2 MiB cap is enforced on the DECOMPRESSED stream, so a small deflated
    /// entry that inflates past it still yields `None` rather than the bytes.
    #[test]
    fn read_preview_rejects_an_oversized_png() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.onecad");
        let png = vec![0x41u8; (MAX_PREVIEW_BYTES + 1) as usize];
        let caches = ContainerCaches {
            preview_png: Some(png),
            ..ContainerCaches::none()
        };
        ContainerWriter::save(&path, &doc(1, 10.0), &caches, &meta()).unwrap();
        // Sanity: the full reader (1 GB cap) DOES serve it — only the light lane caps.
        let loaded = ContainerReader::open(&path).unwrap();
        assert!(matches!(
            loaded.read_cache(PREVIEW_PATH).unwrap(),
            CacheRead::Present(_)
        ));
        assert_eq!(read_preview(&path), None, "over MAX_PREVIEW_BYTES");
    }

    /// Replaces one entry's bytes inside a container, leaving the manifest (and
    /// therefore its sha) untouched — the tamper case.
    fn rewrite_entry(path: &Path, target: &str, replacement: &[u8]) {
        let bytes = std::fs::read(path).unwrap();
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).unwrap();
            if f.is_dir() {
                continue;
            }
            let name = f.name().to_string();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            entries.push((name, buf));
        }
        drop(archive);
        let out = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(out);
        for (name, buf) in entries {
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored);
            zip.start_file(&name, opts).unwrap();
            if name == target {
                zip.write_all(replacement).unwrap();
            } else {
                zip.write_all(&buf).unwrap();
            }
        }
        zip.finish().unwrap();
    }
}
