//! Regen performance BASELINE against the REAL C++ OCCT worker, driven through the
//! app's [`DocumentRuntime`] exactly like `breadth_ops.rs` / `checkpoints.rs`.
//!
//! **Measurement only.** Nothing here asserts a time budget: machines differ, and a
//! wall-clock threshold in CI is a flake generator. What it asserts is that the run
//! is real — every regen PUBLISHES, the history really is ≥ 40 features, the head
//! body really tessellates, and the phase split is wired (a non-zero `worker_ms`
//! that fits inside the measured wall clock). The numbers are printed as a table so
//! a human can compare runs.
//!
//! The part is a 60×60×30 block with 19 blind pockets cut into it — 40 timeline
//! steps, every one of them a real OCCT boolean or profile solve, and none of them
//! carrying a topological element ref (a fillet/hole/chamfer baseline would have to
//! resolve edge and face ids out of each intermediate mesh, which makes a timing
//! fixture brittle for no timing insight).
//!
//! Cases: a cold replay from 0, then an edit at step 3, at mid-history (step 21) and
//! at the tail (step 39). **The three edit cases are expected to cost about what the
//! cold replay costs**: a checkpoint is only minted at head on explicit save
//! (`take_checkpoint_at_head`), so with no checkpoint below the dirty floor the
//! planner's `start_step` is 0 for every one of them (`RegenPlanner::plan_with_ceiling`).
//! That is the baseline's most useful reading, not a defect in the harness.
//!
//! Read the numbers with the build profile in mind: `cargo test` builds the Rust side
//! **unoptimized**, while the sidecar it drives is a Release worker. `planner_ms` (all
//! Rust: plan compile + prefix hashing) is therefore pessimistic relative to a
//! shipped build, and `worker_ms` is not.
//!
//! Gated on `ONECAD_WORKER_PATH` (else dev-tree fallback); a missing binary skips
//! cleanly unless `ONECAD_REQUIRE_WORKER=1`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport, RegenTimings};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::validate_mesh_blob;

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors breadth_ops.rs)
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

fn extrude_op(
    sketch: SketchId,
    dist: f64,
    boolean: BooleanMode,
    target: Option<BodyId>,
) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // empty ⇒ the worker's V1 first-region fallback
            region_identity_version: None,
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
    }))
}

fn extrude_record(
    rec: u128,
    sketch: SketchId,
    dist: f64,
    boolean: BooleanMode,
    target: Option<BodyId>,
) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        extrude_op(sketch, dist, boolean, target),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// The synthetic part: a 60×60×30 block with `POCKETS` blind pockets cut into it
// ─────────────────────────────────────────────────────────────────────────────

/// Base block: sketch at step 0, extrude at step 1.
const REC_BASE_SKETCH: u128 = 0x100;
const REC_BASE_EXTRUDE: u128 = 0x101;
/// Pocket `i` (1-based): sketch at step `2i`, cut extrude at step `2i+1`.
const REC_POCKET_SKETCH: u128 = 0x200;
const REC_POCKET_EXTRUDE: u128 = 0x300;
/// 19 pockets ⇒ 2 + 2×19 = 40 timeline steps.
const POCKETS: usize = 19;

fn base_body() -> BodyId {
    BodyId(Uuid::from_u128(REC_BASE_EXTRUDE))
}

/// The pocket's footprint origin on a 5×4 grid inside the 60×60 block — 6 mm squares
/// on a 12 mm pitch, so no two pockets touch and none reaches an outer wall.
fn pocket_origin(i: usize) -> (f64, f64) {
    let slot = i - 1;
    (
        3.0 + 12.0 * (slot % 5) as f64,
        3.0 + 12.0 * (slot / 5) as f64,
    )
}

/// The pocket's authored depth (varied so the mid/tail edits change real geometry).
fn pocket_depth(i: usize) -> f64 {
    4.0 + (i % 5) as f64
}

/// Appends the whole history. Returns the feature count (= timeline steps).
fn build_part(rt: &mut DocumentRuntime) -> usize {
    let base_sketch = SketchId(Uuid::from_u128(0x10));
    add_op(
        rt,
        sketch_record(
            REC_BASE_SKETCH,
            &rect_sketch(base_sketch, 0x1000, 0.0, 0.0, 60.0, 60.0),
        ),
    );
    add_op(
        rt,
        extrude_record(
            REC_BASE_EXTRUDE,
            base_sketch,
            30.0,
            BooleanMode::NewBody,
            None,
        ),
    );
    for i in 1..=POCKETS {
        let sid = SketchId(Uuid::from_u128(0x20 + i as u128));
        let (x0, y0) = pocket_origin(i);
        add_op(
            rt,
            sketch_record(
                REC_POCKET_SKETCH + i as u128,
                &rect_sketch(sid, 0x2000 + (i as u128) * 0x100, x0, y0, 6.0, 6.0),
            ),
        );
        add_op(
            rt,
            extrude_record(
                REC_POCKET_EXTRUDE + i as u128,
                sid,
                pocket_depth(i),
                BooleanMode::Cut,
                Some(base_body()),
            ),
        );
    }
    2 + 2 * POCKETS
}

/// Retargets pocket `i`'s cut depth — the scalar edit the timed regens react to.
fn edit_pocket_depth(rt: &mut DocumentRuntime, i: usize, depth: f64) {
    let sid = SketchId(Uuid::from_u128(0x20 + i as u128));
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(REC_POCKET_EXTRUDE + i as u128)),
        op: extrude_op(sid, depth, BooleanMode::Cut, Some(base_body())),
    })
    .expect("edit pocket depth");
}

// ─────────────────────────────────────────────────────────────────────────────
// Measurement
// ─────────────────────────────────────────────────────────────────────────────

/// One printed row: the case name, the phase split the runtime reported, the wall
/// clock this test measured around the whole regen call, and the head triangle count.
struct Row {
    case: &'static str,
    features: usize,
    timings: RegenTimings,
    elapsed_ms: f64,
    triangles: u64,
}

/// Runs one regen, times the whole call, and checks it published.
async fn timed_regen(
    rt: &mut DocumentRuntime,
    case: &'static str,
    features: usize,
    from: usize,
) -> Row {
    let started = Instant::now();
    let report: RegenReport = rt
        .run_regen(RegenRequest::ToEnd { from }, CancelToken::new())
        .await;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{case}: expected Published, got {:?}",
        report.outcome
    );
    assert!(
        report.failed_steps.is_empty(),
        "{case}: no step may fail, got {:?}",
        report.failed_steps
    );
    assert!(elapsed_ms.is_finite(), "{case}: wall clock must be finite");
    let triangles = head_triangles(rt).await;
    Row {
        case,
        features,
        timings: report.timings,
        elapsed_ms,
        triangles,
    }
}

/// Total triangles across every head body, read from the MESH1 blob the regen's
/// inline `artifacts.tessellate` seeded into the runtime's cache.
async fn head_triangles(rt: &mut DocumentRuntime) -> u64 {
    let mut total = 0u64;
    for body in rt.head_body_ids() {
        let blob = rt
            .get_mesh(body, Lod::Fine, None)
            .await
            .expect("head body has a display mesh");
        let view = validate_mesh_blob(&blob).expect("head MESH1 validates");
        total += u64::from(view.triangle_count);
    }
    total
}

fn print_table(rows: &[Row]) {
    eprintln!();
    eprintln!(
        "{:<26} | {:>8} | {:>10} | {:>9} | {:>7} | {:>10} | {:>9}",
        "case", "features", "planner_ms", "worker_ms", "mesh_ms", "elapsed_ms", "triangles"
    );
    eprintln!("{}", "-".repeat(96));
    for r in rows {
        eprintln!(
            "{:<26} | {:>8} | {:>10} | {:>9} | {:>7} | {:>10.1} | {:>9}",
            r.case,
            r.features,
            r.timings.planner_ms,
            r.timings.worker_ms,
            r.timings.mesh_ms,
            r.elapsed_ms,
            r.triangles
        );
    }
    eprintln!();
    eprintln!(
        "planner_ms = phase 1 plan compile · worker_ms = phase 2 worker round trips \
         (ExecutePlan op window + RestoreCheckpoint/AcceptPrepared) · mesh_ms = the \
         ExecutePlan post-ops window, where the worker tessellates the prepared bodies \
         and inlines their MESH1 blobs · elapsed_ms = wall clock around the whole \
         run_regen call (phases 1+2+3). Measurement only — no budget is asserted."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The baseline
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn regen_baseline_forty_feature_part() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let features = build_part(&mut rt);
    assert!(
        features >= 40,
        "the baseline part must carry at least 40 features, built {features}"
    );

    let mut rows = Vec::new();
    rows.push(timed_regen(&mut rt, "cold replay from 0", features, 0).await);

    // Step 3 = pocket 1's cut extrude (step 2i+1 for pocket i).
    edit_pocket_depth(&mut rt, 1, 9.0);
    rows.push(timed_regen(&mut rt, "edit step 3 (pocket 1)", features, 3).await);

    // Step 21 = pocket 10's cut extrude — mid history.
    edit_pocket_depth(&mut rt, 10, 11.0);
    rows.push(timed_regen(&mut rt, "edit step 21 (pocket 10)", features, 21).await);

    // Step 39 = pocket 19's cut extrude — the tail.
    edit_pocket_depth(&mut rt, POCKETS, 12.0);
    rows.push(timed_regen(&mut rt, "edit step 39 (pocket 19)", features, 39).await);

    for row in &rows {
        assert!(
            row.triangles > 0,
            "{}: the head body must tessellate",
            row.case
        );
        // The phase split is a partition of the measured call, so a wired clock lands
        // inside it. (+2 ms of slack: each figure is truncated to whole ms.)
        let split = row.timings.planner_ms + row.timings.worker_ms + row.timings.mesh_ms;
        assert!(
            (split as f64) <= row.elapsed_ms + 2.0,
            "{}: split {split} ms must fit inside the measured {:.1} ms",
            row.case,
            row.elapsed_ms
        );
        assert!(
            row.timings.worker_ms > 0,
            "{}: 40 real OCCT steps cannot cost 0 ms of worker time — the phase clock \
             is not wired",
            row.case
        );
    }

    print_table(&rows);
    wm.shutdown().await;
}
