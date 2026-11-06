//! Deterministic admission mutations; not a replacement for a coverage-guided campaign.
use onecad_core::{
    document::{
        component::ComponentDef,
        modules::{ModuleId, ModuleState},
        Document,
    },
    ids::DocumentId,
    io::{
        container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta},
        document_io,
    },
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SEED: u64 = 0x4220_3390_3940_0002;

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn mutate(seed: &[u8], state: &mut u64, iteration: usize) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    match iteration % 4 {
        0 => {
            let length = next(state) as usize % (bytes.len() + 1);
            bytes.truncate(length);
        }
        1 | 2 if !bytes.is_empty() => {
            for _ in 0..1 + iteration % 8 {
                let index = next(state) as usize % bytes.len();
                bytes[index] ^= (next(state) as u8) | 1;
            }
        }
        _ => bytes.extend_from_slice(b"\0{\"schemaVersion\":999999999999999999999}"),
    }
    bytes
}

fn document_seeds() -> Vec<Vec<u8>> {
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0x422)));
    doc.modules.insert(
        ModuleId::parse("org.example.unknown").unwrap(),
        ModuleState::new(77, serde_json::json!({"opaque": [1, "retain", null]})),
    );
    let id = onecad_core::document::component::ComponentDefId(Uuid::from_u128(0x394));
    doc.definitions
        .insert(id, ComponentDef::new(id, "Embedded"));
    doc.timeline
        .insert_at_cursor(onecad_core::document::record::OperationRecord::new(
            onecad_core::ids::RecordId(Uuid::from_u128(0x4221)),
            0,
            "Unknown feature",
            serde_json::from_value(
                serde_json::json!({"type":"FutureFeature", "params":{"opaque":[1,2,3]}}),
            )
            .unwrap(),
        ));
    doc.assembly
        .occurrences
        .insert(onecad_core::document::component::Occurrence {
            id: onecad_core::document::component::OccurrenceId(Uuid::from_u128(0x4222)),
            def: id,
            parent: None,
            local: onecad_core::document::component::RigidTransform::IDENTITY,
            grounded: true,
            name: "Embedded occurrence".into(),
        })
        .unwrap();
    let current = document_io::serialize_document(&doc).unwrap();
    document_io::document_from_value(document_io::parse_value(&current).unwrap()).unwrap();
    let mut future: serde_json::Value = serde_json::from_slice(&current).unwrap();
    future["schemaVersion"] = 3.into();
    let mut invalid = future.clone();
    invalid["schemaVersion"] = 2.into();
    invalid["assembly"] = serde_json::json!({"occurrences":{"occurrences":[{
        "id":Uuid::from_u128(1),"def":id,"parent":null,"name":"Invalid rotation",
        "grounded":false,"local":{"rotation":{"w":2,"x":0,"y":0,"z":0},
        "translation":{"x":0,"y":0,"z":0}}}]}});
    vec![
        current,
        serde_json::to_vec(&future).unwrap(),
        serde_json::to_vec(&invalid).unwrap(),
    ]
}

#[test]
fn bounded_fuzz_document_and_container_admission() {
    let started = std::time::Instant::now();
    let area = tempfile::tempdir().unwrap();
    let saved = area.path().join("original.onecad");
    let candidate = area.path().join("candidate.onecad");
    let doc = Document::new(DocumentId(Uuid::from_u128(0x422)));
    ContainerWriter::save(&saved, &doc, &ContainerCaches::none(), &SaveMeta::default()).unwrap();
    let original = std::fs::read(&saved).unwrap();
    let digest = Sha256::digest(&original);
    let mut state = SEED;
    let mut count = 0;
    for seed in document_seeds() {
        eprintln!(
            "document-seed sha256={:x} bytes={}",
            Sha256::digest(&seed),
            seed.len()
        );
        for iteration in 0..512 {
            let bytes = mutate(&seed, &mut state, iteration);
            if let Ok(value) = document_io::parse_value(&bytes) {
                let _ = document_io::document_from_value(value);
            }
            count += 1;
        }
    }
    let legacy = include_bytes!("../../onecad-app/tests/fixtures/qa_plate_hole.onecad");
    for seed in [&original[..], &legacy[..]] {
        assert!(seed.len() < 4 * 1024 * 1024);
        eprintln!(
            "container-seed sha256={:x} bytes={}",
            Sha256::digest(seed),
            seed.len()
        );
        for iteration in 0..256 {
            std::fs::write(&candidate, mutate(seed, &mut state, iteration)).unwrap();
            let before = std::fs::read(&candidate).unwrap();
            let _ = ContainerReader::open(&candidate);
            assert_eq!(std::fs::read(&candidate).unwrap(), before);
            count += 1;
        }
    }
    assert_eq!(Sha256::digest(std::fs::read(saved).unwrap()), digest);
    eprintln!(
        "bounded-admission-fuzz seed={SEED:016x} cases={count} maxSeedBytes=4194304 elapsedMs={}",
        started.elapsed().as_millis()
    );
}
