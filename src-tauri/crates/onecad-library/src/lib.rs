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

    /// "Save as Component" (spec §7 authoring) — writes a `document`-kind
    /// package from an already-baked solid.
    ///
    /// The caller (the app crate) owns everything this crate deliberately
    /// cannot see: which body was baked, how it was baked (the worker's
    /// `ExportGeometry` lane), the parameter roles the user declared, and the
    /// thumbnail. What lands here is the finished material, and this method
    /// owns the ORDER it has to be written in:
    ///
    /// 1. the geometry blob goes into the content-addressed store, which is
    ///    what makes the same solid authored twice cost one copy;
    /// 2. `source.onecad` and the preview go into the package directory;
    /// 3. `component.toml` is written, the revision recomputed over what is
    ///    now on disk, and the manifest rewritten with it
    ///    ([`package::write_package`]);
    /// 4. the index is rebuilt so the panel can see it without a manual
    ///    reindex.
    ///
    /// **An existing id@version is refused, never overwritten.** Re-saving is
    /// a semver bump (spec §7): instances out in documents recorded the old
    /// revision, and silently changing the package under them is the
    /// substitution spec §0 invariant 4 forbids. The caller decides the new
    /// version; this only enforces that it is not already taken.
    ///
    /// Returns the recorded revision.
    ///
    /// # Errors
    /// [`LibraryError::InvalidIdentity`] for a malformed id/version, a plain
    /// I/O error for a taken id@version or a filesystem failure.
    pub fn save_component(&mut self, new: NewComponent) -> LibraryResult<String> {
        let NewComponent {
            mut package,
            document,
            geometry,
            geometry_codec,
            geometry_format,
            preview_png,
        } = new;
        package::validate_identity(&package)?;

        let id = package.identity.id.clone();
        let version = package.identity.version.clone();
        if self.index.get(&id, Some(&version)).is_some() {
            return Err(LibraryError::Io(format!(
                "{id}@{version} already exists — re-saving a component is a version bump \
                 (existing instances recorded the old revision)"
            )));
        }

        let sha256 = self.blobs.put(&geometry)?;
        // VERSION-QUALIFIED directory. The index maps `id -> version -> path`,
        // so two versions of one component are two entries — and therefore two
        // directories. Writing to a bare `<id>` (the shape the built-in seed
        // packages use, where only one version ever ships) would have the 1.1.0
        // save silently overwrite 1.0.0's manifest, and every instance still
        // recording 1.0.0 would resolve `NeedsRepair` with no file left to
        // explain why. Caught by `saving_the_same_id_and_version_twice_is_refused`.
        let dir = self.root.join(format!("{id}@{version}"));
        std::fs::create_dir_all(&dir)?;
        const DOCUMENT_FILE: &str = "source.onecad";
        std::fs::write(dir.join(DOCUMENT_FILE), &document)?;
        if let Some(png) = &preview_png {
            std::fs::write(dir.join("preview.png"), png)?;
        }
        package.source = package::SourceSpec::Document {
            file: DOCUMENT_FILE.to_string(),
            geometry: sha256,
            geometry_codec,
            geometry_format,
        };

        let revision = package::write_package(&dir, &package)?;
        self.reindex()?;
        Ok(revision)
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

/// Input to [`Library::save_component`].
///
/// `package.source` is IGNORED and overwritten with the `document` source this
/// save actually produces — the caller describes identity, metadata,
/// parameters and attachments; the storage layer decides how geometry is
/// referenced, because it is the half that knows where the blob landed.
#[derive(Debug, Clone)]
pub struct NewComponent {
    pub package: ComponentPackage,
    /// The frozen authoring document (`source.onecad`) — provenance, and the
    /// input a future re-bake replays. Never read on the placement path.
    pub document: Vec<u8>,
    /// The solid baked from that document at authoring time. THIS is what a
    /// placement resolves to, which is what lets a document open with the
    /// library deleted (spec §4).
    pub geometry: Vec<u8>,
    /// `step` | `brep` | `xbf` — the byte form of `geometry`, echoed by the
    /// worker's own bake rather than assumed here.
    pub geometry_codec: String,
    /// Binary format pin for `brep`/`xbf`; absent for `step`.
    pub geometry_format: Option<u32>,
    /// Optional thumbnail (PNG bytes from the viewport snapshot).
    pub preview_png: Option<Vec<u8>>,
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

    fn authored_package(id: &str, version: &str) -> ComponentPackage {
        use crate::package::{Identity, Metadata, SourceSpec};
        use std::collections::BTreeMap;
        ComponentPackage {
            identity: Identity {
                id: id.to_string(),
                version: version.to_string(),
                // Deliberately a lie — `save_component` recomputes it.
                revision: format!("sha256:{}", "f".repeat(64)),
            },
            metadata: Metadata {
                name: "Bracket".to_string(),
                unit: "mm".to_string(),
                ..Default::default()
            },
            // Also ignored: the storage layer decides how geometry is referenced.
            source: SourceSpec::Generator {
                generator: "ignored".to_string(),
                generator_version: 1,
            },
            parameters: BTreeMap::new(),
            attachments: BTreeMap::new(),
        }
    }

    fn new_component(id: &str, version: &str, geometry: &[u8]) -> NewComponent {
        NewComponent {
            package: authored_package(id, version),
            document: b"frozen onecad container".to_vec(),
            geometry: geometry.to_vec(),
            geometry_codec: "xbf".to_string(),
            geometry_format: Some(4),
            preview_png: Some(b"\x89PNG\r\n".to_vec()),
        }
    }

    #[test]
    fn save_component_writes_a_document_package_that_resolves_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        let revision = lib
            .save_component(new_component("acme.bracket", "1.0.0", b"baked solid bytes"))
            .unwrap();

        // Indexed without a manual reindex — the panel reads the index.
        let (_v, entry) = lib.get("acme.bracket", None).expect("indexed");
        assert_eq!(entry.revision, revision);
        assert_eq!(entry.source_kind, "document");

        // And it resolves end to end: the recorded revision matches, the
        // blob comes back byte-identical, and the codec/format pin survives.
        let resolved = lib
            .resolve_source(&ResolveRequest {
                component_id: "acme.bracket",
                component_version: "1.0.0",
                component_revision: &revision,
            })
            .unwrap();
        match resolved {
            ResolvedSource::Document { blob, .. } => {
                assert_eq!(blob.bytes, b"baked solid bytes");
                assert_eq!(blob.codec, "xbf");
                assert_eq!(blob.format, Some(4));
            }
            other => panic!("expected a document source, got {other:?}"),
        }

        // The package directory is version-qualified so a later version does
        // not overwrite this one (see `save_component`).
        assert!(dir
            .path()
            .join("acme.bracket@1.0.0/source.onecad")
            .is_file());
        assert!(dir.path().join("acme.bracket@1.0.0/preview.png").is_file());
    }

    #[test]
    fn saving_the_same_id_and_version_twice_is_refused() {
        // Re-saving is a semver bump: instances already out in documents
        // recorded the old revision, and rewriting the package under them is
        // exactly the silent substitution the whole design forbids.
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        lib.save_component(new_component("acme.bracket", "1.0.0", b"v1"))
            .unwrap();
        let err = lib
            .save_component(new_component("acme.bracket", "1.0.0", b"v2"))
            .expect_err("same id@version");
        assert!(format!("{err:?}").contains("already exists"), "{err:?}");

        // A bumped version is fine, and both stay resolvable.
        lib.save_component(new_component("acme.bracket", "1.1.0", b"v2"))
            .unwrap();
        assert!(lib.get("acme.bracket", Some("1.0.0")).is_some());
        assert!(lib.get("acme.bracket", Some("1.1.0")).is_some());
        assert_eq!(lib.get("acme.bracket", None).map(|(v, _)| v), Some("1.1.0"));
    }

    #[test]
    fn two_components_with_identical_geometry_share_one_blob() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        lib.save_component(new_component("acme.one", "1.0.0", b"same solid"))
            .unwrap();
        lib.save_component(new_component("acme.two", "1.0.0", b"same solid"))
            .unwrap();
        let blobs = std::fs::read_dir(dir.path().join("blobs")).unwrap().count();
        assert_eq!(blobs, 1, "content-addressed storage deduplicates");
    }

    #[test]
    fn save_component_refuses_an_unnamespaced_id_before_writing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        assert!(lib
            .save_component(new_component("unnamespaced", "1.0.0", b"x"))
            .is_err());
        assert!(!dir.path().join("unnamespaced@1.0.0").exists());
    }
}
