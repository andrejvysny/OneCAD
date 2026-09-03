//! Component Library WP-0.1 P0 go/no-go: `ClassifyElement`'s interactive
//! latency against the REAL C++ OCCT worker.
//!
//! The placement/mate-snap solver's hover gesture (spec §5.2) issues one
//! `ClassifyElement` per hover frame. The kernel call itself is cheap (a
//! couple of `BRepAdaptor_*` reads on an already-resolved shape); the real
//! risk is the stdio round-trip. This measures exactly that, against a
//! representative box body — 6 planar faces, 12 straight edges, the same
//! `rect_sketch`/`extrude_record` shape `hole_ops.rs` uses — and prints
//! p50/p95/p99 over N=500 calls, mixing face and edge targets.
//!
//! **Gate: p95 < 16 ms.** A pass is the "go" half of P0 (spec §10); a miss
//! means the live-hover gesture (WP-1.5) falls back to click-to-classify
//! instead — the decision this test exists to make measurable.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing
//! binary skips cleanly (`ONECAD_REQUIRE_WORKER=1` makes that a hard failure).

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
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors hole_ops.rs / preview_edge_shell.rs)
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

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (verbatim shape from hole_ops.rs)
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

/// Builds a 20×20×25 box and returns its `BodyId`.
async fn build_box(rt: &mut DocumentRuntime) -> BodyId {
    let sid = SketchId(Uuid::from_u128(0x5c));
    add_op(
        rt,
        sketch_record(0x1000, &rect_sketch(sid, 0x1100, 20.0, 20.0)),
    );
    add_op(rt, extrude_record(0x1001, sid, 25.0));
    let report = regen_all(rt).await;
    let snap = published(&report, "build_box");
    snap.bodies.first().expect("one published body").body
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn classify_element_p95_latency_under_16ms() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let body = build_box(&mut rt).await;

    // Ordinal topoKeys of a fresh box: 6 faces, 12 edges (`TopExp::MapShapes`
    // order over `BRepPrimAPI_MakeBox`, the same ordinal scheme
    // `ElementMapPartition::shape_for_topokey` resolves).
    let targets: Vec<String> = (1..=6)
        .map(|i| format!("f:{i}"))
        .chain((1..=12).map(|i| format!("e:{i}")))
        .collect();

    // Correctness sanity, not just timing: a box face classifies as a plane
    // with a real frame; a box edge classifies as a line.
    let face = ElementQuery::classify_element_by_topo_key(&wm, body, "f:1")
        .await
        .expect("ClassifyElement(f:1) live")
        .expect("f:1 present on a fresh box");
    assert_eq!(face.kind, "face");
    assert_eq!(face.surface_type, "plane");
    assert!(
        face.frame.is_some(),
        "a planar face carries a seatable frame"
    );
    let edge = ElementQuery::classify_element_by_topo_key(&wm, body, "e:1")
        .await
        .expect("ClassifyElement(e:1) live")
        .expect("e:1 present on a fresh box");
    assert_eq!(edge.kind, "edge");
    assert_eq!(edge.curve_type, "line");

    // The timing gate: N=500 calls cycling the mixed face/edge target set,
    // recording wall-clock per call over the live stdio round-trip.
    const N: usize = 500;
    let mut samples = Vec::with_capacity(N);
    for i in 0..N {
        let topo_key = &targets[i % targets.len()];
        let t0 = Instant::now();
        let result = ElementQuery::classify_element_by_topo_key(&wm, body, topo_key)
            .await
            .expect("ClassifyElement live");
        samples.push(t0.elapsed());
        assert!(result.is_some(), "{topo_key} present on the unchanged box");
    }
    samples.sort();

    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    let p99 = percentile(&samples, 0.99);
    println!(
        "ClassifyElement latency over {N} calls: p50={:.2}ms p95={:.2}ms p99={:.2}ms",
        p50.as_secs_f64() * 1000.0,
        p95.as_secs_f64() * 1000.0,
        p99.as_secs_f64() * 1000.0,
    );

    assert!(
        p95 < Duration::from_millis(16),
        "P0 go/no-go: p95={:.2}ms >= 16ms — live-hover classify is too slow; \
         WP-1.5 falls back to click-to-classify-then-preview",
        p95.as_secs_f64() * 1000.0,
    );

    wm.shutdown().await;
}
