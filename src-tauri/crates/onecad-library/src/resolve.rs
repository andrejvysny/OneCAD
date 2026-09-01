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

/// The worker-consumable geometry source a resolution produced — all three spec
/// §2.1 kinds as of WP-3.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedSource {
    Generator {
        generator_id: String,
        generator_version: u32,
    },
    /// A static payload from the blob store.
    Embedded { blob: ResolvedBlob },
    /// A canonical planar FACE from the blob store, prismed to the placing
    /// instance's `length` at regen (WP-C; see [`SourceSpec::Profile`]). The
    /// blob is bytes like any other — what differs is that the placement carries
    /// a live parameter rather than a frozen solid.
    Profile { blob: ResolvedBlob },
    /// A user-authored component: the baked solid, plus the digest of the frozen
    /// authoring document it was baked from (recorded as provenance and as the
    /// key a future re-bake would need — see [`SourceSpec::Document`]).
    Document {
        document_sha256: String,
        blob: ResolvedBlob,
    },
}

/// Bytes from the library blob store plus everything needed to read them back:
/// the content address (which becomes the placing document's own `imports/` key),
/// the byte form, and the binary format pin.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedBlob {
    /// Bare 64-hex digest — the same value the placing document will key its
    /// copy of these bytes by.
    pub sha256: String,
    /// `step` | `brep` | `xbf`.
    pub codec: String,
    /// Binary format version; `None` only for `step`.
    pub format: Option<u32>,
    /// The verified bytes (the store re-hashes on every read).
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for ResolvedBlob {
    /// Hand-written so a resolution failure log never dumps megabytes of BRep.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedBlob")
            .field("sha256", &self.sha256)
            .field("codec", &self.codec)
            .field("format", &self.format)
            .field("bytes", &format_args!("<{} bytes>", self.bytes.len()))
            .finish()
    }
}

/// The codecs a component blob may be in — the §7.3 replay codecs, verbatim.
const KNOWN_CODECS: [&str; 3] = ["step", "brep", "xbf"];

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
        SourceSpec::Embedded {
            blob,
            codec,
            format,
        } => Ok(ResolvedSource::Embedded {
            blob: read_blob(
                library_root,
                &manifest_path,
                "source.blob",
                &blob,
                &codec,
                format,
            )?,
        }),
        SourceSpec::Profile {
            blob,
            codec,
            format,
        } => {
            // Pinned by NAME, here at the door, rather than left to the worker:
            // the other readers return solids, so a mis-declared codec would fail
            // at the first regen on someone else's machine instead of at ingest.
            if codec != package::PROFILE_CODEC {
                return Err(LibraryError::MalformedPackage {
                    path: manifest_path,
                    message: format!(
                        "source.codec `{codec}` is not `{}` — a profile blob is a single planar \
                         face, and only the brep reader returns one",
                        package::PROFILE_CODEC
                    ),
                });
            }
            Ok(ResolvedSource::Profile {
                blob: read_blob(
                    library_root,
                    &manifest_path,
                    "source.blob",
                    &blob,
                    &codec,
                    format,
                )?,
            })
        }
        SourceSpec::Document {
            file,
            geometry,
            geometry_codec,
            geometry_format,
        } => {
            // The frozen document is hashed, not read: a placement resolves to the
            // BAKED solid (see `SourceSpec::Document`). Reading it here proves the
            // package still carries the document its geometry claims to come from,
            // which is the whole value of recording the digest.
            let document_path = library_root.join(&entry.path).join(&file);
            let document_bytes =
                std::fs::read(&document_path).map_err(|e| LibraryError::MalformedPackage {
                    path: manifest_path.clone(),
                    message: format!(
                        "source.file `{file}` is not readable at {}: {e}",
                        document_path.display()
                    ),
                })?;
            Ok(ResolvedSource::Document {
                document_sha256: crate::blob::sha256_hex(&document_bytes),
                blob: read_blob(
                    library_root,
                    &manifest_path,
                    "source.geometry",
                    &geometry,
                    &geometry_codec,
                    geometry_format,
                )?,
            })
        }
    }
}

/// Reads and validates one blob-store pointer from a manifest.
///
/// Every failure names the manifest and the field, because these are all AUTHORING
/// mistakes in someone's package — an unknown codec, a missing format pin, a blob
/// that was never written. A `document` package whose bake is absent lands here
/// too: refused loudly, never quietly replayed from `source.onecad` (which would
/// need a second worker session and the library folder present at every regen —
/// exactly what spec §4 rules out).
fn read_blob(
    library_root: &Path,
    manifest_path: &Path,
    field: &str,
    key: &str,
    codec: &str,
    format: Option<u32>,
) -> LibraryResult<ResolvedBlob> {
    let malformed = |message: String| LibraryError::MalformedPackage {
        path: manifest_path.to_path_buf(),
        message,
    };
    if !KNOWN_CODECS.contains(&codec) {
        return Err(malformed(format!(
            "{field} codec `{codec}` is unknown (known: {})",
            KNOWN_CODECS.join(", ")
        )));
    }
    if codec != "step" && format.is_none() {
        return Err(malformed(format!(
            "{field} codec `{codec}` requires a binary format version"
        )));
    }
    // Spec §2.1 writes the pointer as `"sha256:<hex>"`; the blob store keys on the
    // bare digest. Accept both spellings rather than making an author guess.
    let sha256 = key.strip_prefix("sha256:").unwrap_or(key).to_string();
    let bytes = crate::blob::BlobStore::new(library_root).get(&sha256)?;
    Ok(ResolvedBlob {
        sha256,
        codec: codec.to_string(),
        format,
        bytes,
    })
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
