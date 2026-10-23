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
fn recents_rebase_three_documents_before_pruning_and_persist_new_locations() {
    let area = tempfile::tempdir().unwrap();
    let old = documents(&area.path().join("old"));
    let current = documents(&area.path().join("current"));
    let recents = area.path().join("recents.json");
    for name in ["a", "b", "c"] {
        let source = old.join(format!("{name}.onecad"));
        write(&source, DocumentId::new());
        record_at(&recents, &source);
        std::fs::rename(&source, current.join(source.file_name().unwrap())).unwrap();
    }
    // Even a caller without container context must not prune relocated entries.
    assert!(list_at(&recents).is_empty());
    assert_eq!(load_at(&recents).len(), 3);
    let entries = list_at_in_documents(&recents, Some(&current));
    assert_eq!(entries.len(), 3);
    for entry in entries {
        assert!(Path::new(&entry.path).starts_with(&current));
        assert!(load_at(&recents)
            .iter()
            .any(|saved| saved.path == entry.path));
    }
    assert_eq!(list_at(&recents).len(), 3);
}

#[test]
fn conflicting_identity_and_unknown_legacy_entry_are_retained_without_guessing() {
    let area = tempfile::tempdir().unwrap();
    let old = documents(&area.path().join("old"));
    let current = documents(&area.path().join("current"));
    let recents = area.path().join("recents.json");
    let source = old.join("part.onecad");
    write(&source, DocumentId::new());
    record_at(&recents, &source);
    std::fs::remove_file(&source).unwrap();
    write(&current.join("part.onecad"), DocumentId::new());
    let before = std::fs::read(&recents).unwrap();
    assert!(list_at_in_documents(&recents, Some(&current)).is_empty());
    assert_eq!(std::fs::read(&recents).unwrap(), before);
    let mut legacy = load_at(&recents);
    legacy[0].document_id = None;
    store_at(&recents, &legacy);
    assert!(list_at_in_documents(&recents, Some(&current)).is_empty());
    assert_eq!(load_at(&recents), legacy);
}

#[test]
fn live_legacy_reference_learns_identity_before_later_container_move() {
    let area = tempfile::tempdir().unwrap();
    let old = documents(&area.path().join("old"));
    let current = documents(&area.path().join("current"));
    let recents = area.path().join("recents.json");
    let source = old.join("part.onecad");
    let id = DocumentId::new();
    write(&source, id);
    std::fs::write(
        &recents,
        serde_json::to_vec(&serde_json::json!([{
            "path": source, "name": "part", "lastOpenedMs": 1234
        }]))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(list_at(&recents).len(), 1);
    assert_eq!(load_at(&recents)[0].document_id, Some(id));
    let target = current.join("part.onecad");
    std::fs::rename(&source, &target).unwrap();
    let rebased = list_at_in_documents(&recents, Some(&current));
    assert_eq!(rebased[0].path, target.to_string_lossy());
}
