//! Original native QA fixture: truthful disconnected-opening refusal and a
//! connected boss-cap workflow through real preview, commit and persistence.
use super::*;
use onecad_app::worker::ElementQuery;
use onecad_protocol::mesh::{MeshLimits, MeshView};

fn fixture_body() -> BodyId {
    BodyId(Uuid::parse_str("88ba8696-36ea-4b00-b2de-d7eb24f39eb4").unwrap())
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qa_plate_hole.onecad")
}

fn face_at_z(blob: &[u8], z: f64) -> (TopoKey, Vec3) {
    let mesh = MeshView::decode(blob, MeshLimits::default()).expect("checked fixture mesh");
    for face in 0..mesh.header().face_count as usize {
        let span = mesh.face_range(face).unwrap();
        let points: Vec<_> = (span.first..span.first + span.count)
            .flat_map(|triangle| mesh.triangle(triangle as usize).unwrap())
            .map(|vertex| mesh.position(vertex as usize).unwrap())
            .collect();
        if !points.is_empty()
            && points
                .iter()
                .all(|point| (f64::from(point[2]) - z).abs() < 1e-5)
        {
            let first = &points[..3];
            let center = std::array::from_fn::<_, 3, _>(|axis| {
                first
                    .iter()
                    .map(|point| f64::from(point[axis]))
                    .sum::<f64>()
                    / 3.0
            });
            return (
                TopoKey::new(mesh.face_id(face).unwrap()),
                Vec3::new_unchecked(center[0], center[1], center[2]),
            );
        }
    }
    panic!("horizontal fixture face at z={z}");
}

async fn promote_face(rt: &mut DocumentRuntime, blob: &[u8], z: f64) -> ElementRef {
    let (key, at) = face_at_z(blob, z);
    if !key.is_valid() {
        assert!(
            key.as_str().starts_with("el_"),
            "persistent fixture face id"
        );
        return anchored_ref(
            fixture_body(),
            &ElementId::new(key.as_str()),
            ElementKind::Face,
            at,
        );
    }
    let anchor = AnchorIntent {
        world_point: at,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    };
    let promoted = rt
        .promote_selection(
            rt.head_snapshot_id().unwrap(),
            fixture_body(),
            vec![(key, Some(anchor))],
        )
        .await
        .expect("promote actual fixture face");
    anchored_ref(
        fixture_body(),
        &ElementId::new(&promoted[0].element_id),
        ElementKind::Face,
        at,
    )
}

async fn mass(wm: &WorkerManager) -> f64 {
    wm.query_mass_properties(fixture_body(), fixture_body().to_string())
        .await
        .expect("exact BRep mass")
        .volume
}

fn assert_mesh_volume(blob: &[u8], expected: f64, tolerance: f64) {
    let view = validate_mesh_blob(blob).expect("checked Shell mesh");
    let volume = mesh_volume(&view, blob);
    assert!(
        (volume - expected).abs() < tolerance,
        "mesh volume {volume}, expected {expected}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_broad_top_refuses_disconnected_skin_without_mutation() {
    let Some(bin) = real_worker() else { return };
    let wm = spawn_worker(bin).await;
    let mut rt = open_over(&wm, &fixture_path());
    let report = clean_regen(&mut rt).await;
    assert_eq!(
        published(&report, "original QA fixture")
            .repair_summary
            .needs_repair_count,
        0
    );
    let original_mesh = body_mesh(&mut rt, fixture_body()).await;
    let face = promote_face(&mut rt, &original_mesh, 4.0).await;
    let original_mass = mass(&wm).await;
    assert!((original_mass - (9600.0 + 36.0 * std::f64::consts::PI)).abs() < 1e-6);
    let revision = rt.revision();
    let snapshot = rt.head_snapshot_id();
    for thickness in [0.5, 1.0, 2.0, 3.0] {
        let candidate = shell_record(0x484, fixture_body(), vec![face.clone()], thickness);
        let error = wm
            .preview_op(
                candidate.op,
                candidate.record_id.to_string(),
                None,
                None,
                Lod::Coarse,
            )
            .await
            .expect_err("broad opening disconnects retained boss skin");
        assert!(
            error
                .to_string()
                .contains("Selected opening disconnects the remaining shell."),
            "{error}"
        );
        assert_eq!(
            mass(&wm).await,
            original_mass,
            "failed preview preserves worker head"
        );
        assert_eq!(rt.revision(), revision);
        assert_eq!(rt.head_snapshot_id(), snapshot);
    }
    assert_eq!(*body_mesh(&mut rt, fixture_body()).await, *original_mesh);
    wm.shutdown().await;
}

async fn assert_committed_cavity(rt: &mut DocumentRuntime, wm: &WorkerManager, volume: f64) {
    assert!(
        (mass(wm).await - volume).abs() < 1e-6,
        "exact committed mass"
    );
    let topology = wm
        .query_body_topology(fixture_body(), fixture_body().to_string())
        .await
        .unwrap();
    assert_eq!(topology.solid_count, 1);
    assert!(topology.face_count > 9, "inner walls exist");
    assert_mesh_volume(&body_mesh(rt, fixture_body()).await, volume, 2.0);
}

async fn replay_healthy(rt: &mut DocumentRuntime, label: &str) {
    assert_eq!(
        published(&clean_regen(rt).await, label)
            .repair_summary
            .needs_repair_count,
        0
    );
}

async fn undo_redo_shell(rt: &mut DocumentRuntime, wm: &WorkerManager, original: f64, hollow: f64) {
    rt.undo().expect("Undo Shell");
    replay_healthy(rt, "Undo").await;
    assert!((mass(wm).await - original).abs() < 1e-6);
    rt.redo().expect("Redo Shell");
    replay_healthy(rt, "Redo").await;
    assert_committed_cavity(rt, wm, hollow).await;
}

async fn save_reopen_shell(
    rt: &mut DocumentRuntime,
    wm: &WorkerManager,
    bin: PathBuf,
    volume: f64,
) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("qa-shell.onecad");
    rt.save(&path, save_meta()).expect("save typed Shell");
    wm.shutdown().await;
    let reopened_worker = spawn_worker(bin).await;
    let mut reopened = open_over(&reopened_worker, &path);
    replay_healthy(&mut reopened, "cold reopen").await;
    assert_committed_cavity(&mut reopened, &reopened_worker, volume).await;
    reopened_worker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_boss_cap_shell_previews_commits_undoes_and_reopens() {
    let Some(bin) = real_worker() else { return };
    let wm = spawn_worker(bin.clone()).await;
    let mut rt = open_over(&wm, &fixture_path());
    replay_healthy(&mut rt, "original QA fixture").await;
    let original_mesh = body_mesh(&mut rt, fixture_body()).await;
    let original_mass = mass(&wm).await;
    let face = promote_face(&mut rt, &original_mesh, 12.0).await;
    let candidate = shell_record(0x485, fixture_body(), vec![face], 1.0);
    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            None,
            None,
            Lod::Coarse,
        )
        .await
        .expect("boss cap opens a connected shell");
    assert!(preview.needs_repair.is_empty());
    assert_eq!(
        mass(&wm).await,
        original_mass,
        "valid preview preserves head"
    );
    let preview_mesh = &preview.bodies[0].mesh;
    add_op(&mut rt, candidate);
    replay_healthy(&mut rt, "Shell commit").await;
    let hollow_mass = mass(&wm).await;
    assert!(
        hollow_mass > 0.0 && hollow_mass < original_mass * 0.75,
        "real cavity removes material"
    );
    assert_mesh_volume(preview_mesh, hollow_mass, 2.0);
    assert_committed_cavity(&mut rt, &wm, hollow_mass).await;
    undo_redo_shell(&mut rt, &wm, original_mass, hollow_mass).await;
    save_reopen_shell(&mut rt, &wm, bin, hollow_mass).await;
}
