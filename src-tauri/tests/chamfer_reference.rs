//! **Chamfer `referenceFaces`** (SCHEMA §7.3 / §9, kernel-hardening WP-F) against
//! the REAL C++ OCCT worker, driven through the app's [`DocumentRuntime`] exactly
//! like `chamfer_angle.rs`.
//!
//! The defect: an asymmetric chamfer measured `radius` on "the adjacent face with
//! the smaller SNAPSHOT-SCOPED ordinal", so an upstream feature that reordered the
//! face map silently MIRRORED the chamfer's legs — same removed volume, same face
//! count, every existing gate green. The fix persists the choice as a typed ref.
//!
//! * `a_typed_reference_face_survives_an_upstream_face_map_reorder` — the pair is
//!   authored from `PrepareEdgeOp`'s new `adjacentFaces`, the 4 mm leg lands on the
//!   TOP (the face the legacy rule would NOT have picked), and it is STILL on the
//!   top after an upstream hole reorders the face map. Measured as the top face's
//!   exact `GProp` area, not a status flag.
//! * `a_legacy_asymmetric_chamfer_halts_needs_repair_once_an_upstream_edit_lands` —
//!   a record with `distance2` and no pairs replays unchanged with no edit, halts
//!   `needsRepair` with the op-built `legacyReferenceFace` reason once anything
//!   upstream is edited, and is repaired by CREATING the pair on the empty slot.
//! * `a_fillet_flip_to_an_asymmetric_chamfer_without_pairs_is_refused` — the core
//!   refuses it by name before any regen is issued.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly (CI sets `ONECAD_REQUIRE_WORKER=1` to make that a hard failure).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ChamferParams, ChamferReferenceFace, ExtrudeMode, ExtrudeParams, HoleParams,
    HoleType, KnownOperation, Operation, OperationRecord, PlaneKind, SketchOpParams,
    SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::repair::RepairReason;
use onecad_core::document::variables::Scalar;
use onecad_core::document::Document;
use onecad_core::edit::{EditCommand, InputPath, InputRef};
use onecad_core::history::Timeline;
use onecad_core::ids::{
    BodyId, ConstraintId, DocumentId, ElementId, EntityId, RecordId, RegionId, SketchId,
    SnapshotId, TopoKey,
};
use onecad_core::io::container::{ContainerCaches, ContainerWriter, SaveMeta};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest, ResolveOutcome,
    ResolveRef, ResolveRequest,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport, RepairSeams};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::{sketch_wire, EdgeOpMode, EdgeOpPick, FaceAddress};
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, FaceBoundaryProjection, MeshProvider, SolverEngine,
    WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors chamfer_angle.rs)
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

async fn regen_from(rt: &mut DocumentRuntime, from: usize) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from }, CancelToken::new())
        .await
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;
const CHAMFER: u128 = 0xF00;
const HOLE: u128 = 0xF10;

/// The stock: sketch (0,0)-(40,20) on the XY frame, extruded 10.
///
/// The frame maps sketch `u → world +Y`, `v → world −X` (see [`xy_plane_ref`]), so
/// the solid is `x ∈ [−20, 0], y ∈ [0, 40], z ∈ [0, 10]` — the same box
/// `protocol/fixtures/chamfer_reference_face.ndjson` measures its ordinals on.
const BOX_U: f64 = 40.0;
const BOX_V: f64 = 20.0;
const BOX_H: f64 = 10.0;
/// Area of the +Z top face on the plain box: 40 × 20.
const TOP_AREA: f64 = 800.0;
/// The chamfer: `radius` 4 measured ON THE TOP, `distance2` 1 on the −X wall.
const CHAMFER_RADIUS: f64 = 4.0;
const CHAMFER_DISTANCE2: f64 = 1.0;
/// Top area after the chamfer, when `radius` is measured on the TOP: the chamfer
/// eats a 4 mm strip along the full 40 mm edge.
const TOP_AREA_CHAMFERED: f64 = TOP_AREA - CHAMFER_RADIUS * BOX_U;
/// …and the top area the LEGACY (smaller-ordinal, −X wall) reading would leave.
/// Present only so the assertion message can name what a regression looks like.
const TOP_AREA_MIRRORED: f64 = TOP_AREA - CHAMFER_DISTANCE2 * BOX_U;

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (verbatim shapes from chamfer_angle.rs)
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

fn rect_sketch(sid: SketchId, base: u128, w: f64, h: f64) -> Sketch {
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
                region: RegionId::new(""), // empty ⇒ V1 first-region fallback
                region_identity_version: None,
                region_anchor: None,
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

fn anchored_ref(body: BodyId, element: &ElementId, kind: ElementKind, at: Vec3) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: element.clone(),
            kind,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: at,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    }
}

/// The asymmetric chamfer this file is about. `reference` names the adjacent face
/// `radius` is measured on; `None` builds the LEGACY shape (which the session
/// refuses to author — see `legacy_chamfer_params`).
fn chamfer_params(
    body: BodyId,
    edge: &ElementId,
    edge_at: Vec3,
    reference: Option<(&ElementId, Vec3)>,
) -> ChamferParams {
    let (pairs, refs) = match reference {
        None => (Vec::new(), Vec::new()),
        Some((face, face_at)) => (
            vec![ChamferReferenceFace {
                edge_id: edge.clone(),
                face_id: face.clone(),
            }],
            vec![anchored_ref(body, face, ElementKind::Face, face_at)],
        ),
    };
    ChamferParams {
        radius: Scalar::new(CHAMFER_RADIUS),
        distance2: Some(Scalar::new(CHAMFER_DISTANCE2)),
        angle_deg: None,
        edge_ids: vec![edge.clone()],
        edges: vec![anchored_ref(body, edge, ElementKind::Edge, edge_at)],
        reference_faces: pairs,
        reference_face_refs: refs,
        chain_tangent_edges: false,
        tangent_closure_version: None,
        extra: Default::default(),
    }
}

fn chamfer_record(rec: u128, params: ChamferParams) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Chamfer",
        Operation::Known(KnownOperation::Chamfer(params)),
    )
}

/// A Ø6 blind hole 10 mm deep into the `−Y` end wall — geometrically disjoint from
/// the chamfered edge (`x = −20`, `z = 10`) and from the chamfer's two faces, but
/// enough to REORDER the body's face map. That reorder is the whole defect.
fn hole_record(rec: u128, body: BodyId, face: &ElementId, at: Vec3) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Hole",
        Operation::Known(KnownOperation::Hole(HoleParams {
            target_body: body,
            face: anchored_ref(body, face, ElementKind::Face, at),
            point: at,
            hole_type: HoleType::Simple,
            diameter: Scalar::new(6.0),
            depth: Some(Scalar::new(10.0)),
            cb_diameter: None,
            cb_depth: None,
            cs_diameter: None,
            cs_angle_deg: None,
            thread: None,
            result_policy_version: Some(2),
            extra: Default::default(),
        })),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 helpers — used ONLY to LOCATE picks; every measured number is kernel-exact
// ─────────────────────────────────────────────────────────────────────────────

const SEC_EDGE_RANGES: u32 = 7;
const SEC_EDGE_POSITIONS: u32 = 8;
const SEC_EDGE_ID_OFFS: u32 = 9;
const SEC_EDGE_ID_CHARS: u32 = 10;

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

fn edge_polyline_stats(blob: &[u8], base: usize, first: usize, count: usize) -> (Vec3, [f64; 3]) {
    let (mut lo, mut hi, mut sum) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3], [0.0; 3]);
    for point in first..first + count {
        let offset = base + point * 12;
        let value = [
            f32_le(blob, offset) as f64,
            f32_le(blob, offset + 4) as f64,
            f32_le(blob, offset + 8) as f64,
        ];
        for axis in 0..3 {
            lo[axis] = lo[axis].min(value[axis]);
            hi[axis] = hi[axis].max(value[axis]);
            sum[axis] += value[axis];
        }
    }
    let n = count as f64;
    (
        Vec3::new_unchecked(sum[0] / n, sum[1] / n, sum[2] / n),
        [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
    )
}

/// The box's TOP ∩ `−X` wall edge: the Y-running edge at `x = bbox_min.x`,
/// `z = bbox_max.z`. Returns its snapshot-scoped `TopoKey` and polyline centroid.
///
/// This is the same edge the WP-F fixture chamfers (`e:10`, centre `(−20, 20, 10)`);
/// its two adjacent faces are the `−X` wall and the TOP, and the LEGACY rule
/// measured `radius` on the wall.
fn top_x_min_edge(view: &MeshHeaderView, blob: &[u8]) -> (String, Vec3) {
    assert!(view.has_edges(), "MESH1 must carry edges for the pick");
    let ranges = view.section(SEC_EDGE_RANGES).expect("EDGE_RANGES");
    let positions = view.section(SEC_EDGE_POSITIONS).expect("EDGE_POSITIONS");
    let keys = id_table(
        view,
        blob,
        SEC_EDGE_ID_OFFS,
        SEC_EDGE_ID_CHARS,
        view.edge_count as usize,
    );
    let (x_min, z_max) = (f64::from(view.bbox_min[0]), f64::from(view.bbox_max[2]));
    let (rbase, pbase) = (ranges.offset as usize, positions.offset as usize);
    let mut found: Option<(String, Vec3)> = None;
    for (edge, key) in keys.iter().enumerate() {
        let first = u32_le(blob, rbase + edge * 8) as usize;
        let count = u32_le(blob, rbase + edge * 8 + 4) as usize;
        if count == 0 {
            continue;
        }
        let (centroid, span) = edge_polyline_stats(blob, pbase, first, count);
        // Y-running, at the −X face, on the top plane.
        if span[1] <= span[0] || span[1] <= span[2] {
            continue;
        }
        if (centroid.x - x_min).abs() > 1e-3 || (centroid.z - z_max).abs() > 1e-3 {
            continue;
        }
        assert!(
            found.is_none(),
            "exactly one top ∩ −X edge is expected on a box"
        );
        assert!(
            (span[1] - BOX_U).abs() < 1e-3,
            "the picked edge spans the full 40 mm length, got {}",
            span[1]
        );
        found = Some((key.clone(), centroid));
    }
    found.expect("the top ∩ −X wall edge")
}

/// The `TopoKey` of the face lying in the plane `z = z_top` at the given snapshot.
/// The viewport analogue: the user picks that face, and a pick IS a snapshot-scoped
/// TopoKey until it is promoted.
async fn top_plane_topo_key(
    wm: &WorkerManager,
    body: BodyId,
    snapshot: SnapshotId,
    z_top: f64,
) -> String {
    for ordinal in 1..=32u32 {
        let key = format!("f:{ordinal}");
        let Some(info) = wm
            .query_element_by_topo_key(snapshot, body, &key)
            .await
            .expect("QueryElement by topoKey")
        else {
            break;
        };
        if info.kind == "face"
            && info.has_normal
            && (info.normal[2].abs() - 1.0).abs() < 1e-6
            && (info.center[2] - z_top).abs() < 1e-6
        {
            return key;
        }
    }
    panic!("a face in the z = {z_top} plane")
}

/// The seed edge's adjacent faces on `snapshot`, derived INDEPENDENTLY of the
/// answer under test: re-bind the chamfer's stored typed edge ref through the §10
/// ladder, then ask `PrepareEdgeOp` on the `{bodyId, topoKey}` it answers with.
///
/// This is the same two-stage derivation SCHEMA §9 mandates, written out by hand so
/// the assertion compares the runtime's answer against the head's real adjacency
/// rather than against itself.
async fn seed_edge_adjacent_faces(stock: &Stock, snapshot: SnapshotId) -> Vec<String> {
    let bound = onecad_core::regen::GeometryEngine::resolve_refs(
        &stock.wm,
        ResolveRequest {
            snapshot_id: snapshot,
            refs: vec![ResolveRef {
                ref_id: "probe".into(),
                element: anchored_ref(stock.body, &stock.edge, ElementKind::Edge, stock.edge_at),
            }],
        },
    )
    .await
    .expect("the seed edge's own ladder dry run");
    let key = match &bound[0].outcome {
        ResolveOutcome::AutoBind {
            topo_key: Some(key),
            ..
        } => key.as_str().to_string(),
        other => panic!("the seed edge must re-bind on the head, got {other:?}"),
    };
    let prepared = FaceBoundaryProjection::prepare_edge_op(
        &stock.wm,
        snapshot,
        EdgeOpMode::Chamfer,
        &[EdgeOpPick {
            body: Some(stock.body),
            address: FaceAddress::TopoKey(&key),
        }],
        false,
    )
    .await
    .expect("PrepareEdgeOp on the re-bound seed edge");
    prepared
        .edges
        .iter()
        .find(|e| e.picked)
        .and_then(|e| e.adjacent_faces.clone())
        .expect("SCHEMA §7.6 `adjacentFaces` on the picked entry")
}

/// The centre of the `−Y` end wall (`y = bbox_min.y`) — where the upstream hole is
/// drilled. Derived from the bbox rather than from a face table: the hole only has
/// to land on that wall, and the wall is axis-aligned.
fn minus_y_wall_centre(view: &MeshHeaderView) -> Vec3 {
    Vec3::new_unchecked(
        f64::from(view.bbox_min[0] + view.bbox_max[0]) / 2.0,
        f64::from(view.bbox_min[1]),
        f64::from(view.bbox_min[2] + view.bbox_max[2]) / 2.0,
    )
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

/// The EXACT `GProp` area of the face `element` names at the current head — the
/// kernel's own measurement (`ElementInfoDto::magnitude`), not a tessellation
/// estimate.
async fn face_area(wm: &WorkerManager, snapshot: SnapshotId, body: BodyId, element: &str) -> f64 {
    let info = wm
        .query_element(snapshot, body, element)
        .await
        .expect("QueryElement")
        .unwrap_or_else(|| panic!("{element} is present at the head"));
    assert_eq!(info.kind, "face", "{element} is a face");
    info.magnitude
}

// ─────────────────────────────────────────────────────────────────────────────
// Stock: the 40×20×10 box with the top ∩ −X edge and both its adjacent faces
// promoted to durable ElementIds
// ─────────────────────────────────────────────────────────────────────────────

struct Stock {
    wm: WorkerManager,
    rt: DocumentRuntime,
    body: BodyId,
    /// The chamfered edge and its pick anchor.
    edge: ElementId,
    edge_at: Vec3,
    /// The +Z top face — the reference face this file authors.
    top: ElementId,
    top_at: Vec3,
    /// The −Y end wall — where the upstream hole goes.
    end_wall: ElementId,
    end_wall_at: Vec3,
}

/// Builds the box, drives the §7.6 handshake on the chamfer edge, and asserts the
/// two fields WP-F adds to it before promoting anything.
async fn stock_box(bin: PathBuf) -> Stock {
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, BOX_U, BOX_V)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_A, sa, BOX_H));
    let report = regen_from(&mut rt, 0).await;
    let snapshot = SnapshotId(report.snapshot_id);
    assert_eq!(
        published(&report, "stock box")
            .repair_summary
            .needs_repair_count,
        0
    );
    let body = body_of(EXTRUDE_A);

    let blob = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&blob).expect("stock box MESH1 validates");
    assert_eq!(view.face_count, 6, "the stock is a sharp six-face box");
    let (edge_key, edge_at) = top_x_min_edge(&view, &blob);
    let end_wall_at = minus_y_wall_centre(&view);

    // ── SCHEMA §7.6 (WP-F): the accepted edge reports its contour and its adjacent
    // faces as TopoKeys, face-ordinal ASCENDING. This is the list the frontend
    // authors the `referenceFaces` pair from.
    let prepared = wm
        .prepare_edge_op(
            snapshot,
            EdgeOpMode::Chamfer,
            &[EdgeOpPick {
                body: Some(body),
                address: FaceAddress::TopoKey(&edge_key),
            }],
            false,
        )
        .await
        .expect("PrepareEdgeOp on the chamfer edge");
    assert!(prepared.refusal.is_none(), "a box edge is chamferable");
    assert_eq!(
        prepared.edges.len(),
        1,
        "chainTangentEdges:false ⇒ one edge"
    );
    let evidence = &prepared.edges[0];
    assert_eq!(
        evidence.contour,
        Some(0),
        "the single picked edge seeds contour 0"
    );
    let adjacent = evidence
        .adjacent_faces
        .clone()
        .expect("SCHEMA §7.6 `adjacentFaces` on every accepted edge");
    assert_eq!(
        adjacent.len(),
        2,
        "a manifold box edge has exactly two adjacent faces, got {adjacent:?}"
    );

    // Which of the two is the TOP is a MEASUREMENT, not an assumption.
    let mut top_index = None;
    for (i, key) in adjacent.iter().enumerate() {
        let info = wm
            .query_element_by_topo_key(snapshot, body, key)
            .await
            .expect("QueryElement by topoKey")
            .unwrap_or_else(|| panic!("{key} resolves on the snapshot it was reported for"));
        if info.has_normal
            && (info.normal[2].abs() - 1.0).abs() < 1e-6
            && (info.center[2] - f64::from(view.bbox_max[2])).abs() < 1e-6
        {
            // `magnitude` is the face's exact GProp AREA — verified here on the
            // plain box before anything relies on it. (The bottom shares the +Z
            // surface normal, so the plane is pinned by the centre too.)
            assert!(
                (info.magnitude - TOP_AREA).abs() < 1e-6,
                "the +Z face of a 40×20 box measures {TOP_AREA} mm², got {}",
                info.magnitude
            );
            top_index = Some(i);
        }
    }
    let top_index = top_index.expect("one of the two adjacent faces is the +Z top");
    // The legacy rule took `adjacentFaces[0]` (the smaller face ordinal). Pinning
    // that the TOP is NOT that face is what makes the rest of this file a real
    // test: the pair we author names the face the old code would not have picked.
    // Provenance: measured on this model, OCCT 8.0.1, 2026-09-03 — the same
    // ordering `protocol/fixtures/chamfer_reference_face.ndjson` records (f:3 −X
    // wall < f:6 top).
    assert_eq!(
        top_index, 1,
        "the top is expected to be the LARGER-ordinal adjacent face, so the legacy \
         rule measured `radius` on the −X wall; got adjacentFaces={adjacent:?}"
    );

    let anchor_of = |at: Vec3| {
        Some(AnchorIntent {
            world_point: at,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        })
    };
    let top_at = Vec3::new_unchecked(
        f64::from(view.bbox_min[0] + view.bbox_max[0]) / 2.0,
        f64::from(view.bbox_min[1] + view.bbox_max[1]) / 2.0,
        f64::from(view.bbox_max[2]),
    );
    let promoted = rt
        .promote_selection(
            snapshot,
            body,
            vec![
                (TopoKey::new(&edge_key), anchor_of(edge_at)),
                (TopoKey::new(&adjacent[top_index]), anchor_of(top_at)),
            ],
        )
        .await
        .unwrap_or_else(|error| panic!("promote the edge + its top adjacent face: {error}"));
    assert_eq!(promoted.len(), 2);
    assert_eq!(promoted[0].kind, "edge");
    assert_eq!(promoted[1].kind, "face");
    let edge = ElementId::new(&promoted[0].element_id);
    let top = ElementId::new(&promoted[1].element_id);

    // The −Y end wall the upstream hole is drilled into, promoted the same way.
    let wall_key = wall_topo_key(&wm, snapshot, body, end_wall_at).await;
    let promoted_wall = rt
        .promote_selection(
            snapshot,
            body,
            vec![(TopoKey::new(&wall_key), anchor_of(end_wall_at))],
        )
        .await
        .unwrap_or_else(|error| panic!("promote the −Y end wall: {error}"));
    let end_wall = ElementId::new(&promoted_wall[0].element_id);

    Stock {
        wm,
        rt,
        body,
        edge,
        edge_at,
        top,
        top_at,
        end_wall,
        end_wall_at,
    }
}

/// The `TopoKey` of the planar face whose bbox centre is `at` — the box's `−Y` end
/// wall. Scanned over the body's face ordinals rather than read off a mesh table,
/// because `QueryElement` reports the kernel's own centre and normal.
async fn wall_topo_key(wm: &WorkerManager, snapshot: SnapshotId, body: BodyId, at: Vec3) -> String {
    for ordinal in 1..=6u32 {
        let key = format!("f:{ordinal}");
        let Some(info) = wm
            .query_element_by_topo_key(snapshot, body, &key)
            .await
            .expect("QueryElement by topoKey")
        else {
            continue;
        };
        // `normal` is the OCCT SURFACE normal, not an outward-oriented one (the
        // y = 0 and y = 40 walls report opposite signs), so the plane is matched by
        // AXIS plus the face's own bbox centre.
        if !info.has_normal || (info.normal[1].abs() - 1.0).abs() > 1e-6 {
            continue;
        }
        if (info.center[1] - at.y).abs() < 1e-6 {
            return key;
        }
    }
    panic!("the box has a −Y end wall")
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) A typed reference face survives an upstream face-map reorder
// ─────────────────────────────────────────────────────────────────────────────

/// The flagship. The pair authored from `PrepareEdgeOp`'s `adjacentFaces` puts the
/// 4 mm leg on the TOP, and it STAYS on the top after a Ø6 hole is inserted
/// UPSTREAM in a wall the chamfer never touches — the exact edit that reorders the
/// face map and, before WP-F, silently mirrored the legs.
///
/// Measured as the top face's exact `GProp` area: 800 − 4·40 = 640 with the pair,
/// 800 − 1·40 = 760 under the legacy reading. Volume, face count and every
/// pre-WP-F check are identical between the two, which is why this is the
/// assertion that had to exist.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_typed_reference_face_survives_an_upstream_face_map_reorder() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let mut stock = stock_box(bin).await;

    add_op(
        &mut stock.rt,
        chamfer_record(
            CHAMFER,
            chamfer_params(
                stock.body,
                &stock.edge,
                stock.edge_at,
                Some((&stock.top, stock.top_at)),
            ),
        ),
    );
    let report = regen_from(&mut stock.rt, 0).await;
    let snapshot = SnapshotId(report.snapshot_id);
    assert_eq!(
        published(&report, "typed chamfer")
            .repair_summary
            .needs_repair_count,
        0,
        "a freshly authored pair binds without repair"
    );
    assert!(
        report.failed_steps.is_empty(),
        "the chamfer step must not fail: {:?}",
        report.failed_steps
    );
    let area = face_area(&stock.wm, snapshot, stock.body, stock.top.as_str()).await;
    assert!(
        (area - TOP_AREA_CHAMFERED).abs() < 1e-6,
        "`radius` is measured on the face the PAIR names: expected {TOP_AREA_CHAMFERED} mm² \
         on the top, got {area} (the legacy smaller-ordinal reading would leave \
         {TOP_AREA_MIRRORED})"
    );

    // ── The reorder. Roll back to just after the extrude and INSERT the hole, so
    // the chamfer replays as a post-edit step (SCHEMA §7.2 `editedFrom` = 2).
    stock
        .rt
        .apply(EditCommand::SetRollback { cursor: 2 })
        .expect("roll back to the extrude");
    add_op(
        &mut stock.rt,
        hole_record(HOLE, stock.body, &stock.end_wall, stock.end_wall_at),
    );
    stock
        .rt
        .apply(EditCommand::SetRollback { cursor: 4 })
        .expect("re-apply the whole timeline");
    let report = regen_from(&mut stock.rt, 2).await;
    let snapshot = SnapshotId(report.snapshot_id);
    assert!(
        report.failed_steps.is_empty(),
        "neither the hole nor the chamfer may fail: {:?}",
        report.failed_steps
    );
    assert_eq!(
        published(&report, "post-reorder")
            .repair_summary
            .needs_repair_count,
        0,
        "a TYPED pair resolves through the ladder, never through an ordinal"
    );
    let area = face_area(&stock.wm, snapshot, stock.body, stock.top.as_str()).await;
    assert!(
        (area - TOP_AREA_CHAMFERED).abs() < 1e-6,
        "the reference face survived the face-map reorder: expected \
         {TOP_AREA_CHAMFERED} mm², got {area} ({TOP_AREA_MIRRORED} means the legs \
         mirrored — the WP-F defect)"
    );

    stock.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (b) The LEGACY record: replay unchanged, halt on an upstream edit, repair by
//     CREATING the pair
// ─────────────────────────────────────────────────────────────────────────────

/// A LEGACY asymmetric chamfer — `distance2` and NO pairs — is a record this build
/// cannot AUTHOR: `AddOperation` refuses it, an update that introduces the asymmetry
/// refuses it, and an update that CLEARS the pairs refuses it too (that last one is
/// the silent-regression guard). It reaches a session exactly one way: out of a
/// document an OLDER build wrote.
///
/// So this reproduces it the honest way — write the container by hand and open it
/// through the production path — rather than through any sanctioned command. Mirrors
/// `scheduler_commit::legacy_container_without_sketch_records_extrudes_after_open`.
fn legacy_container(path: &Path, sketch: &Sketch, body: BodyId, edge: &ElementId, edge_at: Vec3) {
    let mut doc = Document::new(DocumentId::new());
    doc.sketches.insert(sketch.id, sketch.clone());
    let mut legacy = chamfer_params(body, edge, edge_at, None);
    // Belt and braces: `chamfer_params(.., None)` already emits no pairs, and the
    // record goes to disk without passing an `EditSession`.
    legacy.reference_faces.clear();
    legacy.reference_face_refs.clear();
    doc.timeline = Timeline::from_records(vec![
        sketch_record(SKETCH_A, sketch),
        extrude_record(EXTRUDE_A, sketch.id, BOX_H),
        chamfer_record(CHAMFER, legacy),
    ]);
    ContainerWriter::save(path, &doc, &ContainerCaches::none(), &save_meta())
        .expect("write the legacy container");
}

/// [`ContainerWriter::save`] metadata (mirrors `container.rs`'s test `meta()`).
fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-8.0.1".into()),
        created: "2026-09-04T00:00:00Z".into(),
        modified: "2026-09-04T00:00:00Z".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_asymmetric_chamfer_halts_needs_repair_once_an_upstream_edit_lands() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    // Worker A locates the picks on a real box (and runs the §7.6 assertions).
    let mut stock = stock_box(bin.clone()).await;
    let chamfer = RecordId(Uuid::from_u128(CHAMFER));
    let sketch = rect_sketch(SketchId(Uuid::from_u128(0xA)), 0x1000, BOX_U, BOX_V);
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("legacy.onecad");
    legacy_container(&path, &sketch, stock.body, &stock.edge, stock.edge_at);
    stock.wm.shutdown().await;

    // Worker B opens that container through the PRODUCTION path — the only way a
    // legacy record can enter a session under this build.
    let wm = spawn_worker(bin).await;
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    stock.rt = DocumentRuntime::open(&path, engine, meshes, solver).expect("open legacy");
    stock.wm = wm;

    // ── EVERY LANE HALTS. The ordinal fallback is GONE (WP-F review, 2026-09-04):
    // an uncovered contour halts regardless of SCHEMA §7.2 `editedFrom`, because
    // gating it on the edit claim made the guard fire on open and vanish on redo.
    // Both replay lanes are asserted, because they make OPPOSITE `editedFrom`
    // claims: `RevertToEnd` makes none at all, and `ToEnd { from: 0 }` — the lane a
    // REOPEN uses — claims `editedFrom: 0` (kernel-hardening WP-A).
    let halted_in = |rt: &DocumentRuntime, lane: &str| {
        let items = rt.repair_items().to_vec();
        let want = format!("{chamfer}.input1");
        let item = items
            .iter()
            .find(|i| i.ref_id == want)
            .unwrap_or_else(|| panic!("{lane}: the legacy chamfer halts, got {items:?}"));
        assert_eq!(
            item.reason,
            RepairReason::LegacyReferenceFace,
            "{lane}: the op-built reason, not a ladder outcome"
        );
        assert_eq!(
            item.seed_edge_id.as_ref().map(ElementId::as_str),
            Some(stock.edge.as_str()),
            "{lane}: the item names the contour edge the pair must be keyed by"
        );
    };

    let report = stock
        .rt
        .run_regen(RegenRequest::RevertToEnd { from: 0 }, CancelToken::new())
        .await;
    let _ = published(&report, "legacy replay, no edit claim");
    halted_in(&stock.rt, "RevertToEnd{from:0} (no editedFrom claim)");

    // A reopen replays `ToEnd { from: 0 }`. SCHEMA §7.3 records the halt-once
    // migration path: no evidence of the intended face was ever persisted, so a
    // deterministic halt the user answers with one pick is the honest outcome.
    let report = regen_from(&mut stock.rt, 0).await;
    let _ = published(&report, "legacy replay, reopen-style");
    halted_in(&stock.rt, "ToEnd{from:0} (the reopen lane)");

    // ── The upstream edit. Now the ordinal-derived choice is untrustworthy, so the
    // step HALTS instead of silently mirroring the legs.
    stock
        .rt
        .apply(EditCommand::SetRollback { cursor: 2 })
        .expect("roll back to the extrude");
    add_op(
        &mut stock.rt,
        hole_record(HOLE, stock.body, &stock.end_wall, stock.end_wall_at),
    );
    stock
        .rt
        .apply(EditCommand::SetRollback { cursor: 4 })
        .expect("re-apply the whole timeline");
    let report = regen_from(&mut stock.rt, 2).await;
    let snapshot = SnapshotId(report.snapshot_id);

    let items = stock.rt.repair_items().to_vec();
    // N = 1 edge ref, 0 pairs ⇒ the pair the repair creates lands on slot 1.
    let want = format!("{chamfer}.input1");
    let item = items.iter().find(|i| i.ref_id == want).unwrap_or_else(|| {
        panic!("the chamfer step halts needsRepair after the upstream edit, got {items:?}")
    });
    assert_eq!(
        item.reason,
        RepairReason::LegacyReferenceFace,
        "the op-built reason, not a ladder outcome"
    );
    assert_eq!(
        item.seed_edge_id.as_ref().map(ElementId::as_str),
        Some(stock.edge.as_str()),
        "the item names the contour edge the pair must be keyed by"
    );
    assert_eq!(
        item.candidates.len(),
        2,
        "the seed edge's two adjacent faces, at a deliberate tie"
    );
    // SCHEMA §9 encodes the empty slot as `elementId: ""`, and a reader MUST treat
    // that and an absent id alike — `wire::parse_needs_repair` normalizes it, so
    // nothing downstream ever sees `Some("")` masquerading as a last-known id.
    assert!(
        item.element_id.is_none(),
        "the slot the pair will occupy is EMPTY — the repair is a create; got {:?}",
        item.element_id
    );

    // ── The §7.5 dry run answers the SAME slot WITHOUT running a ladder on it and
    // WITHOUT echoing the item's own candidates (their TopoKeys are ordinals of the
    // discarded scratch state, and §7.5 forbids promoting those).
    //
    // The seed edge is NOT addressable on the head by id: the halted step never
    // committed, so the binding it resolved on the scratch state died with it and
    // `PrepareEdgeOp{elementId}` answers `REF_UNRESOLVED` (measured 2026-09-04).
    // SCHEMA §9 therefore re-binds the seed edge through the §10 ladder from the
    // record's OWN stored typed edge ref first, and addresses the adjacency
    // handshake by the `{bodyId, topoKey}` that ladder answers with.
    let resolved = stock
        .rt
        .resolve_refs_with(
            ResolveRequest {
                snapshot_id: snapshot,
                refs: vec![ResolveRef {
                    ref_id: want.clone(),
                    element: ElementRef {
                        primary: None,
                        intent: None,
                        anchor: None,
                        extra: Default::default(),
                    },
                }],
            },
            Some(RepairSeams {
                faces: &stock.wm,
                elements: &stock.wm,
            }),
        )
        .await
        .expect("the dry run answers the empty pair slot");
    let ResolveOutcome::NeedsRepair(evidence) = &resolved[0].outcome else {
        panic!("an empty pair slot is NeedsRepair, never a bind: {resolved:?}")
    };
    assert_eq!(
        evidence.reason,
        RepairReason::LegacyReferenceFace,
        "the reason tells the client this is a CREATE, not a rebind"
    );
    assert_eq!(
        evidence.seed_edge_id.as_ref().map(ElementId::as_str),
        Some(stock.edge.as_str()),
        "the pair's key rides back verbatim"
    );
    assert_eq!(
        resolved[0].body_id,
        Some(stock.body),
        "the client promotes the chosen face against the chamfer's own body"
    );

    // EXACTLY the seed edge's two adjacent faces ON THE HEAD, at a deliberate tie.
    let offered: Vec<String> = evidence
        .candidates
        .iter()
        .map(|c| c.topo_key.as_str().to_string())
        .collect();
    assert_eq!(
        offered.len(),
        2,
        "a manifold edge has two adjacent faces; got {offered:?}"
    );
    assert!(
        evidence
            .candidates
            .iter()
            .all(|c| c.score == 0.5 && c.margin == 0.0),
        "a deliberate tie: the user MUST choose"
    );
    let head_top_key =
        top_plane_topo_key(&stock.wm, stock.body, resolved[0].snapshot_id, BOX_H).await;
    // The head's own adjacency for that edge, derived independently of the answer.
    let head_adjacent = seed_edge_adjacent_faces(&stock, resolved[0].snapshot_id).await;
    assert_eq!(
        offered, head_adjacent,
        "the candidates are the seed edge's LIVE adjacency on the echoed head, in \
         list order — never the item's stale scratch ordinals"
    );
    // Every candidate is PRESENT on the snapshot it may be promoted against, and is
    // positioned at ITS OWN face centre — the panel highlights candidates there and
    // anchors the ref it creates there, so the seed EDGE's anchor (a point on BOTH
    // faces) would stack them and leave an anchor that never separates them again.
    for candidate in &evidence.candidates {
        let info = stock
            .wm
            .query_element_by_topo_key(
                resolved[0].snapshot_id,
                stock.body,
                candidate.topo_key.as_str(),
            )
            .await
            .expect("QueryElement by topoKey")
            .unwrap_or_else(|| {
                panic!(
                    "candidate {} is not present on the echoed snapshot",
                    candidate.topo_key.as_str()
                )
            });
        let at = [
            candidate.world_pos.x,
            candidate.world_pos.y,
            candidate.world_pos.z,
        ];
        assert_eq!(
            at,
            info.center,
            "candidate {} must be highlighted at its own face centre (elementInfo), \
             not at the seed edge's anchor {:?}",
            candidate.topo_key.as_str(),
            stock.edge_at
        );
        assert_ne!(
            at,
            [stock.edge_at.x, stock.edge_at.y, stock.edge_at.z],
            "…and never at the shared edge point"
        );
        assert_eq!(
            candidate.summary,
            format!("planar face, area~{:.0}mm2", info.magnitude),
            "the summary reads exactly like a worker-built candidate's"
        );
    }
    // The TOP is one of the two, at its measured centre — the number the panel
    // actually draws at.
    let top_centre = stock
        .wm
        .query_element_by_topo_key(resolved[0].snapshot_id, stock.body, &head_top_key)
        .await
        .expect("QueryElement by topoKey")
        .expect("the top face resolves on the echoed head")
        .center;
    assert!(
        evidence.candidates.iter().any(|c| {
            c.topo_key.as_str() == head_top_key
                && [c.world_pos.x, c.world_pos.y, c.world_pos.z] == top_centre
        }),
        "the top candidate carries the top face's centre {top_centre:?}"
    );

    // ── The repair itself, through the PANEL path: the user chooses one of the two
    // candidates the dry run offered, promotes THAT TopoKey against the ECHOED
    // snapshot, and the pair is CREATED on the empty slot. Nothing here is a
    // viewport scan — every id comes from the answer above.
    let top_key = offered
        .iter()
        .find(|key| **key == head_top_key)
        .cloned()
        .expect("the TOP is one of the two offered candidates");
    let top_info = stock
        .wm
        .query_element_by_topo_key(resolved[0].snapshot_id, stock.body, &top_key)
        .await
        .expect("QueryElement by topoKey")
        .expect("the top face resolves on the echoed head");
    let top_center =
        Vec3::new_unchecked(top_info.center[0], top_info.center[1], top_info.center[2]);
    let promoted = stock
        .rt
        .promote_selection(
            resolved[0].snapshot_id,
            stock.body,
            vec![(
                TopoKey::new(top_key),
                Some(AnchorIntent {
                    world_point: top_center,
                    surface_uv: None,
                    local_frame: None,
                    adjacency_hint: None,
                    extra: Default::default(),
                }),
            )],
        )
        .await
        .expect("promote the chosen reference face");
    let picked = ElementId::new(&promoted[0].element_id);
    stock
        .rt
        .apply(EditCommand::EditOperationInput {
            record: chamfer,
            path: InputPath::ChamferReferenceFace {
                index: 0,
                edge_id: Some(stock.edge.clone()),
            },
            reference: InputRef::Element(anchored_ref(
                stock.body,
                &picked,
                ElementKind::Face,
                top_center,
            )),
        })
        .expect("the §9 repair CREATES the pair on the empty slot");

    let report = regen_from(&mut stock.rt, 2).await;
    let snapshot = SnapshotId(report.snapshot_id);
    assert!(
        report.failed_steps.is_empty(),
        "the repaired chamfer must not fail: {:?}",
        report.failed_steps
    );
    assert_eq!(
        published(&report, "repaired chamfer")
            .repair_summary
            .needs_repair_count,
        0,
        "one pick closes the halt for good — the record is typed from now on"
    );
    let area = face_area(&stock.wm, snapshot, stock.body, picked.as_str()).await;
    assert!(
        (area - TOP_AREA_CHAMFERED).abs() < 1e-6,
        "the repaired record measures `radius` on the face the USER chose: expected \
         {TOP_AREA_CHAMFERED} mm², got {area}"
    );

    stock.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) The core refuses a flip that lands asymmetric without pairs — no worker
// ─────────────────────────────────────────────────────────────────────────────

/// The sanctioned Fillet⇄Chamfer swap may land on an asymmetric chamfer, and when
/// it does the SAME command must author the reference faces (SCHEMA §7.3). A flip
/// carrying `distance2` and no pairs is refused BY NAME by the single writer,
/// before any regen is issued — the same bar the `distance2`-on-a-Fillet refusal
/// meets. Pure core: no worker, no geometry.
#[test]
fn a_fillet_flip_to_an_asymmetric_chamfer_without_pairs_is_refused() {
    use onecad_core::document::body::BodyMeta;
    use onecad_core::document::record::FilletParams;
    use onecad_core::document::Document;
    use onecad_core::edit::session::DocumentSession;
    use onecad_core::history::Timeline;
    use onecad_core::ids::DocumentId;

    let body = BodyId(Uuid::from_u128(0xB0));
    let record = RecordId(Uuid::from_u128(0xF1));
    let edge = ElementId::new("el_edge");
    let at = Vec3::new_unchecked(-20.0, 20.0, 10.0);

    let mut doc = Document::new(DocumentId(Uuid::from_u128(0x5D)));
    doc.bodies.register(BodyMeta::new(body, "Body", record));
    doc.timeline = Timeline::from_records(vec![OperationRecord::new(
        record,
        0,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(2.0),
            edge_ids: vec![edge.clone()],
            edges: vec![anchored_ref(body, &edge, ElementKind::Edge, at)],
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    )]);
    let mut sess = DocumentSession::new(doc);

    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record,
            op: Operation::Known(KnownOperation::Chamfer(chamfer_params(
                body, &edge, at, None,
            ))),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("referenceFaces"),
        "the flip is refused by name: {err}"
    );

    // …and the SAME flip carrying the pair is accepted.
    let top = ElementId::new("el_top");
    sess.apply(EditCommand::UpdateOperationParams {
        record,
        op: Operation::Known(KnownOperation::Chamfer(chamfer_params(
            body,
            &edge,
            at,
            Some((&top, Vec3::new_unchecked(-10.0, 20.0, 10.0))),
        ))),
    })
    .expect("a flip that authors the pair is the sanctioned one");
}
