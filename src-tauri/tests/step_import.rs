//! STEP-IMPORT WP-A W3 gate — the app layer's `ImportStep` lane against the REAL
//! worker (SCHEMA §7.3 op + §7.8 `InspectStep`).
//!
//! The fixture is **self-produced**: the test builds two disjoint extruded boxes of
//! known exact volume, exports them with the worker's own `ExportStep`, and imports
//! that file back. So every numeric expectation is box arithmetic, not a checked-in
//! binary, and a STEP-writer regression shows up here too.
//!
//! What each test pins:
//!
//! * `import_mints_ordinal_children_with_exact_volumes` — (a) the import mints
//!   `body_<opId>:0` / `:1` under the §7.3 N-body rule, volumes exact vs the
//!   originals; (b) a from-0 replay in the SAME runtime re-mints the SAME ids and
//!   the SAME head signature (the record replays from the stored brep, so replay is
//!   a pure function of the document).
//! * `saved_import_reopens_on_a_fresh_worker` — (c) save → reopen with a FRESH
//!   runtime + FRESH worker: same ids, same volumes, same `document.json` bytes on
//!   a second save.
//! * `suppressing_an_import_keeps_its_blobs` — (d) a suppressed import drops its
//!   bodies from the projection but STILL pins both blobs in the container (the
//!   refcount rule: suppression is a reversible toggle, not data loss).
//! * `insert_step_appends_to_an_existing_document` — (e) the in-editor lane adds to
//!   the open document instead of replacing it.
//! * `import_path_is_wire_only_and_never_hashed` — (f) two lowerings under
//!   different temp roots ⇒ different wire JSON, IDENTICAL planner hash.
//!
//! Gated on `ONECAD_WORKER_PATH` (else dev-tree fallback); `ONECAD_REQUIRE_WORKER=1`
//! hard-fails a missing binary (CI). A missing worker is a quiet local-dev skip.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, ImportSourceCodec, ImportStepParams, KnownOperation,
    Operation, OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef, IMPORT_HEAL_POLICY_V1,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ConstraintId, DocumentId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::io::container::{ContainerReader, SaveMeta};
use onecad_core::math::Vec2;
use onecad_core::regen::{
    history_prefix_hash, CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::imports::{prepare_import, ImportWorkspace, PreparedImport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::{body_id_wire, preview_wire_op, sketch_wire};
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors open_render.rs / wire_contract.rs)
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

fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

fn open_over(wm: &WorkerManager, path: &std::path::Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen the saved container")
}

async fn regen_all(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-7.9.3".into()),
        created: "2026-08-02T00:00:00Z".into(),
        modified: "2026-08-02T00:00:00Z".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Mesh volume (divergence theorem — EXACT for planar-faced solids)
// ─────────────────────────────────────────────────────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;

fn vertex(blob: &[u8], pbase: usize, i: usize) -> [f64; 3] {
    let o = pbase + i * 12;
    [
        f32_le(blob, o) as f64,
        f32_le(blob, o + 4) as f64,
        f32_le(blob, o + 8) as f64,
    ]
}

fn mesh_volume(view: &MeshHeaderView, blob: &[u8]) -> f64 {
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let (pbase, ibase) = (pos.offset as usize, idx.offset as usize);
    let mut vol6 = 0.0f64;
    for t in 0..view.triangle_count as usize {
        let o = ibase + t * 12;
        let a = vertex(blob, pbase, u32_le(blob, o) as usize);
        let b = vertex(blob, pbase, u32_le(blob, o + 4) as usize);
        let c = vertex(blob, pbase, u32_le(blob, o + 8) as usize);
        vol6 += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    (vol6 / 6.0).abs()
}

async fn body_volume(rt: &mut DocumentRuntime, body: BodyId) -> f64 {
    let mesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh");
    let view = validate_mesh_blob(&mesh).expect("valid MESH1 blob");
    mesh_volume(&view, &mesh)
}

/// Head volumes, sorted ascending — the import's ordinal order is the worker's, not
/// the source file's, so a body-to-volume identity assertion would be over-specified.
async fn head_volumes(rt: &mut DocumentRuntime) -> Vec<f64> {
    let mut out = Vec::new();
    for body in rt.head_body_ids() {
        out.push(body_volume(rt, body).await);
    }
    out.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixture: two disjoint extruded boxes (40×20×`dist_a` and 10×10×5)
//
// **Every test uses its OWN `dist_a`.** The blobs are content-addressed and the
// path registry is process-global, so two tests exporting IDENTICAL geometry would
// share one digest — and one finishing (dropping its workspace) could delete the
// file the other's in-flight regen just resolved. Distinct geometry per test keeps
// the parallel run honest instead of papering over it with `--test-threads=1`.
// ─────────────────────────────────────────────────────────────────────────────

const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;
const SKETCH_B: u128 = 0xB00;
const EXTRUDE_B: u128 = 0xB01;
/// Box B is the same in every fixture (only A varies): 10×10×5.
const VOL_B: f64 = 10.0 * 10.0 * 5.0;

/// Box A's volume for an extrude distance of `dist_a`.
fn vol_a(dist_a: f64) -> f64 {
    40.0 * 20.0 * dist_a
}

fn xy_plane_ref() -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: onecad_core::math::Vec3::new_unchecked(0.0, 0.0, 0.0),
        x_axis: onecad_core::math::Vec3::new_unchecked(0.0, 1.0, 0.0),
        y_axis: onecad_core::math::Vec3::new_unchecked(-1.0, 0.0, 0.0),
        normal: onecad_core::math::Vec3::new_unchecked(0.0, 0.0, 1.0),
        extra: Default::default(),
    }
}

/// A fully-constrained rectangle (identical to `open_render::rect_sketch`).
fn rect_sketch(sid: SketchId, base: u128, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (p0s, p0e) = (e(0), e(1));
    let (p1s, p1e) = (e(2), e(3));
    let (p2s, p2e) = (e(4), e(5));
    let (p3s, p3e) = (e(6), e(7));
    let (l0, l1, l2, l3) = (e(0x10), e(0x11), e(0x12), e(0x13));

    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    let pt = |sk: &mut Sketch, id: EntityId, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .unwrap();
    };
    pt(&mut sk, p0s, x0, y0);
    pt(&mut sk, p0e, x0 + w, y0);
    pt(&mut sk, p1s, x0 + w, y0);
    pt(&mut sk, p1e, x0 + w, y0 + h);
    pt(&mut sk, p2s, x0 + w, y0 + h);
    pt(&mut sk, p2e, x0, y0 + h);
    pt(&mut sk, p3s, x0, y0 + h);
    pt(&mut sk, p3e, x0, y0);
    sk.add_entity(SketchEntity::line(l0, p0s, p0e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l1, p1s, p1e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l2, p2s, p2e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l3, p3s, p3e, false))
        .unwrap();

    let coincident = |sk: &mut Sketch, id, a, b| {
        sk.add_constraint(Constraint::Coincident {
            id,
            point1: a,
            point2: b,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    };
    coincident(&mut sk, c(1), p0e, p1s);
    coincident(&mut sk, c(2), p1e, p2s);
    coincident(&mut sk, c(3), p2e, p3s);
    coincident(&mut sk, c(4), p3e, p0s);
    sk.add_constraint(Constraint::Horizontal { id: c(5), line: l0 })
        .unwrap();
    sk.add_constraint(Constraint::Horizontal { id: c(6), line: l2 })
        .unwrap();
    sk.add_constraint(Constraint::Vertical { id: c(7), line: l1 })
        .unwrap();
    sk.add_constraint(Constraint::Vertical { id: c(8), line: l3 })
        .unwrap();
    sk.add_constraint(Constraint::Fixed {
        id: c(9),
        point: p0s,
        at: Vec2::new_unchecked(x0, y0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: p0s,
        point2: p0e,
        value: Scalar::new(w),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: p1s,
        point2: p1e,
        value: Scalar::new(h),
    })
    .unwrap();
    sk
}

fn sketch_record(rec: u128, sk: &Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    let params = SketchOpParams {
        sketch: sk.id,
        plane: xy_plane_ref(),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""),
            extra: Default::default(),
        }),
        distance: Scalar::new(dist),
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
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

fn add(rt: &mut DocumentRuntime, record: OperationRecord) {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("AddOperation");
}

/// Builds the two-box document and returns its `(body A, body B)` ids.
fn build_two_boxes(rt: &mut DocumentRuntime, dist_a: f64) -> (BodyId, BodyId) {
    let sa = SketchId(Uuid::from_u128(0xA));
    let sb = SketchId(Uuid::from_u128(0xB));
    add(
        rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add(rt, extrude_record(EXTRUDE_A, sa, dist_a));
    // Disjoint from A: world Y ∈ [100,110] vs A's [0,40].
    add(
        rt,
        sketch_record(SKETCH_B, &rect_sketch(sb, 0x2000, 100.0, 0.0, 10.0, 10.0)),
    );
    add(rt, extrude_record(EXTRUDE_B, sb, 5.0));
    (
        BodyId(Uuid::from_u128(EXTRUDE_A)),
        BodyId(Uuid::from_u128(EXTRUDE_B)),
    )
}

/// Builds the two-box document on its own worker, exports it to `path`, and asserts
/// the source volumes the import must reproduce.
async fn export_fixture(bin: &std::path::Path, path: &std::path::Path, dist_a: f64) {
    let wm = spawn_worker(bin.to_path_buf()).await;
    let mut rt = runtime_over(&wm);
    let (a, b) = build_two_boxes(&mut rt, dist_a);
    let snap = regen_all(&mut rt).await;
    let snap = published(&snap, "fixture build");
    assert_eq!(snap.bodies.len(), 2, "fixture is two disjoint bodies");
    let (va, vb) = (body_volume(&mut rt, a).await, body_volume(&mut rt, b).await);
    let expect_a = vol_a(dist_a);
    assert!(
        (va - expect_a).abs() < 1.0,
        "source box A = {expect_a}, got {va}"
    );
    assert!((vb - VOL_B).abs() < 1.0, "source box B = {VOL_B}, got {vb}");

    let written = wm
        .export_step(&path.to_string_lossy(), &[a, b], "AP214IS")
        .await
        .expect("ExportStep the fixture");
    assert!(written > 0, "ExportStep wrote bytes");
    wm.shutdown().await;
}

/// The record id of the (single) `ImportStep` feature in the tree.
fn import_record_id(rt: &DocumentRuntime) -> Uuid {
    let features = rt.projection().features;
    let import = features
        .iter()
        .find(|f| f.op_type == "ImportStep")
        .expect("the tree carries the ImportStep feature");
    Uuid::parse_str(&import.id).expect("feature id is the record uuid")
}

/// Drives the runtime-side half of the `import_step` command: probe + author.
async fn do_import(
    wm: &WorkerManager,
    rt: &mut DocumentRuntime,
    step: &std::path::Path,
    at_cursor: bool,
) -> PreparedImport {
    let prepared = prepare_import(wm, step).await.expect("prepare_import");
    rt.add_import_record(&prepared, at_cursor)
        .expect("author the ImportStep record");
    prepared
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) + (b) ordinal children, exact volumes, replay-stable
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn import_mints_ordinal_children_with_exact_volumes() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let step = tmp.path().join("two_boxes.step");
    let dist_a = 25.0;
    export_fixture(&bin, &step, dist_a).await;

    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let prepared = do_import(&wm, &mut rt, &step, false).await;

    // The probe is what the record is authored from — converted-primary replay
    // policy, with the WORKER naming the codec (SCHEMA §7.8 `geometryCodec`).
    assert_eq!(prepared.solid_count, 2, "the STEP carries two solids");
    assert!(
        prepared.params.source_codec.is_converted(),
        "the replayed blob is the healed kernel dump, not the STEP (got {:?})",
        prepared.params.source_codec
    );
    assert_eq!(
        prepared.params.source_codec,
        ImportSourceCodec::Xbf,
        "the conversion lane emits BinXCAF, so imported names/colors survive replay"
    );
    assert!(
        prepared.params.brep_format.is_some(),
        "a converted-codec record MUST pin its binary format version"
    );
    assert_eq!(
        prepared.blobs.len(),
        2,
        "converted replay blob + co-stored STEP provenance"
    );
    assert_eq!(
        prepared.params.provenance_sha256.as_deref(),
        Some(prepared.blobs[1].0.as_str()),
        "the provenance sha addresses the user's ORIGINAL bytes"
    );

    // ── (a) two ordinal children, exact volumes ──────────────────────────────
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "import regen");
    assert_eq!(snap.bodies.len(), 2, "two solids ⇒ two bodies");

    let record = import_record_id(&rt);
    let mut ids: Vec<String> = rt
        .head_body_ids()
        .iter()
        .copied()
        .map(body_id_wire)
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        vec![format!("body_{record}:0"), format!("body_{record}:1")],
        "N>1 mints ordinal children under the §7.3/§2 rule (never a plain body_<opId>)"
    );

    let volumes = head_volumes(&mut rt).await;
    assert!(
        (volumes[0] - VOL_B).abs() < 1.0 && (volumes[1] - vol_a(dist_a)).abs() < 1.0,
        "imported volumes must equal the source boxes ({VOL_B}, {}), got {volumes:?}",
        vol_a(dist_a)
    );

    // ── (b) from-0 replay in the SAME runtime is byte-for-byte the same head ──
    let sig = snap
        .signatures
        .as_ref()
        .map(|s| s.geometry.as_str().to_string());
    let replay = regen_all(&mut rt).await;
    let replay_snap = published(&replay, "from-0 replay");
    let mut replay_ids: Vec<String> = rt
        .head_body_ids()
        .iter()
        .copied()
        .map(body_id_wire)
        .collect();
    replay_ids.sort();
    assert_eq!(replay_ids, ids, "replay re-mints the SAME ordinal children");
    assert_eq!(
        replay_snap
            .signatures
            .as_ref()
            .map(|s| s.geometry.as_str().to_string()),
        sig,
        "replaying the stored brep is a pure function ⇒ identical head signature"
    );
    assert_eq!(
        head_volumes(&mut rt).await,
        volumes,
        "replay volumes are identical"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) save → reopen on a FRESH runtime + FRESH worker
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_import_reopens_on_a_fresh_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let step = tmp.path().join("two_boxes.step");
    let dist_a = 26.0;
    export_fixture(&bin, &step, dist_a).await;
    let doc = tmp.path().join("imported.onecad");

    // ── Session 1: import + save ─────────────────────────────────────────────
    let (ids, volumes, shas) = {
        let wm = spawn_worker(bin.clone()).await;
        let mut rt = runtime_over(&wm);
        let prepared = do_import(&wm, &mut rt, &step, false).await;
        published(&regen_all(&mut rt).await, "session-1 import regen");
        let mut ids: Vec<String> = rt
            .head_body_ids()
            .iter()
            .copied()
            .map(body_id_wire)
            .collect();
        ids.sort();
        // Pin the ABSOLUTE outcome here, not just parity with session 2: a broken
        // import lane publishes zero bodies in BOTH sessions, and "[] == []" would
        // make this whole test vacuously green.
        assert_eq!(ids.len(), 2, "session 1 must actually import two bodies");
        let volumes = head_volumes(&mut rt).await;
        assert!(
            (volumes[0] - VOL_B).abs() < 1.0 && (volumes[1] - vol_a(dist_a)).abs() < 1.0,
            "session-1 volumes must be the source boxes, got {volumes:?}"
        );
        rt.save(&doc, save_meta()).expect("save");
        let shas: Vec<String> = prepared.blobs.iter().map(|(s, _)| s.clone()).collect();
        wm.shutdown().await;
        (ids, volumes, shas)
    };

    // Both blobs landed in the container and verify against their own digests.
    {
        let loaded = ContainerReader::open(&doc).expect("open the saved container");
        assert_eq!(
            loaded.import_blobs().len(),
            2,
            "the container pins BOTH the replay brep and the STEP provenance"
        );
        for sha in &shas {
            let bytes = loaded
                .read_import_blob(sha)
                .unwrap_or_else(|e| panic!("import blob {sha} must read back: {e}"));
            assert!(!bytes.is_empty(), "import blob {sha} is non-empty");
        }
    }

    // ── Session 2: FRESH runtime on a FRESH worker ───────────────────────────
    let wm = spawn_worker(bin).await;
    let mut rt = open_over(&wm, &doc);
    published(&regen_all(&mut rt).await, "reopen regen");
    let mut reopened: Vec<String> = rt
        .head_body_ids()
        .iter()
        .copied()
        .map(body_id_wire)
        .collect();
    reopened.sort();
    assert_eq!(
        reopened, ids,
        "reopen re-mints the SAME ordinal children (the record id is persisted)"
    );
    assert_eq!(
        head_volumes(&mut rt).await,
        volumes,
        "reopened volumes are identical — the brep blob round-tripped through the container"
    );

    // `document.json` byte-stability across a second save (no churn from imports).
    let doc2 = tmp.path().join("imported-again.onecad");
    rt.save(&doc2, save_meta()).expect("second save");
    assert_eq!(
        document_json(&doc),
        document_json(&doc2),
        "document.json must be byte-stable across save → reopen → save"
    );

    wm.shutdown().await;
}

/// The raw `document.json` bytes inside a `.onecad` container.
fn document_json(path: &std::path::Path) -> Vec<u8> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open container");
    let mut zip = zip::ZipArchive::new(file).expect("read container zip");
    let mut entry = zip.by_name("document.json").expect("document.json entry");
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).expect("read document.json");
    bytes
}

// ─────────────────────────────────────────────────────────────────────────────
// (d) suppression drops the bodies but never the blobs
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suppressing_an_import_keeps_its_blobs() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let step = tmp.path().join("two_boxes.step");
    let dist_a = 27.0;
    export_fixture(&bin, &step, dist_a).await;

    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let prepared = do_import(&wm, &mut rt, &step, false).await;
    published(&regen_all(&mut rt).await, "import regen");
    assert_eq!(rt.head_body_ids().len(), 2, "imported bodies are live");

    let record = RecordId(import_record_id(&rt));
    rt.apply(EditCommand::SetOperationSuppression {
        record,
        suppressed: true,
        cascade: false,
    })
    .expect("suppress the import");
    regen_all(&mut rt).await;
    assert!(
        rt.projection().bodies.is_empty(),
        "a suppressed import contributes no bodies to the projection"
    );

    let doc = tmp.path().join("suppressed.onecad");
    rt.save(&doc, save_meta())
        .expect("save with the import suppressed");
    let loaded = ContainerReader::open(&doc).expect("open the saved container");
    for (sha, _) in &prepared.blobs {
        loaded
            .read_import_blob(sha)
            .unwrap_or_else(|e| panic!("a SUPPRESSED record still pins blob {sha}: {e}"));
    }
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (e) the in-editor lane appends instead of replacing
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insert_step_appends_to_an_existing_document() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let step = tmp.path().join("two_boxes.step");
    let dist_a = 28.0;
    export_fixture(&bin, &step, dist_a).await;

    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    // An existing modelled body (box A only — sketch + extrude).
    let sa = SketchId(Uuid::from_u128(0xA));
    add(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add(&mut rt, extrude_record(EXTRUDE_A, sa, 25.0));
    published(&regen_all(&mut rt).await, "pre-import regen");
    let prior = BodyId(Uuid::from_u128(EXTRUDE_A));
    let prior_volume = body_volume(&mut rt, prior).await;
    assert!((prior_volume - vol_a(25.0)).abs() < 1.0);

    // The in-editor lane: at the cursor, into the OPEN document.
    do_import(&wm, &mut rt, &step, true).await;
    published(&regen_all(&mut rt).await, "insert regen");

    let bodies = rt.head_body_ids();
    assert_eq!(
        bodies.len(),
        3,
        "1 existing + 2 imported (the import APPENDS, it never replaces)"
    );
    assert!(
        bodies.contains(&prior),
        "the pre-existing body survives the import"
    );
    assert!(
        (body_volume(&mut rt, prior).await - prior_volume).abs() < 1e-9,
        "the pre-existing body is untouched by the import"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (f) the wire-only `path` never reaches the planner hash — no worker needed
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn import_path_is_wire_only_and_never_hashed() {
    let bytes = b"pretend BinTools bytes";
    let sha = onecad_lib::imports::sha256_hex(bytes);
    let params = ImportStepParams {
        source_sha256: sha.clone(),
        source_codec: ImportSourceCodec::Brep,
        source_name: "bracket.step".into(),
        heal_policy: IMPORT_HEAL_POLICY_V1.into(),
        unit_scale: Scalar::new(1.0),
        brep_format: Some(4),
        provenance_sha256: None,
        extra: Default::default(),
    };
    let record = OperationRecord::new(
        RecordId(Uuid::from_u128(0x1_0000)),
        0,
        "Import bracket",
        Operation::Known(KnownOperation::ImportStep(params)),
    );
    let op_id = record.record_id.to_string();
    let op = record.op.clone();

    let lower_under = |ws: &mut ImportWorkspace| -> String {
        ws.materialize(&sha, ImportSourceCodec::Brep, bytes)
            .expect("materialize");
        preview_wire_op(&op, &op_id)["params"]["path"]
            .as_str()
            .expect("lowering injects params.path")
            .to_string()
    };

    let (path_a, path_b) = {
        // Sequential, not concurrent: resolution returns the FIRST live path, so the
        // second root can only be observed once the first is gone.
        let mut a = ImportWorkspace::new(DocumentId::new());
        let pa = lower_under(&mut a);
        drop(a);
        let mut b = ImportWorkspace::new(DocumentId::new());
        let pb = lower_under(&mut b);
        drop(b);
        (pa, pb)
    };
    assert_ne!(
        path_a, path_b,
        "the two temp roots must be distinct for this test to prove anything"
    );
    assert!(
        path_a.ends_with(&format!("{sha}.brep")) && path_b.ends_with(&format!("{sha}.brep")),
        "the lowered path is the content-addressed blob file, got {path_a:?} / {path_b:?}"
    );

    let records = [record];
    assert_eq!(
        history_prefix_hash(&records).as_str(),
        history_prefix_hash(&records).as_str(),
        "the planner hash is over the CORE params — no absolute path may enter it"
    );
    // The strong form: the wire form DIFFERS across roots while the hash cannot,
    // because the hash never sees the wire form (`wire_op_line` reads the typed op).
    let hash = history_prefix_hash(&records).as_str().to_string();
    let mut ws = ImportWorkspace::new(DocumentId::new());
    let path_c = lower_under(&mut ws);
    assert_ne!(path_c, path_a, "a third root lowers a third path");
    assert_eq!(
        history_prefix_hash(&records).as_str(),
        hash,
        "same document ⇒ same hash regardless of where the blob was materialized"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// A missing blob degrades to ONE failed step, never a Rust panic
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn a_missing_blob_lowers_an_empty_path_instead_of_panicking() {
    let params = ImportStepParams {
        // A digest nothing ever materialized.
        source_sha256: onecad_lib::imports::sha256_hex(b"never materialized"),
        source_codec: ImportSourceCodec::Brep,
        source_name: "gone.step".into(),
        heal_policy: IMPORT_HEAL_POLICY_V1.into(),
        unit_scale: Scalar::new(1.0),
        brep_format: Some(4),
        provenance_sha256: None,
        extra: Default::default(),
    };
    let op = Operation::Known(KnownOperation::ImportStep(params));
    let wire = preview_wire_op(&op, "op_missing");
    assert_eq!(
        wire["params"]["path"].as_str(),
        Some(""),
        "an un-materialized blob lowers an EMPTY path — the worker then fails that ONE \
         step with OP_FAILED (SCHEMA §7.3), which is the designed blast radius"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// STEP product names become body names — and a user rename still wins
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn product_names_label_the_imported_bodies() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let step = tmp.path().join("two_boxes.step");
    let dist_a = 29.0;
    export_fixture(&bin, &step, dist_a).await;

    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let prepared = do_import(&wm, &mut rt, &step, false).await;
    // Self-exported fixture ⇒ OCCT's own translator product names. The VALUE is the
    // writer's business; what this pins is that whatever the probe recovered reaches
    // the body rows by ordinal.
    assert_eq!(
        prepared.product_names.len(),
        2,
        "the probe recovered a name per solid, got {:?}",
        prepared.product_names
    );
    published(&regen_all(&mut rt).await, "import regen");

    let names: Vec<String> = rt
        .projection()
        .bodies
        .values()
        .map(|b| b.name.clone())
        .collect();
    for expected in &prepared.product_names {
        assert!(
            names.contains(expected),
            "product name {expected:?} must label an imported body (got {names:?})"
        );
    }

    // A user rename overlays the imported label and survives the next regen —
    // `apply_import_body_names` writes the MIRROR, `merge_body_metadata` overlays
    // `document.bodies` on top, so the document row wins forever after.
    let body = rt.head_body_ids()[0];
    rt.apply(EditCommand::RenameBody {
        body,
        name: "Bracket".into(),
    })
    .expect("RenameBody on an imported body");
    published(&regen_all(&mut rt).await, "post-rename regen");
    let renamed: Vec<String> = rt
        .projection()
        .bodies
        .values()
        .map(|b| b.name.clone())
        .collect();
    assert!(
        renamed.contains(&"Bracket".to_string()),
        "a user rename must outrank the STEP product name (got {renamed:?})"
    );
    wm.shutdown().await;
}
