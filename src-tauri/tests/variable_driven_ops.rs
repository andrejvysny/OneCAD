//! WP-VE.1 — document variables actually drive geometry, against the REAL C++
//! OCCT worker, driven through the app's [`DocumentRuntime`] exactly like
//! `offset_face.rs` / `component_ops.rs`.
//!
//! Before this work package a `Scalar`'s `expr` was stored and never read:
//! every consumer took the cached `value`, so editing the variable table moved
//! nothing at all (verified empirically — a `SetVariable 10 → 20` produced
//! bit-identical volumes). These tests are the gate that keeps it read.
//!
//! Volumes come from `QueryMassProperties` (`BRepGProp` over the real BRep), so
//! the analytic expectations hold to kernel precision, not to a mesh chord.
//!
//! * `variable_drives_extrude_distance` — an extrude whose `distance` is bound
//!   to `depth` builds the TABLE's number (never the record's stale cache), and
//!   `SetVariable 10 → 20` doubles the volume 4000 → 8000.
//! * `a_missing_variable_fails_that_step_and_leaves_the_others_alone` — an
//!   unresolvable binding fails AS THAT STEP (`FeatureStatus::Error`, a message
//!   naming the variable and the field) while the upstream sketch step stays
//!   fine, and NEVER silently falls back to the stale cached value.
//! * `resolved_values_are_written_back_and_survive_a_save_reopen` — the stored
//!   record keeps `expr` AND its stale `value` until a regen commits; the
//!   resolved number is then written back and round-trips through the container.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly unless `ONECAD_REQUIRE_WORKER=1`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::{Scalar, Unit, Variable};
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId, VariableId};
use onecad_core::io::container::{ContainerReader, SaveMeta};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::dto::FeatureStatus;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};

// Fixed ids.
const SKETCH_REC: u128 = 0xBE0;
const EXTRUDE_REC: u128 = 0xBE1;

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors offset_face.rs / component_ops.rs)
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

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-8.0.1".into()),
        created: "2026-08-13T00:00:00Z".into(),
        modified: "2026-08-13T00:00:00Z".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (rect_sketch verbatim from offset_face.rs)
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
    let op = Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch: sk.id,
        plane: xy_plane_ref(),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Sketch", op)
}

/// An extrude whose `distance` is EXPRESSION-DRIVEN: `cached` is the stale
/// last-evaluated number the pass must ignore, `expr` names the variable.
fn extrude_record_bound(rec: u128, sketch: SketchId, cached: f64, expr: &str) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // first-region fallback
            region_identity_version: None,
            extra: Default::default(),
        }),
        distance: Scalar::with_expr(cached, expr),
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
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Extrude", op)
}

fn var(name: &str, value: f64) -> (VariableId, Variable) {
    let id = VariableId::new();
    (
        id,
        Variable {
            id,
            name: name.to_string(),
            value: Scalar::new(value),
            unit: Unit::Mm,
        },
    )
}

/// The projection row for a record: `(status, status_message, primary_value)`.
/// `primary_value` is read off the **STORED** record (`DocumentProjection` walks
/// the authoritative document, not the regen mirror), which is what makes it a
/// probe of the no-mutation rule.
fn row(rt: &DocumentRuntime, rec: u128) -> (FeatureStatus, Option<String>, Option<f64>) {
    let id = RecordId(Uuid::from_u128(rec)).to_string();
    let f = rt
        .projection()
        .features
        .into_iter()
        .find(|f| f.id == id)
        .unwrap_or_else(|| panic!("no feature row for record {id}"));
    (f.status, f.status_message, f.primary_value)
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. The variable IS the geometry
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn variable_drives_extrude_distance() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xBEA));

    let (depth_id, depth) = var("depth", 10.0);
    rt.apply(EditCommand::AddVariable { variable: depth })
        .expect("AddVariable depth");
    add_op(
        &mut rt,
        sketch_record(SKETCH_REC, &rect_sketch(sid, 0x1000, 0.0, 0.0, 20.0, 20.0)),
    );
    // The cached `value` is deliberately absurd: if the pass ever falls back to
    // it the volume is 400 × 999, not 400 × 10.
    add_op(
        &mut rt,
        extrude_record_bound(EXTRUDE_REC, sid, 999.0, "depth"),
    );

    let report = regen_all(&mut rt).await;
    let _ = published(&report, "variable-driven extrude");
    let body = body_of(EXTRUDE_REC);
    let first = exact_volume(&wm, body).await;
    assert!(
        (first - 4_000.0).abs() < 1e-6,
        "20×20×depth(10) = 4000 — the TABLE's value, not the record's cached 999; got {first}"
    );

    // THE PROOF: move the variable, not the record.
    rt.apply(EditCommand::SetVariable {
        variable: depth_id,
        value: Scalar::new(20.0),
    })
    .expect("SetVariable depth 10 → 20");
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "regen after SetVariable");
    let second = exact_volume(&wm, body).await;
    assert!(
        (second - 8_000.0).abs() < 1e-6,
        "20×20×depth(20) = 8000 after SetVariable; got {second} (was {first})"
    );

    wm.shutdown().await;
    eprintln!("WP-VE.1 PASS: depth 10 → 20 moved the volume {first} → {second}");
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. A broken binding fails LOUDLY, as that step, never on a stale value
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_variable_fails_that_step_and_leaves_the_others_alone() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xBEB));

    let (_, depth) = var("depth", 10.0);
    rt.apply(EditCommand::AddVariable { variable: depth })
        .expect("AddVariable depth");
    add_op(
        &mut rt,
        sketch_record(SKETCH_REC, &rect_sketch(sid, 0x2000, 0.0, 0.0, 20.0, 20.0)),
    );
    // `height` is NOT in the table. The cached 12.5 must never be used.
    add_op(
        &mut rt,
        extrude_record_bound(EXTRUDE_REC, sid, 12.5, "height"),
    );

    let report = regen_all(&mut rt).await;

    // The offending step is Error, with a message naming BOTH the field and the
    // variable — a bare "regen failed" would not be actionable.
    let (status, message, _) = row(&rt, EXTRUDE_REC);
    assert_eq!(
        status,
        FeatureStatus::Error,
        "an unresolvable variable binding must fail AS THAT STEP"
    );
    let message = message.expect("the errored step carries a reason");
    assert!(
        message.contains("height") && message.contains("Extrude.distance"),
        "the reason must name the variable and the field, got: {message}"
    );

    // The upstream step is untouched — this is a per-step refusal, not a
    // document-wide one.
    let (sketch_status, _, _) = row(&rt, SKETCH_REC);
    assert_ne!(
        sketch_status,
        FeatureStatus::Error,
        "the upstream sketch step must not be dragged down"
    );
    assert!(
        report
            .failed_steps
            .iter()
            .any(|f| f.record_id == RecordId(Uuid::from_u128(EXTRUDE_REC)).to_string()),
        "the regen report must surface the failed step: {:?}",
        report.failed_steps
    );

    // And NO geometry was built off the stale 12.5.
    if let Outcome::Published(snap) = &report.outcome {
        assert!(
            snap.bodies.is_empty(),
            "the blocked extrude must publish no body (a stale-value fallback would \
             have produced one): {:?}",
            snap.bodies
        );
    }

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Stored records are effective-copy-only until the regen commits, then the
//    resolved value is written back and round-trips through the container
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolved_values_are_written_back_and_survive_a_save_reopen() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xBEC));
    let dir = tempfile::tempdir().expect("tempdir");
    let before_path = dir.path().join("before.onecad");
    let after_path = dir.path().join("after.onecad");

    let (_, depth) = var("depth", 10.0);
    rt.apply(EditCommand::AddVariable { variable: depth })
        .expect("AddVariable depth");
    add_op(
        &mut rt,
        sketch_record(SKETCH_REC, &rect_sketch(sid, 0x3000, 0.0, 0.0, 20.0, 20.0)),
    );
    add_op(
        &mut rt,
        extrude_record_bound(EXTRUDE_REC, sid, 999.0, "depth"),
    );

    // PRE-REGEN: the substitution ran (the regen mirror already holds 10), yet
    // the STORED record is untouched — `expr` plus its stale 999.
    assert_eq!(
        row(&rt, EXTRUDE_REC).2,
        Some(999.0),
        "planning substitutes on effective COPIES; the stored record must not move"
    );
    rt.save(&before_path, save_meta()).expect("save pre-regen");
    let (value, expr) = saved_extrude_distance(&before_path);
    assert_eq!(expr.as_deref(), Some("depth"));
    assert_eq!(value, 999.0, "pre-regen the stale cached value round-trips");

    // POST-REGEN: the derived writeback lands the number the plan was hashed and
    // built with.
    let _ = published(&regen_all(&mut rt).await, "variable-driven extrude");
    assert_eq!(
        row(&rt, EXTRUDE_REC).2,
        Some(10.0),
        "a committed regen writes the RESOLVED value back onto the stored record"
    );
    rt.save(&after_path, save_meta()).expect("save post-regen");
    let (value, expr) = saved_extrude_distance(&after_path);
    assert_eq!(
        expr.as_deref(),
        Some("depth"),
        "the writeback must never drop the binding"
    );
    assert_eq!(value, 10.0);

    wm.shutdown().await;
}

/// `(value, expr)` of the saved container's extrude `distance` — read back
/// through the real container reader, so this is a genuine serde round-trip.
fn saved_extrude_distance(path: &std::path::Path) -> (f64, Option<String>) {
    let loaded = ContainerReader::open(path).expect("reopen the saved container");
    let rec = loaded
        .document()
        .timeline
        .record_by_id(RecordId(Uuid::from_u128(EXTRUDE_REC)))
        .expect("the extrude record survives the round-trip");
    let Operation::Known(KnownOperation::Extrude(p)) = &rec.op else {
        panic!("the extrude record must round-trip as a typed Extrude");
    };
    (p.distance.value, p.distance.expr.clone())
}
