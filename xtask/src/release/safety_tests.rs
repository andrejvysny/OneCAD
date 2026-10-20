use super::{materials::*, tests::fixture, verify};
use serde_json::{json, Value};
use std::{fs, process::Command};

fn reindex(root: &std::path::Path) {
    let entries = files(root)
        .unwrap()
        .into_iter()
        .filter(|n| n != "inventory.json")
        .map(|path| {
            let bytes = fs::read(root.join(&path)).unwrap();
            Entry {
                sha256: hash(&bytes),
                bytes: bytes.len() as u64,
                executable: executable(&root.join(&path)).unwrap(),
                path,
            }
        })
        .collect();
    save_json(
        &root.join("inventory.json"),
        &Inventory {
            schema_version: 1,
            files: entries,
        },
    )
    .unwrap();
}

#[test]
fn changed_manifest_duplicate_inventory_and_strict_metadata_rejected() {
    let fixture = fixture();
    fixture.prepare().unwrap();
    let root = fixture.payload();
    let inventory = fs::read(root.join("inventory.json")).unwrap();
    let mut parsed: Value = serde_json::from_slice(&inventory).unwrap();
    let first = parsed["files"][0].clone();
    parsed["files"].as_array_mut().unwrap().push(first);
    save_json(&root.join("inventory.json"), &parsed).unwrap();
    assert!(verify::directory(&root, fixture.path("--worker"))
        .unwrap_err()
        .to_string()
        .contains("inventory file set differs"));
    fs::write(root.join("inventory.json"), inventory).unwrap();
    fs::write(root.join("worker-manifest.json"), "{}").unwrap();
    assert!(verify::directory(&root, fixture.path("--worker"))
        .unwrap_err()
        .to_string()
        .contains("inventory hash, size or mode mismatch"));
}

#[test]
fn false_acceptance_claims_unknown_fields_and_invalid_dates_rejected() {
    let fixture = fixture();
    fixture.prepare().unwrap();
    let root = fixture.payload();
    let path = root.join("release-materials.json");
    let metadata: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for date in [
        "2026-02-30T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-13-01T00:00:00Z",
    ] {
        let mut changed = metadata.clone();
        changed["preparedAt"] = json!(date);
        save_json(&path, &changed).unwrap();
        reindex(&root);
        assert!(verify::directory(&root, fixture.path("--worker"))
            .unwrap_err()
            .to_string()
            .contains("invalid release materials metadata"));
    }
    let mut changed = metadata.clone();
    changed["validation"]["publication"] = json!("verified");
    save_json(&path, &changed).unwrap();
    reindex(&root);
    assert!(verify::directory(&root, fixture.path("--worker"))
        .unwrap_err()
        .to_string()
        .contains("invalid release materials metadata"));
    let mut changed = metadata;
    changed["unknown"] = json!(true);
    save_json(&path, &changed).unwrap();
    reindex(&root);
    assert!(verify::directory(&root, fixture.path("--worker"))
        .unwrap_err()
        .to_string()
        .contains("unknown field"));
}

#[test]
fn duplicate_archive_paths_and_unexpected_root_rejected_before_extraction() {
    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir(temporary.path().join("materials")).unwrap();
    fs::write(temporary.path().join("materials/file"), "data").unwrap();
    let archive = temporary.path().join("duplicate.tar.gz");
    execute(
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(temporary.path())
            .args(["materials", "materials/file"]),
    )
    .unwrap();
    assert!(archive_names(&archive)
        .unwrap_err()
        .to_string()
        .contains("duplicate archive path"));
    fs::write(temporary.path().join("unexpected"), "data").unwrap();
    execute(
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(temporary.path())
            .arg("unexpected"),
    )
    .unwrap();
    assert!(archive_names(&archive)
        .unwrap_err()
        .to_string()
        .contains("archive has unexpected root"));
}

#[cfg(unix)]
#[test]
fn cli_preserves_leaf_symlinks_so_required_inputs_reject_them() {
    use std::os::unix::fs::symlink;
    let fixture = fixture();
    let worker_link = fixture.path("--worker").with_file_name("worker-link");
    symlink(fixture.path("--worker"), &worker_link).unwrap();
    let args = [
        "--worker".to_owned(),
        worker_link.to_str().unwrap().to_owned(),
    ];
    let parsed = options(&args, &["--worker"]).unwrap();
    assert_eq!(parsed["--worker"], worker_link);
    assert!(required(&parsed["--worker"])
        .unwrap_err()
        .to_string()
        .contains("missing regular material"));
    let archive = fixture.prepare().unwrap();
    let args = [
        "--archive".to_owned(),
        archive.to_str().unwrap().to_owned(),
        "--worker".to_owned(),
        worker_link.to_str().unwrap().to_owned(),
    ];
    assert!(super::run("verify-release-materials", &args)
        .unwrap_err()
        .to_string()
        .contains("missing regular material"));
}
