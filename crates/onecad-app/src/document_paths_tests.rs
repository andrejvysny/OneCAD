use super::*;
use onecad_core::{
    document::Document,
    io::container::{ContainerCaches, ContainerWriter, SaveMeta},
};

fn documents(root: &Path) -> PathBuf {
    let path = root
        .join("Containers/Data/Application")
        .join(uuid::Uuid::new_v4().to_string())
        .join("Documents");
    std::fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}

fn write(path: &Path, id: DocumentId) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    ContainerWriter::save(
        path,
        &Document::new(id),
        &ContainerCaches::none(),
        &SaveMeta {
            app_version: "test".into(),
            occt_fingerprint: None,
            created: "2026-10-23T00:00:00Z".into(),
            modified: "2026-10-23T00:00:00Z".into(),
        },
    )
    .unwrap();
}

#[test]
fn relocated_container_preserves_relative_location_and_requires_identity() {
    let area = tempfile::tempdir().unwrap();
    let old = documents(&area.path().join("old"));
    let current = documents(&area.path().join("current"));
    let source = old.join("Imported/part.onecad");
    let target = current.join("Imported/part.onecad");
    let id = DocumentId::new();
    write(&target, id);
    assert_eq!(
        relocate_saved_path(&current, &source, id).unwrap(),
        Some(target.clone())
    );
    let before = std::fs::read(&target).unwrap();
    assert!(relocate_saved_path(&current, &source, DocumentId::new()).is_err());
    assert_eq!(std::fs::read(target).unwrap(), before);
}

#[test]
fn existing_source_is_never_redirected_to_a_different_container() {
    let area = tempfile::tempdir().unwrap();
    let old = documents(&area.path().join("old"));
    let current = documents(&area.path().join("current"));
    let source = old.join("part.onecad");
    let id = DocumentId::new();
    write(&source, id);
    write(&current.join("part.onecad"), id);
    assert_eq!(relocate_saved_path(&current, &source, id).unwrap(), None);
}

#[test]
fn external_documents_and_parent_traversal_are_not_container_references() {
    let area = tempfile::tempdir().unwrap();
    let current = documents(&area.path().join("current"));
    let id = DocumentId::new();
    write(&current.join("part.onecad"), id);
    let external = area.path().join("external/Documents/part.onecad");
    assert_eq!(relocate_saved_path(&current, &external, id).unwrap(), None);
    let old = documents(&area.path().join("old"));
    let traversal = old.join("../Documents/part.onecad");
    assert!(!is_container_document(&traversal));
    assert_eq!(relocate_saved_path(&current, &traversal, id).unwrap(), None);
    assert_eq!(
        relocate_saved_path(area.path(), &old.join("part.onecad"), id).unwrap(),
        None
    );
}

#[test]
fn missing_or_corrupt_candidate_cannot_be_adopted() {
    let area = tempfile::tempdir().unwrap();
    let old = documents(&area.path().join("old"));
    let current = documents(&area.path().join("current"));
    let source = old.join("part.onecad");
    let id = DocumentId::new();
    assert_eq!(relocate_saved_path(&current, &source, id).unwrap(), None);
    let target = current.join("part.onecad");
    std::fs::write(&target, b"incomplete container").unwrap();
    assert!(relocate_saved_path(&current, &source, id).is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"incomplete container");
}

#[cfg(unix)]
#[test]
fn symlink_cannot_escape_current_documents_even_with_matching_identity() {
    let area = tempfile::tempdir().unwrap();
    let old = documents(&area.path().join("old"));
    let current = documents(&area.path().join("current"));
    let external = area.path().join("external.onecad");
    let id = DocumentId::new();
    write(&external, id);
    std::os::unix::fs::symlink(&external, current.join("part.onecad")).unwrap();
    assert!(relocate_saved_path(&current, &old.join("part.onecad"), id).is_err());
}
