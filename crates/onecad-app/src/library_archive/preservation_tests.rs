use super::*;
use onecad_core::document::component::{ComponentDef, ComponentDefId};
use onecad_core::document::record::{Operation, OperationRecord};
use onecad_core::history::Timeline;
use onecad_core::ids::{DocumentId, RecordId};
use onecad_core::io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta};
use onecad_core::io::imports::ImportBlobs;
use serde_json::{json, Value};

fn fixture() -> (Document, LibraryArchives, BTreeSet<String>) {
    let mut document = Document::new(DocumentId::new());
    let archives: LibraryArchives = [
        b"first carried archive".to_vec(),
        b"second carried archive".to_vec(),
    ]
    .into_iter()
    .map(|bytes| {
        (
            onecad_library::blob::sha256_hex(&bytes),
            Arc::new(onecad_core::io::library::ArchiveBytes::untracked(bytes)),
        )
    })
    .collect();
    let entries: BTreeMap<_, _> = archives.keys().enumerate().map(|(index, sha)| {
        (format!("sha256:{}", index.to_string().repeat(64)), json!({
            "componentId": "example.part", "componentVersion": "1.0.0", "archiveSha256": sha,
        }))
    }).collect();
    document.modules.insert(
        module_id(),
        ModuleState::new(1, json!({"snapshots": entries})),
    );
    let known = archives.keys().cloned().collect();
    (document, archives, known)
}

fn placed_operation() -> Value {
    json!({"opType":"PlaceComponent", "params": {
        "componentId":"example.part", "componentVersion":"1.0.0",
        "componentRevision":format!("sha256:{}", "0".repeat(64)), "params":{},
        "source":{"kind":"generator", "generatorId":"iso4762", "generatorVersion":1, "params":{}},
        "placement":{"translate":[0,0,0]},
        "mate":{"selfAttachment":"seat", "kind":"concentric", "target":{
            "primary":{"bodyId":uuid::Uuid::new_v4(), "elementId":"el_face", "kind":"face"},
            "anchor":{"worldPoint":[0,0,0], "localFrame":{"origin":[0,0,0], "x":[1,0,0], "y":[0,1,0], "z":[0,0,1]}}
        }}
    }})
}

fn put_record(document: &mut Document, operation: Value, embedded: bool, suppressed: bool) {
    let mut record = OperationRecord::new(
        RecordId::new(),
        0,
        "Consumer",
        serde_json::from_value::<Operation>(operation).unwrap(),
    );
    record.suppressed = suppressed;
    let timeline = Timeline::from_records(vec![record]);
    if embedded {
        let id = ComponentDefId(uuid::Uuid::new_v4());
        let mut definition = ComponentDef::new(id, "Embedded");
        definition.timeline = timeline;
        document.definitions.insert(id, definition);
    } else {
        document.timeline = timeline;
    }
}

fn assert_cold_preservation(
    mut document: Document,
    archives: &LibraryArchives,
    known: &BTreeSet<String>,
) {
    let before = serde_json::to_value(&document.modules).unwrap();
    let authored = serde_json::to_value(&document).unwrap();
    prune_metadata(&mut document);
    assert_eq!(serde_json::to_value(&document.modules).unwrap(), before);
    let carried = snapshot_for_save(&document, archives, known);
    assert_eq!(carried.len(), archives.len());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("offline.onecad");
    ContainerWriter::save_with_imports_and_library(
        &path,
        &document,
        &ContainerCaches::none(),
        &ImportBlobs::new(),
        &carried,
        &SaveMeta::default(),
    )
    .unwrap();
    drop(document);
    let loaded = ContainerReader::open(&path).unwrap();
    assert_eq!(serde_json::to_value(loaded.document()).unwrap(), authored);
    assert_eq!(
        serde_json::to_value(&loaded.document().modules).unwrap(),
        before
    );
    for (sha, bytes) in archives {
        assert_eq!(loaded.read_library_archive(sha).unwrap(), bytes.as_slice());
    }
}

#[test]
fn opaque_root_embedded_and_suppressed_consumers_survive_offline_reopen() {
    for embedded in [false, true] {
        for suppressed in [false, true] {
            let (mut document, archives, known) = fixture();
            put_record(
                &mut document,
                json!({"opType":"FutureComponent", "params":{"reference":"private encoding"}}),
                embedded,
                suppressed,
            );
            assert_cold_preservation(document, &archives, &known);
        }
    }
}

#[test]
fn nested_supported_extensions_preserve_all_snapshot_metadata_and_archives() {
    for embedded in [false, true] {
        for pointer in [
            "/params",
            "/params/source",
            "/params/mate",
            "/params/mate/target",
            "/params/mate/target/primary",
            "/params/mate/target/anchor",
            "/params/mate/target/anchor/localFrame",
        ] {
            let (mut document, archives, known) = fixture();
            let mut operation = placed_operation();
            operation
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(
                    "futureConsumer".into(),
                    json!({"reference":"private encoding"}),
                );
            put_record(&mut document, operation, embedded, true);
            assert_cold_preservation(document, &archives, &known);
        }
    }
}

#[test]
fn document_definition_record_and_determinism_extensions_are_incomplete() {
    for location in [
        "document",
        "root",
        "assembly",
        "definition",
        "record",
        "determinism",
    ] {
        let (mut document, archives, known) = fixture();
        put_record(&mut document, placed_operation(), true, false);
        let value = json!({"reference":"private encoding"});
        match location {
            "document" => {
                document.extra.insert("futureConsumer".into(), value);
            }
            "root" => {
                document
                    .root_definition_extra
                    .insert("futureConsumer".into(), value);
            }
            "assembly" => {
                document
                    .assembly
                    .extra
                    .insert("futureConsumer".into(), value);
            }
            "definition" => {
                document
                    .definitions
                    .values_mut()
                    .next()
                    .unwrap()
                    .extra
                    .insert("futureConsumer".into(), value);
            }
            _ => {
                let definition = document.definitions.values_mut().next().unwrap();
                let mut records = definition.timeline.records().to_vec();
                if location == "record" {
                    records[0].extra.insert("futureConsumer".into(), value);
                } else {
                    records[0]
                        .determinism
                        .extra
                        .insert("futureConsumer".into(), value);
                }
                definition.timeline = Timeline::from_records(records);
            }
        }
        assert_cold_preservation(document, &archives, &known);
    }
}

#[test]
fn complete_knowledge_prunes_only_known_unused_revisions() {
    for embedded in [false, true] {
        let (mut document, archives, known) = fixture();
        put_record(&mut document, placed_operation(), embedded, true);
        let used = format!("sha256:{}", "0".repeat(64));
        let refs = references(&document).unwrap();
        let used_sha = &refs[&used].archive_sha256;
        assert_eq!(
            snapshot_for_save(&document, &archives, &known)
                .keys()
                .collect::<Vec<_>>(),
            vec![used_sha]
        );
        prune_metadata(&mut document);
        let remaining = references(&document).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining.contains_key(&used));
        assert_eq!(
            snapshot_for_save(&document, &archives, &known)[used_sha].as_slice(),
            archives[used_sha].as_slice()
        );
    }
}
