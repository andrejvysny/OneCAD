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
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
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

// ═════════════════════════════════════════════════════════════════════════════
// SKETCH-ON-FACE W1b — `add_sketch_on_face`: the projected host-face boundary
//
// `face_sketch_plane` above answers "where would a sketch on this face sit?".
// This block covers the AUTHORING command: it runs the SCHEMA §7.6
// `ProjectFaceBoundary` handshake (frameOnly → Rust-owned basis → real
// projection), mints the boundary as `referenceLocked` geometry pinned by
// `Fixed`, and commits ONE `EditCommand::AddSketch`.
//
// Every test here drives the REAL C++ worker through the REAL `#[tauri::command]`
// fn, and is `ONECAD_REQUIRE_WORKER`-gated exactly like the gate above.
// ═════════════════════════════════════════════════════════════════════════════

use std::collections::BTreeSet;
use std::str::FromStr;

use onecad_core::edit::SketchEditOp;
use onecad_core::io::container::{ContainerReader, SaveMeta};
use onecad_core::sketch::{SketchAttachment, SketchError};

use onecad_lib::dto::{FinishSketchDto, SketchSessionDto, SketchSolveStatus};

// ── mesh geometry helpers ────────────────────────────────────────────────────

/// Signed volume of a MESH1 body via the divergence theorem — EXACT for a closed,
/// planar-faced polyhedron, so prism arithmetic is testable to f32 precision.
/// Same routine as `wire_contract::mesh_volume` (each worker-backed test file
/// keeps its own harness).
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

/// One tessellated face, reduced to what a pick needs.
struct FaceProbe {
    topo_key: String,
    /// Every vertex of the face's triangles (world, f32-quantized by MESH1).
    verts: Vec<[f64; 3]>,
    /// Unit normal of the face's largest triangle.
    normal: [f64; 3],
    /// Max |distance| of any vertex from the plane through `verts[0]` with
    /// `normal` — ~0 for a planar face, large for a tessellated cylinder.
    planarity: f64,
    /// Mean world Z (used to tell the caps apart).
    avg_z: f64,
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Every face of a MESH1 body, probed for geometry a pick can route on.
fn face_probes(view: &MeshHeaderView, blob: &[u8]) -> Vec<FaceProbe> {
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
    (0..view.face_count as usize)
        .map(|f| {
            let first = u32_le(blob, frbase + f * 8) as usize;
            let count = u32_le(blob, frbase + f * 8 + 4) as usize;
            let mut verts = Vec::new();
            let mut best = ([0.0, 0.0, 1.0], 0.0f64);
            for t in first..first + count {
                let io = ibase + t * 12;
                let tri: Vec<[f64; 3]> = (0..3)
                    .map(|k| vertex(blob, pbase, u32_le(blob, io + k * 4) as usize))
                    .collect();
                let e0 = [
                    tri[1][0] - tri[0][0],
                    tri[1][1] - tri[0][1],
                    tri[1][2] - tri[0][2],
                ];
                let e1 = [
                    tri[2][0] - tri[0][0],
                    tri[2][1] - tri[0][1],
                    tri[2][2] - tri[0][2],
                ];
                let n = cross(e0, e1);
                let len = dot(n, n).sqrt();
                if len > best.1 {
                    best = ([n[0] / len, n[1] / len, n[2] / len], len);
                }
                verts.extend(tri);
            }
            let normal = best.0;
            let planarity = verts
                .iter()
                .map(|v| {
                    let d = [v[0] - verts[0][0], v[1] - verts[0][1], v[2] - verts[0][2]];
                    dot(d, normal).abs()
                })
                .fold(0.0f64, f64::max);
            let avg_z = verts.iter().map(|v| v[2]).sum::<f64>() / verts.len().max(1) as f64;
            FaceProbe {
                topo_key: keys[f].clone(),
                verts,
                normal,
                planarity,
                avg_z,
            }
        })
        .collect()
}

/// MESH1 vertices are f32, so a "point lies on this plane" check cannot be
/// tighter than f32 quantization at the model's scale (~1e-5 mm at 100 mm).
const MESH_PLANE_EPS: f64 = 1e-4;

// ── document / command driving over a managed `AppState` ─────────────────────

async fn state_regen(state: &AppState) -> RegenReport {
    let mut guard = state.runtime.lock().await;
    let rt = guard.as_mut().expect("runtime");
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

async fn state_add_op(state: &AppState, record: OperationRecord) {
    let mut guard = state.runtime.lock().await;
    add_op(guard.as_mut().expect("runtime"), record);
}

async fn state_mesh(state: &AppState, body: BodyId) -> Arc<Vec<u8>> {
    let mut guard = state.runtime.lock().await;
    body_mesh(guard.as_mut().expect("runtime"), body).await
}

async fn state_enter(state: &AppState, sid: SketchId) -> SketchSessionDto {
    let mut guard = state.runtime.lock().await;
    guard
        .as_mut()
        .expect("runtime")
        .enter_sketch(sid)
        .await
        .expect("enter_sketch")
}

async fn state_finish(state: &AppState, sid: SketchId) -> FinishSketchDto {
    let mut guard = state.runtime.lock().await;
    guard
        .as_mut()
        .expect("runtime")
        .finish_sketch(sid)
        .await
        .expect("finish_sketch")
}

async fn state_get_sketch(state: &AppState, sid: SketchId) -> SketchSessionDto {
    let guard = state.runtime.lock().await;
    guard
        .as_ref()
        .expect("runtime")
        .get_sketch(sid)
        .expect("get_sketch")
}

/// The frontend-facing projection (`SketchDto.hostFace` lives only here).
async fn state_projection(state: &AppState) -> onecad_lib::dto::DocumentProjection {
    let guard = state.runtime.lock().await;
    guard.as_ref().expect("runtime").projection()
}

/// `(record count, sketch count)` — the pair a failed command must not move.
async fn state_counts(state: &AppState) -> (usize, usize) {
    let guard = state.runtime.lock().await;
    let projection = guard.as_ref().expect("runtime").projection();
    (projection.features.len(), projection.sketches.len())
}

/// The typed, persisted sketch — round-tripped through the v2 container because
/// the attachment (and its `projectedBoundaryVersion`) is not on any DTO.
async fn saved_sketch(state: &AppState, path: &std::path::Path, sid: SketchId) -> Sketch {
    {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .expect("runtime")
            .save(path, save_meta())
            .expect("save");
    }
    ContainerReader::open(path)
        .expect("reopen container")
        .document()
        .sketch(sid)
        .cloned()
        .expect("the projected sketch survives the container")
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "sketch-on-face-w1b".into(),
        occt_fingerprint: None,
        created: "2026-08-01T00:00:00Z".into(),
        modified: "2026-08-01T00:00:00Z".into(),
    }
}

// ── wire-form readers (the DTO carries SCHEMA §7.3 JSON verbatim) ────────────

fn wire_entities(session: &SketchSessionDto) -> Vec<serde_json::Value> {
    session
        .entities
        .as_array()
        .cloned()
        .expect("entities is an array")
}

fn of_type<'a>(
    entities: &'a [serde_json::Value],
    ty: &str,
) -> Vec<&'a serde_json::Map<String, serde_json::Value>> {
    entities
        .iter()
        .filter_map(serde_json::Value::as_object)
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some(ty))
        .collect()
}

fn entity_id(e: &serde_json::Map<String, serde_json::Value>) -> EntityId {
    EntityId::from_str(e["id"].as_str().expect("id")).expect("uuid")
}

fn point_at(e: &serde_json::Map<String, serde_json::Value>) -> [f64; 2] {
    let at = e["at"].as_array().expect("at");
    [at[0].as_f64().expect("u"), at[1].as_f64().expect("v")]
}

// ── shared fixture: an 80×60 rect extruded 25 into a box ─────────────────────

const BOX_W: f64 = 80.0;
const BOX_D: f64 = 60.0;
const BOX_H: f64 = 25.0;
const SKETCH_B: u128 = 0xF02;
const OP_TAIL: u128 = 0xF03;
const ON_FACE_SKETCH: u128 = 0xFACE;

fn on_face_sketch_id() -> SketchId {
    SketchId(Uuid::from_u128(ON_FACE_SKETCH))
}

/// The `EditCommand::AddOperation` records for the base box (sketch + extrude).
fn box_records(sa: SketchId, draft_deg: f64) -> [OperationRecord; 2] {
    let sketch = sketch_record(SKETCH_A, &rect_sketch(sa, 0x2000, 0.0, 0.0, BOX_W, BOX_D));
    let mut extrude = extrude_record(EXTRUDE_A, sa, BOX_H);
    if draft_deg != 0.0 {
        if let Operation::Known(KnownOperation::Extrude(p)) = &mut extrude.op {
            p.draft_angle_deg = Scalar::new(draft_deg);
        }
    }
    [sketch, extrude]
}

/// Promotes a pick to a Rust-minted `ElementId` (the frontend's pick→promote step).
async fn promote(rt: &mut DocumentRuntime, snap: SnapshotId, body: BodyId, key: &str) -> String {
    rt.promote_selection(snap, body, vec![(TopoKey::new(key), None)])
        .await
        .expect("promote_selection")[0]
        .element_id
        .clone()
}

/// A blind `NewBody` extrude off an EXACT region of `sketch`.
fn region_extrude_record(rec: u128, sketch: SketchId, region: &str, dist: f64) -> OperationRecord {
    let mut record = extrude_record(rec, sketch, dist);
    if let Operation::Known(KnownOperation::Extrude(p)) = &mut record.op {
        p.profile = Some(SketchRegionRef {
            sketch,
            region: RegionId::new(region),
            extra: Default::default(),
        });
    }
    record
}

/// A feature-fused extrude of an EXACT region, bound to an existing body — the op
/// the HOST-BOOLEAN default authors when a face-hosted sketch is extruded
/// (`booleanMode` + `targetBodyId` have ridden the wire since MODEL-HARDEN W2;
/// what changed is that the tool now defaults to them).
///
/// `dist` is SIGNED against the sketch plane's normal, which for a face-hosted
/// sketch IS the host face's OUTWARD normal: positive grows away from the solid
/// (Add), negative pushes into it (Cut).
fn host_extrude_record(
    rec: u128,
    sketch: SketchId,
    region: &str,
    dist: f64,
    mode: BooleanMode,
    target: BodyId,
) -> OperationRecord {
    let mut record = region_extrude_record(rec, sketch, region, dist);
    if let Operation::Known(KnownOperation::Extrude(p)) = &mut record.op {
        p.boolean_mode = mode;
        p.target_body = Some(target);
    }
    record
}

/// A circle profile (centre point + circle) for the pocket cutter.
fn circle_sketch(sid: SketchId, base: u128, cx: f64, cy: f64, r: f64) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Pocket", WorldPlane::XY);
    let c = EntityId(Uuid::from_u128(base));
    sk.add_entity(SketchEntity::point(
        c,
        Vec2::new_unchecked(cx, cy),
        false,
        false,
    ))
    .unwrap();
    sk.add_entity(
        SketchEntity::circle(EntityId(Uuid::from_u128(base + 1)), c, r, false).expect("circle"),
    )
    .unwrap();
    sk
}

/// A `Cut` extrude of `sketch`'s first region into `target`.
fn cut_record(rec: u128, sketch: SketchId, target: BodyId, dist: f64) -> OperationRecord {
    let mut record = extrude_record(rec, sketch, dist);
    if let Operation::Known(KnownOperation::Extrude(p)) = &mut record.op {
        p.boolean_mode = BooleanMode::Cut;
        p.target_body = Some(target);
    }
    record
}

/// The base box plus a blind cylindrical pocket cut UP from the z=0 cap, so that
/// cap carries a real circular hole. Returns `(snapshot, body)`.
async fn build_pocket_box(rt: &mut DocumentRuntime) -> (SnapshotId, BodyId) {
    let sa = SketchId(Uuid::from_u128(0xF));
    let sp = SketchId(Uuid::from_u128(0x1F));
    for record in box_records(sa, 0.0) {
        add_op(rt, record);
    }
    add_op(
        rt,
        sketch_record(
            SKETCH_B,
            &circle_sketch(sp, 0x3000, BOX_W / 2.0, BOX_D / 2.0, 10.0),
        ),
    );
    add_op(rt, cut_record(OP_TAIL, sp, body_of(EXTRUDE_A), 10.0));
    let report = regen_all(rt).await;
    published(&report, "pocket box");
    (SnapshotId(report.snapshot_id), body_of(EXTRUDE_A))
}

/// Boilerplate every W1b test after (a) repeats: build a model, pick a face,
/// promote it, hand the runtime to a managed `AppState`, and run the command.
///
/// Returns the pieces the test needs afterwards. `app` is returned by value
/// because `tauri::State` borrows from it — the caller re-derives its own.
async fn projected_fixture(
    pick: impl Fn(&MeshHeaderView, &[u8]) -> String,
    build: impl for<'a> Fn(
        &'a mut DocumentRuntime,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = (SnapshotId, BodyId)> + 'a>,
    >,
) -> (
    WorkerManager,
    tauri::App<tauri::test::MockRuntime>,
    SketchId,
    BodyId,
    SnapshotId,
) {
    let bin = real_worker().expect("worker binary (callers gate on real_worker first)");
    let wm = spawn_worker(bin).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let (snap, body) = build(&mut rt).await;
    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("fixture MESH1");
    let key = pick(&view, &mesh);
    let face_el = promote(&mut rt, snap, body, &key).await;

    let app = tauri::test::mock_app();
    *app_state.runtime.lock().await = Some(rt);
    app.manage(app_state);
    let sid = on_face_sketch_id();
    onecad_lib::api::add_sketch_on_face(
        app.state(),
        snap.0,
        body_id_wire(body),
        face_el,
        Some(key),
        sid.to_string(),
        "OnFace".into(),
    )
    .await
    .expect("add_sketch_on_face");
    (wm, app, sid, body, snap)
}

/// The plain box with its +Z cap already projected into a sketch.
async fn box_with_projected_top() -> (
    WorkerManager,
    tauri::App<tauri::test::MockRuntime>,
    SketchId,
    BodyId,
    SnapshotId,
) {
    projected_fixture(top_face_pick, |rt| {
        Box::pin(async move {
            let sa = SketchId(Uuid::from_u128(0xF));
            for record in box_records(sa, 0.0) {
                add_op(rt, record);
            }
            let report = regen_all(rt).await;
            published(&report, "base box");
            (SnapshotId(report.snapshot_id), body_of(EXTRUDE_A))
        })
    })
    .await
}

/// The pocketed box with its holed z=0 cap already projected into a sketch.
async fn pocket_box_with_projected_bottom() -> (
    WorkerManager,
    tauri::App<tauri::test::MockRuntime>,
    SketchId,
    BodyId,
    SnapshotId,
) {
    projected_fixture(
        |view, blob| {
            // The z=0 cap — the one the blind pocket opened into.
            face_probes(view, blob)
                .into_iter()
                .filter(|p| p.planarity < MESH_PLANE_EPS)
                .min_by(|a, b| a.avg_z.total_cmp(&b.avg_z))
                .expect("a planar bottom cap")
                .topo_key
        },
        |rt| Box::pin(async move { build_pocket_box(rt).await }),
    )
    .await
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) The headline: a box top projects to 4 locked lines + 4 locked points +
//     4 Fixed, on a frame whose origin lies ON the face.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_sketch_on_face_projects_the_top_face_boundary_as_locked_geometry() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let sa = SketchId(Uuid::from_u128(0xF));
    for record in box_records(sa, 0.0) {
        add_op(&mut rt, record);
    }
    let report = regen_all(&mut rt).await;
    published(&report, "base box");
    let snap = SnapshotId(report.snapshot_id);
    let body = body_of(EXTRUDE_A);

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1");
    let top_key = top_face_pick(&view, &mesh);
    let face_el = promote(&mut rt, snap, body, &top_key).await;

    // A WORLD-plane sketch in the SAME document, so the projection's `hostFace`
    // is proven present-for-a-face-sketch AND absent-for-everything-else in one
    // read (a field that were always Some would pass a presence-only assertion).
    let world_sid = SketchId(Uuid::from_u128(0x000F_ACE0));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(world_sid, "World", WorldPlane::XY),
    })
    .expect("AddSketch on a world plane");

    let app = tauri::test::mock_app();
    *app_state.runtime.lock().await = Some(rt);
    app.manage(app_state);
    let sid = on_face_sketch_id();

    let dto = onecad_lib::api::add_sketch_on_face(
        app.state(),
        snap.0,
        body_id_wire(body),
        face_el.clone(),
        Some(top_key),
        sid.to_string(),
        "Top".into(),
    )
    .await
    .expect("add_sketch_on_face on a planar box cap");

    // The id the CALLER minted is the document id — the frontend keys its
    // projection store by it, so a re-minted id would strand that entry.
    assert_eq!(
        dto.sketch_id,
        sid.to_string(),
        "the sketch id is adopted verbatim"
    );
    assert_eq!(dto.entity_count, 8, "4 boundary points + 4 boundary lines");
    assert_eq!(dto.constraint_count, 4, "one Fixed per projected point");
    assert_eq!(
        dto.face_count, 1,
        "a plain box cap has no coplanar companions"
    );
    assert!(dto.has_closed_boundary);
    assert_eq!(dto.projected_boundary_version, 1);

    // The frame: +Z normal, and an origin that lies ON the cap (z == box height).
    assert!(
        (dto.plane.normal[2] - 1.0).abs() < 1e-9,
        "the cap faces +Z, got {:?}",
        dto.plane.normal
    );
    assert!(
        (dto.plane.origin[2] - BOX_H).abs() < 1e-9,
        "the kernel-exact origin lies ON the cap plane (z == {BOX_H}), got {}",
        dto.plane.origin[2]
    );

    let state: tauri::State<'_, AppState> = app.state();

    // The PROJECTION carries the host binding (SKETCH-ON-FACE W3): it is the only
    // route by which the frontend can answer "is there already a sketch on this
    // face?" — the attachment itself never leaves the core. Identity only, and
    // rendered exactly as `BodyDto.id` / a promoted `elementId` are, so a face
    // pick compares `==` against it without any normalization.
    let proj = state_projection(&state).await;
    let host = proj.sketches[&sid.to_string()]
        .host_face
        .as_ref()
        .expect("a face-hosted sketch projects its host face");
    assert_eq!(host.body_id, body.to_string());
    assert_eq!(host.element_id, face_el);
    assert!(
        proj.sketches[&world_sid.to_string()].host_face.is_none(),
        "a world-plane sketch has no host face"
    );

    let session = state_get_sketch(&state, sid).await;
    let entities = wire_entities(&session);
    assert_eq!(of_type(&entities, "Point").len(), 4);
    assert_eq!(of_type(&entities, "Line").len(), 4);
    assert!(
        entities
            .iter()
            .all(|e| e.get("referenceLocked").and_then(|v| v.as_bool()) == Some(true)),
        "EVERY projected entity is locked — points included ({entities:#?})"
    );
    assert!(
        entities
            .iter()
            .all(|e| e.get("construction").and_then(|v| v.as_bool()) != Some(true)),
        "projected geometry is real, not construction — it must bound regions"
    );
    let constraints = session
        .constraints
        .as_array()
        .cloned()
        .expect("constraints");
    assert_eq!(constraints.len(), 4);
    assert!(
        constraints
            .iter()
            .all(|c| c.get("type").and_then(|t| t.as_str()) == Some("Fixed")),
        "every projected point is pinned ({constraints:#?})"
    );

    // The projected rectangle is the cap's real 80×60 outline (side lengths, in
    // whichever order the wire explorer walked it).
    let points: Vec<[f64; 2]> = of_type(&entities, "Point")
        .iter()
        .map(|p| point_at(p))
        .collect();
    let (mut umin, mut umax, mut vmin, mut vmax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in &points {
        umin = umin.min(p[0]);
        umax = umax.max(p[0]);
        vmin = vmin.min(p[1]);
        vmax = vmax.max(p[1]);
    }
    let mut span = [umax - umin, vmax - vmin];
    span.sort_by(f64::total_cmp);
    assert!(
        (span[0] - BOX_D).abs() < 1e-6 && (span[1] - BOX_W).abs() < 1e-6,
        "the projected outline is the cap's own {BOX_W}×{BOX_D} rectangle, got {span:?}"
    );

    // The attachment + version, read back through the container (no DTO carries them).
    let dir = tempfile::tempdir().unwrap();
    let sketch = saved_sketch(&state, &dir.path().join("OnFace.onecad"), sid).await;
    let (face, version) = match &sketch.attachment {
        SketchAttachment::HostFace {
            face,
            projected_boundary_version,
        } => (face, *projected_boundary_version),
        other => panic!("expected a HostFace attachment, got {other:?}"),
    };
    assert_eq!(version, 1);
    let primary = face.primary.as_ref().expect("the picked face is bound");
    assert_eq!(primary.body, body);
    assert_eq!(primary.kind, onecad_core::document::refs::ElementKind::Face);
    let anchor = face.anchor.as_ref().expect("an anchor is captured");
    assert!(
        (anchor.world_point.z - BOX_H).abs() < 1e-9,
        "the anchor is the kernel-exact ON-plane origin, not a bbox centre"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (b) Every projected point is Fixed, so the fresh sketch enters at dof 0.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_projected_sketch_enters_fully_constrained_at_dof_zero() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let (wm, app, sid, _body, _snap) = box_with_projected_top().await;
    let state: tauri::State<'_, AppState> = app.state();

    let session = state_enter(&state, sid).await;
    assert_eq!(
        session.dof, 0,
        "4 points × 2 dof, each removed by its own Fixed — a projected boundary is rigid"
    );
    assert_eq!(session.status, SketchSolveStatus::FullyConstrained);
    assert!(
        session.conflicting.is_empty(),
        "the Fixed set is consistent"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) The projected lines close into exactly one region.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_projected_boundary_closes_into_exactly_one_region() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let (wm, app, sid, _body, _snap) = box_with_projected_top().await;
    let state: tauri::State<'_, AppState> = app.state();

    state_enter(&state, sid).await;
    let regions = state_finish(&state, sid).await.regions;
    assert_eq!(
        regions.len(),
        1,
        "the four projected lines form ONE closed profile, got {regions:#?}"
    );
    assert!(regions[0].holes.is_empty(), "a plain cap has no holes");

    // The region's boundary IS the projected geometry — locked entities bound
    // regions (the deliberate contrast with construction geometry, SCHEMA §7.3).
    let lines: BTreeSet<String> =
        of_type(&wire_entities(&state_get_sketch(&state, sid).await), "Line")
            .iter()
            .map(|l| l["id"].as_str().expect("id").to_string())
            .collect();
    let outer: BTreeSet<String> = regions[0].outer_loop.iter().cloned().collect();
    assert_eq!(
        outer, lines,
        "the region's outer loop is exactly the four projected lines"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (d) Extruding the projected region by its EXACT regionId lands a prism of the
//     face's own footprint, OUTSIDE the host body (the handedness proof).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extruding_the_projected_region_lands_outside_the_host_face() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let (wm, app, sid, _body, _snap) = box_with_projected_top().await;
    let state: tauri::State<'_, AppState> = app.state();

    // enter → finish MINTS the Sketch timeline record; the regen planner resolves
    // an extrude's profile only from such a record (EXTRUDE-COMMIT-FIX).
    state_enter(&state, sid).await;
    let regions = state_finish(&state, sid).await.regions;
    assert_eq!(regions.len(), 1);

    const RISER: f64 = 7.0;
    state_add_op(
        &state,
        region_extrude_record(OP_TAIL, sid, &regions[0].region_id, RISER),
    )
    .await;
    let report = state_regen(&state).await;
    published(&report, "riser off the projected region");

    let mesh = state_mesh(&state, body_of(OP_TAIL)).await;
    let view = validate_mesh_blob(&mesh).expect("riser MESH1");
    let vol = mesh_volume(&view, &mesh);
    let want = BOX_W * BOX_D * RISER;
    assert!(
        (vol - want).abs() < 1.0,
        "the riser is the cap's own footprint × {RISER} = {want}, got {vol}"
    );
    // HANDEDNESS: the sketch plane's normal is the face's OUTWARD normal, so a
    // positive extrude grows AWAY from the solid. An inverted frame would build
    // the riser back down into the box and this bbox would start below the cap.
    assert!(
        f64::from(view.bbox_min[2]) > BOX_H - 1e-3,
        "the riser starts at the host face (z ≈ {BOX_H}), got {}",
        view.bbox_min[2]
    );
    assert!(
        f64::from(view.bbox_max[2]) > BOX_H + RISER - 1e-3,
        "the riser grows past the host face to z ≈ {}, got {}",
        BOX_H + RISER,
        view.bbox_max[2]
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (e) A TILTED planar face — the case a descriptor `center` gets wrong.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tilted_face_projects_onto_its_own_plane_and_extrudes_to_area_times_height() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    // A 10° DRAFTED extrude: every side face is a planar trapezoid tilted off
    // both the world axes and the caps. (A Chamfer would also do it, but its edge
    // input binds through the resolution ladder by anchor — `wire_contract`'s own
    // chamfer test accepts a clean NeedsRepair — so the tilt would not be
    // deterministic. The draft is.)
    let sa = SketchId(Uuid::from_u128(0xF));
    for record in box_records(sa, 10.0) {
        add_op(&mut rt, record);
    }
    let report = regen_all(&mut rt).await;
    published(&report, "drafted box");
    let snap = SnapshotId(report.snapshot_id);
    let body = body_of(EXTRUDE_A);

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("drafted box MESH1");
    let probes = face_probes(&view, &mesh);
    let tilted = probes
        .iter()
        // Planar, but aligned with neither the caps nor the un-drafted sides.
        .find(|p| {
            p.planarity < MESH_PLANE_EPS && p.normal[2].abs() > 0.05 && p.normal[2].abs() < 0.95
        })
        .expect("a 10° draft tilts every side face off the world axes");
    let tilt_normal = tilted.normal;
    let tilt_verts = tilted.verts.clone();
    let tilt_key = tilted.topo_key.clone();

    // Ground truth for the volume assertion: the kernel's EXACT face area
    // (`BRepGProp` via the element descriptor's `magnitude`), not a value
    // re-derived from the f32 tessellation.
    let area = ElementQuery::query_element_by_topo_key(&wm, snap, body, &tilt_key)
        .await
        .expect("QueryElement")
        .expect("the tilted face resolves")
        .magnitude;
    assert!(
        area > 0.0,
        "the kernel reports the tilted face's exact area"
    );
    let face_el = promote(&mut rt, snap, body, &tilt_key).await;

    let app = tauri::test::mock_app();
    *app_state.runtime.lock().await = Some(rt);
    app.manage(app_state);
    let sid = on_face_sketch_id();

    let dto = onecad_lib::api::add_sketch_on_face(
        app.state(),
        snap.0,
        body_id_wire(body),
        face_el,
        Some(tilt_key),
        sid.to_string(),
        "Tilted".into(),
    )
    .await
    .expect("a tilted PLANAR face hosts a sketch");

    // THE assertion this whole handshake exists for: the frozen origin lies ON
    // the tilted face's plane. A descriptor `center` is an axis-aligned bbox
    // centre and would sit off-plane here, extruding a sliver.
    //
    // Tolerance is f32, not 1e-9: the only vertices available are the MESH1
    // ones, which the format stores as f32 (see MESH_PLANE_EPS).
    let origin = dto.plane.origin;
    let worst = tilt_verts
        .iter()
        .map(|v| {
            dot(
                [origin[0] - v[0], origin[1] - v[1], origin[2] - v[2]],
                tilt_normal,
            )
            .abs()
        })
        .fold(0.0f64, f64::max);
    assert!(
        worst < MESH_PLANE_EPS,
        "the frozen origin must lie ON the tilted face plane; worst |n·(origin−v)| = {worst}"
    );
    assert!(
        (dot(dto.plane.normal, tilt_normal).abs() - 1.0).abs() < 1e-3,
        "the frame's normal is the face's own, got {:?} vs mesh {tilt_normal:?}",
        dto.plane.normal
    );

    // …and the projected profile is the face itself: extruding it `RISER` gives a
    // prism of EXACTLY area × RISER.
    let state: tauri::State<'_, AppState> = app.state();
    state_enter(&state, sid).await;
    let regions = state_finish(&state, sid).await.regions;
    assert_eq!(regions.len(), 1, "the tilted face's outline closes once");

    const RISER: f64 = 6.0;
    state_add_op(
        &state,
        region_extrude_record(OP_TAIL, sid, &regions[0].region_id, RISER),
    )
    .await;
    published(&state_regen(&state).await, "tilted riser");
    let riser = state_mesh(&state, body_of(OP_TAIL)).await;
    let riser_view = validate_mesh_blob(&riser).expect("tilted riser MESH1");
    let vol = mesh_volume(&riser_view, &riser);
    let want = area * RISER;
    assert!(
        (vol - want).abs() / want < 1e-3,
        "prism volume = kernel face area ({area}) × {RISER} = {want}, got {vol}"
    );
    eprintln!("tilted face PASS: area {area} × {RISER} = {want}, mesh volume {vol}");
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (f) A face with a HOLE projects the hole as a Circle and yields two regions.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_face_with_a_pocket_projects_a_circle_and_lists_both_regions() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let (wm, app, sid, _body, _snap) = pocket_box_with_projected_bottom().await;
    let state: tauri::State<'_, AppState> = app.state();

    let entities = wire_entities(&state_get_sketch(&state, sid).await);
    assert_eq!(
        of_type(&entities, "Circle").len(),
        1,
        "the cylindrical pocket's rim is an EXACT Circle, not a polyline fallback \
         ({entities:#?})"
    );
    assert_eq!(
        of_type(&entities, "Line").len(),
        4,
        "plus the cap's outline"
    );
    assert!(
        entities
            .iter()
            .all(|e| e.get("referenceLocked").and_then(|v| v.as_bool()) == Some(true)),
        "the hole is locked reference geometry too"
    );

    state_enter(&state, sid).await;
    let regions = state_finish(&state, sid).await.regions;
    assert_eq!(
        regions.len(),
        2,
        "annulus (rect with the circle as a hole) + the inner disk, got {regions:#?}"
    );
    let annulus = regions
        .iter()
        .find(|r| !r.holes.is_empty())
        .expect("one region carries the circle as a hole");
    let disk = regions
        .iter()
        .find(|r| r.holes.is_empty())
        .expect("the inner disk is listed independently");
    assert_ne!(annulus.region_id, disk.region_id, "two distinct regions");
    assert_eq!(annulus.holes.len(), 1);
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (g) A NON-PLANAR face fails LOUDLY and leaves no orphan behind.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_planar_face_is_refused_and_the_document_is_untouched() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    let (snap, body) = build_pocket_box(&mut rt).await;
    // (fixture built inline: this test needs the CURVED face, not a projected one)

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("pocket box MESH1");
    let probes = face_probes(&view, &mesh);
    let curved = probes
        .iter()
        .max_by(|a, b| a.planarity.total_cmp(&b.planarity))
        .filter(|p| p.planarity > 1e-2)
        .expect("the cylindrical pocket wall tessellates far off any single plane");
    let curved_key = curved.topo_key.clone();
    let face_el = promote(&mut rt, snap, body, &curved_key).await;

    let app = tauri::test::mock_app();
    *app_state.runtime.lock().await = Some(rt);
    app.manage(app_state);
    let state: tauri::State<'_, AppState> = app.state();
    let before = state_counts(&state).await;

    let err = onecad_lib::api::add_sketch_on_face(
        app.state(),
        snap.0,
        body_id_wire(body),
        face_el,
        Some(curved_key),
        on_face_sketch_id().to_string(),
        "Cylinder".into(),
    )
    .await
    .expect_err("a cylindrical face can never host a sketch — it is never approximated");
    let message = err.to_string();
    assert!(
        message.contains("planar"),
        "the refusal must name the reason, got {message:?}"
    );

    // No orphan: the failure happens before anything is applied.
    assert_eq!(
        state_counts(&state).await,
        before,
        "a refused projection leaves the timeline AND the sketch set untouched"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (h) A user dimension AGAINST the boundary drives the USER's geometry — the
//     projected point does not budge. (Adding a constraint against locked
//     geometry is deliberately allowed; only mutation is refused.)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_distance_to_a_projected_point_solves_without_moving_it() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let (wm, app, sid, _body, _snap) = box_with_projected_top().await;
    let state: tauri::State<'_, AppState> = app.state();
    state_enter(&state, sid).await;

    let entities = wire_entities(&state_get_sketch(&state, sid).await);
    let anchor_wire = of_type(&entities, "Point")[0];
    let anchor_id = entity_id(anchor_wire);
    let anchor_at = point_at(anchor_wire);

    let free = EntityId(Uuid::from_u128(0xF9EE));
    let solved = {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .expect("runtime")
            .sketch_upsert(
                sid,
                vec![
                    SketchEditOp::AddEntity {
                        entity: SketchEntity::point(
                            free,
                            Vec2::new_unchecked(anchor_at[0] + 3.0, anchor_at[1] + 4.0),
                            false,
                            false,
                        ),
                    },
                    SketchEditOp::AddConstraint {
                        constraint: Constraint::Distance {
                            id: ConstraintId(Uuid::from_u128(0xD15)),
                            entity1: free,
                            entity2: anchor_id,
                            value: Scalar::new(12.0),
                        },
                    },
                ],
            )
            .await
            .expect("a constraint AGAINST locked geometry is allowed (only mutation is refused)")
    };
    assert_ne!(
        solved.status,
        SketchSolveStatus::OverConstrained,
        "pinning the boundary must not make a user dimension redundant"
    );

    let after = wire_entities(&state_get_sketch(&state, sid).await);
    let moved = of_type(&after, "Point")
        .into_iter()
        .find(|p| entity_id(p) == anchor_id)
        .expect("the projected point survives");
    let now = point_at(moved);
    assert!(
        (now[0] - anchor_at[0]).abs() < 1e-9 && (now[1] - anchor_at[1]).abs() < 1e-9,
        "the projected point is Fixed: the solve moved the USER's point instead \
         ({anchor_at:?} → {now:?})"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (i) Removing a projected line is refused LOUDLY (`SketchError::ReferenceLocked`).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_a_projected_line_is_refused_as_reference_locked() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let (wm, app, sid, _body, _snap) = box_with_projected_top().await;
    let state: tauri::State<'_, AppState> = app.state();
    state_enter(&state, sid).await;

    let before = wire_entities(&state_get_sketch(&state, sid).await);
    let victim = entity_id(of_type(&before, "Line")[0]);

    let err = {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .expect("runtime")
            .sketch_upsert(sid, vec![SketchEditOp::RemoveEntity { entity: victim }])
            .await
            .expect_err("projected boundary geometry must not be removable")
    };
    let message = err.to_string();
    // The domain error text is the contract (`SketchError::ReferenceLocked`) —
    // a partial edit of a projected boundary is worse than a rejected one.
    let expected = SketchError::ReferenceLocked(victim).to_string();
    assert!(
        message.contains(&expected),
        "expected {expected:?} inside the refusal, got {message:?}"
    );

    let after = wire_entities(&state_get_sketch(&state, sid).await);
    assert_eq!(
        after.len(),
        before.len(),
        "the batch is atomic: a rejected edit changes nothing"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (j) Save → FRESH worker reopen: locks, Fixed set, attachment, version and the
//     derived regions all survive the container round-trip.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_projected_sketch_survives_save_and_a_fresh_worker_reopen() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let (wm, app, sid, _body, _snap) = box_with_projected_top().await;
    let state: tauri::State<'_, AppState> = app.state();
    state_enter(&state, sid).await;
    let before_regions = state_finish(&state, sid).await.regions;
    assert_eq!(before_regions.len(), 1);
    let before_entities = wire_entities(&state_get_sketch(&state, sid).await);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Projected.onecad");
    {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .expect("runtime")
            .save(&path, save_meta())
            .expect("save");
    }
    wm.shutdown().await;

    // A genuinely FRESH worker process — nothing about the reopened sketch may
    // depend on solver-lane state left behind by the first one.
    let bin2 = real_worker().expect("worker binary");
    let wm2 = spawn_worker(bin2).await;
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm2.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm2.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm2.clone());
    let mut reopened = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");
    published(&regen_all(&mut reopened).await, "reopened document");

    let session = reopened.get_sketch(sid).expect("the sketch reopened");
    let entities = wire_entities(&session);
    assert_eq!(entities.len(), before_entities.len());
    assert!(
        entities
            .iter()
            .all(|e| e.get("referenceLocked").and_then(|v| v.as_bool()) == Some(true)),
        "the lock flag survives the v2 container ({entities:#?})"
    );
    assert_eq!(
        session
            .constraints
            .as_array()
            .expect("constraints")
            .iter()
            .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("Fixed"))
            .count(),
        4,
        "the Fixed pins survive"
    );

    let sketch = ContainerReader::open(&path)
        .expect("container")
        .document()
        .sketch(sid)
        .cloned()
        .expect("sketch");
    let version = match &sketch.attachment {
        SketchAttachment::HostFace {
            projected_boundary_version,
            ..
        } => *projected_boundary_version,
        other => panic!("the HostFace attachment must survive, got {other:?}"),
    };
    assert_eq!(version, 1, "the version survives");

    reopened.enter_sketch(sid).await.expect("enter reopened");
    let after_regions = reopened.finish_sketch(sid).await.expect("finish").regions;
    assert_eq!(
        after_regions
            .iter()
            .map(|r| &r.region_id)
            .collect::<Vec<_>>(),
        before_regions
            .iter()
            .map(|r| &r.region_id)
            .collect::<Vec<_>>(),
        "regions re-derive IDENTICALLY on a fresh worker (region ids are \
         content-addressed, so a drift here means the geometry drifted)"
    );
    wm2.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (k) A user line drawn ACROSS the projected rectangle splits it into two
//     independently extrudable regions whose prisms sum to the full one.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_line_across_the_projected_rect_yields_two_extrudable_regions() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let (wm, app, sid, _body, _snap) = box_with_projected_top().await;
    let state: tauri::State<'_, AppState> = app.state();
    state_enter(&state, sid).await;

    // The projected rect's own UV extent — the chord must land ON two opposite
    // edges, so it is derived from the projection rather than assumed.
    let entities = wire_entities(&state_get_sketch(&state, sid).await);
    let pts: Vec<[f64; 2]> = of_type(&entities, "Point")
        .iter()
        .map(|p| point_at(p))
        .collect();
    let (umin, umax) = (
        pts.iter().map(|p| p[0]).fold(f64::MAX, f64::min),
        pts.iter().map(|p| p[0]).fold(f64::MIN, f64::max),
    );
    let (vmin, vmax) = (
        pts.iter().map(|p| p[1]).fold(f64::MAX, f64::min),
        pts.iter().map(|p| p[1]).fold(f64::MIN, f64::max),
    );
    let vmid = 0.5 * (vmin + vmax);

    let (a, b, chord) = (
        EntityId(Uuid::from_u128(0xC0DE1)),
        EntityId(Uuid::from_u128(0xC0DE2)),
        EntityId(Uuid::from_u128(0xC0DE3)),
    );
    {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .expect("runtime")
            .sketch_upsert(
                sid,
                vec![
                    SketchEditOp::AddEntity {
                        entity: SketchEntity::point(
                            a,
                            Vec2::new_unchecked(umin, vmid),
                            false,
                            false,
                        ),
                    },
                    SketchEditOp::AddEntity {
                        entity: SketchEntity::point(
                            b,
                            Vec2::new_unchecked(umax, vmid),
                            false,
                            false,
                        ),
                    },
                    SketchEditOp::AddEntity {
                        entity: SketchEntity::line(chord, a, b, false),
                    },
                ],
            )
            .await
            .expect("drawing across locked geometry is ordinary authoring");
    }

    let regions = state_finish(&state, sid).await.regions;
    assert_eq!(
        regions.len(),
        2,
        "the chord splits the projected outline in two, got {regions:#?}"
    );

    const RISER: f64 = 4.0;
    let mut total = 0.0;
    for (i, region) in regions.iter().enumerate() {
        let rec = OP_TAIL + i as u128;
        state_add_op(
            &state,
            region_extrude_record(rec, sid, &region.region_id, RISER),
        )
        .await;
        published(&state_regen(&state).await, "half riser");
        let mesh = state_mesh(&state, body_of(rec)).await;
        let view = validate_mesh_blob(&mesh).expect("half riser MESH1");
        let vol = mesh_volume(&view, &mesh);
        let want = BOX_W * BOX_D * RISER / 2.0;
        assert!(
            (vol - want).abs() < 1.0,
            "each half is {want}, got {vol} (region {})",
            region.region_id
        );
        total += vol;
    }
    let want_total = BOX_W * BOX_D * RISER;
    assert!(
        (total - want_total).abs() < 1.0,
        "the halves sum to the full prism {want_total}, got {total}"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (k) HOST-BOOLEAN — the user story a face-hosted sketch exists for: the op it
//     drives MODIFIES its host body instead of spawning a new one, and the drag
//     DIRECTION decides whether that is material added or removed.
//
//     The tool-layer defaults live in the frontend (`ModelToolController`); what
//     these two prove is the other end — that the exact op those defaults author
//     lands the exact solid, against the real kernel. Together with the extrude
//     handedness proof in (d) (`+dist` grows AWAY from the host face), they pin
//     the whole push/pull loop numerically.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_add_extrude_grows_the_host_body_and_mints_no_new_one() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let (wm, app, sid, host, _snap) = box_with_projected_top().await;
    let state: tauri::State<'_, AppState> = app.state();

    state_enter(&state, sid).await;
    let regions = state_finish(&state, sid).await.regions;
    assert_eq!(regions.len(), 1);

    // Dragging AWAY from the host face (positive against its outward normal).
    const RISER: f64 = 7.0;
    state_add_op(
        &state,
        host_extrude_record(
            OP_TAIL,
            sid,
            &regions[0].region_id,
            RISER,
            BooleanMode::Add,
            host,
        ),
    )
    .await;
    let report = state_regen(&state).await;
    let snapshot = published(&report, "host Add");

    // MODIFIED, not created. The whole point of the default: after the op the
    // document still holds ONE body, and it is the host — a NewBody extrude would
    // leave two, with `body_<opId>` sitting next to the part.
    assert_eq!(
        snapshot.bodies.len(),
        1,
        "an Add fuses into the host — the document must not gain a body, got {:#?}",
        snapshot.bodies.iter().map(|b| b.body).collect::<Vec<_>>()
    );
    assert_eq!(
        snapshot.bodies[0].body, host,
        "the surviving body is the HOST"
    );
    assert!(
        report.changed.iter().any(|(b, _)| *b == host),
        "the host is reported CHANGED (its mesh was rebuilt), got {:?}",
        report.changed
    );
    assert!(
        !report.changed.iter().any(|(b, _)| *b == body_of(OP_TAIL)),
        "the NewBody id was never minted, got {:?}",
        report.changed
    );
    assert!(report.removed.is_empty(), "nothing was removed by an Add");

    // Exact: the host is now its own volume PLUS the cap-footprint riser.
    let mesh = state_mesh(&state, host).await;
    let view = validate_mesh_blob(&mesh).expect("fused MESH1");
    let vol = mesh_volume(&view, &mesh);
    let want = BOX_W * BOX_D * (BOX_H + RISER);
    assert!(
        (vol - want).abs() < 1.0,
        "box + riser = {BOX_W}·{BOX_D}·({BOX_H}+{RISER}) = {want}, got {vol}"
    );
    assert!(
        f64::from(view.bbox_max[2]) > BOX_H + RISER - 1e-3,
        "the fused solid reaches z ≈ {}, got {}",
        BOX_H + RISER,
        view.bbox_max[2]
    );
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_cut_extrude_removes_material_from_the_host_body() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let (wm, app, sid, host, _snap) = box_with_projected_top().await;
    let state: tauri::State<'_, AppState> = app.state();

    state_enter(&state, sid).await;
    let regions = state_finish(&state, sid).await.regions;
    assert_eq!(regions.len(), 1);

    // Dragging INTO the host: NEGATIVE against the face's outward normal, which is
    // exactly what the direction-aware default turns into a Cut.
    const DEPTH: f64 = 7.0;
    state_add_op(
        &state,
        host_extrude_record(
            OP_TAIL,
            sid,
            &regions[0].region_id,
            -DEPTH,
            BooleanMode::Cut,
            host,
        ),
    )
    .await;
    let report = state_regen(&state).await;
    let snapshot = published(&report, "host Cut");

    assert_eq!(
        snapshot.bodies.len(),
        1,
        "a Cut into the host leaves one body, got {:#?}",
        snapshot.bodies.iter().map(|b| b.body).collect::<Vec<_>>()
    );
    assert_eq!(snapshot.bodies[0].body, host);

    // Exact: the cap footprint × DEPTH is GONE. A sign error here would either
    // remove nothing (the tool sitting outside the solid) or fail loudly — this is
    // the numeric proof that "drag in" really subtracts.
    let mesh = state_mesh(&state, host).await;
    let view = validate_mesh_blob(&mesh).expect("pocketed MESH1");
    let vol = mesh_volume(&view, &mesh);
    let want = BOX_W * BOX_D * (BOX_H - DEPTH);
    assert!(
        (vol - want).abs() < 1.0,
        "box − pocket = {BOX_W}·{BOX_D}·({BOX_H}−{DEPTH}) = {want}, got {vol}"
    );
    assert!(
        (f64::from(view.bbox_max[2]) - (BOX_H - DEPTH)).abs() < 1e-2,
        "the cut face is the new top at z ≈ {}, got {}",
        BOX_H - DEPTH,
        view.bbox_max[2]
    );
    wm.shutdown().await;
}
