//! Host directories with explicit desktop, isolated QA and sandbox policies.
use crate::path_policy::PathPolicy;
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
    pub(crate) policy: PathPolicy,
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    pub(crate) logs: PathBuf,
    pub root: PathBuf,
    /// Where documents are opened, saved and exported (`root/documents` for QA).
    pub documents: PathBuf,
    pub document: PathBuf,
    pub open_on_start: bool,
    pub data: PathBuf,
    pub config: PathBuf,
    pub temporary: PathBuf,
    pub(crate) _lock: Option<Arc<fs::File>>,
}

/// Container directories of a sandboxed host, as absolute paths.
#[derive(Clone, Debug)]
pub struct HostLayout {
    pub documents: PathBuf,
    pub app_support: PathBuf,
    pub caches: PathBuf,
    pub temporary: PathBuf,
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
        Self::prepare_optional(&root.ok_or("--qa-root is required")?, document.as_deref())
    }

    pub fn require_launch_environment(&self) -> Result<(), String> {
        if self.policy != PathPolicy::Qa {
            return Ok(());
        }
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

    /// Paths inside an OS-provided app container. The OS already isolates the
    /// container, so neither the QA marker, lock nor launch environment applies.
    /// Service data and settings live under Application Support.
    pub fn sandbox(layout: &HostLayout) -> Result<Self, String> {
        let support = sandbox_dir(&layout.app_support)?;
        let documents = sandbox_dir(&layout.documents)?;
        sandbox_dir(&layout.caches)?;
        let temporary = sandbox_dir(&layout.temporary)?;
        let data = sandbox_dir(&support.join("data"))?;
        let config = sandbox_dir(&support.join("config"))?;
        Ok(Self {
            policy: PathPolicy::Sandbox,
            logs: support.join("logs"),
            document: documents.join("Untitled.onecad"),
            root: support,
            documents,
            open_on_start: false,
            data,
            config,
            temporary,
            _lock: None,
        })
    }

    #[cfg(test)]
    fn prepare(root: &Path, fixture: &Path) -> Result<Self, String> {
        Self::prepare_optional(root, Some(fixture))
    }

    fn prepare_optional(root: &Path, fixture: Option<&Path>) -> Result<Self, String> {
        if fs::symlink_metadata(root).is_ok_and(|value| value.file_type().is_symlink()) {
            return Err("QA root must not be a symlink".into());
        }
        let root = canonical_candidate(root)?;
        require_temporary(&root)?;
        let fixture = fixture
            .map(|path| -> Result<PathBuf, String> {
                let path = fs::canonicalize(path).map_err(|error| error.to_string())?;
                require_temporary(&path)?;
                if path.extension().and_then(|value| value.to_str()) != Some("onecad")
                    || !path.is_file()
                {
                    return Err(
                        "--open must name a prepared .onecad fixture under temporary storage"
                            .into(),
                    );
                }
                Ok(path)
            })
            .transpose()?;
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
        let open_on_start = fixture.is_some();
        let document = match fixture {
            Some(fixture) => copy_fixture(&root, &fixture)?,
            None => root.join("documents/Untitled.onecad"),
        };
        Ok(Self {
            policy: PathPolicy::Qa,
            logs: root.join("logs"),
            data: root.join("data"),
            config: root.join("config"),
            temporary: root.join("temporary"),
            documents: root.join("documents"),
            root,
            document,
            open_on_start,
            _lock: Some(lock),
        })
    }
}

/// Creates an absolute container directory and returns its canonical path, so
/// confinement checks compare resolved paths (`/var` is a link on iOS).
fn sandbox_dir(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("sandbox path must be absolute: {}", path.display()));
    }
    fs::create_dir_all(path).map_err(|error| format!("{}: {error}", path.display()))?;
    fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))
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
    fn root_without_fixture_prepares_blank_start_without_creating_document() {
        let area = tempfile::tempdir().unwrap();
        let root = area.path().join("qa");
        let paths =
            QaPaths::from_args([OsString::from("--qa-root"), root.into_os_string()].into_iter())
                .unwrap();
        assert!(!paths.open_on_start);
        assert!(!paths.document.exists());
        assert!(paths.root.join("documents").is_dir());
    }
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
    fn sandbox_layout_uses_container_directories_without_qa_marker() {
        let area = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(area.path()).unwrap();
        let layout = HostLayout {
            documents: base.join("Documents"),
            app_support: base.join("Library/Application Support"),
            caches: base.join("Library/Caches"),
            temporary: base.join("tmp"),
        };
        let paths = QaPaths::sandbox(&layout).unwrap();
        assert_eq!(paths.documents, layout.documents);
        assert_eq!(paths.data, layout.app_support.join("data"));
        assert_eq!(paths.config, layout.app_support.join("config"));
        assert!(paths.document.starts_with(&paths.documents));
        assert!(!paths.open_on_start && layout.caches.is_dir());
        assert!(!layout.app_support.join(MARKER).exists());
        assert!(
            QaPaths::sandbox(&HostLayout {
                documents: "relative".into(),
                ..layout
            })
            .is_err()
        );
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
