//! Document access policy belongs to the host; services still own file mutations.
use crate::qa::QaPaths;
#[cfg(any(test, feature = "desktop"))]
use std::{ffi::OsString, sync::Arc};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathPolicy {
    Qa,
    Desktop,
    Sandbox,
}

impl QaPaths {
    #[cfg(feature = "desktop")]
    pub(crate) fn startup(args: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let args: Vec<_> = args.collect();
        if args.iter().any(|arg| arg == "--qa-root") {
            return Self::from_args(args.into_iter());
        }
        let home = std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .ok_or("HOME is required to locate the OneCAD desktop profile")?;
        Self::desktop_args(args, Path::new(&home), &std::env::temp_dir())
    }

    #[cfg(any(test, feature = "desktop"))]
    pub(crate) fn desktop_args(
        args: Vec<OsString>,
        home: &Path,
        temporary: &Path,
    ) -> Result<Self, String> {
        let document = match args.as_slice() {
            [] => None,
            [flag, path] if flag == "--open" => Some(PathBuf::from(path)),
            _ => return Err("desktop usage: onecad-ui [--open <document.onecad>]".into()),
        };
        Self::desktop_at(home, temporary, document.as_deref())
    }

    #[cfg(any(test, feature = "desktop"))]
    pub(crate) fn desktop_at(
        home: &Path,
        temporary: &Path,
        document: Option<&Path>,
    ) -> Result<Self, String> {
        if !home.is_absolute() || !temporary.is_absolute() {
            return Err("Desktop home and temporary directories must be absolute".into());
        }
        let home = home
            .canonicalize()
            .map_err(|error| format!("Desktop home: {error}"))?;
        let (support, caches, logs) = desktop_directories(&home);
        let root = directory(&support)?;
        let lock = profile_lock(&root)?;
        let documents = directory(&home.join("Documents"))?;
        let data = directory(&root.join("data"))?;
        let config = directory(&root.join("config"))?;
        directory(&caches)?;
        let temporary = directory(temporary)?;
        let document = document.map(canonical_document).transpose()?;
        Ok(Self {
            policy: PathPolicy::Desktop,
            logs,
            document: document
                .clone()
                .unwrap_or_else(|| documents.join("Untitled.onecad")),
            open_on_start: document.is_some(),
            root,
            documents,
            data,
            config,
            temporary,
            _lock: Some(Arc::new(lock)),
        })
    }

    pub(crate) fn target(&self, target: &Path) -> Result<PathBuf, String> {
        target_path(self.policy, &self.documents, target)
    }

    pub(crate) fn open_source(&self, source: &Path) -> Result<PathBuf, String> {
        if self.policy == PathPolicy::Desktop {
            canonical_document(source)
        } else {
            crate::document_services::copy_for_open(&self.documents, source)
        }
    }

    pub(crate) fn recovery_directory(&self) -> Result<PathBuf, String> {
        if self.policy == PathPolicy::Desktop {
            directory(&self.root.join("Recovered"))
        } else {
            self.documents
                .canonicalize()
                .map_err(|error| error.to_string())
        }
    }
}

pub(crate) fn target_path(
    policy: PathPolicy,
    documents: &Path,
    target: &Path,
) -> Result<PathBuf, String> {
    if !target.is_absolute() {
        return Err("Save and export require absolute paths".into());
    }
    if target
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("Save and export destinations must not be symlinks".into());
    }
    let parent = target
        .parent()
        .ok_or("Missing destination directory")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if policy != PathPolicy::Desktop
        && !parent.starts_with(
            documents
                .canonicalize()
                .map_err(|error| error.to_string())?,
        )
    {
        return Err("Save and export must remain inside the host documents directory".into());
    }
    if target.exists() && !target.is_file() {
        return Err("Save and export destinations must be files".into());
    }
    Ok(parent.join(target.file_name().ok_or("Missing destination filename")?))
}

fn canonical_document(path: &Path) -> Result<PathBuf, String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("onecad"))
    {
        return Err("Open requires an existing .onecad file".into());
    }
    Ok(path)
}

fn directory(path: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(path).map_err(|error| format!("{}: {error}", path.display()))?;
    path.canonicalize().map_err(|error| error.to_string())
}

#[cfg(any(test, feature = "desktop"))]
fn desktop_directories(home: &Path) -> (PathBuf, PathBuf, PathBuf) {
    if cfg!(target_os = "macos") {
        (
            home.join("Library/Application Support/OneCAD"),
            home.join("Library/Caches/OneCAD"),
            home.join("Library/Logs/OneCAD"),
        )
    } else {
        (
            home.join(".local/share/OneCAD"),
            home.join(".cache/OneCAD"),
            home.join(".local/state/OneCAD/logs"),
        )
    }
}

#[cfg(any(test, feature = "desktop"))]
fn profile_lock(root: &Path) -> Result<fs::File, String> {
    let path = root.join(".onecad-desktop.lock");
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("Desktop profile lock must not be a symlink".into());
    }
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| error.to_string())?;
    lock.try_lock()
        .map_err(|_| "OneCAD desktop profile is already in use by another process".to_string())?;
    Ok(lock)
}

#[cfg(test)]
#[path = "path_policy_tests.rs"]
mod tests;
