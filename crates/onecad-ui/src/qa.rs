//! QA paths are explicit and confined to a dedicated temporary root.
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

const MARKER: &str = ".onecad-native-qa";
const MARKER_CONTENT: &str = "onecad-native-qa-v1\n";

#[derive(Clone, Debug)]
pub struct QaPaths {
    pub root: PathBuf,
    pub document: PathBuf,
    pub data: PathBuf,
    pub config: PathBuf,
    pub temporary: PathBuf,
    _lock: Arc<fs::File>,
}

impl QaPaths {
    pub fn from_args(args: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let mut args = args;
        let (mut root, mut document) = (None, None);
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or("required: --qa-root <temporary-root> --open <fixture.onecad>")?;
            match flag.to_str() {
                Some("--qa-root") if root.is_none() => root = Some(PathBuf::from(value)),
                Some("--open") if document.is_none() => document = Some(PathBuf::from(value)),
                _ => return Err("only --qa-root and --open are supported, once each".into()),
            }
        }
        Self::prepare(
            &root.ok_or("--qa-root is required")?,
            &document.ok_or("--open is required")?,
        )
    }

    pub fn require_launch_environment(&self) -> Result<(), String> {
        let actual = fs::canonicalize(std::env::temp_dir()).map_err(|error| error.to_string())?;
        if actual != self.temporary {
            return Err(format!(
                "launch with TMPDIR={} before starting the process",
                self.temporary.display()
            ));
        }
        let logs = std::env::var_os("ONECAD_LOG_DIR")
            .ok_or("ONECAD_LOG_DIR must explicitly name the QA logs directory")?;
        if canonical_candidate(Path::new(&logs))? != self.root.join("logs") {
            return Err("ONECAD_LOG_DIR must stay inside the QA root".into());
        }
        if let Some(library) = std::env::var_os("ONECAD_LIBRARY_ROOT")
            && canonical_candidate(Path::new(&library))? != self.data.join("library")
        {
            return Err(
                "ONECAD_LIBRARY_ROOT must be unset or name the QA data/library directory".into(),
            );
        }
        Ok(())
    }

    fn prepare(root: &Path, fixture: &Path) -> Result<Self, String> {
        if fs::symlink_metadata(root).is_ok_and(|value| value.file_type().is_symlink()) {
            return Err("QA root must not be a symlink".into());
        }
        let root = canonical_candidate(root)?;
        require_temporary(&root)?;
        let fixture = fs::canonicalize(fixture).map_err(|error| error.to_string())?;
        require_temporary(&fixture)?;
        if fixture.extension().and_then(|value| value.to_str()) != Some("onecad")
            || !fixture.is_file()
        {
            return Err(
                "--open must name a prepared .onecad fixture under temporary storage".into(),
            );
        }
        validate_existing_root(&root)?;
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let lock_path = root.join(".onecad-native-qa.lock");
        reject_link(&lock_path)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|error| error.to_string())?;
        lock.try_lock()
            .map_err(|_| "QA root is already in use by another host".to_string())?;
        let lock = Arc::new(lock);
        for name in [MARKER, "data", "config", "temporary", "documents", "logs"] {
            reject_link(&root.join(name))?;
        }
        fs::write(root.join(MARKER), MARKER_CONTENT).map_err(|error| error.to_string())?;
        for directory in ["data", "config", "temporary", "documents", "logs"] {
            let path = root.join(directory);
            fs::create_dir_all(&path).map_err(|error| error.to_string())?;
            if !fs::canonicalize(&path)
                .map_err(|error| error.to_string())?
                .starts_with(&root)
            {
                return Err("QA directory escapes the dedicated root".into());
            }
        }
        let document = copy_fixture(&root, &fixture)?;
        Ok(Self {
            data: root.join("data"),
            config: root.join("config"),
            temporary: root.join("temporary"),
            root,
            document,
            _lock: lock,
        })
    }
}

fn reject_link(path: &Path) -> Result<(), String> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(format!("QA path must not be a symlink: {}", path.display()));
    }
    Ok(())
}

fn validate_existing_root(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    reject_link(&root.join(MARKER))?;
    let entries = fs::read_dir(root).map_err(|error| error.to_string())?;
    if entries.count() > 0
        && fs::read_to_string(root.join(MARKER)).ok().as_deref() != Some(MARKER_CONTENT)
    {
        return Err("existing QA root lacks the native QA marker; refusing shared data".into());
    }
    Ok(())
}

fn copy_fixture(root: &Path, fixture: &Path) -> Result<PathBuf, String> {
    if fixture.starts_with(root.join("documents")) {
        return Ok(fixture.to_owned());
    }
    let name = fixture.file_name().ok_or("fixture lacks a filename")?;
    let output = root.join("documents").join(name);
    if output.exists() {
        return Err(format!(
            "QA copy already exists; reopen it explicitly with --open {}",
            output.display()
        ));
    }
    fs::copy(fixture, &output).map_err(|error| error.to_string())?;
    Ok(output)
}

fn canonical_candidate(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("QA paths must be absolute".into());
    }
    if path.exists() {
        return fs::canonicalize(path).map_err(|error| error.to_string());
    }
    let parent = canonical_candidate(path.parent().ok_or("invalid QA root")?)?;
    Ok(parent.join(path.file_name().ok_or("invalid QA root name")?))
}

fn require_temporary(path: &Path) -> Result<(), String> {
    for root in [std::env::temp_dir(), PathBuf::from("/private/tmp")] {
        if let Ok(root) = fs::canonicalize(root)
            && path != root
            && path.starts_with(root)
        {
            return Ok(());
        }
    }
    Err(
        "QA paths must be dedicated children of OS temporary storage; production roots are refused"
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_isolation_and_reopen_preserve_saved_copy() {
        let area = tempfile::tempdir().unwrap();
        let fixture = area.path().join("plate.onecad");
        fs::write(&fixture, b"synthetic fixture").unwrap();
        let root = area.path().join("qa");
        let paths = QaPaths::prepare(&root, &fixture).unwrap();
        assert!(paths.document.starts_with(fs::canonicalize(&root).unwrap()));
        fs::write(&paths.document, b"saved edit").unwrap();
        assert!(QaPaths::prepare(&root, &fixture).is_err());
        let saved_path = paths.document.clone();
        drop(paths);
        let reopened = QaPaths::prepare(&root, &saved_path).unwrap();
        assert_eq!(fs::read(reopened.document).unwrap(), b"saved edit");
        assert_eq!(fs::read(fixture).unwrap(), b"synthetic fixture");
    }
    #[test]
    fn production_and_unmarked_shared_roots_are_refused() {
        let area = tempfile::tempdir().unwrap();
        let fixture = area.path().join("plate.onecad");
        fs::write(&fixture, b"synthetic fixture").unwrap();
        assert!(QaPaths::prepare(Path::new("/Users/Shared/OneCAD"), &fixture).is_err());
        let root = area.path().join("shared");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("existing"), b"preserve").unwrap();
        assert!(QaPaths::prepare(&root, &fixture).is_err());
        assert_eq!(fs::read(root.join("existing")).unwrap(), b"preserve");
    }
}
