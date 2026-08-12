//! `library.json` — the index (spec §2.2): maps `id → version → package path`
//! plus cached search metadata, atomically written. `indexVersion` is kept so
//! a future remote-registry schema bump is detectable.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{LibraryError, LibraryResult};
use crate::package::{self, ComponentPackage, COMPONENT_MANIFEST_FILE};

pub const INDEX_FILE: &str = "library.json";
pub const INDEX_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    /// Package directory, relative to the library root.
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub category: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source_kind: String,
    #[serde(default)]
    pub parameter_keys: Vec<String>,
    pub revision: String,
    /// `source.generator`/`source.generator_version` — `source_kind ==
    /// "generator"` only (WP-1.5: the placement gesture's ghost-preview draft
    /// needs these to build a valid throwaway `PlaceComponent` op without a
    /// second package read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_version: Option<u32>,
    /// `[attachments]` table verbatim (WP-1.5: the snap solver matches a
    /// hovered target's classification against each attachment's `accepts`).
    #[serde(default)]
    pub attachments: BTreeMap<String, crate::package::AttachmentSpec>,
    /// `[parameters]` table verbatim (WP-2.4: the configurator's role/domain/
    /// snap source — `parameter_keys` above is names only, not enough to
    /// render an editable control or enforce role=free).
    #[serde(default)]
    pub parameters: BTreeMap<String, crate::package::ParameterSpec>,
    /// `metadata.designation` (spec §2.1's BOM string template, e.g. `"ISO
    /// 4762 M{thread}x{length}"`) — the configurator's live-preview source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub designation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryIndex {
    pub index_version: u32,
    /// `id -> version -> entry`.
    #[serde(default)]
    pub components: BTreeMap<String, BTreeMap<String, IndexEntry>>,
}

impl Default for LibraryIndex {
    fn default() -> Self {
        Self {
            index_version: INDEX_VERSION,
            components: BTreeMap::new(),
        }
    }
}

impl LibraryIndex {
    /// Loads `library.json` from `path`. A MISSING file is an empty index
    /// (a fresh library root, not an error) — mirrors the "self-contained,
    /// degrade gracefully" doctrine the rest of this crate follows.
    pub fn load(path: &Path) -> LibraryResult<Self> {
        match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| LibraryError::MalformedIndex {
                path: path.to_path_buf(),
                message: e.to_string(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(LibraryError::Io(e.to_string())),
        }
    }

    /// Writes `library.json` ATOMICALLY: a sibling temp file, then a rename
    /// over the target (mirrors `onecad-core::io::container`'s save
    /// discipline) — a reader never observes a partially-written index.
    pub fn save(&self, path: &Path) -> LibraryResult<()> {
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| LibraryError::Io(format!("serialize library.json: {e}")))?;
        let tmp = sibling_temp_path(path);
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            LibraryError::Io(format!("rename temp -> {}: {e}", path.display()))
        })?;
        Ok(())
    }

    /// Rebuilds the index from the packages actually on disk under
    /// `library_root` — one immediate subdirectory per package, each
    /// containing a `component.toml`. A directory that fails to parse is
    /// SKIPPED (recorded in the returned report), not fatal to the whole
    /// reindex — one bad package must not hide every good one.
    ///
    /// Returns `(index, report)`; the caller decides whether to `save()` the
    /// result. `report`'s counts are what WP-1.6 feeds to the tasks-chip
    /// producer (`begin(id, "Indexing library", 0)` / `setProgress` /
    /// `end`).
    pub fn reindex(library_root: &Path) -> LibraryResult<(Self, ReindexReport)> {
        let mut index = Self::default();
        let mut report = ReindexReport::default();

        let entries = match std::fs::read_dir(library_root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((index, report));
            }
            Err(e) => return Err(LibraryError::Io(e.to_string())),
        };

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join(COMPONENT_MANIFEST_FILE);
            if !manifest_path.is_file() {
                continue; // not a package directory (e.g. `blobs/`)
            }
            report.total += 1;
            match load_package_entry(library_root, &path) {
                Ok((id, version, entry)) => {
                    index
                        .components
                        .entry(id)
                        .or_default()
                        .insert(version, entry);
                    report.indexed += 1;
                }
                Err(e) => {
                    report.skipped.push((path, e.to_string()));
                }
            }
        }
        Ok((index, report))
    }

    /// The entry for `id`, at `version` when given, else the newest version
    /// (lexicographic on the version string — a real semver-aware ordering is
    /// P2 tables work, not this WP's scope).
    #[must_use]
    pub fn get(&self, id: &str, version: Option<&str>) -> Option<(&str, &IndexEntry)> {
        let versions = self.components.get(id)?;
        match version {
            Some(v) => versions.get_key_value(v).map(|(k, e)| (k.as_str(), e)),
            None => versions.iter().next_back().map(|(v, e)| (v.as_str(), e)),
        }
    }
}

#[derive(Debug, Default)]
pub struct ReindexReport {
    pub total: usize,
    pub indexed: usize,
    pub skipped: Vec<(PathBuf, String)>,
}

fn load_package_entry(
    library_root: &Path,
    package_dir: &Path,
) -> LibraryResult<(String, String, IndexEntry)> {
    let manifest_path = package_dir.join(COMPONENT_MANIFEST_FILE);
    let raw = std::fs::read_to_string(&manifest_path)?;
    let pkg: ComponentPackage = package::parse(&raw, &manifest_path)?;
    package::validate_identity(&pkg)?;

    let rel_path = package_dir
        .strip_prefix(library_root)
        .unwrap_or(package_dir)
        .to_string_lossy()
        .to_string();
    let (source_kind, generator_id, generator_version) = match &pkg.source {
        crate::package::SourceSpec::Embedded { .. } => ("embedded", None, None),
        crate::package::SourceSpec::Generator {
            generator,
            generator_version,
        } => (
            "generator",
            Some(generator.clone()),
            Some(*generator_version),
        ),
        crate::package::SourceSpec::Document { .. } => ("document", None, None),
    };
    let entry = IndexEntry {
        path: rel_path,
        name: pkg.metadata.name.clone(),
        category: pkg.metadata.category.clone(),
        tags: pkg.metadata.tags.clone(),
        source_kind: source_kind.to_string(),
        parameter_keys: pkg.parameters.keys().cloned().collect(),
        revision: pkg.identity.revision.clone(),
        generator_id,
        generator_version,
        attachments: pkg.attachments.clone(),
        parameters: pkg.parameters.clone(),
        designation: pkg.metadata.designation.clone(),
    };
    Ok((pkg.identity.id.clone(), pkg.identity.version.clone(), entry))
}

fn sibling_temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "library.json".to_string());
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    path.with_file_name(format!(".{name}.tmp-{pid}-{nanos}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_package(root: &Path, dir_name: &str, id: &str, version: &str, name: &str) {
        let pkg_dir = root.join(dir_name);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        let toml = format!(
            r#"
[identity]
id = "{id}"
version = "{version}"
revision = "sha256:{}"

[metadata]
name = "{name}"

[source]
kind = "generator"
generator = "g"
generator_version = 1
"#,
            "0".repeat(64)
        );
        std::fs::write(pkg_dir.join(COMPONENT_MANIFEST_FILE), toml).unwrap();
    }

    #[test]
    fn load_of_a_missing_file_is_an_empty_index_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let index = LibraryIndex::load(&dir.path().join(INDEX_FILE)).unwrap();
        assert_eq!(index.index_version, INDEX_VERSION);
        assert!(index.components.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(INDEX_FILE);
        let mut index = LibraryIndex::default();
        index
            .components
            .entry("onecad.std.iso4762".into())
            .or_default()
            .insert(
                "1.0.0".into(),
                IndexEntry {
                    path: "onecad.std.iso4762".into(),
                    name: "Socket Head Cap Screw".into(),
                    category: vec!["fasteners".into()],
                    tags: vec![],
                    source_kind: "generator".into(),
                    parameter_keys: vec!["thread".into()],
                    revision: format!("sha256:{}", "0".repeat(64)),
                    generator_id: Some("iso4762".into()),
                    generator_version: Some(1),
                    attachments: BTreeMap::new(),
                    parameters: BTreeMap::new(),
                    designation: None,
                },
            );
        index.save(&path).unwrap();
        let loaded = LibraryIndex::load(&path).unwrap();
        assert_eq!(loaded, index);
    }

    #[test]
    fn reindex_reconciles_disk_and_skips_bad_packages_without_failing_the_whole_pass() {
        let dir = tempfile::tempdir().unwrap();
        write_package(
            dir.path(),
            "onecad.std.iso4762",
            "onecad.std.iso4762",
            "1.0.0",
            "SHCS",
        );
        write_package(
            dir.path(),
            "onecad.std.bearing.608",
            "onecad.std.bearing.608",
            "1.0.0",
            "608 bearing",
        );
        // A malformed package directory (bad TOML) must not abort the reindex.
        let bad_dir = dir.path().join("broken.pkg");
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(bad_dir.join(COMPONENT_MANIFEST_FILE), "not valid toml [[[").unwrap();
        // A non-package directory (e.g. the blob store) must be ignored.
        std::fs::create_dir_all(dir.path().join("blobs")).unwrap();

        let (index, report) = LibraryIndex::reindex(dir.path()).unwrap();
        assert_eq!(
            report.total, 3,
            "3 candidate package dirs (blobs/ excluded)"
        );
        assert_eq!(report.indexed, 2);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(index.components.len(), 2);
        assert!(index.get("onecad.std.iso4762", None).is_some());
        assert!(index.get("onecad.std.bearing.608", Some("1.0.0")).is_some());
        assert!(index.get("onecad.std.bearing.608", Some("9.9.9")).is_none());
    }

    #[test]
    fn reindex_of_a_nonexistent_root_is_an_empty_index() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let (index, report) = LibraryIndex::reindex(&missing).unwrap();
        assert!(index.components.is_empty());
        assert_eq!(report.total, 0);
    }
}
