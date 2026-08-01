//! Regression test for `face_sketch_plane`'s topoKey read-ladder rung — the
//! same latent `element_info` already fixed in MODELING-REACH W2, flagged
//! unfixed here. Real C++ OCCT worker, driven through the app's ACTUAL
//! `#[tauri::command]` fn via `tauri::test::mock_app` (the only public way to
//! construct a `tauri::State` outside a real webview — its constructor is
//! otherwise crate-private). No IPC / webview dispatch is simulated; this is a
//! direct async fn call against a real `tauri::State<AppState>`.
//!
//! THE LATENT (see `api::face_sketch_plane` before this fix): the command
//! resolved its picked face by ElementId ALONE. The worker's element-map
//! partition mints entries ONLY when an OP resolves the id as an input
//! (`PlanExecutor::resolve_input_refs`) — `AcquireElementIds`/`promote_selection`
//! mints the id in RUST and never tells the worker. So a JUST-PROMOTED,
//! never-consumed face id is genuinely absent from the partition, and
//! `face_sketch_plane` failed loudly for exactly the "sketch on the face you
//! just selected" flow `SketchController.tryEnterOnSelectedFace` drives — the
//! feature's single most common path. Fixed by trying the topoKey rung FIRST,
//! mirroring `element_info`'s ladder (see `api::element_info`,
//! `wire_contract::measure_element_info_reports_exact_kernel_quantities`).
//!
//! Gated on `ONECAD_WORKER_PATH` / `ONECAD_REQUIRE_WORKER` like every other
//! worker-backed test in this suite (see `wire_contract.rs`'s `real_worker`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use tauri::Manager;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{
    BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::export::GeometryExporter;
use onecad_lib::state::AppState;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::{body_id_wire, sketch_wire};
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, PreviewEngine, SolverEngine, WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors wire_contract.rs / m2_gate.rs — each worker-backed test file
// keeps its own small harness rather than sharing a `tests/common` module).
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

/// An `AppState` wired to the SAME `WorkerManager` for every facet (engine,
/// meshes, exporter, element-query, preview) — exactly what
/// `state::real_worker_factory` does in production, minus the restart-hook /
/// status-forwarder wiring this test never exercises.
fn app_state_over(wm: &WorkerManager) -> AppState {
    let wm = wm.clone();
    AppState::new(Arc::new(move || {
        let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
        let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
        let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
        let exporter: Arc<dyn GeometryExporter> = Arc::new(wm.clone());
        let elements: Arc<dyn ElementQuery> = Arc::new(wm.clone());
        let preview: Arc<dyn PreviewEngine> = Arc::new(wm.clone());
        (engine, meshes, solver, exporter, elements, preview)
    }))
}

fn add_op(rt: &mut DocumentRuntime, record: OperationRecord) {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("AddOperation");
}

async fn regen_all(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

fn published<'a>(
    report: &'a RegenReport,
    what: &str,
) -> &'a Arc<onecad_core::regen::ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

const SKETCH_A: u128 = 0xF00;
const EXTRUDE_A: u128 = 0xF01;

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

/// Same non-standard XY basis as `wire_contract.rs` (`xAxis=(0,1,0)`,
/// `yAxis=(-1,0,0)`) — deliberately non-identity so a frame silently re-derived
/// anywhere would show up as a DIFFERENT plane, not a coincidental match.
fn xy_plane_ref() -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
        x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
        y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
        normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
        extra: Default::default(),
    }
}

/// A fully-constrained rectangle — the same marshaller-shaped fixture as
/// `wire_contract::rect_sketch` (8 points, 4 lines, coincident corners, H/V, a
/// Fixed anchor, H/V dimensions).
fn rect_sketch(sid: SketchId, base: u128, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (p0s, p0e) = (e(0), e(1));
    let (p1s, p1e) = (e(2), e(3));
    let (p2s, p2e) = (e(4), e(5));
    let (p3s, p3e) = (e(6), e(7));
    let (l0, l1, l2, l3) = (e(0x10), e(0x11), e(0x12), e(0x13));

    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    let pt = |sk: &mut Sketch, id, x: f64, y: f64| {
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
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

/// A simple `NewBody` blind extrude — this fixture never needs the boolean /
/// target-body variants `wire_contract::extrude_record` carries.
fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // empty ⇒ V1 first-region fallback
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

// ── MESH1 id-table helpers (exact for a planar-faced box) ───────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;

fn vertex(blob: &[u8], pbase: usize, i: usize) -> [f64; 3] {
    let o = pbase + i * 12;
    [
        f32_le(blob, o) as f64,
        f32_le(blob, o + 4) as f64,
        f32_le(blob, o + 8) as f64,
    ]
}

fn id_table(
    view: &MeshHeaderView,
    blob: &[u8],
    offs_ty: u32,
    chars_ty: u32,
    count: usize,
) -> Vec<String> {
    let offs = view.section(offs_ty).expect("id-offs");
    let chars = view.section(chars_ty).expect("id-chars");
    let (obase, cbase) = (offs.offset as usize, chars.offset as usize);
    (0..count)
        .map(|i| {
            let lo = u32_le(blob, obase + i * 4) as usize;
            let hi = u32_le(blob, obase + (i + 1) * 4) as usize;
            String::from_utf8_lossy(&blob[cbase + lo..cbase + hi]).into_owned()
        })
        .collect()
}

/// The face with the greatest average world-Z (the extrude's far cap): its
/// `TopoKey`. Mirrors `wire_contract::top_face_pick` (centroid not needed here).
fn top_face_pick(view: &MeshHeaderView, blob: &[u8]) -> String {
    let fr = view.section(SEC_FACE_RANGES).expect("FACE_RANGES");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let (frbase, ibase, pbase) = (fr.offset as usize, idx.offset as usize, pos.offset as usize);
    let keys = id_table(
        view,
        blob,
        SEC_FACE_ID_OFFS,
        SEC_FACE_ID_CHARS,
        view.face_count as usize,
    );
    let mut best: Option<(usize, f64)> = None;
    for f in 0..view.face_count as usize {
        let first = u32_le(blob, frbase + f * 8) as usize;
        let count = u32_le(blob, frbase + f * 8 + 4) as usize;
        let (mut sz, mut n) = (0.0, 0.0f64);
        for t in first..first + count {
            let io = ibase + t * 12;
            for k in 0..3 {
                let v = vertex(blob, pbase, u32_le(blob, io + k * 4) as usize);
                sz += v[2];
                n += 1.0;
            }
        }
        if n == 0.0 {
            continue;
        }
        let z = sz / n;
        if best.is_none_or(|(_, bz)| z > bz) {
            best = Some((f, z));
        }
    }
    let (idx_best, _) = best.expect("at least one face");
    keys[idx_best].clone()
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, onecad_core::regen::Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

// ─────────────────────────────────────────────────────────────────────────────
// The gate
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn face_sketch_plane_resolves_a_promoted_unconsumed_face_via_the_topo_key_rung() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let sa = SketchId(Uuid::from_u128(0xF));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x2000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_A, sa, 25.0));
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "sketch-on-face box");
    let snap_id = SnapshotId(report.snapshot_id);
    let body = body_of(EXTRUDE_A);

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    assert_eq!(view.face_count, 6, "the extruded rect is a 6-faced box");
    let top_key = top_face_pick(&view, &mesh);

    // Promote the pick to a Rust-minted ElementId — and deliberately consume it
    // in NOTHING: no extrude/fillet/etc. references it, so the worker's
    // element-map partition never gains an entry for it (the exact latent).
    let promoted = rt
        .promote_selection(snap_id, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("promote top face");
    let face_el = promoted[0].element_id.clone();

    // Pin WHY the ladder is load-bearing — same fact `element_info`'s own test
    // pins (`wire_contract::measure_element_info_reports_exact_kernel_quantities`):
    // a promoted-but-unconsumed id is genuinely absent from the partition.
    assert!(
        ElementQuery::query_element(&wm, snap_id, body, &face_el)
            .await
            .expect("QueryElement(by elementId)")
            .is_none(),
        "a promoted-but-unconsumed ElementId is not in the worker partition — \
         this is what makes face_sketch_plane's topoKey rung load-bearing"
    );

    // Ground truth for the assertion below: the picked face's OWN descriptor.
    let face = ElementQuery::query_element_by_topo_key(&wm, snap_id, body, &top_key)
        .await
        .expect("QueryElement(top face by topoKey)")
        .expect("the picked face resolves against the body shape");
    assert_eq!(face.kind, "face");
    assert_eq!(face.surface_type, 0, "a box cap is GeomAbs_Plane (0)");

    // ── the actual command, through a REAL tauri::State ─────────────────────
    // `mock_app` is the only public way outside a real webview to construct one
    // (its constructor is otherwise crate-private) — no IPC / webview dispatch
    // is simulated; this is a direct async fn call.
    let app = tauri::test::mock_app();
    *app_state.runtime.lock().await = Some(rt);
    app.manage(app_state);
    let state: tauri::State<'_, AppState> = app.state();

    let plane = onecad_lib::api::face_sketch_plane(
        state,
        snap_id.0,
        body_id_wire(body),
        face_el,
        Some(top_key),
    )
    .await
    .expect(
        "face_sketch_plane must resolve a promoted-but-unconsumed face via the \
         topoKey rung, exactly like element_info's ladder",
    );

    for (axis, got, want) in [
        ("x", plane.normal[0], face.normal[0]),
        ("y", plane.normal[1], face.normal[1]),
        ("z", plane.normal[2], face.normal[2]),
    ] {
        assert!(
            (got - want).abs() < 1e-9,
            "plane.normal.{axis} must match the picked face's own descriptor normal, \
             got {got}, want {want} (plane {:?}, face.normal {:?})",
            plane.normal,
            face.normal
        );
    }
    // The box's far cap is the +Z-facing extrude direction.
    assert!(
        (plane.normal[2] - 1.0).abs() < 1e-6,
        "the box's far cap should face +Z, got {:?}",
        plane.normal
    );

    wm.shutdown().await;
}
