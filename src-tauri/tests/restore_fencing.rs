//! WP-H H0 probe (b) — `RestoreCheckpoint` vs the UNFENCED reader verbs, against
//! the REAL C++ OCCT worker.
//!
//! `Session::restore_checkpoint` installs the checkpoint's bodies + partition into
//! the LIVE head and bumps `snapshotId`. `Tessellate` (and the export verbs) read
//! that live head with no `snapshotId` fence — `tessellate_args` sends none — so
//! between a restore and the plan it seeds, a mesh request serves the ROLLED-BACK
//! geometry while Rust still believes the head is the post-edit one.
//!
//! Two assertions, both for the CORRECT behaviour (H2: restore into a separate
//! `restored_base_` slot, the head changing only at `AcceptPrepared`):
//!   1. the head `snapshotId` does not move on a restore;
//!   2. an unfenced `Tessellate` right after a restore still serves the CURRENT
//!      head's geometry, not the checkpoint's.
//!
//! REQUIRE_WORKER-guarded (CI hard-fails without a worker; local dev skips cleanly).

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
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId, SnapshotId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, CheckpointId, CheckpointRef, GeometryEngine, Lod, ModelSnapshot, Outcome,
    RegenRequest, RestoreRequest,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors checkpoints.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} set but no binary there"
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
        "worker must connect + OpenSession"
    );
    wm
}

fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

async fn regen(rt: &mut DocumentRuntime, from: usize) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from }, CancelToken::new())
        .await
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

/// `(vertexCount, bboxMaxZ)` off a MESH1 header (mesh_format.md §2: `vertexCount`
/// at 0x08, `bboxMaxZ` as an f32 at 0x34, all little-endian).
fn mesh_shape(blob: &[u8]) -> (u32, f32) {
    assert!(
        blob.len() >= 64,
        "MESH1 header is 64 bytes, got {}",
        blob.len()
    );
    let vertices = u32::from_le_bytes(blob[0x08..0x0C].try_into().unwrap());
    let max_z = f32::from_le_bytes(blob[0x34..0x38].try_into().unwrap());
    (vertices, max_z)
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op builders (a fully-constrained rectangle, as checkpoints.rs)
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
    let pt = |sk: &mut Sketch, id, x, y| {
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
    let (_plane, entities, constraints) = onecad_lib::worker::wire::sketch_wire(sk);
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

fn extrude_params(sketch: SketchId, dist: f64) -> ExtrudeParams {
    ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""),
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
    }
}

const SK_A: u128 = 0xA00;
const EX_A: u128 = 0xA01;

// ─────────────────────────────────────────────────────────────────────────────
// The probe
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restore_does_not_roll_back_the_head_readers_see() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // ── (1) box A, 40 × 20 × 25 ────────────────────────────────────────────────
    let sa = SketchId(Uuid::from_u128(0xA));
    rt.apply(EditCommand::AddOperation {
        record: sketch_record(SK_A, &rect_sketch(sa, 0x1000, 40.0, 20.0)),
        at_cursor: true,
    })
    .expect("AddOperation sketch");
    rt.apply(EditCommand::AddOperation {
        record: OperationRecord::new(
            RecordId(Uuid::from_u128(EX_A)),
            0,
            "Extrude",
            Operation::Known(KnownOperation::Extrude(extrude_params(sa, 25.0))),
        ),
        at_cursor: true,
    })
    .expect("AddOperation extrude");
    let report = regen(&mut rt, 0).await;
    let snap = published(&report, "box A @25");
    assert_eq!(snap.bodies.len(), 1, "one body");
    let body: BodyId = snap.bodies[0].body;

    // ── (2) checkpoint the head (step 1 = the extrude) ─────────────────────────
    let artifacts = wm
        .save_checkpoint(1)
        .await
        .expect("SaveCheckpoint at the box-A head");

    // ── (3) EDIT: the same extrude, now 60 mm tall. The head moves. ────────────
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(EX_A)),
        op: Operation::Known(KnownOperation::Extrude(extrude_params(sa, 60.0))),
    })
    .expect("UpdateOperationParams extrude 25 -> 60");
    let report = regen(&mut rt, 0).await;
    let snap = published(&report, "box A @60");
    assert_eq!(snap.bodies.len(), 1, "still one body after the edit");
    assert_eq!(snap.bodies[0].body, body, "the edit keeps the same BodyId");

    // ── (4) the head as Rust sees it, and the mesh it serves ───────────────────
    let head_before = wm.get_worker_head().await.expect("head before the restore");
    let mesh_before = wm
        .fetch_mesh(body, Lod::Coarse, SnapshotId(head_before.snapshot_id.0))
        .await
        .expect("Tessellate the post-edit head");
    let (verts_before, max_z_before) = mesh_shape(&mesh_before);
    assert!(
        (max_z_before - 60.0).abs() < 0.01,
        "sanity: the post-edit head is 60 mm tall, got bboxMaxZ={max_z_before}"
    );

    // ── (5) RestoreCheckpoint, and NOTHING else — no plan is seeded ────────────
    let restore = wm
        .restore_checkpoint(RestoreRequest {
            checkpoint: CheckpointRef {
                step_index: 1,
                checkpoint_id: CheckpointId::new("probe_ckpt_1"),
            },
            expected_history_prefix_hash: artifacts.history_prefix_hash.clone(),
            worker_epoch: wm.epoch(),
            artifacts: Some(artifacts.clone()),
        })
        .await
        .expect("RestoreCheckpoint");
    assert!(
        restore.restored && !restore.drift_detected,
        "probe precondition: the checkpoint must restore cleanly, got {restore:?}"
    );

    let head_after = wm.get_worker_head().await.expect("head after the restore");
    let mesh_after = wm
        .fetch_mesh(body, Lod::Coarse, SnapshotId(head_after.snapshot_id.0))
        .await
        .expect("Tessellate right after the restore");
    let (verts_after, max_z_after) = mesh_shape(&mesh_after);

    eprintln!(
        "restore probe: head snapshotId {} -> {} (restore reported {}); unfenced Tessellate \
         bboxMaxZ {max_z_before} -> {max_z_after} (vertices {verts_before} -> {verts_after})",
        head_before.snapshot_id.0, head_after.snapshot_id.0, restore.snapshot_id.0
    );

    // Both properties are collected so ONE run reports every violation.
    let mut violations: Vec<String> = Vec::new();
    // THE PROBE (1): an unfenced reader still sees the CURRENT head's geometry.
    if (max_z_after - max_z_before).abs() >= 0.01 {
        violations.push(format!(
            "an unfenced Tessellate after a restore served the ROLLED-BACK geometry: bboxMaxZ \
             {max_z_after} (the checkpoint's 25 mm box) instead of the head's {max_z_before}"
        ));
    }
    // THE PROBE (2): a restore prepares a BASE; the head only moves at AcceptPrepared.
    if head_after.snapshot_id.0 != head_before.snapshot_id.0 {
        violations.push(format!(
            "RestoreCheckpoint moved the head snapshotId {} -> {} — the checkpoint was \
             installed as the live head instead of a restored base",
            head_before.snapshot_id.0, head_after.snapshot_id.0
        ));
    }
    assert!(
        violations.is_empty(),
        "restore-into-head hazards: {}",
        violations.join(" ;; ")
    );

    wm.shutdown().await;
}
