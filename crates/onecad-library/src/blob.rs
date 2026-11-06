//! The library's content-addressed blob store — `<library-root>/blobs/<sha256>`
//! (spec §2.3). Generalizes `onecad-core::io::imports`'s design principles
//! (content-address filename, per-file cap, typed `HashMismatch` that
//! degrades gracefully) as a genuinely SEPARATE implementation: this store is
//! a plain filesystem directory OUTSIDE any document, shared across
//! documents and surviving document close — a different lifecycle from the
//! zip-container-embedded `imports/` section, so it cannot literally be the
//! same Rust type.

use std::{io::Write, path::PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{LibraryError, LibraryResult};

pub const BLOBS_DIR: &str = "blobs";

/// A blob larger than this is refused at write time. Mirrors
/// `onecad-core::io::imports::MAX_IMPORT_BLOB_BYTES`'s order of magnitude —
/// library components are small parts, not large assemblies.
pub const MAX_BLOB_BYTES: u64 = 256 * 1024 * 1024;

/// A blob store rooted at `<library-root>/blobs/`.
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    #[must_use]
    pub fn new(library_root: impl Into<PathBuf>) -> Self {
        Self {
            root: library_root.into().join(BLOBS_DIR),
        }
    }

    fn path_for(&self, sha256: &str) -> LibraryResult<PathBuf> {
        if !is_sha256_hex(sha256) {
            return Err(LibraryError::BadBlobKey(sha256.to_string()));
        }
        Ok(self.root.join(sha256))
    }

    /// Writes `bytes` under their own content address, creating the `blobs/`
    /// directory if needed. Returns the sha256 hex key. Idempotent: writing
    /// the same bytes twice is a no-op on the second call (verified by
    /// re-reading, not blindly overwritten, so a partial prior write is
    /// caught rather than silently trusted).
    ///
    /// # Errors
    /// Too large, or an I/O failure.
    pub fn put(&self, bytes: &[u8]) -> LibraryResult<String> {
        self.put_with_writer(bytes, |file, bytes| file.write_all(bytes))
    }

    fn put_with_writer(
        &self,
        bytes: &[u8],
        write: impl FnOnce(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
    ) -> LibraryResult<String> {
        if bytes.len() as u64 > MAX_BLOB_BYTES {
            return Err(LibraryError::BlobTooLarge {
                limit: MAX_BLOB_BYTES,
                actual: bytes.len() as u64,
            });
        }
        let sha256 = sha256_hex(bytes);
        let path = self.path_for(&sha256)?;
        match std::fs::read(&path) {
            Ok(existing) => {
                verify_existing(&sha256, bytes, &existing)?;
                std::fs::File::open(&path)?.sync_all()?;
                create_durable_directory(&self.root)?;
                std::fs::File::open(&self.root)?.sync_all()?;
                return Ok(sha256);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        create_durable_directory(&self.root)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)?;
        write(temporary.as_file_mut(), bytes)?;
        temporary.as_file().sync_all()?;
        // A hard link publishes complete bytes without replacing another writer's
        // winner. Only this writer owns cleanup of its temporary.
        match std::fs::hard_link(temporary.path(), &path) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                verify_existing(&sha256, bytes, &std::fs::read(&path)?)?;
            }
            Err(error) => return Err(error.into()),
        }
        temporary.close()?;
        std::fs::File::open(&self.root)?.sync_all()?;
        Ok(sha256)
    }

    /// Reads a blob by its content address, RE-VERIFYING the hash on every
    /// read (mirrors `onecad-core::io::imports::verify_blob`) — a bad blob is
    /// a per-record failure the caller degrades gracefully from, never a
    /// silently substituted geometry (spec §0 invariant #4).
    ///
    /// # Errors
    /// `BlobMissing` when absent, `BlobHashMismatch` when the bytes on disk
    /// disagree with `sha256`.
    pub fn get(&self, sha256: &str) -> LibraryResult<Vec<u8>> {
        let path = self.path_for(sha256)?;
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                LibraryError::BlobMissing {
                    sha256: sha256.to_string(),
                }
            } else {
                LibraryError::Io(e.to_string())
            }
        })?;
        let actual = sha256_hex(&bytes);
        if actual != sha256 {
            return Err(LibraryError::BlobHashMismatch {
                sha256: sha256.to_string(),
                expected: sha256.to_string(),
                actual,
            });
        }
        Ok(bytes)
    }

    /// True iff a blob with this key is present on disk. Does NOT verify its
    /// hash (a fast existence check for reindexing).
    #[must_use]
    pub fn contains(&self, sha256: &str) -> bool {
        self.path_for(sha256).is_ok_and(|p| p.is_file())
    }
}

fn verify_existing(key: &str, expected: &[u8], existing: &[u8]) -> LibraryResult<()> {
    if existing != expected {
        return Err(LibraryError::BlobHashMismatch {
            sha256: key.to_string(),
            expected: key.to_string(),
            actual: sha256_hex(existing),
        });
    }
    Ok(())
}

fn create_durable_directory(path: &std::path::Path) -> std::io::Result<()> {
    if path.is_dir() {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::File::open(parent)?.sync_all()?;
        }
        return Ok(());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        create_durable_directory(parent)?;
    }
    match std::fs::create_dir(path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => (),
        Err(error) => return Err(error),
    }
    if let Some(parent) = parent {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, BlobStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn put_then_get_round_trips() {
        let (_dir, store) = store();
        let sha = store.put(b"hello component").unwrap();
        assert_eq!(store.get(&sha).unwrap(), b"hello component");
        assert!(store.contains(&sha));
    }

    #[test]
    fn put_is_idempotent_for_identical_bytes() {
        let (_dir, store) = store();
        let sha1 = store.put(b"same bytes").unwrap();
        let sha2 = store.put(b"same bytes").unwrap();
        assert_eq!(sha1, sha2);
        assert_eq!(
            store
                .put_with_writer(b"same bytes", |_, _| {
                    panic!("an existing valid blob must not need a new writer")
                })
                .unwrap(),
            sha1
        );
    }

    #[test]
    fn partial_failed_writer_never_publishes_and_removes_only_its_temporary() {
        let (dir, store) = store();
        std::fs::create_dir_all(&store.root).unwrap();
        let foreign = store.root.join("foreign-temporary");
        std::fs::write(&foreign, b"another writer").unwrap();
        let bytes = b"complete payload";
        let key = sha256_hex(bytes);
        let error = store.put_with_writer(bytes, |file, bytes| {
            file.write_all(&bytes[..3])?;
            assert!(!store.contains(&key));
            Err(std::io::Error::other("injected disk-full write"))
        });
        assert!(error.is_err());
        assert!(!store.contains(&key));
        assert_eq!(std::fs::read(&foreign).unwrap(), b"another writer");
        assert_eq!(
            std::fs::read_dir(dir.path().join(BLOBS_DIR))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(store.put(bytes).unwrap(), key);
        assert_eq!(store.get(&key).unwrap(), bytes);
    }

    #[test]
    fn concurrent_complete_writers_publish_one_verified_blob() {
        let (dir, store) = store();
        let barrier = std::sync::Barrier::new(2);
        let bytes = b"concurrent immutable payload";
        std::thread::scope(|scope| {
            let write = || {
                store.put_with_writer(bytes, |file, bytes| {
                    file.write_all(bytes)?;
                    barrier.wait();
                    Ok(())
                })
            };
            let first = scope.spawn(write);
            let second = scope.spawn(write);
            assert_eq!(
                first.join().unwrap().unwrap(),
                second.join().unwrap().unwrap()
            );
        });
        assert_eq!(store.get(&sha256_hex(bytes)).unwrap(), bytes);
        assert_eq!(
            std::fs::read_dir(dir.path().join(BLOBS_DIR))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn put_refuses_existing_corrupt_content_without_replacing_it() {
        let (_dir, store) = store();
        let bytes = b"valid content";
        let key = store.put(bytes).unwrap();
        std::fs::write(store.path_for(&key).unwrap(), b"corrupt").unwrap();
        assert!(matches!(
            store.put(bytes),
            Err(LibraryError::BlobHashMismatch { .. })
        ));
        assert_eq!(
            std::fs::read(store.path_for(&key).unwrap()).unwrap(),
            b"corrupt"
        );
        assert_eq!(std::fs::read_dir(&store.root).unwrap().count(), 1);
    }

    #[test]
    fn missing_blob_is_a_typed_error() {
        let (_dir, store) = store();
        let err = store.get(&"a".repeat(64)).unwrap_err();
        assert!(matches!(err, LibraryError::BlobMissing { .. }));
    }

    #[test]
    fn bad_key_shape_is_rejected_before_touching_disk() {
        let (_dir, store) = store();
        let err = store.get("not-a-sha256").unwrap_err();
        assert!(matches!(err, LibraryError::BadBlobKey(_)));
    }

    /// The integrity regression pin: a flipped byte on disk must be caught on
    /// read, never silently served as the requested content (mirrors
    /// `onecad-core::io::imports::verify_blob_catches_a_flipped_byte`).
    #[test]
    fn a_flipped_byte_on_disk_is_caught_on_read() {
        let (dir, store) = store();
        let sha = store.put(b"trustworthy bytes").unwrap();
        let path = dir.path().join(BLOBS_DIR).join(&sha);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[0] ^= 0xFF; // flip one byte
        std::fs::write(&path, &bytes).unwrap();

        let err = store.get(&sha).unwrap_err();
        assert!(matches!(err, LibraryError::BlobHashMismatch { .. }));
    }

    #[test]
    fn oversized_blob_is_refused() {
        let (_dir, store) = store();
        let oversized = vec![0u8; (MAX_BLOB_BYTES + 1) as usize];
        let err = store.put(&oversized).unwrap_err();
        assert!(matches!(err, LibraryError::BlobTooLarge { .. }));
    }
}
