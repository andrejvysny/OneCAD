//! `RegistrySource` — the trait boundary between "where a component's bytes
//! live" and everything else in this crate. `LocalRegistry` (filesystem only)
//! is the only implementation in this build; a future `RemoteRegistry` (P4)
//! is the SOLE place network I/O may enter `onecad-library` (see the crate's
//! `Cargo.toml` invariant comment). The local format IS the registry
//! manifest shape (spec §2.2) — widening this trait later is additive, not a
//! format change.

use std::path::{Path, PathBuf};

use crate::error::LibraryResult;
use crate::index::{LibraryIndex, ReindexReport, INDEX_FILE};
use crate::resolve::{self, ResolveRequest, ResolvedSource};

pub trait RegistrySource {
    /// Resolves a recorded component identity to its worker-consumable source.
    fn resolve(&self, req: &ResolveRequest<'_>) -> LibraryResult<ResolvedSource>;

    /// Rebuilds the index from packages on disk and persists it.
    fn reindex(&self) -> LibraryResult<(LibraryIndex, ReindexReport)>;

    /// Loads the CURRENT persisted index without rebuilding it.
    fn index(&self) -> LibraryResult<LibraryIndex>;
}

/// A registry backed entirely by a local directory tree (spec §2 — the local
/// format IS the future registry manifest).
pub struct LocalRegistry {
    root: PathBuf,
}

impl LocalRegistry {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_FILE)
    }
}

impl RegistrySource for LocalRegistry {
    fn resolve(&self, req: &ResolveRequest<'_>) -> LibraryResult<ResolvedSource> {
        let index = LibraryIndex::load(&self.index_path())?;
        resolve::resolve(&self.root, &index, req)
    }

    fn reindex(&self) -> LibraryResult<(LibraryIndex, ReindexReport)> {
        let (index, report) = LibraryIndex::reindex(&self.root)?;
        index.save(&self.index_path())?;
        Ok((index, report))
    }

    fn index(&self) -> LibraryResult<LibraryIndex> {
        LibraryIndex::load(&self.index_path())
    }
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
    fn reindex_persists_then_resolve_reads_it_back() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path());
        let registry = LocalRegistry::new(dir.path());

        let (index, report) = registry.reindex().unwrap();
        assert_eq!(report.indexed, 1);
        assert_eq!(index.components.len(), 1);

        // index() re-reads the PERSISTED file, not an in-memory cache.
        let reloaded = registry.index().unwrap();
        assert_eq!(reloaded, index);

        let resolved = registry
            .resolve(&ResolveRequest {
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
}
