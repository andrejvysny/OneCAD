//! Opaque, content-addressed component package archives carried by a document.
//!
//! Core validates bytes and limits only; the library layer validates package semantics.
use super::{sha256_hex, IoError, IoResult};
use std::{collections::BTreeMap, sync::Arc};

/// Each archive is bounded by the existing per-import blob ceiling.
pub const MAX_LIBRARY_ARCHIVE_BYTES: u64 = super::imports::MAX_IMPORT_BLOB_BYTES;
/// Fixed-layout archive storage. Its admission stays alive while any carrier holds it.
pub struct ArchiveBytes {
    storage: Box<[u8]>,
    used: usize,
    _retention: Arc<dyn Send + Sync>,
}
impl ArchiveBytes {
    /// Allocate the exact admitted layout before reading or encoding archive bytes.
    #[must_use]
    pub fn zeroed(size: usize, retention: Arc<dyn Send + Sync>) -> Self {
        // SAFETY: new_zeroed_slice initializes every byte, and every bit pattern is valid for u8.
        let storage = unsafe { Box::<[u8]>::new_zeroed_slice(size).assume_init() };
        Self {
            storage,
            used: 0,
            _retention: retention,
        }
    }
    /// Builds headless/test storage from existing bytes without host accounting.
    #[must_use]
    pub fn untracked(bytes: Vec<u8>) -> Self {
        let storage = bytes.into_boxed_slice();
        let used = storage.len();
        Self {
            storage,
            used,
            _retention: Arc::new(()),
        }
    }
    pub fn capacity(&self) -> usize {
        self.storage.len()
    }
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        &mut self.storage
    }
    pub fn set_used(&mut self, used: usize) -> IoResult<()> {
        if used > self.storage.len() {
            return Err(IoError::TooLarge("archive exceeds admitted storage".into()));
        }
        self.used = used;
        Ok(())
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.storage[..self.used]
    }
    pub fn len(&self) -> usize {
        self.used
    }
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }
}
impl std::ops::Deref for ArchiveBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}
impl std::fmt::Debug for ArchiveBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveBytes")
            .field("len", &self.used)
            .field("capacity", &self.storage.len())
            .finish()
    }
}

/// Caller-supplied referenced archives, keyed by bare lowercase SHA-256.
pub type LibraryArchives = BTreeMap<String, Arc<ArchiveBytes>>;

/// An archive name and declared size, indexed without decompression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryArchiveInfo {
    pub sha256: String,
    pub declared_size: u64,
    pub(crate) path: String,
}

pub(crate) fn path(sha: &str) -> String {
    format!("library/{sha}.zip")
}

pub(crate) fn parse_path(path: &str) -> Option<String> {
    let sha = path.strip_prefix("library/")?.strip_suffix(".zip")?;
    crate::document::record::is_sha256_hex(sha).then(|| sha.to_string())
}

pub(crate) fn validate(sha: &str, bytes: &[u8]) -> IoResult<()> {
    if !crate::document::record::is_sha256_hex(sha) {
        return Err(IoError::Corrupt(format!(
            "invalid library archive key {sha:?}"
        )));
    }
    if bytes.len() as u64 > MAX_LIBRARY_ARCHIVE_BYTES {
        return Err(IoError::TooLarge(format!(
            "library archive {sha} exceeds {MAX_LIBRARY_ARCHIVE_BYTES} bytes"
        )));
    }
    if sha256_hex(bytes) != sha {
        return Err(IoError::Corrupt(format!(
            "library archive {sha} hash mismatch"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::ids::DocumentId;
    use crate::io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta};
    use crate::io::imports::ImportBlobs;
    use uuid::Uuid;

    #[test]
    fn deterministic_carrier_round_trip_and_explicit_gc() {
        let dir = tempfile::tempdir().unwrap();
        let one = dir.path().join("one.onecad");
        let two = dir.path().join("two.onecad");
        let empty = dir.path().join("empty.onecad");
        let baseline = dir.path().join("baseline.onecad");
        let doc = Document::new(DocumentId(Uuid::from_u128(17)));
        let caches = ContainerCaches::none();
        let meta = SaveMeta::default();
        let imports = ImportBlobs::new();
        let payload = b"opaque library archive".to_vec();
        let sha = sha256_hex(&payload);
        let archives = LibraryArchives::from([(
            sha.clone(),
            Arc::new(ArchiveBytes::untracked(payload.clone())),
        )]);
        for target in [&one, &two] {
            ContainerWriter::save_with_imports_and_library(
                target, &doc, &caches, &imports, &archives, &meta,
            )
            .unwrap();
        }
        assert_eq!(std::fs::read(&one).unwrap(), std::fs::read(&two).unwrap());
        let loaded = ContainerReader::open(&one).unwrap();
        assert_eq!(loaded.library_archives().len(), 1);
        assert_eq!(loaded.read_library_archive(&sha).unwrap(), payload);
        assert!(matches!(
            loaded.read_library_archive_bounded(&sha, 3),
            Err(IoError::TooLarge(_))
        ));
        assert!(loaded.read_library_archive(&"0".repeat(64)).is_err());
        ContainerWriter::save_with_imports_and_library(
            &empty,
            &doc,
            &caches,
            &imports,
            &LibraryArchives::new(),
            &meta,
        )
        .unwrap();
        ContainerWriter::save_with_imports(&baseline, &doc, &caches, &imports, &meta).unwrap();
        assert_eq!(
            std::fs::read(&empty).unwrap(),
            std::fs::read(&baseline).unwrap()
        );
        assert!(ContainerReader::open(&empty)
            .unwrap()
            .library_archives()
            .is_empty());
    }

    #[test]
    fn bad_digest_refuses_without_overwriting_existing_container() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("saved.onecad");
        let doc = Document::new(DocumentId(Uuid::from_u128(18)));
        let caches = ContainerCaches::none();
        let meta = SaveMeta::default();
        ContainerWriter::save(&target, &doc, &caches, &meta).unwrap();
        let before = std::fs::read(&target).unwrap();
        let archives = LibraryArchives::from([(
            "0".repeat(64),
            Arc::new(ArchiveBytes::untracked(b"mismatch".to_vec())),
        )]);
        assert!(ContainerWriter::save_with_imports_and_library(
            &target,
            &doc,
            &caches,
            &ImportBlobs::new(),
            &archives,
            &meta
        )
        .is_err());
        assert_eq!(std::fs::read(&target).unwrap(), before);
    }
    #[test]
    fn modified_archive_payload_is_not_returned() {
        use std::io::{Read, Write};
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original.onecad");
        let modified = dir.path().join("modified.onecad");
        let doc = Document::new(DocumentId(Uuid::from_u128(19)));
        let bytes = b"original archive".to_vec();
        let sha = sha256_hex(&bytes);
        let archives =
            LibraryArchives::from([(sha.clone(), Arc::new(ArchiveBytes::untracked(bytes)))]);
        ContainerWriter::save_with_imports_and_library(
            &original,
            &doc,
            &ContainerCaches::none(),
            &ImportBlobs::new(),
            &archives,
            &SaveMeta::default(),
        )
        .unwrap();
        let mut input = zip::ZipArchive::new(std::fs::File::open(original).unwrap()).unwrap();
        let mut output = zip::ZipWriter::new(std::fs::File::create(&modified).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for index in 0..input.len() {
            let mut entry = input.by_index(index).unwrap();
            let name = entry.name().to_owned();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            if name == path(&sha) {
                data[0] ^= 1;
            }
            output.start_file(name, options).unwrap();
            output.write_all(&data).unwrap();
        }
        output.finish().unwrap();
        let loaded = ContainerReader::open(&modified).unwrap();
        assert!(matches!(
            loaded.read_library_archive(&sha),
            Err(IoError::Corrupt(_))
        ));
    }
    #[test]
    fn unknown_module_reference_survives_explicit_carry_forward() {
        use crate::document::modules::{ModuleId, ModuleState};
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.onecad");
        let second = dir.path().join("second.onecad");
        let mut doc = Document::new(DocumentId(Uuid::from_u128(20)));
        let bytes = b"opaque module archive".to_vec();
        let sha = sha256_hex(&bytes);
        let id = ModuleId::parse("com.example.future").unwrap();
        let state = ModuleState::new(7, serde_json::json!({ "archiveSha256": sha }));
        doc.modules.insert(id.clone(), state.clone());
        let archives = LibraryArchives::from([(
            sha.clone(),
            Arc::new(ArchiveBytes::untracked(bytes.clone())),
        )]);
        ContainerWriter::save_with_imports_and_library(
            &first,
            &doc,
            &ContainerCaches::none(),
            &ImportBlobs::new(),
            &archives,
            &SaveMeta::default(),
        )
        .unwrap();
        let loaded = ContainerReader::open(&first).unwrap();
        assert_eq!(loaded.document().modules.get(&id), Some(&state));
        let carried = LibraryArchives::from([(
            sha.clone(),
            Arc::new(ArchiveBytes::untracked(
                loaded.read_library_archive(&sha).unwrap(),
            )),
        )]);
        ContainerWriter::save_with_imports_and_library(
            &second,
            loaded.document(),
            &ContainerCaches::none(),
            &ImportBlobs::new(),
            &carried,
            &SaveMeta::default(),
        )
        .unwrap();
        let reopened = ContainerReader::open(&second).unwrap();
        assert_eq!(reopened.document().modules.get(&id), Some(&state));
        assert_eq!(reopened.read_library_archive(&sha).unwrap(), bytes);
    }
}
