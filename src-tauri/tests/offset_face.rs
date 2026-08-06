//! `OffsetFace` (SCHEMA §7.3 `op.offsetFace` + §7.6 `PrepareOffsetFace`, added
//! 2026-08-06) integration gate against the REAL C++ OCCT worker, driven through
//! the app's [`DocumentRuntime`] exactly like `breadth_ops.rs` / `hole_ops.rs`.
//!
//! Every test authors its refs through the PRODUCTION transaction — the read-only
//! `PrepareOffsetFace` handshake returns TopoKey + anchor/descriptor EVIDENCE, Rust
//! promotes it through `AcquireElementIds` and mints the `ElementId` (Invariant 2 —
//! the worker never mints), and the typed `ElementRef` is built from what came
//! back. Nothing here hand-rolls a TopoKey into `faceIds`.
//!
//! Volumes come from `QueryMassProperties` (`BRepGProp` over the real BRep), so the
//! analytic expectations are asserted to kernel precision, not to a mesh chord.
//!
//! * `offset_top_face_grows_box` — a 20×20×25 box's top cap pushed +5 ⇒ EXACTLY
//!   1.2 × the stock (12000), the body is MODIFIED in place (same BodyId, still 6
//!   faces, still one body) and the §7.2 lineage names exactly that one body.
//! * `offset_face_radius_resizes_cylinder_boss` — a Ø20 boss's lateral face driven
//!   to `distanceType: Radius` 12 ⇒ EXACTLY `π·12²·25`, and the handshake's
//!   `currentDims.radius` seeded the value it was measured from.
//! * `offset_survives_benign_upstream_edit` — the source extrude's depth 25 → 30:
//!   the ladder rebinds the operated cap (no `NeedsRepair`) and the volume tracks
//!   the new stock, proving the frozen operative set is re-resolved, not re-picked.
//! * `destructive_edit_is_deterministic_needs_repair_then_repairable` — the sketch
//!   is replaced with a tiny rect far away: deterministic `NeedsRepair` (never a
//!   silent wrong bind), the m−1 body is the UNOFFSET box, and an
//!   `EditOperationInput{OffsetFaceFace{0}}` rebind from the repair state's own
//!   candidate makes the very next regen green.
//! * `prepare_offset_face_handshake_and_fence` — the §7.6 contract: closure +
//!   `picked` flags + `targetBodyId` + `currentDims` (`thickness` for `Total`,
//!   `radius` for a cylindrical set), a REFUSAL is an answer (`chainOnTotal`), and
//!   a stale `snapshotId` is refused `STALE_PREVIEW` instead of answered.
//! * `offset_face_deterministic_across_processes` — the same document built in two
//!   FRESH worker processes yields the identical body signature + volume
//!   (Invariant 5).
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly unless `ONECAD_REQUIRE_WORKER=1`.

use std::f64::consts::PI;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, OffsetDistanceType, OffsetFaceParams,
    Operation, OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, InputPath, InputRef};
use onecad_core::ids::{
    BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, EngineError, GeometryEngine, Lod, ModelSnapshot, OpFailureCode, Outcome,
    RegenRequest, StoppedReason,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::dto::PrepareOffsetFaceDto;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::{body_id_wire, sketch_wire, FaceAddress, OffsetFacePick};
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, FaceBoundaryProjection, MeshProvider, SolverEngine,
    WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors breadth_ops.rs / hole_ops.rs)
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

/// The EXACT kernel volume (`BRepGProp`), not a tessellation estimate.
async fn exact_volume(wm: &WorkerManager, body: BodyId) -> f64 {
    ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties")
        .volume
}

// Fixed record ids.
const SKETCH_REC: u128 = 0xA00;
const EXTRUDE_REC: u128 = 0xA01;
const OFFSET_REC: u128 = 0xA02;

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (rect_sketch verbatim from breadth_ops.rs)
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

/// A circle profile (centre point + circle) — extrudes to a cylindrical boss whose
/// lateral face is the operative face of the `Radius` test.
fn circle_sketch(sid: SketchId, base: u128, cx: f64, cy: f64, r: f64) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Circle", WorldPlane::XY);
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

fn sketch_op(sk: &Sketch) -> Operation {
    let (_plane, entities, constraints) = sketch_wire(sk);
    Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch: sk.id,
        plane: xy_plane_ref(),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    }))
}

fn sketch_record(rec: u128, sk: &Sketch) -> OperationRecord {
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Sketch", sketch_op(sk))
}

fn extrude_op(sketch: SketchId, dist: f64) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // first-region fallback
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
    }))
}

fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        extrude_op(sketch, dist),
    )
}

fn offset_op(params: OffsetFaceParams) -> Operation {
    Operation::Known(KnownOperation::OffsetFace(params))
}

fn offset_record(rec: u128, params: OffsetFaceParams) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "OffsetFace",
        offset_op(params),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 face-pick helpers — a TopoKey to feed `PrepareOffsetFace`, chosen the way
// a viewport pick would (from the published tessellation of the head)
// ─────────────────────────────────────────────────────────────────────────────

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

/// One tessellated face reduced to what a pick needs: its snapshot-scoped
/// `TopoKey`, its centroid, and its world-Z extent (the only discriminator that
/// separates a prism's caps from its walls without reading the BRep).
struct FacePick {
    topo_key: String,
    centroid: Vec3,
    z_span: f64,
}

fn face_picks(view: &MeshHeaderView, blob: &[u8]) -> Vec<FacePick> {
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
    let mut out = Vec::with_capacity(view.face_count as usize);
    for (f, topo_key) in keys.iter().enumerate() {
        let first = u32_le(blob, frbase + f * 8) as usize;
        let count = u32_le(blob, frbase + f * 8 + 4) as usize;
        let (mut sx, mut sy, mut sz, mut n) = (0.0, 0.0, 0.0, 0.0f64);
        let (mut zmin, mut zmax) = (f64::MAX, f64::MIN);
        for t in first..first + count {
            let io = ibase + t * 12;
            for k in 0..3 {
                let v = vertex(blob, pbase, u32_le(blob, io + k * 4) as usize);
                sx += v[0];
                sy += v[1];
                sz += v[2];
                n += 1.0;
                zmin = zmin.min(v[2]);
                zmax = zmax.max(v[2]);
            }
        }
        if n == 0.0 {
            continue;
        }
        out.push(FacePick {
            topo_key: topo_key.clone(),
            centroid: Vec3::new_unchecked(sx / n, sy / n, sz / n),
            z_span: zmax - zmin,
        });
    }
    out
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

/// The published head's faces for `body`, plus its tessellated face count.
async fn head_faces(rt: &mut DocumentRuntime, body: BodyId) -> (Vec<FacePick>, u32) {
    let mesh = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("body MESH1 validates");
    (face_picks(&view, &mesh), view.face_count)
}

/// The extrude's far cap — the greatest average world-Z.
fn top_face(picks: &[FacePick]) -> &FacePick {
    picks
        .iter()
        .max_by(|a, b| a.centroid.z.total_cmp(&b.centroid.z))
        .expect("at least one face")
}

/// A prism's lateral wall — the face spanning the full extrude height (a planar cap
/// spans ≈0).
fn lateral_face(picks: &[FacePick]) -> &FacePick {
    picks
        .iter()
        .max_by(|a, b| a.z_span.total_cmp(&b.z_span))
        .expect("at least one face")
}

// ─────────────────────────────────────────────────────────────────────────────
// The production authoring transaction (SCHEMA §7.6 → `AcquireElementIds` → refs)
// ─────────────────────────────────────────────────────────────────────────────

fn anchor_of(evidence: &serde_json::Value) -> AnchorIntent {
    serde_json::from_value(evidence.clone())
        .expect("PrepareOffsetFace anchor parses as AnchorIntent")
}

fn face_ref(body: BodyId, element: &ElementId, anchor: AnchorIntent) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: element.clone(),
            kind: ElementKind::Face,
            extra: Default::default(),
        }),
        intent: None, // `DocumentRuntime::stamp_intents` fills it from the promotion cache
        anchor: Some(anchor),
        extra: Default::default(),
    }
}

/// Runs `PrepareOffsetFace` for `picks` against `snapshot` and returns the raw
/// §7.6 answer — refusals included, since a refusal is an ANSWER (`ok:true`).
async fn prepare(
    wm: &WorkerManager,
    snapshot: SnapshotId,
    body: BodyId,
    picks: &[&str],
    chain: bool,
    distance_type: OffsetDistanceType,
) -> Result<PrepareOffsetFaceDto, EngineError> {
    let picks: Vec<OffsetFacePick<'_>> = picks
        .iter()
        .map(|k| OffsetFacePick {
            body: Some(body),
            address: FaceAddress::TopoKey(k),
        })
        .collect();
    FaceBoundaryProjection::prepare_offset_face(wm, snapshot, &picks, chain, distance_type).await
}

/// The FULL authoring transaction for one OffsetFace record: `PrepareOffsetFace`
/// computes the frozen operative closure and the worker's own on-surface evidence,
/// Rust promotes each entry into an `ElementId` (Invariant 2), and the lockstep
/// `(faceIds, faces)` pair the record persists is built from what came back. The
/// `Total` opposite face rides in the returned option, already promoted.
///
/// Returns the handshake too, so a caller can assert on `currentDims` / `picked`.
#[allow(clippy::type_complexity)]
async fn author_offset(
    wm: &WorkerManager,
    rt: &mut DocumentRuntime,
    snapshot: SnapshotId,
    body: BodyId,
    picks: &[&str],
    chain: bool,
    distance_type: OffsetDistanceType,
) -> (
    Vec<ElementId>,
    Vec<ElementRef>,
    Option<(ElementId, ElementRef)>,
    PrepareOffsetFaceDto,
) {
    let dto = prepare(wm, snapshot, body, picks, chain, distance_type)
        .await
        .expect("PrepareOffsetFace answers against the head");
    assert!(
        dto.refusal.is_none(),
        "PrepareOffsetFace refused this selection: {:?}",
        dto.refusal
    );
    assert_eq!(
        dto.snapshot_id, snapshot.0,
        "the answer ECHOES the head it was computed against"
    );
    assert_eq!(
        dto.target_body_id,
        body_id_wire(body),
        "the closure belongs to the body the picks named"
    );

    let mut ids = Vec::with_capacity(dto.faces.len());
    let mut refs = Vec::with_capacity(dto.faces.len());
    for face in &dto.faces {
        let anchor = anchor_of(face.anchor.as_ref().expect("evidence carries an anchor"));
        let id = promote(rt, snapshot, body, &face.topo_key, &anchor).await;
        refs.push(face_ref(body, &id, anchor));
        ids.push(id);
    }
    let opposite = match &dto.opposite_face {
        None => None,
        Some(face) => {
            let anchor = anchor_of(
                face.anchor
                    .as_ref()
                    .expect("opposite evidence has an anchor"),
            );
            let id = promote(rt, snapshot, body, &face.topo_key, &anchor).await;
            Some((id.clone(), face_ref(body, &id, anchor)))
        }
    };
    (ids, refs, opposite, dto)
}

/// Promotes ONE snapshot-scoped `TopoKey` into a Rust-minted `ElementId`, seeding
/// the promotion cache so `stamp_intents` can freeze the worker's descriptor onto
/// the ref when the record is authored.
async fn promote(
    rt: &mut DocumentRuntime,
    snapshot: SnapshotId,
    body: BodyId,
    topo_key: &str,
    anchor: &AnchorIntent,
) -> ElementId {
    let promoted = rt
        .promote_selection(
            snapshot,
            body,
            vec![(TopoKey::new(topo_key), Some(anchor.clone()))],
        )
        .await
        .expect("AcquireElementIds promotes the pick at the head snapshot");
    let id = ElementId::new(&promoted[0].element_id);
    assert!(
        id.as_str().starts_with("el_"),
        "ElementIds are Rust-minted `el_…` (Invariant 2), got {id}"
    );
    id
}

/// A 20×20 rect extruded `depth`: the standard stock. Returns
/// `(body, head snapshot, top-face TopoKey)`.
async fn build_box(rt: &mut DocumentRuntime, sid: SketchId, depth: f64) -> (BodyId, SnapshotId) {
    add_op(
        rt,
        sketch_record(SKETCH_REC, &rect_sketch(sid, 0x1000, 0.0, 0.0, 20.0, 20.0)),
    );
    add_op(rt, extrude_record(EXTRUDE_REC, sid, depth));
    let report = regen_all(rt).await;
    let _ = published(&report, "stock box");
    (body_of(EXTRUDE_REC), SnapshotId(report.snapshot_id))
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Offset the top cap — the body GROWS in place, exactly
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offset_top_face_grows_box() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xA));
    let (body, snap) = build_box(&mut rt, sid, 25.0).await;

    let stock = exact_volume(&wm, body).await;
    assert!(
        (stock - 10_000.0).abs() < 1e-6,
        "stock is 20×20×25 = 10000, got {stock}"
    );
    let (picks, faces) = head_faces(&mut rt, body).await;
    assert_eq!(faces, 6, "the extruded rect is a 6-faced box");
    let top = top_face(&picks);
    assert!(
        top.centroid.z > 24.0,
        "the cap sits at z≈25, got {}",
        top.centroid.z
    );

    // Author through the production transaction: prepare → promote → typed refs.
    let (face_ids, refs, opposite, dto) = author_offset(
        &wm,
        &mut rt,
        snap,
        body,
        &[&top.topo_key],
        true,
        OffsetDistanceType::Offset,
    )
    .await;
    assert!(
        opposite.is_none(),
        "an `Offset` push-pull has no opposite face"
    );
    assert_eq!(
        dto.faces.len(),
        1,
        "a box cap is G1-tangent to nothing — the closure is the pick alone"
    );
    assert!(dto.faces[0].picked, "the single closure member IS the pick");

    add_op(
        &mut rt,
        offset_record(
            OFFSET_REC,
            OffsetFaceParams {
                face_ids,
                faces: refs,
                distance: Scalar::new(5.0),
                distance_type: OffsetDistanceType::Offset,
                chain_tangent_faces: true,
                opposite_face_id: None,
                opposite_face: None,
                target_body: body,
                extra: Default::default(),
            },
        ),
    );
    let report = regen_all(&mut rt).await;
    let snapshot = published(&report, "offset top face");
    assert_eq!(
        snapshot.repair_summary.needs_repair_count, 0,
        "the promoted ref binds — no repair"
    );
    assert_eq!(snapshot.stopped_reason, StoppedReason::Completed);

    // THE PROOF: exactly 1.2 × the stock (the cap moved +5 along its outward normal).
    let grown = exact_volume(&wm, body).await;
    assert!(
        (grown - 12_000.0).abs() < 1e-6,
        "top cap +5 ⇒ 20×20×30 = 12000 (1.2 × stock), got {grown}"
    );

    // MODIFY lineage (SCHEMA §7.3): one body in, the same body out, still 6 faces.
    assert_eq!(
        rt.head_body_ids(),
        vec![body],
        "OffsetFace mints nothing and deletes nothing"
    );
    let (_, faces_after) = head_faces(&mut rt, body).await;
    assert_eq!(faces_after, 6, "a pure push-pull adds no faces");
    let affected = report
        .affected_bodies
        .get(&Uuid::from_u128(OFFSET_REC).to_string())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        affected,
        vec![body.to_string()],
        "§7.2 lineage: EXACTLY one modified body, the target"
    );
    wm.shutdown().await;
    eprintln!("OffsetFace PASS: {stock} → {grown} (top cap +5, body {body} preserved)");
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. `Radius` on a cylindrical boss — the absolute type is re-derived from the
//    CURRENT radius each regen (σ = +1 for a boss)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offset_face_radius_resizes_cylinder_boss() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xC));

    // Ø20 × 25 boss.
    add_op(
        &mut rt,
        sketch_record(SKETCH_REC, &circle_sketch(sid, 0x2000, 0.0, 0.0, 10.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_REC, sid, 25.0));
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "cylinder boss");
    let body = body_of(EXTRUDE_REC);
    let snap = SnapshotId(report.snapshot_id);

    let before = exact_volume(&wm, body).await;
    assert!(
        (before - PI * 100.0 * 25.0).abs() < 1e-3,
        "boss is π·10²·25, got {before}"
    );
    let (picks, faces) = head_faces(&mut rt, body).await;
    assert_eq!(faces, 3, "a cylinder is two caps + one lateral face");
    let lateral = lateral_face(&picks);
    assert!(
        lateral.z_span > 24.0,
        "the lateral wall spans the full 25 of height, got {}",
        lateral.z_span
    );

    let (face_ids, refs, opposite, dto) = author_offset(
        &wm,
        &mut rt,
        snap,
        body,
        &[&lateral.topo_key],
        true,
        OffsetDistanceType::Radius,
    )
    .await;
    assert!(opposite.is_none(), "`Radius` carries no opposite face");
    // `currentDims.radius` is what SEEDS the tool's numeric field — the value the
    // user then edits. A missing seed would make the UI ask for an absolute radius
    // with nothing to start from.
    let seeded = dto
        .current_dims
        .radius
        .expect("a cylindrical closure reports currentDims.radius");
    assert!(
        (seeded - 10.0).abs() < 1e-6,
        "the seed is the CURRENT radius 10, got {seeded}"
    );

    add_op(
        &mut rt,
        offset_record(
            OFFSET_REC,
            OffsetFaceParams {
                face_ids,
                faces: refs,
                distance: Scalar::new(12.0), // absolute target radius, not a delta
                distance_type: OffsetDistanceType::Radius,
                chain_tangent_faces: true,
                opposite_face_id: None,
                opposite_face: None,
                target_body: body,
                extra: Default::default(),
            },
        ),
    );
    let report = regen_all(&mut rt).await;
    let snapshot = published(&report, "radius offset");
    assert_eq!(snapshot.repair_summary.needs_repair_count, 0);
    assert_eq!(snapshot.stopped_reason, StoppedReason::Completed);

    // THE PROOF: r 10 → 12 (σ=+1 boss ⇒ kernel offset d = +2), height untouched.
    let after = exact_volume(&wm, body).await;
    assert!(
        (after - PI * 144.0 * 25.0).abs() < 1e-3,
        "Radius 12 ⇒ π·12²·25 = {}, got {after}",
        PI * 144.0 * 25.0
    );
    assert_eq!(rt.head_body_ids(), vec![body], "modified in place");
    let (_, faces_after) = head_faces(&mut rt, body).await;
    assert_eq!(faces_after, 3, "still a plain cylinder");
    wm.shutdown().await;
    eprintln!("OffsetFace Radius PASS: r10 → r12, volume {before} → {after}");
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Benign upstream edit — the FROZEN operative set re-resolves through the ladder
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offset_survives_benign_upstream_edit() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xB));
    let (body, snap) = build_box(&mut rt, sid, 25.0).await;

    let (picks, _) = head_faces(&mut rt, body).await;
    let top = top_face(&picks);
    let (face_ids, refs, _, _) = author_offset(
        &wm,
        &mut rt,
        snap,
        body,
        &[&top.topo_key],
        true,
        OffsetDistanceType::Offset,
    )
    .await;
    add_op(
        &mut rt,
        offset_record(
            OFFSET_REC,
            OffsetFaceParams {
                face_ids,
                faces: refs,
                distance: Scalar::new(5.0),
                distance_type: OffsetDistanceType::Offset,
                chain_tangent_faces: true,
                opposite_face_id: None,
                opposite_face: None,
                target_body: body,
                extra: Default::default(),
            },
        ),
    );
    let _ = published(&regen_all(&mut rt).await, "offset before edit");
    let before = exact_volume(&wm, body).await;
    assert!(
        (before - 12_000.0).abs() < 1e-6,
        "before: 20×20×(25+5) = 12000, got {before}"
    );

    // BENIGN upstream edit: the source extrude's depth 25 → 30 (the same magnitude
    // `topology_rebind::h5b_fillet_survives_small_edit` pins). The operated cap keeps
    // its area, normal and footprint and only slides along +Z, so the ladder rebinds
    // it — the frozen operative set is RE-RESOLVED, never re-expanded (SCHEMA §7.3).
    //
    // The anchor term is the only distance-sensitive contribution, so the gate has a
    // real ceiling: at depth 40 the correct cap still RANKS FIRST but scores 0.847 /
    // margin 0.097, just under the 0.85/0.10 bar, and the step reports NeedsRepair
    // rather than auto-binding. That is the designed direction (deterministic
    // NeedsRepair over a silent bind), not a regression — see the report note.
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(EXTRUDE_REC)),
        op: extrude_op(sid, 30.0),
    })
    .expect("edit the source extrude depth");
    let report = regen_all(&mut rt).await;
    let snapshot = published(&report, "offset after upstream edit");
    assert_eq!(
        snapshot.repair_summary.needs_repair_count, 0,
        "a benign upstream edit must NOT push the offset into repair"
    );
    assert_eq!(snapshot.stopped_reason, StoppedReason::Completed);

    // THE PROOF: the offset re-applied on the NEW stock (35 tall, not a stale 30).
    let after = exact_volume(&wm, body).await;
    assert!(
        (after - 14_000.0).abs() < 1e-6,
        "after: 20×20×(30+5) = 14000, got {after}"
    );
    let (_, faces_after) = head_faces(&mut rt, body).await;
    assert_eq!(
        faces_after, 6,
        "still a plain box — nothing was double-offset"
    );
    wm.shutdown().await;
    eprintln!("Upstream-edit PASS: offset volume {before} → {after} (depth 25 → 30)");
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Destructive edit ⇒ deterministic NeedsRepair, then repairable through the
//    normative `InputPath` slot
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn destructive_edit_is_deterministic_needs_repair_then_repairable() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xD));
    let (body, snap) = build_box(&mut rt, sid, 25.0).await;

    let (picks, _) = head_faces(&mut rt, body).await;
    let top = top_face(&picks);
    let (face_ids, refs, _, _) = author_offset(
        &wm,
        &mut rt,
        snap,
        body,
        &[&top.topo_key],
        true,
        OffsetDistanceType::Offset,
    )
    .await;
    add_op(
        &mut rt,
        offset_record(
            OFFSET_REC,
            OffsetFaceParams {
                face_ids,
                faces: refs,
                distance: Scalar::new(5.0),
                distance_type: OffsetDistanceType::Offset,
                chain_tangent_faces: true,
                opposite_face_id: None,
                opposite_face: None,
                target_body: body,
                extra: Default::default(),
            },
        ),
    );
    let _ = published(&regen_all(&mut rt).await, "offset baseline");
    assert!(
        (exact_volume(&wm, body).await - 12_000.0).abs() < 1e-6,
        "precondition: the offset APPLIES on the clean box"
    );

    // DESTRUCTIVE edit: the sketch becomes a tiny rect 500 away. The operated cap's
    // geometry is gone (area 400 → 6, centroid 500 away), so nothing clears the
    // 0.85/0.10 auto-bind gate.
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(SKETCH_REC)),
        op: sketch_op(&rect_sketch(sid, 0x1000, 500.0, 500.0, 3.0, 2.0)),
    })
    .expect("edit sketch (destructive)");
    let broken = regen_all(&mut rt).await;
    let snapshot = published(&broken, "destructive edit").clone();

    // (1) Deterministic NeedsRepair — never a silent bind onto the tiny box's cap.
    assert_eq!(
        snapshot.stopped_reason,
        StoppedReason::NeedsRepair,
        "the OffsetFace step must STOP, not guess"
    );
    assert!(
        snapshot.repair_summary.needs_repair_count > 0,
        "a destroyed operative face is NeedsRepair STATE"
    );
    assert_eq!(
        snapshot.repair_summary.steps,
        vec![2],
        "step 2 (0 sketch, 1 extrude, 2 offsetFace) is the one needing repair"
    );

    // (2) The op did NOT apply: the published body is the m−1 tiny box, 3·2·25 = 150.
    let m_minus_1 = exact_volume(&wm, body).await;
    assert!(
        (m_minus_1 - 150.0).abs() < 1e-6,
        "NeedsRepair publishes the UNOFFSET box (150, not 3·2·30 = 180), got {m_minus_1}"
    );

    // (3) Determinism: a replay produces the IDENTICAL repair payload.
    let key = |rt: &DocumentRuntime| -> Vec<(usize, String, String)> {
        rt.repair_items()
            .iter()
            .map(|i| {
                (
                    i.step_index,
                    i.ref_id.clone(),
                    format!("{:?}/{:?}", i.ladder_failed, i.reason),
                )
            })
            .collect()
    };
    let items_a = key(&rt);
    let replay = regen_all(&mut rt).await;
    let _ = published(&replay, "destructive replay");
    assert_eq!(
        items_a,
        key(&rt),
        "the NeedsRepair payload is IDENTICAL across replays"
    );
    assert!(
        !broken.needs_repair.is_empty(),
        "the needs-repair event carried items"
    );

    // (4) REPAIR through the NORMATIVE slot (SCHEMA §7.3: operative faces first, in
    //     stored order). The re-pick comes from the repair state's OWN top-ranked
    //     candidate where the ladder surfaced one, else from a fresh pick of the new
    //     cap — either way the panel's exact command shape.
    let head = SnapshotId(replay.snapshot_id);
    let candidate = rt
        .repair_items()
        .iter()
        .find(|i| i.step_index == 2)
        .and_then(|i| i.candidates.first())
        .map(|c| c.topo_key.as_str().to_string());
    let repick = match candidate {
        Some(key) => key,
        None => {
            let (picks, _) = head_faces(&mut rt, body).await;
            top_face(&picks).topo_key.clone()
        }
    };
    let dto = prepare(
        &wm,
        head,
        body,
        &[&repick],
        true,
        OffsetDistanceType::Offset,
    )
    .await
    .expect("PrepareOffsetFace answers for the re-pick");
    assert!(
        dto.refusal.is_none(),
        "the re-pick is a valid closure: {dto:?}"
    );
    let anchor = anchor_of(dto.faces[0].anchor.as_ref().expect("re-pick anchor"));
    let repaired_id = promote(&mut rt, head, body, &dto.faces[0].topo_key, &anchor).await;

    rt.apply(EditCommand::EditOperationInput {
        record: RecordId(Uuid::from_u128(OFFSET_REC)),
        path: InputPath::OffsetFaceFace { index: 0 },
        reference: InputRef::Element(face_ref(body, &repaired_id, anchor)),
    })
    .expect("EditOperationInput{OffsetFaceFace{0}} is accepted for an OffsetFace record");

    let fixed = regen_all(&mut rt).await;
    let snapshot = published(&fixed, "repaired offset");
    assert_eq!(
        snapshot.repair_summary.needs_repair_count, 0,
        "the explicit rebind clears the repair state"
    );
    assert_eq!(snapshot.stopped_reason, StoppedReason::Completed);

    // (5) THE PROOF: the offset now applies to the TINY box's cap — 3·2·(25+5) = 180.
    let after = exact_volume(&wm, body).await;
    assert!(
        (after - 180.0).abs() < 1e-6,
        "repaired: 3·2·30 = 180 (m−1 was {m_minus_1}), got {after}"
    );
    wm.shutdown().await;
    eprintln!("Repair PASS: NeedsRepair @ {m_minus_1} → OffsetFaceFace{{0}} rebind → {after}");
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. The §7.6 handshake contract — closure, currentDims, refusals, and the fence
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_offset_face_handshake_and_fence() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xE));
    let (body, snap) = build_box(&mut rt, sid, 25.0).await;
    let (picks, _) = head_faces(&mut rt, body).await;
    let top = top_face(&picks).topo_key.clone();

    // (a) `Offset`: the closure is the pick alone, flagged `picked`, on the target
    //     body, echoing the head it answered against.
    let plain = prepare(&wm, snap, body, &[&top], true, OffsetDistanceType::Offset)
        .await
        .expect("PrepareOffsetFace answers");
    assert!(
        plain.refusal.is_none(),
        "a box cap is in V1 scope: {plain:?}"
    );
    assert_eq!(plain.snapshot_id, snap.0, "the head is ECHOED");
    assert_eq!(plain.target_body_id, body_id_wire(body));
    assert_eq!(plain.faces.len(), 1);
    assert_eq!(plain.faces[0].topo_key, top);
    assert!(plain.faces[0].picked, "the pick is flagged `picked`");
    assert!(
        plain.faces[0].descriptor.is_some() && plain.faces[0].anchor.is_some(),
        "the answer carries §10 descriptor + anchor EVIDENCE"
    );
    // Invariant 2: evidence only — the verb never mints, so nothing here is an id.
    assert!(
        !plain.faces[0].topo_key.starts_with("el_"),
        "a TopoKey is snapshot-scoped evidence, never an ElementId"
    );

    // (b) `Total`: chain OFF ⇒ the box's bottom cap is the unique opposite and
    //     `currentDims.thickness` is the CURRENT 25 the absolute value edits from.
    let total = prepare(&wm, snap, body, &[&top], false, OffsetDistanceType::Total)
        .await
        .expect("PrepareOffsetFace answers");
    assert!(
        total.refusal.is_none(),
        "a clean box has a unique opposite: {total:?}"
    );
    let opposite = total
        .opposite_face
        .as_ref()
        .expect("Total resolves the opposite cap");
    assert_ne!(opposite.topo_key, top, "the opposite is the OTHER cap");
    assert!(
        !opposite.picked,
        "the opposite face is never part of the operative set"
    );
    let thickness = total
        .current_dims
        .thickness
        .expect("Total reports currentDims.thickness");
    assert!(
        (thickness - 25.0).abs() < 1e-6,
        "the seed is the CURRENT thickness 25, got {thickness}"
    );

    // (c) A REFUSAL is an answer (`ok:true`), not an error: `Total` with the chain on
    //     is refused `chainOnTotal` — the tool can explain itself instead of failing.
    let refused = prepare(&wm, snap, body, &[&top], true, OffsetDistanceType::Total)
        .await
        .expect("a refusal still resolves Ok");
    let refusal = refused.refusal.as_ref().expect("chainOnTotal refusal");
    assert_eq!(refusal.code, "chainOnTotal");

    // (d) THE FENCE (SCHEMA §7.6): picks taken against a superseded head are REFUSED
    //     `STALE_PREVIEW`, never answered — this closure is about to be FROZEN into a
    //     record, so a stale ordinal would author the op against the wrong face.
    add_op(
        &mut rt,
        offset_record(
            OFFSET_REC,
            OffsetFaceParams {
                face_ids: vec![ElementId::new("el_placeholder")],
                faces: vec![face_ref(
                    body,
                    &ElementId::new("el_placeholder"),
                    anchor_of(plain.faces[0].anchor.as_ref().expect("anchor")),
                )],
                distance: Scalar::new(0.0),
                distance_type: OffsetDistanceType::Offset,
                chain_tangent_faces: true,
                opposite_face_id: None,
                opposite_face: None,
                target_body: body,
                extra: Default::default(),
            },
        ),
    );
    let moved = regen_all(&mut rt).await;
    let head = SnapshotId(moved.snapshot_id);
    assert_ne!(head, snap, "the edit published a NEW head");

    let err = prepare(&wm, snap, body, &[&top], true, OffsetDistanceType::Offset)
        .await
        .expect_err("a stale snapshotId must be REFUSED, not answered");
    assert!(
        matches!(
            err,
            EngineError::OpFailed {
                code: OpFailureCode::StalePreview,
                ..
            }
        ),
        "expected STALE_PREVIEW, got {err:?}"
    );
    // …and the SAME picks against the live head still answer.
    let fresh = prepare(&wm, head, body, &[&top], true, OffsetDistanceType::Offset)
        .await
        .expect("the live head answers");
    assert_eq!(fresh.snapshot_id, head.0);
    wm.shutdown().await;
    eprintln!(
        "PrepareOffsetFace PASS: closure/evidence + thickness {thickness} + chainOnTotal + STALE_PREVIEW fence"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. Determinism — two FRESH worker processes yield the identical offset body
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offset_face_deterministic_across_processes() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };

    async fn run_once(bin: PathBuf) -> (String, f64, String) {
        let wm = spawn_worker(bin).await;
        let mut rt = runtime_over(&wm);
        let sid = SketchId(Uuid::from_u128(0xF));
        let (body, snap) = build_box(&mut rt, sid, 25.0).await;
        let (picks, _) = head_faces(&mut rt, body).await;
        let top = top_face(&picks);
        let topo_key = top.topo_key.clone();
        let (face_ids, refs, _, _) = author_offset(
            &wm,
            &mut rt,
            snap,
            body,
            &[&topo_key],
            true,
            OffsetDistanceType::Offset,
        )
        .await;
        add_op(
            &mut rt,
            offset_record(
                OFFSET_REC,
                OffsetFaceParams {
                    face_ids,
                    faces: refs,
                    distance: Scalar::new(5.0),
                    distance_type: OffsetDistanceType::Offset,
                    chain_tangent_faces: true,
                    opposite_face_id: None,
                    opposite_face: None,
                    target_body: body,
                    extra: Default::default(),
                },
            ),
        );
        let report = regen_all(&mut rt).await;
        let snapshot = published(&report, "determinism offset");
        assert_eq!(snapshot.repair_summary.needs_repair_count, 0);
        let signature = snapshot
            .bodies
            .iter()
            .find(|b| b.body == body)
            .expect("the offset body is present")
            .signature
            .as_str()
            .to_string();
        let volume = exact_volume(&wm, body).await;
        wm.shutdown().await;
        (signature, volume, topo_key)
    }

    let (sig1, vol1, key1) = run_once(bin.clone()).await;
    let (sig2, vol2, key2) = run_once(bin).await;
    assert_eq!(
        key1, key2,
        "the picked TopoKey is itself deterministic across fresh processes"
    );
    assert_eq!(
        sig1, sig2,
        "the offset body's geometry signature is identical across fresh processes"
    );
    assert!(
        (vol1 - vol2).abs() < 1e-9,
        "and the volume: {vol1} vs {vol2}"
    );
    assert!((vol1 - 12_000.0).abs() < 1e-6, "and exact: {vol1}");
    eprintln!("Determinism PASS: signature {sig1} stable across two worker processes");
}
