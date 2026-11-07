//! Source-derived native cover and lever with an imported box; placements are test inputs.
#[allow(dead_code)]
#[path = "prusa_mixed_support/cover.rs"]
mod cover;
#[path = "prusa_mixed_support/document.rs"]
mod document;
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;
#[allow(dead_code)]
#[path = "prusa_mixed_support/lever.rs"]
mod lever;
#[path = "prusa_mixed_support/source_checks.rs"]
mod source_checks;

use host::Harness;
use onecad_app::{api, dto::AssemblyProjectionDto, state::AppState};
use onecad_assembly::math::{RigidTransform, Vec3d};
use onecad_assembly::occurrence::OccurrencePath;
use onecad_core::{
    document::{
        record::{KnownOperation, Operation},
        variables::Scalar,
    },
    edit::{AssemblyEdit, EditCommand},
    ids::{BodyId, RecordId},
    io::container::ContainerReader,
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use std::{path::Path, sync::Arc, time::Duration};
use uuid::Uuid;

async fn open(path: &Path, area: &Path) -> (Harness, String) {
    let h = Harness::new(area).await;
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
    (h, opened.runtime_session)
}
async fn edit(state: &AppState, session: &str, change: AssemblyEdit) -> AssemblyProjectionDto {
    let p = api::get_assembly_projection(state, session).await.unwrap();
    api::apply_assembly_edit_at_generation(state, session, p.persisted_generation, change)
        .await
        .unwrap()
}
async fn mesh(state: &AppState, session: &str, occ: u128, def: u128) -> Arc<Vec<u8>> {
    let p = api::get_assembly_projection(state, session).await.unwrap();
    let part = p
        .definitions
        .iter()
        .find(|part| part.id == document::definition(def).to_string())
        .unwrap();
    assert_eq!(part.bodies.len(), 1);
    let body: BodyId = part.bodies[0].id.parse().unwrap();
    api::component_mesh_for_path(
        state,
        session,
        &OccurrencePath(vec![document::occurrence(occ)]),
        body,
        part.mesh_generation.unwrap(),
    )
    .await
    .unwrap()
}
async fn replay(state: &AppState, session: &str, defs: &[u128]) {
    for index in defs {
        api::regenerate_component(state, session, document::definition(*index))
            .await
            .unwrap();
    }
}
fn transverse_operation(path: &Path, distance: f64) -> Operation {
    let saved = ContainerReader::open(path).unwrap();
    let mut op = saved
        .document()
        .definitions
        .get(&document::definition(2))
        .unwrap()
        .timeline
        .records()
        .iter()
        .find(|r| r.record_id == RecordId(Uuid::from_u128(0x531020)))
        .unwrap()
        .op
        .clone();
    let Operation::Known(KnownOperation::Extrude(ref mut params)) = op else {
        panic!("lever transverse cut changed kind")
    };
    params.distance = Scalar::new(distance);
    op
}
async fn edit_lever(state: &AppState, session: &str, def: u128, op: Operation) {
    edit(
        state,
        session,
        AssemblyEdit::EditDefinition {
            id: document::definition(def),
            command: Box::new(EditCommand::UpdateOperationParams {
                record: RecordId(Uuid::from_u128(0x531020)),
                op,
            }),
        },
    )
    .await;
    api::regenerate_component(state, session, document::definition(def))
        .await
        .unwrap();
}
fn verify_sources(path: &Path) {
    let saved = ContainerReader::open(path).unwrap();
    assert_eq!(saved.import_blobs().len(), 1);
    let oracle: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap();
    let hash = oracle["parts"]["box"]["sha256"].as_str().unwrap();
    assert_eq!(
        saved.read_import_blob(hash).unwrap(),
        include_bytes!("../../../fixtures/public-cad/prusa/MINI-fsenzor-box.stp")
    );
    let boxed = saved
        .document()
        .definitions
        .get(&document::definition(0))
        .unwrap();
    assert!(
        matches!(&boxed.timeline.records()[0].op, Operation::Known(KnownOperation::ImportStep(p)) if p.source_sha256 == hash)
    );
    for index in [1, 2, 3] {
        let part = saved
            .document()
            .definitions
            .get(&document::definition(index))
            .unwrap();
        assert!(part.timeline.records().iter().all(|r| matches!(
            r.op,
            Operation::Known(
                KnownOperation::Sketch(_) | KnownOperation::Extrude(_) | KnownOperation::Revolve(_)
            )
        )));
    }
}
async fn reopen(path: &Path, area: &Path) {
    verify_sources(path);
    let op = transverse_operation(path, 2.0);
    let (h, session) = open(path, &area.join("fresh-host")).await;
    replay(&h.state, &session, &[0, 1, 2, 3]).await;
    let before = mesh(&h.state, &session, 2, 2).await;
    let independent = mesh(&h.state, &session, 3, 3).await;
    edit_lever(&h.state, &session, 2, op).await;
    let after = mesh(&h.state, &session, 2, 2).await;
    assert_ne!(before.as_slice(), after.as_slice());
    assert_eq!(
        independent.as_slice(),
        mesh(&h.state, &session, 3, 3).await.as_slice()
    );
    api::undo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, &session, &[2]).await;
    assert_eq!(
        before.as_slice(),
        mesh(&h.state, &session, 2, 2).await.as_slice()
    );
    h.close().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prusa_mixed_native_assembly_cold_reopen() {
    host::required_worker();
    if let Some(area) = std::env::var_os("ONECAD_PRUSA_MIXED_CHILD") {
        let area = Path::new(&area);
        reopen(&area.join("assembly.onecad"), area).await;
        return;
    }
    let area = tempfile::tempdir().unwrap();
    let cover_path = area.path().join("cover.onecad");
    let lever_path = area.path().join("lever.onecad");
    cover::build(&cover_path).await;
    lever::build(&lever_path).await;
    let path = area.path().join("assembly.onecad");
    document::write(&path, &cover_path, &lever_path);
    let op = transverse_operation(&path, 4.0);
    let (h, session) = open(&path, &area.path().join("main-host")).await;
    replay(&h.state, &session, &[0, 1, 2]).await;
    exercise(&h, &session, op, transverse_operation(&path, 3.0)).await;
    assert!(
        api::save_document(&h.state, h.host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    h.close().await;
    verify_sources(&path);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "prusa_mixed_native_assembly_cold_reopen",
            "--nocapture",
        ])
        .env("ONECAD_PRUSA_MIXED_CHILD", area.path())
        .status()
        .unwrap();
    assert!(status.success(), "fresh assembly process failed: {status}");
}
async fn exercise(h: &Harness, session: &str, op: Operation, second_op: Operation) {
    let p = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    assert_eq!(p.occurrences.len(), 4);
    for i in 0..3 {
        let bytes = mesh(&h.state, session, i, i).await;
        MeshView::decode(&bytes, MeshLimits::default()).unwrap();
    }
    check_sources(h, session, &p).await;
    let before = mesh(&h.state, session, 2, 2).await;
    assert!(Arc::ptr_eq(&before, &mesh(&h.state, session, 3, 2).await));
    exercise_occurrence(h, session, &p).await;
    exercise_shared_edit(h, session, op, second_op, &before).await;
}

async fn check_sources(h: &Harness, session: &str, p: &AssemblyProjectionDto) {
    let (_, box_datum) = source_checks::resolved_face(
        &h.state,
        session,
        p,
        "box",
        document::definition(0),
        document::occurrence(0),
    )
    .await;
    let (_, lever_datum) = source_checks::resolved_face(
        &h.state,
        session,
        p,
        "lever",
        document::definition(2),
        document::occurrence(2),
    )
    .await;
    let cover_bytes = mesh(&h.state, session, 1, 1).await;
    let oracle: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap();
    source_checks::mesh_properties(
        &MeshView::decode(&cover_bytes, MeshLimits::default()).unwrap(),
        &oracle["parts"]["cover"],
    );
    let pose = p
        .occurrences
        .iter()
        .find(|o| o.id == document::occurrence(2).to_string())
        .unwrap()
        .local;
    assert!((pose.apply_point(lever_datum.frame.origin) - box_datum.frame.origin).norm() < 1e-5);
}

async fn exercise_occurrence(h: &Harness, session: &str, p: &AssemblyProjectionDto) {
    let sibling = p
        .occurrences
        .iter()
        .find(|o| o.id == document::occurrence(2).to_string())
        .unwrap()
        .local;
    let repeat = p
        .occurrences
        .iter()
        .find(|o| o.id == document::occurrence(3).to_string())
        .unwrap()
        .local;
    let moved = edit(
        &h.state,
        session,
        AssemblyEdit::SetPose {
            occurrence: document::occurrence(3),
            pose: RigidTransform::new(
                repeat.rotation,
                repeat.translation + Vec3d::new(2.0, 0.0, 0.0),
            ),
        },
    )
    .await;
    assert_eq!(
        moved
            .occurrences
            .iter()
            .find(|o| o.id == document::occurrence(2).to_string())
            .unwrap()
            .local,
        sibling
    );
    let hidden = edit(
        &h.state,
        session,
        AssemblyEdit::SetVisibility {
            occurrence: document::occurrence(3),
            visible: false,
        },
    )
    .await;
    assert!(
        !hidden
            .occurrences
            .iter()
            .find(|o| o.id == document::occurrence(3).to_string())
            .unwrap()
            .visible
    );
    assert!(
        hidden
            .occurrences
            .iter()
            .find(|o| o.id == document::occurrence(2).to_string())
            .unwrap()
            .visible
    );
    edit(
        &h.state,
        session,
        AssemblyEdit::SetVisibility {
            occurrence: document::occurrence(3),
            visible: true,
        },
    )
    .await;
}

async fn exercise_shared_edit(
    h: &Harness,
    session: &str,
    op: Operation,
    second_op: Operation,
    before: &Arc<Vec<u8>>,
) {
    edit_lever(&h.state, session, 2, op.clone()).await;
    let changed = mesh(&h.state, session, 2, 2).await;
    assert_ne!(before.as_slice(), changed.as_slice());
    assert!(Arc::ptr_eq(&changed, &mesh(&h.state, session, 3, 2).await));
    api::undo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, session, &[2]).await;
    assert_eq!(
        before.as_slice(),
        mesh(&h.state, session, 2, 2).await.as_slice()
    );
    api::redo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, session, &[2]).await;
    assert_eq!(
        changed.as_slice(),
        mesh(&h.state, session, 2, 2).await.as_slice()
    );
    edit(
        &h.state,
        session,
        AssemblyEdit::MakeIndependent {
            occurrence: document::occurrence(3),
            new_definition: document::definition(3),
            name: "Independent lever".into(),
        },
    )
    .await;
    replay(&h.state, session, &[3]).await;
    let independent = mesh(&h.state, session, 3, 3).await;
    assert_eq!(changed.as_slice(), independent.as_slice());
    edit_lever(&h.state, session, 2, second_op).await;
    assert_ne!(
        independent.as_slice(),
        mesh(&h.state, session, 2, 2).await.as_slice()
    );
    assert_eq!(
        independent.as_slice(),
        mesh(&h.state, session, 3, 3).await.as_slice()
    );
}
