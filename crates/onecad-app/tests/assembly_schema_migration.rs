//! Legacy container migration must preserve its source and require a new save path.
use std::{path::PathBuf, sync::Arc};

use onecad_app::{document_runtime::DocumentRuntime, worker::PendingBackend};
use onecad_core::io::container::{ContainerReader, SaveMeta};
use sha2::{Digest, Sha256};

#[test]
fn legacy_fixture_migrates_without_rewriting_source_and_reopens_new_file() {
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qa_plate_hole.onecad");
    let original = std::fs::read(&source).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&original)),
        "a83d1bbac05941030fc97703eea69e9775b2c29326d8c6a91782383c1183d9ec"
    );
    let migrated = ContainerReader::open(&source).unwrap();
    let migrated_json = serde_json::to_value(migrated.document()).unwrap();
    let backend = Arc::new(PendingBackend);
    let mut runtime =
        DocumentRuntime::open(&source, backend.clone(), backend.clone(), backend.clone()).unwrap();
    assert!(
        runtime.path().is_none(),
        "migration requires a new save target"
    );
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("migrated.onecad");
    runtime
        .save(
            &target,
            SaveMeta {
                app_version: "test".into(),
                occt_fingerprint: None,
                created: String::new(),
                modified: String::new(),
            },
        )
        .unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), original);
    let reopened = ContainerReader::open(&target).unwrap();
    let reopened_json = serde_json::to_value(reopened.document()).unwrap();
    assert_eq!(
        reopened_json["rootDefinition"],
        migrated_json["rootDefinition"]
    );
    assert_eq!(reopened.document().schema_version, 2);
    assert_eq!(
        reopened.document().id.to_string(),
        "92a6ebf3-9192-447a-86ad-8e72521118cc"
    );
    assert!(!reopened.document().timeline.records().is_empty());
    assert!(reopened.document().definitions.is_empty());
    assert!(reopened.document().assembly.occurrences.is_empty());
}

#[test]
fn unopened_assembly_is_refused_without_changing_its_bytes() {
    use onecad_core::document::{
        component::{ComponentDef, ComponentDefId},
        Document,
    };
    use onecad_core::ids::DocumentId;
    use onecad_core::io::container::{ContainerCaches, ContainerWriter};
    use onecad_core::io::IoError;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("assembly.onecad");
    let mut doc = Document::new(DocumentId(uuid::Uuid::from_u128(1)));
    let id = ComponentDefId(uuid::Uuid::from_u128(2));
    doc.definitions.insert(id, ComponentDef::new(id, "Part"));
    ContainerWriter::save(
        &path,
        &doc,
        &ContainerCaches::none(),
        &SaveMeta {
            app_version: "test".into(),
            occt_fingerprint: None,
            created: String::new(),
            modified: String::new(),
        },
    )
    .unwrap();
    let before = std::fs::read(&path).unwrap();
    let backend = Arc::new(PendingBackend);
    assert!(matches!(
        DocumentRuntime::open(&path, backend.clone(), backend.clone(), backend),
        Err(IoError::UnsupportedFeature(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
