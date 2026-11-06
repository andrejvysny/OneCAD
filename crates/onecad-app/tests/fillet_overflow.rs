//! Edge-op **overflow "Auto"** (SCHEMA §7.3 Fillet/Chamfer `overflow` +
//! `overflowVersion`, §7.2 `FILLET_OVERFLOW_TRIMMED` / `CHAMFER_OVERFLOW_TRIMMED` /
//! `EDGE_OP_OVERFLOW_UNSUPPORTED`) end to end against the REAL C++ OCCT worker,
//! through the app's [`DocumentRuntime`] (the shared service every host drives).
//!
//! Box 20 × 30 × 4 (sketch + extrude); the picked edge is a TOP edge running along
//! the 30 mm side, so its side support is the 4 mm-tall face. A blend larger than
//! 4 mm consumes that face, which native OCCT refuses and `overflow: auto` builds:
//! fillet = cliff (volume 2085.812877), chamfer = extendedPlane (volume 1896).
//! Volumes are exact `QueryMassProperties` readings (frame-independent).
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly (CI sets `ONECAD_REQUIRE_WORKER=1` to make that a hard failure).

use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ChamferParams, EdgeOverflow, ExtrudeMode, ExtrudeParams, FilletParams,
    KnownOperation, Operation, OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
    EDGE_OVERFLOW_VERSION,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{
    BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::engine::Severity;
use onecad_core::regen::{
    CancelToken, Diagnostic, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest,
    ResolveOutcome, ResolveRef, ResolveRequest,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_app::document_runtime::{DocumentRuntime, RegenReport};
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::wire::sketch_wire;
use onecad_app::worker::{
    resolve_worker_path, ElementQuery, FaceBoundaryProjection, MeshProvider, PreviewEngine,
    SolverEngine, WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};
use tempfile::TempDir;

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

const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;
const OP_EDGE: u128 = 0xF00;
const BOX_VOLUME: f64 = 2400.0; // 20 × 30 × 4
const FILLET_11_2_VOLUME: f64 = 2085.812877;
const FILLET_2_VOLUME: f64 = 2374.247780;
const CHAMFER_6_2_VOLUME: f64 = 1896.0;
const VOLUME_TOL: f64 = 1e-3;

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (shapes from chamfer_angle.rs)
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

/// A typed element ref carrying the operated body + a world-point anchor — the
/// shape a real UI edge pick lowers to.
fn anchored_ref(body: BodyId, element: &ElementId, kind: ElementKind, at: Vec3) -> ElementRef {
    ElementRef {
        import_source_identity: None,
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

/// One-edge Fillet payload; `auto` opts into the overflow builder.
fn fillet_op(body: BodyId, edge: &ElementId, at: Vec3, radius: f64, auto: bool) -> Operation {
    Operation::Known(KnownOperation::Fillet(FilletParams {
        radius: Scalar::new(radius),
        edge_ids: vec![edge.clone()],
        edges: vec![anchored_ref(body, edge, ElementKind::Edge, at)],
        chain_tangent_edges: false,
        tangent_closure_version: None,
        overflow: auto.then_some(EdgeOverflow::Auto),
        overflow_version: auto.then_some(EDGE_OVERFLOW_VERSION),
        extra: Default::default(),
    }))
}

/// One-edge equal-leg Chamfer payload.
fn chamfer_op(body: BodyId, edge: &ElementId, at: Vec3, distance: f64, auto: bool) -> Operation {
    Operation::Known(KnownOperation::Chamfer(ChamferParams {
        radius: Scalar::new(distance),
        distance2: None,
        angle_deg: None,
        edge_ids: vec![edge.clone()],
        edges: vec![anchored_ref(body, edge, ElementKind::Edge, at)],
        reference_faces: Vec::new(),
        reference_face_refs: Vec::new(),
        chain_tangent_edges: false,
        tangent_closure_version: None,
        overflow: auto.then_some(EdgeOverflow::Auto),
        overflow_version: auto.then_some(EDGE_OVERFLOW_VERSION),
        extra: Default::default(),
    }))
}

fn edge_record(name: &str, op: Operation) -> OperationRecord {
    OperationRecord::new(RecordId(Uuid::from_u128(OP_EDGE)), 0, name, op)
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 helpers (edge picking + face count)
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

/// Signed tessellation volume (divergence theorem). Only used for the
/// preview-vs-commit EQUALITY: both sides tessellate the same shape at the same
/// LOD, so the curved cliff face cancels out of the comparison.
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

/// The TOP (max-z) edge that is longest, ties broken by smallest `(x, y)`: on
/// the 20 × 30 × 4 box that is a 30 mm edge whose side support is the 4 mm face.
fn top_edge_pick(view: &MeshHeaderView, blob: &[u8]) -> (String, Vec3) {
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
    let top = f64::from(view.bbox_max[2]);
    let (rbase, pbase) = (ranges.offset as usize, positions.offset as usize);
    let mut best: Option<(usize, f64, Vec3)> = None;
    for edge in 0..view.edge_count as usize {
        let first = u32_le(blob, rbase + edge * 8) as usize;
        let count = u32_le(blob, rbase + edge * 8 + 4) as usize;
        if count == 0 {
            continue;
        }
        let (centroid, span) = edge_polyline_stats(blob, pbase, first, count);
        if span[2] > 1e-3 || (centroid.z - top).abs() > 1e-3 {
            continue; // not a top edge
        }
        let length = span[0].max(span[1]);
        let better = best.as_ref().is_none_or(|(_, l, c)| {
            length > *l + 1e-6
                || ((length - *l).abs() <= 1e-6 && (centroid.x, centroid.y) < (c.x, c.y))
        });
        if better {
            best = Some((edge, length, centroid));
        }
    }
    let (edge, _, centroid) = best.expect("at least one top edge");
    (keys[edge].clone(), centroid)
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

/// Measured `(exact GProp volume, MESH1 face count)` of the live body.
async fn measure(rt: &mut DocumentRuntime, wm: &WorkerManager, body: BodyId) -> (f64, u32) {
    let mass = ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties");
    let blob = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&blob).expect("MESH1 validates");
    (mass.volume, view.face_count)
}

fn assert_volume(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() < VOLUME_TOL,
        "{what}: volume {got} != {want} (±{VOLUME_TOL})"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Stock: sharp box with the top edge and top face promoted to ElementIds
// ─────────────────────────────────────────────────────────────────────────────

struct Stock {
    wm: WorkerManager,
    rt: DocumentRuntime,
    body: BodyId,
    edge: ElementId,
    anchor: Vec3,
    /// The top face (the edge's adjacent face at max z) — the downstream reference.
    top_face: ElementId,
    top_anchor: Vec3,
}

fn anchor_intent(at: Vec3) -> Option<AnchorIntent> {
    Some(AnchorIntent {
        world_point: at,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    })
}

/// Builds the `a × b × h` box on a fresh worker (sketch + extrude), verifies it is
/// the sharp six-face solid, then promotes its longest top edge and top face.
async fn stock_box(bin: PathBuf, a: f64, b: f64, h: f64) -> Stock {
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sa = SketchId(Uuid::from_u128(0xA));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, a, b)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_A, sa, h));
    let report = regen_all(&mut rt).await;
    let snapshot = SnapshotId(report.snapshot_id);
    assert_eq!(
        published(&report, "stock box")
            .repair_summary
            .needs_repair_count,
        0
    );
    let body = body_of(EXTRUDE_A);
    let (volume, faces) = measure(&mut rt, &wm, body).await;
    assert_eq!(faces, 6, "the stock is a sharp six-face box");
    assert_volume(volume, a * b * h, "stock box");

    let blob = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&blob).expect("stock MESH1 validates");
    let (topo_key, anchor) = top_edge_pick(&view, &blob);
    let promoted = rt
        .promote_selection(
            snapshot,
            body,
            vec![(TopoKey::new(&topo_key), anchor_intent(anchor))],
        )
        .await
        .unwrap_or_else(|e| panic!("promote the top edge: {e}"));
    let edge = ElementId::new(&promoted[0].element_id);

    // The edge's two adjacent faces; the one whose centre sits at max z is the top.
    let prepared = wm
        .prepare_edge_op(
            snapshot,
            onecad_app::worker::wire::EdgeOpMode::Fillet,
            &[onecad_app::worker::wire::EdgeOpPick {
                body: Some(body),
                address: onecad_app::worker::wire::FaceAddress::ElementId(edge.as_str()),
            }],
            false,
        )
        .await
        .expect("PrepareEdgeOp on the promoted edge");
    let adjacent = prepared.edges[0]
        .adjacent_faces
        .clone()
        .expect("`adjacentFaces` on every accepted edge");
    let mut top_face = None;
    let mut top_anchor = anchor;
    for key in &adjacent {
        let info = wm
            .query_element_by_topo_key(snapshot, body, key)
            .await
            .expect("QueryElement by topoKey")
            .expect("adjacent face resolves on its own snapshot");
        if (info.center[2] - f64::from(view.bbox_max[2])).abs() < 1e-3 {
            let at = Vec3::new_unchecked(info.center[0], info.center[1], info.center[2]);
            let p = rt
                .promote_selection(snapshot, body, vec![(TopoKey::new(key), anchor_intent(at))])
                .await
                .unwrap_or_else(|e| panic!("promote the top face: {e}"));
            top_face = Some(ElementId::new(&p[0].element_id));
            top_anchor = at;
        }
    }
    Stock {
        wm,
        rt,
        body,
        edge,
        anchor,
        top_face: top_face.expect("one adjacent face is the top face"),
        top_anchor,
    }
}

impl Stock {
    /// The 20 × 30 × 4 box of the matrix.
    async fn slab(bin: PathBuf) -> Self {
        stock_box(bin, 30.0, 20.0, 4.0).await
    }

    fn add_fillet(&mut self, radius: f64, auto: bool) {
        let op = self.fillet(radius, auto);
        add_op(&mut self.rt, edge_record("Fillet", op));
    }

    /// The record's CURRENT op (descriptor/anchor evidence the runtime stamped on
    /// it) with only the radius changed — what the UI's radius re-edit sends.
    fn live_fillet_with_radius(&self, radius: f64) -> Operation {
        let mut op = self
            .rt
            .operation_record(RecordId(Uuid::from_u128(OP_EDGE)))
            .expect("fillet record")
            .op;
        match &mut op {
            Operation::Known(KnownOperation::Fillet(p)) => p.radius = Scalar::new(radius),
            other => panic!("expected Fillet, got {other:?}"),
        }
        op
    }

    fn fillet(&self, radius: f64, auto: bool) -> Operation {
        fillet_op(self.body, &self.edge, self.anchor, radius, auto)
    }

    async fn measure(&mut self) -> (f64, u32) {
        measure(&mut self.rt, &self.wm, self.body).await
    }

    /// Every diagnostic on the document's features plus any failed step.
    fn diagnostics(&self, report: &RegenReport) -> Vec<Diagnostic> {
        let mut all: Vec<Diagnostic> = self
            .rt
            .projection()
            .features
            .iter()
            .flat_map(|f| f.diagnostics.iter().cloned())
            .collect();
        all.extend(
            report
                .failed_steps
                .iter()
                .flat_map(|s| s.diagnostics.clone()),
        );
        all
    }
}

fn with_code<'a>(diags: &'a [Diagnostic], code: &str) -> Vec<&'a Diagnostic> {
    diags.iter().filter(|d| d.code == code).collect()
}

/// Asserts exactly one `code` warning whose `evidence.overflowKind` is `kind`.
fn assert_trim_warning(diags: &[Diagnostic], code: &str, kind: &str) {
    let hits = with_code(diags, code);
    assert_eq!(hits.len(), 1, "exactly one {code}, got {diags:#?}");
    assert_eq!(hits[0].severity, Severity::Warning);
    let evidence = hits[0].evidence.as_ref().expect("warning carries evidence");
    assert_eq!(
        evidence["overflowKind"],
        serde_json::json!(kind),
        "evidence: {evidence:#}"
    );
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: None,
        created: "2026-11-01T00:00:00Z".into(),
        modified: "2026-11-01T00:00:00Z".into(),
    }
}

/// Concatenated text of every container entry, whitespace-stripped, so a key/value
/// pair can be searched for regardless of pretty-printing.
fn container_text(path: &std::path::Path) -> String {
    let bytes = std::fs::read(path).expect("read container");
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    let mut text = String::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).expect("entry");
        let mut raw = Vec::new();
        entry.read_to_end(&mut raw).expect("read entry");
        if let Ok(s) = String::from_utf8(raw) {
            text.extend(s.chars().filter(|c| !c.is_whitespace()));
        }
    }
    text
}

/// Fresh runtime over the saved container (new worker process = no live state).
fn reopen(wm: &WorkerManager, path: &std::path::Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen")
}

macro_rules! worker_or_skip {
    () => {
        match real_worker() {
            Some(bin) => bin,
            None => {
                eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
                return;
            }
        }
    };
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Fillet r=11.2 Auto commits as a cliff
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fillet_overflow_auto_commits_as_cliff() {
    let mut s = Stock::slab(worker_or_skip!()).await;
    s.add_fillet(11.2, true);
    let report = regen_all(&mut s.rt).await;
    assert_eq!(
        published(&report, "overflow fillet")
            .repair_summary
            .needs_repair_count,
        0
    );
    assert!(report.failed_steps.is_empty(), "{:?}", report.failed_steps);
    let (volume, faces) = s.measure().await;
    assert_volume(volume, FILLET_11_2_VOLUME, "cliff fillet");
    assert_eq!(faces, 6, "consumed side face is replaced by the cliff");
    let diags = s.diagnostics(&report);
    assert_trim_warning(&diags, "FILLET_OVERFLOW_TRIMMED", "cliff");
    s.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Same fillet WITHOUT the opt-in keeps failing, body intact
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fillet_without_overflow_still_fails() {
    let mut s = Stock::slab(worker_or_skip!()).await;
    s.add_fillet(11.2, false);
    let revision_before = s.rt.projection().revision;
    let report = regen_all(&mut s.rt).await;
    assert_eq!(report.failed_steps.len(), 1, "native refusal: {report:?}");
    let diags = s.diagnostics(&report);
    assert!(
        with_code(&diags, "FILLET_OVERFLOW_TRIMMED").is_empty()
            && with_code(&diags, "EDGE_OP_OVERFLOW_UNSUPPORTED").is_empty(),
        "no opt-in ⇒ no overflow diagnostics: {diags:#?}"
    );
    assert_eq!(
        s.rt.projection().revision,
        revision_before,
        "a failed regen must not bump the document revision"
    );
    let (volume, faces) = s.measure().await;
    assert_volume(volume, BOX_VOLUME, "refused fillet");
    assert_eq!(faces, 6);
    s.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Chamfer d=6.2 Auto commits as extendedPlane
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chamfer_overflow_auto_commits_as_extended_plane() {
    let mut s = Stock::slab(worker_or_skip!()).await;
    let op = chamfer_op(s.body, &s.edge, s.anchor, 6.2, true);
    add_op(&mut s.rt, edge_record("Chamfer", op));
    let report = regen_all(&mut s.rt).await;
    assert!(report.failed_steps.is_empty(), "{:?}", report.failed_steps);
    published(&report, "overflow chamfer");
    let (volume, _faces) = s.measure().await;
    assert_volume(volume, CHAMFER_6_2_VOLUME, "extendedPlane chamfer");
    let diags = s.diagnostics(&report);
    assert_trim_warning(&diags, "CHAMFER_OVERFLOW_TRIMMED", "extendedPlane");
    s.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. The previewed candidate IS the committed solid
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_equals_commit_for_overflow_fillet() {
    let mut s = Stock::slab(worker_or_skip!()).await;
    let revision_before = s.rt.projection().revision;
    let candidate = edge_record("Fillet", s.fillet(11.2, true));
    let preview =
        s.wm.preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            None,
            None,
            Lod::Fine,
        )
        .await
        .expect("overflow Fillet PreviewOp");
    assert!(
        preview.needs_repair.is_empty(),
        "{:?}",
        preview.needs_repair
    );
    assert_eq!(preview.bodies.len(), 1, "one candidate body");
    let blob = &preview.bodies[0].mesh;
    let view = validate_mesh_blob(blob).expect("preview MESH1 validates");
    let preview_volume = mesh_volume(&view, blob);
    // Fine-LOD tessellation of the curved cliff: near the exact cliff volume.
    assert!(
        (preview_volume - FILLET_11_2_VOLUME).abs() < 1.0,
        "preview is the cliff solid, got {preview_volume}"
    );
    assert_eq!(s.rt.projection().revision, revision_before, "no trace");
    let (untouched, untouched_faces) = s.measure().await;
    assert_volume(untouched, BOX_VOLUME, "real body during preview");
    assert_eq!(untouched_faces, 6);

    add_op(&mut s.rt, candidate);
    let report = regen_all(&mut s.rt).await;
    published(&report, "commit");
    let blob =
        s.rt.get_mesh(s.body, Lod::Fine, None)
            .await
            .expect("fine body mesh");
    let committed_view = validate_mesh_blob(&blob).expect("commit MESH1");
    assert_eq!(committed_view.face_count, view.face_count);
    let committed = mesh_volume(&committed_view, &blob);
    assert!(
        (committed - preview_volume).abs() < 1e-9,
        "preview {preview_volume} and commit {committed} must agree"
    );
    let (exact, faces) = s.measure().await;
    assert_volume(exact, FILLET_11_2_VOLUME, "committed");
    assert_eq!(faces, view.face_count);
    s.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Undo → box; redo → cliff
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undo_redo_of_overflow_fillet() {
    let mut s = Stock::slab(worker_or_skip!()).await;
    s.add_fillet(11.2, true);
    regen_all(&mut s.rt).await;
    assert_volume(s.measure().await.0, FILLET_11_2_VOLUME, "committed");

    s.rt.undo().expect("undo the fillet");
    regen_all(&mut s.rt).await;
    let (volume, faces) = s.measure().await;
    assert_volume(volume, BOX_VOLUME, "after undo");
    assert_eq!(faces, 6);

    s.rt.redo().expect("redo").expect("redo entry");
    let report = regen_all(&mut s.rt).await;
    assert!(report.failed_steps.is_empty(), "{:?}", report.failed_steps);
    assert_volume(s.measure().await.0, FILLET_11_2_VOLUME, "after redo");
    assert_trim_warning(&s.diagnostics(&report), "FILLET_OVERFLOW_TRIMMED", "cliff");
    s.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. Save + fresh reopen replays the overflow
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_reopen_replays_overflow_fillet() {
    let bin = worker_or_skip!();
    let tmp = TempDir::new().expect("temp dir");
    let path = tmp.path().join("overflow.onecad");
    {
        let mut s = Stock::slab(bin.clone()).await;
        s.add_fillet(11.2, true);
        regen_all(&mut s.rt).await;
        s.rt.save(&path, save_meta()).expect("save");
        s.wm.shutdown().await;
    }
    let text = container_text(&path);
    assert!(
        text.contains("\"overflow\":\"auto\"") && text.contains("\"overflowVersion\":1"),
        "persisted feature must carry overflow auto + version 1"
    );

    let wm = spawn_worker(bin).await;
    let mut rt = reopen(&wm, &path);
    let report = regen_all(&mut rt).await;
    assert!(report.failed_steps.is_empty(), "{:?}", report.failed_steps);
    published(&report, "reopened replay");
    let (volume, faces) = measure(&mut rt, &wm, body_of(EXTRUDE_A)).await;
    assert_volume(volume, FILLET_11_2_VOLUME, "reopened");
    assert_eq!(faces, 6);
    let mut diags: Vec<Diagnostic> = rt
        .projection()
        .features
        .iter()
        .flat_map(|f| f.diagnostics.iter().cloned())
        .collect();
    diags.extend(
        report
            .failed_steps
            .iter()
            .flat_map(|s| s.diagnostics.clone()),
    );
    assert_trim_warning(&diags, "FILLET_OVERFLOW_TRIMMED", "cliff");
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. Re-edit radius 2 → 11.2 → 2; the top face keeps resolving
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn re_edit_radius_through_overflow_and_back() {
    let mut s = Stock::slab(worker_or_skip!()).await;
    s.add_fillet(2.0, true);
    let steps = [
        (2.0, FILLET_2_VOLUME, false),
        (11.2, FILLET_11_2_VOLUME, true),
        (2.0, FILLET_2_VOLUME, false),
    ];
    for (i, (radius, want, trimmed)) in steps.into_iter().enumerate() {
        if i > 0 {
            s.rt.apply(EditCommand::UpdateOperationParams {
                record: RecordId(Uuid::from_u128(OP_EDGE)),
                op: s.live_fillet_with_radius(radius),
            })
            .expect("re-edit radius");
        }
        let report = regen_all(&mut s.rt).await;
        assert!(
            report.failed_steps.is_empty(),
            "step {i}: {:?}",
            report.failed_steps
        );
        let snapshot = SnapshotId(report.snapshot_id);
        assert_volume(s.measure().await.0, want, &format!("r={radius} step {i}"));
        let diags = s.diagnostics(&report);
        if trimmed {
            assert_trim_warning(&diags, "FILLET_OVERFLOW_TRIMMED", "cliff");
        } else {
            assert!(
                with_code(&diags, "FILLET_OVERFLOW_TRIMMED").is_empty(),
                "native r={radius} must not warn: {diags:#?}"
            );
        }
        // Downstream-reference proxy: a ref to the top face (what a later feature
        // would hold) still resolves through the ladder, to the SAME element.
        let probe = ResolveRef {
            ref_id: "top".into(),
            element: anchored_ref(s.body, &s.top_face, ElementKind::Face, s.top_anchor),
        };
        let resolved =
            s.rt.resolve_refs(ResolveRequest {
                snapshot_id: snapshot,
                refs: vec![probe],
            })
            .await
            .expect("resolve_refs on the top face");
        match &resolved[0].outcome {
            ResolveOutcome::Unchanged { .. } => {}
            ResolveOutcome::AutoBind { element_id, .. } => assert!(
                element_id.as_str().is_empty() || element_id == &s.top_face,
                "top face rebound to a different element at r={radius}: {element_id:?}"
            ),
            other => panic!("top face unresolved at r={radius} step {i}: {other:?}"),
        }
    }
    s.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. Both supports consumed: refused with a named reason
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_supports_consumed_is_refused_with_reason() {
    let mut s = stock_box(worker_or_skip!(), 10.0, 10.0, 10.0).await;
    s.add_fillet(11.0, true);
    let report = regen_all(&mut s.rt).await;
    assert_eq!(report.failed_steps.len(), 1, "refused: {report:?}");
    let diags = s.diagnostics(&report);
    let hits = with_code(&diags, "EDGE_OP_OVERFLOW_UNSUPPORTED");
    assert!(!hits.is_empty(), "expected refusal diagnostic: {diags:#?}");
    assert_eq!(hits[0].severity, Severity::Error);
    assert_eq!(
        hits[0].reason_code.as_deref(),
        Some("OVERFLOW_BOTH_SUPPORTS_CONSUMED"),
        "diagnostic: {:#?}",
        hits[0]
    );
    assert!(with_code(&diags, "FILLET_OVERFLOW_TRIMMED").is_empty());
    let (volume, faces) = s.measure().await;
    assert_volume(volume, 1000.0, "refused cube fillet");
    assert_eq!(faces, 6);
    s.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 9. A document saved without the fields keeps the old replay semantics
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_document_without_overflow_still_fails_on_reopen() {
    let bin = worker_or_skip!();
    let tmp = TempDir::new().expect("temp dir");
    let path = tmp.path().join("legacy.onecad");
    {
        let mut s = Stock::slab(bin.clone()).await;
        s.add_fillet(11.2, false);
        regen_all(&mut s.rt).await;
        s.rt.save(&path, save_meta()).expect("save");
        s.wm.shutdown().await;
    }
    let text = container_text(&path);
    assert!(
        !text.contains("\"overflow\""),
        "pre-feature params must not grow an overflow key"
    );

    let wm = spawn_worker(bin).await;
    let mut rt = reopen(&wm, &path);
    let report = regen_all(&mut rt).await;
    assert_eq!(
        report.failed_steps.len(),
        1,
        "still native-only: {report:?}"
    );
    let diags: Vec<Diagnostic> = report
        .failed_steps
        .iter()
        .flat_map(|s| s.diagnostics.clone())
        .collect();
    assert!(
        with_code(&diags, "FILLET_OVERFLOW_TRIMMED").is_empty()
            && with_code(&diags, "EDGE_OP_OVERFLOW_UNSUPPORTED").is_empty(),
        "{diags:#?}"
    );
    let (volume, faces) = measure(&mut rt, &wm, body_of(EXTRUDE_A)).await;
    assert_volume(volume, BOX_VOLUME, "legacy reopen");
    assert_eq!(faces, 6);
    wm.shutdown().await;
}
