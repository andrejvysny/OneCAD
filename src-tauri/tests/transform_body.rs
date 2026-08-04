//! WP-B W0 `TransformBody` integration gate against the REAL C++ OCCT worker,
//! driven through the app's [`DocumentRuntime`] exactly like `breadth_ops.rs`.
//!
//! Proves the op end to end (core params → wire lowering → worker dispatch → OCCT →
//! MESH1) and, crucially, the SCHEMA §7.3 **edit-safety gate**:
//!
//! * `translate_moves_exactly_and_preserves_the_body_id` — a `[15,0,0]` move shifts
//!   the bbox by exactly 15, keeps the volume, keeps the BodyId (MODIFY lineage —
//!   the first body-level op with it), and advances the worker hash chain.
//! * `rotate_about_z_preserves_volume` — 45° about the body's own centre.
//! * `copy_two_targets_mints_ordinal_children` — `copy:true` × 2 ⇒ `body_<opId>:0`
//!   / `:1`, both sources intact (4 bodies at head).
//! * `copy_one_target_mints_the_plain_body_id` — the N == 1 minting branch.
//! * `multi_target_move_moves_both_bodies` — two bodies, ONE record.
//! * `undo_restores_the_exact_prior_hash_chain` — cumulative placement is undoable.
//! * `editing_a_transform_seeds_needs_repair_on_the_downstream_fillet` — the V1
//!   gate: the healthy fit-check flow (move → fillet the moved box) MUST work, and
//!   editing the move afterwards must make the fillet `NeedsRepair` rather than
//!   silently re-binding it; undo restores the healthy binding exactly.
//! * `suppressing_a_transform_seeds_the_same_gate` — suppression is the same class
//!   of geometry move.
//!
//! **Numeric-precision note.** MESH1 vertex positions are f32 (`mesh_format.md`),
//! so a volume read back through the mesh lane carries ~1e-3 absolute error on a
//! 10⁴ mm³ body — the exact `1e-9` volume invariance under a rigid motion is pinned
//! worker-side against the real `BRepGProp` shape (`test_transform_body`). Here the
//! bounds are the tightest the mesh lane can honestly support (1.0 on 10 000).
//!
//! **World-placement note.** The named XY sketch plane uses the OneCAD-CPP
//! non-standard basis (`xAxis=(0,1,0)`, `yAxis=(−1,0,0)` — SCHEMA §7.3), so a
//! sketch rectangle at `(0,0)+20×20` extrudes to a body at world `x ∈ [−20,0]`,
//! `y ∈ [0,20]`. Every placement assertion below is therefore a bbox **delta**
//! against the body's own pre-move bbox, never an absolute world coordinate — the
//! delta is what a rigid motion actually promises, and it stays correct if the
//! basis is ever revisited.
//!
//! Gated on `ONECAD_WORKER_PATH` (else dev-tree fallback); a missing binary skips
//! cleanly unless `ONECAD_REQUIRE_WORKER=1`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::body::split_child_uuid;
use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef, TransformBodyParams,
    TransformRotation,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::history::StepState;
use onecad_core::ids::{BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{
    Constraint, CurvePosition, Sketch, SketchAttachment, SketchEntity, WorldPlane,
};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors breadth_ops.rs / wire_contract.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} is set but no worker binary exists there"
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

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// Fixed record ids (distinct per scenario so a shared runtime never collides).
const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;
const SKETCH_B: u128 = 0xB00;
const EXTRUDE_B: u128 = 0xB01;
const OP_MOVE: u128 = 0xC00;
const OP_FILLET: u128 = 0xD00;

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + record builders (rect_sketch verbatim from breadth_ops.rs)
// ─────────────────────────────────────────────────────────────────────────────

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
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
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
        })),
    )
}

/// A `TransformBody` operation. `rotate` defaults to identity about the origin.
fn transform_op(
    targets: Vec<BodyId>,
    translate: [f64; 3],
    rotate: Option<TransformRotation>,
    copy: bool,
) -> Operation {
    Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
        targets,
        translate: [
            Scalar::new(translate[0]),
            Scalar::new(translate[1]),
            Scalar::new(translate[2]),
        ],
        rotate: rotate.unwrap_or_default(),
        copy,
        extra: Default::default(),
    }))
}

fn transform_record(
    rec: u128,
    targets: Vec<BodyId>,
    translate: [f64; 3],
    rotate: Option<TransformRotation>,
    copy: bool,
) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Move",
        transform_op(targets, translate, rotate, copy),
    )
}

/// A single-edge fillet carrying a typed [`ElementRef`] (Rust-minted `ElementId` +
/// a world-midpoint anchor) — the SCHEMA §7.3 `inputs[]` shape the ladder binds.
fn fillet_record(body: BodyId, edge_el: ElementId, anchor: Vec3, radius: f64) -> OperationRecord {
    let edge_ref = ElementRef {
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
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(OP_FILLET)),
        0,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(radius),
            edge_ids: vec![edge_el],
            edges: vec![edge_ref],
            chain_tangent_edges: false,
            extra: Default::default(),
        })),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 geometry helpers (from breadth_ops.rs / topology_rebind.rs)
// ─────────────────────────────────────────────────────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
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

/// Signed volume via the divergence theorem — exact for a closed planar-faced
/// polyhedron, to f32 vertex precision.
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

fn bbox_min(view: &MeshHeaderView) -> [f64; 3] {
    [
        f64::from(view.bbox_min[0]),
        f64::from(view.bbox_min[1]),
        f64::from(view.bbox_min[2]),
    ]
}

fn bbox_dims(view: &MeshHeaderView) -> [f64; 3] {
    [
        f64::from(view.bbox_max[0] - view.bbox_min[0]),
        f64::from(view.bbox_max[1] - view.bbox_min[1]),
        f64::from(view.bbox_max[2] - view.bbox_min[2]),
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

/// The edge with the greatest world-Z extent (a vertical box edge — always safely
/// fillettable) → its centroid, used as the fillet ref's world anchor.
fn vertical_edge_anchor(view: &MeshHeaderView, blob: &[u8]) -> Vec3 {
    assert!(
        view.has_edges(),
        "MESH1 must carry edges for the fillet pick"
    );
    let er = view.section(SEC_EDGE_RANGES).expect("EDGE_RANGES");
    let ep = view.section(SEC_EDGE_POSITIONS).expect("EDGE_POSITIONS");
    let _keys = id_table(
        view,
        blob,
        SEC_EDGE_ID_OFFS,
        SEC_EDGE_ID_CHARS,
        view.edge_count as usize,
    );
    let (erbase, epbase) = (er.offset as usize, ep.offset as usize);
    let mut best: Option<(f64, Vec3)> = None;
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
        if best.is_none_or(|(s, _)| span > s) {
            best = Some((span, centroid));
        }
    }
    best.expect("at least one edge").1
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

/// `(volume, bboxMin, bboxDims)` of a body's current mesh.
async fn body_geom(rt: &mut DocumentRuntime, body: BodyId) -> (f64, [f64; 3], [f64; 3]) {
    let mesh = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("MESH1 validates");
    (mesh_volume(&view, &mesh), bbox_min(&view), bbox_dims(&view))
}

/// Builds a 20×20 rect extruded `depth` (volume 400·depth) and regenerates.
async fn build_box(
    rt: &mut DocumentRuntime,
    sketch_rec: u128,
    extrude_rec: u128,
    sk_seed: u128,
    x0: f64,
    y0: f64,
    depth: f64,
) -> BodyId {
    let sid = SketchId(Uuid::from_u128(sk_seed));
    add_op(
        rt,
        sketch_record(
            sketch_rec,
            &rect_sketch(sid, sk_seed << 8, x0, y0, 20.0, 20.0),
        ),
    );
    add_op(rt, extrude_record(extrude_rec, sid, depth));
    body_of(extrude_rec)
}

const BOX_VOL: f64 = 20.0 * 20.0 * 25.0; // 10 000

// ─────────────────────────────────────────────────────────────────────────────
// (1) Translate — exact move, volume + BodyId preserved, hash chain advances
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn translate_moves_exactly_and_preserves_the_body_id() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let body = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let _ = published(&regen_all(&mut rt).await, "box");
    let (vol0, min0, dims0) = body_geom(&mut rt, body).await;
    let head0 = wm.get_worker_head().await.expect("head before move");

    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![body], [15.0, 0.0, 0.0], None, false),
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "translate");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "a rigid move resolves everything"
    );
    assert_eq!(
        snap.bodies.len(),
        1,
        "MODIFY lineage mints NO body — the moved box IS the box"
    );
    assert_eq!(
        snap.bodies[0].body, body,
        "BodyId preserved across the move (modify lineage, SCHEMA §7.3)"
    );

    let (vol1, min1, dims1) = body_geom(&mut rt, body).await;
    assert!(
        (vol1 - BOX_VOL).abs() < 1.0 && (vol1 - vol0).abs() < 1.0,
        "a rigid motion preserves volume: {vol0} → {vol1} (want {BOX_VOL})"
    );
    assert!(
        (min1[0] - min0[0] - 15.0).abs() < 1e-3,
        "bbox x shifted by EXACTLY 15: {} → {}",
        min0[0],
        min1[0]
    );
    assert!(
        (min1[1] - min0[1]).abs() < 1e-3 && (min1[2] - min0[2]).abs() < 1e-3,
        "y/z untouched: {min0:?} → {min1:?}"
    );
    for i in 0..3 {
        assert!(
            (dims1[i] - dims0[i]).abs() < 1e-3,
            "the box did not change shape: {dims0:?} → {dims1:?}"
        );
    }

    let head1 = wm.get_worker_head().await.expect("head after move");
    assert_ne!(
        head0.history_prefix_hash, head1.history_prefix_hash,
        "adding a TransformBody advances the hash chain"
    );

    wm.shutdown().await;
    eprintln!(
        "TransformBody translate PASS: bbox {min0:?} → {min1:?}, volume {vol1}, id preserved"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (2) Rotate — 45° about Z through the body's own centre
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rotate_about_z_preserves_volume() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let body = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let _ = published(&regen_all(&mut rt).await, "box");
    let (vol0, _, dims0) = body_geom(&mut rt, body).await;

    // Pivot = the 20×20×25 box's own centre (10, 10, 12.5) — the FROZEN pivot a
    // first authoring captures.
    let rotate = TransformRotation {
        center: Vec3::new_unchecked(10.0, 10.0, 12.5),
        axis: Vec3::new_unchecked(0.0, 0.0, 1.0),
        angle_deg: Scalar::new(45.0),
    };
    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![body], [0.0, 0.0, 0.0], Some(rotate), false),
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "rotate");
    assert_eq!(snap.repair_summary.needs_repair_count, 0);
    assert_eq!(snap.bodies.len(), 1);

    let (vol1, _, dims1) = body_geom(&mut rt, body).await;
    assert!(
        (vol1 - vol0).abs() < 1.0,
        "a rotation preserves volume: {vol0} → {vol1}"
    );
    // A 45° turn of a 20×20 footprint about its centre widens the AXIS-ALIGNED
    // bbox to 20·√2 ≈ 28.284 in x and y, and leaves z alone. If the rotation were
    // silently dropped this would still read 20×20.
    let want = 20.0 * std::f64::consts::SQRT_2;
    assert!(
        (dims1[0] - want).abs() < 0.05 && (dims1[1] - want).abs() < 0.05,
        "45° about Z ⇒ bbox footprint {want:.3}² (was {dims0:?}), got {dims1:?}"
    );
    assert!(
        (dims1[2] - dims0[2]).abs() < 1e-3,
        "the rotation axis dimension is untouched"
    );

    wm.shutdown().await;
    eprintln!("TransformBody rotate PASS: footprint {dims0:?} → {dims1:?}, volume {vol1}");
}

// ─────────────────────────────────────────────────────────────────────────────
// (3) copy: true — §2 N-body ordinal minting; sources intact
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_two_targets_mints_ordinal_children() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let a = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let b = build_box(&mut rt, SKETCH_B, EXTRUDE_B, 0xB, 100.0, 0.0, 10.0).await;
    let _ = published(&regen_all(&mut rt).await, "two boxes");
    let (_, a_min0, _) = body_geom(&mut rt, a).await;
    let (_, b_min0, _) = body_geom(&mut rt, b).await;

    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![a, b], [0.0, 200.0, 0.0], None, true),
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "copy two");

    let child0 = BodyId(split_child_uuid(Uuid::from_u128(OP_MOVE), 0));
    let child1 = BodyId(split_child_uuid(Uuid::from_u128(OP_MOVE), 1));
    let head: std::collections::HashSet<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    assert_eq!(head.len(), 4, "2 sources + 2 copies, got {head:?}");
    assert!(
        head.contains(&a) && head.contains(&b),
        "copy: true PRESERVES its sources (no `deleted` event), got {head:?}"
    );
    assert!(
        head.contains(&child0) && head.contains(&child1),
        "N > 1 ⇒ ordinal children body_<opId>:0/:1, got {head:?}"
    );

    // Ordinals follow `targets` order: :0 copies A (10 000), :1 copies B (4 000).
    let (v0, min0, _) = body_geom(&mut rt, child0).await;
    let (v1, min1, _) = body_geom(&mut rt, child1).await;
    assert!(
        (v0 - BOX_VOL).abs() < 1.0,
        "child :0 = copy of targets[0] (box A, 10000), got {v0}"
    );
    assert!(
        (v1 - 20.0 * 20.0 * 10.0).abs() < 1.0,
        "child :1 = copy of targets[1] (box B, 4000), got {v1}"
    );
    assert!(
        (min0[1] - a_min0[1] - 200.0).abs() < 1e-3 && (min1[1] - b_min0[1] - 200.0).abs() < 1e-3,
        "each copy sits +200 in Y from ITS OWN source: {a_min0:?}→{min0:?} / {b_min0:?}→{min1:?}"
    );
    // The sources are exactly where they were.
    let (_, amin, _) = body_geom(&mut rt, a).await;
    let (_, bmin, _) = body_geom(&mut rt, b).await;
    assert!(
        (amin[1] - a_min0[1]).abs() < 1e-3 && (bmin[1] - b_min0[1]).abs() < 1e-3,
        "copy: true leaves both SOURCES untouched, got {amin:?} / {bmin:?}"
    );

    wm.shutdown().await;
    eprintln!("TransformBody copy×2 PASS: :0={v0} :1={v1}, 4 bodies at head, sources intact");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_one_target_mints_the_plain_body_id() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let a = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let _ = published(&regen_all(&mut rt).await, "box");
    let (_, a_min0, _) = body_geom(&mut rt, a).await;
    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![a], [0.0, 60.0, 0.0], None, true),
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "copy one");

    let head: std::collections::HashSet<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    assert_eq!(head.len(), 2, "source + one copy, got {head:?}");
    assert!(
        head.contains(&a) && head.contains(&body_of(OP_MOVE)),
        "exactly ONE target ⇒ PLAIN body_<opId> (no `:0` suffix), got {head:?}"
    );
    let (v, min, _) = body_geom(&mut rt, body_of(OP_MOVE)).await;
    assert!(
        (v - BOX_VOL).abs() < 1.0,
        "the copy keeps the source volume"
    );
    assert!(
        (min[1] - a_min0[1] - 60.0).abs() < 1e-3,
        "the copy sits +60 in Y from its source: {a_min0:?} → {min:?}"
    );

    wm.shutdown().await;
    eprintln!("TransformBody copy×1 PASS: plain body_<opId> minted, volume {v}");
}

// ─────────────────────────────────────────────────────────────────────────────
// (4) Multi-target move — two bodies, ONE record
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_target_move_moves_both_bodies() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let a = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let b = build_box(&mut rt, SKETCH_B, EXTRUDE_B, 0xB, 100.0, 0.0, 10.0).await;
    let _ = published(&regen_all(&mut rt).await, "two boxes");
    let (_, a_min0, _) = body_geom(&mut rt, a).await;
    let (_, b_min0, _) = body_geom(&mut rt, b).await;

    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![a, b], [0.0, 0.0, 40.0], None, false),
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "multi move");
    assert_eq!(
        snap.bodies.len(),
        2,
        "still exactly the two original bodies"
    );

    let (va, amin, _) = body_geom(&mut rt, a).await;
    let (vb, bmin, _) = body_geom(&mut rt, b).await;
    assert!(
        (amin[2] - a_min0[2] - 40.0).abs() < 1e-3 && (bmin[2] - b_min0[2] - 40.0).abs() < 1e-3,
        "ONE record moved BOTH targets +40 in Z, got {a_min0:?}→{amin:?} / {b_min0:?}→{bmin:?}"
    );
    assert!((va - BOX_VOL).abs() < 1.0 && (vb - 4000.0).abs() < 1.0);
    assert!(
        (amin[0] - a_min0[0]).abs() < 1e-3
            && (amin[1] - a_min0[1]).abs() < 1e-3
            && (bmin[0] - b_min0[0]).abs() < 1e-3
            && (bmin[1] - b_min0[1]).abs() < 1e-3,
        "each body kept its own X/Y — a shared rigid motion, not a merge"
    );
    assert!(
        (bmin[1] - amin[1]).abs() > 50.0,
        "the two bodies are still 100mm apart (the move did not collapse them)"
    );

    wm.shutdown().await;
    eprintln!("TransformBody multi-target PASS: both bodies at z=40");
}

// ─────────────────────────────────────────────────────────────────────────────
// (5) Undo — the exact prior hash chain
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undo_restores_the_exact_prior_hash_chain() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let body = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let _ = published(&regen_all(&mut rt).await, "box");
    let head0 = wm.get_worker_head().await.expect("head before");
    let (_, min0, _) = body_geom(&mut rt, body).await;

    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![body], [33.0, 0.0, 0.0], None, false),
    );
    let _ = published(&regen_all(&mut rt).await, "move");
    let (_, moved, _) = body_geom(&mut rt, body).await;
    assert!((moved[0] - min0[0] - 33.0).abs() < 1e-3, "moved +33");

    assert!(rt.undo().is_some(), "undo the move");
    let _ = published(&regen_all(&mut rt).await, "after undo");
    let head1 = wm.get_worker_head().await.expect("head after undo");
    assert_eq!(
        head0.history_prefix_hash, head1.history_prefix_hash,
        "undo restores the EXACT prior hash chain"
    );
    let (_, back, _) = body_geom(&mut rt, body).await;
    assert!(
        (back[0] - min0[0]).abs() < 1e-3,
        "geometry back where it started: {min0:?} → {moved:?} → {back:?}"
    );

    wm.shutdown().await;
    eprintln!("TransformBody undo PASS: hash chain restored exactly");
}

// ─────────────────────────────────────────────────────────────────────────────
// (6) THE EDIT-SAFETY GATE (SCHEMA §7.3, V1)
// ─────────────────────────────────────────────────────────────────────────────

/// The healthy fit-check flow MUST work — move a part, then fillet an edge of the
/// MOVED part and have it resolve cleanly — and editing that move afterwards MUST
/// make the fillet `NeedsRepair`, never a silent re-bind. Undo restores the healthy
/// binding exactly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn editing_a_transform_seeds_needs_repair_on_the_downstream_fillet() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // steps: 0 Sketch · 1 Extrude · 2 TransformBody · 3 Fillet
    let body = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let _ = published(&regen_all(&mut rt).await, "box");
    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![body], [15.0, 0.0, 0.0], None, false),
    );
    let _ = published(&regen_all(&mut rt).await, "move");

    // ── the HEALTHY flow: pick an edge of the MOVED box and fillet it ──────────
    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("MESH1 validates");
    let anchor = vertical_edge_anchor(&view, &mesh);
    const R: f64 = 3.0;
    add_op(
        &mut rt,
        fillet_record(body, ElementId::new("el_move_edge"), anchor, R),
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "fillet on the moved box");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "HEALTHY: an op authored AFTER a move resolves honestly (level-1 rebind)"
    );
    let (filleted_vol, _, _) = body_geom(&mut rt, body).await;
    // A vertical-edge fillet removes (1 − π/4)·r²·height of material (≈48.3 here).
    // The bound is 10 mm³ because the fillet's CURVED face is chord-approximated at
    // Lod::Coarse, so the mesh under-fills it by a few mm³ — the exact-arithmetic
    // check lives in the ctest, this one only has to prove the fillet APPLIED.
    let expected_removed = (1.0 - std::f64::consts::FRAC_PI_4) * R * R * 25.0;
    assert!(
        (filleted_vol - (BOX_VOL - expected_removed)).abs() < 10.0,
        "the fillet really applied: {filleted_vol} ≈ {}",
        BOX_VOL - expected_removed
    );
    assert!(
        (BOX_VOL - filleted_vol) > 5.0,
        "the filleted volume is DISTINGUISHABLE from the plain box"
    );
    let healthy_head = wm.get_worker_head().await.expect("healthy head");

    // ── EDIT the move's translate ⇒ the gate fires ────────────────────────────
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(OP_MOVE)),
        op: transform_op(vec![body], [15.0, 40.0, 0.0], None, false),
    })
    .expect("UpdateOperationParams on the TransformBody");

    let gated: Vec<_> = rt.repair_items().iter().filter(|i| i.seeded).collect();
    assert!(
        !gated.is_empty(),
        "editing a TransformBody seeds NeedsRepair on every downstream ref"
    );
    assert!(
        gated.iter().all(|i| i.step_index == 3),
        "the seed lands on the FILLET step (3), got {:?}",
        gated.iter().map(|i| i.step_index).collect::<Vec<_>>()
    );
    assert!(
        gated
            .iter()
            .any(|i| i.element_id.as_ref().map(|e| e.as_str()) == Some("el_move_edge")),
        "the gate names the fillet's edge ref"
    );

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "regen after the transform edit");
    assert!(
        snap.repair_summary.needs_repair_count > 0,
        "the document reports NeedsRepair after a transform edit"
    );
    assert!(
        snap.step_states
            .iter()
            .any(|(i, st)| *i == 3 && *st == StepState::NeedsRepair),
        "the fillet step is NeedsRepair, got {:?}",
        snap.step_states
    );
    assert!(
        !rep.needs_repair.is_empty(),
        "the RegenReport surfaces the gate to the frontend"
    );

    // NEVER silently bound: the gated fillet did NOT execute, so the published
    // body is the plain (moved) box — no filleted volume of ANY variant appears.
    assert_eq!(snap.bodies.len(), 1);
    let (gated_vol, gmin, _) = body_geom(&mut rt, body).await;
    assert!(
        (gated_vol - BOX_VOL).abs() < 1.0,
        "publish ≤ m−1: the plain box, NOT a (possibly mis-bound) fillet — got {gated_vol}"
    );
    assert!(
        (gated_vol - filleted_vol).abs() > 5.0,
        "the published volume is NOT the filleted one — the ref was never re-bound"
    );
    assert!(
        (gmin[1] - 40.0).abs() < 1e-3,
        "the EDITED move itself still applied (the gate truncates AFTER it)"
    );

    // ── UNDO ⇒ the gate lifts, the fillet is healthy again, hash restored ─────
    assert!(rt.undo().is_some(), "undo the transform edit");
    assert!(
        rt.repair_items().iter().all(|i| !i.seeded),
        "the seed rides the SAME undo entry as the edit that planted it"
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "regen after undo");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "undo restores the previous binding state exactly"
    );
    let (restored_vol, _, _) = body_geom(&mut rt, body).await;
    assert!(
        (restored_vol - filleted_vol).abs() < 1.0,
        "the fillet is back: {filleted_vol} vs {restored_vol}"
    );
    let head_after = wm.get_worker_head().await.expect("head after undo");
    assert_eq!(
        healthy_head.history_prefix_hash, head_after.history_prefix_hash,
        "undo restores the exact pre-edit hash chain"
    );

    wm.shutdown().await;
    eprintln!(
        "Edit-safety gate PASS: healthy {filleted_vol:.1} → gated {gated_vol:.1} (unfilleted) \
         → restored {restored_vol:.1}"
    );
}

/// Suppression (and un-suppression) of a `TransformBody` moves downstream geometry
/// exactly like a params edit, so it seeds the SAME gate.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suppressing_a_transform_seeds_the_same_gate() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let body = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let _ = published(&regen_all(&mut rt).await, "box");
    let (_, unmoved_min, _) = body_geom(&mut rt, body).await;
    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![body], [15.0, 0.0, 0.0], None, false),
    );
    let _ = published(&regen_all(&mut rt).await, "move");

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("MESH1 validates");
    let anchor = vertical_edge_anchor(&view, &mesh);
    add_op(
        &mut rt,
        fillet_record(body, ElementId::new("el_move_edge"), anchor, 3.0),
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "fillet");
    assert_eq!(snap.repair_summary.needs_repair_count, 0, "healthy first");
    let (filleted_vol, _, _) = body_geom(&mut rt, body).await;

    rt.apply(EditCommand::SetOperationSuppression {
        record: RecordId(Uuid::from_u128(OP_MOVE)),
        suppressed: true,
        cascade: false,
    })
    .expect("SetOperationSuppression on the TransformBody");
    assert!(
        rt.repair_items()
            .iter()
            .any(|i| i.seeded && i.step_index == 3),
        "suppressing a TransformBody seeds the same downstream gate"
    );

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "regen after suppress");
    assert!(
        snap.step_states
            .iter()
            .any(|(i, st)| *i == 3 && *st == StepState::NeedsRepair),
        "the fillet step is NeedsRepair, got {:?}",
        snap.step_states
    );
    let (gated_vol, gmin, _) = body_geom(&mut rt, body).await;
    assert!(
        (gated_vol - BOX_VOL).abs() < 1.0 && (gated_vol - filleted_vol).abs() > 5.0,
        "the fillet was NOT re-bound against the un-moved box — got {gated_vol}"
    );
    assert!(
        (gmin[0] - unmoved_min[0]).abs() < 1e-3,
        "the move itself is suppressed (box back where it was), {unmoved_min:?} vs {gmin:?}"
    );

    // Un-suppressing seeds the gate again (the placement re-appears).
    assert!(rt.undo().is_some(), "undo the suppression");
    assert!(
        rt.repair_items().iter().all(|i| !i.seeded),
        "the suppression's seed rides its own undo entry"
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "regen after unsuppress");
    assert_eq!(snap.repair_summary.needs_repair_count, 0);

    wm.shutdown().await;
    eprintln!("Suppression gate PASS: gated {gated_vol:.1} vs healthy {filleted_vol:.1}");
}

// ─────────────────────────────────────────────────────────────────────────────
// (8) WP-FIX W4 / VF-B5a — a FACE-HOSTED SKETCH is downstream of the move
// ─────────────────────────────────────────────────────────────────────────────

const SKETCH_HOST_REC: u128 = 0xE00;
const EXTRUDE_HOST: u128 = 0xE01;
const HOST_FACE_EL: &str = "el_host_top";

/// A rect sketch glued to a face of `body` — the shape `add_sketch_on_face`
/// authors: a typed `ElementRef` host plus a frame frozen on that face.
fn hosted_rect_sketch(sid: SketchId, base: u128, body: BodyId, el: &str, z: f64) -> Sketch {
    let src = rect_sketch(sid, base, 2.0, 2.0, 6.0, 6.0);
    let mut sk = Sketch::new(
        sid,
        "OnFace",
        SketchAttachment::HostFace {
            face: ElementRef {
                primary: Some(PrimaryRef {
                    body,
                    element: ElementId::new(el),
                    kind: ElementKind::Face,
                    extra: Default::default(),
                }),
                intent: None,
                anchor: Some(AnchorIntent {
                    world_point: Vec3::new_unchecked(0.0, 0.0, z),
                    surface_uv: None,
                    local_frame: None,
                    adjacency_hint: None,
                    extra: Default::default(),
                }),
                extra: Default::default(),
            },
            projected_boundary_version: 1,
        },
    );
    // The frozen frame: the host face's own plane (the box's top cap).
    sk.plane = onecad_core::sketch::plane_from_point_normal(
        Vec3::new_unchecked(0.0, 0.0, z),
        Vec3::new_unchecked(0.0, 0.0, 1.0),
    );
    for e in src.entities() {
        sk.add_entity(e.clone()).expect("hosted entity");
    }
    for c in src.constraints() {
        sk.add_constraint(c.clone()).expect("hosted constraint");
    }
    sk
}

/// **VF-B5a.** A sketch attached to a FACE of a moved body — and everything built
/// from it — must be gated when that move is edited.
///
/// RED (pre-W4): `Operation::derive_inputs` returned NOTHING for a `Sketch` op, so
/// the record declared no dependency on its host body at all. `transform_gate_items`
/// intersects `inputs.bodies` with the transform's moved set, found nothing, and
/// seeded ZERO items: the sketch's frozen frame silently stayed at the OLD placement
/// while the box moved, and the body extruded from it published at the stale
/// location with no `NeedsRepair` anywhere — the exact silent-wrong-geometry the
/// gate exists to kill.
///
/// This drives the REAL stamp path: `finish_sketch` → `upsert_sketch_record` mints
/// the record with `host_face` from the live attachment.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn editing_a_transform_gates_a_sketch_hosted_on_the_moved_face() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // steps: 0 Sketch · 1 Extrude(box) · 2 Move · 3 Sketch(on the moved face) · 4 Extrude
    let body = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let _ = published(&regen_all(&mut rt).await, "box");
    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![body], [15.0, 0.0, 0.0], None, false),
    );
    let _ = published(&regen_all(&mut rt).await, "move");

    let host_sid = SketchId(Uuid::from_u128(SKETCH_HOST_REC));
    rt.apply(EditCommand::AddSketch {
        sketch: hosted_rect_sketch(host_sid, SKETCH_HOST_REC << 8, body, HOST_FACE_EL, 25.0),
    })
    .expect("AddSketch on the moved face");
    rt.finish_sketch(host_sid)
        .await
        .expect("finishSketch mints the Sketch record");
    add_op(&mut rt, extrude_record(EXTRUDE_HOST, host_sid, 4.0));
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "hosted extrude");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "HEALTHY: the hosted sketch and its extrude resolve honestly"
    );
    assert_eq!(snap.bodies.len(), 2, "the box plus the hosted body");

    // ── EDIT the move ⇒ the gate must fire ON THE SKETCH STEP ────────────────
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(OP_MOVE)),
        op: transform_op(vec![body], [15.0, 40.0, 0.0], None, false),
    })
    .expect("UpdateOperationParams on the TransformBody");

    let gated: Vec<_> = rt.repair_items().iter().filter(|i| i.seeded).collect();
    assert!(
        !gated.is_empty(),
        "editing the Move gates the face-hosted sketch (VF-B5a: this was EMPTY — \
         zero NeedsRepair while the geometry moved under the sketch)"
    );
    assert!(
        gated.iter().any(|i| i.step_index == 3),
        "the SKETCH step (3) is gated, got {:?}",
        gated.iter().map(|i| i.step_index).collect::<Vec<_>>()
    );
    assert!(
        gated
            .iter()
            .any(|i| i.element_id.as_ref().map(ElementId::as_str) == Some(HOST_FACE_EL)),
        "the gate names the sketch's HOST FACE ref"
    );
    let ceiling = gated.iter().map(|i| i.step_index).min().unwrap();
    assert_eq!(
        ceiling, 3,
        "the regen ceiling stops BELOW the sketch step, so neither the sketch nor \
         the body extruded from it can execute against a stale frozen frame"
    );

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "regen after the transform edit");
    assert!(
        snap.step_states
            .iter()
            .any(|(i, st)| *i == 3 && *st == StepState::NeedsRepair),
        "the hosted-sketch step is NeedsRepair, got {:?}",
        snap.step_states
    );
    assert_eq!(
        snap.bodies.len(),
        1,
        "publish ≤ m−1: only the moved box — the hosted body was NOT re-published \
         against the stale frame"
    );

    // ── UNDO ⇒ the gate lifts and the hosted body comes back ─────────────────
    assert!(rt.undo().is_some(), "undo the transform edit");
    assert!(
        rt.repair_items().iter().all(|i| !i.seeded),
        "the seed rides the SAME undo entry as the edit that planted it"
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "regen after undo");
    assert_eq!(snap.repair_summary.needs_repair_count, 0);
    assert_eq!(snap.bodies.len(), 2, "the hosted body is back");

    wm.shutdown().await;
    eprintln!("Host-face gate PASS: sketch step 3 gated, ceiling {ceiling}");
}

/// The LEGACY document shape: the `Sketch` record carries NO `host_face` param
/// (it predates the field), so `derive_inputs` still declares nothing — yet the
/// gate must cover it via the document-level attachment bridge. The record is
/// never rewritten: `inputs` is inside the golden prefix hash, so backfilling it
/// would move every legacy document's hash and bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_hosted_sketch_record_is_gated_through_its_attachment() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let body = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let _ = published(&regen_all(&mut rt).await, "box");
    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![body], [15.0, 0.0, 0.0], None, false),
    );
    let _ = published(&regen_all(&mut rt).await, "move");

    // The document knows the attachment…
    let host_sid = SketchId(Uuid::from_u128(SKETCH_HOST_REC));
    let sk = hosted_rect_sketch(host_sid, SKETCH_HOST_REC << 8, body, HOST_FACE_EL, 25.0);
    rt.apply(EditCommand::AddSketch { sketch: sk.clone() })
        .expect("AddSketch");
    // …but the RECORD is authored in the pre-W4 shape: no `host_face` param.
    let legacy = sketch_record(SKETCH_HOST_REC, &sk);
    let Operation::Known(KnownOperation::Sketch(p)) = &legacy.op else {
        panic!("Sketch record")
    };
    assert!(p.host_face.is_none(), "precondition: the legacy shape");
    assert!(
        legacy.op.derive_inputs().bodies.is_empty(),
        "precondition: it declares no dependency at all"
    );
    add_op(&mut rt, legacy);
    add_op(&mut rt, extrude_record(EXTRUDE_HOST, host_sid, 4.0));
    let _ = published(&regen_all(&mut rt).await, "legacy hosted extrude");

    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(OP_MOVE)),
        op: transform_op(vec![body], [15.0, 40.0, 0.0], None, false),
    })
    .expect("UpdateOperationParams");

    let gated: Vec<_> = rt.repair_items().iter().filter(|i| i.seeded).collect();
    assert!(
        gated.iter().any(|i| i.step_index == 3
            && i.element_id.as_ref().map(ElementId::as_str) == Some(HOST_FACE_EL)),
        "a legacy hosted sketch is gated through the attachment bridge, got {:?}",
        gated
    );

    wm.shutdown().await;
    eprintln!("Legacy attachment-bridge gate PASS");
}

/// **VF-B5b.** DELETING a `TransformBody` moves downstream geometry exactly like
/// suppressing it, so it must seed the SAME gate.
///
/// RED (pre-W4): `remove_operation` never touched the repair state — the one edit
/// that removes a placement outright was the one edit that re-bound every
/// downstream ref silently, while merely suppressing the same record gated it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_a_transform_seeds_the_gate_like_suppressing_it() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // steps: 0 Sketch · 1 Extrude · 2 Move · 3 Fillet(on the moved box)
    let body = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 0.0, 25.0).await;
    let _ = published(&regen_all(&mut rt).await, "box");
    add_op(
        &mut rt,
        transform_record(OP_MOVE, vec![body], [15.0, 0.0, 0.0], None, false),
    );
    let _ = published(&regen_all(&mut rt).await, "move");
    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("MESH1 validates");
    let anchor = vertical_edge_anchor(&view, &mesh);
    add_op(
        &mut rt,
        fillet_record(body, ElementId::new("el_move_edge"), anchor, 3.0),
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "fillet on the moved box");
    assert_eq!(snap.repair_summary.needs_repair_count, 0, "healthy first");
    let (filleted_vol, _, _) = body_geom(&mut rt, body).await;

    // ── DELETE the Move ⇒ the gate fires at the POST-removal index ───────────
    rt.apply(EditCommand::RemoveOperation {
        record: RecordId(Uuid::from_u128(OP_MOVE)),
    })
    .expect("RemoveOperation on the TransformBody");
    let gated: Vec<_> = rt.repair_items().iter().filter(|i| i.seeded).collect();
    assert!(
        !gated.is_empty(),
        "deleting a TransformBody seeds the edit-safety gate (VF-B5b: this was \
         EMPTY — the delete path bypassed the gate entirely)"
    );
    assert!(
        gated.iter().all(|i| i.step_index == 2),
        "the fillet shifted from step 3 to step 2 when the Move was removed; the \
         seeded step_index must address the POST-removal timeline, got {:?}",
        gated.iter().map(|i| i.step_index).collect::<Vec<_>>()
    );
    assert!(
        gated
            .iter()
            .any(|i| i.element_id.as_ref().map(ElementId::as_str) == Some("el_move_edge")),
        "the gate names the fillet's edge ref"
    );

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "regen after the delete");
    assert!(
        snap.step_states
            .iter()
            .any(|(i, st)| *i == 2 && *st == StepState::NeedsRepair),
        "the fillet step is NeedsRepair, got {:?}",
        snap.step_states
    );
    let (gated_vol, _, _) = body_geom(&mut rt, body).await;
    assert!(
        (gated_vol - BOX_VOL).abs() < 1.0 && (gated_vol - filleted_vol).abs() > 5.0,
        "publish ≤ m−1: the plain box, NOT a re-bound fillet — got {gated_vol}"
    );

    // ── UNDO restores the timeline AND the repair state ──────────────────────
    assert!(rt.undo().is_some(), "undo the delete");
    assert!(
        rt.repair_items().iter().all(|i| !i.seeded),
        "the delete's seeds ride the delete's own undo entry"
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "regen after undo");
    assert_eq!(snap.repair_summary.needs_repair_count, 0);
    let (restored_vol, _, _) = body_geom(&mut rt, body).await;
    assert!(
        (restored_vol - filleted_vol).abs() < 1.0,
        "the fillet is back: {filleted_vol} vs {restored_vol}"
    );

    wm.shutdown().await;
    eprintln!("Delete gate PASS: healthy {filleted_vol:.1} → gated {gated_vol:.1}");
}
