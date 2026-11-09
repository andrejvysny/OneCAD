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
    /// Source sketches keyed by id. The caller must refuse target collisions;
    /// certified identities are never remapped.
    pub sketches: BTreeMap<SketchId, Sketch>,
    /// Import blobs referenced by the source timeline's `ImportStep` records.
    /// `(sha256, blob)` — content-addressed, deduplicated by the carrier on insert.
    pub import_blobs: BTreeMap<String, ImportBlob>,
    /// Definition-local persistent topology authority; preserved without remapping.
    pub elements: crate::document::element_index::ElementIndex,
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
    #[error("assembly project import is not supported; open it without flattening")]
    AssemblyUnsupported,
    #[error("project import cannot yet carry embedded component packages; open the source document directly")]
    EmbeddedLibraryUnsupported,
    #[error("invalid source topology authority: {0}")]
    InvalidIdentity(String),
}

/// Reads a `.onecad` file and extracts the subset of its state that can be
/// imported into another document.
///
/// Records and element evidence retain their source identities. The caller must
/// refuse target namespace collisions before publishing the import transaction.
pub fn read_project_for_import(path: &Path) -> Result<ImportedProject, ProjectImportError> {
    let loaded = ContainerReader::open(path).map_err(ProjectImportError::Container)?;
    let source_doc: Document = loaded.document().clone();

    if !source_doc.definitions.is_empty()
        || !source_doc.assembly.occurrences.is_empty()
        || !source_doc.assembly.joints.is_empty()
    {
        return Err(ProjectImportError::AssemblyUnsupported);
    }

    if source_doc
        .modules
        .keys()
        .any(|id| id.as_str() == "onecad.library")
        || !loaded.library_archives().is_empty()
    {
        return Err(ProjectImportError::EmbeddedLibraryUnsupported);
    }

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
                bytes: std::sync::Arc::new(crate::io::imports::OwnedSource::untracked(bytes)),
            },
        );
    }

    crate::edit::project_import::ProjectImportEdit {
        records: source_doc.timeline.records().to_vec(),
        sketches: source_doc.sketches.clone(),
        elements: source_doc.elements.clone(),
    }
    .validate()
    .map_err(|error| ProjectImportError::InvalidIdentity(error.to_string()))?;
    for (_, entry) in source_doc.elements.iter() {
        if let Some(cert) = &entry.import_source_identity {
            if source_doc
                .bodies
                .get(entry.body)
                .is_none_or(|body| body.created_by != cert.producer_record_id)
                || import_blobs.get(&cert.source_sha256).is_none_or(|blob| {
                    !matches!(
                        (&cert.codec, blob.codec),
                        (
                            crate::document::import_source_identity::ImportIdentityCodec::Step,
                            crate::document::record::ImportSourceCodec::Step
                        ) | (
                            crate::document::import_source_identity::ImportIdentityCodec::Xbf,
                            crate::document::record::ImportSourceCodec::Xbf
                        )
                    )
                })
            {
                return Err(ProjectImportError::InvalidIdentity(
                    "certificate body producer or source blob mismatch".into(),
                ));
            }
        }
    }

    Ok(ImportedProject {
        records: source_doc.timeline.records().to_vec(),
        sketches: source_doc.sketches,
        import_blobs,
        elements: source_doc.elements,
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
            || target
                .definitions
                .values()
                .any(|definition| definition.timeline.record_by_id(rec.record_id).is_some())
            || !record_ids.insert(rec.record_id)
        {
            return Some(ImportCollisions::RecordId(rec.record_id));
        }
    }
    for sketch_id in source.sketches.keys() {
        if target.sketches.contains_key(sketch_id)
            || target
                .definitions
                .values()
                .any(|definition| definition.sketches.contains_key(sketch_id))
            || !sketch_ids.insert(*sketch_id)
        {
            return Some(ImportCollisions::SketchId(*sketch_id));
        }
    }
    for (id, entry) in source.elements.iter() {
        if target.elements.contains(id)
            || target
                .definitions
                .values()
                .any(|definition| definition.elements.contains(id))
        {
            return Some(ImportCollisions::ElementId(id.clone()));
        }
        if target.bodies.get(entry.body).is_some()
            || target
                .elements
                .iter()
                .any(|(_, existing)| existing.body == entry.body)
            || target.definitions.values().any(|definition| {
                definition.bodies.get(entry.body).is_some()
                    || definition
                        .elements
                        .iter()
                        .any(|(_, existing)| existing.body == entry.body)
            })
        {
            return Some(ImportCollisions::BodyId(entry.body));
        }
    }
    None
}

/// The first detected collision that blocks a project import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportCollisions {
    RecordId(RecordId),
    SketchId(SketchId),
    ElementId(crate::ids::ElementId),
    BodyId(crate::ids::BodyId),
}
