//! The library's content-addressed blob store — `<library-root>/blobs/<sha256>`
//! (spec §2.3). Generalizes `onecad-core::io::imports`'s design principles
//! (content-address filename, per-file cap, typed `HashMismatch` that
//! degrades gracefully) as a genuinely SEPARATE implementation: this store is
//! a plain filesystem directory OUTSIDE any document, shared across
//! documents and surviving document close — a different lifecycle from the
//! zip-container-embedded `imports/` section, so it cannot literally be the
//! same Rust type.

use std::path::PathBuf;

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
        if bytes.len() as u64 > MAX_BLOB_BYTES {
            return Err(LibraryError::BlobTooLarge {
                limit: MAX_BLOB_BYTES,
                actual: bytes.len() as u64,
            });
        }
        let sha256 = sha256_hex(bytes);
        let path = self.path_for(&sha256)?;
        if path.exists() {
            // Already present — verify rather than trust (a prior write may
            // have been interrupted). A mismatch here means a hash COLLISION
            // or filesystem corruption, not an ordinary code path; refuse
            // loudly rather than silently overwrite.
            let existing = std::fs::read(&path)?;
            if existing != bytes {
                return Err(LibraryError::BlobHashMismatch {
                    sha256: sha256.clone(),
                    expected: sha256,
                    actual: sha256_hex(&existing),
                });
            }
            return Ok(sha256);
        }
        std::fs::create_dir_all(&self.root)?;
        std::fs::write(&path, bytes)?;
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
