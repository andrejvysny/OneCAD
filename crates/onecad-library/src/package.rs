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
    /// Name of the `[configurations.<name>]` table a placement uses when the
    /// caller picks none. Absent ⇒ no preselected variant (the package's own
    /// `[parameters]` defaults).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_configuration: Option<String>,
}

fn default_unit() -> String {
    "mm".to_string()
}

/// `[source]` — exactly one of `embedded` | `generator` | `document` (spec §2.1).
///
/// **WP-3.2 widened the two blob-backed kinds.** Spec §2.1's comment lines name
/// only `blob` / `file`, which is not enough to READ the bytes back: the worker's
/// replay readers need to know which byte form they are (`step` text vs. BinTools
/// `brep` vs. BinXCAF `xbf`) and, for the two binary forms, the format version
/// they were written in — a record pinned to a version the worker does not write
/// must fail loudly rather than be silently misparsed (SCHEMA §7.3
/// `ImportStep.brepFormat`). So both kinds carry `codec` + `format`, and
/// `document` additionally carries the baked-geometry pointer.
///
/// A `document` package's geometry is **baked at authoring**, not replayed at
/// placement: replaying a nested document needs a second worker session (the
/// worker is one-session-per-process) and the library folder to still exist,
/// and spec §4 requires a placed component to render with the library deleted.
/// A `document` package with no baked geometry is refused, loudly, by
/// [`resolve`](crate::resolve::resolve) — never silently replayed, never
/// substituted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SourceSpec {
    Embedded {
        /// Blob-store key. Accepts a bare 64-hex digest or spec §2.1's
        /// `"sha256:<hex>"` spelling.
        blob: String,
        /// Byte form of `blob` (`step` | `brep` | `xbf`). Defaults to `step`,
        /// matching spec §2.1's "a STEP/BREP payload" wording where a plain
        /// STEP file is the unadorned case.
        #[serde(default = "default_blob_codec")]
        codec: String,
        /// Binary format version — REQUIRED for `brep` / `xbf`, meaningless for
        /// `step`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<u32>,
    },
    Generator {
        generator: String,
        generator_version: u32,
    },
    /// A **length-parametric extrusion of an embedded planar profile** (WP-C).
    ///
    /// Vendor stock arrives as one fixed length — a 500 mm aluminium stick —
    /// while the component has to be placeable at any length, which no baked
    /// kind can express. `blob` is a SINGLE canonical planar face in the `brep`
    /// replay codec, not a solid, and the worker prisms it along `+Z` by the
    /// placing instance's `length`. The `[parameters].length` entry declares the
    /// package default.
    Profile {
        /// Blob-store key of the canonical face. Accepts a bare 64-hex digest or
        /// spec §2.1's `"sha256:<hex>"` spelling.
        blob: String,
        /// Pinned to `brep` by name — the `step` and `xbf` readers return
        /// SOLIDS, and this blob is a face.
        #[serde(default = "default_profile_codec")]
        codec: String,
        /// BinTools format version `blob` was written in. REQUIRED in practice;
        /// [`resolve`](crate::resolve::resolve) refuses its absence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<u32>,
    },
    Document {
        /// The frozen `.onecad` inside the package directory — identity,
        /// provenance, and the input a future re-bake (parameter overrides)
        /// would replay. Never read on the placement path.
        file: String,
        /// Blob-store key of the solid baked from `file` at authoring time.
        /// This is what a placement actually resolves to.
        geometry: String,
        #[serde(default = "default_document_geometry_codec")]
        geometry_codec: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        geometry_format: Option<u32>,
    },
}

fn default_blob_codec() -> String {
    "step".to_string()
}

fn default_profile_codec() -> String {
    // A canonical profile face is written by the worker's own BinTools lane, and
    // only the `brep` reader hands a face back.
    PROFILE_CODEC.to_string()
}

/// The one codec a [`SourceSpec::Profile`] blob may be in.
pub const PROFILE_CODEC: &str = "brep";

fn default_document_geometry_codec() -> String {
    // A bake comes out of the worker's own ExportGeometry lane, whose
    // attribute-preserving form is xbf (SCHEMA §7.8).
    "xbf".to_string()
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

impl ParameterSpec {
    /// A `role = "free"` parameter of a `document` package, bound to a NAMED
    /// VARIABLE of that package's frozen source document (WP-F1.3).
    ///
    /// `key` carries the variable name and `value` the variable's value at
    /// authoring time — the package's declared default, and the number the
    /// frozen document already builds, so an instance that overrides nothing
    /// resolves to exactly the baked solid.
    ///
    /// Constructed here rather than in the app crate so `toml::Value` stays an
    /// implementation detail of the package format (the app crate does not
    /// depend on `toml`).
    #[must_use]
    pub fn free_variable(variable: &str, value: f64) -> Self {
        Self {
            role: ParameterRole::Free,
            key: Some(variable.to_string()),
            value: Some(toml::Value::Float(value)),
            domain: None,
            snap: None,
            min: None,
            from: None,
        }
    }

    /// This spec's declared default as a number, when it has one.
    #[must_use]
    pub fn number(&self) -> Option<f64> {
        match self.value.as_ref()? {
            toml::Value::Float(f) => Some(*f),
            toml::Value::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterRole {
    Free,
    Table,
    Computed,
}

/// A `[attachments].<key>` entry — a named mate point (spec §2.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttachmentSpec {
    /// Local geometry the attachment names (`"face:head_underside"`,
    /// `"cylinder:shank"`).
    pub on: String,
    /// Geometry kinds this attachment mates with.
    pub accepts: Vec<String>,
    /// Component-local seating frame (WP-F1.1). Absent ⇒ the identity frame,
    /// i.e. the component seats at its own model origin — which is exactly
    /// how every pre-WP-F1.1 package behaves, so existing manifests stay
    /// byte-valid and place identically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<AttachmentFrame>,
}

/// The local basis an attachment seats FROM (spec §2.1 `[attachments].<key>
/// .frame`; WP-F1.1). Component-local, right-handed — `y` is derived (`z × x`)
/// rather than authored, so a left-handed basis cannot be written down at all.
///
/// `z` is the seating direction (the axis a `concentric` mate aligns to the
/// target axis, or a `coincident` mate to the target normal); `x` fixes only
/// the roll about it; `origin` is the point on the component that lands on the
/// target's seat point.
///
/// [`parse`] normalizes every frame it reads, so downstream code (the record
/// freeze, the index, both solvers) only ever sees an orthonormal basis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AttachmentFrame {
    #[serde(default)]
    pub origin: [f64; 3],
    #[serde(default = "unit_z")]
    pub z: [f64; 3],
    #[serde(default = "unit_x")]
    pub x: [f64; 3],
}

fn unit_z() -> [f64; 3] {
    [0.0, 0.0, 1.0]
}

fn unit_x() -> [f64; 3] {
    [1.0, 0.0, 0.0]
}

impl Default for AttachmentFrame {
    fn default() -> Self {
        Self {
            origin: [0.0; 3],
            z: unit_z(),
            x: unit_x(),
        }
    }
}

/// Below this an axis is treated as having no direction at all.
const AXIS_EPS: f64 = 1e-9;

impl AttachmentFrame {
    /// The orthonormalized form: `z` normalized, `x` re-orthogonalized against
    /// `z` (Gram-Schmidt) and normalized. `None` when either axis is
    /// degenerate — a zero-length axis, or an `x` parallel to `z` — neither of
    /// which names a basis at all, so both are refused rather than guessed at.
    ///
    /// Re-orthogonalizing instead of refusing a non-perpendicular `x` is
    /// deliberate: `z` is the seating direction and is authoritative, `x` only
    /// fixes the roll about it. A hand-typed `x` a fraction of a degree off is
    /// a rounding artifact, not a different intent — and projecting it still
    /// yields the roll the author asked for, while refusing would reject
    /// perfectly meaningful manifests over float noise.
    #[must_use]
    pub fn orthonormalized(&self) -> Option<Self> {
        let z = normalize(self.z)?;
        let dot = z[0] * self.x[0] + z[1] * self.x[1] + z[2] * self.x[2];
        let x = normalize([
            self.x[0] - z[0] * dot,
            self.x[1] - z[1] * dot,
            self.x[2] - z[2] * dot,
        ])?;
        if !self.origin.iter().all(|c| c.is_finite()) {
            return None;
        }
        Some(Self {
            origin: self.origin,
            z,
            x,
        })
    }
}

fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if !len.is_finite() || len < AXIS_EPS {
        return None;
    }
    Some([v[0] / len, v[1] / len, v[2] / len])
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
    /// `[configurations.<name>]` — named variants, each a map of `role = "free"`
    /// parameter key to value. Omitted from the manifest when empty, so every
    /// pre-configuration package round-trips byte-identically.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub configurations: Configurations,
}

/// Named variants of a package: configuration name -> (free-param key -> value).
pub type Configurations = BTreeMap<String, BTreeMap<String, toml::Value>>;

/// Longest accepted configuration name.
pub const MAX_CONFIGURATION_NAME_LEN: usize = 64;

/// A configuration name: non-empty, at most [`MAX_CONFIGURATION_NAME_LEN`]
/// characters, from `[A-Za-z0-9 _.-]`.
#[must_use]
pub fn is_valid_configuration_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_CONFIGURATION_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '.' | '-'))
}

/// Refuses configurations that could never be applied: bad names, empty
/// variants, keys that are not declared `role = "free"` parameters, and a
/// default that names no configuration.
///
/// # Errors
/// [`LibraryError::MalformedPackage`] naming the offending configuration.
pub fn validate_configurations(pkg: &ComponentPackage, path: &Path) -> LibraryResult<()> {
    let bad = |message: String| LibraryError::MalformedPackage {
        path: path.to_path_buf(),
        message,
    };
    for (name, values) in &pkg.configurations {
        if !is_valid_configuration_name(name) {
            return Err(bad(format!(
                "configurations.`{name}`: a name is 1..={MAX_CONFIGURATION_NAME_LEN} characters \
                 of [A-Za-z0-9 _.-]"
            )));
        }
        if values.is_empty() {
            return Err(bad(format!(
                "configurations.`{name}` is empty; a configuration sets at least one parameter"
            )));
        }
        for key in values.keys() {
            match pkg.parameters.get(key) {
                Some(spec) if spec.role == ParameterRole::Free => {}
                Some(_) => {
                    return Err(bad(format!(
                        "configurations.`{name}`: `{key}` is not a role = \"free\" parameter"
                    )))
                }
                None => {
                    return Err(bad(format!(
                        "configurations.`{name}`: `{key}` is not a declared parameter"
                    )))
                }
            }
        }
    }
    match &pkg.metadata.default_configuration {
        Some(d) if !pkg.configurations.contains_key(d) => Err(bad(format!(
            "metadata.default_configuration `{d}` names no [configurations] table"
        ))),
        _ => Ok(()),
    }
}

/// Parses `component.toml` bytes into a [`ComponentPackage`]. `path` is used
/// only for the error message.
///
/// Attachment frames are normalized HERE, at the one door every reader comes
/// through (the index, the placement freeze, the configurator), so no
/// downstream consumer has to re-check handedness or unit length — and a
/// degenerate frame is refused as a malformed package instead of reaching a
/// solver as a divide-by-zero.
pub fn parse(toml_str: &str, path: &Path) -> LibraryResult<ComponentPackage> {
    let mut pkg: ComponentPackage =
        toml::from_str(toml_str).map_err(|e| LibraryError::MalformedPackage {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
    for (key, spec) in &mut pkg.attachments {
        let Some(frame) = spec.frame else { continue };
        spec.frame =
            Some(
                frame
                    .orthonormalized()
                    .ok_or_else(|| LibraryError::MalformedPackage {
                        path: path.to_path_buf(),
                        message: format!(
                            "attachments.{key}.frame is degenerate: `z` and `x` must be non-zero, \
                     finite, and not parallel (got z = {:?}, x = {:?})",
                            frame.z, frame.x
                        ),
                    })?,
            );
    }
    legacy_generator_defaults(&mut pkg);
    validate_configurations(&pkg, path)?;
    Ok(pkg)
}

/// Legacy shim for pre-`SEED_VERSION` 6 seed manifests: a GENERATOR package's
/// free parameter that sets `key` but no `value` (`thread = { role = "free",
/// key = "M6", … }`) meant "`key` is the default". Every consumer reads
/// defaults from `value`, so it is filled in here, at the one door manifests
/// come through. `document` packages are untouched: their `key` names a
/// source-document variable.
fn legacy_generator_defaults(pkg: &mut ComponentPackage) {
    if !matches!(pkg.source, SourceSpec::Generator { .. }) {
        return;
    }
    for spec in pkg.parameters.values_mut() {
        if spec.role == ParameterRole::Free && spec.value.is_none() {
            spec.value = spec.key.clone().map(toml::Value::String);
        }
    }
}

/// Serializes a package back to `component.toml` text (spec §2.1).
///
/// The inverse of [`parse`], and the authoring half of the format: "Save as
/// Component" (WP-B2) writes packages, it does not only read them. Round-trip
/// fidelity is pinned by test — a field this cannot write is a field an
/// authored package silently loses.
///
/// # Errors
/// A package whose `[parameters]` carry values TOML cannot represent.
pub fn to_toml(pkg: &ComponentPackage) -> LibraryResult<String> {
    toml::to_string_pretty(pkg).map_err(|e| LibraryError::Io(format!("serialize package: {e}")))
}

/// Writes `component.toml` into `package_dir`, then recomputes and rewrites
/// `identity.revision` so the manifest describes what is ACTUALLY on disk.
///
/// Two passes, deliberately: the revision hashes every file in the directory
/// EXCEPT the manifest, so every other file must already be written when it is
/// computed — and the computed value then has to land in the manifest itself.
/// Writing the manifest once with a guessed revision is how a package ends up
/// claiming a hash it does not have, which makes every placement of it resolve
/// `NeedsRepair` (spec §4).
///
/// Returns the recorded revision.
///
/// # Errors
/// I/O failure, or a package that fails [`validate_identity`].
pub fn write_package(package_dir: &Path, pkg: &ComponentPackage) -> LibraryResult<String> {
    validate_identity(pkg)?;
    validate_configurations(pkg, &package_dir.join(COMPONENT_MANIFEST_FILE))?;
    std::fs::create_dir_all(package_dir)?;
    let manifest_path = package_dir.join(COMPONENT_MANIFEST_FILE);
    std::fs::write(&manifest_path, to_toml(pkg)?)?;

    let document = match &pkg.source {
        SourceSpec::Document { file, .. } => {
            Some(crate::resolve::read_document(package_dir, file)?)
        }
        _ => None,
    };
    let canonical = crate::semantic::parse(&to_toml(pkg)?, &manifest_path)?;
    let revision = crate::semantic::revision(&canonical, document.as_deref())?;
    let mut stamped = pkg.clone();
    stamped.identity.revision = revision.clone();
    std::fs::write(&manifest_path, to_toml(&stamped)?)?;
    Ok(revision)
}

/// Validates the package's identity invariants (spec §2.1): namespaced id,
/// non-empty version, well-formed `sha256:` revision.
///
/// The id and version are also PATH components (`<root>/<id>@<version>`,
/// `Library::save_component`), so both are restricted to a path-safe charset —
/// an id like `onecad.std/../evil` must never reach a `Path::join`. The op
/// layer applies the same rule (`PlaceComponentParams::validate` in
/// `onecad-core::document::record`; the crates deliberately share no dep, so
/// the two checks are kept in lockstep by their tests).
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
    if !is_path_safe_id(&pkg.identity.id) {
        return Err(LibraryError::InvalidIdentity(format!(
            "identity.id `{}` may only contain [a-z0-9._-] and no `..` segment \
             (it names the package directory)",
            pkg.identity.id
        )));
    }
    if pkg.identity.version.trim().is_empty() {
        return Err(LibraryError::InvalidIdentity(
            "identity.version must not be empty".into(),
        ));
    }
    if !is_path_safe_version(&pkg.identity.version) {
        return Err(LibraryError::InvalidIdentity(format!(
            "identity.version `{}` must be MAJOR.MINOR.PATCH (digits only; no pre-release \
             or build metadata, which the version ordering cannot compare)",
            pkg.identity.version
        )));
    }
    match pkg
        .identity
        .revision
        .strip_prefix("sha256:")
        .or_else(|| pkg.identity.revision.strip_prefix(crate::semantic::PREFIX))
    {
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

fn is_path_safe_id(id: &str) -> bool {
    id.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
        && id.split('.').all(|seg| !seg.is_empty())
}

fn is_path_safe_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Recomputes a package directory's content hash (spec §2.1 `identity.revision`):
/// the [`SourceSpec`] references [`hash_referenced_geometry`] folds in
/// FIRST, then SHA-256 over every file in the package directory EXCEPT
/// `component.toml` itself (the manifest that carries the hash cannot hash
/// itself), sorted by relative path for determinism, each entry contributing
/// `path\0<bytes>\0`.
///
/// **Why the source is hashed at all.** `embedded`/`profile` packages
/// (`Library::save_embedded_component`) keep their geometry in the library's
/// content-addressed `blobs/` store, one level ABOVE the package directory —
/// the file walk below never sees it. Without folding in the blob digest,
/// every such package hashes to the SHA-256 of an empty input regardless of
/// what geometry it actually references: two different components come out
/// revision-identical, and swapping the blob under a fixed `id@version` is
/// undetectable by [`resolve`](crate::resolve::resolve)'s revision check. A
/// `document` package's frozen `source.onecad` IS in the directory (walked
/// below) but its baked `geometry` field is itself a `blobs/` pointer, so it
/// is folded in for the same reason.
///
/// **This only affects the revision RECOMPUTED at [`write_package`] time.**
/// A `document`-kind package already on disk keeps whatever `identity.revision`
/// its manifest carries — [`crate::index`] caches that value on reindex and
/// never recomputes it on read, so an existing package written before this
/// fix keeps its old (empty-hash-derived) revision until it is next saved.
///
/// # Errors
/// An I/O error reading the directory or a file within it.
pub fn compute_revision(package_dir: &Path) -> LibraryResult<String> {
    let mut entries: Vec<PathBuf> = Vec::new();
    collect_files(package_dir, package_dir, &mut entries)?;
    entries.sort();

    let mut hasher = Sha256::new();
    hash_referenced_geometry(package_dir, &mut hasher)?;

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

/// Folds the content address of geometry `component.toml`'s `[source]`
/// references — which may live outside `package_dir` entirely — into
/// `hasher`, BEFORE [`compute_revision`]'s file-entry loop, so the two halves
/// of the hash can never collide in interpretation (a blob digest can never
/// be mistaken for a file's contents, or vice versa).
///
/// One domain-separated line per source kind: `source:embedded:<sha>:<codec>:
/// <format>\n` for [`SourceSpec::Embedded`], `source:profile:…` for
/// [`SourceSpec::Profile`], `source:document:…` for [`SourceSpec::Document`]'s
/// baked `geometry` pointer, `source:generator:<id>:<version>\n` for
/// [`SourceSpec::Generator`] (which references no blob at all, but is still
/// pinned so a generator swap under a fixed id/version is not silently
/// invisible either). A blob digest is normalized to bare 64-hex before
/// hashing — both spellings `resolve`'s own blob reader accepts
/// (`"sha256:<hex>"` and bare) must produce the SAME revision.
///
/// A missing or unparsable manifest contributes nothing: `compute_revision`
/// is sometimes called (directly, by tests) against a directory whose
/// `component.toml` does not exist yet, and identity validity is
/// [`validate_identity`]'s job, not this one's.
fn hash_referenced_geometry(package_dir: &Path, hasher: &mut Sha256) -> LibraryResult<()> {
    let manifest_path = package_dir.join(COMPONENT_MANIFEST_FILE);
    let raw = match std::fs::read_to_string(&manifest_path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let Ok(pkg) = parse(&raw, &manifest_path) else {
        return Ok(());
    };
    match pkg.source {
        SourceSpec::Embedded {
            blob,
            codec,
            format,
        } => hash_blob_ref(hasher, "embedded", &blob, &codec, format),
        SourceSpec::Profile {
            blob,
            codec,
            format,
        } => hash_blob_ref(hasher, "profile", &blob, &codec, format),
        SourceSpec::Document {
            geometry,
            geometry_codec,
            geometry_format,
            ..
        } => hash_blob_ref(
            hasher,
            "document",
            &geometry,
            &geometry_codec,
            geometry_format,
        ),
        // A generator package references no blob, so it contributes nothing
        // here: its revision stays exactly what it was before blob-kind sources
        // joined this hash, which keeps every shipped seed manifest's DECLARED
        // revision valid (`library_seed::tests::declared_revisions_match_a_
        // manifest_only_package` pins that). A generator swap under a fixed
        // id@version is already visible through `generator`/`generator_version`
        // in the manifest itself.
        SourceSpec::Generator { .. } => {}
    }
    Ok(())
}

/// One `source:<kind>:<bare-sha256>:<codec>:<format>\n` line — see
/// [`hash_referenced_geometry`].
fn hash_blob_ref(hasher: &mut Sha256, kind: &str, blob: &str, codec: &str, format: Option<u32>) {
    // Spec §2.1 writes the pointer as `"sha256:<hex>"`; `resolve::read_blob`
    // accepts a bare digest too. Both spellings must hash the same.
    let bare = blob.strip_prefix("sha256:").unwrap_or(blob);
    let format = format
        .map(|f| f.to_string())
        .unwrap_or_else(|| "none".to_string());
    hasher.update(format!("source:{kind}:{bare}:{codec}:{format}\n").as_bytes());
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

    /// WP-F1.1: an attachment with no `frame` reads as `None`, which is what
    /// keeps every already-authored package (the seeded catalog included)
    /// byte-valid and placing exactly as before.
    #[test]
    fn an_attachment_with_no_frame_stays_absent() {
        let pkg = parse(valid_toml(), Path::new("component.toml")).unwrap();
        assert!(pkg.attachments["head_seat"].frame.is_none());
        assert!(pkg.attachments["shank_axis"].frame.is_none());
    }

    #[test]
    fn an_attachment_frame_parses_and_is_orthonormalized() {
        let toml = valid_toml().replace(
            r#"accepts = ["plane"] }"#,
            r#"accepts = ["plane"], frame = { origin = [0.0, 0.0, 10.0], z = [0.0, 0.0, 2.0], x = [3.0, 0.0, 0.5] } }"#,
        );
        let pkg = parse(&toml, Path::new("component.toml")).unwrap();
        let frame = pkg.attachments["head_seat"].frame.unwrap();
        assert_eq!(frame.origin, [0.0, 0.0, 10.0]);
        // `z` normalized, and `x` re-orthogonalized against it (its 0.5 of z
        // is projected out) then normalized — the solvers may assume both.
        assert_eq!(frame.z, [0.0, 0.0, 1.0]);
        assert!((frame.x[0] - 1.0).abs() < 1e-12, "x = {:?}", frame.x);
        assert!(frame.x[1].abs() < 1e-12 && frame.x[2].abs() < 1e-12);

        // A frame an authoring pass writes back must survive the round trip —
        // a lost frame is a component that silently reverts to origin-seating.
        let again = parse(&to_toml(&pkg).unwrap(), Path::new("component.toml")).unwrap();
        assert_eq!(pkg, again);
    }

    /// Partial frames are legal: an attachment that only moves the seat point
    /// takes the identity axes, which is the common authoring case.
    #[test]
    fn an_origin_only_frame_takes_the_identity_axes() {
        let toml = valid_toml().replace(
            r#"accepts = ["plane"] }"#,
            r#"accepts = ["plane"], frame = { origin = [1.0, 2.0, 3.0] } }"#,
        );
        let pkg = parse(&toml, Path::new("component.toml")).unwrap();
        let frame = pkg.attachments["head_seat"].frame.unwrap();
        assert_eq!(frame.origin, [1.0, 2.0, 3.0]);
        assert_eq!(frame.z, [0.0, 0.0, 1.0]);
        assert_eq!(frame.x, [1.0, 0.0, 0.0]);
    }

    /// A degenerate axis is refused at the door, never normalized into a
    /// divide-by-zero inside a solver.
    #[test]
    fn a_degenerate_attachment_frame_is_refused() {
        for bad in [
            r#"frame = { z = [0.0, 0.0, 0.0] }"#,
            r#"frame = { x = [0.0, 0.0, 0.0] }"#,
            // `x` parallel to `z` names no roll at all — Gram-Schmidt leaves
            // nothing behind, so there is no basis to recover.
            r#"frame = { z = [0.0, 0.0, 1.0], x = [0.0, 0.0, 4.0] }"#,
        ] {
            let toml = valid_toml().replace(
                r#"accepts = ["plane"] }"#,
                &format!(r#"accepts = ["plane"], {bad} }}"#),
            );
            let err = parse(&toml, Path::new("component.toml")).unwrap_err();
            assert!(
                matches!(err, LibraryError::MalformedPackage { .. }),
                "`{bad}` must be refused as malformed"
            );
        }
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
    fn path_escaping_id_or_version_is_rejected() {
        // id and version name the package directory (`<id>@<version>`) — a
        // separator or dot-dot segment must never reach a `Path::join`.
        for evil in [
            "onecad.std/../evil",
            "..",
            "a..b",
            r"onecad\evil.x",
            "Onecad.Std.X",
            "onecad.std.iso 4762",
        ] {
            let mut pkg = parse(valid_toml(), Path::new("component.toml")).unwrap();
            pkg.identity.id = evil.to_string();
            assert!(
                validate_identity(&pkg).is_err(),
                "id `{evil}` must be refused"
            );
        }
        let mut pkg = parse(valid_toml(), Path::new("component.toml")).unwrap();
        pkg.identity.version = "1.0.0/../..".to_string();
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

    /// Swaps `valid_toml()`'s `[source]` table for an arbitrary one — the
    /// tests below only care about `[source]`, not the rest of the manifest.
    fn toml_with_source(source_block: &str) -> String {
        valid_toml().replace(
            "[source]\nkind = \"generator\"\ngenerator = \"iso4762\"\ngenerator_version = 1",
            source_block,
        )
    }

    /// The defect this fix exists for: an `embedded` package's geometry lives
    /// in `blobs/`, outside the directory the file-entry loop hashes, so
    /// without folding the blob digest in, every embedded package hashed to
    /// the SHA-256 of empty input regardless of what it referenced. Also
    /// covers `format`: it is not cosmetic (a `brep`/`xbf` reader keyed to the
    /// wrong version reads the wrong bytes), so it must move the revision too.
    #[test]
    fn embedded_source_folds_the_blob_digest_and_format_into_the_revision() {
        let dir = tempfile::tempdir().unwrap();
        let write_and_hash = |blob_hex: &str, format: u32| -> String {
            let toml = toml_with_source(&format!(
                "[source]\nkind = \"embedded\"\nblob = \"sha256:{blob_hex}\"\ncodec = \"brep\"\nformat = {format}"
            ));
            std::fs::write(dir.path().join(COMPONENT_MANIFEST_FILE), toml).unwrap();
            compute_revision(dir.path()).unwrap()
        };

        let rev_a = write_and_hash(&"a".repeat(64), 3);
        let rev_b = write_and_hash(&"b".repeat(64), 3);
        assert_ne!(
            rev_a, rev_b,
            "a different blob digest must move the revision"
        );

        let rev_a_fmt4 = write_and_hash(&"a".repeat(64), 4);
        assert_ne!(
            rev_a, rev_a_fmt4,
            "changing `format` alone (same blob) must move the revision"
        );
    }

    /// Spec §2.1 writes a blob pointer as `"sha256:<hex>"`; the blob store (and
    /// `resolve::read_blob`) key on the bare digest and accept both spellings.
    /// The revision must not depend on which spelling an author typed.
    #[test]
    fn bare_and_sha256_prefixed_blob_spellings_hash_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let hex = "c".repeat(64);

        let prefixed = toml_with_source(&format!(
            "[source]\nkind = \"embedded\"\nblob = \"sha256:{hex}\"\ncodec = \"brep\"\nformat = 1"
        ));
        std::fs::write(dir.path().join(COMPONENT_MANIFEST_FILE), prefixed).unwrap();
        let rev_prefixed = compute_revision(dir.path()).unwrap();

        let bare = toml_with_source(&format!(
            "[source]\nkind = \"embedded\"\nblob = \"{hex}\"\ncodec = \"brep\"\nformat = 1"
        ));
        std::fs::write(dir.path().join(COMPONENT_MANIFEST_FILE), bare).unwrap();
        let rev_bare = compute_revision(dir.path()).unwrap();

        assert_eq!(
            rev_prefixed, rev_bare,
            "`sha256:` and bare spellings of the same blob must be the same revision"
        );
    }

    /// A `document` package's frozen `source.onecad` IS in the directory (so
    /// the file-entry loop already covers IT), but its baked `geometry`
    /// pointer is itself a `blobs/` digest — a re-bake under the same document
    /// must move the revision just as much as an embedded blob swap does.
    #[test]
    fn document_source_revision_changes_when_its_geometry_blob_changes() {
        let dir = tempfile::tempdir().unwrap();
        // Held constant across both computations so the change is isolated to
        // the `geometry` pointer, not the file the loop below also hashes.
        std::fs::write(dir.path().join("source.onecad"), b"frozen document bytes").unwrap();

        let write_and_hash = |geometry_hex: &str| -> String {
            let toml = toml_with_source(&format!(
                "[source]\nkind = \"document\"\nfile = \"source.onecad\"\ngeometry = \"{geometry_hex}\"\ngeometry_codec = \"xbf\"\ngeometry_format = 4"
            ));
            std::fs::write(dir.path().join(COMPONENT_MANIFEST_FILE), toml).unwrap();
            compute_revision(dir.path()).unwrap()
        };

        let rev_a = write_and_hash(&"1".repeat(64));
        let rev_b = write_and_hash(&"2".repeat(64));
        assert_ne!(
            rev_a, rev_b,
            "a re-baked geometry blob under the same document must move the revision"
        );
    }

    #[test]
    fn a_package_round_trips_through_to_toml() {
        // What this pins: an authored package that loses a field on write is a
        // package that silently changes meaning the next time it is read.
        let pkg = parse(valid_toml(), Path::new("component.toml")).unwrap();
        let text = to_toml(&pkg).unwrap();
        let again = parse(&text, Path::new("component.toml")).unwrap();
        assert_eq!(pkg, again);
    }

    #[test]
    fn write_package_stamps_the_revision_of_what_is_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let pkg_dir = dir.path().join("acme.bracket");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        // A sibling file is what the revision actually hashes.
        std::fs::write(pkg_dir.join("source.onecad"), b"frozen document bytes").unwrap();

        let mut pkg = parse(valid_toml(), Path::new("component.toml")).unwrap();
        pkg.identity.id = "acme.bracket".to_string();
        pkg.identity.revision = format!("sha256:{}", "f".repeat(64)); // deliberately wrong

        let recorded = write_package(&pkg_dir, &pkg).unwrap();
        let on_disk = parse(
            &std::fs::read_to_string(pkg_dir.join(COMPONENT_MANIFEST_FILE)).unwrap(),
            Path::new("component.toml"),
        )
        .unwrap();
        assert_eq!(on_disk.identity.revision, recorded);
        assert_eq!(recorded, crate::semantic::revision(&on_disk, None).unwrap());
        assert_ne!(recorded, format!("sha256:{}", "f".repeat(64)));
    }

    #[test]
    fn write_package_refuses_an_invalid_identity_before_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        let pkg_dir = dir.path().join("nope");
        let mut pkg = parse(valid_toml(), Path::new("component.toml")).unwrap();
        pkg.identity.id = "unnamespaced".to_string();
        assert!(write_package(&pkg_dir, &pkg).is_err());
        assert!(!pkg_dir.join(COMPONENT_MANIFEST_FILE).exists());
    }

    fn with_configs(extra: &str) -> String {
        format!("{}\n{extra}", valid_toml())
    }

    fn parse_err(text: &str) -> String {
        parse(text, Path::new("component.toml"))
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn configurations_parse_and_round_trip() {
        let text = with_configs(
            "[configurations.Small]\nlength = 10\n\n[configurations.\"Heavy Duty\"]\nlength = 40\nthread = \"M8\"\n",
        );
        let text = text.replace(
            "tags = [\"metric\", \"shcs\"]",
            "tags = [\"metric\", \"shcs\"]\ndefault_configuration = \"Small\"",
        );
        let pkg = parse(&text, Path::new("component.toml")).unwrap();
        assert_eq!(pkg.configurations.len(), 2);
        assert_eq!(pkg.metadata.default_configuration.as_deref(), Some("Small"));
        let again = parse(&to_toml(&pkg).unwrap(), Path::new("component.toml")).unwrap();
        assert_eq!(pkg, again);
    }

    #[test]
    fn a_package_without_configurations_serializes_none_of_them() {
        let pkg = parse(valid_toml(), Path::new("component.toml")).unwrap();
        let text = to_toml(&pkg).unwrap();
        assert!(!text.contains("configurations"), "{text}");
        assert!(!text.contains("default_configuration"), "{text}");
    }

    #[test]
    fn a_configuration_naming_an_undeclared_or_non_free_key_is_refused() {
        let undeclared = parse_err(&with_configs("[configurations.A]\nnope = 1\n"));
        assert!(
            undeclared.contains("`nope` is not a declared parameter"),
            "{undeclared}"
        );
        let table = parse_err(&with_configs("[configurations.A]\nhead_d = 1\n"));
        assert!(table.contains("`head_d` is not a role"), "{table}");
    }

    #[test]
    fn an_empty_configuration_is_refused() {
        let msg = parse_err(&with_configs("[configurations.A]\n"));
        assert!(msg.contains("configurations.`A` is empty"), "{msg}");
    }

    #[test]
    fn a_default_configuration_must_name_a_configuration() {
        let text = valid_toml().replace(
            "tags = [\"metric\", \"shcs\"]",
            "tags = [\"metric\", \"shcs\"]\ndefault_configuration = \"Ghost\"",
        );
        let msg = parse_err(&text);
        assert!(msg.contains("default_configuration `Ghost`"), "{msg}");
    }

    #[test]
    fn configuration_names_are_bounded_and_charset_checked() {
        assert!(is_valid_configuration_name("Heavy 2.0_a-b"));
        assert!(!is_valid_configuration_name(""));
        assert!(!is_valid_configuration_name(&"x".repeat(65)));
        assert!(is_valid_configuration_name(&"x".repeat(64)));
        assert!(!is_valid_configuration_name("a/b"));
        let msg = parse_err(&with_configs("[configurations.\"a/b\"]\nlength = 1\n"));
        assert!(msg.contains("configurations.`a/b`"), "{msg}");
    }

    #[test]
    fn a_version_must_be_plain_major_minor_patch() {
        let mut pkg = parse(valid_toml(), Path::new("component.toml")).unwrap();
        for ok in ["1.0.0", "0.12.304"] {
            pkg.identity.version = ok.to_string();
            validate_identity(&pkg).unwrap();
        }
        for bad in [
            "1.0",
            "1.0.0-rc1",
            "1.0.0+b5",
            "1.x.0",
            "1..0",
            "1.0.0.0",
            "v1.0.0",
        ] {
            pkg.identity.version = bad.to_string();
            assert!(validate_identity(&pkg).is_err(), "{bad} must be refused");
        }
    }

    const LEGACY_NUT: &str = r#"
[identity]
id = "onecad.std.iso4032"
version = "1.0.0"
revision = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
[metadata]
name = "Hex Nut"
designation = "ISO 4032 {thread}"
[source]
kind = "generator"
generator = "iso4032"
generator_version = 1
[parameters]
thread = { role = "free", key = "M6", domain = ["M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"] }
"#;

    const LEGACY_SCREW: &str = r#"
[identity]
id = "onecad.std.iso4762"
version = "1.0.0"
revision = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
[metadata]
name = "Socket Head Cap Screw"
designation = "ISO 4762 {thread}x{length}"
[source]
kind = "generator"
generator = "iso4762"
generator_version = 1
[parameters]
thread = { role = "free", key = "M6", domain = ["M2", "M6", "M8"] }
length = { role = "free", value = 20, snap = "preferred", min = 2 }
thread_detail = { role = "free", value = "cosmetic", domain = ["cosmetic", "modeled"] }
"#;

    #[test]
    fn a_legacy_generator_key_is_the_default_when_no_value_is_declared() {
        for text in [LEGACY_NUT, LEGACY_SCREW] {
            let pkg = parse(text, Path::new("component.toml")).unwrap();
            assert_eq!(
                pkg.parameters["thread"].value,
                Some(toml::Value::String("M6".into())),
                "the legacy `key` is the default"
            );
        }
        let screw = parse(LEGACY_SCREW, Path::new("component.toml")).unwrap();
        assert_eq!(screw.parameters["length"].number(), Some(20.0));
    }

    #[test]
    fn the_key_of_a_document_parameter_is_never_read_as_a_default() {
        let text = valid_toml().replace(
            "kind = \"generator\"\ngenerator = \"iso4762\"\ngenerator_version = 1",
            "kind = \"document\"\nfile = \"s.onecad\"\ngeometry = \"abc\"",
        );
        let pkg = parse(&text, Path::new("component.toml")).unwrap();
        assert_eq!(pkg.parameters["thread"].value, None);
    }
}
