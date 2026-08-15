//! WP-C T3 `Hole` integration gate against the REAL C++ OCCT worker, driven
//! through the app's [`DocumentRuntime`] exactly like `preview_edge_shell.rs`.
//!
//! Proves the op end to end (core params → wire lowering → worker dispatch → OCCT
//! → MESH1) with EXACT kernel numbers, not mesh-chord approximations: every volume
//! here comes from `QueryMassProperties` (SCHEMA §7.5 — `BRepGProp` over the real
//! BRep), so a `π·r²·t` expectation is asserted to kernel precision.
//!
//! * `simple_through_hole_removes_the_exact_cylinder` — a Ø6 through-all hole in a
//!   20×20×25 box removes exactly `π·3²·25`, the host keeps its BodyId (MODIFY
//!   lineage — SCHEMA §7.3 "nothing minted"), and the body count is unchanged.
//! * `through_all_drills_the_full_thickness_of_each_host` — the SAME hole params on
//!   a 25 mm and a 40 mm host each remove that host's OWN full thickness. This is
//!   what makes `depth: null` a real end condition rather than a magic number: the
//!   extent is derived from the host's bbox per regen.
//! * `counterbore_and_countersink_remove_their_analytic_volumes` — the two dressed
//!   profiles, including the DERIVED countersink cone depth
//!   (`(csD−d)/2 / tan(csAngle/2)`), which is the only thing that proves
//!   `csAngleDeg` reached the kernel at all.
//! * `editing_the_diameter_redrills_the_hole` — a param-only re-edit (Ø6 → Ø10)
//!   through the single writer; the volume follows and the record keeps identity.
//! * `hole_preview_matches_the_commit` — the `PreviewOp` candidate IS the body that
//!   later commits (the house gate every previewable op carries).
//! * `a_point_off_the_face_fails_loudly_and_recoverably` — the SCHEMA §7.3 fence:
//!   a frozen point that no longer lies on the resolved face is a NAMED, recoverable
//!   step error, never a hole drilled through empty space.
//! * `saved_hole_reopens_identically` — save → reopen on a FRESH worker replays to
//!   the same volume, and `document.json` is byte-stable across a second save.
//! * `a_cross_body_element_ref_never_drills_the_wrong_body` — VF-M7: an ElementId
//!   minted on body A, reused by a hole on body B, must not seat the drill on B's
//!   Nth face (`worker/tests/test_cross_body_element_ref.cpp` is the unit-level
//!   twin). Asserted by exact volume on BOTH bodies.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly (CI sets `ONECAD_REQUIRE_WORKER=1` to make that a hard failure).

use std::f64::consts::PI;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, HoleParams, HoleType, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, InputPath, InputRef};
use onecad_core::ids::{BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest, StoppedReason,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, PreviewEngine, SolverEngine, WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors preview_edge_shell.rs)
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

fn open_over(wm: &WorkerManager, path: &std::path::Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen the saved container")
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

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-7.9.3".into()),
        created: "2026-08-03T00:00:00Z".into(),
        modified: "2026-08-03T00:00:00Z".into(),
    }
}

/// Exact kernel volume of a body (`QueryMassProperties`, SCHEMA §7.5) — read from
/// the BRep, never re-derived from the tessellation, so `π·r²·t` can be asserted to
/// kernel precision instead of mesh-chord precision.
async fn exact_volume(wm: &WorkerManager, body: BodyId) -> f64 {
    ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties")
        .volume
}

/// The analytic volume a hole removes from a host, given the full profile.
/// Mirrors `worker/tests/test_hole_op.cpp` so a divergence shows up on both sides.
fn removed_volume(
    diameter: f64,
    depth: f64,
    cb: Option<(f64, f64)>,
    cs: Option<(f64, f64)>,
) -> f64 {
    let r = diameter / 2.0;
    let mut v = PI * r * r * depth;
    if let Some((cb_d, cb_t)) = cb {
        let cb_r = cb_d / 2.0;
        // Union with the drill: the counterbore's first `cb_t` overlaps it.
        v += PI * cb_r * cb_r * cb_t - PI * r * r * cb_t;
    }
    if let Some((cs_d, cs_angle)) = cs {
        let cs_r = cs_d / 2.0;
        let cs_t = (cs_r - r) / (cs_angle.to_radians() / 2.0).tan();
        let frustum = (PI * cs_t / 3.0) * (cs_r * cs_r + cs_r * r + r * r);
        v += frustum - PI * r * r * cs_t;
    }
    v
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (verbatim shapes from preview_edge_shell.rs)
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

/// The typed host-FACE ref a real face pick lowers to: identity + a world-point
/// anchor (no frozen descriptor — the anchor alone auto-binds a face whose centre
/// is nowhere near any other face's, which is every face of a box).
fn face_ref(body: BodyId, element: &ElementId, at: Vec3) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: element.clone(),
            kind: ElementKind::Face,
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

/// A hole authored on `body`'s face at `at`, drilled from that same point.
fn hole_params(body: BodyId, element: &str, at: Vec3, diameter: f64) -> HoleParams {
    let id = ElementId::new(element);
    HoleParams {
        target_body: body,
        face: face_ref(body, &id, at),
        point: at,
        hole_type: HoleType::Simple,
        diameter: Scalar::new(diameter),
        depth: None, // through-all
        cb_diameter: None,
        cb_depth: None,
        cs_diameter: None,
        cs_angle_deg: None,
        result_policy_version: Some(2),
        extra: Default::default(),
    }
}

fn hole_record(rec: u128, params: HoleParams) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Hole",
        Operation::Known(KnownOperation::Hole(params)),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 helpers (only to LOCATE the top face — every number is kernel-exact)
// ─────────────────────────────────────────────────────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;

fn vertex(blob: &[u8], pbase: usize, i: usize) -> [f64; 3] {
    let o = pbase + i * 12;
    [
        f32_le(blob, o) as f64,
        f32_le(blob, o + 4) as f64,
        f32_le(blob, o + 8) as f64,
    ]
}

/// The centroid of the face with the greatest average world-Z (the extrude cap).
/// Used as BOTH the ref anchor and the frozen hole `point` — a face centroid is
/// unambiguous evidence for a FACE ref (unlike for an edge ref, where it is a
/// symmetric tie), and it is by construction inside the face's boundary.
fn top_face_centroid(view: &MeshHeaderView, blob: &[u8]) -> Vec3 {
    let fr = view.section(SEC_FACE_RANGES).expect("FACE_RANGES");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let (frbase, ibase, pbase) = (fr.offset as usize, idx.offset as usize, pos.offset as usize);
    let mut best: Option<Vec3> = None;
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
        let c = Vec3::new_unchecked(sx / n, sy / n, sz / n);
        if best.is_none_or(|b| c.z > b.z) {
            best = Some(c);
        }
    }
    best.expect("at least one face")
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

/// Builds a `20×20×depth` box and returns `(bodyId, top-face centroid)`.
async fn build_box(
    rt: &mut DocumentRuntime,
    sketch_rec: u128,
    extrude_rec: u128,
    sketch_base: u128,
    x0: f64,
    depth: f64,
) -> (BodyId, Vec3) {
    let sid = SketchId(Uuid::from_u128(sketch_base));
    add_op(
        rt,
        sketch_record(
            sketch_rec,
            &rect_sketch(sid, sketch_base + 0x1000, x0, 0.0, 20.0, 20.0),
        ),
    );
    add_op(rt, extrude_record(extrude_rec, sid, depth));
    let rep = regen_all(rt).await;
    let _ = published(&rep, "box");
    let body = BodyId(Uuid::from_u128(extrude_rec));
    let mesh = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    assert_eq!(view.face_count, 6, "a fresh box has 6 faces");
    (body, top_face_centroid(&view, &mesh))
}

const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;
const SKETCH_B: u128 = 0xB00;
const EXTRUDE_B: u128 = 0xB01;
const HOLE_A: u128 = 0xF01;
const HOLE_B: u128 = 0xF02;

// ─────────────────────────────────────────────────────────────────────────────
// 1. Simple through-all
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simple_through_hole_removes_the_exact_cylinder() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (body, top) = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 25.0).await;

    let solid = exact_volume(&wm, body).await;
    assert!(
        (solid - 10000.0).abs() < 1e-6,
        "20×20×25 = 10000, got {solid}"
    );

    add_op(
        &mut rt,
        hole_record(HOLE_A, hole_params(body, "el_hole_a", top, 6.0)),
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "hole");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "the top-face anchor must auto-bind"
    );
    // SCHEMA §7.3: modified-host lineage — nothing minted, id preserved.
    assert_eq!(
        rt.head_body_ids(),
        vec![body],
        "a Hole mints nothing and deletes nothing"
    );

    let want = 10000.0 - removed_volume(6.0, 25.0, None, None);
    let got = exact_volume(&wm, body).await;
    assert!(
        (got - want).abs() < want * 1e-9,
        "through-all Ø6 in a 25 mm host removes π·3²·25: want {want}, got {got}"
    );

    // The drilled body gains the cylindrical wall (6 → 7 faces).
    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("drilled MESH1 validates");
    assert_eq!(
        view.face_count, 7,
        "a through hole adds exactly one cylindrical wall face"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Through-all is bounded by EACH host's own extent
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn through_all_drills_the_full_thickness_of_each_host() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    // Two hosts of DIFFERENT thickness, side by side, one identical hole each.
    let (body_a, top_a) = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 25.0).await;
    let (body_b, top_b) = build_box(&mut rt, SKETCH_B, EXTRUDE_B, 0xB, 40.0, 40.0).await;

    add_op(
        &mut rt,
        hole_record(HOLE_A, hole_params(body_a, "el_hole_a", top_a, 6.0)),
    );
    add_op(
        &mut rt,
        hole_record(HOLE_B, hole_params(body_b, "el_hole_b", top_b, 6.0)),
    );
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "two holes");
    let repair_items = rt.repair_items();
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "two-host hole repair evidence: {:?}",
        repair_items
    );

    for (body, thickness, base) in [(body_a, 25.0, 10000.0), (body_b, 40.0, 16000.0)] {
        let want = base - removed_volume(6.0, thickness, None, None);
        let got = exact_volume(&wm, body).await;
        assert!(
            (got - want).abs() < want * 1e-9,
            "through-all must consume the host's OWN {thickness} mm: want {want}, got {got}"
        );
    }
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Counterbore + countersink analytic volumes
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn counterbore_and_countersink_remove_their_analytic_volumes() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (body_a, top_a) = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 25.0).await;
    let (body_b, top_b) = build_box(&mut rt, SKETCH_B, EXTRUDE_B, 0xB, 40.0, 25.0).await;

    // M6 SHCS counterbore: Ø6.6 clearance, Ø11 × 6.6 head recess.
    let mut cb = hole_params(body_a, "el_hole_a", top_a, 6.6);
    cb.hole_type = HoleType::Counterbore;
    cb.depth = Some(Scalar::new(20.0));
    cb.cb_diameter = Some(Scalar::new(11.0));
    cb.cb_depth = Some(Scalar::new(6.6));
    add_op(&mut rt, hole_record(HOLE_A, cb));

    // DIN 74 form A countersink for M6: Ø6.6 clearance, Ø12.4 head, 90°.
    let mut cs = hole_params(body_b, "el_hole_b", top_b, 6.6);
    cs.hole_type = HoleType::Countersink;
    cs.depth = Some(Scalar::new(20.0));
    cs.cs_diameter = Some(Scalar::new(12.4));
    cs.cs_angle_deg = Some(Scalar::new(90.0));
    add_op(&mut rt, hole_record(HOLE_B, cs));

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "dressed holes");
    assert_eq!(snap.repair_summary.needs_repair_count, 0);

    let want_cb = 10000.0 - removed_volume(6.6, 20.0, Some((11.0, 6.6)), None);
    let got_cb = exact_volume(&wm, body_a).await;
    assert!(
        (got_cb - want_cb).abs() < want_cb * 1e-9,
        "counterbore: want {want_cb}, got {got_cb}"
    );

    let want_cs = 10000.0 - removed_volume(6.6, 20.0, None, Some((12.4, 90.0)));
    let got_cs = exact_volume(&wm, body_b).await;
    assert!(
        (got_cs - want_cs).abs() < want_cs * 1e-9,
        "countersink (derived cone depth from csAngleDeg): want {want_cs}, got {got_cs}"
    );
    // The countersink MUST remove more than the bare drill — otherwise `csAngleDeg`
    // never reached the kernel and the assertion above would be measuring nothing.
    assert!(
        got_cs < 10000.0 - removed_volume(6.6, 20.0, None, None),
        "the countersink cone must actually be cut"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Parametric re-edit
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn editing_the_diameter_redrills_the_hole() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (body, top) = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 25.0).await;

    let params = hole_params(body, "el_hole_a", top, 6.0);
    add_op(&mut rt, hole_record(HOLE_A, params.clone()));
    published(&regen_all(&mut rt).await, "hole Ø6");
    let want6 = 10000.0 - removed_volume(6.0, 25.0, None, None);
    let got6 = exact_volume(&wm, body).await;
    assert!((got6 - want6).abs() < want6 * 1e-9, "Ø6: {got6} vs {want6}");

    // Param-only re-edit through the single writer: same record, bigger drill.
    let widened = HoleParams {
        diameter: Scalar::new(10.0),
        ..params
    };
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(HOLE_A)),
        op: Operation::Known(KnownOperation::Hole(widened)),
    })
    .expect("UpdateOperationParams(Hole)");
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "hole Ø10");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "widening the drill must not disturb the face binding"
    );
    let want10 = 10000.0 - removed_volume(10.0, 25.0, None, None);
    let got10 = exact_volume(&wm, body).await;
    assert!(
        (got10 - want10).abs() < want10 * 1e-9,
        "Ø10 re-edit: want {want10}, got {got10}"
    );
    assert_eq!(
        rt.head_body_ids(),
        vec![body],
        "a re-edit still mints nothing"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. PreviewOp ≡ commit
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hole_preview_matches_the_commit() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (body, top) = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 25.0).await;

    let head_before = rt.projection().revision;
    let candidate = hole_record(HOLE_A, hole_params(body, "el_hole_a", top, 6.0));

    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            None,
            None,
            Lod::Coarse,
        )
        .await
        .expect("Hole PreviewOp reaches the worker");
    assert!(
        preview.needs_repair.is_empty(),
        "the top-face anchor binds in the preview scratch too, got {:?}",
        preview.needs_repair
    );
    assert_eq!(
        preview.changed_bodies,
        vec![body.to_string()],
        "a Hole MODIFIES the host (id preserved)"
    );
    assert_eq!(preview.bodies.len(), 1, "one candidate body");
    let view = validate_mesh_blob(&preview.bodies[0].mesh).expect("preview MESH1 validates");
    assert_eq!(view.face_count, 7, "the candidate already shows the bore");

    // No trace on the real document.
    assert_eq!(
        rt.projection().revision,
        head_before,
        "a Hole preview must not bump the document revision"
    );
    let still = exact_volume(&wm, body).await;
    assert!(
        (still - 10000.0).abs() < 1e-6,
        "the real body is still solid after the preview, got {still}"
    );

    // The SAME record commits and lands where the preview promised.
    add_op(&mut rt, candidate);
    published(&regen_all(&mut rt).await, "commit");
    let committed = body_mesh(&mut rt, body).await;
    let committed_view = validate_mesh_blob(&committed).expect("committed MESH1");
    assert_eq!(
        committed_view.face_count, view.face_count,
        "preview and commit have the same face count — that is the entire point"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. The frozen-point fence
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_point_off_the_face_fails_loudly_and_recoverably() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (body, top) = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 25.0).await;

    // The ref still names the top face, but the frozen point sits 10 mm above it.
    let mut params = hole_params(body, "el_hole_a", top, 6.0);
    params.point = Vec3::new_unchecked(top.x, top.y, top.z + 10.0);
    add_op(&mut rt, hole_record(HOLE_A, params));
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "off-face hole");

    // Recoverable: the step errors by name, the prior geometry survives intact.
    let states = format!("{:?}", snap.step_states);
    assert!(
        states.contains("off the resolved face plane"),
        "the fence must NAME the reason, got {states}"
    );
    let solid = exact_volume(&wm, body).await;
    assert!(
        (solid - 10000.0).abs() < 1e-6,
        "a refused hole leaves the host untouched, got {solid}"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. Save → reopen identity
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_hole_reopens_identically() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let doc = tmp.path().join("hole.onecad");

    // ── Session 1: author + save ─────────────────────────────────────────────
    let (body, volume) = {
        let wm = spawn_worker(bin.clone()).await;
        let mut rt = runtime_over(&wm);
        let (body, top) = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 25.0).await;
        let mut params = hole_params(body, "el_hole_a", top, 6.6);
        params.hole_type = HoleType::Counterbore;
        params.depth = Some(Scalar::new(18.0));
        params.cb_diameter = Some(Scalar::new(11.0));
        params.cb_depth = Some(Scalar::new(6.6));
        add_op(&mut rt, hole_record(HOLE_A, params));
        published(&regen_all(&mut rt).await, "session-1 hole");
        let volume = exact_volume(&wm, body).await;
        // Pin the ABSOLUTE outcome, so a broken lane cannot make session-1 ==
        // session-2 vacuously true.
        let want = 10000.0 - removed_volume(6.6, 18.0, Some((11.0, 6.6)), None);
        assert!(
            (volume - want).abs() < want * 1e-9,
            "session 1 must actually drill: want {want}, got {volume}"
        );
        rt.save(&doc, save_meta()).expect("save");
        wm.shutdown().await;
        (body, volume)
    };

    // ── Session 2: FRESH runtime on a FRESH worker ───────────────────────────
    let wm = spawn_worker(bin).await;
    let mut rt = open_over(&wm, &doc);
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "reopen regen");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "the persisted face ref must re-bind on a replay from zero"
    );
    assert_eq!(rt.head_body_ids(), vec![body], "same body, same id");
    let reopened = exact_volume(&wm, body).await;
    assert_eq!(
        reopened, volume,
        "a reopened hole replays to the IDENTICAL volume"
    );

    // `document.json` byte-stability across a second save (no churn from the op).
    let doc2 = tmp.path().join("hole-again.onecad");
    rt.save(&doc2, save_meta()).expect("second save");
    assert_eq!(
        document_json(&doc),
        document_json(&doc2),
        "document.json must be byte-stable across save → reopen → save"
    );
    wm.shutdown().await;
}

/// The raw `document.json` bytes inside a `.onecad` container.
fn document_json(path: &std::path::Path) -> Vec<u8> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open container");
    let mut zip = zip::ZipArchive::new(file).expect("read container zip");
    let mut entry = zip.by_name("document.json").expect("document.json entry");
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).expect("read document.json");
    bytes
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. VF-M7 — a cross-body ElementId must never seat the drill
// ─────────────────────────────────────────────────────────────────────────────

/// An `ElementId` is globally unique and body-INDEPENDENT (Invariant 2), so nothing
/// stops one minted on body A from riding an op on body B — a copied/stale ref, or a
/// hand-authored record. The worker's element-map entry for it carries a `TopoKey`,
/// which is an ORDINAL in *A's* face map; applying it to B does not fail, it silently
/// returns B's Nth face. `PlanExecutor::resolve_input_refs` deliberately does not
/// re-mint an id it already tracks, so the foreign entry reaches the op intact.
///
/// The two ordinals really do diverge here, and for an ordinary reason: drilling A
/// rebuilds A's face map, so `apply_history` relabels A's top face from `f:6` to
/// `f:5`. `f:5` on the still-pristine B is its **bottom** face — a perfectly valid
/// ordinal that means something else entirely. Before the VF-M7 fix the drill seated
/// there and the op died on the point fence ("point is off the resolved face plane")
/// against a face the user never named, leaving B undrilled at 16000 — a failure no
/// amount of re-picking could clear. `worker/tests/test_cross_body_element_ref.cpp`
/// is the unit-level twin, where the same mis-bind drills silently.
///
/// Both bodies are volume-checked against `QueryMassProperties`: B must lose exactly
/// one Ø2 cylinder drilled through its OWN 40 mm from the TOP, and A exactly its own
/// Ø6 — no drill may wander across bodies in either direction.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cross_body_element_ref_never_drills_the_wrong_body() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (body_a, top_a) = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 25.0).await;
    let (body_b, top_b) = build_box(&mut rt, SKETCH_B, EXTRUDE_B, 0xB, 40.0, 40.0).await;

    // The shared ElementId is minted HERE, against body A's top face.
    add_op(
        &mut rt,
        hole_record(HOLE_A, hole_params(body_a, "el_shared", top_a, 6.0)),
    );
    // ...and reused on body B. Only the ElementId is foreign — the anchor names B's
    // own top face, which is what the resolution ladder must bind instead.
    add_op(
        &mut rt,
        hole_record(HOLE_B, hole_params(body_b, "el_shared", top_b, 2.0)),
    );

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "cross-body hole");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "B's own top-face anchor must auto-bind despite the foreign ElementId"
    );
    // Timeline: 0 sketchA, 1 extrudeA, 2 sketchB, 3 extrudeB, 4 holeA, 5 holeB.
    assert_eq!(
        snap.step_index,
        Some(5),
        "both holes must execute — a mis-seated drill stops the plan at m−1"
    );
    assert_eq!(
        rt.head_body_ids(),
        vec![body_a, body_b],
        "both hosts survive with their ids"
    );

    // B: its OWN 40 mm thickness. A bottom/side-face mis-seat leaves this at 16000,
    // and drilling the wrong host would leave it there too — neither can pass.
    let want_b = 16000.0 - removed_volume(2.0, 40.0, None, None);
    let got_b = exact_volume(&wm, body_b).await;
    assert!(
        (got_b - want_b).abs() < want_b * 1e-9,
        "B must lose exactly its own through-hole: want {want_b}, got {got_b}"
    );

    // A: exactly one hole. The shared id must not have dragged B's drill back here.
    let want_a = 10000.0 - removed_volume(6.0, 25.0, None, None);
    let got_a = exact_volume(&wm, body_a).await;
    assert!(
        (got_a - want_a).abs() < want_a * 1e-9,
        "A must be untouched by the op on B: want {want_a}, got {got_a}"
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 9. HISTORY-HARDEN H9 — `InputPath::HoleFace` end to end
// ─────────────────────────────────────────────────────────────────────────────

/// The centroid of the face with the greatest average world-Y — the box's `+Y`
/// side. Twin of [`top_face_centroid`]; a side face is what makes the rebind
/// PROVABLE by volume (a `+Y` through-hole crosses 20 mm of this box, a `+Z` one
/// crosses 25 mm, so the two land at different volumes).
fn plus_y_face_centroid(view: &MeshHeaderView, blob: &[u8]) -> Vec3 {
    let fr = view.section(SEC_FACE_RANGES).expect("FACE_RANGES");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let (frbase, ibase, pbase) = (fr.offset as usize, idx.offset as usize, pos.offset as usize);
    let mut best: Option<Vec3> = None;
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
        let c = Vec3::new_unchecked(sx / n, sy / n, sz / n);
        if best.is_none_or(|b| c.y > b.y) {
            best = Some(c);
        }
    }
    best.expect("at least one face")
}

/// A face ref with an id but NO evidence at all — no descriptor, no anchor. This
/// is what "the reference broke" looks like to the ladder: the exact-id rung
/// misses and there is nothing below it to score.
fn blind_face_ref(body: BodyId, element: &str) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: ElementId::new(element),
            kind: ElementKind::Face,
            extra: Default::default(),
        }),
        intent: None,
        anchor: None,
        extra: Default::default(),
    }
}

/// **The H9 flagship.** A hole's host face is re-picked through
/// `EditOperationInput{InputPath::HoleFace}` and the drill moves to the new face
/// — proven by the EXACT kernel volume, not by a status flag.
///
/// Before H9 there was no `InputPath` that could address a hole's host face at
/// all: the repair panel sent every rebind as `FilletEdges{k}`, which the core
/// rejected for a Hole, so a hole whose face reference broke was UNREPAIRABLE
/// except by rebuilding the feature.
///
/// The sequence is break → repair, so both directions of the write-through are
/// exercised:
///  1. drill through the TOP face (25 mm of stock) — the baseline volume;
///  2. BREAK it with a `UpdateOperationParams` that blinds the face ref and moves
///     the frozen `point` onto the `+Y` side face — the step can no longer resolve;
///  3. REPAIR it with `EditOperationInput{HoleFace}` naming the `+Y` face;
///  4. the drill now crosses 20 mm, not 25 — it landed on the RE-PICKED face.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hole_host_face_rebind_moves_the_drill_to_the_repicked_face() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (body, top) = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 25.0).await;
    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    let side = plus_y_face_centroid(&view, &mesh);
    assert!(
        side.y > 19.0 && top.z > 24.0,
        "expected a +Y side face at y≈20 and a top face at z≈25, got side={side:?} top={top:?}"
    );

    // (1) Baseline: through the TOP — 25 mm of stock.
    let hole = RecordId(Uuid::from_u128(HOLE_A));
    add_op(
        &mut rt,
        hole_record(HOLE_A, hole_params(body, "el_top_face", top, 6.0)),
    );
    let rep = regen_all(&mut rt).await;
    assert_eq!(
        published(&rep, "top hole")
            .repair_summary
            .needs_repair_count,
        0
    );
    let want_top = 10000.0 - removed_volume(6.0, 25.0, None, None);
    let got_top = exact_volume(&wm, body).await;
    assert!(
        (got_top - want_top).abs() < want_top * 1e-9,
        "baseline top hole: want {want_top}, got {got_top}"
    );

    // (2) BREAK: blind the face ref and move the frozen point to the side face.
    let mut broken = hole_params(body, "el_gone", side, 6.0);
    broken.face = blind_face_ref(body, "el_gone");
    rt.apply(EditCommand::UpdateOperationParams {
        record: hole,
        op: Operation::Known(KnownOperation::Hole(broken)),
    })
    .expect("blind the hole's face ref");
    let broken_report = regen_all(&mut rt).await;
    // A ref with NO evidence must produce a deterministic NeedsRepair, never a
    // plausible-looking drill. (This is also the case that exposed the empty-anchor
    // protocol defect — see `wire::parse_needs_repair`. Before that fix this regen
    // came back `EngineFailed(Protocol)` and tore the worker down, so the item
    // could never reach the repair panel at all.)
    let snap = published(&broken_report, "blinded hole");
    assert_eq!(snap.stopped_reason, StoppedReason::NeedsRepair);
    assert_eq!(snap.repair_summary.needs_repair_count, 1);
    assert_eq!(
        snap.repair_summary.steps,
        vec![2],
        "the HOLE step (0 sketch, 1 extrude, 2 hole) is the one needing repair"
    );
    let broken_volume = exact_volume(&wm, body).await;
    assert!(
        (broken_volume - 10000.0).abs() < 1e-6,
        "an unresolvable host face drills NOTHING — the stock is back to solid 10000, \
         got {broken_volume}"
    );

    // (3) REPAIR through the NEW input path, with real evidence on the ref.
    rt.apply(EditCommand::EditOperationInput {
        record: hole,
        path: InputPath::HoleFace,
        reference: InputRef::Element(face_ref(body, &ElementId::new("el_side_face"), side)),
    })
    .expect("EditOperationInput{HoleFace} is accepted for a Hole record");
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "repaired hole");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "the re-picked face carries an anchor, so the ladder binds it"
    );

    // (4) THE PROOF: the drill crosses the box's 20 mm Y extent, not its 25 mm Z.
    let want_side = 10000.0 - removed_volume(6.0, 20.0, None, None);
    let got_side = exact_volume(&wm, body).await;
    assert!(
        (got_side - want_side).abs() < want_side * 1e-9,
        "the rebound hole must be drilled through the +Y face (20 mm of stock): \
         want {want_side}, got {got_side} (a top-face drill would read {want_top})"
    );
    assert!(
        (got_side - want_top).abs() > 100.0,
        "the two faces must give clearly different volumes for this to prove anything"
    );
    assert_eq!(
        rt.head_body_ids(),
        vec![body],
        "a rebind mints nothing and deletes nothing"
    );
    eprintln!("H9 HoleFace rebind PASS: top {got_top} → side {got_side}");
    wm.shutdown().await;
}

/// The core refuses a `HoleFace` rebind whose ref carries no identity, and
/// refuses the path on a record that is not a Hole — neither reaches the worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hole_face_rebind_is_refused_without_identity_or_on_the_wrong_op() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (body, top) = build_box(&mut rt, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 25.0).await;
    let hole = RecordId(Uuid::from_u128(HOLE_A));
    add_op(
        &mut rt,
        hole_record(HOLE_A, hole_params(body, "el_top_face", top, 6.0)),
    );
    let _ = regen_all(&mut rt).await;

    // Evidence-only ref (no `primary`): an explicit user re-pick that silently
    // fell through to the ladder would defeat the point of re-picking.
    let mut anchor_only = face_ref(body, &ElementId::new("x"), top);
    anchor_only.primary = None;
    let err = rt
        .apply(EditCommand::EditOperationInput {
            record: hole,
            path: InputPath::HoleFace,
            reference: InputRef::Element(anchor_only),
        })
        .expect_err("a hole face rebind needs a primary element id");
    assert!(
        format!("{err}").contains("primary element id"),
        "expected a primary-id rejection, got: {err}"
    );

    // The path is checked against the op: `HoleFace` on the EXTRUDE record is a
    // mis-repair, and the exact class of bug the FE slot table exists to prevent.
    let err = rt
        .apply(EditCommand::EditOperationInput {
            record: RecordId(Uuid::from_u128(EXTRUDE_A)),
            path: InputPath::HoleFace,
            reference: InputRef::Element(face_ref(body, &ElementId::new("y"), top)),
        })
        .expect_err("HoleFace must not apply to an Extrude");
    assert!(
        format!("{err}").contains("does not match the operation type"),
        "expected a path/op mismatch rejection, got: {err}"
    );
    wm.shutdown().await;
}
