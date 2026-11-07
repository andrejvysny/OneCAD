//! Source-derived native cover and lever with an imported box; placements are test inputs.
#[path = "prusa_mixed_support/admission.rs"]
mod admission;
#[allow(dead_code)]
#[path = "prusa_native_cover_support/mod.rs"]
mod cover;
#[path = "prusa_mixed_support/document.rs"]
mod document;
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;
#[allow(dead_code)]
#[path = "prusa_native_lever_support/mod.rs"]
mod lever;
#[path = "prusa_mixed_support/mesh_compare.rs"]
mod mesh_compare;
#[path = "public_assembly_artifacts/mod.rs"]
mod public_assembly_artifacts;
#[path = "prusa_mixed_support/source_checks.rs"]
mod source_checks;

use host::Harness;
use onecad_app::{api, dto::AssemblyProjectionDto, state::AppState};
use onecad_assembly::math::{RigidTransform, Vec3d};
use onecad_assembly::occurrence::OccurrencePath;
use onecad_core::{
    document::{
        record::{KnownOperation, Operation, OperationRecord, TransformBodyParams},
        variables::Scalar,
    },
    edit::{AssemblyEdit, EditCommand},
    ids::{BodyId, RecordId},
    io::container::ContainerReader,
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use source_checks::verify_sources;
use std::{path::Path, sync::Arc, time::Duration};
use uuid::Uuid;

async fn open(path: &Path, area: &Path, probe: Option<&admission::Probe>) -> (Harness, String) {
    let h = Harness::new(area).await;
    if let Some(probe) = probe {
        probe.install(&h.state);
    }
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
    eprintln!("mixed native edit definition={def}: {op:?}");
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

async fn ambiguous_cut_refuses_rebind(
    h: &Harness,
    session: &str,
    op: Operation,
    previous: &[u8],
    independent: &[u8],
) {
    edit(
        &h.state,
        session,
        AssemblyEdit::EditDefinition {
            id: document::definition(2),
            command: Box::new(EditCommand::UpdateOperationParams {
                record: RecordId(Uuid::from_u128(0x531020)),
                op,
            }),
        },
    )
    .await;
    let error = api::regenerate_component(&h.state, session, document::definition(2))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, onecad_app::error::ApiError::UnresolvedTopology(_)),
        "{error:?}"
    );
    mesh_compare::equal(
        "independent after refused ambiguous cut",
        independent,
        mesh(&h.state, session, 3, 3).await.as_slice(),
    );
    api::undo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, session, &[2]).await;
    mesh_compare::equal(
        "ambiguous cut undo",
        previous,
        mesh(&h.state, session, 2, 2).await.as_slice(),
    );
    mesh_compare::equal(
        "independent after ambiguous cut undo",
        independent,
        mesh(&h.state, session, 3, 3).await.as_slice(),
    );
}

async fn apply_translation(state: &AppState, session: &str, record: u128, distance: f64) {
    let projection = api::get_assembly_projection(state, session).await.unwrap();
    let part = projection
        .definitions
        .iter()
        .find(|part| part.id == document::definition(2).to_string())
        .unwrap();
    let targets = part
        .bodies
        .iter()
        .map(|body| body.id.parse().unwrap())
        .collect();
    let op = Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
        targets,
        translate: [Scalar::new(distance), Scalar::new(0.0), Scalar::new(0.0)],
        rotate: Default::default(),
        copy: false,
        extra: Default::default(),
    }));
    edit(
        state,
        session,
        AssemblyEdit::EditDefinition {
            id: document::definition(2),
            command: Box::new(EditCommand::AddOperation {
                record: OperationRecord::new(
                    RecordId(Uuid::from_u128(record)),
                    0,
                    "Translate native lever",
                    op,
                ),
                at_cursor: true,
            }),
        },
    )
    .await;
}

async fn translate_lever(state: &AppState, session: &str, record: u128, distance: f64) {
    apply_translation(state, session, record, distance).await;
    api::regenerate_component(state, session, document::definition(2))
        .await
        .unwrap();
}

async fn reopen(path: &Path, area: &Path) {
    verify_sources(path);
    let (h, session) = open(path, &area.join("fresh-host"), None).await;
    replay(&h.state, &session, &[0, 1, 2, 3]).await;
    let before = mesh(&h.state, &session, 2, 2).await;
    let independent = mesh(&h.state, &session, 3, 3).await;
    translate_lever(&h.state, &session, 0xA602, 1.0).await;
    let after = mesh(&h.state, &session, 2, 2).await;
    mesh_compare::different("reopen edit", before.as_slice(), after.as_slice());
    mesh_compare::equal(
        "reopen independent after edit",
        independent.as_slice(),
        mesh(&h.state, &session, 3, 3).await.as_slice(),
    );
    api::undo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, &session, &[2]).await;
    mesh_compare::equal(
        "reopen undo",
        before.as_slice(),
        mesh(&h.state, &session, 2, 2).await.as_slice(),
    );
    h.close().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prusa_mixed_native_assembly_cold_reopen() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("onecad_lib::document_runtime=debug")
        .with_test_writer()
        .try_init();
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
    let probe = admission::Probe::default();
    let (h, session) = open(&path, &area.path().join("main-host"), Some(&probe)).await;
    replay(&h.state, &session, &[0, 1, 2]).await;
    exercise(&h, &session, op, transverse_operation(&path, 3.0), &probe).await;
    assert!(
        api::save_document(&h.state, h.host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    h.close().await;
    let released = probe.snapshot();
    assert_eq!(released.guards, 0, "{released:?}");
    assert_eq!(released.bytes, 0, "{released:?}");
    verify_sources(&path);
    public_assembly_artifacts::retain(
        &path,
        "prusa-mixed-native-cover-lever",
        include_bytes!("prusa_mixed_assembly.rs"),
        "native cover32/lever8 histories and independent lever copy; pinned imported box; authored test placements, not physical fit qualification",
    );
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
async fn exercise(
    h: &Harness,
    session: &str,
    op: Operation,
    second_op: Operation,
    probe: &admission::Probe,
) {
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
    exercise_shared_edit(h, session, op, second_op, &before, probe).await;
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
    source_checks::assert_axis_line(
        pose.apply_point(lever_datum.frame.origin),
        pose.rotation.rotate(lever_datum.frame.z),
        box_datum.frame.origin,
        box_datum.frame.z,
    );
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
    probe: &admission::Probe,
) {
    edit_lever(&h.state, session, 2, op.clone()).await;
    let changed = mesh(&h.state, session, 2, 2).await;
    mesh_compare::different("shared edit", before.as_slice(), changed.as_slice());
    assert!(Arc::ptr_eq(&changed, &mesh(&h.state, session, 3, 2).await));
    api::undo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, session, &[2]).await;
    mesh_compare::equal(
        "undo shared edit",
        before.as_slice(),
        mesh(&h.state, session, 2, 2).await.as_slice(),
    );
    api::redo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, session, &[2]).await;
    mesh_compare::equal(
        "redo shared edit",
        changed.as_slice(),
        mesh(&h.state, session, 2, 2).await.as_slice(),
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
    mesh_compare::equal(
        "independent copy",
        changed.as_slice(),
        independent.as_slice(),
    );
    ambiguous_cut_refuses_rebind(
        h,
        session,
        second_op,
        changed.as_slice(),
        independent.as_slice(),
    )
    .await;
    admission::budgeted_translation(h, session, probe, independent.as_slice()).await;
    mesh_compare::different(
        "original after independent copy",
        independent.as_slice(),
        mesh(&h.state, session, 2, 2).await.as_slice(),
    );
    mesh_compare::equal(
        "independent after original edit",
        independent.as_slice(),
        mesh(&h.state, session, 3, 3).await.as_slice(),
    );
}
