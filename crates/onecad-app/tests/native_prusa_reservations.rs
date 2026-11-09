//! Synthetic orphan reservations exercise real native history replay and persistence.
//! They do not establish worker-generated NeedsRepair or public geometry equivalence.
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;

use onecad_app::api;
use onecad_core::{
    document::{
        component::{ComponentDefId, Occurrence, OccurrenceId, OccurrencePath, RigidTransform},
        element_index::{ElementBindingState, ElementEntry},
        refs::ElementKind,
        Document,
    },
    edit::{AssemblyEdit, EditCommand},
    ids::ElementId,
    io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta},
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use serde_json::json;
use std::{path::Path, time::Duration};
use uuid::Uuid;

const RESERVED: &str = "el_00000000000000000000000000000001";
const SEEDS: [u128; 2] = [0xd205, 0xd206];
const TEST: &str = "native_prusa_retained_reservations_replay_and_cold_reopen";

fn write(path: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/qualification/assembly-public-10x20/native-compact-10x20.onecad");
    let loaded = ContainerReader::open(&source).unwrap();
    let mut document = Document::new(loaded.document().id);
    for seed in SEEDS {
        let id = ComponentDefId(Uuid::from_u128(seed));
        let mut part = loaded.document().definitions[&id].clone();
        let element = ElementId::new(RESERVED);
        assert!(part.elements.get(&element).is_none());
        let mut entry = ElementEntry::new(part.bodies.bodies()[0].id, ElementKind::Face)
            .with_evidence(None, Some(json!({"syntheticOrphanEvidence":true})));
        entry.binding_state = Some(ElementBindingState::Quarantined);
        part.elements.insert(element, entry);
        document.definitions.insert(id, part);
        document
            .assembly
            .occurrences
            .insert(Occurrence {
                id: OccurrenceId(Uuid::from_u128(seed)),
                def: id,
                parent: None,
                local: RigidTransform::IDENTITY,
                grounded: true,
                name: "Native history".into(),
            })
            .unwrap();
    }
    let meta = SaveMeta {
        app_version: "native-reservation-test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save(path, &document, &ContainerCaches::none(), &meta).unwrap();
}

fn reservations(path: &Path) -> serde_json::Value {
    let loaded = ContainerReader::open(path).unwrap();
    let entries: Vec<_> = SEEDS
        .into_iter()
        .map(|seed| {
            let part = &loaded.document().definitions[&ComponentDefId(Uuid::from_u128(seed))];
            let entry = part.elements.get(&ElementId::new(RESERVED)).unwrap();
            assert!(entry.is_quarantined());
            assert!(!entry.has_rebind_evidence());
            entry.clone()
        })
        .collect();
    serde_json::to_value(entries).unwrap()
}

async fn replay(h: &host::Harness, session: &str) {
    for seed in SEEDS {
        let id = ComponentDefId(Uuid::from_u128(seed));
        let (_, projection) = api::regenerate_component(&h.state, session, id)
            .await
            .unwrap();
        let part = projection
            .definitions
            .iter()
            .find(|part| part.id == id.to_string())
            .unwrap();
        assert_eq!(part.bodies.len(), 1);
        let mesh = api::component_mesh_for_path(
            &h.state,
            session,
            &OccurrencePath(vec![OccurrenceId(Uuid::from_u128(seed))]),
            part.bodies[0].id.parse().unwrap(),
            part.mesh_generation.unwrap(),
        )
        .await
        .unwrap();
        let checked = MeshView::decode(&mesh, MeshLimits::default()).unwrap();
        assert!(checked.header().triangle_count > 0);
        for face in 0..checked.header().face_count as usize {
            assert_ne!(checked.face_id(face), Some(RESERVED));
        }
        for edge in 0..checked.header().edge_count as usize {
            assert_ne!(checked.edge_id(edge), Some(RESERVED));
        }
    }
}

async fn phase(path: &Path, area: &Path, cold: bool) {
    let expected = reservations(path);
    let h = host::Harness::new(area).await;
    let opened = api::open_assembly_document(
        &h.state,
        h.host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(
        h.state
            .readiness()
            .wait_ready(Duration::from_secs(15))
            .await
    );
    replay(&h, &opened.runtime_session).await;
    edit_name(&h, &opened.runtime_session, cold).await;
    replay(&h, &opened.runtime_session).await;
    assert!(
        api::save_document(&h.state, h.host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    h.close().await;
    assert_eq!(reservations(path), expected);
}

async fn edit_name(h: &host::Harness, session: &str, cold: bool) {
    let id = ComponentDefId(Uuid::from_u128(SEEDS[1]));
    let before = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    let part = before
        .definitions
        .iter()
        .find(|part| part.id == id.to_string())
        .unwrap();
    let original = part.bodies[0].name.clone();
    let body = part.bodies[0].id.parse().unwrap();
    let name = if cold {
        "Cold reopened lever"
    } else {
        "Edited lever"
    };
    api::apply_assembly_edit_at_generation(
        &h.state,
        session,
        before.persisted_generation,
        AssemblyEdit::EditDefinition {
            id,
            command: Box::new(EditCommand::RenameBody {
                body,
                name: name.into(),
            }),
        },
    )
    .await
    .unwrap();
    api::undo(&h.state, h.host.clone()).await.unwrap();
    assert_name(h, session, id, &original).await;
    api::redo(&h.state, h.host.clone()).await.unwrap();
    assert_name(h, session, id, name).await;
}

async fn assert_name(h: &host::Harness, session: &str, id: ComponentDefId, name: &str) {
    let projection = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    let part = projection
        .definitions
        .iter()
        .find(|part| part.id == id.to_string())
        .unwrap();
    assert_eq!(part.bodies[0].name, name);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_prusa_retained_reservations_replay_and_cold_reopen() {
    host::required_worker();
    if let Some(area) = std::env::var_os("ONECAD_PRUSA_RESERVATION_CHILD") {
        let area = Path::new(&area);
        phase(&area.join("assembly.onecad"), &area.join("cold"), true).await;
        return;
    }
    let area = tempfile::tempdir().unwrap();
    let path = area.path().join("assembly.onecad");
    write(&path);
    phase(&path, &area.path().join("parent"), false).await;
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env("ONECAD_PRUSA_RESERVATION_CHILD", area.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    for line in stdout.lines().chain(stderr.lines()) {
        eprintln!("fresh-process evidence: {line}");
    }
    assert!(
        output.status.success(),
        "fresh process failed: {}",
        output.status
    );
    assert_eq!(stdout.matches(&format!("test {TEST} ... ok")).count(), 1);
}
