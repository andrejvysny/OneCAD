use super::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

pub const REQUIRED_SOURCE: &[&str] = &[
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "THIRD_PARTY_NOTICES",
    "ATTRIBUTION.md",
    "Cargo.toml",
    "Cargo.lock",
    "xtask/Cargo.toml",
    "crates/onecad-ui/Cargo.toml",
    "crates/onecad-extension-api/Cargo.toml",
    "crates/onecad-extension-api/src/lib.rs",
    "crates/onecad-extension-api/wit/extension.wit",
    "crates/onecad-extension-host/Cargo.toml",
    "crates/onecad-extension-sdk/Cargo.toml",
    "crates/onecad-extension-sdk/src/lib.rs",
    "crates/onecad-extension-sdk/src/bin/onecad-addon-build.rs",
    "crates/onecad-extension-sdk/README.md",
    "addons/bambu-studio/Cargo.toml",
    "addons/bambu-studio/manifest.json",
    "addons/bambu-studio/src/lib.rs",
    "addons/bambu-studio/README.md",
    "assets/native/tokens.css",
    "worker/CMakeLists.txt",
    "worker/cmake/Dependencies.cmake",
    "worker/third_party/planegcs/LICENSE",
    "worker/third_party/planegcs/VENDOR.txt",
    "worker/third_party/planegcs/GCS.cpp",
    "worker/third_party/planegcs/CMakeLists.txt",
    "worker/third_party/nlohmann/json.hpp",
    "worker/third_party/nlohmann/json_fwd.hpp",
    "worker/third_party/nlohmann/VENDOR.txt",
    "scripts/build-worker.sh",
    "scripts/build-addons.sh",
    "scripts/build-pinned-occt.sh",
    "scripts/occt-fingerprint.sh",
    "scripts/package-macos.sh",
    "scripts/bundle-dylibs.sh",
    "scripts/macos-signing.sh",
    "scripts/macos-minimum.sh",
    "packaging/macos/host-entitlements.plist",
    "docs/PACKAGING.md",
];

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn required(path: &Path) -> Result<Vec<u8>> {
    if !fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        return Err(format!("missing regular material: {}", path.display()).into());
    }
    let bytes = fs::read(path)?;
    if bytes.is_empty() {
        return Err(format!("empty material: {}", path.display()).into());
    }
    Ok(bytes)
}

pub fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().ok_or("missing parent directory")?)?;
    fs::write(path, bytes)?;
    Ok(())
}

pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    save(path, &bytes)
}

pub fn execute(command: &mut Command) -> Result<Vec<u8>> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "{:?} failed: {}",
            command.get_program(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(output.stdout)
}

pub fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    execute(Command::new("git").arg("-C").arg(root).args(args))
}

pub fn safe_path(name: &str) -> Result<()> {
    if name.is_empty()
        || name.bytes().any(|b| b < 32 || b == 127 || b == b'\\')
        || name.starts_with('/')
        || name.ends_with('/')
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("unsafe material path: {name:?}").into());
    }
    Ok(())
}

pub fn files(root: &Path) -> Result<Vec<String>> {
    fn visit(root: &Path, dir: &Path, output: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let name = path
                .strip_prefix(root)?
                .to_str()
                .ok_or("non-UTF-8 material path")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            safe_path(&name)?;
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                visit(root, &path, output)?;
            } else if metadata.is_file() {
                output.push(name);
            } else {
                return Err(
                    format!("unsupported material type (including symlink): {name}").into(),
                );
            }
        }
        Ok(())
    }
    let mut output = Vec::new();
    visit(root, root, &mut output)?;
    output.sort();
    Ok(output)
}

fn absolute(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => (),
            other => result.push(other),
        }
    }
    Ok(result)
}

pub fn canonical(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => (),
            other => result.push(other),
        }
        if fs::symlink_metadata(&result).is_ok() {
            result = fs::canonicalize(result)?;
        }
    }
    Ok(result)
}

pub fn options(args: &[String], required: &[&str]) -> Result<BTreeMap<String, PathBuf>> {
    let mut output = BTreeMap::new();
    for pair in args.chunks(2) {
        if pair.len() != 2
            || !required.contains(&pair[0].as_str())
            || pair[1].starts_with("--")
            || pair[1].is_empty()
            || output.contains_key(&pair[0])
        {
            return Err(format!("expected options: {}", required.join(" ")).into());
        }
        output.insert(pair[0].clone(), absolute(Path::new(&pair[1]))?);
    }
    for name in required {
        if !output.contains_key(*name) {
            return Err(format!("required option: {name}").into());
        }
    }
    Ok(output)
}

pub fn digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid SHA-256 digest".into());
    }
    Ok(())
}

pub fn check_source(root: &Path) -> Result<()> {
    for name in REQUIRED_SOURCE {
        required(&root.join(name))?;
    }
    for name in [
        "crates/onecad-app/src",
        "crates/onecad-ui/src",
        "crates/onecad-render/src",
        "crates/onecad-extension-host/src",
        "xtask/src",
        "worker/src",
        "worker/third_party/planegcs",
    ] {
        if !root.join(name).is_dir() || files(&root.join(name))?.is_empty() {
            return Err(format!("missing source directory: {name}").into());
        }
    }
    Ok(())
}

pub fn executable(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(fs::symlink_metadata(path)?.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = fs::symlink_metadata(path)?;
        Ok(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Occt {
    pub schema_version: u32,
    pub version: String,
    pub tag: String,
    pub source_commit: String,
    pub archive_sha256: String,
    pub build_id: String,
    pub kernel_policy_version: u32,
    pub normalized_build_options: String,
}

impl Occt {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.version != "8.0.1"
            || self.tag != "V8_0_1"
            || self.source_commit != "b8f597c677811d1f9f4d8a97f5ae2825c0353a42"
            || self.kernel_policy_version != 1
            || self.normalized_build_options.is_empty()
            || !Regex::new(r"^[A-Za-z0-9._+-]+$")?.is_match(&self.build_id)
        {
            return Err("invalid OCCT metadata".into());
        }
        digest(&self.archive_sha256)
    }
}

pub fn occt_pin(root: &Path) -> Result<(String, String)> {
    let script = String::from_utf8(required(&root.join("scripts/build-pinned-occt.sh"))?)?;
    let branch = Regex::new(r"(?s)8\.0\.1 \| V8_0_1\)(.*?);;")?
        .captures(&script)
        .and_then(|m| m.get(1))
        .ok_or("missing pinned OCCT 8.0.1 policy in source snapshot")?
        .as_str();
    let archive = Regex::new(r#"ARCHIVE_SHA256="([0-9a-f]{64})""#)?
        .captures(branch)
        .and_then(|m| m.get(1))
        .ok_or("missing pinned OCCT archive hash")?
        .as_str()
        .to_owned();
    let options = Regex::new(r#"readonly NORMALIZED_OPTIONS="([^"]+)""#)?
        .captures(&script)
        .and_then(|m| m.get(1))
        .ok_or("missing pinned OCCT build options")?
        .as_str()
        .to_owned();
    if !branch.contains("b8f597c677811d1f9f4d8a97f5ae2825c0353a42") {
        return Err("missing pinned OCCT source commit".into());
    }
    Ok((archive, options))
}

pub fn check_occt(root: &Path, metadata: &Occt, archive: &Path) -> Result<()> {
    metadata.validate()?;
    let (archive_hash, options) = occt_pin(root)?;
    if metadata.archive_sha256 != archive_hash || hash(&required(archive)?) != archive_hash {
        return Err("OCCT archive SHA-256 differs from pinned build policy".into());
    }
    if metadata.normalized_build_options != options {
        return Err("OCCT build options differ from pinned policy".into());
    }
    Ok(())
}

pub fn check_cmake(metadata: &Occt, file: &Path) -> Result<()> {
    let cmake = String::from_utf8(required(file)?)?;
    for (key, value) in [
        ("ONECAD_OCCT_SOURCE_COMMIT", &metadata.source_commit),
        (
            "ONECAD_OCCT_NORMALIZED_BUILD_OPTIONS",
            &metadata.normalized_build_options,
        ),
        ("ONECAD_OCCT_BUILT_ID", &metadata.build_id),
    ] {
        let pattern = format!(r#"(?m)^set\({key} "([^"\n]+)"\)$"#);
        let matches: Vec<_> = Regex::new(&pattern)?
            .captures_iter(&cmake)
            .map(|m| m[1].to_string())
            .collect();
        if matches != [value.clone()] {
            return Err(format!("OCCT CMake metadata differs: {key}").into());
        }
    }
    Ok(())
}

pub fn archive_names(archive: &Path) -> Result<()> {
    let names = String::from_utf8(execute(Command::new("tar").arg("-tzf").arg(archive))?)?;
    let mut seen = BTreeSet::new();
    for raw in names.lines() {
        let name = raw.strip_suffix('/').unwrap_or(raw);
        safe_path(name)?;
        if name != "materials" && !name.starts_with("materials/") {
            return Err("archive has unexpected root".into());
        }
        if !seen.insert(name) {
            return Err(format!("duplicate archive path: {name}").into());
        }
    }
    let listing = execute(Command::new("tar").arg("-tvzf").arg(archive))?;
    if String::from_utf8(listing)?
        .lines()
        .any(|line| !line.starts_with('-') && !line.starts_with('d'))
    {
        return Err("archive contains unsupported links or special files".into());
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    pub schema_version: u32,
    pub prepared_at: String,
    pub scope: String,
    pub source: Source,
    pub worker: Worker,
    pub occt: Occt,
    pub dependencies: Dependencies,
    pub validation: Validation,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub revision: String,
    pub dirty: bool,
    pub exact_tags: Vec<String>,
    pub snapshot_policy: String,
    pub file_count: usize,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Worker {
    pub binary_sha256: String,
    pub manifest_sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependencies {
    pub eigen: String,
    pub boost: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Validation {
    pub build_provenance: String,
    pub relink: String,
    pub publication: String,
    pub transitive_license_inventory: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Inventory {
    pub schema_version: u32,
    pub files: Vec<Entry>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub executable: bool,
}

impl Metadata {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.scope != "local-source-material-preparation"
            || !Regex::new(r"^[0-9a-f]{40,64}$")?.is_match(&self.source.revision)
            || self.source.snapshot_policy != "tracked-and-untracked-nonignored"
            || self.source.file_count == 0
            || !datetime(&self.prepared_at)
            || self.dependencies.eigen.is_empty()
            || self.dependencies.boost.is_empty()
            || self.validation.build_provenance != "unverified"
            || self.validation.relink != "unverified"
            || self.validation.publication != "unverified"
            || self.validation.transitive_license_inventory != "incomplete"
        {
            return Err("invalid release materials metadata".into());
        }
        digest(&self.worker.binary_sha256)?;
        digest(&self.worker.manifest_sha256)?;
        self.occt.validate()
    }
}

pub fn worker_manifest(bytes: &[u8]) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value["manifestVersion"] != 1
        || value["occt"]["version"] != "8.0.1"
        || value["occt"]["fingerprint"]
            .as_str()
            .is_none_or(str::is_empty)
    {
        return Err("invalid worker manifest".into());
    }
    digest(
        value["binarySha256"]
            .as_str()
            .ok_or("missing worker hash")?,
    )?;
    Ok(value)
}

fn datetime(value: &str) -> bool {
    let Ok(pattern) = Regex::new(
        r"^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})(\.[0-9]+)?Z$",
    ) else {
        return false;
    };
    let Some(parts) = pattern.captures(value) else {
        return false;
    };
    let fields: Vec<u32> = (1..=6)
        .map(|i| parts[i].parse().unwrap_or(u32::MAX))
        .collect();
    let (year, month, day) = (fields[0], fields[1], fields[2]);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return false,
    };
    (1..=days).contains(&day) && fields[3] < 24 && fields[4] < 60 && fields[5] < 60
}
