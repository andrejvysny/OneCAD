//! `component.toml` — the package manifest (spec §2.1). Parse/validate +
//! identity rules (namespaced id, semver-shaped version, revision hash
//! recomputed on write).
//!
//! Deliberately a SEPARATE type family from `onecad-core::document::record`'s
//! `ComponentSourceRef`/`ComponentParamValue`: those are wire/SCHEMA-normative
//! op-params types, while `[parameters]`'s `role`/`domain`/`snap` shape here
//! has no op-params analogue (see the crate's `Cargo.toml` invariant comment).
//! The app crate (WP-1.3) translates between the two at authoring time.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{LibraryError, LibraryResult};

pub const COMPONENT_MANIFEST_FILE: &str = "component.toml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    /// Stable, namespaced id (`<ns>.<...>`), never reused.
    pub id: String,
    /// Semver.
    pub version: String,
    /// `"sha256:<64 lowercase hex>"` content hash over the package — see
    /// [`compute_revision`].
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Metadata {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub designation: Option<String>,
    #[serde(default)]
    pub category: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_unit")]
    pub unit: String,
}

fn default_unit() -> String {
    "mm".to_string()
}

/// `[source]` — exactly one of `embedded` | `generator` | `document` (spec §2.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SourceSpec {
    Embedded {
        blob: String,
    },
    Generator {
        generator: String,
        generator_version: u32,
    },
    Document {
        file: String,
    },
}

/// A `[parameters].<key>` entry. Fields are role-dependent (spec §2.1); kept as
/// a flat optional struct rather than a per-role enum since TOML's own shape
/// already mixes them loosely and a package author is expected to supply only
/// the fields their role uses — `tables.rs`/the app-crate authoring layer
/// enforces which combination is actually valid for a given `role`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterSpec {
    pub role: ParameterRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<Vec<toml::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// `role = "table"` only: the dimension-table column this value derives from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterRole {
    Free,
    Table,
    Computed,
}

/// A `[attachments].<key>` entry — a named mate point (spec §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentSpec {
    /// Local geometry the attachment names (`"face:head_underside"`,
    /// `"cylinder:shank"`).
    pub on: String,
    /// Geometry kinds this attachment mates with.
    pub accepts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentPackage {
    pub identity: Identity,
    #[serde(default)]
    pub metadata: Metadata,
    pub source: SourceSpec,
    #[serde(default)]
    pub parameters: BTreeMap<String, ParameterSpec>,
    #[serde(default)]
    pub attachments: BTreeMap<String, AttachmentSpec>,
}

/// Parses `component.toml` bytes into a [`ComponentPackage`]. `path` is used
/// only for the error message.
pub fn parse(toml_str: &str, path: &Path) -> LibraryResult<ComponentPackage> {
    toml::from_str(toml_str).map_err(|e| LibraryError::MalformedPackage {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// Validates the package's identity invariants (spec §2.1): namespaced id,
/// non-empty version, well-formed `sha256:` revision.
pub fn validate_identity(pkg: &ComponentPackage) -> LibraryResult<()> {
    if pkg.identity.id.trim().is_empty() {
        return Err(LibraryError::InvalidIdentity(
            "identity.id must not be empty".into(),
        ));
    }
    if !pkg.identity.id.contains('.') {
        return Err(LibraryError::InvalidIdentity(format!(
            "identity.id `{}` must be namespaced (`<ns>.<...>`)",
            pkg.identity.id
        )));
    }
    if pkg.identity.version.trim().is_empty() {
        return Err(LibraryError::InvalidIdentity(
            "identity.version must not be empty".into(),
        ));
    }
    match pkg.identity.revision.strip_prefix("sha256:") {
        Some(hex) if is_sha256_hex(hex) => {}
        _ => {
            return Err(LibraryError::InvalidIdentity(format!(
                "identity.revision `{}` must be `sha256:` + 64 lowercase-hex chars",
                pkg.identity.revision
            )))
        }
    }
    Ok(())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Recomputes a package directory's content hash (spec §2.1 `identity.revision`):
/// SHA-256 over every file in the package directory EXCEPT `component.toml`
/// itself (the manifest that carries the hash cannot hash itself), sorted by
/// relative path for determinism, each entry contributing `path\0<bytes>\0`.
///
/// # Errors
/// An I/O error reading the directory or a file within it.
pub fn compute_revision(package_dir: &Path) -> LibraryResult<String> {
    let mut entries: Vec<PathBuf> = Vec::new();
    collect_files(package_dir, package_dir, &mut entries)?;
    entries.sort();

    let mut hasher = Sha256::new();
    for rel in &entries {
        if rel == Path::new(COMPONENT_MANIFEST_FILE) {
            continue;
        }
        let bytes = std::fs::read(package_dir.join(rel))?;
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update([0u8]);
        hasher.update(&bytes);
        hasher.update([0u8]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> LibraryResult<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else {
            out.push(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_toml() -> &'static str {
        r#"
[identity]
id = "onecad.std.iso4762"
version = "1.0.0"
revision = "sha256:0000000000000000000000000000000000000000000000000000000000000000"

[metadata]
name = "Socket Head Cap Screw"
standard = "ISO 4762"
category = ["fasteners", "socket-head"]
tags = ["metric", "shcs"]

[source]
kind = "generator"
generator = "iso4762"
generator_version = 1

[parameters]
thread = { role = "free", key = "M6", domain = ["M3", "M4", "M5", "M6", "M8"] }
length = { role = "free", value = 20, snap = "preferred", min = 4 }
head_d = { role = "table", from = "iso4762.dk" }

[attachments]
head_seat = { on = "face:head_underside", accepts = ["plane"] }
shank_axis = { on = "cylinder:shank", accepts = ["cylinder", "hole", "circularEdge"] }
"#
    }

    #[test]
    fn valid_package_parses_and_validates() {
        let pkg = parse(valid_toml(), Path::new("component.toml")).unwrap();
        assert_eq!(pkg.identity.id, "onecad.std.iso4762");
        assert_eq!(pkg.metadata.unit, "mm"); // default
        assert!(matches!(pkg.source, SourceSpec::Generator { .. }));
        assert_eq!(pkg.parameters.len(), 3);
        assert_eq!(pkg.attachments.len(), 2);
        assert!(validate_identity(&pkg).is_ok());
    }

    #[test]
    fn malformed_toml_is_a_typed_error_not_a_panic() {
        let err = parse("not valid toml [[[", Path::new("component.toml")).unwrap_err();
        assert!(matches!(err, LibraryError::MalformedPackage { .. }));
    }

    #[test]
    fn unnamespaced_id_is_rejected() {
        let toml = valid_toml().replace(r#"id = "onecad.std.iso4762""#, r#"id = "unnamespaced""#);
        let pkg = parse(&toml, Path::new("component.toml")).unwrap();
        assert!(validate_identity(&pkg).is_err());
    }

    #[test]
    fn malformed_revision_is_rejected() {
        let toml = valid_toml().replace(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "not-a-hash",
        );
        let pkg = parse(&toml, Path::new("component.toml")).unwrap();
        assert!(validate_identity(&pkg).is_err());
    }

    #[test]
    fn revision_recompute_is_deterministic_and_content_addressed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(COMPONENT_MANIFEST_FILE), valid_toml()).unwrap();
        std::fs::write(dir.path().join("preview.webp"), b"fake-image-bytes").unwrap();

        let rev1 = compute_revision(dir.path()).unwrap();
        let rev2 = compute_revision(dir.path()).unwrap();
        assert_eq!(rev1, rev2, "deterministic across repeated calls");
        assert!(rev1.starts_with("sha256:"));

        // Changing a non-manifest file changes the revision.
        std::fs::write(dir.path().join("preview.webp"), b"different-bytes").unwrap();
        let rev3 = compute_revision(dir.path()).unwrap();
        assert_ne!(rev1, rev3, "content change moves the revision");

        // Changing ONLY component.toml's own bytes does NOT change the revision
        // (it is excluded from its own hash) — proves the self-reference is broken.
        std::fs::write(dir.path().join("preview.webp"), b"fake-image-bytes").unwrap();
        let mut toml_with_different_name = valid_toml().to_string();
        toml_with_different_name.push_str("\n# a trailing comment\n");
        std::fs::write(
            dir.path().join(COMPONENT_MANIFEST_FILE),
            &toml_with_different_name,
        )
        .unwrap();
        let rev4 = compute_revision(dir.path()).unwrap();
        assert_eq!(
            rev1, rev4,
            "component.toml's own bytes are excluded from its hash"
        );
    }
}
