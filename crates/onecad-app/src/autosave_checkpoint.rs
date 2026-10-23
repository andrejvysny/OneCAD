//! Preserve the previous checkpoint pair until both durable replacements succeed.
use onecad_core::io::IoError;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Backup {
    originals: [PathBuf; 2],
    copies: [Option<PathBuf>; 2],
}

impl Backup {
    fn create(container: &Path, marker: &Path) -> Result<Self, IoError> {
        let parent = container
            .parent()
            .ok_or_else(|| IoError::Io("Missing checkpoint directory".into()))?;
        let retained = fs::read_dir(parent)?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".checkpoint-backup-")
            })
            .count();
        if retained >= 16 {
            return Err(IoError::Io(
                "Recovery has 16 retained backup files; preserve/reconcile them before retrying"
                    .into(),
            ));
        }
        let mut backup = Self {
            originals: [container.into(), marker.into()],
            copies: [None, None],
        };
        for index in 0..2 {
            match fs::symlink_metadata(&backup.originals[index]) {
                Ok(metadata) if metadata.is_file() => {
                    let copy = parent.join(format!(
                        ".checkpoint-backup-{}-{index}",
                        uuid::Uuid::new_v4()
                    ));
                    if let Err(error) = fs::hard_link(&backup.originals[index], &copy) {
                        backup.cleanup();
                        return Err(IoError::Io(format!(
                            "Cannot preserve previous checkpoint: {error}"
                        )));
                    }
                    backup.copies[index] = Some(copy);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                other => {
                    backup.cleanup();
                    return Err(IoError::Io(format!(
                        "Checkpoint backup requires regular files: {other:?}"
                    )));
                }
            }
        }
        fs::File::open(parent)?.sync_all()?;
        Ok(backup)
    }

    fn restore(&mut self) -> Result<(), IoError> {
        let mut failures = Vec::new();
        for index in 0..2 {
            let result = if let Some(copy) = &self.copies[index] {
                fs::rename(copy, &self.originals[index]).and_then(|()| {
                    // rename is a no-op when both names still refer to the old inode.
                    match fs::remove_file(copy) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        result => result,
                    }
                })
            } else {
                match fs::remove_file(&self.originals[index]) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    result => result,
                }
            };
            if let Err(error) = result {
                failures.push(format!(
                    "{}: {error}; backup {:?}",
                    self.originals[index].display(),
                    self.copies[index]
                ));
            }
        }
        if let Some(parent) = self.originals[0].parent() {
            if let Err(error) = fs::File::open(parent).and_then(|file| file.sync_all()) {
                failures.push(format!("rollback directory sync: {error}"));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(IoError::Io(failures.join("; ")))
        }
    }

    fn cleanup(&mut self) {
        for copy in self.copies.iter().flatten() {
            if let Err(error) = fs::remove_file(copy) {
                tracing::warn!(path = %copy.display(), %error, "checkpoint backup cleanup failed");
            }
        }
    }
}

pub(super) fn preserve_pair(
    container: &Path,
    marker: &Path,
    write: impl FnOnce() -> Result<(), IoError>,
) -> Result<(), IoError> {
    let mut backup = Backup::create(container, marker)?;
    match write() {
        Ok(()) => {
            backup.cleanup();
            Ok(())
        }
        Err(error) => match backup.restore() {
            Ok(()) => Err(error),
            Err(rollback) => Err(IoError::Io(format!(
                "{error}; checkpoint rollback failed: {rollback}"
            ))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_failure_restores_prior_container_and_marker() {
        let area = tempfile::tempdir().unwrap();
        let container = area.path().join("checkpoint.onecad");
        let marker = area.path().join("marker.json");
        fs::write(&container, b"old accepted checkpoint").unwrap();
        fs::write(&marker, b"old marker").unwrap();
        let result = preserve_pair(&container, &marker, || {
            fs::remove_file(&container)?;
            fs::write(&container, b"new checkpoint")?;
            Err(IoError::Io("injected marker failure".into()))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&container).unwrap(), b"old accepted checkpoint");
        assert_eq!(fs::read(&marker).unwrap(), b"old marker");
        assert_eq!(fs::read_dir(area.path()).unwrap().count(), 2);
    }

    #[test]
    fn rollback_failure_retains_prior_bytes_and_reports_backup() {
        let area = tempfile::tempdir().unwrap();
        let container = area.path().join("checkpoint.onecad");
        let marker = area.path().join("marker.json");
        fs::write(&container, b"prior checkpoint").unwrap();
        fs::write(&marker, b"prior marker").unwrap();
        let result = preserve_pair(&container, &marker, || {
            fs::remove_file(&container)?;
            fs::create_dir(&container)?;
            fs::write(container.join("block-rename"), b"failure")?;
            Err(IoError::Io("injected write failure".into()))
        })
        .unwrap_err()
        .to_string();
        assert!(result.contains("checkpoint rollback failed"));
        assert!(result.contains(".checkpoint-backup-"));
        let retained = fs::read_dir(area.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".checkpoint-backup-")
            })
            .map(|entry| fs::read(entry.path()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(retained, vec![b"prior checkpoint".to_vec()]);
    }
}
