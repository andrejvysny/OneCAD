use super::*;
use onecad_core::{
    document::Document,
    ids::DocumentId,
    io::container::{ContainerCaches, ContainerWriter, SaveMeta},
};

fn write(path: &Path, document: &Document) {
    ContainerWriter::save(
        path,
        document,
        &ContainerCaches::none(),
        &SaveMeta {
            app_version: "test".into(),
            occt_fingerprint: None,
            created: "2026-10-22T00:00:00Z".into(),
            modified: "2026-10-22T00:00:00Z".into(),
        },
    )
    .unwrap();
}

#[test]
fn legacy_inbox_migration_preserves_identity_source_and_existing_edited_copy() {
    let area = tempfile::tempdir().unwrap();
    let inbox = area.path().join("Inbox");
    fs::create_dir(&inbox).unwrap();
    let source = inbox.join("legacy.onecad");
    let document = Document::new(DocumentId(uuid::Uuid::new_v4()));
    write(&source, &document);
    let original = fs::read(&source).unwrap();
    let target = migrate(area.path(), &source).unwrap();
    assert_eq!(
        target,
        area.path()
            .join("Imported/Inbox/legacy.onecad")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(
        ContainerReader::open(&target).unwrap().document().id,
        document.id
    );
    let mut edited = document.clone();
    edited.settings.tolerance_policy_hash = "edited working copy".into();
    write(&target, &edited);
    assert_eq!(migrate(area.path(), &source).unwrap(), target);
    assert_eq!(
        ContainerReader::open(&target)
            .unwrap()
            .document()
            .settings
            .tolerance_policy_hash,
        edited.settings.tolerance_policy_hash
    );
    assert_eq!(fs::read(&source).unwrap(), original);
}

#[test]
fn legacy_migration_refuses_conflicting_identity_without_replacing_either_file() {
    let area = tempfile::tempdir().unwrap();
    fs::create_dir(area.path().join("Inbox")).unwrap();
    fs::create_dir_all(area.path().join("Imported/Inbox")).unwrap();
    let source = area.path().join("Inbox/collision.onecad");
    let target = area.path().join("Imported/Inbox/collision.onecad");
    write(&source, &Document::new(DocumentId(uuid::Uuid::new_v4())));
    write(&target, &Document::new(DocumentId(uuid::Uuid::new_v4())));
    let before = fs::read(&target).unwrap();
    assert!(
        migrate(area.path(), &source)
            .unwrap_err()
            .contains("another document")
    );
    assert_eq!(fs::read(&target).unwrap(), before);
    assert!(source.is_file());
}
