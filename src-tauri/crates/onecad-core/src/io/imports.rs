//! `imports/<sha256>.<codec>` — content-addressed import source blobs.
//!
//! # A third section class
//!
//! [`container`](super::container) knows two classes: **authoritative**
//! (`document.json` — a hash mismatch is [`IoError::Corrupt`], the container does
//! not open) and **cache** (`geometry/`, `meshes/`, `checkpoints/`, `preview.png`
//! — degradable, a mismatch is `Stale` and the load is unaffected). An import
//! blob is neither. It is **authoritative-for-a-record**:
//!
//! * *Authoritative* because it is irreplaceable input. No regen can reconstruct
//!   the source STEP; dropping it would silently un-model the body forever.
//! * *…for a record* because its blast radius is exactly one timeline step. A
//!   missing or corrupt blob must therefore **NOT** fail
//!   [`ContainerReader::open`](super::container::ContainerReader::open) — the
//!   document opens, the tree renders, every other feature still regenerates, and
//!   the failure surfaces (loudly) when that one `ImportStep` is executed. A
//!   document with one unrenderable body is a repairable document; a document
//!   that will not open is not.
//!
//! Consequently blobs are read **lazily and never at open**, and are excluded
//! from [`LoadedContainer::cache_entries`](super::container::LoadedContainer::cache_entries)
//! and [`read_cache`](super::container::LoadedContainer::read_cache) — a stale
//! `opsHash` must not report an import blob `Stale`, because unlike a cache it
//! can never be regenerated.
//!
//! # Integrity
//!
//! The section name IS the digest, so the manifest hash is redundant here: the
//! reader verifies the decompressed bytes against the **requested sha** on every
//! read ([`ImportBlobError::HashMismatch`]), which catches bit-rot and tampering
//! without trusting the manifest at all. The writer verifies the same equation so
//! a mis-keyed blob is rejected at save rather than becoming an unreadable entry.
//!
//! # Caps
//!
//! Each blob is capped at [`MAX_IMPORT_BLOB_BYTES`] (256 MiB) on **both** write
//! and read — a zip bomb declaring a small entry that inflates past the cap hits
//! [`IoError::TooLarge`] mid-read, not an OOM. Blobs also count against the
//! whole-container [`MAX_TOTAL_BYTES`](super::container::MAX_TOTAL_BYTES) /
//! [`MAX_ENTRIES`](super::container::MAX_ENTRIES) budgets, which are enforced
//! over the raw ZIP directory before anything here runs.
//!
//! # Refcounting at save
//!
//! Blobs are written by *reference*, not by presence in the carrier: see
//! [`referenced_import_shas`].

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::document::record::{is_sha256_hex, ImportSourceCodec, KnownOperation, Operation};
use crate::document::Document;

use super::{IoError, IoResult};

/// Import-blob directory prefix (`imports/<sha256>.<codec>`).
pub const IMPORTS_DIR: &str = "imports/";

/// Per-blob hard cap, enforced on write and on read. Deliberately far below the
/// generic 1 GB cache-blob cap: an import source is a human-authored CAD file,
/// and 256 MiB is already an order of magnitude past any realistic one.
pub const MAX_IMPORT_BLOB_BYTES: u64 = 256 * 1024 * 1024;

/// One import source blob: its codec plus the raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportBlob {
    /// Which codec the bytes are in (picks the section extension).
    pub codec: ImportSourceCodec,
    /// The source bytes, opaque to the core.
    pub bytes: Vec<u8>,
}

/// The document-level import-blob carrier: `sha256 → blob`, keyed by the SAME
/// lowercase-hex digest an
/// [`ImportStepParams::source_sha256`](crate::document::record::ImportStepParams::source_sha256)
/// carries.
///
/// Deliberately NOT a field of [`Document`]: blobs are container payload, not
/// document state. Keeping them out is what makes `document.json` byte-identical
/// for a document that has no imports — and unchanged in *shape* for one that
/// does.
pub type ImportBlobs = BTreeMap<String, ImportBlob>;

/// Metadata for one import blob present in a container — **no bytes read**.
/// Collected from the ZIP central directory while the archive is already being
/// walked for the cap guards, so listing is free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportBlobInfo {
    /// The content hash (= the entry basename, = the key to read it back).
    pub sha256: String,
    /// The codec, decoded from the entry extension.
    pub codec: ImportSourceCodec,
    /// The DECLARED uncompressed size from the ZIP directory. Advisory only —
    /// a hostile archive may lie, which is why the read path caps independently.
    pub declared_size: u64,
    /// The full archive entry path.
    pub path: String,
}

/// Failures specific to import blobs.
///
/// Deliberately **not** folded into [`IoError`]: every variant here is a
/// one-record failure that leaves the container open and usable, whereas an
/// `IoError::Corrupt` means the container itself is unusable. Collapsing the two
/// would make a single bad import blob look like a dead file.
#[derive(Debug, Error)]
pub enum ImportBlobError {
    /// No `imports/<sha256>.*` entry in this container.
    #[error("import blob {0} is not present in this container")]
    Missing(String),

    /// The decompressed bytes do not hash to the requested digest (bit-rot or
    /// tampering). Content-addressing makes this self-detecting.
    #[error("import blob {requested} failed content verification (bytes hash to {actual})")]
    HashMismatch {
        /// The digest that was asked for (and that names the entry).
        requested: String,
        /// The digest the bytes actually have.
        actual: String,
    },

    /// The requested key is not a 64-character lowercase-hex sha256.
    #[error("import blob key `{0}` is not a 64-character lowercase-hex sha256")]
    BadKey(String),

    /// Transport / size-cap failure reading the entry (e.g.
    /// [`IoError::TooLarge`] when the entry inflates past
    /// [`MAX_IMPORT_BLOB_BYTES`]).
    #[error(transparent)]
    Io(#[from] IoError),
}

/// Convenience result alias for import-blob access.
pub type ImportBlobResult<T> = Result<T, ImportBlobError>;

/// The archive path for an import blob.
#[must_use]
pub fn import_blob_path(sha256: &str, codec: ImportSourceCodec) -> String {
    format!("{IMPORTS_DIR}{sha256}.{}", codec.extension())
}

/// Splits an `imports/<sha256>.<ext>` entry name into its `(sha256, codec)`.
///
/// `None` for anything else — a foreign, misnamed, or nested entry under
/// `imports/` is IGNORED rather than guessed at or treated as an error, so a
/// container written by a future build (new codec) still opens here.
#[must_use]
pub fn parse_import_blob_path(path: &str) -> Option<(String, ImportSourceCodec)> {
    let rest = path.strip_prefix(IMPORTS_DIR)?;
    if rest.contains('/') {
        return None;
    }
    let (sha, ext) = rest.rsplit_once('.')?;
    if !is_sha256_hex(sha) {
        return None;
    }
    ImportSourceCodec::from_extension(ext).map(|codec| (sha.to_string(), codec))
}

/// Every import-blob digest **pinned by a live timeline record**.
///
/// Refcounting is keyed to the record list, not to the carrier: a blob is written
/// iff some record in `document.timeline.records()` is an `ImportStep` naming it.
/// Three consequences, all intentional:
///
/// * **Orphans are dropped at save.** Re-importing over an existing feature (or
///   undoing an import and saving) leaves a blob nothing references; carrying it
///   forever would grow every subsequent save monotonically.
/// * **A SUPPRESSED record still pins its blob.** Suppression is a display/regen
///   toggle the user expects to be reversible; dropping the source would turn a
///   reversible toggle into destructive data loss on the next save.
/// * **A rolled-back record (past the cursor) still pins its blob**, for the same
///   reason — it is still in the timeline, so it is still live.
#[must_use]
pub fn referenced_import_shas(document: &Document) -> BTreeSet<String> {
    document
        .timeline
        .records()
        .iter()
        .flat_map(|rec| match &rec.op {
            Operation::Known(KnownOperation::ImportStep(p)) => {
                // The replayed blob AND (brep-primary policy) the co-stored
                // original both stay pinned — dropping the provenance would
                // orphan the user's actual file at the first save.
                let mut shas = vec![p.source_sha256.clone()];
                shas.extend(p.provenance_sha256.clone());
                shas
            }
            _ => Vec::new(),
        })
        .collect()
}

/// Selects and validates the import sections to write: the referenced blobs
/// only, in deterministic (digest) order, as `(entry path, bytes)` pairs.
///
/// A referenced digest with no blob in `blobs` is SKIPPED rather than rejected —
/// the caller cannot write bytes it does not have, and refusing the save would
/// make an already-degraded document unsaveable. The step fails loudly at regen.
///
/// # Errors
/// * [`IoError::Corrupt`] — a malformed key, or bytes that do not hash to it.
/// * [`IoError::TooLarge`] — a blob over [`MAX_IMPORT_BLOB_BYTES`].
pub(crate) fn import_sections<'a>(
    document: &Document,
    blobs: &'a ImportBlobs,
) -> IoResult<Vec<(String, &'a [u8])>> {
    let mut out = Vec::new();
    for sha in referenced_import_shas(document) {
        let Some(blob) = blobs.get(&sha) else {
            continue;
        };
        if !is_sha256_hex(&sha) {
            return Err(IoError::Corrupt(format!(
                "import blob key `{sha}` is not a 64-character lowercase-hex sha256"
            )));
        }
        if blob.bytes.len() as u64 > MAX_IMPORT_BLOB_BYTES {
            return Err(IoError::TooLarge(format!(
                "import blob {sha} is {} bytes, over the {MAX_IMPORT_BLOB_BYTES}-byte per-blob cap",
                blob.bytes.len()
            )));
        }
        let actual = super::sha256_hex(&blob.bytes);
        if actual != sha {
            return Err(IoError::Corrupt(format!(
                "import blob keyed {sha} contains bytes hashing to {actual}"
            )));
        }
        out.push((import_blob_path(&sha, blob.codec), blob.bytes.as_slice()));
    }
    Ok(out)
}

/// Verifies decompressed blob bytes against the digest that named them.
///
/// # Errors
/// [`ImportBlobError::HashMismatch`] when the content hash disagrees.
pub(crate) fn verify_blob(sha256: &str, bytes: Vec<u8>) -> ImportBlobResult<Vec<u8>> {
    let actual = super::sha256_hex(&bytes);
    if actual != sha256 {
        return Err(ImportBlobError::HashMismatch {
            requested: sha256.to_string(),
            actual,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::record::{ImportStepParams, OperationRecord};
    use crate::document::variables::Scalar;
    use crate::ids::{DocumentId, RecordId};
    use uuid::Uuid;

    fn sha_of(bytes: &[u8]) -> String {
        super::super::sha256_hex(bytes)
    }

    fn import_op(sha: &str, codec: ImportSourceCodec) -> Operation {
        Operation::Known(KnownOperation::ImportStep(ImportStepParams {
            source_sha256: sha.to_string(),
            source_codec: codec,
            source_name: "bracket.step".into(),
            heal_policy: "v1".into(),
            unit_scale: Scalar::new(1.0),
            brep_format: match codec {
                ImportSourceCodec::Brep => Some(1),
                ImportSourceCodec::Step => None,
            },
            provenance_sha256: None,
            extra: Default::default(),
        }))
    }

    fn doc_with_imports(ops: &[(u128, Operation, bool)]) -> Document {
        let mut d = Document::new(DocumentId(Uuid::from_u128(0xD0C)));
        for (seed, op, suppressed) in ops {
            let mut rec =
                OperationRecord::new(RecordId(Uuid::from_u128(*seed)), 0, "Import", op.clone());
            rec.suppressed = *suppressed;
            d.timeline.insert_at_cursor(rec);
        }
        d
    }

    #[test]
    fn blob_path_round_trips_through_parse() {
        let sha = sha_of(b"ISO-10303-21;");
        for codec in [ImportSourceCodec::Step, ImportSourceCodec::Brep] {
            let path = import_blob_path(&sha, codec);
            assert!(path.starts_with(IMPORTS_DIR));
            assert_eq!(parse_import_blob_path(&path), Some((sha.clone(), codec)));
        }
    }

    #[test]
    fn parse_rejects_foreign_and_hostile_entry_names() {
        let sha = sha_of(b"x");
        // Unknown extension → ignored (a future codec must not be guessed at).
        assert_eq!(
            parse_import_blob_path(&format!("{IMPORTS_DIR}{sha}.iges")),
            None
        );
        // Not a digest.
        assert_eq!(parse_import_blob_path("imports/readme.step"), None);
        // Uppercase hex is a second spelling of one digest → rejected.
        assert_eq!(
            parse_import_blob_path(&format!("{IMPORTS_DIR}{}.step", sha.to_uppercase())),
            None
        );
        // Nested paths never parse (defense in depth beyond the zip guard).
        assert_eq!(
            parse_import_blob_path(&format!("{IMPORTS_DIR}sub/{sha}.step")),
            None
        );
        // Not an imports entry at all.
        assert_eq!(
            parse_import_blob_path(&format!("geometry/{sha}.brep")),
            None
        );
    }

    #[test]
    fn referenced_shas_include_suppressed_and_rolled_back_records() {
        let a = sha_of(b"aaa");
        let b = sha_of(b"bbb");
        let mut d = doc_with_imports(&[
            (0x1, import_op(&a, ImportSourceCodec::Step), false),
            (0x2, import_op(&b, ImportSourceCodec::Brep), true), // suppressed
        ]);
        // Roll the cursor back so record 2 is past it — still live, still pinned.
        d.timeline.set_cursor(1);
        let refs = referenced_import_shas(&d);
        assert!(refs.contains(&a));
        assert!(refs.contains(&b), "a suppressed record still pins its blob");
    }

    #[test]
    fn referenced_shas_include_provenance() {
        // Brep-primary policy: the record replays the brep (source_sha256) while
        // the user's original STEP is co-stored under provenance_sha256 — BOTH
        // must be pinned or the provenance is orphan-dropped at the first save.
        let replay = sha_of(b"brep bytes");
        let provenance = sha_of(b"original step bytes");
        let mut op = import_op(&replay, ImportSourceCodec::Brep);
        if let Operation::Known(KnownOperation::ImportStep(p)) = &mut op {
            p.provenance_sha256 = Some(provenance.clone());
        }
        let d = doc_with_imports(&[(0x1, op, false)]);
        let refs = referenced_import_shas(&d);
        assert!(refs.contains(&replay));
        assert!(refs.contains(&provenance), "provenance blob stays pinned");
    }

    #[test]
    fn import_sections_drops_orphans_and_keeps_referenced() {
        let live = b"ISO-10303-21; live".to_vec();
        let orphan = b"ISO-10303-21; orphan".to_vec();
        let live_sha = sha_of(&live);
        let orphan_sha = sha_of(&orphan);

        let d = doc_with_imports(&[(0x1, import_op(&live_sha, ImportSourceCodec::Step), false)]);
        let mut blobs = ImportBlobs::new();
        blobs.insert(
            live_sha.clone(),
            ImportBlob {
                codec: ImportSourceCodec::Step,
                bytes: live.clone(),
            },
        );
        blobs.insert(
            orphan_sha,
            ImportBlob {
                codec: ImportSourceCodec::Step,
                bytes: orphan,
            },
        );

        let sections = import_sections(&d, &blobs).unwrap();
        assert_eq!(sections.len(), 1, "orphan blob dropped at save");
        assert_eq!(
            sections[0].0,
            import_blob_path(&live_sha, ImportSourceCodec::Step)
        );
        assert_eq!(sections[0].1, live.as_slice());
    }

    #[test]
    fn import_sections_skips_a_referenced_blob_the_caller_did_not_supply() {
        let sha = sha_of(b"absent");
        let d = doc_with_imports(&[(0x1, import_op(&sha, ImportSourceCodec::Step), false)]);
        assert!(import_sections(&d, &ImportBlobs::new()).unwrap().is_empty());
    }

    #[test]
    fn import_sections_rejects_a_miskeyed_blob() {
        let sha = sha_of(b"declared");
        let d = doc_with_imports(&[(0x1, import_op(&sha, ImportSourceCodec::Step), false)]);
        let mut blobs = ImportBlobs::new();
        blobs.insert(
            sha,
            ImportBlob {
                codec: ImportSourceCodec::Step,
                bytes: b"actually something else".to_vec(),
            },
        );
        assert!(matches!(
            import_sections(&d, &blobs),
            Err(IoError::Corrupt(_))
        ));
    }

    #[test]
    fn import_sections_rejects_a_malformed_key() {
        // A hostile `sourceSha256` that would inject a path separator never
        // reaches the archive: the digest-shape guard rejects it first.
        let d = doc_with_imports(&[(
            0x1,
            import_op("../../etc/passwd", ImportSourceCodec::Step),
            false,
        )]);
        let mut blobs = ImportBlobs::new();
        blobs.insert(
            "../../etc/passwd".into(),
            ImportBlob {
                codec: ImportSourceCodec::Step,
                bytes: b"pwned".to_vec(),
            },
        );
        assert!(matches!(
            import_sections(&d, &blobs),
            Err(IoError::Corrupt(_))
        ));
    }

    #[test]
    fn verify_blob_catches_a_flipped_byte() {
        let bytes = b"ISO-10303-21; HEADER;".to_vec();
        let sha = sha_of(&bytes);
        assert_eq!(verify_blob(&sha, bytes.clone()).unwrap(), bytes);

        let mut flipped = bytes;
        flipped[4] ^= 0x01;
        assert!(matches!(
            verify_blob(&sha, flipped),
            Err(ImportBlobError::HashMismatch { .. })
        ));
    }
}
