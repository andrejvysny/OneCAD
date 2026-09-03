//! WP-P end-to-end gate: project body edges into a sketch, extrude the region
//! they bound, then edit the source and watch the projection go stale, update,
//! and detach. Real C++ OCCT worker over OCW1, driven through the app's ACTUAL
//! `#[tauri::command]` fns via `tauri::test::mock_app`.
//!
//! What each assertion is really pinning:
//!
//! * **`faceOutline` closes into a region.** Reference-locked geometry BOUNDS
//!   regions (unlike construction geometry), so a projected face outline must be
//!   extrudable. The volume is analytic, not a golden number.
//! * **A projection does NOT track its source.** After the box widens, the
//!   projected rectangle is still the OLD rectangle — that is the point of a
//!   frozen projection, and the reason a staleness warning exists at all.
//! * **`PROJECTION_STALE` is a WARNING that appears, and clears.** It is merged
//!   onto the sketch's timeline step beside `EXPR_UNRESOLVED`, from an UNLOCKED
//!   post-publish probe that drops its own verdict if the head moved.
//! * **`update_projection` keeps the entity ids.** An update moves geometry; it
//!   does not re-mint identity, or every constraint hung off a projected line and
//!   every derived `regionId` would move with it.
//! * **`detach_projection` really frees the geometry**: the entities are no longer
//!   reference-locked, their `Fixed` pins are gone, and the provenance map is
//!   empty.
//! * **Two loud refusals**: a projection into a sketch with an open drag gesture,
//!   and a promote → project → commit whose head moved underneath it.
//!
//! Gated on `ONECAD_WORKER_PATH` / `ONECAD_REQUIRE_WORKER` like every other
//! worker-backed test in this suite, and additionally on the worker actually
//! carrying the `ProjectToSketchPlane` verb — a worker without it must FAIL here,
//! never skip green.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use tauri::Manager;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId, SnapshotId};
use onecad_core::math::Vec2;
use onecad_core::regen::{CancelToken, GeometryEngine, Outcome, RegenRequest};
use onecad_core::sketch::{
    Constraint, CurvePosition, Sketch, SketchAttachment, SketchEntity, WorldPlane,
};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport, PROJECTION_STALE_CODE};
use onecad_lib::export::GeometryExporter;
use onecad_lib::state::AppState;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::body_id_wire;
use onecad_lib::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors sketch_on_face.rs / wire_contract.rs — each worker-backed test
// file keeps its own small harness rather than sharing a `tests/common` module).
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

const EXTRUDE_BASE: u128 = 0xB01;
const EXTRUDE_PROJ: u128 = 0xB03;
const EXTRUDE_CUT: u128 = 0xB05;

const SID_BASE: u128 = 0xB10;
const SID_PROJ: u128 = 0xB11;
const SID_NOTCH: u128 = 0xB12;

/// The base rect's width dimension — edited later to widen the box.
const WIDTH_DIM: u128 = 0x2000 + 0x40 + 10;

/// A fully-constrained rectangle (8 points, 4 lines, coincident corners, H/V, a
/// Fixed anchor, H/V dimensions) — the same fixture shape `sketch_on_face.rs`
/// uses. `c(10)` is the WIDTH dimension this test later edits.
fn rect_sketch(sid: SketchId, base: u128, w: f64, h: f64) -> Sketch {
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
    pt(&mut sk, p0s, 0.0, 0.0);
    pt(&mut sk, p0e, w, 0.0);
    pt(&mut sk, p1s, w, 0.0);
    pt(&mut sk, p1e, w, h);
    pt(&mut sk, p2s, w, h);
    pt(&mut sk, p2e, 0.0, h);
    pt(&mut sk, p3s, 0.0, h);
    pt(&mut sk, p3e, 0.0, 0.0);
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
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(0.0, 0.0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: p0s,
        point1_position: CurvePosition::Arbitrary,
        point2: p0e,
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(w),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: p1s,
        point1_position: CurvePosition::Arbitrary,
        point2: p1e,
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(h),
    })
    .unwrap();
    sk
}

/// Builds the 40 x 20 x 25 base box.
///
/// The rect goes in through `AddSketch` (so it lives in `document.sketches` and
/// its width dimension stays editable) and `finish_sketch` mints its timeline
/// record — the same order the interactive flow uses. Authoring the record
/// directly would leave no document sketch to edit later.
async fn build_box(rt: &mut DocumentRuntime, base: SketchId) {
    rt.apply(EditCommand::AddSketch {
        sketch: rect_sketch(base, 0x2000, 40.0, 20.0),
    })
    .expect("AddSketch(base rect)");
    rt.finish_sketch(base).await.expect("finishSketch(base)");
    add_op(rt, extrude_record(EXTRUDE_BASE, base, 25.0));
}

fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // empty ⇒ V1 first-region fallback
            region_identity_version: None,
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

/// A fully-pinned axis-aligned rectangle `[x0,x1] × [y0,y1]` — the notch profile
/// the blocker case cuts with. Four points shared by four lines is already a
/// closed loop to the region finder (the same shape a projection produces), and
/// four `Fixed` pins make it fully constrained without a dimension to edit.
fn pinned_rect(sid: SketchId, base: u128, x0: f64, y0: f64, x1: f64, y1: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let mut sk = Sketch::on_world_plane(sid, "Notch", WorldPlane::XY);
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    for (i, (x, y)) in corners.iter().enumerate() {
        sk.add_entity(SketchEntity::point(
            e(i as u128),
            Vec2::new_unchecked(*x, *y),
            false,
            false,
        ))
        .unwrap();
    }
    for i in 0..4u128 {
        sk.add_entity(SketchEntity::line(e(0x10 + i), e(i), e((i + 1) % 4), false))
            .unwrap();
    }
    for (i, (x, y)) in corners.iter().enumerate() {
        sk.add_constraint(Constraint::Fixed {
            id: c(i as u128),
            point: e(i as u128),
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new_unchecked(*x, *y),
        })
        .unwrap();
    }
    sk
}

/// The same extrude, as a CUT into `target`.
fn cut_record(rec: u128, sketch: SketchId, dist: f64, target: BodyId) -> OperationRecord {
    let mut record = extrude_record(rec, sketch, dist);
    if let Operation::Known(KnownOperation::Extrude(params)) = &mut record.op {
        params.boolean_mode = BooleanMode::Cut;
        params.target_body = Some(target);
    }
    record
}

// ── MESH1 helpers (exact for a planar-faced box) ─────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;
const SEC_EDGE_RANGES: u32 = 7;
const SEC_EDGE_POSITIONS: u32 = 8;
const SEC_EDGE_ID_OFFS: u32 = 9;
const SEC_EDGE_ID_CHARS: u32 = 10;

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

/// The `TopoKey` of the face with the greatest average world Z (the extrude's far
/// cap).
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
    keys[best.expect("at least one face").0].clone()
}

/// The `TopoKey` of the face with the LOWEST average world Y — the box's `y = 0`
/// side wall, which sees the top face edge-on.
fn side_face_pick(view: &MeshHeaderView, blob: &[u8]) -> String {
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
        let (mut sy, mut n) = (0.0, 0.0f64);
        for t in first..first + count {
            let io = ibase + t * 12;
            for k in 0..3 {
                let v = vertex(blob, pbase, u32_le(blob, io + k * 4) as usize);
                sy += v[1];
                n += 1.0;
            }
        }
        if n == 0.0 {
            continue;
        }
        let y = sy / n;
        if best.is_none_or(|(_, by)| y < by) {
            best = Some((f, y));
        }
    }
    keys[best.expect("at least one face").0].clone()
}

/// The `TopoKey` of the TOP cap edge that runs along X — one edge, so
/// `mode:"edges"` has exactly one straight curve to answer with.
fn top_x_edge_pick(view: &MeshHeaderView, blob: &[u8]) -> String {
    assert!(view.has_edges(), "MESH1 must carry edges for an edge pick");
    let ranges = view.section(SEC_EDGE_RANGES).expect("EDGE_RANGES");
    let positions = view.section(SEC_EDGE_POSITIONS).expect("EDGE_POSITIONS");
    let keys = id_table(
        view,
        blob,
        SEC_EDGE_ID_OFFS,
        SEC_EDGE_ID_CHARS,
        view.edge_count as usize,
    );
    let (rbase, pbase) = (ranges.offset as usize, positions.offset as usize);
    let mut best: Option<(usize, f64)> = None;
    for edge in 0..view.edge_count as usize {
        let first = u32_le(blob, rbase + edge * 8) as usize;
        let count = u32_le(blob, rbase + edge * 8 + 4) as usize;
        if count == 0 {
            continue;
        }
        let (mut lo, mut hi, mut sz) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3], 0.0);
        for point in first..first + count {
            let v = vertex(blob, pbase, point);
            for axis in 0..3 {
                lo[axis] = lo[axis].min(v[axis]);
                hi[axis] = hi[axis].max(v[axis]);
            }
            sz += v[2];
        }
        let spans = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        // Along X, and at the top: `sz / count` is the mean Z.
        if spans[0] <= spans[1] || spans[0] <= spans[2] {
            continue;
        }
        let z = sz / count as f64;
        if best.is_none_or(|(_, bz)| z > bz) {
            best = Some((edge, z));
        }
    }
    keys[best.expect("a top X-parallel edge").0].clone()
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, onecad_core::regen::Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

/// The worker must actually carry the verb. A worker without it FAILS here — a
/// skipped WP-P gate would be a green lie.
fn require_verb(bin: &std::path::Path) {
    let out = std::process::Command::new("strings")
        .arg(bin)
        .output()
        .expect("run `strings` over the worker binary");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("ProjectToSketchPlane"),
        "the staged worker at {bin:?} does not carry the `ProjectToSketchPlane` verb — \
         rebuild the sidecar (scripts/build-worker.sh Release). REFUSING to skip."
    );
}

/// Sketch-plane UV of every point in a sketch, in entity order.
fn point_uvs(sketch: &Sketch) -> Vec<[f64; 2]> {
    sketch
        .entities()
        .iter()
        .filter_map(|e| match e {
            SketchEntity::Point { at, .. } => Some([at.x, at.y]),
            _ => None,
        })
        .collect()
}

fn max_u(sketch: &Sketch) -> f64 {
    point_uvs(sketch)
        .iter()
        .map(|p| p[0])
        .fold(f64::MIN, f64::max)
}

// ─────────────────────────────────────────────────────────────────────────────
// The gate
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projected_edges_bound_a_region_go_stale_update_and_detach() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    require_verb(&bin);
    let wm = spawn_worker(bin).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    // ── A 40 × 20 × 25 box ───────────────────────────────────────────────────
    let base = SketchId(Uuid::from_u128(SID_BASE));
    let proj = SketchId(Uuid::from_u128(SID_PROJ));
    build_box(&mut rt, base).await;
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "base box");
    let snapshot = SnapshotId(report.snapshot_id);
    let body = BodyId(Uuid::from_u128(EXTRUDE_BASE));

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    assert_eq!(view.face_count, 6, "the extruded rect is a 6-faced box");
    let top_key = top_face_pick(&view, &mesh);
    let side_key = side_face_pick(&view, &mesh);
    assert_ne!(top_key, side_key);

    // An EMPTY sketch on world XY. Deliberately not `add_sketch_on_face`: that
    // would seed the sketch with the host face's own boundary and the projection
    // under test would land on top of it.
    let mut target = Sketch::new(
        proj,
        "Projected",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    target.plane = onecad_core::sketch::SketchPlane::xy();
    rt.apply(EditCommand::AddSketch { sketch: target })
        .expect("AddSketch");

    let app = tauri::test::mock_app();
    *app_state.runtime.lock().await = Some(rt);
    app.manage(app_state);
    let handle = app.handle().clone();
    let state = || handle.state::<AppState>();
    // The runtime / projector handles are cloned ONCE: `state()` hands back a
    // temporary whose borrow cannot outlive the statement.
    let runtime = state().runtime.clone();

    // The PICK ANCHOR is load-bearing, exactly as it is in the real UI: without a
    // world point the §7.5 ladder cannot tell the top cap from the bottom one
    // after the box is edited (both are planar, same area, opposite normals) and
    // answers `NeedsRepair` — correctly. `update_projection` below depends on the
    // ladder re-binding this source confidently.
    let top_face = ElementQuery::query_element_by_topo_key(&wm, snapshot, body, &top_key)
        .await
        .expect("QueryElement(top face)")
        .expect("the picked cap resolves");
    let anchor = serde_json::json!({ "worldPoint": top_face.center });

    // ── Project the TOP face's whole boundary into it ────────────────────────
    let dto = onecad_lib::api::project_to_sketch(
        state(),
        snapshot.0,
        proj.to_string(),
        "faceOutline".into(),
        vec![serde_json::from_value(serde_json::json!({
            "bodyId": body_id_wire(body),
            "topoKey": top_key,
            "anchor": anchor,
        }))
        .unwrap()],
    )
    .await
    .expect("projectToSketch(faceOutline) on the box top face");

    assert!(
        dto.refusals.is_empty(),
        "a planar box cap parallel to the sketch plane must project cleanly, got {:?}",
        dto.refusals
    );
    assert_eq!(dto.entities.len(), 4, "a rectangle's outline is four lines");
    assert_eq!(
        dto.point_count, 4,
        "the merge collapses eight ends onto four corners"
    );
    assert!(
        dto.entities.iter().all(|e| e.entity_type == "Line"),
        "every box-cap boundary curve is a Line, got {:?}",
        dto.entities
    );
    assert!(
        dto.entities.iter().all(|e| e.projected_hash.len() == 16
            && e.projected_hash.chars().all(|c| c.is_ascii_hexdigit())),
        "projectedHash is 16 hex chars (SCHEMA §2), got {:?}",
        dto.entities
    );
    let projected_ids: Vec<EntityId> = dto
        .entities
        .iter()
        .map(|e| e.entity_id.parse().expect("entity id"))
        .collect();

    {
        let guard = runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        let sketch = rt.sketch_snapshot(proj, "t").unwrap();
        assert_eq!(sketch.projections.len(), 4, "one provenance row per curve");
        assert!(
            sketch
                .entities()
                .iter()
                .all(SketchEntity::is_reference_locked),
            "everything projected is locked"
        );
        assert_eq!(
            sketch
                .constraints()
                .iter()
                .filter(|c| matches!(c, Constraint::Fixed { .. }))
                .count(),
            4,
            "one Fixed per merged projected point"
        );
        // The outline of a 40 × 20 cap.
        let mut us: Vec<f64> = point_uvs(&sketch).iter().map(|p| p[0]).collect();
        us.sort_by(f64::total_cmp);
        assert!(
            (us[0] - 0.0).abs() < 1e-6 && (us[3] - 40.0).abs() < 1e-6,
            "u span 0..40, got {us:?}"
        );
    }

    // ── The projected region is extrudable, and its volume is analytic ───────
    {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let regions = rt.finish_sketch(proj).await.expect("finishSketch");
        assert_eq!(
            regions.regions.len(),
            1,
            "the projected outline closes into exactly one region"
        );
        add_op(rt, extrude_record(EXTRUDE_PROJ, proj, 5.0));
        let report = regen_all(rt).await;
        let _ = published(&report, "extrude off the projected region");
    }
    let derived = BodyId(Uuid::from_u128(EXTRUDE_PROJ));
    let props = ElementQuery::query_mass_properties(&wm, derived, body_id_wire(derived))
        .await
        .expect("QueryMassProperties");
    assert!(
        (props.volume - 40.0 * 20.0 * 5.0).abs() < 1e-6,
        "the projected 40 x 20 outline extruded 5 mm is 4000 mm³, got {}",
        props.volume
    );

    // ── Widen the box: the projection is FROZEN and must not follow ─────────
    let before: Vec<[f64; 2]> = {
        let guard = runtime.lock().await;
        point_uvs(&guard.as_ref().unwrap().sketch_snapshot(proj, "t").unwrap())
    };
    let head = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        rt.apply(EditCommand::SketchEdit {
            sketch: base,
            ops: vec![SketchEditOp::SetDimension {
                constraint: ConstraintId(Uuid::from_u128(WIDTH_DIM)),
                value: Scalar::new(44.0),
            }],
        })
        .expect("widen the base rect");
        rt.finish_sketch(base).await.expect("finishSketch(base)");
        let report = regen_all(rt).await;
        let _ = published(&report, "widened box");
        SnapshotId(report.snapshot_id)
    };
    assert_ne!(head, snapshot, "the head moved");

    {
        let guard = runtime.lock().await;
        let sketch = guard.as_ref().unwrap().sketch_snapshot(proj, "t").unwrap();
        assert_eq!(
            point_uvs(&sketch),
            before,
            "a projection is FROZEN: widening the source must not move the projected geometry"
        );
    }

    // ── The staleness probe raises PROJECTION_STALE on the sketch's step ─────
    let projector = state().face_projection();
    onecad_lib::sketch_projection::refresh_projection_staleness(
        &runtime,
        &projector,
        &CancelToken::new(),
    )
    .await;
    {
        let guard = runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        let stale = rt.projection_stale_entities(proj);
        // THREE, not four. Widening the box along +X leaves the `u = 0` wall's
        // projected line exactly where it was, and the hash covers the projected
        // UV geometry alone — so that one line is genuinely not stale. A
        // whole-sketch "something moved" flag would have said four.
        assert_eq!(
            stale.len(),
            3,
            "only the projected lines whose UV actually moved are stale (got {stale:?})"
        );
        assert!(
            stale.iter().all(|e| projected_ids.contains(e)),
            "the verdict names committed projected entities"
        );
        let codes: Vec<String> = rt
            .projection()
            .features
            .iter()
            .flat_map(|f| f.diagnostics.iter().map(|d| d.code.clone()))
            .collect();
        assert!(
            codes.iter().any(|c| c == PROJECTION_STALE_CODE),
            "the sketch step must carry a PROJECTION_STALE diagnostic, got {codes:?}"
        );
        // The diagnostic carries structured evidence: which sketch, which entities.
        let evidence = rt
            .projection()
            .features
            .iter()
            .flat_map(|f| f.diagnostics.iter())
            .find(|d| d.code == PROJECTION_STALE_CODE)
            .and_then(|d| d.evidence.clone())
            .expect("PROJECTION_STALE carries evidence");
        assert_eq!(evidence["sketchId"], serde_json::json!(proj.to_string()));
        assert_eq!(
            evidence["entityIds"].as_array().map(|a| a.len()),
            Some(3),
            "evidence names the three stale entities, got {evidence}"
        );
    }

    // ── update_projection moves the entities and clears the warning ──────────
    let updated = onecad_lib::api::update_projection(state(), proj.to_string())
        .await
        .expect("updateProjection");
    assert_eq!(
        updated.entities.len(),
        3,
        "the three moved lines were replaced, got {:?} / refusals {:?}",
        updated.entities,
        updated.refusals
    );
    assert!(
        updated.refusals.is_empty(),
        "the §7.5 ladder re-bound the source, so nothing is refused: {:?}",
        updated.refusals
    );
    assert!(
        updated
            .entities
            .iter()
            .map(|e| e.entity_id.parse::<EntityId>().unwrap())
            .all(|id| projected_ids.contains(&id)),
        "an update MOVES geometry; it must not re-mint entity identity"
    );
    {
        let guard = runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        let sketch = rt.sketch_snapshot(proj, "t").unwrap();
        assert!(
            (max_u(&sketch) - 44.0).abs() < 1e-6,
            "the re-projection follows the widened cap to u = 44, got {:?}",
            point_uvs(&sketch)
        );
        assert!(
            sketch
                .entities()
                .iter()
                .all(SketchEntity::is_reference_locked),
            "an update leaves the geometry locked"
        );
        assert!(
            rt.projection_stale_entities(proj).is_empty(),
            "updating clears the staleness verdict"
        );
    }

    // ── detach_projection frees the geometry ────────────────────────────────
    let detached = onecad_lib::api::detach_projection(state(), proj.to_string(), None)
        .await
        .expect("detachProjection");
    assert_eq!(detached.entity_ids.len(), 4);
    assert_eq!(detached.remaining, 0);
    assert_eq!(
        detached.released_constraints, 4,
        "every Fixed pin is released"
    );
    {
        let guard = runtime.lock().await;
        let sketch = guard.as_ref().unwrap().sketch_snapshot(proj, "t").unwrap();
        assert!(sketch.projections.is_empty(), "the provenance map is empty");
        assert!(
            !sketch
                .entities()
                .iter()
                .any(SketchEntity::is_reference_locked),
            "detached geometry is ordinary editable sketch geometry"
        );
        assert!(
            !sketch
                .constraints()
                .iter()
                .any(|c| matches!(c, Constraint::Fixed { .. })),
            "the pins are gone with the lock"
        );
    }
    // …and it is really editable now — the edit layer would refuse this while the
    // entity was still reference-locked.
    {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let sketch = rt.sketch_snapshot(proj, "t").unwrap();
        let point = sketch
            .entities()
            .iter()
            .find(|e| matches!(e, SketchEntity::Point { .. }))
            .unwrap()
            .id();
        rt.apply(EditCommand::SketchEdit {
            sketch: proj,
            ops: vec![SketchEditOp::SetEntityPositions {
                positions: vec![(point, Vec2::new_unchecked(3.0, 4.0))],
            }],
        })
        .expect("a detached point moves like any other");
    }
}

/// A sketch on a SIDE face sees the top cap edge-on: the two X-parallel edges
/// project onto ONE line and the two Y-parallel ones collapse. The verb answers
/// per source rather than failing the call — that is the batch rule.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_face_sketch_sees_the_top_cap_edge_on() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    require_verb(&bin);
    let wm = spawn_worker(bin).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let base = SketchId(Uuid::from_u128(SID_BASE));
    build_box(&mut rt, base).await;
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "base box");
    let snapshot = SnapshotId(report.snapshot_id);
    let body = BodyId(Uuid::from_u128(EXTRUDE_BASE));

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    let top_key = top_face_pick(&view, &mesh);
    let side_key = side_face_pick(&view, &mesh);

    let app = tauri::test::mock_app();
    *app_state.runtime.lock().await = Some(rt);
    app.manage(app_state);
    let handle = app.handle().clone();
    let state = || handle.state::<AppState>();

    // A real sketch ON the side face, so the plane is the kernel's, not the
    // world's — the projection must be expressed in THAT basis.
    let sketch_id = Uuid::from_u128(SID_PROJ).to_string();
    onecad_lib::api::add_sketch_on_face(
        state(),
        snapshot.0,
        body_id_wire(body),
        String::new(),
        Some(side_key.clone()),
        sketch_id.clone(),
        "Side".into(),
    )
    .await
    .expect("addSketchOnFace(side wall)");

    let dto = onecad_lib::api::project_to_sketch(
        state(),
        snapshot.0,
        sketch_id.clone(),
        "faceOutline".into(),
        vec![serde_json::from_value(serde_json::json!({
            "bodyId": body_id_wire(body),
            "topoKey": top_key,
        }))
        .unwrap()],
    )
    .await
    .expect("projectToSketch onto the side-face sketch");

    // The worker's `faceOutline` rules make this exact, not "whatever comes
    // back" (`EdgeProjector::projectFaceOutline` + `appendPrimitives`):
    //
    //  * the cap's two Y-parallel edges project to POINTS on this plane, and a
    //    degenerate BOUNDARY edge is SKIPPED rather than refused (half an outline
    //    does not close, so only a NAMED refusal fails the source);
    //  * the two X-parallel ones project onto the SAME segment, and `lineExists`
    //    suppresses the coincident duplicate within the source's own run.
    //
    // One Line over two merged points, and nothing refused.
    assert!(
        dto.refusals.is_empty(),
        "an edge-on planar cap is answered, not refused: {:?}",
        dto.refusals
    );
    assert!(
        !dto.entities.is_empty(),
        "the call must produce an ANSWER, not an empty response"
    );
    assert_eq!(
        dto.entities.len(),
        1,
        "the cap collapses to ONE chord line on this plane, got {:?}",
        dto.entities
    );
    assert_eq!(dto.point_count, 2, "…over its two merged endpoints");
    assert!(
        dto.entities.iter().all(|e| e.entity_type == "Line"),
        "a planar cap's straight edges stay Lines in any basis, got {:?}",
        dto.entities
    );
}

/// `mode:"edges"` on a SINGLE picked edge: one Line, two points, one provenance
/// row at ordinal 0. A one-row run is the shape the F1/F2 run guards can never
/// fire on, which is why it is pinned separately from the `faceOutline` cases.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_single_projected_edge_is_one_line_at_ordinal_zero() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    require_verb(&bin);
    let wm = spawn_worker(bin).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let base = SketchId(Uuid::from_u128(SID_BASE));
    let proj = SketchId(Uuid::from_u128(SID_PROJ));
    build_box(&mut rt, base).await;
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "base box");
    let snapshot = SnapshotId(report.snapshot_id);
    let body = BodyId(Uuid::from_u128(EXTRUDE_BASE));

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    let edge_key = top_x_edge_pick(&view, &mesh);

    let mut target = Sketch::new(
        proj,
        "Projected",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    target.plane = onecad_core::sketch::SketchPlane::xy();
    rt.apply(EditCommand::AddSketch { sketch: target })
        .expect("AddSketch");

    let app = tauri::test::mock_app();
    *app_state.runtime.lock().await = Some(rt);
    app.manage(app_state);
    let handle = app.handle().clone();
    let state = || handle.state::<AppState>();
    let runtime = state().runtime.clone();

    let source = || {
        vec![serde_json::from_value(serde_json::json!({
            "bodyId": body_id_wire(body),
            "topoKey": edge_key,
        }))
        .unwrap()]
    };
    let dto = onecad_lib::api::project_to_sketch(
        state(),
        snapshot.0,
        proj.to_string(),
        "edges".into(),
        source(),
    )
    .await
    .expect("projectToSketch(edges) on one box edge");

    assert!(dto.refusals.is_empty(), "{:?}", dto.refusals);
    assert_eq!(dto.entities.len(), 1, "one edge is one curve");
    assert_eq!(dto.entities[0].entity_type, "Line");
    assert_eq!(dto.point_count, 2, "a segment has two ends");
    {
        let guard = runtime.lock().await;
        let sketch = guard.as_ref().unwrap().sketch_snapshot(proj, "t").unwrap();
        assert_eq!(sketch.projections.len(), 1);
        let row = sketch.projections.values().next().unwrap();
        assert_eq!(
            row.source_ordinal, 0,
            "the only entity of a one-edge run sits at ordinal 0"
        );
        assert_eq!(
            row.source_kind,
            onecad_core::document::refs::ElementKind::Edge,
            "the KIND is persisted so an update re-requests it in `edges` mode"
        );
    }

    // WP-P F15: the same source again is REFUSED, not projected a second time —
    // two provenance runs for one element could never be told apart afterwards.
    let again = onecad_lib::api::project_to_sketch(
        state(),
        snapshot.0,
        proj.to_string(),
        "edges".into(),
        source(),
    )
    .await
    .expect("the second call ANSWERS; it does not error");
    assert!(again.entities.is_empty(), "{:?}", again.entities);
    assert_eq!(again.refusals.len(), 1);
    assert_eq!(again.refusals[0].code, "alreadyProjected");
    {
        let guard = runtime.lock().await;
        let sketch = guard.as_ref().unwrap().sketch_snapshot(proj, "t").unwrap();
        assert_eq!(
            sketch.projections.len(),
            1,
            "the refused re-projection committed NOTHING"
        );
        assert_eq!(sketch.entities().len(), 3, "…not one extra point or line");
    }
}

/// Two loud refusals: an open drag gesture, and a head that moved between the
/// pick and the commit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projection_is_refused_mid_gesture_and_against_a_moved_head() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    require_verb(&bin);
    let wm = spawn_worker(bin).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let base = SketchId(Uuid::from_u128(SID_BASE));
    let proj = SketchId(Uuid::from_u128(SID_PROJ));
    build_box(&mut rt, base).await;
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "base box");
    let stale_snapshot = SnapshotId(report.snapshot_id);
    let body = BodyId(Uuid::from_u128(EXTRUDE_BASE));

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    let top_key = top_face_pick(&view, &mesh);

    // A sketch holding one free point, so a gesture has something to grab.
    let mut target = Sketch::new(
        proj,
        "Projected",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    let grabbed = EntityId(Uuid::from_u128(0xC0FFEE));
    target
        .add_entity(SketchEntity::point(
            grabbed,
            Vec2::new_unchecked(1.0, 1.0),
            false,
            false,
        ))
        .unwrap();
    rt.apply(EditCommand::AddSketch { sketch: target })
        .expect("AddSketch");
    rt.enter_sketch(proj).await.expect("enterSketch");
    rt.begin_gesture(
        proj,
        grabbed,
        onecad_lib::worker::wire::GestureTarget::point(grabbed),
    )
    .await
    .expect("beginGesture");

    let app = tauri::test::mock_app();
    *app_state.runtime.lock().await = Some(rt);
    app.manage(app_state);
    let handle = app.handle().clone();
    let state = || handle.state::<AppState>();
    // The runtime / projector handles are cloned ONCE: `state()` hands back a
    // temporary whose borrow cannot outlive the statement.
    let runtime = state().runtime.clone();

    let source = || {
        vec![serde_json::from_value(serde_json::json!({
            "bodyId": body_id_wire(body),
            "topoKey": top_key,
        }))
        .unwrap()]
    };

    // ── 1. refused while a gesture is open ──────────────────────────────────
    let err = onecad_lib::api::project_to_sketch(
        state(),
        stale_snapshot.0,
        proj.to_string(),
        "faceOutline".into(),
        source(),
    )
    .await
    .expect_err("a projection must not land under an open drag gesture");
    assert!(
        format!("{err}").contains("drag gesture"),
        "the refusal must name the gesture, got {err}"
    );

    // ── 2. refused against a moved head ─────────────────────────────────────
    {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        rt.cancel_sketch(proj).await.expect("cancelSketch");
        rt.apply(EditCommand::SketchEdit {
            sketch: base,
            ops: vec![SketchEditOp::SetDimension {
                constraint: ConstraintId(Uuid::from_u128(WIDTH_DIM)),
                value: Scalar::new(55.0),
            }],
        })
        .expect("widen the base rect");
        rt.finish_sketch(base).await.expect("finishSketch(base)");
        let report = regen_all(rt).await;
        let _ = published(&report, "moved head");
        assert_ne!(SnapshotId(report.snapshot_id), stale_snapshot);
    }
    let err = onecad_lib::api::project_to_sketch(
        state(),
        stale_snapshot.0, // the OLD head
        proj.to_string(),
        "faceOutline".into(),
        source(),
    )
    .await
    .expect_err("a pick taken against a superseded head must be refused, not committed");
    assert!(
        format!("{err}").contains("re-pick") || format!("{err}").contains("stale"),
        "the refusal must say the pick is stale, got {err}"
    );
    {
        let guard = runtime.lock().await;
        let sketch = guard.as_ref().unwrap().sketch_snapshot(proj, "t").unwrap();
        assert!(
            sketch.projections.is_empty(),
            "a refused projection commits NOTHING"
        );
        assert_eq!(
            sketch.entities().len(),
            1,
            "…not even the points, got {:?}",
            sketch.entities()
        );
    }
}

/// **The blocker case (WP-P F1).** A projected face outline whose boundary SPLITS
/// must never be re-associated positionally.
///
/// Four rows are committed from the box's top cap. A through-slot then notches
/// one wall, so the cap's `y = 0` edge becomes two segments and the notch adds
/// three more: the run the ordinals were frozen against is gone, and ordinal 1 of
/// the new run is a DIFFERENT physical edge from ordinal 1 of the old one.
///
/// Positional matching would have rewritten each committed line from whatever now
/// occupies its ordinal — a silent wrong bind with no refusal to explain it, the
/// H5-B class this whole migration exists to remove. Instead the SOURCE is refused
/// once, nothing is applied, and every row stays stale with its old geometry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_split_face_boundary_refuses_the_source_instead_of_re_associating() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    require_verb(&bin);
    let wm = spawn_worker(bin).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let base = SketchId(Uuid::from_u128(SID_BASE));
    let proj = SketchId(Uuid::from_u128(SID_PROJ));
    let notch = SketchId(Uuid::from_u128(SID_NOTCH));
    build_box(&mut rt, base).await;
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "base box");
    let snapshot = SnapshotId(report.snapshot_id);
    let body = BodyId(Uuid::from_u128(EXTRUDE_BASE));

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    let top_key = top_face_pick(&view, &mesh);

    let mut target = Sketch::new(
        proj,
        "Projected",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    target.plane = onecad_core::sketch::SketchPlane::xy();
    rt.apply(EditCommand::AddSketch { sketch: target })
        .expect("AddSketch");

    let app = tauri::test::mock_app();
    *app_state.runtime.lock().await = Some(rt);
    app.manage(app_state);
    let handle = app.handle().clone();
    let state = || handle.state::<AppState>();
    let runtime = state().runtime.clone();

    // The anchor is load-bearing: without a world point the ladder cannot tell the
    // top cap from the bottom one after the cut, and the source would come back
    // `absent` — a different (also correct) refusal that would not exercise F1.
    let top_face = ElementQuery::query_element_by_topo_key(&wm, snapshot, body, &top_key)
        .await
        .expect("QueryElement(top face)")
        .expect("the picked cap resolves");
    let anchor = serde_json::json!({ "worldPoint": top_face.center });

    let dto = onecad_lib::api::project_to_sketch(
        state(),
        snapshot.0,
        proj.to_string(),
        "faceOutline".into(),
        vec![serde_json::from_value(serde_json::json!({
            "bodyId": body_id_wire(body),
            "topoKey": top_key,
            "anchor": anchor,
        }))
        .unwrap()],
    )
    .await
    .expect("projectToSketch(faceOutline) on the box top face");
    assert_eq!(dto.entities.len(), 4, "a rectangle's outline is four lines");
    assert!(dto.refusals.is_empty(), "{:?}", dto.refusals);
    let projected_ids: Vec<EntityId> = dto
        .entities
        .iter()
        .map(|e| e.entity_id.parse().expect("entity id"))
        .collect();

    // The projected sketch needs its own timeline record for the warning to have a
    // step to land on.
    let before: Vec<[f64; 2]> = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        rt.finish_sketch(proj).await.expect("finishSketch(proj)");
        point_uvs(&rt.sketch_snapshot(proj, "t").unwrap())
    };

    // ── Notch ONE wall with a through-slot: the cap's y = 0 edge splits ──────
    {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        rt.apply(EditCommand::AddSketch {
            sketch: pinned_rect(notch, 0x3000, 10.0, -5.0, 20.0, 5.0),
        })
        .expect("AddSketch(notch)");
        rt.finish_sketch(notch).await.expect("finishSketch(notch)");
        add_op(rt, cut_record(EXTRUDE_CUT, notch, 25.0, body));
        let report = regen_all(rt).await;
        let _ = published(&report, "notched box");
    }

    // ── Every row is stale, matching hashes included ─────────────────────────
    let projector = state().face_projection();
    onecad_lib::sketch_projection::refresh_projection_staleness(
        &runtime,
        &projector,
        &CancelToken::new(),
    )
    .await;
    {
        let guard = runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        let stale = rt.projection_stale_entities(proj);
        assert_eq!(
            stale.len(),
            4,
            "a re-numbered run is stale WHOLESALE — not just the rows whose hash \
             happened to move (got {stale:?})"
        );
        assert!(stale.iter().all(|e| projected_ids.contains(e)));
    }

    // ── update_projection refuses the SOURCE and applies nothing ─────────────
    let updated = onecad_lib::api::update_projection(state(), proj.to_string())
        .await
        .expect("updateProjection answers; it does not error");
    assert!(
        updated.entities.is_empty(),
        "not one row may be rewritten from a re-numbered run, got {:?}",
        updated.entities
    );
    assert_eq!(
        updated.refusals.len(),
        1,
        "ONE refusal for the source, not one per row: {:?}",
        updated.refusals
    );
    assert_eq!(updated.refusals[0].code, "topologyChanged");
    assert!(
        updated.refusals[0]
            .message
            .contains("4 edges were projected"),
        "the refusal names the source and the counts, got {:?}",
        updated.refusals[0]
    );

    {
        let guard = runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        let sketch = rt.sketch_snapshot(proj, "t").unwrap();
        assert_eq!(
            point_uvs(&sketch),
            before,
            "a refused update leaves the OLD geometry exactly where it was"
        );
        assert_eq!(sketch.projections.len(), 4, "…and every provenance row");
        assert!(
            sketch
                .entities()
                .iter()
                .all(SketchEntity::is_reference_locked),
            "…still reference-locked"
        );
        assert_eq!(
            rt.projection_stale_entities(proj).len(),
            4,
            "a refused update does NOT clear the warning — nothing was fixed"
        );
        assert!(
            rt.projection()
                .features
                .iter()
                .flat_map(|f| f.diagnostics.iter())
                .any(|d| d.code == PROJECTION_STALE_CODE),
            "the PROJECTION_STALE diagnostic is still on the sketch's step"
        );
    }

    // ── detach_projection with an EXPLICIT subset ────────────────────────────
    let keep: Vec<EntityId> = projected_ids[2..].to_vec();
    let dropped: Vec<String> = projected_ids[..2].iter().map(ToString::to_string).collect();
    let detached =
        onecad_lib::api::detach_projection(state(), proj.to_string(), Some(dropped.clone()))
            .await
            .expect("detachProjection(subset)");
    assert_eq!(detached.entity_ids, dropped, "only the named ids");
    assert_eq!(detached.remaining, 2, "the other two stay projected");
    {
        let guard = runtime.lock().await;
        let sketch = guard.as_ref().unwrap().sketch_snapshot(proj, "t").unwrap();
        assert_eq!(sketch.projections.len(), 2);
        for id in &projected_ids[..2] {
            assert!(!sketch.projections.contains_key(id));
            assert!(
                !sketch.get_entity(*id).unwrap().is_reference_locked(),
                "a detached line is ordinary editable geometry"
            );
        }
        for id in &keep {
            assert!(sketch.projections.contains_key(id));
            assert!(
                sketch.get_entity(*id).unwrap().is_reference_locked(),
                "the rows that were NOT named keep their lock"
            );
        }
    }
}
