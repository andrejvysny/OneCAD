//! STEP-IMPORT WP-A W5 — the DOWNSTREAM-OPS acceptance gate.
//!
//! `step_import.rs` (W3) proves the import lane itself: ordinal children, exact
//! volumes, replay stability, save/reopen, suppression, the never-hashed path.
//! This file proves the thing that actually decides whether STEP import shipped:
//! **an imported body is a first-class citizen of the parametric system.** Every
//! downstream op works on it, and its identity survives process death.
//!
//! The fixture is self-produced (same discipline as W3): the test builds three
//! disjoint boxes of known exact volume, exports them through the worker's own
//! `ExportStep`, and imports that file back — so every number here is box
//! arithmetic, never a checked-in binary.
//!
//! ## Ordinal-agnostic by construction
//!
//! The import's ordinal order is the WORKER's, not the source file's (W3 says so
//! and refuses to over-specify it). So this gate never assumes `:k` is a
//! particular box. Instead every downstream expectation is a RATIO of the child's
//! own measured volume, arranged so the arithmetic is identical whichever box
//! landed at that ordinal:
//!
//! * every box is `footprint × 10`, so a **fillet** of radius 2 on a vertical edge
//!   removes `10·r²(1−π/4)` — a function of the edge length alone;
//! * the boolean tool is a **slab** covering every box's footprint over `z∈[0,5]`,
//!   so `Cut` removes exactly half of whichever box it targets;
//! * the face-hosted riser is `footprint × 5`, i.e. exactly half the host again.
//!
//! ## Phases (one document, two worker processes — m2_gate's style)
//!
//! 1. import a 3-solid STEP ⇒ 3 ordinal children, exact volumes, MESH1 green;
//! 2. promote an edge of child `:1` ⇒ **fillet** it, analytic volume drop, 0 repairs;
//! 3. **boolean** a modelled slab into child `:0`, §7.2 lineage (target modified,
//!    tool consumed, siblings untouched);
//! 4. **sketch on an imported planar face** of `:2` ⇒ projected locked boundary ⇒
//!    host-boolean `Add` MODIFIES the host, mints no body;
//! 5. MESH1 for every body at head;
//! 6. **process death**: save ⇒ FRESH runtime on a FRESH worker ⇒ identical
//!    `historyPrefixHash`, identical volumes, identical persisted `ElementId`s, the
//!    fillet still binds with zero `NeedsRepair`, and the worker's ladder reaches
//!    the IDENTICAL verdict on the same id in the new process;
//! 7. **repair honesty**: delete the import ⇒ every downstream record goes to a
//!    LOUD documented failure state (never a silent wrong bind) ⇒ undo restores the
//!    hash chain exactly.
//!
//! Deliberately NOT asserted here: the import record's codec/format fields, blob
//! section names and blob file extensions. Those are the conversion lane's private
//! business (W4.5 is moving them); this gate is about geometry, identity and hashes.
//!
//! Gated on `ONECAD_WORKER_PATH` (else dev-tree fallback); `ONECAD_REQUIRE_WORKER=1`
//! hard-fails a missing binary (CI). A missing worker is a quiet local-dev skip.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use tauri::Manager;

use onecad_core::document::record::{
    BooleanMode, BooleanOp, BooleanParams, ExtrudeMode, ExtrudeParams, FilletParams,
    KnownOperation, Operation, OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::history::StepState;
use onecad_core::ids::{
    BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest, ResolveOutcome,
    ResolveRef, ResolveRequest, StoppedReason,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_core::io::container::SaveMeta;
use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::export::GeometryExporter;
use onecad_lib::imports::prepare_import;
use onecad_lib::state::AppState;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::{body_id_wire, sketch_wire};
use onecad_lib::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors step_import.rs / sketch_on_face.rs — each worker-backed test
// file keeps its own small harness rather than sharing a `tests/common` module)
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

/// An `AppState` wired to the SAME `WorkerManager` for every facet — what
/// `state::real_worker_factory` does in production, minus the restart-hook wiring
/// this gate never exercises. Needed because `add_sketch_on_face` is only
/// reachable through a real `tauri::State<AppState>`.
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
        app_version: "step-import-w5".into(),
        occt_fingerprint: Some("occt-7.9.3".into()),
        created: "2026-08-02T00:00:00Z".into(),
        modified: "2026-08-02T00:00:00Z".into(),
    }
}

fn add_op(rt: &mut DocumentRuntime, record: OperationRecord) {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("AddOperation");
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 readers (exact volume for planar-faced solids; pick helpers)
// ─────────────────────────────────────────────────────────────────────────────

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

/// Signed volume via the divergence theorem — EXACT for a closed, planar-faced
/// polyhedron, and accurate to the tessellation deflection for a filleted one.
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

/// The face with the greatest average world-Z (the box's top cap): its `TopoKey`.
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
        let (mut sz, mut n) = (0.0f64, 0.0f64);
        for t in first..first + count {
            let io = ibase + t * 12;
            for k in 0..3 {
                sz += vertex(blob, pbase, u32_le(blob, io + k * 4) as usize)[2];
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

/// The edge with the greatest world-Z extent (a vertical box edge, always safely
/// fillettable) → `(topoKey, centroid anchor)`. Same routine as `topology_rebind`.
fn vertical_edge_pick(view: &MeshHeaderView, blob: &[u8]) -> (String, Vec3) {
    assert!(
        view.has_edges(),
        "MESH1 must carry edges for the fillet pick"
    );
    let er = view.section(SEC_EDGE_RANGES).expect("EDGE_RANGES");
    let ep = view.section(SEC_EDGE_POSITIONS).expect("EDGE_POSITIONS");
    let keys = id_table(
        view,
        blob,
        SEC_EDGE_ID_OFFS,
        SEC_EDGE_ID_CHARS,
        view.edge_count as usize,
    );
    let (erbase, epbase) = (er.offset as usize, ep.offset as usize);

    let mut best: Option<(usize, f64, Vec3)> = None;
    for i in 0..view.edge_count as usize {
        let first = u32_le(blob, erbase + i * 8) as usize;
        let count = u32_le(blob, erbase + i * 8 + 4) as usize;
        if count == 0 {
            continue;
        }
        let (mut zmin, mut zmax) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut sx, mut sy, mut sz) = (0.0f64, 0.0f64, 0.0f64);
        for p in 0..count {
            let o = epbase + (first + p) * 12;
            let (x, y, z) = (
                f32_le(blob, o) as f64,
                f32_le(blob, o + 4) as f64,
                f32_le(blob, o + 8) as f64,
            );
            zmin = zmin.min(z);
            zmax = zmax.max(z);
            sx += x;
            sy += y;
            sz += z;
        }
        let span = zmax - zmin;
        let centroid = Vec3::new_unchecked(sx / count as f64, sy / count as f64, sz / count as f64);
        if best.is_none_or(|(_, s, _)| span > s) {
            best = Some((i, span, centroid));
        }
    }
    let (idx, _span, centroid) = best.expect("at least one edge");
    (keys[idx].clone(), centroid)
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .unwrap_or_else(|| panic!("get_mesh must serve body {}", body_id_wire(body)))
}

async fn body_volume(rt: &mut DocumentRuntime, body: BodyId) -> f64 {
    let mesh = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("valid MESH1 blob");
    mesh_volume(&view, &mesh)
}

/// `wire id → volume` for every body at head, MESH1-validated on the way through.
async fn head_volumes(rt: &mut DocumentRuntime) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for body in rt.head_body_ids() {
        out.insert(body_id_wire(body), body_volume(rt, body).await);
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixture geometry
//
// Three DISJOINT boxes, all 10 tall, laid out along world Y. The non-standard XY
// basis below maps sketch `(u,v)` to world `(-v, u)`, so a `rect(u0,v0,w,h)` spans
// world Y over `w` and world X over `h`.
//
//   box0  rect(  0,0,10,10) → worldY[  0, 10]  10×10×10 = 1000
//   box1  rect(100,0,20,10) → worldY[100,120]  20×10×10 = 2000
//   box2  rect(200,0,30,10) → worldY[200,230]  30×10×10 = 3000
// ─────────────────────────────────────────────────────────────────────────────

/// Every fixture box is this tall — the single fact the fillet arithmetic needs.
const BOX_H: f64 = 10.0;
const VOL_0: f64 = 1000.0;
const VOL_1: f64 = 2000.0;
const VOL_2: f64 = 3000.0;

/// Fillet radius applied to one vertical edge of an imported child.
const FILLET_R: f64 = 2.0;

/// The slab tool spans every box's footprint over `z∈[0, SLAB_H]`, so a `Cut`
/// against ANY of them removes exactly `SLAB_H/BOX_H` of that box.
const SLAB_H: f64 = 5.0;
/// The face-hosted riser: `footprint × RISER` = exactly half the host again.
const RISER: f64 = 5.0;

// Fixture-document record ids (the exporter's own throwaway document).
const FX_SKETCH_0: u128 = 0x0100;
const FX_EXTRUDE_0: u128 = 0x0101;
const FX_SKETCH_1: u128 = 0x0110;
const FX_EXTRUDE_1: u128 = 0x0111;
const FX_SKETCH_2: u128 = 0x0120;
const FX_EXTRUDE_2: u128 = 0x0121;

// Downstream record ids authored ON TOP of the import.
const FILLET_REC: u128 = 0x0200;
const SLAB_SKETCH_REC: u128 = 0x0210;
const SLAB_EXTRUDE_REC: u128 = 0x0211;
const BOOLEAN_REC: u128 = 0x0212;
const FACE_EXTRUDE_REC: u128 = 0x0220;
const ON_FACE_SKETCH: u128 = 0x02FA;

/// Same non-standard XY basis as `wire_contract` / `sketch_on_face` — deliberately
/// non-identity so a frame silently re-derived anywhere shows up as a DIFFERENT
/// plane rather than a coincidental match.
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

/// A fully-constrained rectangle (identical to `step_import::rect_sketch`).
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
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(x0, y0),
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

fn extrude_params(sketch: SketchId, region: &str, dist: f64) -> ExtrudeParams {
    ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(region),
            region_identity_version: (!region.is_empty()).then_some(3),
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
    }
}

fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(extrude_params(sketch, "", dist))),
    )
}

/// A feature-fused extrude of an EXACT region bound to an existing body — the op a
/// face-hosted sketch's push/pull authors (`sketch_on_face::host_extrude_record`).
fn host_extrude_record(
    rec: u128,
    sketch: SketchId,
    region: &str,
    dist: f64,
    mode: BooleanMode,
    target: BodyId,
) -> OperationRecord {
    let mut params = extrude_params(sketch, region, dist);
    params.boolean_mode = mode;
    params.target_body = Some(target);
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

fn boolean_record(rec: u128, op: BooleanOp, target: BodyId, tool: BodyId) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Boolean",
        Operation::Known(KnownOperation::Boolean(BooleanParams {
            operation: op,
            target_body: target,
            tool_body: tool,
            extra: Default::default(),
        })),
    )
}

/// The typed `ElementRef` (Rust-minted id + world-midpoint anchor) a fillet's
/// SCHEMA §7.3 `inputs[]` carries.
fn edge_ref(body: BodyId, edge_el: &ElementId, anchor: Vec3) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: edge_el.clone(),
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: anchor,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    }
}

fn fillet_record(rec: u128, body: BodyId, edge_el: &ElementId, anchor: Vec3) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(FILLET_R),
            edge_ids: vec![edge_el.clone()],
            edges: vec![edge_ref(body, edge_el, anchor)],
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    )
}

/// Builds the three-box document on its OWN worker, exports it to `path`, asserts
/// the source volumes the import must reproduce, then tears that worker down.
async fn export_fixture(bin: &std::path::Path, path: &std::path::Path) {
    let wm = spawn_worker(bin.to_path_buf()).await;
    let mut rt = runtime_over(&wm);

    let s0 = SketchId(Uuid::from_u128(0x10));
    let s1 = SketchId(Uuid::from_u128(0x11));
    let s2 = SketchId(Uuid::from_u128(0x12));
    add_op(
        &mut rt,
        sketch_record(FX_SKETCH_0, &rect_sketch(s0, 0x1000, 0.0, 0.0, 10.0, 10.0)),
    );
    add_op(&mut rt, extrude_record(FX_EXTRUDE_0, s0, BOX_H));
    add_op(
        &mut rt,
        sketch_record(
            FX_SKETCH_1,
            &rect_sketch(s1, 0x2000, 100.0, 0.0, 20.0, 10.0),
        ),
    );
    add_op(&mut rt, extrude_record(FX_EXTRUDE_1, s1, BOX_H));
    add_op(
        &mut rt,
        sketch_record(
            FX_SKETCH_2,
            &rect_sketch(s2, 0x3000, 200.0, 0.0, 30.0, 10.0),
        ),
    );
    add_op(&mut rt, extrude_record(FX_EXTRUDE_2, s2, BOX_H));

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "fixture build");
    assert_eq!(snap.bodies.len(), 3, "fixture is three disjoint bodies");

    let bodies = [
        BodyId(Uuid::from_u128(FX_EXTRUDE_0)),
        BodyId(Uuid::from_u128(FX_EXTRUDE_1)),
        BodyId(Uuid::from_u128(FX_EXTRUDE_2)),
    ];
    for (body, want) in bodies.iter().zip([VOL_0, VOL_1, VOL_2]) {
        let got = body_volume(&mut rt, *body).await;
        assert!(
            (got - want).abs() < 1.0,
            "source box volume {want}, got {got}"
        );
    }

    let written = wm
        .export_step(&path.to_string_lossy(), &bodies, "AP214IS")
        .await
        .expect("ExportStep the fixture");
    assert!(written > 0, "ExportStep wrote bytes");
    wm.shutdown().await;
}

/// The record id of the (single) `ImportStep` feature in the tree.
fn import_record_id(rt: &DocumentRuntime) -> Uuid {
    let import = rt
        .projection()
        .features
        .into_iter()
        .find(|f| f.op_type == "ImportStep")
        .expect("the tree carries the ImportStep feature");
    Uuid::parse_str(&import.id).expect("feature id is the record uuid")
}

/// `wire id → BodyId` at head, so a phase can address `body_<opId>:k` by ordinal
/// without assuming WHICH source box landed there.
fn head_by_wire(rt: &DocumentRuntime) -> BTreeMap<String, BodyId> {
    rt.head_body_ids()
        .into_iter()
        .map(|b| (body_id_wire(b), b))
        .collect()
}

fn child(map: &BTreeMap<String, BodyId>, record: Uuid, k: usize) -> BodyId {
    let wire = format!("body_{record}:{k}");
    *map.get(&wire).unwrap_or_else(|| {
        panic!(
            "head must carry the imported child {wire}, got {:?}",
            map.keys()
        )
    })
}

/// The `StepState` of the record with this id in a published snapshot.
fn state_of(rt: &DocumentRuntime, snap: &ModelSnapshot, record: RecordId) -> Option<StepState> {
    let index = rt
        .projection()
        .features
        .iter()
        .position(|f| f.id == record.0.to_string())?;
    snap.step_states
        .iter()
        .find(|(i, _)| *i == index)
        .map(|(_, s)| s.clone())
}

// ─────────────────────────────────────────────────────────────────────────────
// THE GATE
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn imported_bodies_are_first_class_downstream_citizens() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let step = tmp.path().join("three_boxes.step");
    export_fixture(&bin, &step).await;

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 1 — import ⇒ three ordinal children with exact volumes
    // ═════════════════════════════════════════════════════════════════════════
    let wm = spawn_worker(bin.clone()).await;
    let app_state = app_state_over(&wm);
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let prepared = prepare_import(&wm, &step).await.expect("prepare_import");
    assert_eq!(prepared.solid_count, 3, "the STEP carries three solids");
    rt.add_import_record(&prepared, false)
        .expect("author the ImportStep record");

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "PHASE 1 import regen");
    assert_eq!(snap.bodies.len(), 3, "three solids ⇒ three bodies");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "PHASE 1: a plain import needs no repair"
    );
    let import_rec = import_record_id(&rt);
    let snap1 = SnapshotId(report.snapshot_id);

    let by_wire = head_by_wire(&rt);
    let mut ids: Vec<&String> = by_wire.keys().collect();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            &format!("body_{import_rec}:0"),
            &format!("body_{import_rec}:1"),
            &format!("body_{import_rec}:2"),
        ],
        "N>1 mints ordinal children under the §7.3/§2 rule (never a plain body_<opId>)"
    );
    let (c0, c1, c2) = (
        child(&by_wire, import_rec, 0),
        child(&by_wire, import_rec, 1),
        child(&by_wire, import_rec, 2),
    );

    // PHASE 5 (first half): every imported body serves a VALID MESH1 blob.
    let imported = head_volumes(&mut rt).await;
    let mut sorted: Vec<f64> = imported.values().copied().collect();
    sorted.sort_by(f64::total_cmp);
    for (got, want) in sorted.iter().zip([VOL_0, VOL_1, VOL_2]) {
        assert!(
            (got - want).abs() < 1.0,
            "imported volumes must equal the source boxes ({VOL_0}, {VOL_1}, {VOL_2}), got {sorted:?}"
        );
    }
    let (v0, v1, v2) = (
        imported[&body_id_wire(c0)],
        imported[&body_id_wire(c1)],
        imported[&body_id_wire(c2)],
    );
    eprintln!("PHASE 1 PASS: 3 ordinal children of body_{import_rec}, volumes {sorted:?} (:0={v0:.1}, :1={v1:.1}, :2={v2:.1})");

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 2 — promote an edge of the IMPORTED child `:1` and fillet it
    // ═════════════════════════════════════════════════════════════════════════
    let mesh = body_mesh(&mut rt, c1).await;
    let view = validate_mesh_blob(&mesh).expect("imported child MESH1");
    assert_eq!(view.face_count, 6, "an imported box is a 6-faced solid");
    let (edge_key, edge_anchor) = vertical_edge_pick(&view, &mesh);

    let anchor_intent = AnchorIntent {
        world_point: edge_anchor,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    };
    let promoted = rt
        .promote_selection(
            snap1,
            c1,
            vec![(TopoKey::new(&edge_key), Some(anchor_intent))],
        )
        .await
        .expect("promote an edge of an imported body");
    let edge_el = ElementId::new(promoted[0].element_id.clone());
    assert!(
        edge_el.as_str().starts_with("el_"),
        "the promotion mints a Rust-owned ElementId, got {}",
        edge_el.as_str()
    );
    assert_eq!(
        promoted[0].body_id,
        body_id_wire(c1),
        "the promotion is scoped to the imported child it was picked on"
    );

    add_op(
        &mut rt,
        fillet_record(FILLET_REC, c1, &edge_el, edge_anchor),
    );
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "PHASE 2 fillet on an imported body");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "PHASE 2: the fillet binds an imported body's edge with NO NeedsRepair"
    );
    assert!(
        report.needs_repair.is_empty(),
        "PHASE 2: the needs-repair event set is empty, got {:?}",
        report.needs_repair
    );
    assert!(
        report.failed_steps.is_empty(),
        "PHASE 2: no step failed, got {:?}",
        report.failed_steps
    );

    let fmesh = body_mesh(&mut rt, c1).await;
    let fview = validate_mesh_blob(&fmesh).expect("filleted MESH1");
    assert!(
        fview.face_count >= 7,
        "PHASE 2: filleting one edge adds a rolled face (6 → ≥7), got {}",
        fview.face_count
    );
    let filleted_vol = mesh_volume(&fview, &fmesh);
    // The wedge between a sharp corner and its inscribed quarter-cylinder:
    // r²(1 − π/4) per unit length, over the full BOX_H-long vertical edge.
    let analytic_removed = FILLET_R * FILLET_R * (1.0 - std::f64::consts::FRAC_PI_4) * BOX_H;
    let removed = v1 - filleted_vol;
    // The band is ONE-SIDED by construction: MESH1 approximates the rolled
    // quarter-cylinder with INSCRIBED chords, so the meshed solid is always a hair
    // SMALLER than the exact one and the measured removal always sits at or just
    // ABOVE the analytic wedge — never below it. The observed excess is exactly the
    // 4-segment-per-quarter chordal deficit `2n(θ−sin θ)·BOX_H` (n=4, θ=π/8 ⇒ 0.801),
    // i.e. removed = 9.3853 against analytic 8.5841 (run 2026-08-02, OCCT 7.9.3,
    // Lod::Coarse). The 20 % ceiling leaves room for a tessellation-policy change
    // while still rejecting every wrong bind: r=1 removes 2.15, r=3 removes 19.3,
    // and an equal-leg chamfer of 2 removes 20.0.
    assert!(
        removed >= analytic_removed - 1e-6 && removed < analytic_removed * 1.20,
        "PHASE 2: the fillet must remove r²(1−π/4)·{BOX_H} = {analytic_removed:.4} mm³ \
         (+chordal deficit), got {removed:.4} (before {v1:.3}, after {filleted_vol:.3})"
    );
    let vol_c1 = filleted_vol;
    eprintln!(
        "PHASE 2 PASS: fillet r={FILLET_R} on imported edge {edge_key} ({}) — faces 6→{}, removed {removed:.4} ≈ {analytic_removed:.4}, 0 needsRepair",
        edge_el.as_str(),
        fview.face_count
    );

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 3 — boolean a MODELLED slab into the imported child `:0`
    //
    // The slab covers every box footprint over z∈[0,SLAB_H], so `Cut` removes
    // exactly half of whichever box `:0` turned out to be — and the two SIBLING
    // imported bodies, which the slab also passes through, must be UNTOUCHED
    // (a boolean is scoped to its target, §7.2 lineage).
    // ═════════════════════════════════════════════════════════════════════════
    let slab_sid = SketchId(Uuid::from_u128(0x21));
    add_op(
        &mut rt,
        sketch_record(
            SLAB_SKETCH_REC,
            &rect_sketch(slab_sid, 0x4000, -10.0, 0.0, 250.0, 10.0),
        ),
    );
    add_op(&mut rt, extrude_record(SLAB_EXTRUDE_REC, slab_sid, SLAB_H));
    let slab = BodyId(Uuid::from_u128(SLAB_EXTRUDE_REC));
    add_op(
        &mut rt,
        boolean_record(BOOLEAN_REC, BooleanOp::Cut, c0, slab),
    );

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "PHASE 3 boolean on an imported body");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "PHASE 3: the boolean binds the imported target with no NeedsRepair"
    );
    assert_eq!(
        snap.bodies.len(),
        3,
        "PHASE 3: target modified + tool consumed ⇒ still three bodies, got {:?}",
        snap.bodies
            .iter()
            .map(|b| body_id_wire(b.body))
            .collect::<Vec<_>>()
    );
    let after_bool = head_by_wire(&rt);
    assert!(
        after_bool.contains_key(&body_id_wire(c0)),
        "PHASE 3: the boolean MODIFIES the imported target — its id is preserved"
    );
    assert!(
        !after_bool.contains_key(&body_id_wire(slab)),
        "PHASE 3: the boolean CONSUMES the modelled tool body"
    );
    assert!(
        report.changed.iter().any(|(b, _)| *b == c0),
        "PHASE 3: the imported target is reported changed, got {:?}",
        report
            .changed
            .iter()
            .map(|(b, _)| body_id_wire(*b))
            .collect::<Vec<_>>()
    );
    // §7.2 lineage, per record: the Boolean's affected set is EXACTLY the imported
    // target — it MODIFIED `:0` and created nothing. (`report.removed` is empty by
    // design here: a from-0 replay creates AND consumes the tool inside the same
    // regen, so the tool was never "present before". `affected_bodies` renders the
    // frontend-facing `BodyDto.id` form — the bare derived uuid, not the worker's
    // `body_<opId>:k` wire string.)
    let bool_affected = report
        .affected_bodies
        .get(&Uuid::from_u128(BOOLEAN_REC).to_string())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        bool_affected,
        vec![c0.to_string()],
        "PHASE 3: the Boolean's §7.2 lineage names ONLY the imported target it modified"
    );
    let slab_affected = report
        .affected_bodies
        .get(&Uuid::from_u128(SLAB_EXTRUDE_REC).to_string())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        slab_affected,
        vec![slab.to_string()],
        "PHASE 3: the tool's own lineage still names the tool it created"
    );

    let vol_c0 = body_volume(&mut rt, c0).await;
    let want_c0 = v0 * (BOX_H - SLAB_H) / BOX_H;
    assert!(
        (vol_c0 - want_c0).abs() < 1.0,
        "PHASE 3: Cut removes the slab's half of the imported box — want {want_c0}, got {vol_c0}"
    );
    // The slab passes straight through the other two imported bodies; a boolean
    // that leaked past its target would show up here immediately.
    assert!(
        (body_volume(&mut rt, c1).await - vol_c1).abs() < 1e-9,
        "PHASE 3: sibling `:1` is untouched by a boolean targeting `:0`"
    );
    assert!(
        (body_volume(&mut rt, c2).await - v2).abs() < 1e-9,
        "PHASE 3: sibling `:2` is untouched by a boolean targeting `:0`"
    );
    eprintln!("PHASE 3 PASS: Cut(imported :0, modelled slab) = {vol_c0:.1} (= {v0:.1}·{SLAB_H}/{BOX_H}), tool consumed, siblings untouched");

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 4 — sketch on an IMPORTED planar face, then host-boolean Add
    // ═════════════════════════════════════════════════════════════════════════
    let snap4 = SnapshotId(report.snapshot_id);
    let mesh2 = body_mesh(&mut rt, c2).await;
    let view2 = validate_mesh_blob(&mesh2).expect("imported `:2` MESH1");
    let top_key = top_face_pick(&view2, &mesh2);
    let face_el = rt
        .promote_selection(snap4, c2, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("promote a face of an imported body")[0]
        .element_id
        .clone();

    // `add_sketch_on_face` is only reachable through a real `tauri::State`;
    // `mock_app` is the only public way to build one outside a webview.
    let runtime_handle = app_state.runtime.clone();
    let app = tauri::test::mock_app();
    *runtime_handle.lock().await = Some(rt);
    app.manage(app_state);
    let sid = SketchId(Uuid::from_u128(ON_FACE_SKETCH));

    let dto = onecad_lib::api::add_sketch_on_face(
        app.state(),
        snap4.0,
        body_id_wire(c2),
        face_el.clone(),
        Some(top_key.clone()),
        sid.to_string(),
        "OnImportedFace".into(),
    )
    .await
    .expect("add_sketch_on_face on an IMPORTED planar face");

    assert_eq!(
        dto.entity_count, 8,
        "PHASE 4: a rectangular imported cap projects 4 locked points + 4 locked lines"
    );
    assert_eq!(
        dto.constraint_count, 4,
        "PHASE 4: one Fixed per projected point (reference-locked geometry)"
    );
    assert!(
        dto.has_closed_boundary,
        "PHASE 4: the projected imported cap closes"
    );
    assert_eq!(dto.projected_boundary_version, 1);
    assert!(
        (dto.plane.normal[2] - 1.0).abs() < 1e-9,
        "PHASE 4: the imported cap faces +Z, got {:?}",
        dto.plane.normal
    );
    assert!(
        (dto.plane.origin[2] - BOX_H).abs() < 1e-9,
        "PHASE 4: the kernel-exact origin lies ON the imported cap (z == {BOX_H}), got {}",
        dto.plane.origin[2]
    );

    let mut rt = runtime_handle.lock().await.take().expect("runtime back");
    let regions = {
        rt.enter_sketch(sid).await.expect("enter the face sketch");
        rt.finish_sketch(sid).await.expect("finish").regions
    };
    assert_eq!(
        regions.len(),
        1,
        "PHASE 4: the projected imported cap yields exactly one region"
    );

    add_op(
        &mut rt,
        host_extrude_record(
            FACE_EXTRUDE_REC,
            sid,
            &regions[0].region_id,
            RISER,
            BooleanMode::Add,
            c2,
        ),
    );
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "PHASE 4 host Add on an imported body");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "PHASE 4: the host-boolean Add needs no repair"
    );
    assert_eq!(
        snap.bodies.len(),
        3,
        "PHASE 4: an Add FUSES into the imported host — the document must not gain a body, got {:?}",
        snap.bodies.iter().map(|b| body_id_wire(b.body)).collect::<Vec<_>>()
    );
    assert!(
        report.changed.iter().any(|(b, _)| *b == c2),
        "PHASE 4: the imported host is reported CHANGED"
    );
    assert!(
        !report
            .changed
            .iter()
            .any(|(b, _)| *b == BodyId(Uuid::from_u128(FACE_EXTRUDE_REC))),
        "PHASE 4: no NewBody id was minted, got {:?}",
        report
            .changed
            .iter()
            .map(|(b, _)| body_id_wire(*b))
            .collect::<Vec<_>>()
    );
    let vol_c2 = body_volume(&mut rt, c2).await;
    let want_c2 = v2 * (BOX_H + RISER) / BOX_H;
    assert!(
        (vol_c2 - want_c2).abs() < 1.0,
        "PHASE 4: host Add grows the imported body by footprint×{RISER} — want {want_c2}, got {vol_c2}"
    );
    eprintln!(
        "PHASE 4 PASS: sketch on imported face {top_key} ({face_el}) → host Add ⇒ `:2` {v2:.1} → {vol_c2:.1}, no new body"
    );

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 5 — MESH1 for EVERY body at head validates
    // ═════════════════════════════════════════════════════════════════════════
    let head = head_by_wire(&rt);
    assert_eq!(head.len(), 3, "PHASE 5: three bodies at head");
    for (wire, body) in &head {
        let blob = body_mesh(&mut rt, *body).await;
        let view = validate_mesh_blob(&blob)
            .unwrap_or_else(|e| panic!("PHASE 5: {wire} must serve a VALID MESH1 blob: {e:?}"));
        assert!(
            view.triangle_count > 0 && view.face_count > 0,
            "PHASE 5: {wire} MESH1 carries real geometry"
        );
    }
    let pre_save_volumes = head_volumes(&mut rt).await;

    // The deterministic top-cap pick on the FINAL `:2` (riser included) plus the
    // kernel's own descriptor for it — the pair phase 6 replays in a new process.
    let final_mesh2 = body_mesh(&mut rt, c2).await;
    let final_view2 = validate_mesh_blob(&final_mesh2).expect("final `:2` MESH1");
    let final_top_key = top_face_pick(&final_view2, &final_mesh2);
    let final_snapshot = SnapshotId(report.snapshot_id);
    let cap_before =
        ElementQuery::query_element_by_topo_key(&wm, final_snapshot, c2, &final_top_key)
            .await
            .expect("QueryElement by topoKey")
            .expect("the picked cap resolves against the imported body");
    eprintln!("PHASE 5 PASS: MESH1 green for all 3 bodies — {pre_save_volumes:?}");

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 6 — process death: save ⇒ FRESH runtime on a FRESH worker
    // ═════════════════════════════════════════════════════════════════════════
    // The dry-run ladder verdict on the FILLET's persisted edge ref, taken in
    // process 1 — the reference the reopened process must reproduce EXACTLY.
    let probe = ResolveRef {
        ref_id: "w5.fillet.edge".into(),
        element: edge_ref(c1, &edge_el, edge_anchor),
    };
    let before = rt
        .resolve_refs(ResolveRequest {
            snapshot_id: SnapshotId(report.snapshot_id),
            refs: vec![probe.clone()],
        })
        .await
        .expect("resolve_refs dry-run in process 1");
    assert_eq!(before.len(), 1);

    // The persisted identity: the ElementId inside the FILLET record, and the one
    // inside the face-hosted sketch's attachment.
    let fillet_id_before = fillet_element_id(&rt, RecordId(Uuid::from_u128(FILLET_REC)));
    let host_face_before = host_face_element_id(&rt, sid);
    assert_eq!(
        fillet_id_before,
        edge_el.as_str(),
        "PHASE 6 precondition: the fillet record carries the promoted edge id"
    );
    assert_eq!(
        host_face_before, face_el,
        "PHASE 6 precondition: the sketch attachment carries the promoted face id"
    );

    let head1 = wm.get_worker_head().await.expect("worker 1 head");
    let doc = tmp.path().join("imported-downstream.onecad");
    rt.save(&doc, save_meta()).expect("save");
    drop(rt);
    wm.shutdown().await;

    let wm2 = spawn_worker(bin).await;
    let mut rt2 = open_over(&wm2, &doc);
    let report2 = regen_all(&mut rt2).await;
    let snap2 = published(&report2, "PHASE 6 reopen regen");

    // (a) the hash chain
    let head2 = wm2.get_worker_head().await.expect("worker 2 head");
    assert_eq!(
        head1.history_prefix_hash.as_str(),
        head2.history_prefix_hash.as_str(),
        "PHASE 6a: identical historyPrefixHash across two worker PROCESSES"
    );

    // (c) the fillet still binds — zero NeedsRepair after process death
    assert_eq!(
        snap2.repair_summary.needs_repair_count, 0,
        "PHASE 6c: the fillet on the imported body regens clean in a fresh process"
    );
    assert!(
        report2.failed_steps.is_empty(),
        "PHASE 6c: no step failed on reopen, got {:?}",
        report2.failed_steps
    );

    // (d) volumes
    let reopened_volumes = head_volumes(&mut rt2).await;
    assert_eq!(
        reopened_volumes.keys().collect::<Vec<_>>(),
        pre_save_volumes.keys().collect::<Vec<_>>(),
        "PHASE 6d: the SAME body ids come back (the record id is persisted)"
    );
    for (wire, want) in &pre_save_volumes {
        let got = reopened_volumes[wire];
        assert!(
            (got - want).abs() < 1e-6,
            "PHASE 6d: {wire} volume must survive process death — want {want}, got {got}"
        );
    }

    // (b) identity — the persisted ElementIds are byte-identical, and the SAME id
    // still designates the SAME element: the worker's ladder in a FRESH PROCESS
    // reaches the IDENTICAL verdict on it. (A fillet CONSUMES its edge, so the
    // correct verdict is NOT an autoBind — see `topology_rebind`'s note; what this
    // pins is that whatever the ladder concludes, it concludes it about the same
    // minted id, identically, in a process that never saw the promotion.)
    assert_eq!(
        fillet_element_id(&rt2, RecordId(Uuid::from_u128(FILLET_REC))),
        fillet_id_before,
        "PHASE 6b: the fillet's ElementId survives the container VERBATIM"
    );
    assert_eq!(
        host_face_element_id(&rt2, sid),
        host_face_before,
        "PHASE 6b: the face-hosted sketch's host ElementId survives the container VERBATIM"
    );
    let after = rt2
        .resolve_refs(ResolveRequest {
            snapshot_id: SnapshotId(report2.snapshot_id),
            refs: vec![probe],
        })
        .await
        .expect("resolve_refs dry-run in process 2");
    assert_eq!(after.len(), 1);
    assert_eq!(
        outcome_fingerprint(&before[0].outcome),
        outcome_fingerprint(&after[0].outcome),
        "PHASE 6b: the ladder's verdict on the SAME minted ElementId is IDENTICAL \
         across process death — before {:?}, after {:?}",
        before[0].outcome,
        after[0].outcome
    );

    // …and the topoKey EVIDENCE is deterministic too: the SAME pick on the reopened
    // body re-derives the SAME snapshot-scoped key, which still addresses a face with
    // the SAME kernel descriptor. That is the "same id ⇒ same face" half of
    // Invariant 1 stated in the only terms a topoKey may be stated in (§9 evidence).
    let snap2_id = SnapshotId(report2.snapshot_id);
    let mesh2b = body_mesh(&mut rt2, c2).await;
    let view2b = validate_mesh_blob(&mesh2b).expect("reopened `:2` MESH1");
    let reopened_top_key = top_face_pick(&view2b, &mesh2b);
    assert_eq!(
        reopened_top_key, final_top_key,
        "PHASE 6b: the top-cap pick on the imported body is deterministic across processes"
    );
    let cap_after = ElementQuery::query_element_by_topo_key(&wm2, snap2_id, c2, &reopened_top_key)
        .await
        .expect("QueryElement by topoKey in process 2")
        .expect("the same pick still resolves");
    assert_eq!(
        cap_after.kind, cap_before.kind,
        "PHASE 6b: the re-picked element is still a face"
    );
    assert!(
        (cap_after.magnitude - cap_before.magnitude).abs() < 1e-9
            && (0..3).all(|i| (cap_after.normal[i] - cap_before.normal[i]).abs() < 1e-9),
        "PHASE 6b: the same pick designates the SAME face across process death — \
         before area {} normal {:?}, after area {} normal {:?}",
        cap_before.magnitude,
        cap_before.normal,
        cap_after.magnitude,
        cap_after.normal
    );

    // The re-promotion itself: picking that key again reuses the id, and a repeated
    // pick is STABLE (Invariant 1's in-session half).
    //
    // What this deliberately does NOT assert is that the FIRST pick in the new
    // process returns the pre-save id. It cannot, and that is by design rather than
    // by accident: `DocumentRuntime::promoted` is in-memory, and the worker's
    // element-map partition mints an entry only when an OP resolves an id as an
    // input — so an ElementId no record carries has nothing to be recovered FROM.
    // The document's durable identity is the set of ids its RECORDS carry, which is
    // exactly what 6b above pins verbatim.
    let repromoted = rt2
        .promote_selection(snap2_id, c2, vec![(TopoKey::new(&reopened_top_key), None)])
        .await
        .expect("re-promote the same pick in process 2")[0]
        .element_id
        .clone();
    assert!(
        repromoted.starts_with("el_"),
        "PHASE 6b: the re-promotion mints a well-formed ElementId, got {repromoted}"
    );
    let repromoted_again = rt2
        .promote_selection(snap2_id, c2, vec![(TopoKey::new(&reopened_top_key), None)])
        .await
        .expect("re-promote twice")[0]
        .element_id
        .clone();
    assert_eq!(
        repromoted, repromoted_again,
        "PHASE 6b: re-picking the SAME (body, topoKey) reuses the SAME id (Invariant 1)"
    );
    eprintln!(
        "PHASE 6 PASS: hash chain {} identical across processes; ElementIds verbatim; \
         ladder verdict {:?} reproduced; cap pick {reopened_top_key} stable (re-promoted {repromoted}, \
         pre-save host id {host_face_before}); volumes exact",
        &head1.history_prefix_hash.as_str()[..12],
        outcome_fingerprint(&after[0].outcome)
    );

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 7 — repair honesty: delete the import, everything downstream must
    //           fail LOUDLY (never a silent wrong bind); undo restores the chain
    // ═════════════════════════════════════════════════════════════════════════
    let chain_before_delete = head2.history_prefix_hash.as_str().to_string();
    let import_rec2 = import_record_id(&rt2);
    rt2.apply(EditCommand::RemoveOperation {
        record: RecordId(import_rec2),
    })
    .expect("RemoveOperation on the ImportStep record");

    let report3 = regen_all(&mut rt2).await;
    let snap3 = published(&report3, "PHASE 7 degraded regen").clone();
    // The plan STOPS at the first op whose input cannot be bound and publishes
    // `m−1` (SCHEMA §7.2/§8) — it never runs the rest against whatever happens to
    // be lying around.
    assert_eq!(
        snap3.stopped_reason,
        StoppedReason::NeedsRepair,
        "PHASE 7: an unbindable downstream ref stops the plan as STATE (§8), not as a crash"
    );
    let fillet_rec = RecordId(Uuid::from_u128(FILLET_REC));
    let fillet_state = state_of(&rt2, &snap3, fillet_rec);
    assert!(
        matches!(
            fillet_state,
            Some(StepState::Error { .. } | StepState::NeedsRepair)
        ),
        "PHASE 7: the first downstream op must be LOUD (Error / NeedsRepair), got {fillet_state:?} \
         — a Valid step here would mean it silently bound some OTHER body"
    );
    // …and it is surfaced to the UI, naming the exact record and the §9 reason.
    let item = report3
        .needs_repair
        .iter()
        .find(|i| i.op_id == fillet_rec.0.to_string())
        .unwrap_or_else(|| {
            panic!(
                "PHASE 7: the repair banner must name the fillet record, got {:?}",
                report3.needs_repair
            )
        });
    assert_eq!(
        item.candidate_count, 0,
        "PHASE 7: with the import gone there are NO candidates — the ladder must say so \
         rather than reach for a survivor, got {item:?}"
    );
    for (name, rec) in [
        ("Boolean", RecordId(Uuid::from_u128(BOOLEAN_REC))),
        ("host Extrude", RecordId(Uuid::from_u128(FACE_EXTRUDE_REC))),
    ] {
        let state = state_of(&rt2, &snap3, rec);
        assert!(
            !matches!(state, Some(StepState::Valid)),
            "PHASE 7: {name} sits BEHIND the stopped step — it must NOT report Valid, got {state:?}"
        );
        eprintln!("PHASE 7: {name} ⇒ {state:?}");
    }
    assert!(
        snap3.bodies.is_empty(),
        "PHASE 7: the plan stopped at m−1 = the empty base — no body may be published \
         from a document whose only geometry source was deleted, got {:?}",
        snap3
            .bodies
            .iter()
            .map(|b| body_id_wire(b.body))
            .collect::<Vec<_>>()
    );
    eprintln!(
        "PHASE 7: Fillet ⇒ {fillet_state:?} (reason {:?}, candidates {})",
        item.reason, item.candidate_count
    );

    // Undo restores the record — and with it the exact hash chain.
    assert!(
        rt2.undo().is_some(),
        "PHASE 7: undo restores the deleted import record"
    );
    let report4 = regen_all(&mut rt2).await;
    let snap4b = published(&report4, "PHASE 7 undo regen");
    assert_eq!(
        snap4b.repair_summary.needs_repair_count, 0,
        "PHASE 7: undo restores a fully-bound document"
    );
    assert!(
        report4.failed_steps.is_empty(),
        "PHASE 7: undo restores a clean timeline, got {:?}",
        report4.failed_steps
    );
    let head4 = wm2.get_worker_head().await.expect("worker head after undo");
    assert_eq!(
        head4.history_prefix_hash.as_str(),
        chain_before_delete,
        "PHASE 7: undo restores the EXACT hash chain"
    );
    let restored = head_volumes(&mut rt2).await;
    assert_eq!(
        restored.keys().collect::<Vec<_>>(),
        pre_save_volumes.keys().collect::<Vec<_>>(),
        "PHASE 7: undo restores the same imported body ids"
    );
    for (wire, want) in &pre_save_volumes {
        let got = restored[wire];
        assert!(
            (got - want).abs() < 1e-6,
            "PHASE 7: {wire} volume restored exactly — want {want}, got {got}"
        );
    }

    wm2.shutdown().await;
    eprintln!("W5 GATE: all 7 phases asserted");
}

// ─────────────────────────────────────────────────────────────────────────────
// Small readers used by phase 6/7
// ─────────────────────────────────────────────────────────────────────────────

/// The `ElementId` string persisted in a Fillet record's first edge ref.
fn fillet_element_id(rt: &DocumentRuntime, record: RecordId) -> String {
    let params = rt
        .operation_params(record)
        .expect("the fillet record's params");
    params["edges"][0]["primary"]["elementId"]
        .as_str()
        .expect("the fillet edge ref carries a primary elementId")
        .to_string()
}

/// The `ElementId` string persisted in a face-hosted sketch's attachment.
fn host_face_element_id(rt: &DocumentRuntime, sketch: SketchId) -> String {
    rt.projection()
        .sketches
        .get(&sketch.to_string())
        .expect("the face-hosted sketch is projected")
        .host_face
        .as_ref()
        .expect("a HostFace attachment projects its primary binding")
        .element_id
        .clone()
}

/// A comparable, process-independent shape of a ladder verdict: the variant plus
/// the `ElementId` it processed plus its topoKey evidence. Scores/margins are
/// floats the two processes need not reproduce bit-for-bit; identity and evidence
/// they must.
fn outcome_fingerprint(outcome: &ResolveOutcome) -> (String, Option<String>, Vec<String>) {
    match outcome {
        ResolveOutcome::AutoBind {
            element_id,
            topo_key,
            ..
        } => (
            "autoBind".into(),
            Some(element_id.as_str().to_string()),
            topo_key.iter().map(|k| k.as_str().to_string()).collect(),
        ),
        ResolveOutcome::Unchanged { element_id } => (
            "unchanged".into(),
            element_id.as_ref().map(|e| e.as_str().to_string()),
            vec![],
        ),
        ResolveOutcome::NeedsRepair(item) => (
            "needsRepair".into(),
            item.element_id.as_ref().map(|e| e.as_str().to_string()),
            item.candidates
                .iter()
                .map(|c| c.topo_key.as_str().to_string())
                .collect(),
        ),
    }
}
