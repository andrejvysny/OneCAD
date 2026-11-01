//! Seeded standard-part families against the REAL C++ OCCT worker: for every
//! package `seed_library` ships (ISO 4762 screw, ISO 4032 nut, ISO 7089 washer)
//! a placement through the application's `place_component_at` — default params
//! and M10 — is regenerated, meshed, and its exact `QueryMassProperties` volume
//! compared to an analytic oracle typed here from the BOLTS columns (never read
//! out of the Rust or C++ tables). The document is then saved and reopened on a
//! FRESH worker and must keep the same volume.
//!
//! A second test mates the washer (Coincident on `seat`) and the nut
//! (ConcentricAndCoincident on `bore_axis`) onto a bored plate's top.
//!
//! Chamfers on the real nut and the hex socket of the screw are not modeled
//! (documented at the generators), so they are not in the oracles either.
//!
//! Gated on `ONECAD_WORKER_PATH`; `ONECAD_REQUIRE_WORKER=1` makes a missing
//! binary a hard failure.

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use onecad_core::document::record::ComponentParamValue;
use onecad_core::document::record::OperationRecord;
use onecad_core::ids::{BodyId, RecordId, SketchId};
use onecad_core::io::container::SaveMeta;
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use uuid::Uuid;

use onecad_app::api;
use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::library::{place_component_at, promote_mate_target, reindex_library_at};
use onecad_app::state::AppState;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (copied minimal from component_placement_pipeline.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} is set but no worker binary exists there \
             (misconfiguration — refusing to skip as green)"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved (CI must hard-fail here)"
    );
    None
}

async fn spawn_worker(bin: PathBuf) -> WorkerManager {
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "real worker must connect + handshake + OpenSession"
    );
    wm
}

fn app_state_over(wm: &WorkerManager) -> AppState {
    let wm = wm.clone();
    AppState::new(Arc::new(move || {
        let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
        let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
        let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
        let exporter: Arc<dyn onecad_app::export::GeometryExporter> = Arc::new(wm.clone());
        let elements: Arc<dyn ElementQuery> = Arc::new(wm.clone());
        let preview: Arc<dyn PreviewEngine> = Arc::new(wm.clone());
        let face_projection: Arc<dyn FaceBoundaryProjection> = Arc::new(wm.clone());
        let step_import: Arc<dyn StepImport> = Arc::new(wm.clone());
        let circuit: Arc<dyn CircuitControl> = Arc::new(wm.clone());
        let readiness: Arc<dyn WorkerReadiness> = Arc::new(wm.clone());
        (
            engine,
            meshes,
            solver,
            exporter,
            elements,
            preview,
            face_projection,
            step_import,
            circuit,
            readiness,
        )
    }))
}

async fn open_blank(state: &AppState) {
    let (engine, meshes, solver) = state.make_backend();
    *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
}

fn open_over(wm: &WorkerManager, path: &Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen the saved container")
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-8.0.1".into()),
        created: "2026-11-01T00:00:00Z".into(),
        modified: "2026-11-01T00:00:00Z".into(),
    }
}

/// Regens the open document to its end and returns the published snapshot.
async fn regen(state: &AppState) -> Arc<ModelSnapshot> {
    let mut guard = state.runtime.lock().await;
    let rt = guard.as_mut().expect("open document");
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    match report.outcome {
        Outcome::Published(s) => s,
        other => panic!("expected Published, got {other:?}"),
    }
}

async fn exact_volume(wm: &WorkerManager, body: BodyId) -> f64 {
    ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties")
        .volume
}

fn mesh_bbox(blob: &[u8]) -> ([f64; 3], [f64; 3]) {
    let mesh = MeshView::decode(blob, MeshLimits::default()).expect("MESH1");
    let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for i in 0..mesh.header().vertex_count as usize {
        let p = mesh.position(i).expect("vertex");
        for a in 0..3 {
            min[a] = min[a].min(f64::from(p[a]));
            max[a] = max[a].max(f64::from(p[a]));
        }
    }
    (min, max)
}

async fn committed_bbox(state: &AppState, body: BodyId) -> ([f64; 3], [f64; 3]) {
    let blob = api::get_mesh(state, body.to_string(), "fine".into(), None, None)
        .await
        .expect("committed mesh");
    mesh_bbox(&blob)
}

// ─────────────────────────────────────────────────────────────────────────────
// Oracles
// ─────────────────────────────────────────────────────────────────────────────

/// What a placement should measure: exact volume and the local-frame z extent.
struct Oracle {
    volume: f64,
    z_min: f64,
    z_max: f64,
}

/// ISO 4762 at the default length 20 mm: head `π(dk/2)²k` + shank `π(d/2)²·L`.
/// `(dk, k, d)` from BOLTS `hexsocketheadcap`.
fn screw(dk: f64, k: f64, d: f64) -> Oracle {
    let l = 20.0;
    Oracle {
        volume: PI * (dk / 2.0).powi(2) * k + PI * (d / 2.0).powi(2) * l,
        z_min: -l,
        z_max: k,
    }
}

/// ISO 4032: hexagon `(3√3/2)·(s/√3)²` times `m`, minus the bore `π(d/2)²·m`.
fn nut(d: f64, s: f64, m: f64) -> Oracle {
    let r = s / 3f64.sqrt();
    Oracle {
        volume: 1.5 * 3f64.sqrt() * r * r * m - PI * (d / 2.0).powi(2) * m,
        z_min: 0.0,
        z_max: m,
    }
}

/// ISO 7089: `π((d2/2)² − (d1/2)²)·h` (M10 inner Ø is the corrected 10.5).
fn washer(d1: f64, d2: f64, h: f64) -> Oracle {
    Oracle {
        volume: PI * ((d2 / 2.0).powi(2) - (d1 / 2.0).powi(2)) * h,
        z_min: 0.0,
        z_max: h,
    }
}

fn thread_param(thread: &str) -> BTreeMap<String, ComponentParamValue> {
    BTreeMap::from([(
        "thread".to_string(),
        ComponentParamValue::Text(thread.to_string()),
    )])
}

/// `(package id, free params, oracle, label)` — the default (M6) and M10 of
/// every seeded family.
fn cases() -> Vec<(
    &'static str,
    BTreeMap<String, ComponentParamValue>,
    Oracle,
    String,
)> {
    vec![
        (
            "onecad.std.iso4762",
            BTreeMap::new(),
            screw(10.0, 6.0, 6.0),
            "iso4762 default".into(),
        ),
        (
            "onecad.std.iso4762",
            thread_param("M10"),
            screw(16.0, 10.0, 10.0),
            "iso4762 M10".into(),
        ),
        (
            "onecad.std.iso4032",
            BTreeMap::new(),
            nut(6.0, 10.0, 5.2),
            "iso4032 default".into(),
        ),
        (
            "onecad.std.iso4032",
            thread_param("M10"),
            nut(10.0, 16.0, 8.4),
            "iso4032 M10".into(),
        ),
        (
            "onecad.std.iso7089",
            BTreeMap::new(),
            washer(6.4, 12.0, 1.6),
            "iso7089 default".into(),
        ),
        (
            "onecad.std.iso7089",
            thread_param("M10"),
            washer(10.5, 20.0, 2.0),
            "iso7089 M10".into(),
        ),
    ]
}

fn assert_rel(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() <= 1e-6 * want.abs(),
        "{what}: got {got}, want {want}"
    );
}

#[test]
fn the_cases_cover_every_seeded_package() {
    let covered: std::collections::BTreeSet<&str> = cases().iter().map(|c| c.0).collect();
    let shipped: std::collections::BTreeSet<&str> = onecad_app::library_seed::SEED_PACKAGES
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(covered, shipped, "a new seed package needs a case here");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_seeded_family_places_meshes_and_survives_reopen() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library = tempfile::tempdir().expect("tempdir");
    onecad_app::library_seed::seed_library(library.path()).expect("seed");
    reindex_library_at(library.path()).expect("reindex");
    let doc_dir = tempfile::tempdir().expect("tempdir");

    for (n, (id, params, oracle, label)) in cases().into_iter().enumerate() {
        let wm = spawn_worker(bin.clone()).await;
        let state = app_state_over(&wm);
        open_blank(&state).await;
        place_component_at(
            library.path(),
            &state,
            id.to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            params,
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("{label}: place failed: {e:?}"));
        let snap = regen(&state).await;
        assert_eq!(snap.bodies.len(), 1, "{label}: one body");
        let body = snap.bodies[0].body;

        let volume = exact_volume(&wm, body).await;
        assert_rel(volume, oracle.volume, &format!("{label}: exact volume"));

        let (lo, hi) = committed_bbox(&state, body).await;
        assert!(
            (lo[2] - oracle.z_min).abs() < 1e-3 && (hi[2] - oracle.z_max).abs() < 1e-3,
            "{label}: meshed z extent [{}, {}] vs [{}, {}]",
            lo[2],
            hi[2],
            oracle.z_min,
            oracle.z_max
        );

        let path = doc_dir.path().join(format!("family{n}.onecad"));
        state
            .runtime
            .lock()
            .await
            .as_mut()
            .unwrap()
            .save(&path, save_meta())
            .expect("save");
        wm.shutdown().await;
        wm.retire();

        let wm2 = spawn_worker(bin.clone()).await;
        let mut reopened = open_over(&wm2, &path);
        let report = reopened
            .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
            .await;
        let Outcome::Published(snap2) = report.outcome else {
            panic!("{label}: reopen must publish");
        };
        assert_eq!(snap2.bodies.len(), 1, "{label}: reopened body count");
        let reopened_volume = exact_volume(&wm2, snap2.bodies[0].body).await;
        assert_rel(
            reopened_volume,
            volume,
            &format!("{label}: reopened volume"),
        );
        wm2.shutdown().await;
        wm2.retire();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Mated placement: the nut and washer sit ON the plate (z ∈ [0, t] in their own
// frame; a plane mate aligns local +Z with the target's outward normal).
// ─────────────────────────────────────────────────────────────────────────────

const PLATE_TOP_Z: f64 = 5.0;
const HOLE_RADIUS: f64 = 4.2; // Ø8.4, the ISO 7089 M8 bore

fn plate_sketch(sid: SketchId) -> onecad_core::sketch::Sketch {
    use onecad_core::ids::EntityId;
    use onecad_core::math::Vec2;
    use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

    let e = |n: u128| EntityId(Uuid::from_u128(0xF1_0000 + n));
    let mut sk = Sketch::on_world_plane(sid, "Plate", WorldPlane::XY);
    let point = |sk: &mut Sketch, id: EntityId, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .expect("point");
    };
    point(&mut sk, e(0), -10.0, -10.0);
    point(&mut sk, e(1), 10.0, -10.0);
    point(&mut sk, e(2), 10.0, 10.0);
    point(&mut sk, e(3), -10.0, 10.0);
    for (l, a, b) in [(0x10, 0, 1), (0x11, 1, 2), (0x12, 2, 3), (0x13, 3, 0)] {
        sk.add_entity(SketchEntity::line(e(l), e(a), e(b), false))
            .expect("line");
    }
    point(&mut sk, e(0x20), 0.0, 0.0);
    sk.add_entity(SketchEntity::circle(e(0x21), e(0x20), HOLE_RADIUS, false).expect("radius"))
        .expect("hole circle");
    sk
}

/// Authors a 20x20x5 plate with a Ø8.4 through hole and regenerates it.
async fn author_plate(state: &AppState) -> BodyId {
    use onecad_core::document::record::{
        BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, PlaneKind,
        SketchOpParams, SketchPlaneRef,
    };
    use onecad_core::document::variables::Scalar;
    use onecad_core::edit::EditCommand;
    use onecad_core::math::Vec3;

    let sid = SketchId(Uuid::from_u128(0xF1_0002));
    let sk = plate_sketch(sid);
    let (_plane, entities, constraints) = onecad_app::worker::wire::sketch_wire(&sk);
    let sketch_op = Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch: sid,
        plane: SketchPlaneRef {
            kind: PlaneKind::Xy,
            origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
            y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
            normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
            extra: Default::default(),
        },
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    }));
    let extrude_op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(onecad_core::document::refs::SketchRegionRef {
            sketch: sid,
            region: onecad_core::ids::RegionId::new(""),
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(PLATE_TOP_Z),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }));
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().expect("open document");
        for (n, op, name) in [
            (0xF1_0001u128, sketch_op, "Sketch"),
            (0xF1_0003, extrude_op, "Extrude"),
        ] {
            rt.apply(EditCommand::AddOperation {
                record: OperationRecord::new(RecordId(Uuid::from_u128(n)), 0, name, op),
                at_cursor: true,
            })
            .expect("AddOperation");
        }
    }
    let snap = regen(state).await;
    assert_eq!(snap.bodies.len(), 1, "one plate");
    snap.bodies[0].body
}

/// Top face (plane, normal +Z, at z = 5) key.
async fn find_top_face(wm: &WorkerManager, body: BodyId) -> String {
    for i in 1..=8 {
        let key = format!("f:{i}");
        let Some(dto) = ElementQuery::classify_element_by_topo_key(wm, body, &key)
            .await
            .expect("ClassifyElement")
        else {
            continue;
        };
        let Some(frame) = dto.frame else { continue };
        let Some(n) = frame.normal else { continue };
        let height: f64 = (0..3).map(|a| frame.origin[a] * n[a]).sum();
        if dto.surface_type == "plane" && n[2] > 0.999 && (height - PLATE_TOP_Z).abs() < 1e-6 {
            return key;
        }
    }
    panic!("no top face at z = {PLATE_TOP_Z}");
}

/// The hole's circular rim on the top face: `(key, centre)`.
async fn find_top_rim(wm: &WorkerManager, body: BodyId) -> (String, [f64; 3]) {
    for i in 1..=24 {
        let key = format!("e:{i}");
        let Some(dto) = ElementQuery::classify_element_by_topo_key(wm, body, &key)
            .await
            .expect("ClassifyElement")
        else {
            continue;
        };
        if dto.curve_type != "circle" {
            continue;
        }
        let Some(frame) = dto.frame else { continue };
        if (frame.radius.unwrap_or(0.0) - HOLE_RADIUS).abs() < 1e-6
            && (frame.origin[2] - PLATE_TOP_Z).abs() < 1e-6
        {
            return (key, frame.origin);
        }
    }
    panic!("no top rim circle of radius {HOLE_RADIUS}");
}

/// Mated placement of `id` (M8) on `target`; returns the placed body's bbox.
async fn place_mated(
    library: &Path,
    state: &AppState,
    id: &str,
    input: onecad_app::dto::PlaceComponentMateInput,
) -> ([f64; 3], [f64; 3]) {
    let existing: Vec<BodyId> = regen(state).await.bodies.iter().map(|b| b.body).collect();
    let before = existing.len();
    place_component_at(
        library,
        state,
        id.to_string(),
        "1.0.0".to_string(),
        // Deliberately wrong in z (the mate must re-seat it) but over solid
        // plate in xy: a coincident seat is projected along the normal and
        // must land ON the face, not in the hole.
        [8.0, 8.0, 11.0],
        None,
        thread_param("M8"),
        Some(input),
    )
    .await
    .unwrap_or_else(|e| panic!("{id}: mated place failed: {e:?}"));
    let snap = regen(state).await;
    assert_eq!(snap.bodies.len(), before + 1, "{id}: one new body");
    let placed = snap
        .bodies
        .iter()
        .find(|b| !existing.contains(&b.body))
        .unwrap()
        .body;
    committed_bbox(state, placed).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_washer_and_a_nut_mate_onto_the_top_of_a_bored_plate() {
    use onecad_app::dto::PlaceComponentMateInput;
    use onecad_app::worker::wire::body_id_wire;
    use onecad_core::document::record::MateKind;

    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library = tempfile::tempdir().expect("tempdir");
    onecad_app::library_seed::seed_library(library.path()).expect("seed");
    reindex_library_at(library.path()).expect("reindex");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let plate = author_plate(&state).await;

    // Washer: Coincident on the top face via `seat`; sits ON it, z ∈ [5, 5 + h].
    let face_key = find_top_face(&wm, plate).await;
    let anchor = [8.0, 8.0, PLATE_TOP_Z];
    let element = promote_mate_target(&state, &body_id_wire(plate), &face_key, anchor)
        .await
        .expect("promote the top face");
    let (lo, hi) = place_mated(
        library.path(),
        &state,
        "onecad.std.iso7089",
        PlaceComponentMateInput {
            self_attachment: "seat".into(),
            target_body_id: body_id_wire(plate),
            target_topo_key: face_key,
            target_element_id: Some(element),
            target_kind: "face".into(),
            kind: MateKind::Coincident,
            flipped: false,
            anchor_world_point: anchor,
        },
    )
    .await;
    let h = 1.6; // ISO 7089 M8 thickness
    assert!(
        (lo[2] - PLATE_TOP_Z).abs() < 1e-3 && (hi[2] - (PLATE_TOP_Z + h)).abs() < 1e-3,
        "washer z [{}, {}] vs [{PLATE_TOP_Z}, {}]",
        lo[2],
        hi[2],
        PLATE_TOP_Z + h
    );

    // Nut: ConcentricAndCoincident on the top rim via `bore_axis`.
    let (edge_key, centre) = find_top_rim(&wm, plate).await;
    let anchor = [centre[0] + HOLE_RADIUS, centre[1], PLATE_TOP_Z];
    let element = promote_mate_target(&state, &body_id_wire(plate), &edge_key, anchor)
        .await
        .expect("promote the rim");
    let (lo, hi) = place_mated(
        library.path(),
        &state,
        "onecad.std.iso4032",
        PlaceComponentMateInput {
            self_attachment: "bore_axis".into(),
            target_body_id: body_id_wire(plate),
            target_topo_key: edge_key,
            target_element_id: Some(element),
            target_kind: "edge".into(),
            kind: MateKind::ConcentricAndCoincident,
            flipped: NUT_FLIPPED,
            anchor_world_point: anchor,
        },
    )
    .await;
    let m = 6.8; // ISO 4032 M8 thickness
    assert!(
        (lo[2] - PLATE_TOP_Z).abs() < 1e-3 && (hi[2] - (PLATE_TOP_Z + m)).abs() < 1e-3,
        "nut z [{}, {}] vs [{PLATE_TOP_Z}, {}]",
        lo[2],
        hi[2],
        PLATE_TOP_Z + m
    );
    // Centred on the hole axis (the hexagon is symmetric about its axis).
    for a in 0..2 {
        let mid = (lo[a] + hi[a]) / 2.0;
        assert!(
            (mid - centre[a]).abs() < 1e-3,
            "nut axis {a}: {mid} vs {}",
            centre[a]
        );
    }
    wm.shutdown().await;
}

/// Measured: with the rim's axis pointing out of the plate, `flipped = false`
/// seats the nut's +Z along it, i.e. above the top face.
const NUT_FLIPPED: bool = false;
