//! Import a `.onecad` project into the current document.
//!
//! A project import is a **static snapshot**: the source document's timeline is
//! appended to the current document's timeline and its import blobs are copied
//! into the current carrier. Regen then replays the merged timeline from 0.
//!
//! Live cross-document references (XREF) are explicitly out of scope for V1.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::document::record::OperationRecord;
use crate::document::Document;
use crate::ids::{RecordId, SketchId};
use crate::io::container::ContainerReader;
use crate::io::imports::{ImportBlob, ImportBlobError};
use crate::io::IoError;
use crate::sketch::Sketch;

/// Everything extracted from a source `.onecad` document that is eligible for
/// import into another document.
#[derive(Debug, Clone)]
pub struct ImportedProject {
    /// Source timeline records, in order. Their [`RecordId`]s are preserved so
    /// worker-minted body ids stay deterministic across the import.
    pub records: Vec<OperationRecord>,
    /// Source sketches keyed by id. Collisions with the target document are the
    /// caller's responsibility to detect/remap.
    pub sketches: BTreeMap<SketchId, Sketch>,
    /// Import blobs referenced by the source timeline's `ImportStep` records.
    /// `(sha256, blob)` — content-addressed, deduplicated by the carrier on insert.
    pub import_blobs: BTreeMap<String, ImportBlob>,
}

/// Errors that can occur while reading a source project for import.
#[derive(Debug, thiserror::Error)]
pub enum ProjectImportError {
    #[error("cannot open source container: {0}")]
    Container(#[from] IoError),
    #[error("cannot read import blob {sha}: {source}")]
    ImportBlob {
        sha: String,
        source: ImportBlobError,
    },
    #[error("source document has no records to import")]
    EmptyDocument,
}

/// Reads a `.onecad` file and extracts the subset of its state that can be
/// imported into another document.
///
/// The returned records keep their original [`RecordId`]s; the caller must
/// verify these do not collide with the target document before appending them.
pub fn read_project_for_import(path: &Path) -> Result<ImportedProject, ProjectImportError> {
    let loaded = ContainerReader::open(path).map_err(ProjectImportError::Container)?;
    let source_doc: Document = loaded.document().clone();

    if source_doc.timeline.records().is_empty() {
        return Err(ProjectImportError::EmptyDocument);
    }

    let mut import_blobs = BTreeMap::new();
    for info in loaded.import_blobs() {
        let bytes =
            loaded
                .read_import_blob(&info.sha256)
                .map_err(|e| ProjectImportError::ImportBlob {
                    sha: info.sha256.clone(),
                    source: e,
                })?;
        import_blobs.insert(
            info.sha256.clone(),
            ImportBlob {
                codec: info.codec,
                bytes: std::sync::Arc::new(bytes),
            },
        );
    }

    Ok(ImportedProject {
        records: source_doc.timeline.records().to_vec(),
        sketches: source_doc.sketches,
        import_blobs,
    })
}

/// Checks whether importing `source` into `target` would collide on any
/// identity the import preserves. Returns the offending ids when it would.
#[must_use]
pub fn find_import_collisions(
    target: &Document,
    source: &ImportedProject,
) -> Option<ImportCollisions> {
    let mut record_ids = HashSet::new();
    let mut sketch_ids = HashSet::new();

    for rec in &source.records {
        if target.timeline.record_by_id(rec.record_id).is_some()
            || !record_ids.insert(rec.record_id)
        {
            return Some(ImportCollisions::RecordId(rec.record_id));
        }
    }
    for sketch_id in source.sketches.keys() {
        if target.sketches.contains_key(sketch_id) || !sketch_ids.insert(*sketch_id) {
            return Some(ImportCollisions::SketchId(*sketch_id));
        }
    }
    None
}

/// The first detected collision that blocks a project import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportCollisions {
    RecordId(RecordId),
    SketchId(SketchId),
}
