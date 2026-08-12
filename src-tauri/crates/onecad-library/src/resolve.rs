//! Resolution: given `{id, version, revision, source}` (the identity a placed
//! instance recorded at authoring — spec §4), produce the worker-consumable
//! geometry source. Verifies the package's CURRENT revision still matches the
//! recorded hash; a mismatch or absence is a typed [`LibraryError`], never a
//! panic or a load-breaking `Err` — the caller (the app-crate authoring
//! layer, WP-1.3) is the one that converts it into `NeedsRepair` (spec §4:
//! "never an `Err` that breaks document load").

use std::path::Path;

use crate::error::{LibraryError, LibraryResult};
use crate::index::LibraryIndex;
use crate::package::{self, ComponentPackage, SourceSpec, COMPONENT_MANIFEST_FILE};

pub struct ResolveRequest<'a> {
    pub component_id: &'a str,
    pub component_version: &'a str,
    /// The `sha256:…` revision the placing instance recorded at authoring.
    pub component_revision: &'a str,
}

/// The worker-consumable geometry source a resolution produced. P0/WP-1.1
/// scope: `Generator` only (mirrors `onecad-core`'s `ComponentSourceRef`
/// reduction — `Embedded`/`Document` land WP-1.2/P3).
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedSource {
    Generator {
        generator_id: String,
        generator_version: u32,
    },
}

/// Resolves a recorded component identity against the library ON DISK.
///
/// # Errors
/// [`LibraryError::NotFound`] when the id/version isn't in the index;
/// [`LibraryError::RevisionMismatch`] when the package's current content hash
/// disagrees with the recorded one (the package changed underneath the
/// instance); a package-parse error if the manifest itself is malformed.
pub fn resolve(
    library_root: &Path,
    index: &LibraryIndex,
    req: &ResolveRequest,
) -> LibraryResult<ResolvedSource> {
    let (_version, entry) = index
        .get(req.component_id, Some(req.component_version))
        .ok_or_else(|| {
            LibraryError::NotFound(format!("{}@{}", req.component_id, req.component_version))
        })?;

    if entry.revision != req.component_revision {
        return Err(LibraryError::RevisionMismatch {
            component_id: req.component_id.to_string(),
            recorded: req.component_revision.to_string(),
            current: entry.revision.clone(),
        });
    }

    let manifest_path = library_root.join(&entry.path).join(COMPONENT_MANIFEST_FILE);
    let raw = std::fs::read_to_string(&manifest_path)?;
    let pkg: ComponentPackage = package::parse(&raw, &manifest_path)?;

    match pkg.source {
        SourceSpec::Generator {
            generator,
            generator_version,
        } => Ok(ResolvedSource::Generator {
            generator_id: generator,
            generator_version,
        }),
        SourceSpec::Embedded { .. } => Err(LibraryError::MalformedPackage {
            path: manifest_path,
            message: "embedded source resolution lands in WP-1.2".into(),
        }),
        SourceSpec::Document { .. } => Err(LibraryError::MalformedPackage {
            path: manifest_path,
            message: "document source resolution lands in P3".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::COMPONENT_MANIFEST_FILE;

    fn seed_package(root: &Path, revision_zeros: char) -> LibraryIndex {
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
            revision_zeros.to_string().repeat(64)
        );
        std::fs::write(dir.join(COMPONENT_MANIFEST_FILE), toml).unwrap();
        let (index, report) = LibraryIndex::reindex(root).unwrap();
        assert_eq!(report.indexed, 1, "seed package must index cleanly");
        index
    }

    #[test]
    fn resolve_matches_generator_source() {
        let dir = tempfile::tempdir().unwrap();
        let index = seed_package(dir.path(), '0');
        let req = ResolveRequest {
            component_id: "onecad.std.iso4762",
            component_version: "1.0.0",
            component_revision: &format!("sha256:{}", "0".repeat(64)),
        };
        let resolved = resolve(dir.path(), &index, &req).unwrap();
        assert_eq!(
            resolved,
            ResolvedSource::Generator {
                generator_id: "iso4762".to_string(),
                generator_version: 1,
            }
        );
    }

    #[test]
    fn revision_mismatch_is_a_typed_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let index = seed_package(dir.path(), '0');
        let req = ResolveRequest {
            component_id: "onecad.std.iso4762",
            component_version: "1.0.0",
            // The instance recorded a DIFFERENT revision than the package has now.
            component_revision: &format!("sha256:{}", "f".repeat(64)),
        };
        let err = resolve(dir.path(), &index, &req).unwrap_err();
        assert!(matches!(err, LibraryError::RevisionMismatch { .. }));
    }

    #[test]
    fn unknown_component_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let index = seed_package(dir.path(), '0');
        let req = ResolveRequest {
            component_id: "onecad.std.does-not-exist",
            component_version: "1.0.0",
            component_revision: "sha256:whatever",
        };
        let err = resolve(dir.path(), &index, &req).unwrap_err();
        assert!(matches!(err, LibraryError::NotFound(_)));
    }
}
