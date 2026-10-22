use crate::{api, HostError, Result};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) const MAX_COMPONENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PACKAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILES: usize = 128;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

pub(crate) struct PackageContents {
    pub manifest: api::Manifest,
    pub fingerprint: String,
    pub component: Vec<u8>,
    files: Vec<(PathBuf, Vec<u8>)>,
}

pub(crate) fn read_package(root: &Path) -> Result<PackageContents> {
    let root = std::path::absolute(root)?;
    let mut files = Vec::new();
    let parent = root.parent().unwrap_or_else(|| Path::new("."));
    let parent = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
    let name = root
        .file_name()
        .ok_or_else(|| invalid("package needs a directory name"))?;
    let directory = parent.open_dir_nofollow(name)?;
    collect(&directory, Path::new(""), &mut files, &mut 0, &mut 0)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let manifest_bytes = files
        .iter()
        .find(|(p, _)| p == Path::new(api::MANIFEST_FILE))
        .map(|(_, bytes)| bytes)
        .ok_or_else(|| invalid("missing manifest.json"))?;
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(invalid("manifest exceeds 64 KiB"));
    }
    let manifest: api::Manifest = serde_json::from_slice(manifest_bytes)?;
    manifest.validate()?;
    let component = files
        .iter()
        .find(|(p, _)| p == Path::new(&manifest.entry))
        .map(|(_, bytes)| bytes.clone())
        .ok_or_else(|| invalid("missing entry file"))?;
    if component.len() > MAX_COMPONENT_BYTES {
        return Err(invalid("component exceeds 16 MiB"));
    }
    if manifest.runtime == api::ExtensionRuntime::PortableComponent
        && !component.starts_with(b"\0asm")
    {
        return Err(invalid("entry is not WebAssembly binary"));
    }
    let fingerprint = fingerprint(&files);
    Ok(PackageContents {
        manifest,
        fingerprint,
        component,
        files,
    })
}

fn collect(
    directory: &Dir,
    relative: &Path,
    files: &mut Vec<(PathBuf, Vec<u8>)>,
    total: &mut usize,
    entries: &mut usize,
) -> Result<()> {
    if relative.components().count() > 8 {
        return Err(invalid("package directory nesting exceeds limit"));
    }
    for entry in directory.entries()? {
        let entry = entry?;
        *entries += 1;
        if *entries > 256 {
            return Err(invalid("package entry limit exceeded"));
        }
        let name = entry.file_name();
        let portable = name
            .to_str()
            .ok_or_else(|| invalid("package paths must be UTF-8"))?;
        if portable.contains('\\') || portable.chars().any(char::is_control) {
            return Err(invalid("package paths must be portable filenames"));
        }
        let path = relative.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(
                &directory.open_dir_nofollow(&name)?,
                &path,
                files,
                total,
                entries,
            )?;
        } else if kind.is_file() {
            read_file(directory, &name, path, files, total)?;
        } else {
            return Err(invalid("package contains a symbolic link or special file"));
        }
    }
    Ok(())
}

fn read_file(
    directory: &Dir,
    name: &std::ffi::OsStr,
    path: PathBuf,
    files: &mut Vec<(PathBuf, Vec<u8>)>,
    total: &mut usize,
) -> Result<()> {
    if files.len() >= MAX_FILES {
        return Err(invalid("package file limit exceeded"));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    let file = directory.open_with(name, &options)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_PACKAGE_BYTES as u64 {
        return Err(invalid("package contains a special or oversized file"));
    }
    // The nofollow/nonblock handle prevents link/FIFO swaps during inspection.
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take((MAX_PACKAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    *total += bytes.len();
    if *total > MAX_PACKAGE_BYTES {
        return Err(invalid("package exceeds 32 MiB"));
    }
    files.push((path, bytes));
    Ok(())
}

fn fingerprint(files: &[(PathBuf, Vec<u8>)]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"onecad-package-v1\0");
    for (path, bytes) in files {
        let normalized = path.to_string_lossy().replace('\\', "/");
        hash.update((normalized.len() as u64).to_le_bytes());
        hash.update(normalized.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}

impl PackageContents {
    pub fn copy_into(&self, destination: &Path) -> Result<()> {
        fs::create_dir(destination)?;
        let result = self.write_files(destination);
        if result.is_err() {
            let _ = fs::remove_dir_all(destination);
        }
        result
    }

    fn write_files(&self, destination: &Path) -> Result<()> {
        for (relative, bytes) in &self.files {
            let target = destination.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(target, bytes)?;
        }
        Ok(())
    }
}

fn invalid(message: &str) -> HostError {
    HostError::InvalidPackage(message.to_owned())
}
