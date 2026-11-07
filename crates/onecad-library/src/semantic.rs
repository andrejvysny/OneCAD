//! Versioned semantic package identity. See `docs/format-compatibility.md`.

use crate::error::{LibraryError, LibraryResult};
use crate::package::{self, ComponentPackage, SourceSpec};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const PREFIX: &str = "sha256v2:";

/// Hash a parsed semantic manifest and the captured document payload, if any.
pub fn revision(pkg: &ComponentPackage, document: Option<&[u8]>) -> LibraryResult<String> {
    let mut canonical = pkg.clone();
    canonical.identity.revision.clear();
    match &mut canonical.source {
        SourceSpec::Embedded { blob, .. } | SourceSpec::Profile { blob, .. } => {
            normalize_blob(blob)
        }
        SourceSpec::Document { geometry, .. } => normalize_blob(geometry),
        SourceSpec::Generator { .. } => {}
    }
    if matches!(canonical.source, SourceSpec::Document { .. }) != document.is_some() {
        return Err(LibraryError::InvalidIdentity(
            "document payload closure is incomplete".into(),
        ));
    }
    let mut value = serde_json::to_value(&canonical)
        .map_err(|e| LibraryError::InvalidIdentity(format!("canonical manifest: {e}")))?;
    value.sort_all_objects();
    let bytes = serde_json::to_vec(&value)
        .map_err(|e| LibraryError::InvalidIdentity(format!("canonical manifest: {e}")))?;
    let mut hash = Sha256::new();
    hash.update(b"onecad.component.semantic.v2\0");
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    match document {
        Some(bytes) => {
            hash.update([1]);
            hash.update(Sha256::digest(bytes));
        }
        None => hash.update([0]),
    }
    Ok(format!("{PREFIX}{:x}", hash.finalize()))
}

fn normalize_blob(blob: &mut String) {
    if let Some(bare) = blob.strip_prefix("sha256:") {
        *blob = bare.to_owned();
    }
}

/// Refuse unknown manifest data, which the typed canonical encoder cannot preserve.
pub fn parse(raw: &str, path: &Path) -> LibraryResult<ComponentPackage> {
    let input: toml::Value = toml::from_str(raw).map_err(|e| LibraryError::MalformedPackage {
        path: path.into(),
        message: format!("{e}"),
    })?;
    reject_nonfinite(&input, path)?;
    let pkg = package::parse(raw, path)?;
    let known = toml::Value::try_from(&pkg).map_err(|e| LibraryError::MalformedPackage {
        path: path.into(),
        message: format!("{e}"),
    })?;
    reject_unknown(&input, &known, path)?;
    Ok(pkg)
}

fn reject_nonfinite(value: &toml::Value, path: &Path) -> LibraryResult<()> {
    match value {
        toml::Value::Float(x) if !x.is_finite() => {
            return Err(LibraryError::MalformedPackage {
                path: path.into(),
                message: "non-finite numbers are not supported in semantic manifests".into(),
            })
        }
        toml::Value::Array(values) => {
            for item in values {
                reject_nonfinite(item, path)?;
            }
        }
        toml::Value::Table(values) => {
            for item in values.values() {
                reject_nonfinite(item, path)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn reject_unknown(input: &toml::Value, known: &toml::Value, path: &Path) -> LibraryResult<()> {
    if let (toml::Value::Table(input), toml::Value::Table(known)) = (input, known) {
        for (key, value) in input {
            let Some(expected) = known.get(key) else {
                return Err(LibraryError::MalformedPackage {
                    path: path.into(),
                    message: format!("unknown semantic manifest field `{key}`"),
                });
            };
            reject_unknown(value, expected, path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_seed_hashes() {
        for (id, raw) in [
            (
                "iso4762",
                include_str!(
                    "../../onecad-app/resources/library-seed/onecad.std.iso4762/component.toml"
                ),
            ),
            (
                "iso4032",
                include_str!(
                    "../../onecad-app/resources/library-seed/onecad.std.iso4032/component.toml"
                ),
            ),
            (
                "iso7089",
                include_str!(
                    "../../onecad-app/resources/library-seed/onecad.std.iso7089/component.toml"
                ),
            ),
        ] {
            let pkg = parse(raw, Path::new(id)).unwrap();
            assert_eq!(revision(&pkg, None).unwrap(), pkg.identity.revision, "{id}");
        }
    }
    #[test]
    fn canonical_semantics_are_stable_and_sensitive() {
        let a = r#"[identity]
id='acme.part'
version='1.0.0'
revision='sha256:0000000000000000000000000000000000000000000000000000000000000000'
[metadata]
name='Part'
[source]
kind='generator'
generator='shape'
generator_version=1
[parameters]
length={role='free', value=5, min=1.0}
"#;
        let b = r#"[source]
generator_version = 1
generator = 'shape'
kind = 'generator'
[metadata]
name = 'Part'
[identity]
revision = 'sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
version = '1.0.0'
id = 'acme.part'
[parameters]
length = { min = 1.0, value = 5, role = 'free' }
"#;
        let pa = parse(a, Path::new("a")).unwrap();
        let pb = parse(b, Path::new("b")).unwrap();
        let original = revision(&pa, None).unwrap();
        assert_eq!(original, revision(&pb, None).unwrap());
        let mut changed = pa.clone();
        changed.metadata.unit = "inch".into();
        assert_ne!(original, revision(&changed, None).unwrap());
        let mut changed = pa.clone();
        changed.parameters.get_mut("length").unwrap().value = Some(toml::Value::Integer(6));
        assert_ne!(original, revision(&changed, None).unwrap());
        let mut changed = pa.clone();
        changed.parameters.get_mut("length").unwrap().key = Some("binding".into());
        assert_ne!(original, revision(&changed, None).unwrap());
        let mut changed = pa.clone();
        changed.source = SourceSpec::Generator {
            generator: "other".into(),
            generator_version: 1,
        };
        assert_ne!(original, revision(&changed, None).unwrap());
        let mut changed = pa.clone();
        changed.parameters.get_mut("length").unwrap().domain = Some(vec![toml::Value::Integer(5)]);
        assert_ne!(original, revision(&changed, None).unwrap());
        let mut changed = pa.clone();
        changed.configurations.insert(
            "small".into(),
            [("length".into(), toml::Value::Integer(5))]
                .into_iter()
                .collect(),
        );
        assert_ne!(original, revision(&changed, None).unwrap());
        let mut changed = pa.clone();
        changed.attachments.insert(
            "seat".into(),
            package::AttachmentSpec {
                on: "face:1".into(),
                accepts: vec!["plane".into()],
                frame: Some(package::AttachmentFrame {
                    origin: [1.0, 0.0, 0.0],
                    ..Default::default()
                }),
            },
        );
        assert_ne!(original, revision(&changed, None).unwrap());
        assert!(parse(
            &a.replace("name='Part'", "name='Part'\nunknown=1"),
            Path::new("a")
        )
        .is_err());
    }
}
