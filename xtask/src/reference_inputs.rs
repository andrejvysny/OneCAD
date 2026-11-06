//! Offline identity checks for immutable public inputs; no geometry acceptance.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Component, Path},
};

const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    files: Vec<Input>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    path: String,
    byte_length: u64,
    sha256: String,
    source_url: String,
    revision: String,
    license: String,
    units: String,
    intended_use: String,
    archive_member: Option<String>,
    geometry_oracle_status: Option<String>,
}

pub fn run(args: &[String]) -> crate::Result {
    if !args.is_empty() {
        return Err("usage: cargo xtask verify-reference-inputs".into());
    }
    let count = verify(Path::new("fixtures/public-cad"))?;
    println!(
        "Verified {count} pinned offline input files; geometry and native acceptance unqualified"
    );
    Ok(())
}

fn verify(root: &Path) -> crate::Result<usize> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
    if manifest.schema_version != 1 || manifest.files.is_empty() {
        return Err("unsupported or empty public-input manifest".into());
    }
    let root = fs::canonicalize(root)?;
    let mut paths = BTreeSet::new();
    for input in &manifest.files {
        if !paths.insert(input.path.clone()) {
            return Err(format!("duplicate public input: {}", input.path).into());
        }
        verify_input(&root, input)?;
    }
    let mut discovered = BTreeSet::new();
    discover(&root, &root, &mut discovered)?;
    discovered.remove("manifest.json");
    discovered.remove("README.md");
    if discovered != paths {
        return Err("public-input inventory differs from declared files".into());
    }
    Ok(paths.len())
}

fn discover(root: &Path, directory: &Path, files: &mut BTreeSet<String>) -> crate::Result {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            discover(root, &entry.path(), files)?;
        } else if kind.is_file() {
            files.insert(
                entry
                    .path()
                    .strip_prefix(root)?
                    .to_str()
                    .ok_or("non-UTF8 input path")?
                    .to_string(),
            );
        } else {
            return Err("public-input symlink or special file refused".into());
        }
    }
    Ok(())
}

fn verify_provenance(input: &Input) -> crate::Result {
    let path = Path::new(&input.path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || input.byte_length > MAX_INPUT_BYTES
        || !input.source_url.starts_with("https://")
        || input.revision.is_empty()
        || input.license.is_empty()
        || input.units.trim().is_empty()
        || input.intended_use.trim().is_empty()
    {
        return Err(format!("invalid public-input provenance: {}", input.path).into());
    }
    if input.revision.starts_with("archive-sha256:")
        && !input.archive_member.as_deref().is_some_and(|member| {
            !member.is_empty()
                && Path::new(member)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
        })
    {
        return Err("archive input requires a safe exact member path".into());
    }
    if matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("step" | "stp" | "stl")
    ) && input.geometry_oracle_status.as_deref() != Some("pending")
    {
        return Err(
            "pinned-only geometry input requires explicit pending geometry qualification".into(),
        );
    }
    Ok(())
}

fn verify_input(root: &Path, input: &Input) -> crate::Result {
    verify_provenance(input)?;
    let path = fs::canonicalize(root.join(&input.path))?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(format!("public input escapes fixture root: {}", input.path).into());
    }
    if fs::metadata(&path)?.len() != input.byte_length {
        return Err(format!("public input size mismatch: {}", input.path).into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != input.byte_length {
        return Err(format!("public input size changed during read: {}", input.path).into());
    }
    let digest = format!("{:x}", Sha256::digest(bytes));
    if digest != input.sha256 {
        return Err(format!("public input hash mismatch: {}", input.path).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let area = tempfile::tempdir().unwrap();
        fs::write(area.path().join("part.step"), b"original").unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "files": [{"path":"part.step", "byteLength":8,
                "sha256":format!("{:x}", Sha256::digest(b"original")),
                "sourceUrl":"https://example.invalid/pinned", "revision":"immutable",
                "license":"LicenseRef-test", "units":"mm", "intendedUse":"test identity",
                "geometryOracleStatus":"pending"}]
        });
        fs::write(area.path().join("manifest.json"), manifest.to_string()).unwrap();
        area
    }

    #[test]
    fn offline_inputs_fail_closed_after_mutation_or_omission() {
        let area = fixture();
        assert_eq!(verify(area.path()).unwrap(), 1);
        fs::write(area.path().join("part.step"), b"tampered").unwrap();
        assert!(verify(area.path())
            .unwrap_err()
            .to_string()
            .contains("hash mismatch"));
        fs::remove_file(area.path().join("part.step")).unwrap();
        assert!(verify(area.path()).is_err());
    }

    #[test]
    fn undeclared_file_and_oversized_admission_are_refused() {
        let area = fixture();
        fs::write(area.path().join("unadmitted.step"), b"unverified").unwrap();
        assert!(verify(area.path())
            .unwrap_err()
            .to_string()
            .contains("inventory"));
        fs::remove_file(area.path().join("unadmitted.step")).unwrap();
        let path = area.path().join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["files"][0]["byteLength"] = (MAX_INPUT_BYTES + 1).into();
        fs::write(path, manifest.to_string()).unwrap();
        assert!(verify(area.path())
            .unwrap_err()
            .to_string()
            .contains("invalid public-input provenance"));
    }

    #[test]
    fn future_duplicate_and_escaping_inputs_are_refused() {
        let area = fixture();
        let path = area.path().join("manifest.json");
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for kind in ["future", "duplicate", "escape"] {
            let mut changed = original.clone();
            match kind {
                "future" => changed["schemaVersion"] = 2.into(),
                "duplicate" => changed["files"]
                    .as_array_mut()
                    .unwrap()
                    .push(original["files"][0].clone()),
                _ => changed["files"][0]["path"] = "../part.step".into(),
            }
            fs::write(&path, changed.to_string()).unwrap();
            assert!(verify(area.path()).is_err(), "{kind}");
        }
    }

    #[test]
    fn missing_semantic_provenance_and_archive_member_are_refused() {
        let area = fixture();
        let path = area.path().join("manifest.json");
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for field in ["units", "intendedUse", "geometryOracleStatus"] {
            let mut changed = original.clone();
            changed["files"][0].as_object_mut().unwrap().remove(field);
            fs::write(&path, changed.to_string()).unwrap();
            assert!(verify(area.path()).is_err(), "{field}");
        }
        let mut changed = original;
        changed["files"][0]["revision"] = "archive-sha256:pinned".into();
        fs::write(&path, changed.to_string()).unwrap();
        assert!(verify(area.path()).is_err());
    }

    #[test]
    fn admitted_reference_inventory_is_exact_and_offline() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/public-cad");
        assert_eq!(verify(&root).unwrap(), 27);
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
        let expected: BTreeSet<_> = [
            "framework/ExpansionCard_Printable_M2.pdf",
            "framework/ExpansionCard_ThreadedInsert.stp",
            "framework/LICENSE",
            "framework/README.md",
            "nist/README.txt",
            "nist/nist_ftc_07_asme1_rd.pdf",
            "nist/nist_ftc_07_asme1_rd.stp",
            "nist/nist_ftc_08_asme1_rc.pdf",
            "nist/nist_ftc_08_asme1_rc.stp",
            "nist/nist_ftc_09_asme1_rd.pdf",
            "nist/nist_ftc_09_asme1_rd.stp",
            "nist/nist_ftc_10_asme1_rb.pdf",
            "nist/nist_ftc_10_asme1_rb.stp",
            "prusa/LICENSE",
            "prusa/MINI-fsenzor-box.stp",
            "prusa/MINI-fsenzor-cover.stp",
            "prusa/MINI-fsenzor-lever.stp",
            "prusa/README.md",
            "robonine/DOCS-LICENSE.txt",
            "robonine/LICENSES/Apache-2.0.txt",
            "robonine/LICENSES/CERN-OHL-P-2.0.txt",
            "robonine/LICENSING.md",
            "robonine/NOTICE",
            "robonine/REUSE.toml",
            "robonine/simulation/so_arm_101_description/meshes/visual/clamp_1.stl",
            "robonine/simulation/so_arm_101_description/meshes/visual/clamp_2.stl",
            "robonine/simulation/so_arm_101_description/urdf/so_101.urdf.xacro",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            manifest
                .files
                .iter()
                .map(|input| input.path.as_str())
                .collect::<BTreeSet<_>>(),
            expected
        );
        let step_count = manifest
            .files
            .iter()
            .filter(|input| input.path.ends_with(".stp"))
            .count();
        assert_eq!(step_count, 8);
        assert!(
            !root.join("robonine/RB9.01.062.000 Gripper.STEP").exists(),
            "quarantined mixed-vendor STEP is not admitted"
        );
    }
}
