//! WP-D1 blast-radius isolation against the REAL C++ OCCT worker, driven through
//! the app's [`DocumentRuntime`] exactly like `transform_body.rs` / `breadth_ops.rs`.
//!
//! A step that fails must not delete or block bodies **outside its dependency
//! closure**. Before WP-D1 one broken feature truncated the whole plan and every
//! later body — including an imported vendor component — vanished from the
//! published snapshot, which made light multi-part fit-check unusable.
//!
//! The timeline here is two genuinely independent parts with a break between them:
//!
//! ```text
//!   0 Sketch A · 1 Extrude A · 2 Extrude A with a BOGUS regionId (fails)
//!   3 Sketch B · 4 Extrude B
//! ```
//!
//! Steps 3 and 4 consume no body and no sketch the halt touches, so their
//! independence is PROVABLE (`onecad_core::regen::isolation`) and the isolation
//! pass runs them. Step 2 stays loudly `Error`; step 1's body is untouched.
//!
//! Gated on `ONECAD_WORKER_PATH` (else dev-tree fallback); a missing binary skips
//! cleanly unless `ONECAD_REQUIRE_WORKER=1`.

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
use onecad_core::history::StepState;
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest, StoppedReason,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness + sketch builders (verbatim from transform_body.rs / breadth_ops.rs)
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

/// An extrude of `sketch`'s region `region` — an EMPTY region id takes the V1
/// first-region fallback, a non-empty one MUST match or the op fails loudly with
/// the available ids (the deterministic worker-side break this gate needs).
fn extrude_record(rec: u128, sketch: SketchId, region: &str, dist: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(region),
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

const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;
const BROKEN: u128 = 0xA02;
const SKETCH_B: u128 = 0xB00;
const EXTRUDE_B: u128 = 0xB01;

/// The product gate: one part breaks, the OTHER part stays on screen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_broken_feature_does_not_take_an_independent_part_with_it() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sk_a = SketchId(Uuid::from_u128(0xA10));
    let sk_b = SketchId(Uuid::from_u128(0xB10));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sk_a, 0xA20, 0.0, 0.0, 20.0, 20.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_A, sk_a, "", 25.0));
    // The break, sitting BETWEEN the two parts.
    add_op(&mut rt, extrude_record(BROKEN, sk_a, "no-such-region", 5.0));
    add_op(
        &mut rt,
        sketch_record(SKETCH_B, &rect_sketch(sk_b, 0xB20, 60.0, 0.0, 10.0, 10.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_B, sk_b, "", 8.0));

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "isolated regen");

    // The halt is LOUD — never silently dropped or silently completed.
    assert_eq!(snap.stopped_reason, StoppedReason::OpFailed);

    let part_a = body_of(EXTRUDE_A);
    let part_b = body_of(EXTRUDE_B);
    let published_bodies: Vec<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    assert!(
        published_bodies.contains(&part_a),
        "the pre-halt part survives: {published_bodies:?}"
    );
    assert!(
        published_bodies.contains(&part_b),
        "the part with NO dependency on the break must survive it: {published_bodies:?}"
    );
    assert_eq!(
        published_bodies.len(),
        2,
        "and nothing else: {published_bodies:?}"
    );
    assert!(
        !report.removed.contains(&part_b),
        "the independent part is never reported removed: {:?}",
        report.removed
    );

    // The timeline reads honestly: the break is Error, its independent successors
    // are Valid, and the valid LINEAR prefix still ends below the break.
    let states: std::collections::BTreeMap<usize, StepState> =
        snap.step_states.iter().cloned().collect();
    assert!(
        matches!(states.get(&2), Some(StepState::Error { .. })),
        "step 2 is Error: {states:?}"
    );
    assert_eq!(states.get(&3), Some(&StepState::Valid), "{states:?}");
    assert_eq!(states.get(&4), Some(&StepState::Valid), "{states:?}");
    assert_eq!(
        snap.step_index,
        Some(1),
        "the valid linear prefix ends below the halt"
    );

    // Design point 3: the exclusion was ephemeral — no record was suppressed.
    assert!(
        rt.projection().features.iter().all(|f| !f.suppressed),
        "the isolation pass must never touch the record `suppressed` flag"
    );
    // And the break still explains itself to the user.
    assert_eq!(report.failed_steps.len(), 1, "{:?}", report.failed_steps);
    assert_eq!(
        report.failed_steps[0].record_id,
        RecordId(Uuid::from_u128(BROKEN)).to_string()
    );
}
