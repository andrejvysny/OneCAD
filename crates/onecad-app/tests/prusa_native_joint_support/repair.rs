//! Actual native hinge-face deletion; independent from synthetic reservations.
use crate::{cover, document, host, joints, lever, open};
use onecad_app::api;
use onecad_assembly::math::Vec3d;
use onecad_core::{
    document::component::JointSide,
    edit::{AssemblyEdit, EditCommand},
    ids::{ElementId, RecordId},
    io::container::ContainerReader,
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use serde_json::{json, Value};
use std::path::Path;
use uuid::Uuid;

const TEST: &str = "native_prusa_hinge_face_deletion_preserves_repair_evidence";

async fn save(h: &host::Harness) {
    assert!(
        api::save_document(&h.state, h.host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
}

fn evidence(path: &Path, element: &ElementId) -> Value {
    let loaded = ContainerReader::open(path).unwrap();
    let part = &loaded.document().definitions[&document::definition(2)];
    let entry = part.elements.get(element).unwrap();
    json!({"entry":entry,"bindings":loaded.document().assembly.joint_bindings,
        "occurrences":loaded.document().assembly.occurrences})
}

async fn cold(area: &Path) {
    let path = area.join("assembly.onecad");
    let element = ElementId::new(std::fs::read_to_string(area.join("element.txt")).unwrap());
    let expected: Value =
        serde_json::from_slice(&std::fs::read(area.join("expected.json")).unwrap()).unwrap();
    assert_eq!(evidence(&path, &element), expected);
    let phase = std::env::var("ONECAD_PRUSA_REPAIR_PHASE").unwrap();
    let (h, session) = open(&path, &area.join(format!("cold-{phase}"))).await;
    let p = api::get_assembly_projection(&h.state, &session)
        .await
        .unwrap();
    if phase == "unrepaired" {
        let bindings = api::get_joint_bindings(&h.state, &session, p.persisted_generation)
            .await
            .unwrap();
        let marked = bindings
            .iter()
            .find(|binding| binding.joint == joints::hinge_id() && binding.side == JointSide::B)
            .unwrap();
        assert!(marked.needs_repair);
        assert_eq!(marked.definition, document::definition(2));
        assert_eq!(marked.path.0, vec![document::occurrence(2)]);
        assert_eq!(marked.element.primary.as_ref().unwrap().element, element);
        assert_eq!(
            serde_json::to_value(&bindings).unwrap(),
            expected["bindings"]
        );
        let before_drive = snapshot(&h, &session).await;
        assert!(joints::drive(&h.state, &session, 0.2).await.is_err());
        assert_eq!(snapshot(&h, &session).await, before_drive);
        assert_eq!(
            api::get_assembly_projection(&h.state, &session)
                .await
                .unwrap()
                .persisted_generation,
            p.persisted_generation
        );
        repair(&h, &session).await;
    } else {
        assert_eq!(phase, "repaired");
        assert!(p
            .joint_bindings
            .iter()
            .all(|binding| binding.verified && !binding.needs_repair));
        let hinge = p
            .joints
            .iter()
            .find(|joint| joint.id == joints::hinge_id())
            .unwrap();
        joints::aligned(&p, hinge, 0.0);
        assert_eq!(
            joints::drive(&h.state, &session, 0.2)
                .await
                .unwrap()
                .remaining_dof,
            0
        );
        let moved = api::get_assembly_projection(&h.state, &session)
            .await
            .unwrap();
        joints::aligned(&moved, hinge, 0.2);
        assert_eq!(
            joints::pose(&moved, document::occurrence(0)),
            joints::pose(&p, document::occurrence(0))
        );
    }
    save(&h).await;
    h.close().await;
    let current = evidence(&path, &element);
    assert_eq!(current["entry"], expected["entry"]);
    std::fs::write(
        area.join("expected.json"),
        serde_json::to_vec(&current).unwrap(),
    )
    .unwrap();
}

pub async fn run() {
    host::required_worker();
    if let Some(area) = std::env::var_os("ONECAD_PRUSA_REPAIR_CHILD") {
        cold(Path::new(&area)).await;
        return;
    }
    let area = tempfile::tempdir().unwrap();
    let path = area.path().join("assembly.onecad");
    let (cover_path, lever_path) = (
        area.path().join("cover.onecad"),
        area.path().join("lever.onecad"),
    );
    cover::build(&cover_path).await;
    lever::build(&lever_path).await;
    document::write(&path, &cover_path, &lever_path);
    let (h, session) = open(&path, &area.path().join("parent")).await;
    let (joint_set, bindings) = joints::fixtures(&h.state, &session).await;
    joints::solve(&h.state, &session, joint_set, bindings, vec![])
        .await
        .unwrap();
    save(&h).await;
    delete(&h, &session, &path, area.path()).await;
    h.close().await;
    for phase in ["unrepaired", "repaired"] {
        launch(area.path(), phase);
    }
}

fn launch(area: &Path, phase: &str) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env("ONECAD_PRUSA_REPAIR_CHILD", area)
        .env("ONECAD_PRUSA_REPAIR_PHASE", phase)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    for line in stdout.lines().chain(stderr.lines()) {
        eprintln!("fresh-process evidence: {line}");
    }
    assert!(
        output.status.success(),
        "fresh repair process failed: {}",
        output.status
    );
    assert_eq!(stdout.matches(&format!("test {TEST} ... ok")).count(), 1);
}

async fn delete(h: &host::Harness, session: &str, path: &Path, area: &Path) {
    let before = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    let bindings = api::get_joint_bindings(&h.state, session, before.persisted_generation)
        .await
        .unwrap();
    let binding = bindings
        .iter()
        .find(|binding| binding.side == JointSide::B)
        .unwrap();
    let element = binding.element.primary.as_ref().unwrap().element.clone();
    let old = evidence(path, &element)["entry"].clone();
    api::apply_assembly_edit_at_generation(
        &h.state,
        session,
        before.persisted_generation,
        AssemblyEdit::EditDefinition {
            id: document::definition(2),
            command: Box::new(EditCommand::RemoveOperation {
                record: RecordId(Uuid::from_u128(0x531000 + 22)),
            }),
        },
    )
    .await
    .unwrap();
    let (_, changed) = api::regenerate_component(&h.state, session, document::definition(2))
        .await
        .unwrap();
    assert_eq!(
        changed
            .occurrences
            .iter()
            .map(|o| o.local)
            .collect::<Vec<_>>(),
        before
            .occurrences
            .iter()
            .map(|o| o.local)
            .collect::<Vec<_>>()
    );
    assert!(changed
        .joint_bindings
        .iter()
        .any(|binding| binding.needs_repair));
    let mut expected_binding = binding.clone();
    expected_binding.needs_repair = true;
    let marked = api::get_joint_bindings(&h.state, session, changed.persisted_generation)
        .await
        .unwrap();
    assert_eq!(
        marked
            .iter()
            .find(|row| row.joint == joints::hinge_id() && row.side == JointSide::B)
            .unwrap(),
        &expected_binding
    );
    save(h).await;
    let current = evidence(path, &element);
    let mut expected_entry = old;
    expected_entry["topologyBindingState"] = json!("quarantined");
    assert_eq!(current["entry"], expected_entry);
    std::fs::write(area.join("element.txt"), element.as_str()).unwrap();
    std::fs::write(
        area.join("expected.json"),
        serde_json::to_vec(&current).unwrap(),
    )
    .unwrap();
}

async fn snapshot(h: &host::Harness, session: &str) -> Value {
    let p = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    let bindings = api::get_joint_bindings(&h.state, session, p.persisted_generation)
        .await
        .unwrap();
    json!({"joints":p.joints,"occurrences":p.occurrences,"bindings":bindings})
}

async fn repair(h: &host::Harness, session: &str) {
    let datum = repick(h, session).await;
    let before = snapshot(h, session).await;
    let p = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    let mut bindings = api::get_joint_bindings(&h.state, session, p.persisted_generation)
        .await
        .unwrap();
    let binding = bindings
        .iter_mut()
        .find(|binding| binding.side == JointSide::B)
        .unwrap();
    assert!(binding.needs_repair);
    assert_ne!(binding.element, datum.reference);
    binding.element = datum.reference;
    binding.surface = datum.surface;
    binding.radius = datum.radius;
    binding.orientation_x = datum.frame.x;
    binding.needs_repair = false;
    let ground = joints::pose(&p, document::occurrence(0));
    let mut joint_set = p.joints;
    let hinge = joint_set
        .iter_mut()
        .find(|joint| joint.id == joints::hinge_id())
        .unwrap();
    hinge.b.frame = datum.frame;
    assert_eq!(
        joints::solve(&h.state, session, joint_set, bindings, vec![])
            .await
            .unwrap()
            .remaining_dof,
        1
    );
    let repaired = snapshot(h, session).await;
    let current = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    assert!(current
        .joint_bindings
        .iter()
        .all(|binding| binding.verified && !binding.needs_repair));
    let hinge = current
        .joints
        .iter()
        .find(|joint| joint.id == joints::hinge_id())
        .unwrap();
    free_angle_within_limits(&current, hinge);
    assert_eq!(joints::pose(&current, document::occurrence(0)), ground);
    api::undo(&h.state, h.host.clone()).await.unwrap();
    assert_eq!(snapshot(h, session).await, before);
    api::redo(&h.state, h.host.clone()).await.unwrap();
    assert_eq!(snapshot(h, session).await, repaired);
    assert_eq!(
        joints::drive(&h.state, session, 0.0)
            .await
            .unwrap()
            .remaining_dof,
        0
    );
    let driven = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    let hinge = driven
        .joints
        .iter()
        .find(|joint| joint.id == joints::hinge_id())
        .unwrap();
    joints::aligned(&driven, hinge, 0.0);
    assert_eq!(joints::pose(&driven, document::occurrence(0)), ground);
}

async fn repick(h: &host::Harness, session: &str) -> api::JointOriginResolution {
    let p = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    let part = p
        .definitions
        .iter()
        .find(|part| part.id == document::definition(2).to_string())
        .unwrap();
    let body = part.bodies[0].id.parse().unwrap();
    let path = onecad_core::document::component::OccurrencePath(vec![document::occurrence(2)]);
    let generation = part.mesh_generation.unwrap();
    let mesh = api::component_mesh_for_path(&h.state, session, &path, body, generation)
        .await
        .unwrap();
    let checked = MeshView::decode(&mesh, MeshLimits::default()).unwrap();
    assert!(checked.header().face_count < 64);
    for face in 0..checked.header().face_count as usize {
        let id = checked.face_id(face).unwrap();
        for x in [Vec3d::X, Vec3d::Y] {
            if let Ok(datum) = api::component_joint_origin(
                &h.state,
                api::JointOriginRequest {
                    session,
                    generation: p.persisted_generation,
                    path: &path,
                    body,
                    mesh_generation: generation,
                    element: id,
                    x,
                },
            )
            .await
            {
                return datum;
            }
        }
    }
    panic!("native lever has no current supported joint datum");
}

fn free_angle_within_limits(
    p: &onecad_app::dto::AssemblyProjectionDto,
    hinge: &onecad_assembly::joint::Joint,
) {
    let world = |origin: &onecad_assembly::joint::JointOrigin| {
        joints::pose(p, origin.occurrence).compose(&origin.frame.to_transform().unwrap())
    };
    let a = world(&hinge.a);
    let b = world(&hinge.b);
    let (za, xa, xb) = (
        a.apply_vector(Vec3d::Z),
        a.apply_vector(Vec3d::X),
        b.apply_vector(Vec3d::X),
    );
    let angle = za.dot(xa.cross(xb)).atan2(xa.dot(xb));
    let limits = hinge.limits.unwrap();
    assert!(
        angle >= limits.min - 1e-9 && angle <= limits.max + 1e-9,
        "{angle}"
    );
    joints::aligned(p, hinge, angle);
}
