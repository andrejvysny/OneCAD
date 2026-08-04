//! Edge-op (Fillet / Chamfer) + Shell **PreviewOp ≡ commit** gate, against the REAL
//! C++ OCCT worker, driven through the app's [`DocumentRuntime`] exactly like
//! `wire_contract.rs`.
//!
//! Until this wave the three value-drag tools committed BLIND: the user dragged a
//! radius or a thickness, released, and discovered whether OCCT accepted it by
//! reading the history row that appeared. The frontend now previews them through
//! the same `PreviewOp` lane Extrude uses and BLOCKS the ✓ on a failed candidate —
//! which is only worth anything if the candidate is the op that later commits.
//!
//! This file pins that equality from the wire side, modeled on
//! `wire_contract.rs preview_matches_the_commit_and_leaves_no_trace`:
//!
//! * `fillet_preview_matches_the_commit` / `chamfer_preview_matches_the_commit` —
//!   the candidate ROUNDS/BEVELS (more faces, less volume than the plain box), the
//!   head is untouched, and committing the SAME record lands on the previewed
//!   volume.
//! * `shell_preview_matches_the_commit` — the candidate is the hollow cup, equal to
//!   both the analytic volume and the committed one.
//! * `edge_op_preview_without_typed_edges_fails_loudly` — a fillet carrying only
//!   bare `edgeIds` (no typed `edges`) derives NO body and MUST fail by name. This
//!   is the wire-side lock on the frontend's dual-field rule: `previewOps.ts`
//!   refuses a draft whose `inputs` are not in lockstep with `params.edgeIds`
//!   precisely because `filletParams` silently drops the typed `edges` on a
//!   mismatch, and the op then arrives here — a preview that "succeeded" with a
//!   dropped body binding would be a preview of nothing.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly (CI sets `ONECAD_REQUIRE_WORKER=1` to make that a hard failure).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ChamferParams, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation,
    Operation, OperationRecord, PlaneKind, ShellParams, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, InputPath, InputRef};
use onecad_core::ids::{BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, MeshProvider, PreviewEngine, SolverEngine, WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors wire_contract.rs)
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
const SKETCH_COL: u128 = 0xC00;
const EXTRUDE_COL: u128 = 0xC01;
const OP_TAIL: u128 = 0xF00;

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (verbatim shapes from wire_contract.rs)
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

fn extrude_params(sketch: SketchId, dist: f64, mode: ExtrudeMode) -> ExtrudeParams {
    ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // empty ⇒ V1 first-region fallback
            extra: Default::default(),
        }),
        distance: Scalar::new(dist),
        draft_angle_deg: Scalar::new(0.0),
        mode,
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
        Operation::Known(KnownOperation::Extrude(extrude_params(
            sketch,
            dist,
            ExtrudeMode::Blind,
        ))),
    )
}

/// A `ToFace` extrude whose direction-1 target is `face` — used only to MINT the
/// element id into the partition (`resolve_input_refs`) so a later bare-`openFaces`
/// Shell can bind it, which is the production tracking path (`breadth_ops.rs`).
fn extrude_to_face_record(rec: u128, sketch: SketchId, face: ElementRef) -> OperationRecord {
    let mut params = extrude_params(sketch, 1.0, ExtrudeMode::ToFace);
    params.target_face = Some(face);
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

/// A typed element ref carrying the operated body + a world-point anchor — the
/// shape a real UI edge/face pick lowers to.
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

fn fillet_record(rec: u128, body: BodyId, at: Vec3, radius: f64) -> OperationRecord {
    let edge = ElementId::new("el_edge_pick");
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(radius),
            edge_ids: vec![edge.clone()],
            edges: vec![anchored_ref(body, &edge, ElementKind::Edge, at)],
            chain_tangent_edges: false,
            extra: Default::default(),
        })),
    )
}

fn chamfer_record(rec: u128, body: BodyId, at: Vec3, distance: f64) -> OperationRecord {
    let edge = ElementId::new("el_edge_pick");
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Chamfer",
        Operation::Known(KnownOperation::Chamfer(ChamferParams {
            radius: Scalar::new(distance),
            distance2: None,
            edge_ids: vec![edge.clone()],
            edges: vec![anchored_ref(body, &edge, ElementKind::Edge, at)],
            chain_tangent_edges: false,
            extra: Default::default(),
        })),
    )
}

fn shell_record(rec: u128, body: BodyId, faces: Vec<ElementId>, thickness: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Shell",
        Operation::Known(KnownOperation::Shell(ShellParams {
            thickness: Scalar::new(thickness),
            open_faces: faces,
            target_body: Some(body),
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

fn vertex(blob: &[u8], pbase: usize, i: usize) -> [f64; 3] {
    let o = pbase + i * 12;
    [
        f32_le(blob, o) as f64,
        f32_le(blob, o + 4) as f64,
        f32_le(blob, o + 8) as f64,
    ]
}

/// Signed volume via the divergence theorem — EXACT for a closed polyhedron, and
/// accurate to the tessellation for the rounded/hollow bodies here (both sides of
/// every comparison are tessellated the same way, at the same LOD).
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

fn id_table(view: &MeshHeaderView, blob: &[u8], count: usize) -> Vec<String> {
    let offs = view.section(SEC_FACE_ID_OFFS).expect("id-offs");
    let chars = view.section(SEC_FACE_ID_CHARS).expect("id-chars");
    let (obase, cbase) = (offs.offset as usize, chars.offset as usize);
    (0..count)
        .map(|i| {
            let lo = u32_le(blob, obase + i * 4) as usize;
            let hi = u32_le(blob, obase + (i + 1) * 4) as usize;
            String::from_utf8_lossy(&blob[cbase + lo..cbase + hi]).into_owned()
        })
        .collect()
}

/// The face with the greatest average world-Z (the extrude cap): `(TopoKey, centroid)`.
fn top_face_pick(view: &MeshHeaderView, blob: &[u8]) -> (String, Vec3) {
    let fr = view.section(SEC_FACE_RANGES).expect("FACE_RANGES");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let (frbase, ibase, pbase) = (fr.offset as usize, idx.offset as usize, pos.offset as usize);
    let keys = id_table(view, blob, view.face_count as usize);
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

/// Midpoint of the box's top edge running along +X (at `y = ymin`, `z = zmax`).
///
/// An anchor EXACTLY on an edge's midpoint is what makes the ladder bind it
/// outright: the anchor feature scores 1.0 there while every other edge centre on
/// the body sits a large fraction of the body diagonal away, so the auto-bind gate
/// (score ≥0.85 AND margin ≥0.10) is cleared without ambiguity. Anchoring on a
/// FACE centroid instead — equidistant from all four of its edges — is a symmetric
/// tie and correctly resolves to NeedsRepair, which is why `fillet_body_context`
/// in `wire_contract.rs` has to hedge and this file does not.
fn top_x_edge_midpoint(view: &MeshHeaderView) -> Vec3 {
    Vec3::new_unchecked(
        f64::from(view.bbox_min[0] + view.bbox_max[0]) / 2.0,
        f64::from(view.bbox_min[1]),
        f64::from(view.bbox_max[2]),
    )
}

/// The length of that edge (the X span) — the fillet/chamfer sweeps along it.
fn x_span(view: &MeshHeaderView) -> f64 {
    f64::from(view.bbox_max[0] - view.bbox_min[0])
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

/// Builds a `w × h` rect extruded `depth` as body A and returns its id.
async fn build_box_a(rt: &mut DocumentRuntime, w: f64, h: f64, depth: f64) -> BodyId {
    let sa = SketchId(Uuid::from_u128(0xA));
    add_op(
        rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, w, h)),
    );
    add_op(rt, extrude_record(EXTRUDE_A, sa, depth));
    let rep = regen_all(rt).await;
    let _ = published(&rep, "box A");
    body_of(EXTRUDE_A)
}

// ─────────────────────────────────────────────────────────────────────────────
// Fillet / Chamfer: the candidate IS the committed body
// ─────────────────────────────────────────────────────────────────────────────

/// Shared body for the two edge-op tests: preview the record, assert the candidate
/// really cut material off the sharp edge, assert no trace, then commit the SAME
/// record and assert the two volumes agree.
async fn edge_op_preview_equals_commit(
    op_name: &str,
    make: fn(u128, BodyId, Vec3, f64) -> OperationRecord,
) {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let body_a = build_box_a(&mut rt, 40.0, 20.0, 25.0).await;

    let plain = body_mesh(&mut rt, body_a).await;
    let plain_view = validate_mesh_blob(&plain).expect("box A MESH1 validates");
    assert_eq!(plain_view.face_count, 6, "box A starts sharp (6 faces)");
    let plain_volume = mesh_volume(&plain_view, &plain);
    assert!(
        (plain_volume - 20000.0).abs() < 1.0,
        "40×20×25 = 20000, got {plain_volume}"
    );
    let anchor = top_x_edge_midpoint(&plain_view);
    let edge_len = x_span(&plain_view);

    let head_before = rt.projection().revision;
    let candidate = make(OP_TAIL, body_a, anchor, 2.0);

    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            None,
            None,
            Lod::Coarse,
        )
        .await
        .unwrap_or_else(|e| panic!("{op_name} PreviewOp reaches the worker: {e}"));
    assert!(
        preview.needs_repair.is_empty(),
        "{op_name}: the edge-midpoint anchor must auto-bind, got {:?}",
        preview.needs_repair
    );
    assert_eq!(
        preview.changed_bodies,
        vec![body_a.to_string()],
        "{op_name} MODIFIES the target body (id preserved)"
    );
    assert_eq!(preview.bodies.len(), 1, "{op_name}: one candidate body");

    let blob = &preview.bodies[0].mesh;
    let view = validate_mesh_blob(blob).expect("preview MESH1 validates");
    let preview_volume = mesh_volume(&view, blob);
    // A rounded/bevelled edge adds a face and removes the corner wedge — this is
    // what makes the preview WORTH showing (the pre-wave UI showed nothing at all).
    assert!(
        view.face_count > plain_view.face_count,
        "{op_name} candidate adds a face (6 → >6), got {}",
        view.face_count
    );
    assert!(
        preview_volume < plain_volume,
        "{op_name} candidate removes material: {preview_volume} < {plain_volume}"
    );
    // Sanity on the magnitude: a 2mm fillet over the edge removes r²−πr²/4 per unit
    // length; a 2mm equal-leg chamfer removes r²/2. Both are a few tens of mm³, so
    // a candidate that "shrank" by thousands would be some other op entirely.
    let removed = plain_volume - preview_volume;
    let fillet_removed = (4.0 - std::f64::consts::FRAC_PI_4 * 4.0) * edge_len;
    let chamfer_removed = 2.0 * edge_len;
    assert!(
        removed > 0.5 * fillet_removed.min(chamfer_removed)
            && removed < 2.0 * fillet_removed.max(chamfer_removed),
        "{op_name} removed {removed} mm³, expected ≈{fillet_removed} (fillet) / {chamfer_removed} (chamfer)"
    );

    // No trace: same revision, same geometry on the real body.
    assert_eq!(
        rt.projection().revision,
        head_before,
        "{op_name} preview must not bump the document revision"
    );
    let after = body_mesh(&mut rt, body_a).await;
    let after_view = validate_mesh_blob(&after).expect("post-preview MESH1");
    assert_eq!(
        after_view.face_count, 6,
        "{op_name}: the real body is still sharp after the preview"
    );

    // The SAME record now commits — and lands where the preview promised.
    add_op(&mut rt, candidate);
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, op_name);
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "{op_name} commit resolves the same edge the preview did"
    );
    let committed = body_mesh(&mut rt, body_a).await;
    let committed_view = validate_mesh_blob(&committed).expect("committed MESH1");
    let committed_volume = mesh_volume(&committed_view, &committed);
    assert_eq!(
        committed_view.face_count, view.face_count,
        "{op_name}: preview and commit have the same face count"
    );
    assert!(
        (committed_volume - preview_volume).abs() < 1.0,
        "{op_name}: preview {preview_volume} and commit {committed_volume} must agree — that is the entire point"
    );
    eprintln!(
        "{op_name} preview/commit agreement PASS: {preview_volume} == {committed_volume} (plain {plain_volume})"
    );
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fillet_preview_matches_the_commit() {
    edge_op_preview_equals_commit("Fillet", fillet_record).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chamfer_preview_matches_the_commit() {
    edge_op_preview_equals_commit("Chamfer", chamfer_record).await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Shell: the candidate IS the committed cup
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_preview_matches_the_commit() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let body_a = build_box_a(&mut rt, 20.0, 20.0, 25.0).await;

    let plain = body_mesh(&mut rt, body_a).await;
    let plain_view = validate_mesh_blob(&plain).expect("box A MESH1 validates");
    assert_eq!(plain_view.face_count, 6);
    let plain_volume = mesh_volume(&plain_view, &plain);
    assert!(
        (plain_volume - 10000.0).abs() < 1.0,
        "20×20×25 = 10000, got {plain_volume}"
    );
    let (_top_key, centroid) = top_face_pick(&plain_view, &plain);
    assert!(centroid.z > 24.0, "top face at z≈25, got {}", centroid.z);

    // `ShellParams` carries only BARE ElementIds (no anchor rides the wire — see
    // `wire.rs face_input_refs`), so the open face must already be TRACKED in the
    // partition. A ToFace extrude claiming the same elementId mints it during the
    // same regen, which is the production tracking path (`breadth_ops.rs`).
    let open_face = ElementId::new("el_shell_top");
    let face_ref = anchored_ref(body_a, &open_face, ElementKind::Face, centroid);
    let scol = SketchId(Uuid::from_u128(0xC0));
    add_op(
        &mut rt,
        sketch_record(SKETCH_COL, &rect_sketch(scol, 0x3000, 5.0, 5.0, 5.0, 5.0)),
    );
    add_op(&mut rt, extrude_to_face_record(EXTRUDE_COL, scol, face_ref));
    let report = regen_all(&mut rt).await;
    let rep = published(&report, "ToFace tracking column");
    assert_eq!(
        rep.repair_summary.needs_repair_count, 0,
        "the tracking extrude must bind the top face"
    );

    let head_before = rt.projection().revision;
    let candidate = shell_record(OP_TAIL, body_a, vec![open_face], 1.0);

    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            None,
            None,
            Lod::Coarse,
        )
        .await
        .expect("Shell PreviewOp reaches the worker");
    assert!(
        preview.needs_repair.is_empty(),
        "the partition-tracked open face binds in the preview scratch too, got {:?}",
        preview.needs_repair
    );
    assert_eq!(preview.changed_bodies, vec![body_a.to_string()]);
    assert_eq!(preview.bodies.len(), 1);

    let blob = &preview.bodies[0].mesh;
    let view = validate_mesh_blob(blob).expect("preview MESH1 validates");
    let preview_volume = mesh_volume(&view, blob);
    // Outer 10000 − inner cavity 18×18×24 (7776) = 2224 (exact box arithmetic).
    assert!(
        (preview_volume - 2224.0).abs() < 2.0,
        "shell(20×20×25, t=1, top open) = 10000 − 7776 = 2224, got {preview_volume}"
    );

    assert_eq!(
        rt.projection().revision,
        head_before,
        "a Shell preview must not bump the document revision"
    );
    let after = body_mesh(&mut rt, body_a).await;
    let after_view = validate_mesh_blob(&after).expect("post-preview MESH1");
    let after_volume = mesh_volume(&after_view, &after);
    assert!(
        (after_volume - 10000.0).abs() < 1.0,
        "the real body is still solid after the preview (10000), got {after_volume}"
    );

    add_op(&mut rt, candidate);
    let commit_report = regen_all(&mut rt).await;
    let snap = published(&commit_report, "shell commit");
    assert_eq!(snap.repair_summary.needs_repair_count, 0);
    let committed = body_mesh(&mut rt, body_a).await;
    let committed_view = validate_mesh_blob(&committed).expect("committed MESH1");
    let committed_volume = mesh_volume(&committed_view, &committed);
    assert!(
        (committed_volume - preview_volume).abs() < 1.0,
        "Shell preview {preview_volume} and commit {committed_volume} must agree"
    );
    eprintln!("Shell preview/commit agreement PASS: {preview_volume} == {committed_volume}");
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// The dual-field rule, locked from the wire side
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edge_op_preview_without_typed_edges_fails_loudly() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let body_a = build_box_a(&mut rt, 40.0, 20.0, 25.0).await;

    // `edgeIds` only — no typed `edges`, so NO body is derivable from the op. This
    // is precisely the degraded shape `filletParams` produces when the frontend's
    // inputs and edgeIds fall out of lockstep, and it must be LOUD: a preview that
    // quietly succeeded here would be a preview of an op the commit cannot run.
    let bare = Operation::Known(KnownOperation::Fillet(FilletParams {
        radius: Scalar::new(2.0),
        edge_ids: vec![ElementId::new("el_edge_pick")],
        edges: vec![],
        chain_tangent_edges: false,
        extra: Default::default(),
    }));
    let err = wm
        .preview_op(
            bare,
            Uuid::from_u128(OP_TAIL).to_string(),
            None,
            None,
            Lod::Coarse,
        )
        .await
        .expect_err("a body-less fillet must not preview as a success");
    let text = err.to_string();
    assert!(
        text.contains("body input") || text.contains("body"),
        "the failure NAMES the missing body binding: {text}"
    );
    eprintln!("bare-edgeIds fillet preview correctly refused: {text}");

    // …and the head is untouched by the refusal.
    let after = body_mesh(&mut rt, body_a).await;
    let after_view = validate_mesh_blob(&after).expect("post-failure MESH1");
    assert_eq!(after_view.face_count, 6);
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// HISTORY-HARDEN H9 — `InputPath::ShellOpenFaces`, and the weakness it inherits
// ─────────────────────────────────────────────────────────────────────────────

/// A shell's open face can be RE-PICKED through
/// `EditOperationInput{InputPath::ShellOpenFaces{k}}`, and the re-pick lands
/// because it writes the ElementId DIRECTLY — the resolution ladder is not
/// involved in an explicit user rebind.
///
/// **The recorded weakness (expected-weak, pinned deliberately).**
/// `ShellParams::open_faces` is a bare `Vec<ElementId>`, so the descriptor/anchor
/// evidence on the ref the user picked has NOWHERE to be stored and is dropped
/// (`edit/session.rs set_shell_open_face`). A shell open face is therefore
/// anchor-less on the wire (`wire.rs face_input_refs`) and depends entirely on the
/// worker's partition tracking: re-pick an id the partition does not track and the
/// step goes NeedsRepair with no candidate rung to fall back on. That is the
/// honest V1 behaviour and this test states it in both directions:
///  (a) rebinding to an UNTRACKED id → NeedsRepair, geometry unchanged;
///  (b) rebinding back to the tracked id → the cup rebuilds to the same volume.
///
/// Closing this properly needs a typed `ShellParams::faces` slot (a params change
/// + a SCHEMA §7.3 amendment), which is out of H9's scope.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_open_face_rebind_writes_through_but_carries_no_evidence() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let body_a = build_box_a(&mut rt, 20.0, 20.0, 25.0).await;
    let plain = body_mesh(&mut rt, body_a).await;
    let plain_view = validate_mesh_blob(&plain).expect("box A MESH1 validates");
    let (_top_key, centroid) = top_face_pick(&plain_view, &plain);

    // Same partition-tracking setup as `shell_preview_matches_the_commit`: a ToFace
    // extrude MINTS the open face's ElementId during the regen, which is the only
    // way a bare-id shell face can bind at all.
    let open_face = ElementId::new("el_shell_top");
    let face_ref = anchored_ref(body_a, &open_face, ElementKind::Face, centroid);
    let scol = SketchId(Uuid::from_u128(0xC0));
    add_op(
        &mut rt,
        sketch_record(SKETCH_COL, &rect_sketch(scol, 0x3000, 5.0, 5.0, 5.0, 5.0)),
    );
    add_op(&mut rt, extrude_to_face_record(EXTRUDE_COL, scol, face_ref));
    let shell = RecordId(Uuid::from_u128(OP_TAIL));
    add_op(
        &mut rt,
        shell_record(OP_TAIL, body_a, vec![open_face.clone()], 1.0),
    );
    let rep = regen_all(&mut rt).await;
    assert_eq!(
        published(&rep, "shell").repair_summary.needs_repair_count,
        0
    );
    let cup = body_mesh(&mut rt, body_a).await;
    let cup_view = validate_mesh_blob(&cup).expect("cup MESH1");
    let cup_volume = mesh_volume(&cup_view, &cup);
    assert!(
        (cup_volume - 2224.0).abs() < 2.0,
        "shell(20×20×25, t=1, top open) = 2224, got {cup_volume}"
    );

    // (a) Re-pick an id the partition does NOT track. The ref carries a perfectly
    // good anchor, but the bare-id slot cannot store it, so nothing can rescue the
    // binding — NeedsRepair, deterministically, never a guessed face.
    rt.apply(EditCommand::EditOperationInput {
        record: shell,
        path: InputPath::ShellOpenFaces { index: 0 },
        reference: InputRef::Element(anchored_ref(
            body_a,
            &ElementId::new("el_never_minted"),
            ElementKind::Face,
            centroid,
        )),
    })
    .expect("ShellOpenFaces{0} is accepted for a Shell record");
    // The write-through IS observable in the record even though the evidence is not.
    let stored = rt
        .operation_params(shell)
        .expect("shell params")
        .get("openFaces")
        .cloned()
        .expect("openFaces");
    assert_eq!(
        stored,
        serde_json::json!(["el_never_minted"]),
        "the rebind wrote the bare id into slot 0"
    );
    let broken = regen_all(&mut rt).await;
    let snap = published(&broken, "untracked shell face");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 1,
        "an untracked open face has NO evidence rung to fall back on — this is the \
         recorded weakness of a bare-id slot, not a bug in the rebind"
    );
    let after = body_mesh(&mut rt, body_a).await;
    let after_view = validate_mesh_blob(&after).expect("post-break MESH1");
    let after_volume = mesh_volume(&after_view, &after);
    assert!(
        (after_volume - 10000.0).abs() < 1.0,
        "the un-shelled box is back (10000) — the step did not run, and certainly \
         did not open some other face: got {after_volume}"
    );

    // (b) Re-pick the TRACKED id: an explicit rebind writes the id directly, so it
    // binds without the ladder ever being consulted. The cup comes back exactly.
    rt.apply(EditCommand::EditOperationInput {
        record: shell,
        path: InputPath::ShellOpenFaces { index: 0 },
        reference: InputRef::Element(anchored_ref(
            body_a,
            &open_face,
            ElementKind::Face,
            centroid,
        )),
    })
    .expect("re-pick the tracked open face");
    let rep = regen_all(&mut rt).await;
    assert_eq!(
        published(&rep, "repaired shell")
            .repair_summary
            .needs_repair_count,
        0
    );
    let fixed = body_mesh(&mut rt, body_a).await;
    let fixed_view = validate_mesh_blob(&fixed).expect("repaired MESH1");
    let fixed_volume = mesh_volume(&fixed_view, &fixed);
    assert!(
        (fixed_volume - cup_volume).abs() < 1.0,
        "the repaired shell must rebuild the SAME cup: want {cup_volume}, got {fixed_volume}"
    );
    eprintln!(
        "H9 ShellOpenFaces rebind PASS: cup {cup_volume} → broken {after_volume} → {fixed_volume}"
    );
    wm.shutdown().await;
}

/// The core's bounds + identity discipline on the shell arm, at the app layer:
/// a gap past the end and an evidence-only ref are both refused before the worker
/// sees anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_open_face_rebind_refuses_out_of_range_and_identity_less_refs() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let body_a = build_box_a(&mut rt, 20.0, 20.0, 25.0).await;
    let plain = body_mesh(&mut rt, body_a).await;
    let plain_view = validate_mesh_blob(&plain).expect("box MESH1");
    let (_k, centroid) = top_face_pick(&plain_view, &plain);
    let shell = RecordId(Uuid::from_u128(OP_TAIL));
    add_op(
        &mut rt,
        shell_record(OP_TAIL, body_a, vec![ElementId::new("el_a")], 1.0),
    );

    let err = rt
        .apply(EditCommand::EditOperationInput {
            record: shell,
            path: InputPath::ShellOpenFaces { index: 3 },
            reference: InputRef::Element(anchored_ref(
                body_a,
                &ElementId::new("el_b"),
                ElementKind::Face,
                centroid,
            )),
        })
        .expect_err("a gap past the end of openFaces is out of range");
    assert!(
        format!("{err}").contains("out of range"),
        "expected an out-of-range rejection, got: {err}"
    );

    let mut anchor_only = anchored_ref(body_a, &ElementId::new("x"), ElementKind::Face, centroid);
    anchor_only.primary = None;
    let err = rt
        .apply(EditCommand::EditOperationInput {
            record: shell,
            path: InputPath::ShellOpenFaces { index: 0 },
            reference: InputRef::Element(anchor_only),
        })
        .expect_err("a bare-id slot needs a primary element id");
    assert!(
        format!("{err}").contains("primary element id"),
        "expected a primary-id rejection, got: {err}"
    );

    // Neither refusal touched the record.
    assert_eq!(
        rt.operation_params(shell)
            .expect("shell params")
            .get("openFaces")
            .cloned()
            .expect("openFaces"),
        serde_json::json!(["el_a"])
    );
    wm.shutdown().await;
}
