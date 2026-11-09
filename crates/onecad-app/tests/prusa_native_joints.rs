//! Native cover and lever in a source-axis revolute fixture; not a physical fit test.
#[allow(dead_code)]
#[path = "prusa_native_cover_support/mod.rs"]
mod cover;
#[path = "prusa_native_joint_support/document.rs"]
mod document;
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;
#[path = "prusa_native_joint_support/joints.rs"]
mod joints;
#[allow(dead_code)]
#[path = "prusa_native_lever_support/mod.rs"]
mod lever;
#[allow(dead_code)]
#[path = "prusa_mixed_support/source_checks.rs"]
mod source_checks;

use host::Harness;
use onecad_app::{api, state::AppState};
use onecad_assembly::{
    joint::{JointKind, JointLimits},
    math::RigidTransform,
    occurrence::OccurrencePath,
};
use onecad_core::{
    document::record::{KnownOperation, Operation},
    ids::{BodyId, RecordId},
    io::container::ContainerReader,
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use std::{path::Path, time::Duration};
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
    for i in 0..3 {
        api::regenerate_component(&h.state, &opened.runtime_session, document::definition(i))
            .await
            .unwrap();
    }
    (h, opened.runtime_session)
}

fn verify_native_inputs(path: &Path) {
    let saved = ContainerReader::open(path).unwrap();
    assert_eq!(saved.import_blobs().len(), 1);
    for (index, count) in [(1, 32), (2, 8)] {
        let part = saved
            .document()
            .definitions
            .get(&document::definition(index))
            .unwrap();
        assert_eq!(part.timeline.records().len(), count);
        assert!(part
            .timeline
            .records()
            .iter()
            .all(|r| !matches!(r.op, Operation::Known(KnownOperation::ImportStep(_)))));
    }
    assert_eq!(saved.document().assembly.occurrences.iter().count(), 3);
}

async fn bindings_are_scoped(state: &AppState, session: &str) {
    let p = api::get_assembly_projection(state, session).await.unwrap();
    assert_eq!(p.joint_bindings.len(), 2);
    assert!(p
        .joint_bindings
        .iter()
        .all(|b| b.verified && !b.needs_repair));
    let bindings = api::get_joint_bindings(state, session, p.persisted_generation)
        .await
        .unwrap();
    for binding in &bindings {
        let (index, def) = match binding.side {
            onecad_core::document::component::JointSide::A => (0, 0),
            onecad_core::document::component::JointSide::B => (2, 2),
        };
        assert_eq!(binding.joint, joints::hinge_id());
        assert_eq!(
            binding.path,
            OccurrencePath(vec![document::occurrence(index)])
        );
        assert_eq!(binding.definition, document::definition(def));
    }
}

async fn assert_unchanged(
    state: &AppState,
    session: &str,
    expected: &onecad_app::dto::AssemblyProjectionDto,
) {
    let current = api::get_assembly_projection(state, session).await.unwrap();
    assert_eq!(current.persisted_generation, expected.persisted_generation);
    assert_eq!(joints::stable(&current), joints::stable(expected));
}

async fn child(path: &Path, area: &Path) {
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(area.join("expected.json")).unwrap()).unwrap();
    let (h, session) = open(path, &area.join("fresh-host")).await;
    let p = api::get_assembly_projection(&h.state, &session)
        .await
        .unwrap();
    assert_eq!(joints::stable(&p), expected);
    bindings_are_scoped(&h.state, &session).await;
    let report = joints::drive(&h.state, &session, -0.2).await.unwrap();
    assert_eq!(report.remaining_dof, 0);
    let updated = api::get_assembly_projection(&h.state, &session)
        .await
        .unwrap();
    let hinge = updated
        .joints
        .iter()
        .find(|j| j.id == joints::hinge_id())
        .unwrap();
    joints::aligned(&updated, hinge, -0.2);
    assert_eq!(
        joints::pose(&updated, document::occurrence(0)),
        RigidTransform::IDENTITY
    );
    joints::assert_pose_near(
        joints::pose(&updated, document::occurrence(1)),
        joints::pose(&p, document::occurrence(1)),
    );
    assert!(
        api::save_document(&h.state, h.host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_prusa_revolute_survives_history_and_fresh_worker() {
    host::required_worker();
    if let Some(area) = std::env::var_os("ONECAD_PRUSA_NATIVE_JOINT_CHILD") {
        let area = Path::new(&area);
        child(&area.join("assembly.onecad"), area).await;
        return;
    }
    let area = tempfile::tempdir().unwrap();
    let cover_path = area.path().join("cover.onecad");
    let lever_path = area.path().join("lever.onecad");
    cover::build(&cover_path).await;
    lever::build(&lever_path).await;
    let path = area.path().join("assembly.onecad");
    document::write(&path, &cover_path, &lever_path);
    verify_native_inputs(&path);
    let (h, session) = open(&path, &area.path().join("main-host")).await;
    let (joint_set, bound) = joints::fixtures(&h.state, &session).await;
    assert_eq!(
        joint_set
            .iter()
            .filter(|j| j.kind == JointKind::Revolute)
            .count(),
        1
    );
    let report = joints::solve(&h.state, &session, joint_set.clone(), bound, vec![])
        .await
        .unwrap();
    eprintln!("Prusa free solve: {report:?}");
    let initial = api::get_assembly_projection(&h.state, &session)
        .await
        .unwrap();
    eprintln!(
        "Prusa free solve joints={:?}, occurrences={:?}",
        initial.joints, initial.occurrences
    );
    assert_eq!(report.remaining_dof, 1);
    let rest = api::get_assembly_projection(&h.state, &session)
        .await
        .unwrap();
    bindings_are_scoped(&h.state, &session).await;
    let hinge = rest
        .joints
        .iter()
        .find(|j| j.id == joints::hinge_id())
        .unwrap()
        .clone();
    joints::aligned(&rest, &hinge, 0.0);
    let report = joints::drive(&h.state, &session, 0.25).await.unwrap();
    assert_eq!(report.remaining_dof, 0);
    let moved = api::get_assembly_projection(&h.state, &session)
        .await
        .unwrap();
    joints::aligned(&moved, &hinge, 0.25);
    assert_eq!(
        joints::pose(&moved, document::occurrence(0)),
        RigidTransform::IDENTITY
    );
    joints::assert_pose_near(
        joints::pose(&moved, document::occurrence(1)),
        joints::pose(&rest, document::occurrence(1)),
    );
    api::undo(&h.state, h.host.clone()).await.unwrap();
    assert_eq!(
        joints::stable(
            &api::get_assembly_projection(&h.state, &session)
                .await
                .unwrap()
        ),
        joints::stable(&rest)
    );
    api::redo(&h.state, h.host.clone()).await.unwrap();
    assert_eq!(
        joints::stable(
            &api::get_assembly_projection(&h.state, &session)
                .await
                .unwrap()
        ),
        joints::stable(&moved)
    );
    let endpoint = std::f64::consts::PI / 6.0;
    let report = joints::drive(&h.state, &session, endpoint).await.unwrap();
    assert_eq!(report.remaining_dof, 0);
    let at_limit = api::get_assembly_projection(&h.state, &session)
        .await
        .unwrap();
    joints::aligned(&at_limit, &hinge, endpoint);
    joints::assert_pose_near(
        joints::pose(&at_limit, document::occurrence(1)),
        joints::pose(&rest, document::occurrence(1)),
    );
    joints::drive(&h.state, &session, 0.25).await.unwrap();
    let moved = api::get_assembly_projection(&h.state, &session)
        .await
        .unwrap();
    joints::aligned(&moved, &hinge, 0.25);
    let mut invalid = moved.joints.clone();
    invalid
        .iter_mut()
        .find(|j| j.id == joints::hinge_id())
        .unwrap()
        .limits = Some(JointLimits { min: 1., max: -1. });
    let bound = api::get_joint_bindings(&h.state, &session, moved.persisted_generation)
        .await
        .unwrap();
    assert!(joints::solve(&h.state, &session, invalid, bound, vec![])
        .await
        .is_err());
    assert_unchanged(&h.state, &session, &moved).await;
    assert!(joints::drive(&h.state, &session, 1.0).await.is_err());
    assert_unchanged(&h.state, &session, &moved).await;
    std::fs::write(
        area.path().join("expected.json"),
        serde_json::to_vec(&joints::stable(&moved)).unwrap(),
    )
    .unwrap();
    assert!(
        api::save_document(&h.state, h.host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    h.close().await;
    verify_native_inputs(&path);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native_prusa_revolute_survives_history_and_fresh_worker",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ONECAD_PRUSA_NATIVE_JOINT_CHILD", area.path())
        .status()
        .unwrap();
    assert!(status.success(), "fresh process failed: {status}");
}

#[path = "prusa_native_joint_support/repair.rs"]
mod repair;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_prusa_hinge_face_deletion_preserves_repair_evidence() {
    repair::run().await;
}
