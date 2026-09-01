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
pub mod template;

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

    /// The BLOB-kind authoring leg (spec §7): writes an `embedded`- or
    /// `profile`-kind package from geometry bytes the caller already has.
    ///
    /// [`save_component`](Self::save_component) is the `document` leg — it
    /// freezes an authoring `.onecad` and bakes a solid from it. This one has no
    /// authoring document at all: the bytes ARE the component. Two callers want
    /// that. An `embedded` package is a vendor solid dropped in as-is; a
    /// `profile` package is WP-C's canonical planar face, placed at any length.
    ///
    /// Same write ORDER and the same refusals as `save_component` — blob first,
    /// then the package directory, then `component.toml` with the revision
    /// recomputed over what is on disk, then reindex; an existing `id@version`
    /// is refused rather than overwritten, because instances out in documents
    /// recorded the old revision.
    ///
    /// # Errors
    /// [`LibraryError::InvalidIdentity`] for a malformed id/version, a plain
    /// I/O error for a taken `id@version` or a filesystem failure.
    pub fn save_embedded_component(
        &mut self,
        req: EmbeddedComponentRequest,
    ) -> LibraryResult<SavedComponent> {
        let EmbeddedComponentRequest {
            mut package,
            kind,
            geometry,
            geometry_codec,
            geometry_format,
            preview_png,
        } = req;
        package::validate_identity(&package)?;

        let id = package.identity.id.clone();
        let version = package.identity.version.clone();
        if self.index.get(&id, Some(&version)).is_some() {
            return Err(LibraryError::Io(format!(
                "{id}@{version} already exists — re-saving a component is a version bump \
                 (existing instances recorded the old revision)"
            )));
        }
        if matches!(kind, BlobComponentKind::Profile) && geometry_codec != package::PROFILE_CODEC {
            return Err(LibraryError::Io(format!(
                "a profile component's geometry must be `{}` (got `{geometry_codec}`) — the \
                 blob is a single planar face, and only the brep reader returns one",
                package::PROFILE_CODEC
            )));
        }

        let sha256 = self.blobs.put(&geometry)?;
        // VERSION-QUALIFIED directory, for the same reason `save_component`
        // uses one: two versions of one component are two index entries and must
        // be two directories.
        let dir = self.root.join(format!("{id}@{version}"));
        std::fs::create_dir_all(&dir)?;
        if let Some(png) = &preview_png {
            std::fs::write(dir.join("preview.png"), png)?;
        }
        package.source = match kind {
            BlobComponentKind::Embedded => package::SourceSpec::Embedded {
                blob: sha256.clone(),
                codec: geometry_codec,
                format: geometry_format,
            },
            BlobComponentKind::Profile => package::SourceSpec::Profile {
                blob: sha256.clone(),
                codec: geometry_codec,
                format: geometry_format,
            },
        };

        let revision = package::write_package(&dir, &package)?;
        self.reindex()?;
        Ok(SavedComponent { revision, sha256 })
    }

    /// "Save as Template" (spec §8) — writes a frozen document under
    /// `templates/`. See [`template`] for why templates are listed by reading
    /// the directory rather than through `library.json`.
    ///
    /// # Errors
    /// A malformed id/name, a taken id, or a filesystem failure.
    pub fn save_template(
        &mut self,
        new: template::NewTemplate,
    ) -> LibraryResult<template::TemplateEntry> {
        template::save(&self.root, new)
    }

    /// Every template under this root, sorted by name.
    #[must_use]
    pub fn templates(&self) -> Vec<template::TemplateEntry> {
        template::list(&self.root)
    }

    /// One template by id, or `None`.
    #[must_use]
    pub fn template(&self, id: &str) -> Option<template::TemplateEntry> {
        template::get(&self.root, id)
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

/// Which blob-kind [`Library::save_embedded_component`] writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobComponentKind {
    /// A vendor solid stored as-is.
    Embedded,
    /// WP-C's canonical planar face, prismed to the instance's `length`.
    Profile,
}

/// Input to [`Library::save_embedded_component`].
///
/// `package.source` is IGNORED and overwritten with the source this save
/// actually produces — the caller describes identity, metadata, parameters and
/// attachments; the storage layer decides how geometry is referenced, because it
/// is the half that knows where the blob landed.
#[derive(Debug, Clone)]
pub struct EmbeddedComponentRequest {
    pub package: ComponentPackage,
    pub kind: BlobComponentKind,
    /// The geometry bytes. A solid for [`BlobComponentKind::Embedded`], a single
    /// canonical planar face for [`BlobComponentKind::Profile`].
    pub geometry: Vec<u8>,
    /// `step` | `brep` | `xbf` — must be `brep` for a profile.
    pub geometry_codec: String,
    /// Binary format pin for `brep`/`xbf`; absent for `step`.
    pub geometry_format: Option<u32>,
    /// Optional thumbnail (PNG bytes from the viewport snapshot).
    pub preview_png: Option<Vec<u8>>,
}

/// What [`Library::save_embedded_component`] recorded.
///
/// Carries the blob digest as well as the revision because the WP-C ingest lane
/// needs it: the worker content-addressed the face it wrote, and the caller
/// records that same digest as the placed `profile` source's `sha256` rather
/// than re-hashing the bytes it just handed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedComponent {
    /// The recorded `"sha256:…"` package revision.
    pub revision: String,
    /// Bare 64-hex digest of the geometry blob in the store.
    pub sha256: String,
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

    fn blob_request(
        id: &str,
        kind: BlobComponentKind,
        geometry: &[u8],
        codec: &str,
    ) -> EmbeddedComponentRequest {
        EmbeddedComponentRequest {
            package: authored_package(id, "1.0.0"),
            kind,
            geometry: geometry.to_vec(),
            geometry_codec: codec.to_string(),
            geometry_format: Some(4),
            preview_png: Some(b"\x89PNG\r\n".to_vec()),
        }
    }

    /// The `embedded` leg existed as a KIND from WP-3.2 but had no authoring
    /// path at all — nothing in the workspace ever constructed
    /// `SourceSpec::Embedded`, so a vendor solid could be read back but never
    /// written. This is that leg, end to end.
    #[test]
    fn save_embedded_component_writes_a_package_that_resolves_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        let saved = lib
            .save_embedded_component(blob_request(
                "acme.spacer",
                BlobComponentKind::Embedded,
                b"vendor solid bytes",
                "brep",
            ))
            .unwrap();

        let (_v, entry) = lib.get("acme.spacer", None).expect("indexed");
        assert_eq!(entry.revision, saved.revision);
        assert_eq!(entry.source_kind, "embedded");

        let resolved = lib
            .resolve_source(&ResolveRequest {
                component_id: "acme.spacer",
                component_version: "1.0.0",
                component_revision: &saved.revision,
            })
            .unwrap();
        match resolved {
            ResolvedSource::Embedded { blob } => {
                assert_eq!(blob.bytes, b"vendor solid bytes");
                assert_eq!(blob.codec, "brep");
                assert_eq!(blob.format, Some(4));
                assert_eq!(
                    blob.sha256, saved.sha256,
                    "the reported digest is the one the store keyed the bytes by"
                );
            }
            other => panic!("expected an embedded source, got {other:?}"),
        }
        // No `source.onecad`: the bytes ARE the component, there is nothing to
        // replay.
        assert!(!dir.path().join("acme.spacer@1.0.0/source.onecad").exists());
        assert!(dir.path().join("acme.spacer@1.0.0/preview.png").is_file());
    }

    /// WP-C: the same leg writing a `profile` package, whose blob is a single
    /// canonical planar FACE the worker prisms to the placing instance's length.
    #[test]
    fn save_embedded_component_writes_a_profile_package_that_resolves_back() {
        use crate::package::{ParameterRole, ParameterSpec};

        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        let mut req = blob_request(
            "acme.extrusion.2020",
            BlobComponentKind::Profile,
            b"canonical face bytes",
            "brep",
        );
        // The package's declared default length — the stick the vendor ships.
        req.package.parameters.insert(
            "length".to_string(),
            ParameterSpec {
                role: ParameterRole::Free,
                key: Some("length".to_string()),
                value: Some(toml::Value::Float(500.0)),
                domain: None,
                snap: None,
                min: Some(1.0),
                from: None,
            },
        );
        let saved = lib.save_embedded_component(req).unwrap();

        let (_v, entry) = lib.get("acme.extrusion.2020", None).expect("indexed");
        assert_eq!(entry.source_kind, "profile");
        assert_eq!(entry.parameter_keys, vec!["length".to_string()]);

        let resolved = lib
            .resolve_source(&ResolveRequest {
                component_id: "acme.extrusion.2020",
                component_version: "1.0.0",
                component_revision: &saved.revision,
            })
            .unwrap();
        match resolved {
            ResolvedSource::Profile { blob } => {
                assert_eq!(blob.bytes, b"canonical face bytes");
                assert_eq!(blob.codec, "brep");
                assert_eq!(blob.format, Some(4));
                assert_eq!(blob.sha256, saved.sha256);
            }
            other => panic!("expected a profile source, got {other:?}"),
        }
    }

    /// A profile blob is a FACE, and only the brep reader returns one. A `step`
    /// or `xbf` profile must die at authoring, not at the first regen on
    /// someone else's machine.
    #[test]
    fn a_profile_component_refuses_a_non_brep_codec() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        for codec in ["step", "xbf"] {
            let err = lib
                .save_embedded_component(blob_request(
                    "acme.extrusion.2020",
                    BlobComponentKind::Profile,
                    b"not a face",
                    codec,
                ))
                .expect_err("a non-brep profile codec must be refused");
            assert!(format!("{err:?}").contains("brep"), "{err:?}");
        }
        assert!(!dir.path().join("acme.extrusion.2020@1.0.0").exists());
    }

    #[test]
    fn saving_the_same_blob_component_id_and_version_twice_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        lib.save_embedded_component(blob_request(
            "acme.spacer",
            BlobComponentKind::Embedded,
            b"v1",
            "brep",
        ))
        .unwrap();
        let err = lib
            .save_embedded_component(blob_request(
                "acme.spacer",
                BlobComponentKind::Embedded,
                b"v2",
                "brep",
            ))
            .expect_err("same id@version");
        assert!(format!("{err:?}").contains("already exists"), "{err:?}");
    }

    /// The SHA-256 of an empty input — what EVERY embedded/profile package's
    /// revision collapsed to before `compute_revision` folded the referenced
    /// blob digest in, because their geometry lives in `blobs/`, outside the
    /// package directory the file walk hashes.
    const EMPTY_INPUT_SHA256: &str =
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    /// The defect this crate shipped with: two different components, saved
    /// with otherwise-identical manifests, must not come out revision-identical
    /// just because their geometry lives in the blob store rather than the
    /// package directory.
    #[test]
    fn embedded_components_with_different_geometry_get_different_revisions() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        let saved_a = lib
            .save_embedded_component(blob_request(
                "acme.spacer.a",
                BlobComponentKind::Embedded,
                b"vendor solid A",
                "brep",
            ))
            .unwrap();
        let saved_b = lib
            .save_embedded_component(blob_request(
                "acme.spacer.b",
                BlobComponentKind::Embedded,
                b"vendor solid B, different geometry entirely",
                "brep",
            ))
            .unwrap();
        assert_ne!(
            saved_a.revision, saved_b.revision,
            "different geometry must not share a revision"
        );
        assert_ne!(
            saved_a.revision, EMPTY_INPUT_SHA256,
            "no longer the empty-input tell"
        );
        assert_ne!(
            saved_b.revision, EMPTY_INPUT_SHA256,
            "no longer the empty-input tell"
        );
    }

    /// Proves the check `resolve()` performs is meaningful again: the OLD
    /// bug's revision (every embedded package used to share it) no longer
    /// matches a freshly written package, so a stale caller carrying it is
    /// refused loudly rather than silently bound to the wrong geometry.
    #[test]
    fn resolve_refuses_the_pre_fix_empty_input_revision_against_a_new_package() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        lib.save_embedded_component(blob_request(
            "acme.spacer",
            BlobComponentKind::Embedded,
            b"vendor solid bytes",
            "brep",
        ))
        .unwrap();

        let err = lib
            .resolve_source(&ResolveRequest {
                component_id: "acme.spacer",
                component_version: "1.0.0",
                component_revision: EMPTY_INPUT_SHA256,
            })
            .expect_err("a stale revision must never resolve");
        assert!(
            matches!(err, LibraryError::RevisionMismatch { .. }),
            "{err:?}"
        );
    }
}
