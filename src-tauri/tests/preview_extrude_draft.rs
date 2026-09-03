//! Draft preview/commit equality against the REAL C++ OCCT worker — roadmap WP0.6.
//!
//! The work package requires that a requested draft be applied or refused, and
//! that preview equal commit. The ctest `extrude_draft` proves applied-or-refused
//! at the BRep level, including the mandated circular-profile red probe. This file
//! proves the other half, which no unit test can: that the DRAG shows the taper the
//! commit will land on, and that previewing it leaves the head untouched.
//!
//! It matters even though both lanes route through `execute_candidate_op` →
//! `ops::execute_extrude` (`PlanExecutor.cpp:277`), because a refusal path is where
//! the two lanes could diverge: a preview that swallowed the refusal and showed an
//! undrafted prism would promise geometry the commit then refuses to produce. Both
//! tests here are taken at `Lod::Fine` on both sides, so the comparison is a real
//! equality assertion and not a faceting tolerance.
//!
//! * `draft_preview_matches_the_commit` — a 10×10 profile extruded 10 mm at 10°
//!   previews the closed-form frustum, leaves the head byte-identical, and commits
//!   to the volume the preview promised.
//! * `a_refused_draft_previews_nothing` — a circular profile, whose extrusion has
//!   no planar side face, must refuse in the PREVIEW lane too rather than showing a
//!   straight cylinder the commit would never publish.
//!
//! Provenance: the frustum expectation is the closed form asserted in
//! `worker/tests/test_extrude_draft.cpp`; the harness mirrors `preview_revolve.rs`.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly unless `ONECAD_REQUIRE_WORKER=1` (CI hard-fails).

use std::f64::consts::PI;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{ConstraintId, EntityId, RecordId, RegionId, SketchId};
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

// ── Harness (mirrors preview_revolve.rs) ────────────────────────────────────

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

/// The whole observable head state a preview MUST NOT touch.
fn head_fingerprint(rt: &DocumentRuntime) -> (u64, u64, u64, Vec<(String, String)>) {
    let snapshot = rt
        .subscribe_snapshots()
        .borrow()
        .clone()
        .expect("a published head snapshot");
    let mut bodies: Vec<(String, String)> = snapshot
        .bodies
        .iter()
        .map(|b| (b.body.to_string(), b.signature.0.clone()))
        .collect();
    bodies.sort();
    (
        snapshot.id.0,
        snapshot.generation,
        rt.projection().revision,
        bodies,
    )
}

const SKETCH_A: u128 = 0xD00;
const EXTRUDE_A: u128 = 0xD01;

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

// ── Sketch + record builders (verbatim shape from preview_revolve.rs) ───────

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

/// A single closed circle — the profile whose extrusion has no planar side face,
/// so `apply_draft` must refuse in BOTH lanes.
fn circle_sketch(sid: SketchId, base: u128, radius: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (center, circle) = (e(0), e(0x10));
    let mut sk = Sketch::on_world_plane(sid, "Circle", WorldPlane::XY);
    sk.add_entity(SketchEntity::point(
        center,
        Vec2::new_unchecked(0.0, 0.0),
        false,
        false,
    ))
    .unwrap();
    sk.add_entity(SketchEntity::circle(circle, center, radius, false).unwrap())
        .unwrap();
    sk.add_constraint(Constraint::Fixed {
        id: c(0),
        point: center,
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(0.0, 0.0),
    })
    .unwrap();
    sk.add_constraint(Constraint::Radius {
        id: c(1),
        entity: circle,
        value: Scalar::new(radius),
    })
    .unwrap();
    sk
}

fn sketch_record(rec: u128, sk: &Sketch, plane: SketchPlaneRef) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    let params = SketchOpParams {
        sketch: sk.id,
        plane,
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

/// The extrude the drag previews and the commit materializes — ONE record.
fn draft_extrude_record(rec: u128, sketch: SketchId, dist: f64, draft_deg: f64) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // first-region fallback (V1)
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(dist),
        draft_angle_deg: Scalar::new(draft_deg),
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

// ── MESH1 geometry helpers (from preview_revolve.rs) ────────────────────────

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

/// Signed volume via the divergence theorem — EXACT for a closed, planar-faced
/// polyhedron, which a drafted prism is.
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

fn blob_volume(blob: &[u8], what: &str) -> f64 {
    let view = validate_mesh_blob(blob).unwrap_or_else(|e| panic!("{what}: MESH1 invalid: {e:?}"));
    mesh_volume(&view, blob)
}

/// Frustum of a square pyramid: V = h/3 · (A₁ + A₂ + √(A₁A₂)).
fn frustum_volume(base_side: f64, top_side: f64, height: f64) -> f64 {
    let (a1, a2) = (base_side * base_side, top_side * top_side);
    height / 3.0 * (a1 + a2 + (a1 * a2).sqrt())
}

// ── 1. The drag shows the taper the commit lands on ─────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_preview_matches_the_commit() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0xD));
    let sk = rect_sketch(sid, 0x2000, 0.0, 0.0, 10.0, 10.0);
    add_op(&mut rt, sketch_record(SKETCH_A, &sk, xy_plane_ref()));
    rt.apply(EditCommand::AddSketch { sketch: sk.clone() })
        .expect("AddSketch");
    let base = regen_all(&mut rt).await;
    assert!(
        published(&base, "profile sketch").bodies.is_empty(),
        "the head is a bare sketch — no body yet"
    );
    rt.enter_sketch(sid).await.expect("enter_sketch");

    let before = head_fingerprint(&rt);

    const HEIGHT: f64 = 10.0;
    const ANGLE: f64 = 10.0;
    let candidate = draft_extrude_record(EXTRUDE_A, sid, HEIGHT, ANGLE);
    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            Some(sid.0.to_string()),
            None,
            Lod::Fine,
        )
        .await
        .expect("PreviewOp reaches the worker");

    assert_eq!(
        preview.needs_repair.len(),
        0,
        "a well-formed drafted extrude resolves cleanly: {:?}",
        preview.needs_repair
    );
    assert_eq!(
        preview.bodies.len(),
        1,
        "a NewBody extrude previews exactly one body"
    );

    // The preview is the FRUSTUM, not the straight prism. A preview that silently
    // dropped the draft would land on 1000 here and still "match" a commit that
    // dropped it too — so this is asserted against the closed form, not the commit.
    let inset = HEIGHT * (ANGLE * PI / 180.0).tan();
    let expected = frustum_volume(10.0, 10.0 - 2.0 * inset, HEIGHT);
    let pv = blob_volume(&preview.bodies[0].mesh, "preview");
    eprintln!("draft {ANGLE}° preview volume = {pv:.3} vs closed form {expected:.3}");
    assert!(
        (pv - expected).abs() < 1.0,
        "the drag shows the drafted frustum: preview {pv:.3} vs closed form {expected:.3}"
    );
    assert!(
        (pv - 1000.0).abs() > 1.0,
        "the preview must not be the UNDRAFTED prism (1000): got {pv:.3}"
    );

    assert_eq!(
        head_fingerprint(&rt),
        before,
        "PreviewOp must leave the head byte-identical"
    );

    // Commit the SAME record and land where the preview promised.
    add_op(&mut rt, candidate);
    let committed = regen_all(&mut rt).await;
    let snapshot = published(&committed, "drafted extrude");
    assert_eq!(snapshot.bodies.len(), 1, "one committed body");
    let mesh = wm
        .fetch_mesh(snapshot.bodies[0].body, Lod::Fine, snapshot.id)
        .await
        .expect("committed body mesh");
    let cv = blob_volume(&mesh, "commit");
    assert!(
        (cv - pv).abs() < 1.0,
        "commit lands on the previewed taper: commit {cv:.3} vs preview {pv:.3}"
    );
}

// ── 2. A refused draft must refuse in the preview lane too ──────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_draft_previews_nothing() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // A circular profile: its extrusion's only side face is a cylinder, so
    // `apply_draft` finds no eligible planar face and refuses. The preview must
    // refuse identically — showing a straight cylinder here would promise the user
    // geometry the commit then declines to publish.
    let sid = SketchId(Uuid::from_u128(0xE));
    let sk = circle_sketch(sid, 0x3000, 5.0);
    add_op(&mut rt, sketch_record(SKETCH_A, &sk, xy_plane_ref()));
    rt.apply(EditCommand::AddSketch { sketch: sk.clone() })
        .expect("AddSketch");
    regen_all(&mut rt).await;
    rt.enter_sketch(sid).await.expect("enter_sketch");

    let before = head_fingerprint(&rt);
    let candidate = draft_extrude_record(EXTRUDE_A, sid, 10.0, 10.0);
    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            Some(sid.0.to_string()),
            None,
            Lod::Fine,
        )
        .await;

    // Either the request errors, or it resolves carrying no body. What it must
    // never do is hand back a straight cylinder.
    match preview {
        Err(e) => {
            eprintln!("refused draft preview errored as expected: {e:?}");
            // SCHEMA §7.6: a failed preview carries the same bounded diagnostics as
            // its candidate ExecutePlan step. Pinning the CODE here is what proves
            // the two lanes agree on WHICH defect this is — routable without
            // matching message text on either side.
            let rendered = format!("{e:?}");
            assert!(
                rendered.contains("EXTRUDE_DRAFT_NO_PLANAR_FACE"),
                "the preview refusal must carry the same stable code the commit does, got {rendered}"
            );
        }
        Ok(p) => {
            let volumes: Vec<f64> = p
                .bodies
                .iter()
                .map(|b| blob_volume(&b.mesh, "refused preview"))
                .collect();
            eprintln!(
                "refused draft preview returned {} bodies: {volumes:?}",
                p.bodies.len()
            );
            let straight = PI * 25.0 * 10.0;
            for v in volumes {
                assert!(
                    (v - straight).abs() > 1.0,
                    "the preview handed back the UNDRAFTED cylinder ({v:.3} ≈ {straight:.3}) \
                     for a draft the commit refuses"
                );
            }
        }
    }

    assert_eq!(
        head_fingerprint(&rt),
        before,
        "a refused preview must leave the head byte-identical"
    );

    // …and the commit refuses, so the two lanes agree.
    add_op(&mut rt, candidate);
    let committed = regen_all(&mut rt).await;
    assert!(
        !matches!(&committed.outcome, Outcome::Published(s) if !s.bodies.is_empty()),
        "the commit must refuse a draft with no eligible planar side face, got {:?}",
        committed.outcome
    );
}
