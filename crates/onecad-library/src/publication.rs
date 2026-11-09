//! Package durability ordering before atomic directory publication.
use crate::{LibraryError, LibraryResult};
use std::{fs::File, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PublicationStage {
    PayloadSync,
    DirectorySync,
    Promotion,
    ParentSync,
}

pub(super) fn synchronize_tree(
    directory: &Path,
    hook: &mut dyn FnMut(PublicationStage) -> std::io::Result<()>,
) -> LibraryResult<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            synchronize_tree(&entry.path(), hook)?;
        } else if kind.is_file() {
            hook(PublicationStage::PayloadSync)?;
            File::open(entry.path())?.sync_all()?;
        } else {
            return Err(LibraryError::Io("unsupported staged package entry".into()));
        }
    }
    hook(PublicationStage::DirectorySync)?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

pub(crate) fn create_durable_directory(path: &Path) -> std::io::Result<()> {
    create_directory_with_sync(path, &mut |parent| File::open(parent)?.sync_all())
}

fn create_directory_with_sync(
    path: &Path,
    synchronize: &mut dyn FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if path.is_dir() {
        if let Some(parent) = parent {
            synchronize(parent)?;
        }
        return Ok(());
    }
    if let Some(parent) = parent {
        create_directory_with_sync(parent, synchronize)?;
    }
    match std::fs::create_dir(path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => (),
        Err(error) => return Err(error),
    }
    synchronize(parent.unwrap_or(Path::new(".")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_library_root_syncs_each_created_ancestor_entry() {
        let area = tempfile::tempdir().unwrap();
        let root = area.path().join("new-parent/library");
        let mut synced = Vec::new();
        create_directory_with_sync(&root, &mut |parent| {
            synced.push(parent.to_path_buf());
            File::open(parent)?.sync_all()
        })
        .unwrap();
        assert_eq!(
            synced,
            vec![
                area.path().parent().unwrap().to_path_buf(),
                area.path().to_path_buf(),
                area.path().join("new-parent")
            ]
        );
        assert!(root.is_dir());
    }

    #[test]
    fn failed_ancestor_sync_prevents_child_root_creation() {
        let area = tempfile::tempdir().unwrap();
        let root = area.path().join("new-parent/library");
        let result = create_directory_with_sync(&root, &mut |parent| {
            if parent == area.path() {
                Err(std::io::Error::other("injected ancestry sync"))
            } else {
                File::open(parent)?.sync_all()
            }
        });
        assert!(result.is_err());
        assert!(!root.exists());
    }
}
