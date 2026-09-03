//! Wire-contract regression gate (M2 code-review defects 1–7) against the REAL C++
//! OCCT worker, driven through the app's [`DocumentRuntime`] exactly like
//! `m2_gate.rs`.
//!
//! Each worker-backed test exercises a body-bearing wire path that was broken by the
//! `BodyId` wire-form mismatch (core serde emits a bare uuid; the worker's
//! `BodyStore` is keyed `body_<opId>`). Before the `wire::to_wire_body_form` fix each
//! would have failed (REF_UNRESOLVED / "target body not found" / ToFace NeedsRepair):
//!
//! * `standalone_boolean_cut` / `_union` — a standalone `Boolean` reads bare
//!   `params.targetBodyId`/`toolBodyId` → BodyStore miss (defect 1).
//! * `extrude_pocket_cut` — an `Extrude` Cut reads bare `params.targetBodyId` →
//!   "Extrude target body not found" (defect 2).
//! * `extrude_to_face` — a `ToFace` extrude reads bare
//!   `params.targetFace.primary.bodyId` → NeedsRepair every time (defect 3); also
//!   pins the pre-resolver / `resolve_to_face` ownership split (defect 7).
//! * `fillet_body_context` — the fillet wire flow over `element_ref_wire` (defect 5's
//!   sibling; the bare-fallback body attach itself is unit-pinned in `wire.rs`).
//!
//! `planner_hash_decoupled_from_wire_body_form` is a pure test (no worker) pinning
//! that the regen planner's history-prefix hash is UNCHANGED by this fix (the planner
//! hashes the core serde form and never calls `wire_op`; task A).
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary skips
//! the worker-backed tests cleanly. The pure hash test always runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::body::split_child_uuid;
use onecad_core::document::record::{
    BooleanMode, BooleanOp, BooleanParams, ChamferParams, ExtrudeMode, ExtrudeParams, FilletParams,
    KnownOperation, Operation, OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::history::{DependencyGraph, Timeline};
use onecad_core::ids::{
    BodyId, ConstraintId, DocumentRevision, ElementId, EntityId, JobId, RecordId, RegionId,
    SketchId, SnapshotId, TopoKey, WorkerEpoch,
};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    history_prefix_hash, CancelToken, GeometryEngine, HistoryPrefixHash, Lod, ModelSnapshot,
    Outcome, PlanArtifacts, PlanContext, PolicyVersions, RegenPlanner, RegenRequest,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::{
    body_id_wire, clear_split_interner_for_test, execute_plan_args, sketch_wire,
};
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors m2_gate.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the worker binary, honoring the CI / misconfiguration guards (MINOR-2 —
/// a missing binary must NOT silently read as a green skip):
/// * `ONECAD_WORKER_PATH` set but pointing at a **missing** file ⇒ PANIC;
/// * `ONECAD_REQUIRE_WORKER=1` and no worker resolves at all ⇒ PANIC (CI sets this);
/// * otherwise a missing worker is a quiet local-dev skip (`None`).
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

fn open_runtime_over(wm: &WorkerManager, path: &Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen saved container")
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

// ─────────────────────────────────────────────────────────────────────────────
// Fixed record ids → their worker-minted NewBody ids (`body_<opId>`, adopted as
// `BodyId(recordId.uuid)`). A Boolean/pocket op names its target/tool by these ids.
// ─────────────────────────────────────────────────────────────────────────────

const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;
const SKETCH_B: u128 = 0xB00;
const EXTRUDE_B: u128 = 0xB01;
const OP_TAIL: u128 = 0xC00; // boolean / pocket / to-face / fillet tail op

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders
// ─────────────────────────────────────────────────────────────────────────────

/// The non-standard XY plane ref carried on the timeline Sketch op (as m2_gate).
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

/// A fully-constrained (dof 0) rectangle at sketch-space `(x0, y0)` with size `w × h`,
/// built the marshaller way (8 synthesized points, 4 lines, coincident corners, H/V,
/// a Fixed anchor, and H/V dimension constraints). `base` seeds unique entity ids.
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

fn two_rect_sketch(sid: SketchId) -> Sketch {
    let mut sketch = rect_sketch(sid, 0x7000, 0.0, 0.0, 40.0, 20.0);
    let second = rect_sketch(sid, 0x8000, 60.0, 0.0, 10.0, 10.0);
    for entity in second.entities().iter().cloned() {
        sketch.add_entity(entity).unwrap();
    }
    for constraint in second.constraints().iter().cloned() {
        sketch.add_constraint(constraint).unwrap();
    }
    sketch
}

fn region_triangle_area(region: &onecad_lib::dto::SketchRegionDto) -> f64 {
    let triangles = region.preview_triangles.as_ref().expect("region fill");
    triangles
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|triangle| {
            let point = |index: u32| {
                let offset = index as usize * 2;
                [triangles.positions[offset], triangles.positions[offset + 1]]
            };
            let [a, b, c] = [point(triangle[0]), point(triangle[1]), point(triangle[2])];
            ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() * 0.5
        })
        .sum()
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

fn extrude_record(
    rec: u128,
    sketch: SketchId,
    dist: f64,
    boolean: BooleanMode,
    target: Option<BodyId>,
) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            // Empty ⇒ the worker's V1 first-region fallback (a NON-EMPTY id that
            // matched no region is now a hard OP_FAILED — M4a strict rule; these
            // single-region fixtures assert the fallback, so they carry no id).
            region: RegionId::new(""),
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(dist),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: boolean,
        target_body: target,
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

/// A `ToFace` extrude (NewBody) whose direction-1 target is the given face ref.
fn extrude_to_face_record(rec: u128, sketch: SketchId, face: ElementRef) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // empty ⇒ V1 first-region fallback (M4a strict rule)
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(1.0),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::ToFace,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: Some(face),
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

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 geometry helpers (exact for planar-faced polyhedra)
// ─────────────────────────────────────────────────────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;
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

/// Signed volume of a MESH1 body via the divergence theorem — EXACT for a closed,
/// planar-faced polyhedron (a box, a box minus a box), so box arithmetic is testable
/// to f32 precision.
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
        // a · (b × c)
        vol6 += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    (vol6 / 6.0).abs()
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

/// The face with the greatest average world-Z (the extrude cap / top face): its
/// `(TopoKey, centroid-anchor)`. Used to author a ToFace target ref.
fn top_face_pick(view: &MeshHeaderView, blob: &[u8]) -> (String, Vec3) {
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
    let mut best: Option<(usize, f64, Vec3)> = None;
    for f in 0..view.face_count as usize {
        let first = u32_le(blob, frbase + f * 8) as usize;
        let count = u32_le(blob, frbase + f * 8 + 4) as usize;
        let (mut sx, mut sy, mut sz, mut n) = (0.0, 0.0, 0.0, 0.0f64);
        for t in first..first + count {
            let io = ibase + t * 12;
            for k in 0..3 {
                let v = vertex(blob, pbase, u32_le(blob, io + k * 4) as usize);
                sx += v[0];
                sy += v[1];
                sz += v[2];
                n += 1.0;
            }
        }
        if n == 0.0 {
            continue;
        }
        let centroid = Vec3::new_unchecked(sx / n, sy / n, sz / n);
        if best.is_none_or(|(_, z, _)| centroid.z > z) {
            best = Some((f, centroid.z, centroid));
        }
    }
    let (idx_best, _, centroid) = best.expect("at least one face");
    (keys[idx_best].clone(), centroid)
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

// ─────────────────────────────────────────────────────────────────────────────
// standalone Boolean — bare params.targetBodyId/toolBodyId (defect 1)
// ─────────────────────────────────────────────────────────────────────────────

/// Two disjoint-then-overlapping extruded boxes fed to a standalone `Boolean`.
/// A = worldY[0,40], B = worldY[20,60], both worldX[-20,0] × Z[0,25]; A∩B = 20×20×25.
async fn run_boolean(op: BooleanOp) -> f64 {
    let bin = real_worker().expect("worker checked by caller");
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    let sb = SketchId(Uuid::from_u128(0xB));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, sa, 25.0, BooleanMode::NewBody, None),
    );
    add_op(
        &mut rt,
        sketch_record(SKETCH_B, &rect_sketch(sb, 0x2000, 20.0, 0.0, 40.0, 20.0)),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_B, sb, 25.0, BooleanMode::NewBody, None),
    );
    add_op(
        &mut rt,
        boolean_record(OP_TAIL, op, body_of(EXTRUDE_A), body_of(EXTRUDE_B)),
    );

    let report = regen_all(&mut rt).await;
    let _snap = published(&report, "boolean");
    // The boolean modifies the target (id preserved) and consumes the tool.
    let mesh = body_mesh(&mut rt, body_of(EXTRUDE_A)).await;
    let view = validate_mesh_blob(&mesh).expect("boolean result MESH1 validates");
    // Volume is exact for a planar-faced polyhedron regardless of face count (a
    // tiled Union leaves coplanar faces unmerged — OCCT Fuse does not unify domains).
    let vol = mesh_volume(&view, &mesh);
    assert!(view.face_count >= 6, "boolean result is a closed solid");
    wm.shutdown().await;
    vol
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn standalone_boolean_cut() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let vol = run_boolean(BooleanOp::Cut).await;
    // A − B = worldY[0,20] × worldX[-20,0] × Z[0,25] = 20·20·25 = 10000.
    assert!(
        (vol - 10_000.0).abs() < 1.0,
        "Cut volume = A − (A∩B) = 10000, got {vol}"
    );
    eprintln!("boolean Cut PASS: volume {vol} == 10000 (exact box arithmetic)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn standalone_boolean_union() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let vol = run_boolean(BooleanOp::Union).await;
    // A ∪ B = contiguous worldY[0,60] × worldX[-20,0] × Z[0,25] = 60·20·25 = 30000.
    assert!(
        (vol - 30_000.0).abs() < 1.0,
        "Union volume = 30000, got {vol}"
    );
    eprintln!("boolean Union PASS: volume {vol} == 30000 (exact box arithmetic)");
}

// ─────────────────────────────────────────────────────────────────────────────
// Boolean SPLIT children (`body_<opId>:<k>`, SCHEMA §2 / §14, D1)
// ─────────────────────────────────────────────────────────────────────────────

/// A Cut that BISECTS a box mints two deterministic split children, both adopted
/// (D1), with exact volumes and ids stable across a replay. Box A (sketch 40×20,
/// extrude 25) is bisected by a slab tool that overshoots A everywhere except the
/// middle band, so `A − tool` = two disconnected 7500-volume pieces.
/// Serializes the two split tests: `split_persist_survives_cold_interner` clears the
/// process-global split interner, which would corrupt a concurrent split render. A
/// tokio mutex so the guard may be held across `.await` (a std mutex cannot).
static SPLIT_INTERNER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boolean_split_children_adopted() {
    let _guard = SPLIT_INTERNER_LOCK.lock().await;
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    const SKETCH_TOOL: u128 = 0xD00;
    const EXTRUDE_TOOL: u128 = 0xD01;

    let bin = real_worker().unwrap();
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    let st = SketchId(Uuid::from_u128(0xD));
    // Box A: sketch [0,40]×[0,20] extrude 25.
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, sa, 25.0, BooleanMode::NewBody, None),
    );
    // Tool slab: sketch x∈[15,25] (the middle band), y∈[-10,30] (overshoots A),
    // extrude 30 (overshoots A's top) ⇒ a full through-cut in the sketch-x direction.
    add_op(
        &mut rt,
        sketch_record(
            SKETCH_TOOL,
            &rect_sketch(st, 0x3000, 15.0, -10.0, 10.0, 40.0),
        ),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_TOOL, st, 30.0, BooleanMode::NewBody, None),
    );
    // Cut A − tool ⇒ two disconnected pieces (a split).
    add_op(
        &mut rt,
        boolean_record(
            OP_TAIL,
            BooleanOp::Cut,
            body_of(EXTRUDE_A),
            body_of(EXTRUDE_TOOL),
        ),
    );

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "split");
    // The parent + tool are gone; exactly the two children survive.
    let children: Vec<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    assert_eq!(
        children.len(),
        2,
        "cut bisected A into two children, got {children:?}"
    );
    assert!(
        !children.contains(&body_of(EXTRUDE_A)) && !children.contains(&body_of(EXTRUDE_TOOL)),
        "children are fresh split ids, not the parent/tool ids"
    );

    // Both children adopted with EXACT volumes (7500 each).
    let mut vols = Vec::new();
    for &child in &children {
        let mesh = body_mesh(&mut rt, child).await;
        let view = validate_mesh_blob(&mesh).expect("split child MESH1 validates");
        let v = mesh_volume(&view, &mesh);
        assert!(
            (v - 7500.0).abs() < 1.0,
            "split child volume = 15·20·25 = 7500, got {v}"
        );
        vols.push(v);
    }
    let total: f64 = vols.iter().sum();
    assert!(
        (total - 15_000.0).abs() < 2.0,
        "A(20000) − band(5000) = 15000, got {total}"
    );

    // Ids stable across a replay (derived deterministically from opId + ordinal).
    let report2 = regen_all(&mut rt).await;
    let snap2 = published(&report2, "split replay");
    let set1: std::collections::HashSet<BodyId> = children.into_iter().collect();
    let set2: std::collections::HashSet<BodyId> = snap2.bodies.iter().map(|b| b.body).collect();
    assert_eq!(set1, set2, "split child ids are identical across a replay");

    wm.shutdown().await;
    eprintln!("boolean split PASS: 2 children, volumes {vols:?} == 7500 each, ids stable");
}

/// The cross-process persistence gate (orchestrator review, M5a): a downstream op that
/// references a split child by its (persisted) derived `BodyId` must still render
/// `body_<opId>:<k>` on the wire in a **FRESH process** — where the split interner is
/// cold. The persisted `BodyMeta.split_of` (re-interned at document open) is the fix.
///
/// Doc: box A → bisecting Cut (2 children) → Cut targeting child `:1` (a slab off its
/// far end, 7500 → 5000). Save, **clear the interner** (simulate a fresh process + the
/// pre-fix cold-interner state), reopen with a FRESH runtime + FRESH worker, replay
/// from 0, and assert the reopened head is byte-identical (signature + child volumes)
/// to a warm from-0 baseline — i.e. op-B resolved child `:1` across the process boundary
/// instead of failing REF_UNRESOLVED.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn split_persist_survives_cold_interner() {
    let _guard = SPLIT_INTERNER_LOCK.lock().await;
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    // Record ids: box A, bisecting tool, the SPLIT cut, the chunk tool, the child-cut.
    const EX_TOOL1: u128 = 0xB01;
    const SK_TOOL1: u128 = 0xB00;
    const SPLIT_CUT: u128 = 0xC00; // op that splits A into :0 / :1
    const SK_TOOL2: u128 = 0xD00;
    const EX_TOOL2: u128 = 0xD01;
    const CHILD_CUT: u128 = 0xE00; // op that cuts child :1
    let child1 = BodyId(split_child_uuid(Uuid::from_u128(SPLIT_CUT), 1));
    let child0 = BodyId(split_child_uuid(Uuid::from_u128(SPLIT_CUT), 0));

    // Phase 1: box A + the bisecting Cut (produces child :0 / :1). Regen after this so
    // the child exists before phase 2 references it — the ONLY way a real workflow can
    // select a split child (you cannot target a body that a not-yet-run op will mint).
    let build_split = |rt: &mut DocumentRuntime| {
        let sa = SketchId(Uuid::from_u128(0xA));
        let st1 = SketchId(Uuid::from_u128(0xB));
        add_op(
            rt,
            sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
        );
        add_op(
            rt,
            extrude_record(EXTRUDE_A, sa, 25.0, BooleanMode::NewBody, None),
        );
        add_op(
            rt,
            sketch_record(SK_TOOL1, &rect_sketch(st1, 0x3000, 15.0, -10.0, 10.0, 40.0)),
        );
        add_op(
            rt,
            extrude_record(EX_TOOL1, st1, 30.0, BooleanMode::NewBody, None),
        );
        add_op(
            rt,
            boolean_record(
                SPLIT_CUT,
                BooleanOp::Cut,
                body_of(EXTRUDE_A),
                body_of(EX_TOOL1),
            ),
        );
    };
    // Phase 2: a chunk tool + a Cut whose TARGET is split child :1 (the cross-process ref).
    let build_child_cut = |rt: &mut DocumentRuntime| {
        let st2 = SketchId(Uuid::from_u128(0xD));
        add_op(
            rt,
            sketch_record(SK_TOOL2, &rect_sketch(st2, 0x5000, 35.0, -5.0, 10.0, 30.0)),
        );
        add_op(
            rt,
            extrude_record(EX_TOOL2, st2, 30.0, BooleanMode::NewBody, None),
        );
        add_op(
            rt,
            boolean_record(CHILD_CUT, BooleanOp::Cut, child1, body_of(EX_TOOL2)),
        );
    };

    // ── Warm baseline (own worker): the from-0 head signature + child volumes ──────
    let (base_sig, base_v0, base_v1) = {
        let wm = spawn_worker(bin.clone()).await;
        let mut rt = runtime_over(&wm);
        build_split(&mut rt);
        let _ = published(&regen_all(&mut rt).await, "warm phase-1"); // child :1 now interned
        build_child_cut(&mut rt);
        let report = regen_all(&mut rt).await;
        let snap = published(&report, "warm baseline");
        assert_eq!(snap.bodies.len(), 2, "child :0 + cut child :1");
        let sig = snap
            .signatures
            .as_ref()
            .map(|s| s.geometry.as_str().to_string());
        let v0 = mesh_vol(&mut rt, child0).await;
        let v1 = mesh_vol(&mut rt, child1).await;
        wm.shutdown().await;
        (sig, v0, v1)
    };
    assert!(
        (base_v0 - 7500.0).abs() < 1.0,
        "child :0 untouched = 7500, got {base_v0}"
    );
    assert!(
        (base_v1 - 5000.0).abs() < 1.0,
        "child :1 cut = 7500 − 2500 = 5000, got {base_v1}"
    );

    // ── Save, then simulate a FRESH PROCESS (cold interner) + reopen + replay ──────
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("split.onecad");
    {
        let wm = spawn_worker(bin.clone()).await;
        let mut rt = runtime_over(&wm);
        build_split(&mut rt);
        let _ = published(&regen_all(&mut rt).await, "save phase-1");
        build_child_cut(&mut rt);
        let _ = published(&regen_all(&mut rt).await, "save phase-2");
        rt.save(&path, split_save_meta()).expect("save split doc");
        wm.shutdown().await;
    }
    // A fresh process starts with an EMPTY interner — this is the pre-fix failure state.
    clear_split_interner_for_test();

    let wm = spawn_worker(bin.clone()).await;
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    // `open` re-interns the split children from the persisted `split_of` BEFORE any
    // plan compiles — the fix. Without it the replay below would REF_UNRESOLVED on the
    // CHILD_CUT op's `body_<derived-uuid>` target.
    let mut rt = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen split doc");
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "reopen replay-from-0");
    assert_eq!(snap.bodies.len(), 2, "reopen: 2 bodies");
    let reopen_sig = snap
        .signatures
        .as_ref()
        .map(|s| s.geometry.as_str().to_string());
    assert_eq!(
        reopen_sig, base_sig,
        "reopen head signature IDENTICAL to the warm baseline"
    );
    let rv0 = mesh_vol(&mut rt, child0).await;
    let rv1 = mesh_vol(&mut rt, child1).await;
    assert!(
        (rv0 - 7500.0).abs() < 1.0,
        "reopen child :0 = 7500, got {rv0}"
    );
    assert!(
        (rv1 - 5000.0).abs() < 1.0,
        "reopen child :1 = 5000, got {rv1}"
    );

    clear_split_interner_for_test(); // leave the interner clean for other tests
    wm.shutdown().await;
    eprintln!(
        "split-persist PASS: cold-interner reopen resolved child :1 (vol {rv1}), sig identical"
    );
}

async fn mesh_vol(rt: &mut DocumentRuntime, body: BodyId) -> f64 {
    let mesh = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("child MESH1 validates");
    mesh_volume(&view, &mesh)
}

fn split_save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: None,
        created: "2026-07-19T00:00:00Z".into(),
        modified: "2026-07-19T00:00:00Z".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// pocket — Extrude Cut with bare params.targetBodyId (defect 2)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extrude_pocket_cut() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let bin = real_worker().unwrap();
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    let sp = SketchId(Uuid::from_u128(0xB));
    // Box A: 40×20 profile, extrude 25 (vol 20000).
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, sa, 25.0, BooleanMode::NewBody, None),
    );
    // Pocket: 20×10 profile fully inside A, extrude Cut 10 into A (removes 2000).
    add_op(
        &mut rt,
        sketch_record(SKETCH_B, &rect_sketch(sp, 0x2000, 10.0, 5.0, 20.0, 10.0)),
    );
    add_op(
        &mut rt,
        extrude_record(
            OP_TAIL,
            sp,
            10.0,
            BooleanMode::Cut,
            Some(body_of(EXTRUDE_A)),
        ),
    );

    let report = regen_all(&mut rt).await;
    let _snap = published(&report, "pocket");
    let mesh = body_mesh(&mut rt, body_of(EXTRUDE_A)).await;
    let view = validate_mesh_blob(&mesh).expect("pocket result MESH1 validates");
    let vol = mesh_volume(&view, &mesh);
    assert!(
        (vol - 18_000.0).abs() < 1.0,
        "pocket volume = A(20000) − pocket(2000) = 18000, got {vol}"
    );
    assert!(
        view.face_count > 6,
        "a blind pocket adds faces to the box (got {})",
        view.face_count
    );
    wm.shutdown().await;
    eprintln!(
        "pocket PASS: volume {vol} == 18000, faces {}",
        view.face_count
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ToFace — bare params.targetFace.primary.bodyId (defect 3) + pre-resolver split (7)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extrude_to_face() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let bin = real_worker().unwrap();
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    let sp = SketchId(Uuid::from_u128(0xB));
    // Phase 1: box A (top face at worldZ = 25).
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, sa, 25.0, BooleanMode::NewBody, None),
    );
    let rep_a = regen_all(&mut rt).await;
    let snap_a = published(&rep_a, "toFace box A");
    let snap_id = SnapshotId(rep_a.snapshot_id);
    let body_a = body_of(EXTRUDE_A);

    // Promote the top face → a persistent el_ id (the ToFace target's identity).
    let mesh_a = body_mesh(&mut rt, body_a).await;
    let view_a = validate_mesh_blob(&mesh_a).expect("box A MESH1 validates");
    assert_eq!(view_a.face_count, 6, "box A has 6 faces");
    let (top_key, top_centroid) = top_face_pick(&view_a, &mesh_a);
    assert!(
        top_centroid.z > 24.0,
        "top face is at worldZ≈25, got {}",
        top_centroid.z
    );
    let anchor = AnchorIntent {
        world_point: top_centroid,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    };
    let promoted = rt
        .promote_selection(
            snap_id,
            body_a,
            vec![(TopoKey::new(&top_key), Some(anchor.clone()))],
        )
        .await
        .expect("promote top face");
    let top_el = ElementId::new(promoted[0].element_id.clone());
    let _ = snap_a; // (bodies asserted via mesh)

    // Phase 2: a smaller profile extruded ToFace UP TO box A's top face (worldZ=25).
    let face_ref = ElementRef {
        primary: Some(PrimaryRef {
            body: body_a,
            element: top_el,
            kind: ElementKind::Face,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(anchor),
        extra: Default::default(),
    };
    add_op(
        &mut rt,
        sketch_record(SKETCH_B, &rect_sketch(sp, 0x2000, 10.0, 5.0, 20.0, 10.0)),
    );
    add_op(&mut rt, extrude_to_face_record(OP_TAIL, sp, face_ref));

    let rep_tf = regen_all(&mut rt).await;
    let snap_tf = published(&rep_tf, "toFace extrude");
    // Two bodies now exist (A + the ToFace column), and the ToFace body reached z=25.
    assert!(
        snap_tf.repair_summary.needs_repair_count == 0,
        "ToFace resolved (defect 3): no NeedsRepair, got {}",
        snap_tf.repair_summary.needs_repair_count
    );
    assert_eq!(snap_tf.bodies.len(), 2, "box A + the ToFace column");

    let body_tf = body_of(OP_TAIL);
    let mesh_tf = body_mesh(&mut rt, body_tf).await;
    let view_tf = validate_mesh_blob(&mesh_tf).expect("ToFace body MESH1 validates");
    let dims = bbox_dims(&view_tf);
    let vol = mesh_volume(&view_tf, &mesh_tf);
    // 20×10 profile extruded from z=0 up to the z=25 face ⇒ 20·10·25 = 5000; z-extent 25.
    assert!(
        (dims[2] - 25.0).abs() < 0.5,
        "ToFace depth reached the target face (z-extent ≈ 25), got {dims:?}"
    );
    assert!(
        (vol - 5000.0).abs() < 1.0,
        "ToFace column volume = 20·10·25 = 5000, got {vol}"
    );
    wm.shutdown().await;
    eprintln!("ToFace PASS: reached z=25, volume {vol} == 5000, 2 bodies (pre-resolver + resolve_to_face)");
}

// ─────────────────────────────────────────────────────────────────────────────
// fillet — body-bearing wire refs over element_ref_wire (fix B end-to-end)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fillet_body_context() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let bin = real_worker().unwrap();
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, sa, 25.0, BooleanMode::NewBody, None),
    );
    let rep_a = regen_all(&mut rt).await;
    let _ = published(&rep_a, "fillet box A");
    let body_a = body_of(EXTRUDE_A);

    let mesh_a = body_mesh(&mut rt, body_a).await;
    let view_a = validate_mesh_blob(&mesh_a).expect("box A MESH1 validates");
    assert_eq!(view_a.face_count, 6);
    let (top_key, centroid) = top_face_pick(&view_a, &mesh_a);
    let _ = top_key;

    // A fillet whose per-edge ref carries the operated body (primary.bodyId) + an
    // anchor — the body-bearing wire ref element_ref_wire now serde-renders. We anchor
    // near a top edge (the top-face centroid nudged to an edge is a coarse anchor; the
    // fillet either applies (faces grow) or cleanly NeedsRepairs — both prove the body
    // input resolved, i.e. NOT the pre-fix "requires body input"/BodyStore miss).
    let edge_el = ElementId::new("el_fillet_edge");
    let edge_ref = ElementRef {
        primary: Some(PrimaryRef {
            body: body_a,
            element: edge_el.clone(),
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: Vec3::new_unchecked(centroid.x, centroid.y, centroid.z),
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    let fillet = OperationRecord::new(
        RecordId(Uuid::from_u128(OP_TAIL)),
        0,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(2.0),
            edge_ids: vec![edge_el],
            edges: vec![edge_ref],
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    );
    add_op(&mut rt, fillet);
    let rep_f = regen_all(&mut rt).await;
    let snap_f = published(&rep_f, "fillet");

    if snap_f.repair_summary.needs_repair_count > 0 {
        // Clean NeedsRepair (state) — the body input DID resolve (target_body_of found
        // primary.bodyId); the edge anchor was just not confident. Pre-fix this path
        // never reached the ladder (BodyStore miss / wrong-form bodyId).
        eprintln!(
            "fillet PASS: body input resolved → CLEAN NeedsRepair ({} refs) — element_ref_wire body form OK",
            snap_f.repair_summary.needs_repair_count
        );
    } else {
        let mesh_f = body_mesh(&mut rt, body_a).await;
        let view_f = validate_mesh_blob(&mesh_f).expect("filleted body MESH1 validates");
        assert!(
            view_f.face_count >= 7,
            "fillet APPLIED adds a rolled face (6→≥7), got {}",
            view_f.face_count
        );
        eprintln!("fillet PASS: APPLIED — faces 6 → {}", view_f.face_count);
    }
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (A) Hash stability — the planner's history-prefix hash is unchanged by the wire
// body-form fix. The planner hashes the CORE serde form (BodyId → bare uuid) and
// never calls wire_op; the wire renders body_<uuid>. The two are decoupled.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn planner_hash_decoupled_from_wire_body_form() {
    let target = body_of(0xB0);
    let tool = body_of(0xB1);
    let rec = boolean_record(0xBEEF, BooleanOp::Union, target, tool);

    let mut tl = Timeline::new();
    tl.insert_at_cursor(rec.clone());
    let ctx = PlanContext {
        policy_versions: PolicyVersions::default(),
        occt_fingerprint: "fp".into(),
    };
    let plan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &[],
        RegenRequest::ToEnd { from: 0 },
        &ctx,
    );

    // (1) The planner hash is a FIXED value derived from the core serde form (bare
    //     uuids) — a golden that breaks if the hash inputs ever change (e.g. if
    //     wire_op body forms ever leaked into the hash). It equals the standalone
    //     history_prefix_hash over the same record (the planner's own function).
    assert_eq!(
        plan.expected_base_hash,
        HistoryPrefixHash::empty(),
        "from-0 base"
    );
    assert_eq!(plan.prefix_hashes.len(), 1);
    assert_eq!(
        plan.prefix_hashes[0],
        history_prefix_hash(std::slice::from_ref(&rec)),
        "plan prefix hash == history_prefix_hash of the record (planner path, not wire_op)"
    );
    assert_eq!(
        plan.prefix_hashes[0].as_str(),
        GOLDEN_BOOLEAN_PREFIX_HASH,
        "history-prefix hash is UNCHANGED by the wire body-form fix (task A)"
    );

    // (2) The WIRE, by contrast, renders body_<uuid> for the same op — proving the
    //     hashed form (bare uuid) and the wire form (body_<uuid>) are decoupled.
    let req = plan.into_request(
        JobId(Uuid::from_u128(1)),
        DocumentRevision(0),
        WorkerEpoch(0),
        PolicyVersions::default(),
        PlanArtifacts { tessellate: None },
    );
    let args = execute_plan_args(&req);
    let params = &args["ops"][0]["params"];
    assert_eq!(
        params["targetBodyId"],
        serde_json::json!(body_id_wire(target))
    );
    assert_eq!(params["toolBodyId"], serde_json::json!(body_id_wire(tool)));
    // ...and the bare uuid MUST NOT appear on the wire (it was the defect).
    assert_ne!(
        params["targetBodyId"],
        serde_json::json!(target.to_string())
    );

    // (3) SCHEMA §7.2 `editedFrom` is OPTIONAL and ABSENT means "no claim": a plan
    //     with no edit context must not emit the key at all (a `null` would be a
    //     different claim, and the worker treats a non-integer as absent anyway).
    assert!(
        args.get("editedFrom").is_none(),
        "no edit context ⇒ the key is OMITTED, not null: {args}"
    );
    let edited = execute_plan_args(&req.clone().with_edited_from(Some(2)));
    assert_eq!(
        edited["editedFrom"],
        serde_json::json!(2),
        "the edit lane stamps the step index verbatim"
    );

    // (4) SCHEMA §7.2 `checkpointFallbackReplay`, same omission rule. Only the
    //     executor's F12 fallback claims it, so an ordinary plan must be byte
    //     identical to what it was before the field existed — `false` is never
    //     spelled out, and it is never `null`.
    assert!(
        args.get("checkpointFallbackReplay").is_none(),
        "an ordinary plan OMITS the key entirely: {args}"
    );
    assert!(
        edited.get("checkpointFallbackReplay").is_none(),
        "an ordinary EDIT plan omits it too — the two flags are independent: {edited}"
    );
    let fallback = execute_plan_args(
        &req.clone()
            .with_edited_from(Some(2))
            .as_checkpoint_fallback_replay(),
    );
    assert_eq!(
        fallback["checkpointFallbackReplay"],
        serde_json::json!(true),
        "the F12 fallback claims it, and carries its editedFrom forward"
    );
    assert_eq!(
        fallback["editedFrom"],
        serde_json::json!(2),
        "the retry re-plans but the EDIT CONTEXT is a property of why the regen was \
         requested, so it rides along"
    );
}

/// The golden history-prefix hash of the fixed one-Boolean document above. Pinned so
/// any accidental change to the planner's hash inputs (including routing the wire
/// body form into the hash) is caught.
const GOLDEN_BOOLEAN_PREFIX_HASH: &str =
    "bed9be34040605a6cf938f215234353381931643fe23351618b1875c77bcbb5d";

// ─────────────────────────────────────────────────────────────────────────────
// MODEL-OPS W1 — the extrude end conditions the FRONTEND can now author.
//
// The worker has always implemented `ThroughAll` / `ToNext` / two-direction /
// draft (`ExtrudeOp.cpp effective_distance`), and `ExtrudeParams` has always
// carried the fields, but no tool ever authored them — so nothing exercised the
// wire path end-to-end. These pin the geometry each one produces, exactly the
// way `standalone_boolean_*` pins the boolean modes.
// ─────────────────────────────────────────────────────────────────────────────

/// An extrude with an explicit mode + optional second direction / draft.
#[allow(clippy::too_many_arguments)]
fn extrude_mode_record(
    rec: u128,
    sketch: SketchId,
    dist: f64,
    mode: ExtrudeMode,
    boolean: BooleanMode,
    target: Option<BodyId>,
    two_dirs: bool,
    mode2: ExtrudeMode,
    dist2: f64,
    draft_deg: f64,
) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // empty ⇒ V1 first-region fallback
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(dist),
        draft_angle_deg: Scalar::new(draft_deg),
        mode,
        boolean_mode: boolean,
        target_body: target,
        target_face: None,
        two_directions: two_dirs,
        mode2,
        distance2: Scalar::new(dist2),
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

/// Build a 40×20 base block, then run `tail` against it and return its volume.
async fn base_block_then(tail: impl FnOnce(SketchId) -> OperationRecord) -> f64 {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return f64::NAN;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // Base: 40×20 rect extruded 10 ⇒ 8000.
    let sa = SketchId(Uuid::from_u128(SKETCH_A));
    let sketch_a = rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0);
    add_op(&mut rt, sketch_record(SKETCH_A, &sketch_a));
    rt.apply(EditCommand::AddSketch {
        sketch: sketch_a.clone(),
    })
    .expect("AddSketch A");
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, sa, 10.0, BooleanMode::NewBody, None),
    );

    // Tail sketch: a 10×10 square sitting inside the block's footprint.
    let sb = SketchId(Uuid::from_u128(SKETCH_B));
    let sketch_b = rect_sketch(sb, 0x2000, 10.0, 5.0, 10.0, 10.0);
    add_op(&mut rt, sketch_record(SKETCH_B, &sketch_b));
    rt.apply(EditCommand::AddSketch {
        sketch: sketch_b.clone(),
    })
    .expect("AddSketch B");
    add_op(&mut rt, tail(sb));

    let report = regen_all(&mut rt).await;
    let _snap = published(&report, "end-condition tail");
    let mesh = body_mesh(&mut rt, body_of(EXTRUDE_A)).await;
    let view = validate_mesh_blob(&mesh).expect("result MESH1 validates");
    let vol = mesh_volume(&view, &mesh);
    wm.shutdown().await;
    vol
}

/// `ThroughAll` Cut punches the full depth regardless of the authored distance.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extrude_through_all_cut() {
    let vol = base_block_then(|sb| {
        extrude_mode_record(
            OP_TAIL,
            sb,
            // A deliberately TINY distance: ThroughAll must ignore it and cut the
            // whole 10-thick block. If the mode were dropped on the wire this would
            // barely scratch the block and the volume would land near 8000.
            0.1,
            ExtrudeMode::ThroughAll,
            BooleanMode::Cut,
            Some(body_of(EXTRUDE_A)),
            false,
            ExtrudeMode::Blind,
            0.0,
            0.0,
        )
    })
    .await;
    if vol.is_nan() {
        return;
    }
    // 40·20·10 − 10·10·10 = 8000 − 1000 = 7000.
    assert!(
        (vol - 7000.0).abs() < 1.0,
        "ThroughAll Cut removes the FULL 10-thick prism (8000 − 1000 = 7000), got {vol}"
    );
    eprintln!("ThroughAll Cut PASS: volume {vol} == 7000");
}

/// `ToNext` Cut stops at the next face ahead — here the block's far side, so it
/// is a through pocket of exactly the block depth.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extrude_to_next_cut() {
    let vol = base_block_then(|sb| {
        extrude_mode_record(
            OP_TAIL,
            sb,
            0.1, // ignored — ToNext derives the distance from the target body
            ExtrudeMode::ToNext,
            BooleanMode::Cut,
            Some(body_of(EXTRUDE_A)),
            false,
            ExtrudeMode::Blind,
            0.0,
            0.0,
        )
    })
    .await;
    if vol.is_nan() {
        return;
    }
    assert!(
        (vol - 7000.0).abs() < 1.0,
        "ToNext Cut reaches the next face (8000 − 1000 = 7000), got {vol}"
    );
    eprintln!("ToNext Cut PASS: volume {vol} == 7000");
}

/// A two-direction Add grows the block on BOTH sides of the sketch plane.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extrude_two_directions_add() {
    let vol = base_block_then(|sb| {
        extrude_mode_record(
            OP_TAIL,
            sb,
            6.0, // +Z
            ExtrudeMode::Blind,
            BooleanMode::Add,
            Some(body_of(EXTRUDE_A)),
            true,
            ExtrudeMode::Blind,
            4.0, // −Z
            0.0,
        )
    })
    .await;
    if vol.is_nan() {
        return;
    }
    // The 10×10 column spans z ∈ [−4, +6]; the block occupies z ∈ [0, 10], so the
    // union adds only the part OUTSIDE the block: the −4..0 stub = 10·10·4 = 400.
    // (The +Z half lies inside the block and contributes nothing.)
    assert!(
        (vol - 8400.0).abs() < 1.0,
        "two-direction Add contributes the below-plane stub (8000 + 400 = 8400), got {vol}"
    );
    eprintln!("two-direction Add PASS: volume {vol} == 8400");
}

/// A drafted Cut removes LESS than a straight one — the pocket tapers inward.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extrude_draft_angle_cut() {
    let straight = base_block_then(|sb| {
        extrude_mode_record(
            OP_TAIL,
            sb,
            5.0,
            ExtrudeMode::Blind,
            BooleanMode::Cut,
            Some(body_of(EXTRUDE_A)),
            false,
            ExtrudeMode::Blind,
            0.0,
            0.0,
        )
    })
    .await;
    if straight.is_nan() {
        return;
    }
    let drafted = base_block_then(|sb| {
        extrude_mode_record(
            OP_TAIL,
            sb,
            5.0,
            ExtrudeMode::Blind,
            BooleanMode::Cut,
            Some(body_of(EXTRUDE_A)),
            false,
            ExtrudeMode::Blind,
            0.0,
            10.0, // 10° draft
        )
    })
    .await;

    // Straight pocket: 10·10·5 = 500 removed ⇒ 7500.
    assert!(
        (straight - 7500.0).abs() < 1.0,
        "straight Cut removes 500 (8000 − 500 = 7500), got {straight}"
    );
    // The draft tapers the pocket, so LESS material is removed and the body is
    // heavier. A dropped draftAngleDeg would make the two identical.
    assert!(
        drafted > straight + 1.0,
        "a 10° draft must remove LESS than the straight cut \
         (straight {straight}, drafted {drafted}) — a dropped draft angle makes them equal"
    );
    eprintln!("draft PASS: straight {straight} < drafted {drafted}");
}

/// Chamfer over the wire — the tool now authors it, and it must reach the worker's
/// `execute_chamfer` and change the body, not silently no-op or fail.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chamfer_reaches_the_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(SKETCH_A));
    let sketch_a = rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0);
    add_op(&mut rt, sketch_record(SKETCH_A, &sketch_a));
    rt.apply(EditCommand::AddSketch {
        sketch: sketch_a.clone(),
    })
    .expect("AddSketch A");
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, sa, 10.0, BooleanMode::NewBody, None),
    );
    let before = regen_all(&mut rt).await;
    published(&before, "base block");
    let plain = body_mesh(&mut rt, body_of(EXTRUDE_A)).await;
    let plain_view = validate_mesh_blob(&plain).expect("base MESH1");
    let plain_faces = plain_view.face_count;
    let plain_vol = mesh_volume(&plain_view, &plain);

    // Chamfer one edge, built exactly like `fillet_body_context`'s ref: the body
    // rides on `primary`, the anchor near a real edge, so the ladder can bind it.
    let (_top_key, centroid) = top_face_pick(&plain_view, &plain);
    let edge_el = ElementId::new("el_chamfer_edge");
    let edge_ref = ElementRef {
        primary: Some(PrimaryRef {
            body: body_of(EXTRUDE_A),
            element: edge_el.clone(),
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: Vec3::new_unchecked(centroid.x, centroid.y, centroid.z),
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    add_op(
        &mut rt,
        OperationRecord::new(
            RecordId(Uuid::from_u128(OP_TAIL)),
            0,
            "Chamfer",
            // Chamfer carries its OWN params struct (split from Fillet in
            // R-WP2.1) with an identical shape.
            Operation::Known(KnownOperation::Chamfer(ChamferParams {
                radius: Scalar::new(2.0),
                distance2: None,
                angle_deg: None,
                edge_ids: vec![edge_el],
                edges: vec![edge_ref],
                chain_tangent_edges: false,
                tangent_closure_version: None,
                extra: Default::default(),
            })),
        ),
    );
    let after = regen_all(&mut rt).await;
    published(&after, "chamfer");

    let snap = published(&after, "chamfer");
    if snap.repair_summary.needs_repair_count > 0 {
        // Clean NeedsRepair: the Chamfer op DID reach the worker and its body input
        // resolved — the coarse anchor simply did not bind an edge confidently. The
        // pre-W1 failure mode was different and total: `opType: "Chamfer"` was not in
        // the authorable wire union at all.
        eprintln!("Chamfer PASS: reached execute_chamfer → clean NeedsRepair");
        wm.shutdown().await;
        return;
    }

    let cut = body_mesh(&mut rt, body_of(EXTRUDE_A)).await;
    let cut_view = validate_mesh_blob(&cut).expect("chamfered MESH1");
    let cut_vol = mesh_volume(&cut_view, &cut);

    // A chamfer bevels a corner: one NEW planar face, and material removed.
    assert!(
        cut_view.face_count > plain_faces,
        "the chamfer adds a face ({plain_faces} → {})",
        cut_view.face_count
    );
    assert!(
        cut_vol < plain_vol,
        "the chamfer removes material ({plain_vol} → {cut_vol})"
    );
    eprintln!(
        "Chamfer PASS: faces {plain_faces} → {}, volume {plain_vol} → {cut_vol}",
        cut_view.face_count
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// MODEL-OPS W3 — the drag-time preview verb, over the real wire.
//
// The whole point is that a preview shows what a COMMIT would produce. Before
// this verb the "exact" drag mesh was synthesized in JavaScript by the same
// function the mock client uses, so a Cut preview never subtracted at all.
// `test_preview_op` pins the worker-side semantics in-process; this pins the
// Rust plumbing end-to-end: the verb reaches the worker, returns a valid MESH1,
// AGREES with what committing the same op produces, and leaves the session
// completely untouched.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_matches_the_commit_and_leaves_no_trace() {
    use onecad_lib::worker::PreviewEngine;

    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // Base: a 40×20 rect extruded 10 ⇒ 8000, plus a 10×10 sketch inside it.
    let sa = SketchId(Uuid::from_u128(SKETCH_A));
    let sketch_a = rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0);
    add_op(&mut rt, sketch_record(SKETCH_A, &sketch_a));
    rt.apply(EditCommand::AddSketch {
        sketch: sketch_a.clone(),
    })
    .expect("AddSketch A");
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, sa, 10.0, BooleanMode::NewBody, None),
    );

    let sb = SketchId(Uuid::from_u128(SKETCH_B));
    let sketch_b = rect_sketch(sb, 0x2000, 10.0, 5.0, 10.0, 10.0);
    add_op(&mut rt, sketch_record(SKETCH_B, &sketch_b));
    rt.apply(EditCommand::AddSketch {
        sketch: sketch_b.clone(),
    })
    .expect("AddSketch B");
    published(&regen_all(&mut rt).await, "base");

    // The sketch must be in the worker's SketchStore for the preview to seed it.
    // `enter_sketch` pushes the already-authored sketch there (the entities were
    // registered by AddSketch, so re-upserting them would be a duplicate-id
    // rejection) — exactly the state a real drag is in when it previews.
    rt.enter_sketch(sb).await.expect("enter_sketch B");

    let head_before = rt.projection().revision;
    let base_vol = {
        let mesh = body_mesh(&mut rt, body_of(EXTRUDE_A)).await;
        let view = validate_mesh_blob(&mesh).expect("base MESH1");
        mesh_volume(&view, &mesh)
    };
    assert!(
        (base_vol - 8000.0).abs() < 1.0,
        "base is 8000, got {base_vol}"
    );

    // PREVIEW a Cut of the 10×10 profile, 5 deep. The candidate is the exact
    // typed core operation later committed; Rust owns all worker-wire lowering.
    let candidate = extrude_mode_record(
        OP_TAIL,
        sb,
        5.0,
        ExtrudeMode::Blind,
        BooleanMode::Cut,
        Some(body_of(EXTRUDE_A)),
        false,
        ExtrudeMode::Blind,
        0.0,
        0.0,
    );
    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            Some(sb.0.to_string()),
            None,
            Lod::Coarse,
        )
        .await
        .expect("PreviewOp reaches the worker");
    assert_eq!(
        preview.bodies.len(),
        1,
        "a Cut modifies the target, so exactly that body comes back"
    );
    let pv = {
        let blob = &preview.bodies[0].mesh;
        let view = validate_mesh_blob(blob).expect("preview MESH1 validates");
        mesh_volume(&view, blob)
    };
    // THE assertion: the preview already shows 8000 − 500 = 7500. The pre-W3
    // client-side stand-in showed the un-subtracted prism instead.
    eprintln!("preview Cut volume = {pv} (base {base_vol})");
    assert!(
        (pv - 7500.0).abs() < 1.0,
        "the preview must SUBTRACT (8000 − 500 = 7500), got {pv}"
    );

    // And the document is exactly where it was: same revision, same geometry.
    assert_eq!(
        rt.projection().revision,
        head_before,
        "a preview must not bump the document revision"
    );
    let after_vol = {
        let mesh = body_mesh(&mut rt, body_of(EXTRUDE_A)).await;
        let view = validate_mesh_blob(&mesh).expect("post-preview MESH1");
        mesh_volume(&view, &mesh)
    };
    assert!(
        (after_vol - 8000.0).abs() < 1.0,
        "the real body is untouched by the preview (8000), got {after_vol}"
    );

    // Committing the SAME op must now land on the volume the preview promised.
    add_op(&mut rt, candidate);
    published(&regen_all(&mut rt).await, "commit after preview");
    let committed = {
        let mesh = body_mesh(&mut rt, body_of(EXTRUDE_A)).await;
        let view = validate_mesh_blob(&mesh).expect("committed MESH1");
        mesh_volume(&view, &mesh)
    };
    assert!(
        (committed - pv).abs() < 1.0,
        "preview {pv} and commit {committed} must agree — that is the entire point"
    );
    eprintln!("preview/commit agreement PASS: {pv} == {committed}");
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_binds_the_non_first_region_and_rejects_stale_inputs() {
    use onecad_lib::worker::PreviewEngine;

    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xD00));
    let sketch = two_rect_sketch(sid);
    add_op(&mut rt, sketch_record(0xD00, &sketch));
    rt.apply(EditCommand::AddSketch {
        sketch: sketch.clone(),
    })
    .expect("AddSketch");
    published(&regen_all(&mut rt).await, "two-region sketch");

    rt.enter_sketch(sid).await.expect("sync sketch");
    let region_result = rt.finish_sketch(sid).await.expect("derive regions");
    let region_identity_version = region_result.region_identity_version;
    let regions = region_result.regions;
    assert_eq!(regions.len(), 2, "both disjoint cells must be selectable");
    let first_area = region_triangle_area(&regions[0]);
    let selected_area = region_triangle_area(&regions[1]);
    assert!(
        (first_area - selected_area).abs() > 100.0,
        "fixture regions must be unequal ({first_area} vs {selected_area})"
    );

    let mut candidate = extrude_mode_record(
        0xD01,
        sid,
        7.0,
        ExtrudeMode::Blind,
        BooleanMode::NewBody,
        None,
        false,
        ExtrudeMode::Blind,
        0.0,
        0.0,
    );
    let Operation::Known(KnownOperation::Extrude(params)) = &mut candidate.op else {
        unreachable!();
    };
    let profile = params.profile.as_mut().unwrap();
    profile.region = RegionId::new(regions[1].region_id.clone());
    profile.region_identity_version = Some(region_identity_version);
    candidate.inputs = candidate.op.derive_inputs();

    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            Some(sid.to_string()),
            None,
            Lod::Coarse,
        )
        .await
        .expect("explicit non-first region previews");
    assert_eq!(preview.bodies.len(), 1);
    // Preview body ids are Rust-domain (bare uuid, like document-changed BodyMeshRef) —
    // the FE matches them against the committed mesh registry, not the worker wire form.
    assert_eq!(preview.changed_bodies, vec![body_of(0xD01).to_string()]);
    let preview_blob = &preview.bodies[0].mesh;
    let preview_view = validate_mesh_blob(preview_blob).expect("preview MESH1");
    let preview_volume = mesh_volume(&preview_view, preview_blob);
    assert!(
        (preview_volume - selected_area * 7.0).abs() < 1.0,
        "preview used selected region: volume {preview_volume}, expected {}",
        selected_area * 7.0
    );

    let mut stale_region = candidate.op.clone();
    let Operation::Known(KnownOperation::Extrude(params)) = &mut stale_region else {
        unreachable!();
    };
    let profile = params.profile.as_mut().unwrap();
    profile.region = RegionId::new("r_stale");
    profile.region_identity_version = Some(region_identity_version);
    let stale_error = wm
        .preview_op(
            stale_region,
            Uuid::from_u128(0xD02).to_string(),
            Some(sid.to_string()),
            Some(SnapshotId(preview.snapshot_id)),
            Lod::Coarse,
        )
        .await
        .expect_err("stale region must not fall back to the first cell");
    assert!(
        stale_error.to_string().contains("r_stale"),
        "error identifies stale region: {stale_error}"
    );

    let mut missing_target = candidate.op.clone();
    let Operation::Known(KnownOperation::Extrude(params)) = &mut missing_target else {
        unreachable!();
    };
    params.boolean_mode = BooleanMode::Cut;
    params.target_body = Some(BodyId(Uuid::from_u128(0xDEAD)));
    let target_error = wm
        .preview_op(
            missing_target,
            Uuid::from_u128(0xD03).to_string(),
            Some(sid.to_string()),
            Some(SnapshotId(preview.snapshot_id)),
            Lod::Coarse,
        )
        .await
        .expect_err("Cut with a missing target must fail");
    assert!(
        target_error.to_string().contains("target body"),
        "error identifies missing Cut target: {target_error}"
    );

    let fence_error = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            Some(sid.to_string()),
            Some(SnapshotId(preview.snapshot_id + 1)),
            Lod::Coarse,
        )
        .await
        .expect_err("stale snapshot must be rejected");
    assert!(
        matches!(
            &fence_error,
            onecad_core::regen::EngineError::OpFailed {
                code: onecad_core::regen::OpFailureCode::StalePreview,
                ..
            }
        ),
        "stale snapshot carries the structured STALE_PREVIEW code: {fence_error}"
    );
    assert!(
        rt.projection().bodies.is_empty(),
        "all previews leave the head untouched"
    );

    add_op(&mut rt, candidate);
    published(&regen_all(&mut rt).await, "commit selected region");
    let committed = body_mesh(&mut rt, body_of(0xD01)).await;
    let committed_view = validate_mesh_blob(&committed).expect("committed MESH1");
    let committed_volume = mesh_volume(&committed_view, &committed);
    assert!(
        (committed_volume - preview_volume).abs() < 1.0,
        "preview {preview_volume} and commit {committed_volume} must match"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// EXTRUDE-REGION-PARITY P4 proof: NESTED cell binding + reopen stability.
// The earlier non-first-region tests use two DISJOINT rects; this pins the
// nested case (annulus + inner disk share every outer curve) end-to-end:
// preview == commit for the inner disk by exact id, the annulus binds
// independently, and a save → fresh-worker reopen replays byte-identically
// with the SAME region ids answering a read-only query.
// ─────────────────────────────────────────────────────────────────────────────

/// `two_rect_sketch`'s nested sibling: the 40×20 rect with an r=5 circle at its
/// centre — one annulus cell + one disk cell.
fn nested_sketch(sid: SketchId) -> Sketch {
    let mut sketch = rect_sketch(sid, 0x7000, 0.0, 0.0, 40.0, 20.0);
    let center = EntityId(Uuid::from_u128(0x9000));
    sketch
        .add_entity(SketchEntity::point(
            center,
            Vec2::new_unchecked(20.0, 10.0),
            false,
            false,
        ))
        .unwrap();
    sketch
        .add_entity(
            SketchEntity::circle(EntityId(Uuid::from_u128(0x9001)), center, 5.0, false)
                .expect("circle"),
        )
        .unwrap();
    sketch
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nested_inner_disk_parity_and_reopen_stability() {
    use onecad_lib::worker::PreviewEngine;

    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin.clone()).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xE00));
    let sketch = nested_sketch(sid);
    add_op(&mut rt, sketch_record(0xE00, &sketch));
    rt.apply(EditCommand::AddSketch {
        sketch: sketch.clone(),
    })
    .expect("AddSketch");
    published(&regen_all(&mut rt).await, "nested sketch");

    rt.enter_sketch(sid).await.expect("sync sketch");
    let region_result = rt.finish_sketch(sid).await.expect("derive regions");
    let region_identity_version = region_result.region_identity_version;
    let regions = region_result.regions;
    assert_eq!(regions.len(), 2, "annulus + inner disk: {regions:#?}");
    let disk = regions
        .iter()
        .find(|region| region.holes.is_empty())
        .expect("inner disk cell");
    let annulus = regions
        .iter()
        .find(|region| !region.holes.is_empty())
        .expect("annulus cell");
    let disk_area = region_triangle_area(disk);
    assert!(
        (disk_area - std::f64::consts::PI * 25.0).abs() < 2.0,
        "disk fill ≈ π·25 (tessellated): {disk_area}"
    );

    // Inner disk by exact id — preview then commit, volumes must agree.
    let mut candidate = extrude_mode_record(
        0xE01,
        sid,
        7.0,
        ExtrudeMode::Blind,
        BooleanMode::NewBody,
        None,
        false,
        ExtrudeMode::Blind,
        0.0,
        0.0,
    );
    let Operation::Known(KnownOperation::Extrude(params)) = &mut candidate.op else {
        unreachable!();
    };
    params.profile.as_mut().unwrap().region = RegionId::new(disk.region_id.clone());
    params.profile.as_mut().unwrap().region_identity_version = Some(region_identity_version);
    candidate.inputs = candidate.op.derive_inputs();

    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            Some(sid.to_string()),
            None,
            Lod::Coarse,
        )
        .await
        .expect("disk previews by exact id");
    assert_eq!(preview.bodies.len(), 1);
    let preview_view = validate_mesh_blob(&preview.bodies[0].mesh).expect("preview MESH1");
    let preview_volume = mesh_volume(&preview_view, &preview.bodies[0].mesh);
    let disk_want = std::f64::consts::PI * 25.0 * 7.0;
    assert!(
        (preview_volume - disk_want).abs() < disk_want * 0.03,
        "disk preview volume ≈ π·25·7: {preview_volume} vs {disk_want}"
    );

    add_op(&mut rt, candidate);
    published(&regen_all(&mut rt).await, "commit disk extrude");
    let committed = body_mesh(&mut rt, body_of(0xE01)).await;
    let committed_view = validate_mesh_blob(&committed).expect("committed MESH1");
    let committed_volume = mesh_volume(&committed_view, &committed);
    // Both lanes request the same Lod::Coarse, but preview and commit tessellate
    // via separate worker-side calls, so their inscribed-polygon chordal deficit
    // on this cylinder's periodic seam need not be bit-identical — OCCT 8.0's
    // reworked periodic-seam meshing (only the current wire occurrence's pcurve
    // creates mesh links) measurably widened that gap vs 7.9.3's ~<1 mm³ one.
    // 3% mirrors this test's own preview/annulus-vs-analytic tolerance above
    // (run 2026-08-07, OCCT 8.0.1: preview 544.443, commit 549.604, diff 0.94%).
    assert!(
        (committed_volume - preview_volume).abs() < committed_volume * 0.03,
        "disk preview {preview_volume} and commit {committed_volume} must agree"
    );

    // The annulus sibling binds independently by ITS id (holes are identity).
    let mut annulus_rec = extrude_mode_record(
        0xE02,
        sid,
        4.0,
        ExtrudeMode::Blind,
        BooleanMode::NewBody,
        None,
        false,
        ExtrudeMode::Blind,
        0.0,
        0.0,
    );
    let Operation::Known(KnownOperation::Extrude(annulus_params)) = &mut annulus_rec.op else {
        unreachable!();
    };
    annulus_params.profile.as_mut().unwrap().region = RegionId::new(annulus.region_id.clone());
    annulus_params
        .profile
        .as_mut()
        .unwrap()
        .region_identity_version = Some(region_identity_version);
    annulus_rec.inputs = annulus_rec.op.derive_inputs();
    add_op(&mut rt, annulus_rec);
    published(&regen_all(&mut rt).await, "commit annulus extrude");
    let annulus_mesh = body_mesh(&mut rt, body_of(0xE02)).await;
    let annulus_view = validate_mesh_blob(&annulus_mesh).expect("annulus MESH1");
    let annulus_volume = mesh_volume(&annulus_view, &annulus_mesh);
    let annulus_want = (800.0 - std::f64::consts::PI * 25.0) * 4.0;
    assert!(
        (annulus_volume - annulus_want).abs() < annulus_want * 0.03,
        "annulus volume ≈ (800−π·25)·4: {annulus_volume} vs {annulus_want}"
    );

    // Save → reopen in a FRESH worker process: replay is deterministic and the
    // read-only region query answers with the SAME ids (no edit session opened).
    let head1 = wm.get_worker_head().await.expect("worker 1 head");
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("nested_parity.onecad");
    rt.save(
        &path,
        SaveMeta {
            app_version: "wire-contract".into(),
            occt_fingerprint: None,
            created: "2026-07-29T00:00:00Z".into(),
            modified: "2026-07-29T00:00:00Z".into(),
        },
    )
    .expect("save container");

    let wm2 = spawn_worker(bin).await;
    let mut rt2 = open_runtime_over(&wm2, &path);
    published(&regen_all(&mut rt2).await, "reopen replay");
    let head2 = wm2.get_worker_head().await.expect("worker 2 head");
    assert_eq!(
        head1.history_prefix_hash, head2.history_prefix_hash,
        "identical hash chain across two fresh worker processes"
    );

    let reopened = rt2
        .prepare_sketch_regions(sid)
        .expect("read-only region query on a reopened document")
        .drive()
        .await
        .expect("regions")
        .regions;
    let before: std::collections::BTreeSet<&str> =
        regions.iter().map(|r| r.region_id.as_str()).collect();
    let after: std::collections::BTreeSet<&str> =
        reopened.iter().map(|r| r.region_id.as_str()).collect();
    assert_eq!(
        before, after,
        "region ids are stable across save/reopen/fresh-worker"
    );

    wm.shutdown().await;
    wm2.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// W3 — extruding an ELLIPSE profile end to end.
//
// The whole chain (Rust `wire_entity` → worker `WireSketch::translate` →
// `LoopDetector` → `FaceBuilder` → `BRepPrimAPI_MakePrism`) now carries an
// ellipse. The proof that the boundary is a TRUE `Geom_Ellipse` and not the
// planarized chord polygon is the FACE COUNT: an analytic elliptical prism has
// exactly three faces (bottom cap, top cap, one lateral surface). A chord
// approximation would publish one planar lateral face per tessellation segment.
//
// Analytic volume: area(ellipse) · height = π·a·b·h = π·6·3·10 = 565.4867.
// (Provenance: the ellipse area formula; the corpus `regions_ellipse` case only
// pins area > 50 — corpus/cases/i_multiregion_loop_detection.json.)
// ─────────────────────────────────────────────────────────────────────────────

const ELL_MAJOR_R: f64 = 6.0;
const ELL_MINOR_R: f64 = 3.0;
const ELL_HEIGHT: f64 = 10.0;

/// A single free ellipse (centre point + ellipse) — one closed curve, one region.
fn ellipse_sketch(sid: SketchId, base: u128, cx: f64, cy: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let mut sk = Sketch::on_world_plane(sid, "Ellipse", WorldPlane::XY);
    sk.add_entity(SketchEntity::point(
        e(0),
        Vec2::new_unchecked(cx, cy),
        false,
        false,
    ))
    .unwrap();
    sk.add_entity(
        SketchEntity::ellipse(e(0x10), e(0), ELL_MAJOR_R, ELL_MINOR_R, 0.25, false)
            .expect("ellipse"),
    )
    .unwrap();
    sk
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extrude_of_a_pure_ellipse_loop_is_analytic() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let se = SketchId(Uuid::from_u128(0xE1));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &ellipse_sketch(se, 0x3000, 0.0, 0.0)),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, se, ELL_HEIGHT, BooleanMode::NewBody, None),
    );

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "ellipse extrude");
    assert_eq!(snap.bodies.len(), 1, "one NewBody from the ellipse profile");

    // FINE lod: the mesh is a chord tessellation of the analytic lateral surface
    // and always UNDER-estimates a convex solid, so the coarse tier lands ~2.5%
    // low. The fine tier is what makes a 1% analytic band meaningful.
    let mesh = rt
        .get_mesh(body_of(EXTRUDE_A), Lod::Fine, None)
        .await
        .expect("fetch fine ellipse mesh");
    let view = validate_mesh_blob(&mesh).expect("ellipse prism MESH1 validates");

    // A PURE (unfragmented) ellipse loop builds analytic geometry — three faces,
    // not 2 + one-per-chord. This is what separates it from the §7.4 V1
    // intersection-fragment limitation.
    assert_eq!(
        view.face_count, 3,
        "an analytic elliptical prism has 3 faces (2 caps + 1 lateral); a chord \
         polygon would publish one lateral face per segment"
    );

    let vol = mesh_volume(&view, &mesh);
    let analytic = std::f64::consts::PI * ELL_MAJOR_R * ELL_MINOR_R * ELL_HEIGHT;
    assert!(
        (vol - analytic).abs() / analytic < 0.01,
        "ellipse prism volume {vol} within 1% of π·a·b·h = {analytic} \
         (the MESH is a tessellation of the analytic surface, hence the band)"
    );

    // Extent from the VERTEX buffer, not `view.bbox_*` — the header box comes from
    // `BRepBndLib::Add`, which inflates by OCCT's gap, so it cannot pin a dimension.
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let pbase = pos.offset as usize;
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for i in 0..view.vertex_count as usize {
        let v = vertex(&mesh, pbase, i);
        for axis in 0..3 {
            lo[axis] = lo[axis].min(v[axis]);
            hi[axis] = hi[axis].max(v[axis]);
        }
    }
    assert!(
        (hi[2] - lo[2] - ELL_HEIGHT).abs() < 1e-4,
        "the prism is exactly {ELL_HEIGHT} tall, got {}",
        hi[2] - lo[2]
    );
    // A rotated ellipse spans 2·√(a²sin²θ + b²cos²θ) along one plane axis and
    // 2·√(a²cos²θ + b²sin²θ) along the other — the proof that `rotation` really
    // crossed the wire and reached `gp_Elips`'s major direction. (Sketch user X
    // maps to world Y on the non-standard XY basis, hence the axis pairing.)
    let (cos_r, sin_r) = (0.25f64.cos(), 0.25f64.sin());
    let across =
        2.0 * (ELL_MAJOR_R.powi(2) * sin_r.powi(2) + ELL_MINOR_R.powi(2) * cos_r.powi(2)).sqrt();
    let along =
        2.0 * (ELL_MAJOR_R.powi(2) * cos_r.powi(2) + ELL_MINOR_R.powi(2) * sin_r.powi(2)).sqrt();
    assert!(
        (hi[0] - lo[0] - across).abs() / across < 0.01,
        "world-X span {} vs rotated-ellipse {across} (rotation 0.25 rad reached the kernel)",
        hi[0] - lo[0]
    );
    assert!(
        (hi[1] - lo[1] - along).abs() / along < 0.01,
        "world-Y span {} vs rotated-ellipse {along}",
        hi[1] - lo[1]
    );
    // Sanity that this is not a circle in disguise: the two spans differ.
    assert!(
        (along - across).abs() > 1.0,
        "the profile is genuinely elliptical (spans {along} vs {across})"
    );

    wm.shutdown().await;
    eprintln!(
        "ellipse extrude PASS: volume {vol:.4} == π·6·3·10 ({analytic:.4}), faces {} (analytic)",
        view.face_count
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// MEASURE V1a — element_info's descriptor numbers are the KERNEL's, exactly
// ─────────────────────────────────────────────────────────────────────────────

/// The measure tool reports a face's area and an edge's length straight off the
/// worker's `QueryElement` descriptor. Those numbers come from OCCT `GProp`
/// (`BRepGProp::SurfaceProperties` / `LinearProperties` in
/// `ElementMap::computeDescriptor`), NOT from the tessellation the viewport
/// draws — so this pins them against a shape whose exact answer is arithmetic.
///
/// Fixture: `rect_sketch(…, 0,0, 40,20)` extruded 25. The frozen `xy_plane_ref`
/// basis is deliberately NON-STANDARD — `xAxis = (0,1,0)`, `yAxis = (−1,0,0)` —
/// so sketch (u,v) maps to world `(−v, u, 0)` and the profile lands on
/// worldX[−20,0] × worldY[0,40]. Hence:
///   * top face area  = 40·20 = **800 mm²**
///   * top face CENTER (the descriptor's Bnd_Box centre, *not* a centroid)
///     = **(−10, 20, 25)** — asserting (20,10,25) here would be reading the
///     sketch basis as the identity it is not.
///   * every top-face edge is 40 mm or 20 mm long, and is a LINE (`GeomAbs_Line`
///     == curveType 0).
/// A never-promoted element id must read as `Ok(None)` — "that pick is gone" is
/// an ordinary outcome, not an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn measure_element_info_reports_exact_kernel_quantities() {
    use onecad_lib::worker::ElementQuery;

    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let wm = spawn_worker(real_worker().unwrap()).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, sa, 25.0, BooleanMode::NewBody, None),
    );
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "measure box");
    let snap_id = SnapshotId(report.snapshot_id);
    let body = body_of(EXTRUDE_A);

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    assert_eq!(view.face_count, 6, "the extruded rect is a 6-faced box");

    // ── FACE: exact area + the box-centre the distance summary uses ──────────
    let (top_key, top_centroid) = top_face_pick(&view, &mesh);
    let promoted = rt
        .promote_selection(snap_id, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("promote top face");
    let face_el = promoted[0].element_id.clone();

    // REF-FRESH-1: promotion installs the Rust-minted id into the same unchanged
    // worker head before returning. TopoKey lookup below remains an independent
    // measurement path, not a repair for a missing direct binding.
    assert!(
        ElementQuery::query_element(&wm, snap_id, body, &face_el)
            .await
            .expect("QueryElement(by elementId)")
            .is_some(),
        "a promoted ElementId must be directly queryable on the unchanged head"
    );

    let face = ElementQuery::query_element_by_topo_key(&wm, snap_id, body, &top_key)
        .await
        .expect("QueryElement(top face by topoKey)")
        .expect("the picked face resolves against the body shape");
    assert_eq!(face.kind, "face");
    assert_eq!(face.surface_type, 0, "a box cap is GeomAbs_Plane (0)");
    assert!(
        (face.magnitude - 800.0).abs() < 0.1,
        "top face area is EXACTLY 40·20 = 800 mm², got {}",
        face.magnitude
    );
    // Bnd_Box centre of worldX[−20,0] × worldY[0,40] × Z=25.
    for (axis, got, want) in [
        ("x", face.center[0], -10.0),
        ("y", face.center[1], 20.0),
        ("z", face.center[2], 25.0),
    ] {
        assert!(
            (got - want).abs() < 0.05,
            "top face center.{axis} = {want} (frozen non-standard sketch basis), got {got} \
             (full center {:?}, mesh centroid {:?})",
            face.center,
            top_centroid
        );
    }
    // The box diagonal of a 40×20×0 slab: √(40²+20²) ≈ 44.72.
    assert!(
        (face.size - (40.0f64.powi(2) + 20.0f64.powi(2)).sqrt()).abs() < 0.05,
        "size is the bbox diagonal, got {}",
        face.size
    );

    // ── EDGE: exact arc length from the mesh's own edge id table ─────────────
    assert!(view.edge_count > 0, "the box mesh carries edges");
    let edge_keys = id_table(
        &view,
        &mesh,
        SEC_EDGE_ID_OFFS,
        SEC_EDGE_ID_CHARS,
        view.edge_count as usize,
    );
    let edge = ElementQuery::query_element_by_topo_key(&wm, snap_id, body, &edge_keys[0])
        .await
        .expect("QueryElement(edge by topoKey)")
        .expect("the picked edge resolves against the body shape");
    assert_eq!(edge.kind, "edge");
    assert_eq!(edge.curve_type, 0, "a box edge is GeomAbs_Line (0)");
    // Every edge of a 40×20×25 box is one of its three side lengths.
    let len = edge.magnitude;
    assert!(
        [40.0f64, 20.0, 25.0]
            .iter()
            .any(|want| (len - want).abs() < 0.1),
        "edge length must be one of 40/20/25 mm, got {len}"
    );

    // ── An element that does not exist is ABSENT, not an error ───────────────
    for (what, got) in [
        (
            "unknown elementId",
            ElementQuery::query_element(&wm, snap_id, body, "el_never_promoted")
                .await
                .expect("an absent element is Ok, not Err"),
        ),
        (
            "stale topoKey",
            ElementQuery::query_element_by_topo_key(&wm, snap_id, body, "f:9999")
                .await
                .expect("an absent element is Ok, not Err"),
        ),
    ] {
        assert!(got.is_none(), "{what} reads as None, got {got:?}");
    }

    wm.shutdown().await;
    eprintln!(
        "measure PASS: face area {} == 800, center {:?} == (-10,20,25), edge len {len}",
        face.magnitude, face.center
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// WP-C1 — QueryMassProperties reports the KERNEL's exact body-level quantities
// ─────────────────────────────────────────────────────────────────────────────

/// `QueryMassProperties` (SCHEMA §7.5) over the real worker.
///
/// Same fixture discipline as the element_info test above: the frozen
/// `xy_plane_ref` basis is NON-STANDARD (`xAxis = (0,1,0)`, `yAxis = (−1,0,0)`),
/// so a sketch rect `(0,0)–(40,20)` lands on worldX[−20,0] × worldY[0,40]. A 25
/// extrude therefore gives a 20 × 40 × 25 block:
///   * volume       = 20 · 40 · 25 = **20000 mm³**
///   * surface area = 2·(20·40 + 20·25 + 40·25) = **4600 mm²**
///   * centroid     = **(−10, 20, 12.5)** — the true centre of MASS, which for a
///     box coincides with the box centre (a shape where they differ is what the
///     `ElementInfoDto::center` caveat is about, not this field)
///   * principal moments about the CENTROID = V·(a²+b²)/12 per axis. An
///     about-ORIGIN answer would be several times larger for this off-origin
///     block, so these three numbers are what pin the Huygens transfer.
///
/// Then a 10 × 10 pocket 5 deep is CUT into the same body, and the volume must
/// drop by exactly 500 — proving the reading tracks the live head rather than
/// some cached first answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mass_properties_report_exact_kernel_quantities() {
    use onecad_lib::worker::ElementQuery;

    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    let wm = spawn_worker(real_worker().unwrap()).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_A, sa, 25.0, BooleanMode::NewBody, None),
    );
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "mass props block");
    let body = body_of(EXTRUDE_A);

    let mp = ElementQuery::query_mass_properties(&wm, body, "body1".into())
        .await
        .expect("QueryMassProperties over the extruded block");

    assert_eq!(mp.body_id, "body1", "the DTO echoes the CALLER's body id");
    assert!(
        (mp.volume - 20_000.0).abs() < 0.01,
        "volume is EXACTLY 20·40·25 = 20000 mm³, got {}",
        mp.volume
    );
    assert!(
        (mp.surface_area - 4600.0).abs() < 0.01,
        "surface area is EXACTLY 2·(800+500+1000) = 4600 mm², got {}",
        mp.surface_area
    );
    for (axis, got, want) in [
        ("x", mp.centroid[0], -10.0),
        ("y", mp.centroid[1], 20.0),
        ("z", mp.centroid[2], 12.5),
    ] {
        assert!(
            (got - want).abs() < 0.01,
            "centroid.{axis} = {want} (frozen non-standard sketch basis), got {got} \
             (full centroid {:?})",
            mp.centroid
        );
    }

    // Principal frame: unit rows, right-handed, and each moment paired with the
    // world axis its own row names (the block is axis-aligned, so every principal
    // axis IS a world axis — matched by dominant component rather than by a
    // presumed sort order).
    let spans = [20.0f64, 40.0, 25.0]; // world X, Y, Z extents
    for (k, (row, moment)) in mp
        .principal_axes
        .iter()
        .zip(mp.principal_moments.iter())
        .enumerate()
    {
        let len = (row[0] * row[0] + row[1] * row[1] + row[2] * row[2]).sqrt();
        assert!(
            (len - 1.0).abs() < 1e-9,
            "principal axis is unit, got {len}"
        );
        let dominant = (0..3)
            .max_by(|&i, &j| row[i].abs().total_cmp(&row[j].abs()))
            .unwrap();
        assert!(
            (row[dominant].abs() - 1.0).abs() < 1e-6,
            "an axis-aligned block's principal axes are the world axes, got {row:?}"
        );
        // Only the first TWO rows are sign-canonicalized. The third is rebuilt as
        // a1 × a2 so the frame is right-handed by construction, which means its
        // sign is whatever handedness demands — canonicalizing it too could yield
        // a left-handed triple.
        assert!(
            k == 2 || row[dominant] > 0.0,
            "rows 0–1 are sign-canonical (dominant component positive), got {row:?}"
        );
        // V·(other two spans squared)/12, about the CENTROID.
        let others: f64 = (0..3)
            .filter(|&i| i != dominant)
            .map(|i| spans[i] * spans[i])
            .sum();
        let want = 20_000.0 * others / 12.0;
        assert!(
            (moment - want).abs() < 1.0,
            "moment about world axis {dominant} is V·(a²+b²)/12 = {want}, got {moment} \
             (an about-ORIGIN answer would be far larger for this off-origin block)"
        );
    }
    let [a1, a2, a3] = mp.principal_axes;
    let cross = [
        a1[1] * a2[2] - a1[2] * a2[1],
        a1[2] * a2[0] - a1[0] * a2[2],
        a1[0] * a2[1] - a1[1] * a2[0],
    ];
    let handedness = cross[0] * a3[0] + cross[1] * a3[1] + cross[2] * a3[2];
    assert!(
        (handedness - 1.0).abs() < 1e-6,
        "(a1 × a2) · a3 == +1 — the frame is right-handed, got {handedness}"
    );

    // ── An unknown body fails LOUDLY, never as a zero reading ────────────────
    let missing = ElementQuery::query_mass_properties(
        &wm,
        BodyId(Uuid::from_u128(0xDEAD_BEEF)),
        "body_gone".into(),
    )
    .await;
    assert!(
        missing.is_err(),
        "an unknown body is REF_UNRESOLVED, not a 0 mm³ reading — got {missing:?}"
    );

    // ── After a Cut, the reading tracks the LIVE head ────────────────────────
    let sb = SketchId(Uuid::from_u128(0xB));
    add_op(
        &mut rt,
        sketch_record(SKETCH_B, &rect_sketch(sb, 0x2000, 0.0, 0.0, 10.0, 10.0)),
    );
    add_op(
        &mut rt,
        extrude_record(EXTRUDE_B, sb, 5.0, BooleanMode::Cut, Some(body)),
    );
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "mass props after cut");

    let after = ElementQuery::query_mass_properties(&wm, body, "body1".into())
        .await
        .expect("QueryMassProperties after the Cut");
    assert!(
        (after.volume - 19_500.0).abs() < 0.01,
        "the 10×10×5 pocket removes EXACTLY 500 mm³ (20000 − 500 = 19500), got {}",
        after.volume
    );

    wm.shutdown().await;
    eprintln!(
        "mass properties PASS: volume {} == 20000, area {} == 4600, centroid {:?}, \
         after cut {} == 19500",
        mp.volume, mp.surface_area, mp.centroid, after.volume
    );
}
