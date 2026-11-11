//! Real-worker regressions for the reference and publication lifecycle defects
//! found by the 8103a3af kernel audit and its independent second review.
//!
//! Each test drives the production `WorkerManager` against the real OCCT
//! sidecar; there is no stub fallback. `ONECAD_REQUIRE_WORKER=1` turns a missing
//! binary into a failure instead of a quiet skip.
//!
//! * audit 2 / SR-02 — a rename during a regen must not strand its geometry.
//! * audit 8 / SR-04 — a promotion during a regen survives the regen's commit.
//! * SR-05 (audit's unnumbered claim) — rollback to an empty prefix keeps the
//!   durable evidence through roll-forward, save and a fresh-process reopen.
//! * NEW-02 — a completed rebind is owed again after a same-epoch full replay.
//! * audit 11 / SR-07 — a mesh re-fetched at a newer head is never adopted
//!   under the older head's identity.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use uuid::Uuid;

use onecad_app::document_runtime::{DocumentRuntime, RegenReport};
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};
use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, RegenHint};
use onecad_core::ids::{
    BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId, TopoKey,
};
use onecad_core::io::container::{ContainerReader, SaveMeta};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest, RegenScheduler,
    SchedulerStatus,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

const SK_A: u128 = 0xA00;
const EX_A: u128 = 0xA01;
const RED: [u8; 4] = [255, 0, 0, 255];

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
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved"
    );
    None
}

async fn fresh(bin: &Path) -> (WorkerManager, DocumentRuntime) {
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin.to_path_buf()));
    assert!(
        wm.wait_ready(Duration::from_secs(20)).await,
        "worker startup"
    );
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let rt = DocumentRuntime::new_blank(engine, meshes, solver);
    (wm, rt)
}

fn reopen(path: &Path, wm: &WorkerManager) -> DocumentRuntime {
    DocumentRuntime::open(
        path,
        Arc::new(wm.clone()),
        Arc::new(wm.clone()),
        Arc::new(wm.clone()),
    )
    .expect("saved document reopens")
}

async fn replay(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::RevertToEnd { from: 0 }, CancelToken::new())
        .await
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(snapshot) => snapshot,
        other => panic!("{what}: {other:?}"),
    }
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "reference-lifecycle-test".into(),
        occt_fingerprint: None,
        created: "2026-11-11T00:00:00Z".into(),
        modified: "2026-11-11T00:00:00Z".into(),
    }
}

fn mesh_z_span(bytes: &[u8]) -> f64 {
    let mesh = onecad_protocol::mesh::MeshView::decode(bytes, Default::default()).unwrap();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for i in 0..mesh.header().vertex_count {
        let z = f64::from(mesh.position(i as usize).unwrap()[2]);
        lo = lo.min(z);
        hi = hi.max(z);
    }
    hi - lo
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
    let pt = |sk: &mut Sketch, id, x, y| {
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
    for (id, a, b) in [
        (l0, p0s, p0e),
        (l1, p1s, p1e),
        (l2, p2s, p2e),
        (l3, p3s, p3e),
    ] {
        sk.add_entity(SketchEntity::line(id, a, b, false)).unwrap();
    }
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
    let (_plane, entities, constraints) = onecad_app::worker::wire::sketch_wire(sk);
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sk.id,
            plane: xy_plane_ref(),
            entities: entities.as_array().cloned().unwrap_or_default(),
            constraints: constraints.as_array().cloned().unwrap_or_default(),
            host_face: None,
            extra: Default::default(),
        })),
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
        })),
    )
}

/// Builds sketch+extrude into `rt`, regens, and returns the published body.
async fn build_and_regen(rt: &mut DocumentRuntime) -> BodyId {
    let sa = SketchId(Uuid::from_u128(0xA));
    for record in [
        sketch_record(SK_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
        extrude_record(EX_A, sa, 25.0),
    ] {
        rt.apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .expect("AddOperation");
    }
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let snap = published(&report, "build");
    assert_eq!(snap.bodies.len(), 1, "one box body");
    snap.bodies[0].body
}

fn face_pick() -> Vec<(TopoKey, Option<onecad_core::document::refs::AnchorIntent>)> {
    vec![(TopoKey::new("f:1"), None)]
}

fn set_extrude_depth(rt: &mut DocumentRuntime, depth: f64) {
    let mut record = rt
        .operation_record(RecordId(Uuid::from_u128(EX_A)))
        .unwrap();
    let Operation::Known(KnownOperation::Extrude(params)) = &mut record.op else {
        panic!("extrude");
    };
    params.distance = Scalar::new(depth);
    rt.apply(EditCommand::UpdateOperationParams {
        record: record.record_id,
        op: record.op,
    })
    .unwrap();
}

/// The production guard from `regen_driver.rs`: rebind only when owed.
async fn rebind_if_owed(rt: &mut DocumentRuntime) -> Option<(usize, usize)> {
    if !rt.has_unbound_persisted_elements() {
        return None;
    }
    Some(rt.rebind_persisted_elements().await)
}

async fn bound(wm: &WorkerManager, rt: &DocumentRuntime, body: BodyId, id: &ElementId) -> bool {
    wm.query_element(rt.head_snapshot_id().unwrap(), body, id.as_str())
        .await
        .unwrap()
        .is_some()
}

/// Audit 2 / SR-02 through the production scheduler and driver: the rename
/// lands after the plan is prepared and before the worker result is adopted.
#[tokio::test]
async fn rename_during_regen_still_publishes_the_new_body() {
    let Some(bin) = real_worker() else { return };
    let (wm, mut rt) = fresh(&bin).await;
    let body = build_and_regen(&mut rt).await;
    let second = SketchId(Uuid::from_u128(0xB));
    for record in [
        sketch_record(0xB00, &rect_sketch(second, 0x2000, 50.0, 0.0, 20.0, 20.0)),
        extrude_record(0xB01, second, 5.0),
    ] {
        rt.apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .unwrap();
    }
    let runtime = Arc::new(tokio::sync::Mutex::new(Some(rt)));
    let outcomes = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = outcomes.clone();
    let renamer = runtime.clone();
    let pending = Mutex::new(Some(()));
    let driver = onecad_app::regen_driver_with_started(
        runtime.clone(),
        Arc::new(move |report: &RegenReport, _| {
            sink.lock().unwrap().push(report.outcome_str().to_string());
        }),
        Arc::new(move |_| {
            if pending.lock().unwrap().take().is_none() {
                return;
            }
            let mut guard = renamer.try_lock().expect("lock is free during the drive");
            let outcome = guard
                .as_mut()
                .unwrap()
                .apply(EditCommand::RenameBody {
                    body,
                    name: "Renamed while geometry pending".into(),
                })
                .unwrap();
            assert_eq!(outcome.regen, RegenHint::None);
        }),
        Arc::new(|_| Box::pin(async {})),
        Arc::new(tokio::sync::watch::channel(0u64).0),
    );
    let (scheduler, handle) = RegenScheduler::new(driver);
    let mut status = handle.subscribe_status();
    handle.request(RegenRequest::ToEnd { from: 2 });
    let task = tokio::spawn(scheduler.run());
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            status.changed().await.unwrap();
            if !matches!(*status.borrow(), SchedulerStatus::Running { .. }) {
                break;
            }
        }
    })
    .await
    .expect("regen lane settles");
    task.abort();
    assert_eq!(*outcomes.lock().unwrap(), ["superseded", "published"]);
    let guard = runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    assert_eq!(rt.projection().features.len(), 4);
    assert_eq!(rt.head_body_ids().len(), 2, "the owed body is published");
    assert_eq!(
        rt.projection().bodies[&body.to_string()].name,
        "Renamed while geometry pending"
    );
    let added = rt
        .head_body_ids()
        .into_iter()
        .find(|id| *id != body)
        .unwrap();
    let mass = wm
        .query_mass_properties(added, added.to_string())
        .await
        .unwrap();
    assert!(
        (mass.volume - 2_000.0).abs() < 2_000.0 * 1e-9,
        "20×20×5 box"
    );
    drop(guard);
    wm.shutdown().await;
}

/// Audit 8 / SR-04: same persistent id, same evidence, re-bound on the new head.
#[tokio::test]
async fn promotion_during_replay_survives_and_rebinds() {
    let Some(bin) = real_worker() else { return };
    let (wm, mut rt) = fresh(&bin).await;
    let body = build_and_regen(&mut rt).await;
    let snapshot = rt.head_snapshot_id().unwrap();
    let prepared = rt
        .begin_regen(RegenRequest::RevertToEnd { from: 0 })
        .unwrap();
    let promoted = rt
        .promote_selection(snapshot, body, face_pick())
        .await
        .unwrap();
    let element = ElementId::new(promoted[0].element_id.clone());
    let report = rt.finish_regen(prepared.drive(CancelToken::new()).await);
    assert!(report.published());
    assert_eq!(rt.persisted_element_count(), 1, "promotion kept");
    assert_eq!(rebind_if_owed(&mut rt).await, Some((1, 0)));
    assert!(
        bound(&wm, &rt, body, &element).await,
        "same id is bound again"
    );
    wm.shutdown().await;
}

/// SR-05: rollback to zero hides geometry and keeps the coloured face's
/// identity through roll-forward, save and a fresh worker process.
#[tokio::test]
async fn rollback_to_empty_keeps_face_colour_identity_through_cold_reopen() {
    let Some(bin) = real_worker() else { return };
    let (wm, mut rt) = fresh(&bin).await;
    let body = build_and_regen(&mut rt).await;
    let promoted = rt
        .promote_selection(rt.head_snapshot_id().unwrap(), body, face_pick())
        .await
        .unwrap();
    let element = ElementId::new(promoted[0].element_id.clone());
    rt.apply(EditCommand::SetFaceColor {
        body,
        element_id: element.clone(),
        color: Some(RED),
    })
    .unwrap();

    rt.apply(EditCommand::SetRollback { cursor: 0 }).unwrap();
    assert!(replay(&mut rt).await.published());
    assert!(
        rt.head_body_ids().is_empty(),
        "the empty prefix shows nothing"
    );
    assert_eq!(rt.persisted_element_count(), 1, "evidence stays dormant");
    assert_eq!(
        rebind_if_owed(&mut rt).await.unwrap_or_default(),
        (0, 0),
        "no body exists, so nothing may be bound"
    );

    rt.apply(EditCommand::SetRollback { cursor: 2 }).unwrap();
    assert!(replay(&mut rt).await.published());
    assert_eq!(rt.head_body_ids(), vec![body]);
    assert_eq!(rebind_if_owed(&mut rt).await, Some((1, 0)));
    assert!(bound(&wm, &rt, body, &element).await);

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("clear-reopen.onecad");
    rt.save(&path, save_meta()).unwrap();
    let saved = ContainerReader::open(&path).unwrap();
    let document = &saved.outcome.document;
    assert_eq!(document.elements.len(), 1);
    assert_eq!(
        document.bodies.get(body).unwrap().face_colors.get(&element),
        Some(&RED)
    );
    wm.shutdown().await;

    let (cold, _) = fresh(&bin).await;
    let mut reopened = reopen(&path, &cold);
    assert!(replay(&mut reopened).await.published());
    assert_eq!(rebind_if_owed(&mut reopened).await, Some((1, 0)));
    assert!(bound(&cold, &reopened, body, &element).await);
    let mass = cold
        .query_mass_properties(body, body.to_string())
        .await
        .unwrap();
    assert!((mass.volume - 20_000.0).abs() < 20_000.0 * 1e-9, "40×20×25");
    cold.shutdown().await;
}

/// NEW-02: an appearance-only id is not an operation input, so only the
/// rebind pass can restore it after a full replay replaced the partition.
#[tokio::test]
async fn rebind_is_owed_again_after_a_same_epoch_full_replay() {
    let Some(bin) = real_worker() else { return };
    let (wm, mut rt) = fresh(&bin).await;
    let body = build_and_regen(&mut rt).await;
    let promoted = rt
        .promote_selection(rt.head_snapshot_id().unwrap(), body, face_pick())
        .await
        .unwrap();
    let element = ElementId::new(promoted[0].element_id.clone());
    rt.apply(EditCommand::SetFaceColor {
        body,
        element_id: element.clone(),
        color: Some(RED),
    })
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rebind-lifecycle.onecad");
    rt.save(&path, save_meta()).unwrap();
    wm.shutdown().await;

    let (cold, _) = fresh(&bin).await;
    let mut reopened = reopen(&path, &cold);
    assert!(replay(&mut reopened).await.published());
    assert_eq!(rebind_if_owed(&mut reopened).await, Some((1, 0)));
    assert!(bound(&cold, &reopened, body, &element).await);
    let epoch = cold.current_epoch();

    for round in 0..2 {
        assert!(replay(&mut reopened).await.published());
        assert_eq!(cold.current_epoch(), epoch, "no worker restart");
        assert_eq!(
            rebind_if_owed(&mut reopened).await,
            Some((1, 0)),
            "replay {round} owes and completes the rebind"
        );
        assert!(bound(&cold, &reopened, body, &element).await);
    }
    assert_eq!(reopened.persisted_element_count(), 1);
    cold.shutdown().await;
}

/// Audit 11 / SR-07: the worker accepted a 50 mm box while Rust still
/// publishes the 25 mm head. The transport re-issues the stale fetch at the
/// fresh head; those bytes must not be cached under the old head.
#[tokio::test]
async fn mesh_refetched_at_a_newer_head_is_not_adopted_under_the_old_one() {
    let Some(bin) = real_worker() else { return };
    let (wm, mut rt) = fresh(&bin).await;
    let body = build_and_regen(&mut rt).await;
    let old_snapshot = rt.head_snapshot_id().unwrap();
    set_extrude_depth(&mut rt, 50.0);
    let ticket = rt.prepare_mesh_fetch(body, Lod::Coarse, None);
    let plan = rt.begin_regen(RegenRequest::ToEnd { from: 1 }).unwrap();
    let driven = plan.drive(CancelToken::new()).await;
    assert_eq!(rt.head_snapshot_id(), Some(old_snapshot));
    let Err(fetched) = ticket.drive().await else {
        panic!("expected an uncached worker fetch");
    };
    assert!(
        rt.adopt_mesh_fetch(fetched).is_none(),
        "bytes of the 50 mm head are refused under the 25 mm head"
    );
    assert!(rt.finish_regen(driven).published());
    let current = rt.get_mesh(body, Lod::Coarse, None).await.unwrap();
    assert!((mesh_z_span(&current) - 50.0).abs() < 1e-5);
    wm.shutdown().await;
}

/// Same root cause as SR-05, found while fixing it: a checkpoint-accelerated
/// regen seeds its element base from the checkpoint artifacts, which hold no
/// durable evidence, so an appearance-only identity was erased by the next
/// edit after any checkpoint.
#[tokio::test]
async fn checkpoint_accelerated_regen_keeps_durable_element_evidence() {
    let Some(bin) = real_worker() else { return };
    let (wm, mut rt) = fresh(&bin).await;
    let body = build_and_regen(&mut rt).await;
    let promoted = rt
        .promote_selection(rt.head_snapshot_id().unwrap(), body, face_pick())
        .await
        .unwrap();
    let element = ElementId::new(promoted[0].element_id.clone());
    rt.apply(EditCommand::SetFaceColor {
        body,
        element_id: element.clone(),
        color: Some(RED),
    })
    .unwrap();
    rt.take_checkpoint_at_head().await;

    let second = SketchId(Uuid::from_u128(0xB));
    for record in [
        sketch_record(0xB00, &rect_sketch(second, 0x2000, 50.0, 0.0, 20.0, 20.0)),
        extrude_record(0xB01, second, 5.0),
    ] {
        rt.apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .unwrap();
    }
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 2 }, CancelToken::new())
        .await;
    assert!(report.published());
    assert!(
        rt.last_regen_used_checkpoint(),
        "the fixture must exercise the checkpoint base, not a replay from zero"
    );
    assert_eq!(rt.head_body_ids().len(), 2);
    assert_eq!(
        rt.persisted_element_count(),
        1,
        "evidence survives the base"
    );
    assert_eq!(rebind_if_owed(&mut rt).await, Some((1, 0)));
    assert!(bound(&wm, &rt, body, &element).await);
    wm.shutdown().await;
}
