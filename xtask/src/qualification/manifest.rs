use super::campaign::REQUIRED;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Read, path::Path, process::Command};

fn git(args: &[&str]) -> crate::Result<String> {
    let output = Command::new("git").args(args).output()?;
    if !output.status.success() {
        return Err("cannot capture source provenance".into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

pub(super) fn source() -> crate::Result<Value> {
    let untracked = git(&["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut files = BTreeMap::new();
    for path in untracked.split('\0').filter(|path| !path.is_empty()) {
        files.insert(path, digest(Path::new(path))?);
    }
    Ok(json!({"commit":git(&["rev-parse", "HEAD"])?.trim(),
        "tree":git(&["rev-parse", "HEAD^{tree}"])?.trim(),
        "status":git(&["status", "--porcelain=v1", "--untracked-files=all"])?,
        "trackedDiffSha256":format!("{:x}",Sha256::digest(git(&["diff", "HEAD", "--binary"])?.as_bytes())),
        "untrackedFiles":files}))
}

fn digest(path: &Path) -> crate::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn artifacts(root: &Path) -> crate::Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    collect(root, root, &mut result)?;
    result.remove("campaign.json");
    Ok(result)
}

fn collect(root: &Path, dir: &Path, result: &mut BTreeMap<String, String>) -> crate::Result {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(root, &entry.path(), result)?;
        } else if kind.is_file() {
            result.insert(
                entry
                    .path()
                    .strip_prefix(root)?
                    .to_str()
                    .ok_or("non-UTF8 evidence path")?
                    .into(),
                digest(&entry.path())?,
            );
        } else if kind.is_symlink() {
            let target = fs::read_link(entry.path())?;
            if target.is_absolute() || fs::canonicalize(entry.path())?.strip_prefix(root).is_err() {
                return Err("evidence symlink escapes campaign".into());
            }
            result.insert(
                entry
                    .path()
                    .strip_prefix(root)?
                    .to_str()
                    .ok_or("non-UTF8 evidence path")?
                    .into(),
                format!("symlink:{}", target.display()),
            );
        } else {
            return Err("special evidence file refused".into());
        }
    }
    Ok(())
}

pub(super) fn verify(root: &Path) -> crate::Result {
    let evidence: Value = serde_json::from_slice(&fs::read(root.join("campaign.json"))?)?;
    let phases = evidence["phases"]
        .as_array()
        .ok_or("missing campaign phases")?;
    let names: Vec<_> = phases
        .iter()
        .filter_map(|phase| phase["name"].as_str())
        .collect();
    if evidence["schemaVersion"] != 1
        || evidence["status"] != "passed"
        || names != REQUIRED
        || phases.iter().any(|phase| phase["passed"] != true)
    {
        return Err("qualification incomplete: every required phase must pass exactly once".into());
    }
    if evidence["source"] != source()? {
        return Err("qualification source differs from current checkout".into());
    }
    if evidence["artifacts"] != json!(artifacts(root)?) {
        return Err("qualification artifact inventory or hashes changed".into());
    }
    let children: Value = serde_json::from_slice(&fs::read(root.join("nested-children.json"))?)?;
    if children != super::nested::verify(&root.join("native"))? {
        return Err("fresh-process child evidence changed".into());
    }
    let metal: Value = serde_json::from_slice(&fs::read(root.join("metal-inventory.json"))?)?;
    if metal != super::inventories::metal(root)? {
        return Err("Metal inventory evidence changed".into());
    }
    let ctest: Value = serde_json::from_slice(&fs::read(root.join("ctest-inventory.json"))?)?;
    let workspace: Value =
        serde_json::from_slice(&fs::read(root.join("workspace-inventory.json"))?)?;
    if ctest != super::inventories::ctest(root)?
        || workspace != super::inventories::workspace(root)?
    {
        return Err("workspace or CTest inventory evidence changed".into());
    }
    for name in REQUIRED {
        let command: Value =
            serde_json::from_slice(&fs::read(root.join(format!("{name}.command.json")))?)?;
        if command["status"] != 0 || !command["signal"].is_null() {
            return Err(format!("qualification command did not succeed: {name}").into());
        }
    }
    println!("Automated campaign verified; physical input and latest-platform acceptance remain separate");
    Ok(())
}
