//! Permanent source deletion must preserve occurrence ownership during repair.
use super::*;

#[path = "deleted_body_repair/picking.rs"]
mod picking;
use crate::assembly_controls::{Action, OriginPick, Request as AssemblyRequest};
use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointOrigin},
    math::{Frame, Quat, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId},
};
use onecad_core::{
    document::component::{JointSide, OccurrencePath},
    edit::{AssemblyEdit, EditCommand},
    ids::RecordId,
};
use picking::pick;

const NEW_SOURCE: &str = "00000000-0000-0000-0000-00000000e505";
const STOCK: &str = "88ba8696-36ea-4b00-b2de-d7eb24f39eb4";

const DEFINITION: ComponentDefId = ComponentDefId(uuid::Uuid::from_u128(0xE501));
const A: OccurrenceId = OccurrenceId(uuid::Uuid::from_u128(0xE502));
const B: OccurrenceId = OccurrenceId(uuid::Uuid::from_u128(0xE503));
const JOINT: JointId = JointId(uuid::Uuid::from_u128(0xE504));

async fn command(
    backend: &mut Backend,
    session: &str,
    action: Action,
) -> Result<crate::assembly_controls::Reply, String> {
    let generation = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap()
        .persisted_generation;
    crate::assembly_controls::run(
        backend,
        AssemblyRequest {
            sequence: 1,
            session: session.into(),
            generation,
            cancellation: matches!(&action, Action::Solve { .. } | Action::SolveBound { .. })
                .then(|| Arc::new(std::sync::atomic::AtomicBool::new(false))),
            action,
        },
    )
    .await
}
async fn accept(backend: &mut Backend, session: &str, action: Action) {
    let reply = command(backend, session, action).await.unwrap();
    assert!(
        reply.capture_error.is_none(),
        "accepted edit did not refresh scene: {:?}",
        reply.capture_error
    );
}
fn occurrence(id: OccurrenceId, x: f64, grounded: bool) -> Occurrence {
    Occurrence {
        id,
        def: DEFINITION,
        parent: None,
        local: RigidTransform::new(Quat::IDENTITY, Vec3d::new(x, 0.0, 0.0)),
        grounded,
        name: format!("Instance {id}"),
    }
}

fn delete_source() -> Action {
    Action::Edit(AssemblyEdit::EditDefinition {
        id: DEFINITION,
        command: Box::new(EditCommand::RemoveOperation {
            record: "00000000-0000-0000-0000-00000000e505"
                .parse::<RecordId>()
                .unwrap(),
        }),
    })
}
fn poses(projection: &onecad_app::dto::AssemblyProjectionDto) -> Vec<RigidTransform> {
    projection.occurrences.iter().map(|row| row.local).collect()
}
fn assert_joint_coincident(projection: &onecad_app::dto::AssemblyProjectionDto) {
    let joint = &projection.joints[0];
    let world = |origin: &JointOrigin| {
        let pose = projection
            .occurrences
            .iter()
            .find(|row| row.id == origin.occurrence.to_string())
            .unwrap()
            .local;
        (
            pose.rotation.rotate(origin.frame.origin) + pose.translation,
            pose.rotation.rotate(origin.frame.x),
            pose.rotation.rotate(origin.frame.z),
        )
    };
    let (pa, xa, za) = world(&joint.a);
    let (pb, xb, zb) = world(&joint.b);
    let tolerance = onecad_assembly::solver::SolveRequest::new(vec![], vec![]).tolerance;
    for delta in [pa - pb, xa - xb, za - zb] {
        assert!(
            delta.x.abs() <= tolerance && delta.y.abs() <= tolerance && delta.z.abs() <= tolerance,
            "joint frames differ: {delta:?}"
        );
    }
}
async fn bindings(
    backend: &Backend,
    session: &str,
) -> Vec<onecad_core::document::component::JointBinding> {
    let p = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap();
    api::get_joint_bindings(&backend.state, session, p.persisted_generation)
        .await
        .unwrap()
}
fn solve(joint: Joint, picks: Vec<OriginPick>) -> Action {
    Action::SolveBound {
        joint: JOINT,
        joints: vec![joint],
        drives: vec![],
        picks,
        unbound: vec![],
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn deleted_body_repair_rejects_sibling_and_reopens() {
    if let Some(area) = std::env::var_os("ONECAD_DELETED_BODY_CHILD") {
        cold_reopen(std::path::Path::new(&area)).await;
        return;
    }
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let area = tempfile::tempdir().unwrap();
    let (mut backend, _) = backend(area.path()).await;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../onecad-app/tests/fixtures/qa_plate_hole.onecad");
    let path = backend
        .paths
        .root
        .join("documents/deleted-body-repair.onecad");
    fs::copy(source, &path).unwrap();
    intent(&mut backend, Request::Open(Some(path.clone())))
        .await
        .unwrap();
    let session = api::get_projection(&backend.state)
        .await
        .unwrap()
        .runtime_session;
    accept(
        &mut backend,
        &session,
        Action::Edit(AssemblyEdit::CopyRoot {
            id: DEFINITION,
            name: "Shared plate".into(),
        }),
    )
    .await;
    let source = onecad_core::io::container::ContainerReader::open(&path).unwrap();
    let mut record = source.document().timeline.records()[1].clone();
    record.record_id = NEW_SOURCE.parse().unwrap();
    record.name = "Independent source body".into();
    record.outputs.clear();
    if let onecad_core::document::record::Operation::Known(
        onecad_core::document::record::KnownOperation::Extrude(params),
    ) = &mut record.op
    {
        params.distance = onecad_core::document::variables::Scalar::new(20.0);
    } else {
        panic!("expected native stock extrude");
    }
    accept(
        &mut backend,
        &session,
        Action::Edit(AssemblyEdit::EditDefinition {
            id: DEFINITION,
            command: Box::new(EditCommand::AddOperation {
                record,
                at_cursor: false,
            }),
        }),
    )
    .await;
    for instance in [occurrence(A, 200.0, true), occurrence(B, 300.0, false)] {
        accept(
            &mut backend,
            &session,
            Action::Edit(AssemblyEdit::InsertOccurrence {
                occurrence: instance,
            }),
        )
        .await;
    }
    let (picked, frame) = pick(&mut backend, &session, A, true).await;
    let mut other = frame;
    other.origin = other.origin - Vec3d::new(100.0, 0.0, 0.0);
    let joint = Joint {
        id: JOINT,
        name: "Shared source face".into(),
        kind: JointKind::Rigid,
        a: JointOrigin {
            occurrence: A,
            frame,
        },
        b: JointOrigin {
            occurrence: B,
            frame: other,
        },
        limits: None,
        value: 0.0,
        suppressed: false,
    };
    accept(&mut backend, &session, solve(joint, vec![picked.clone()])).await;
    let before = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    let original = bindings(&backend, &session).await;
    assert_eq!(original.len(), 1);
    assert_eq!(original[0].path, OccurrencePath(vec![A]));
    assert!(!original[0].needs_repair);
    let (fresh_a, _) = pick(&mut backend, &session, A, true).await;
    let accepted_scene = backend
        .shared
        .display
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(fresh_a.selection.scene, accepted_scene.scene.identity());
    let deleted = command(&mut backend, &session, delete_source())
        .await
        .unwrap();
    assert!(
        deleted.capture_error.is_none(),
        "deleted-source capture failed: {:?}",
        deleted.capture_error
    );
    let damaged = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    let marked = bindings(&backend, &session).await;
    let mut expected = original.clone();
    expected[0].needs_repair = true;
    assert_eq!(marked, expected);
    assert_eq!(poses(&damaged), poses(&before));
    let stale = command(
        &mut backend,
        &session,
        solve(damaged.joints[0].clone(), vec![fresh_a]),
    )
    .await
    .unwrap_err();
    assert!(
        stale.contains("Selected face belongs to a stale scene"),
        "{stale}"
    );
    assert_eq!(bindings(&backend, &session).await, marked);
    let restored = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    assert_eq!(bindings(&backend, &session).await, marked);
    assert_eq!(poses(&restored), poses(&before));
    let (sibling, _) = pick(&mut backend, &session, B, false).await;
    let fresh = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    assert_eq!(bindings(&backend, &session).await, marked);
    assert_eq!(poses(&fresh), poses(&before));
    let error = command(
        &mut backend,
        &session,
        solve(fresh.joints[0].clone(), vec![sibling]),
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("Selected face belongs to another occurrence"),
        "{error}"
    );
    assert_eq!(bindings(&backend, &session).await, marked);
    assert_eq!(
        serde_json::to_value(
            api::get_assembly_projection(&backend.state, &session)
                .await
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(&fresh).unwrap()
    );
    let (repick, _) = pick(&mut backend, &session, A, false).await;
    let selected_body = repick.selection.source_body;
    let selected_element = repick.selection.topology_id.clone();
    accept(
        &mut backend,
        &session,
        solve(restored.joints[0].clone(), vec![repick]),
    )
    .await;
    let repaired = bindings(&backend, &session).await;
    assert_eq!(repaired.len(), 1);
    assert_eq!(repaired[0].path, OccurrencePath(vec![A]));
    assert_eq!(repaired[0].body, selected_body);
    assert_eq!(
        repaired[0]
            .element
            .primary
            .as_ref()
            .unwrap()
            .element
            .as_str(),
        selected_element
    );
    assert!(!repaired[0].needs_repair);
    assert_ne!(repaired[0].element, original[0].element);
    let post = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    let post_poses = poses(&post);
    assert_eq!(post_poses[0], poses(&before)[0]);
    assert_joint_coincident(&post);
    api::undo(&backend.state, backend.host.clone())
        .await
        .unwrap();
    assert_eq!(bindings(&backend, &session).await, marked);
    assert_eq!(
        poses(
            &api::get_assembly_projection(&backend.state, &session)
                .await
                .unwrap()
        ),
        poses(&before)
    );
    api::redo(&backend.state, backend.host.clone())
        .await
        .unwrap();
    assert_eq!(bindings(&backend, &session).await, repaired);
    assert_eq!(
        poses(
            &api::get_assembly_projection(&backend.state, &session)
                .await
                .unwrap()
        ),
        post_poses
    );
    assert!(
        api::save_document(
            &backend.state,
            backend.host.clone(),
            Some(path.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
    fs::write(
        area.path().join("saved-path"),
        path.to_string_lossy().as_bytes(),
    )
    .unwrap();
    fs::write(
        area.path().join("binding.json"),
        serde_json::to_vec(&repaired).unwrap(),
    )
    .unwrap();
    fs::write(
        area.path().join("poses.json"),
        serde_json::to_vec(&post_poses).unwrap(),
    )
    .unwrap();
    run_cold_process(area.path());
}

fn run_cold_process(area: &Path) {
    const NAME: &str = "document_services::tests::document_io_tests::deleted_body_repair::deleted_body_repair_rejects_sibling_and_reopens";
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--ignored", "--nocapture"])
        .env("ONECAD_DELETED_BODY_CHILD", area)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    // Keep nested harness records distinct from the strict parent inventory.
    for line in stdout.lines().chain(stderr.lines()) {
        eprintln!("fresh-process evidence: {line}");
    }
    assert!(
        output.status.success(),
        "fresh process failed: {}",
        output.status
    );
    assert_eq!(stdout.matches(&format!("test {NAME} ... ok")).count(), 1);
    assert_eq!(
        stdout
            .matches("test result: ok. 1 passed; 0 failed; 0 ignored;")
            .count(),
        1
    );
}

async fn cold_reopen(area: &Path) {
    let (mut backend, _) = backend(&area.join("cold")).await;
    let path = fs::read_to_string(area.join("saved-path")).unwrap();
    let copy = backend.paths.root.join("documents/cold.onecad");
    fs::copy(path, &copy).unwrap();
    intent(&mut backend, Request::Open(Some(copy)))
        .await
        .unwrap();
    let session = api::get_projection(&backend.state)
        .await
        .unwrap()
        .runtime_session;
    crate::services::load_ready_display(&mut backend)
        .await
        .unwrap();
    let expected: Vec<onecad_core::document::component::JointBinding> =
        serde_json::from_slice(&fs::read(area.join("binding.json")).unwrap()).unwrap();
    assert_eq!(bindings(&backend, &session).await, expected);
    let expected_poses: Vec<RigidTransform> =
        serde_json::from_slice(&fs::read(area.join("poses.json")).unwrap()).unwrap();
    let projection = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    assert_eq!(poses(&projection), expected_poses);
    assert_joint_coincident(&projection);
    let edited = super::constrained_reopen::move_free_instance(
        &mut backend,
        &session,
        B,
        occurrence(OccurrenceId(uuid::Uuid::from_u128(0xE506)), 0.0, false),
        RigidTransform::new(Quat::IDENTITY, Vec3d::new(301.0, 0.0, 0.0)),
    )
    .await;
    assert_eq!(bindings(&backend, &session).await, expected);
    assert_joint_coincident(&edited);
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}
