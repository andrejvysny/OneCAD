use super::{materials::*, Result};
use std::{collections::BTreeSet, fs, path::Path, process::Command};

pub fn run(args: &[String]) -> Result<()> {
    let options = options(args, &["--archive", "--worker"])?;
    archive(&options["--archive"], &options["--worker"])?;
    println!("Release material hashes and required files verified; build, relink and publication remain unverified.");
    Ok(())
}

pub fn directory(root: &Path, worker: &Path) -> Result<()> {
    check_inventory(root)?;
    let metadata: Metadata =
        serde_json::from_slice(&required(&root.join("release-materials.json"))?)?;
    metadata.validate()?;
    let manifest_bytes = required(&root.join("worker-manifest.json"))?;
    let manifest = worker_manifest(&manifest_bytes)?;
    if hash(&required(worker)?) != metadata.worker.binary_sha256
        || manifest["binarySha256"] != metadata.worker.binary_sha256
        || hash(&manifest_bytes) != metadata.worker.manifest_sha256
    {
        return Err("worker or worker manifest SHA-256 differs from release materials".into());
    }
    let source = root.join("source");
    check_source(&source)?;
    if files(&source)?.len() != metadata.source.file_count {
        return Err("source file count differs".into());
    }
    check_occt(
        &source,
        &metadata.occt,
        &root.join("occt/OCCT-V8_0_1.tar.gz"),
    )?;
    check_cmake(&metadata.occt, &root.join("occt/occt-build.cmake"))?;
    let recorded: Occt = serde_json::from_slice(&required(&root.join("occt/occt-build.json"))?)?;
    if recorded != metadata.occt {
        return Err("OCCT metadata differs from inventory".into());
    }
    check_evidence(root, metadata.source.dirty)
}

fn check_inventory(root: &Path) -> Result<()> {
    let inventory: Inventory = serde_json::from_slice(&required(&root.join("inventory.json"))?)?;
    if inventory.schema_version != 1 {
        return Err("invalid inventory schema version".into());
    }
    let actual: BTreeSet<_> = files(root)?
        .into_iter()
        .filter(|n| n != "inventory.json")
        .collect();
    let listed: BTreeSet<_> = inventory.files.iter().map(|e| e.path.clone()).collect();
    if listed.len() != inventory.files.len() || listed != actual {
        return Err("inventory file set differs from payload".into());
    }
    for entry in inventory.files {
        safe_path(&entry.path)?;
        digest(&entry.sha256)?;
        let path = root.join(&entry.path);
        let bytes = fs::read(&path)?;
        if hash(&bytes) != entry.sha256
            || bytes.len() as u64 != entry.bytes
            || executable(&path)? != entry.executable
        {
            return Err(format!("inventory hash, size or mode mismatch: {}", entry.path).into());
        }
    }
    Ok(())
}

fn check_evidence(root: &Path, dirty: bool) -> Result<()> {
    for name in [
        "RELINK.md",
        "occt/occt-build.cmake",
        "licenses/OCCT-LICENSE_LGPL_21.txt",
        "licenses/OCCT-LGPL_EXCEPTION.txt",
        "licenses/Boost-LICENSE_1_0.txt",
        "dependencies/eigen-version.cmake",
        "dependencies/boost-version.hpp",
        "licenses/Eigen-COPYING.README",
        "licenses/Eigen-COPYING.MPL2",
        "licenses/Eigen-COPYING.BSD",
        "licenses/Eigen-COPYING.APACHE",
        "licenses/Eigen-COPYING.MINPACK",
    ] {
        required(&root.join(name))?;
    }
    // A clean snapshot legitimately has empty status and patch files.
    for name in ["source-status.z", "source-changes.patch"] {
        if !fs::symlink_metadata(root.join(name)).is_ok_and(|m| m.is_file()) {
            return Err(format!("missing source evidence: {name}").into());
        }
    }
    if !fs::read(root.join("source-status.z"))?.is_empty() != dirty {
        return Err("dirty state differs from source status evidence".into());
    }
    Ok(())
}

pub fn archive(archive: &Path, worker: &Path) -> Result<()> {
    archive_names(archive)?;
    let temporary = tempfile::Builder::new()
        .prefix("onecad-materials-verify-")
        .tempdir()?;
    execute(
        Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(temporary.path()),
    )?;
    directory(&temporary.path().join("materials"), worker)
}
