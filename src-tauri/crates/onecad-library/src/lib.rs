//! `onecad-library` — the Component Library's storage layer (spec §4). Pure
//! domain crate, no Tauri, no network I/O (see `Cargo.toml`'s invariant
//! comment). Package read/write, index management, content-addressed blob
//! store, and resolution live here; the app crate (`src-tauri/src/library.rs`,
//! WP-1.3) is the sole consumer and the only place this crate's types meet
//! `onecad-core`'s.

pub mod blob;
pub mod error;
pub mod index;
pub mod package;
pub mod registry;
pub mod resolve;
pub mod tables;

use std::path::{Path, PathBuf};

pub use error::{LibraryError, LibraryResult};
pub use package::ComponentPackage;

use blob::BlobStore;
use index::{IndexEntry, LibraryIndex, ReindexReport};
use registry::{LocalRegistry, RegistrySource};
use resolve::{ResolveRequest, ResolvedSource};

/// A local component library rooted at a directory (spec §4 suggested
/// surface). Owns the index, the blob store, and resolution; callers never
/// touch `library.json` or `blobs/` directly.
pub struct Library {
    root: PathBuf,
    registry: LocalRegistry,
    blobs: BlobStore,
    index: LibraryIndex,
}

impl Library {
    /// Opens (or lazily creates) a library rooted at `root`. Creating the
    /// directory itself is deferred to the first write (`reindex`/blob
    /// `put`/`save_component`) — `open` on a not-yet-existing root is a
    /// legitimate "fresh library" state, not an error.
    pub fn open(root: impl Into<PathBuf>) -> LibraryResult<Self> {
        let root = root.into();
        let registry = LocalRegistry::new(root.clone());
        let index = registry.index().unwrap_or_default();
        Ok(Self {
            blobs: BlobStore::new(&root),
            root,
            registry,
            index,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn index(&self) -> &LibraryIndex {
        &self.index
    }

    #[must_use]
    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    /// Rebuilds the index from packages on disk and persists it. Feeds
    /// WP-1.6's tasks-chip producer: the returned [`ReindexReport`] carries
    /// `total`/`indexed` counts a caller can turn into `begin`/`setProgress`.
    pub fn reindex(&mut self) -> LibraryResult<ReindexReport> {
        let (index, report) = self.registry.reindex()?;
        self.index = index;
        Ok(report)
    }

    /// The entry for `id`, at `version` when given, else the newest indexed
    /// version.
    #[must_use]
    pub fn get(&self, id: &str, version: Option<&str>) -> Option<(&str, &IndexEntry)> {
        self.index.get(id, version)
    }

    /// Resolves a placed instance's recorded identity to its
    /// worker-consumable source. Revision mismatch or absence is a typed
    /// [`LibraryError`] (never a load-breaking `Err`) — see [`resolve::resolve`].
    pub fn resolve_source(&self, req: &ResolveRequest<'_>) -> LibraryResult<ResolvedSource> {
        self.registry.resolve(req)
    }

    /// "Save as Component" (spec §7 authoring flow). NOT implemented in this
    /// build — authoring depends on document/geometry access this crate
    /// deliberately does not have (capturing a body/selection, attachment
    /// placement, a viewport thumbnail), which is P3 scope wired through the
    /// app crate. The signature is settled now so WP-1.1's callers (none
    /// yet) and P3's implementer share one surface.
    ///
    /// # Errors
    /// Always — this is a stub.
    pub fn save_component(&mut self, _new: NewComponent) -> LibraryResult<String> {
        Err(LibraryError::Io(
            "Library::save_component is not implemented (P3 — Component Library authoring)".into(),
        ))
    }

    /// "Save as Template" (spec §8). Same P3 deferral as [`Self::save_component`].
    ///
    /// # Errors
    /// Always — this is a stub.
    pub fn save_template(&mut self, _new: NewTemplate) -> LibraryResult<String> {
        Err(LibraryError::Io(
            "Library::save_template is not implemented (P3 — Component Library authoring)".into(),
        ))
    }
}

/// Input to [`Library::save_component`] (P3 scope — declared now for the
/// surface, not yet constructible by any P1 caller).
#[derive(Debug, Clone)]
pub struct NewComponent {
    pub package: ComponentPackage,
    /// The frozen document blob (`document` source kind) or generator
    /// resolution the package's `[source]` describes.
    pub payload: Vec<u8>,
}

/// Input to [`Library::save_template`] (P3 scope).
#[derive(Debug, Clone)]
pub struct NewTemplate {
    pub name: String,
    pub document_bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::COMPONENT_MANIFEST_FILE;

    fn seed(root: &Path) {
        let dir = root.join("onecad.std.iso4762");
        std::fs::create_dir_all(&dir).unwrap();
        let toml = format!(
            r#"
[identity]
id = "onecad.std.iso4762"
version = "1.0.0"
revision = "sha256:{}"

[metadata]
name = "SHCS"

[source]
kind = "generator"
generator = "iso4762"
generator_version = 1
"#,
            "0".repeat(64)
        );
        std::fs::write(dir.join(COMPONENT_MANIFEST_FILE), toml).unwrap();
    }

    #[test]
    fn open_on_a_fresh_nonexistent_root_is_an_empty_library() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("not-yet-created");
        let lib = Library::open(&root).unwrap();
        assert!(lib.index().components.is_empty());
    }

    #[test]
    fn open_reindex_get_resolve_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path());
        let mut lib = Library::open(dir.path()).unwrap();
        assert!(
            lib.index().components.is_empty(),
            "not indexed until reindex()"
        );

        let report = lib.reindex().unwrap();
        assert_eq!(report.indexed, 1);
        assert!(lib.get("onecad.std.iso4762", None).is_some());

        let resolved = lib
            .resolve_source(&ResolveRequest {
                component_id: "onecad.std.iso4762",
                component_version: "1.0.0",
                component_revision: &format!("sha256:{}", "0".repeat(64)),
            })
            .unwrap();
        assert_eq!(
            resolved,
            ResolvedSource::Generator {
                generator_id: "iso4762".to_string(),
                generator_version: 1,
            }
        );
    }

    #[test]
    fn reopen_reads_the_previously_persisted_index_without_reindexing() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path());
        let mut lib1 = Library::open(dir.path()).unwrap();
        lib1.reindex().unwrap();
        drop(lib1);

        let lib2 = Library::open(dir.path()).unwrap();
        assert_eq!(
            lib2.index().components.len(),
            1,
            "index.json persisted across open()"
        );
    }
}
