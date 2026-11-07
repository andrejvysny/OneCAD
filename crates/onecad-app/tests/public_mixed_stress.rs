//! Public mixed-source assembly stress. Placements are test inputs, not physical assembly evidence.
#[path = "public_mixed_stress_support/checks.rs"]
mod checks;
#[allow(dead_code)]
#[path = "prusa_native_cover_support/mod.rs"]
mod cover;
#[path = "public_mixed_stress_support/fixture.rs"]
mod fixture;
#[path = "public_mixed_stress_support/framework_refusal.rs"]
mod framework_refusal;
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;
#[allow(dead_code)]
#[path = "prusa_native_lever_support/mod.rs"]
mod lever;
#[path = "prusa_mixed_support/mesh_compare.rs"]
mod mesh_compare;
#[path = "public_mixed_stress_support/native.rs"]
mod native;
#[allow(dead_code)]
#[path = "native_plate_support/profile.rs"]
mod profile;
#[path = "public_assembly_artifacts/mod.rs"]
mod public_assembly_artifacts;

use host::Harness;
use onecad_app::{api, dto::AssemblyProjectionDto, state::AppState};
use onecad_assembly::{
    math::{Quat, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId, OccurrencePath},
};
use onecad_core::{
    document::{
        component::ComponentDef,
        record::{KnownOperation, Operation, OperationRecord, TransformBodyParams},
        variables::Scalar,
        Document,
    },
    edit::{AssemblyEdit, EditCommand},
    ids::{BodyId, DocumentId, RecordId},
    io::container::ContainerReader,
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc, time::Duration};
use uuid::Uuid;

// Both cases own the process-wide auxiliary worker permit through teardown.
static WORKER_LANE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn open(path: &Path, root: &Path) -> (Harness, String) {
    let h = Harness::new(root).await;
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
async fn edit(state: &AppState, session: &str, command: AssemblyEdit) -> AssemblyProjectionDto {
    let p = api::get_assembly_projection(state, session).await.unwrap();
    api::apply_assembly_edit_at_generation(state, session, p.persisted_generation, command)
        .await
        .unwrap()
}
async fn replay(state: &AppState, session: &str, indices: &[usize]) {
    for index in indices {
        api::regenerate_component(state, session, fixture::def(*index))
            .await
            .unwrap_or_else(|error| panic!("definition {index} replay failed: {error:?}"));
    }
}
async fn meshes(
    state: &AppState,
    session: &str,
    index: usize,
    occurrence: usize,
) -> Vec<Arc<Vec<u8>>> {
    let p = api::get_assembly_projection(state, session).await.unwrap();
    let part = p
        .definitions
        .iter()
        .find(|part| part.id == fixture::def(index).to_string())
        .unwrap();
    assert!(
        !part.bodies.is_empty(),
        "definition {index} produced no solids"
    );
    let generation = part.mesh_generation.unwrap();
    let mut out = Vec::new();
    for body in &part.bodies {
        out.push(
            api::component_mesh_for_path(
                state,
                session,
                &OccurrencePath(vec![fixture::occ(occurrence)]),
                body.id.parse::<BodyId>().unwrap(),
                generation,
            )
            .await
            .unwrap(),
        );
    }
    out
}
async fn mesh_digests(state: &AppState, session: &str) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    for i in 0..11 {
        let occurrence = if i == 10 { 13 } else { 2 * i };
        let bodies = meshes(state, session, i, occurrence).await;
        result.push(
            bodies
                .iter()
                .map(|bytes| format!("{:x}", Sha256::digest(bytes.as_slice())))
                .collect(),
        );
    }
    result
}

fn stable_projection(p: &AssemblyProjectionDto) -> serde_json::Value {
    serde_json::json!({"occurrences": p.occurrences, "definitions": p.definitions.iter().map(|d| serde_json::json!({"id": d.id, "name": d.name, "bodies": d.bodies})).collect::<Vec<_>>()})
}
async fn verify_scene(state: &AppState, session: &str) {
    let projection = api::get_assembly_projection(state, session).await.unwrap();
    assert_eq!(projection.occurrences.len(), 20);
    assert!(projection.occurrences.iter().all(|row| row.visible));
    assert_eq!(projection.definitions.len(), 11); // Root plus ten source definitions.
    let mut total_triangles = 0usize;
    let mut total_bytes = 0usize;
    let mut total_bodies = 0usize;
    let mut total_segments = 0usize;
    let mut boxes = Vec::new();
    for i in 0..10 {
        let first = meshes(state, session, i, i * 2).await;
        let second = meshes(state, session, i, i * 2 + 1).await;
        assert_eq!(first.len(), second.len());
        assert_eq!(first.len(), 1, "source {i}");
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for (a, b) in first.iter().zip(&second) {
            assert!(
                Arc::ptr_eq(a, b),
                "definition {i} occurrences must share mesh"
            );
            let view = MeshView::decode(a, MeshLimits::default()).unwrap();
            total_triangles += 2 * view.header().triangle_count as usize;
            total_bytes += a.len();
            total_bodies += 2;
            for edge in 0..view.header().edge_count as usize {
                total_segments +=
                    2 * view.edge_range(edge).unwrap().count.saturating_sub(1) as usize;
            }
            for axis in 0..3 {
                lo[axis] = lo[axis].min(f64::from(view.header().bbox_min[axis]));
                hi[axis] = hi[axis].max(f64::from(view.header().bbox_max[axis]));
            }
        }
        checks::source_mesh(i, &first);
        for n in 0..2 {
            let row = projection
                .occurrences
                .iter()
                .find(|row| row.id == fixture::occ(2 * i + n).to_string())
                .unwrap();
            assert_eq!(row.definition, fixture::def(i).to_string());
            assert_eq!(row.grounded, i == 0 && n == 0);
            assert_eq!(row.local.rotation, Quat::IDENTITY);
            let t = row.local.translation;
            let offset = [t.x, t.y, t.z];
            boxes.push((
                [lo[0] + offset[0], lo[1] + offset[1], lo[2] + offset[2]],
                [hi[0] + offset[0], hi[1] + offset[1], hi[2] + offset[2]],
            ));
        }
    }
    assert!(
        total_triangles <= 500_000,
        "occurrence triangle total {total_triangles}"
    );
    assert!(
        total_bytes <= 256 * 1024 * 1024,
        "unique mesh payload bytes {total_bytes}"
    );
    assert!(
        total_segments <= 500_000,
        "visible edge segments {total_segments}"
    );
    assert!(total_bodies <= 4096, "occurrence bodies {total_bodies}");
    for i in 0..boxes.len() {
        for j in 0..i {
            assert!(
                (0..3).any(|axis| boxes[i].0[axis] > boxes[j].1[axis]
                    || boxes[j].0[axis] > boxes[i].1[axis]),
                "authored occurrence bounds overlap: {i}, {j}"
            );
        }
    }
    eprintln!("public mixed 10 definitions, 20 occurrences: bodies={total_bodies} visible_triangles={total_triangles} visible_segments={total_segments} unique_mesh_payload_bytes={total_bytes}; scene metadata/BVH not measured");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_mixed_ten_definitions_twenty_occurrences_cold_reopen() {
    let _worker_lane = WORKER_LANE.lock().await;
    host::required_worker();
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    if let Some(area) = std::env::var_os("ONECAD_PUBLIC_MIXED_CHILD") {
        child(Path::new(&area)).await;
        return;
    }
    let area = tempfile::tempdir().unwrap();
    checks::verify_nist(&area.path().join("nist-host")).await;
    let cover_path = area.path().join("cover.onecad");
    let lever_path = area.path().join("lever.onecad");
    let plate_path = area.path().join("plate.onecad");
    cover::build(&cover_path).await;
    lever::build(&lever_path).await;
    native::build_hole_plate(&plate_path, &area.path().join("plate-host")).await;
    let constrained_path = area.path().join("constrained.onecad");
    native::build_constrained(&constrained_path, &area.path().join("constrained-host")).await;
    let path = area.path().join("mixed.onecad");
    fixture::write(
        &path,
        &cover_path,
        &lever_path,
        &plate_path,
        &constrained_path,
    );
    let (h, session) = open(&path, &area.path().join("main-host")).await;
    replay(&h.state, &session, &(0..10).collect::<Vec<_>>()).await;
    verify_scene(&h.state, &session).await;
    checks::exercise(&h, &session, &lever_path).await;
    let expected = serde_json::json!({"meshes": mesh_digests(&h.state, &session).await, "projection": stable_projection(&api::get_assembly_projection(&h.state, &session).await.unwrap())});
    std::fs::write(
        area.path().join("expected.json"),
        serde_json::to_vec(&expected).unwrap(),
    )
    .unwrap();
    assert!(
        api::save_document(&h.state, h.host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    h.close().await;
    fixture::verify_saved(&path);
    public_assembly_artifacts::retain(&path, "public-mixed-10x20", include_bytes!("public_mixed_stress.rs"), "Six byte-pinned public imports: four Robonine analytic leaves, Prusa box and NIST FTC09 STEP; native histories: Prusa cover, Prusa lever, constrained plate and plate with hole. Authored placements are stress inputs, not a physical assembly.");
    fixture::retain_manifest();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "public_mixed_ten_definitions_twenty_occurrences_cold_reopen",
            "--nocapture",
        ])
        .env("ONECAD_PUBLIC_MIXED_CHILD", area.path())
        .status()
        .unwrap();
    assert!(status.success(), "cold process failed: {status}");
}
async fn child(area: &Path) {
    fixture::verify_saved(&area.join("mixed.onecad"));
    let (h, session) = open(&area.join("mixed.onecad"), &area.join("fresh-host")).await;
    replay(&h.state, &session, &(0..11).collect::<Vec<_>>()).await;
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(area.join("expected.json")).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(mesh_digests(&h.state, &session).await).unwrap(),
        expected["meshes"]
    );
    assert_eq!(
        stable_projection(
            &api::get_assembly_projection(&h.state, &session)
                .await
                .unwrap()
        ),
        expected["projection"]
    );
    let before = meshes(&h.state, &session, 6, 12).await;
    let independent = meshes(&h.state, &session, 10, 13).await;
    checks::translate(&h.state, &session, 0xD603, 1.0).await;
    let after = meshes(&h.state, &session, 6, 12).await;
    mesh_compare::different("cold edit", before[0].as_slice(), after[0].as_slice());
    mesh_compare::equal(
        "cold edit leaves copy unchanged",
        independent[0].as_slice(),
        meshes(&h.state, &session, 10, 13).await[0].as_slice(),
    );
    api::undo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, &session, &[6]).await;
    mesh_compare::equal(
        "cold undo",
        before[0].as_slice(),
        meshes(&h.state, &session, 6, 12).await[0].as_slice(),
    );
    h.close().await;
}
