//! A static project import must not silently discard a component's source closure.
use onecad_core::{
    document::{
        modules::{ModuleId, ModuleState},
        record::OperationRecord,
        Document,
    },
    ids::{DocumentId, RecordId},
    io::{
        container::{ContainerCaches, ContainerWriter, SaveMeta},
        library::ArchiveBytes,
        project_import::{read_project_for_import, ProjectImportError},
    },
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;

#[test]
fn project_import_refuses_library_metadata_or_unknown_archive_without_rewriting_source() {
    let area = tempfile::tempdir().unwrap();
    for known in [true, false] {
        let mut document = Document::new(DocumentId(Uuid::from_u128(0x165)));
        document.timeline.insert_at_cursor(OperationRecord::new(
            RecordId(Uuid::from_u128(0x166)),
            0,
            "Future feature",
            serde_json::from_value(serde_json::json!({"type":"FutureFeature", "params":{}}))
                .unwrap(),
        ));
        let mut archives = BTreeMap::new();
        if known {
            document.modules.insert(
                ModuleId::parse("onecad.library").unwrap(),
                ModuleState::new(1, serde_json::json!({"snapshots":{}})),
            );
        } else {
            let bytes = b"opaque archive owned by a future module".to_vec();
            archives.insert(
                format!("{:x}", Sha256::digest(&bytes)),
                Arc::new(ArchiveBytes::untracked(bytes)),
            );
        }
        let path = area.path().join(format!("source-{known}.onecad"));
        ContainerWriter::save_with_imports_and_library(
            &path,
            &document,
            &ContainerCaches::none(),
            &Default::default(),
            &archives,
            &SaveMeta::default(),
        )
        .unwrap();
        let original = std::fs::read(&path).unwrap();
        assert!(matches!(
            read_project_for_import(&path),
            Err(ProjectImportError::EmbeddedLibraryUnsupported)
        ));
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}
