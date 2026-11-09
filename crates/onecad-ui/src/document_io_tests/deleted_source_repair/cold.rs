//! Fresh process and worker must preserve repaired intent and reserved identities.
use super::*;
use onecad_core::{document::element_index::ElementEntry, ids::ElementId};
use std::collections::BTreeMap;

fn reserved(path: &Path) -> BTreeMap<ElementId, ElementEntry> {
    let loaded = onecad_core::io::container::ContainerReader::open(path).unwrap();
    loaded.document().definitions[&DEFINITION]
        .elements
        .iter()
        .filter(|(_, entry)| entry.is_quarantined())
        .map(|(id, entry)| (id.clone(), entry.clone()))
        .collect()
}

pub(super) fn store_reserved(
    path: &Path,
    area: &Path,
    original: &onecad_core::document::refs::ElementRef,
) {
    let entries = reserved(path);
    let primary = original.primary.as_ref().unwrap();
    let old = entries.get(&primary.element).unwrap();
    assert_eq!(old.body, primary.body);
    assert_eq!(old.kind, primary.kind);
    assert_eq!(old.anchor, original.anchor);
    assert_eq!(
        old.descriptor.as_ref(),
        original.intent.as_ref().map(|intent| &intent.descriptor)
    );
    assert_eq!(old.import_source_identity, original.import_source_identity);
    assert!(!old.has_rebind_evidence());
    fs::write(
        area.join("reserved.json"),
        serde_json::to_vec(&entries).unwrap(),
    )
    .unwrap();
}

pub(super) fn run(area: &Path) {
    const NAME: &str = "document_services::tests::document_io_tests::deleted_source_repair::deleted_source_repair_rejects_sibling_and_reopens";
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--ignored", "--nocapture"])
        .env("ONECAD_DELETED_SOURCE_CHILD", area)
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

pub(super) async fn check_unrepaired(
    backend: &mut Backend,
    path: &Path,
    area: &Path,
    original: &onecad_core::document::refs::ElementRef,
) {
    let projection = api::get_projection(&backend.state).await.unwrap();
    let bindings = bindings(backend, &projection.runtime_session).await;
    assert!(bindings.iter().any(|binding| binding.needs_repair));
    let poses = poses(
        &api::get_assembly_projection(&backend.state, &projection.runtime_session)
            .await
            .unwrap(),
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
    let copy_area = area.join("unrepaired");
    fs::create_dir(&copy_area).unwrap();
    store_reserved(path, &copy_area, original);
    fs::write(
        copy_area.join("saved-path"),
        path.to_string_lossy().as_bytes(),
    )
    .unwrap();
    fs::write(
        copy_area.join("binding.json"),
        serde_json::to_vec(&bindings).unwrap(),
    )
    .unwrap();
    fs::write(
        copy_area.join("poses.json"),
        serde_json::to_vec(&poses).unwrap(),
    )
    .unwrap();
    run(&copy_area);
}

pub(super) async fn reopen(area: &Path) {
    let (mut backend, _) = backend(&area.join("cold")).await;
    let path = fs::read_to_string(area.join("saved-path")).unwrap();
    let copy = backend.paths.root.join("documents/cold.onecad");
    fs::copy(path, &copy).unwrap();
    let expected_reserved: BTreeMap<ElementId, ElementEntry> =
        serde_json::from_slice(&fs::read(area.join("reserved.json")).unwrap()).unwrap();
    assert!(!expected_reserved.is_empty());
    assert_eq!(reserved(&copy), expected_reserved);
    intent(&mut backend, Request::Open(Some(copy.clone())))
        .await
        .unwrap();
    let session = api::get_projection(&backend.state)
        .await
        .unwrap()
        .runtime_session;
    let display = crate::services::load_ready_display(&mut backend)
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
    for body in display.scene.bodies() {
        let mesh = body.mesh().unwrap();
        for row in 0..mesh.header().face_count as usize {
            assert!(!expected_reserved.contains_key(&ElementId::new(mesh.face_id(row).unwrap())));
        }
    }
    if expected.iter().any(|binding| binding.needs_repair) {
        repair_after_reopen(
            &mut backend,
            &session,
            &projection,
            &expected_reserved,
            &copy,
        )
        .await;
        return;
    }
    let edited = super::super::constrained_reopen::move_free_instance(
        &mut backend,
        &session,
        B,
        occurrence(OccurrenceId(uuid::Uuid::from_u128(0xE506)), 0.0, false),
        RigidTransform::new(Quat::IDENTITY, Vec3d::new(301.0, 0.0, 0.0)),
    )
    .await;
    assert_eq!(bindings(&backend, &session).await, expected);
    assert_joint_coincident(&edited);
    assert!(
        api::save_document(
            &backend.state,
            backend.host.clone(),
            Some(copy.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    assert_eq!(reserved(&copy), expected_reserved);
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}

async fn repair_after_reopen(
    backend: &mut Backend,
    session: &str,
    before: &onecad_app::dto::AssemblyProjectionDto,
    reserved_entries: &BTreeMap<ElementId, ElementEntry>,
    path: &Path,
) {
    let (fresh, _) = pick(backend, session, A, false).await;
    let selected = fresh.selection.topology_id.clone();
    assert!(!reserved_entries.contains_key(&ElementId::new(&selected)));
    accept(
        backend,
        session,
        solve(before.joints[0].clone(), vec![fresh]),
    )
    .await;
    let repaired = bindings(backend, session).await;
    assert_eq!(repaired[0].path, OccurrencePath(vec![A]));
    assert!(!repaired[0].needs_repair);
    assert_eq!(
        repaired[0]
            .element
            .primary
            .as_ref()
            .unwrap()
            .element
            .as_str(),
        selected
    );
    let after = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap();
    assert_eq!(poses(&after)[0], poses(before)[0]);
    assert_joint_coincident(&after);
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
    assert_eq!(&reserved(path), reserved_entries);
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}
