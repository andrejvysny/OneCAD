//! Per-hole identity: a downstream ref names ONE hole of a multi-point Hole by
//! its point id (frozen as `intent.descriptor.originSlot` at promotion), so a
//! point edit can move, keep or orphan it — never hand it to a sibling.

use std::f64::consts::SQRT_2;

use uuid::Uuid;

use onecad_core::document::record::{
    ChamferParams, HolePlacement, HolePointId, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::refs::ElementKind;
use onecad_core::document::variables::Scalar;
use onecad_core::ids::{BodyId, ElementId, RecordId, SnapshotId, TopoKey};
use onecad_core::math::Vec3;

use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::dto::FeatureStatus;
use onecad_app::worker::WorkerManager;

use crate::*;

const CHAMFER_LEG: f64 = 0.5;
const RIM: f64 = 2.0;

fn chamfer_record(body: BodyId, edge: &ElementId, at: Vec3) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Chamfer(ChamferParams {
        radius: Scalar::new(CHAMFER_LEG),
        distance2: None,
        angle_deg: None,
        edge_ids: vec![edge.clone()],
        edges: vec![anchored_ref(body, edge, ElementKind::Edge, at)],
        reference_faces: Vec::new(),
        reference_face_refs: Vec::new(),
        chain_tangent_edges: false,
        tangent_closure_version: None,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(CHAMFER)), 0, "Chamfer", op)
}

/// Which top rims (by hole centre y) carry the chamfer, and which are plain.
async fn chamfered_rims(wm: &WorkerManager, plate: BodyId, ys: &[f64]) -> Vec<bool> {
    let found = circles(wm, plate).await;
    ys.iter()
        .map(|y| {
            let at = top(-10.0, *y);
            has_circle(&found, at, RIM + CHAMFER_LEG) && !has_circle(&found, at, RIM)
        })
        .collect()
}

/// Rewrites the Hole's placements (`id`, world y) and replays from the Hole step.
async fn place(rt: &mut DocumentRuntime, list: &[(HolePointId, f64)]) -> usize {
    let mut params = stored_hole(rt, HOLE_A);
    let placements = list
        .iter()
        .map(|(id, y)| HolePlacement {
            id: Some(*id),
            at: top(-10.0, *y),
        })
        .collect();
    params.set_placements(placements).expect("canonical writer");
    edit_hole(rt, HOLE_A, params);
    // Steps: 0 sketch, 1 extrude, 2 hole, 3 chamfer — the edit lands on 2.
    let report = regen_from(rt, 2).await;
    published(&report, "hole point edit")
        .repair_summary
        .needs_repair_count
}

/// Three Ø4 holes at y = 10/30/50 on plate A plus a Chamfer on hole 2's top
/// rim, promoted exactly the way a viewport pick does. Returns the plate and
/// the hole point ids.
async fn seed_rim_chamfer(
    rt: &mut DocumentRuntime,
    wm: &WorkerManager,
) -> (BodyId, Vec<HolePointId>) {
    let plate = build_plate(rt, SKETCH_A, EXTRUDE_A, 0.0).await;
    let params = holes(
        plate,
        top(-10.0, 30.0),
        &[top(-10.0, 10.0), top(-10.0, 30.0), top(-10.0, 50.0)],
        4.0,
    );
    let ids: Vec<HolePointId> = params.placements().iter().map(|p| p.id.unwrap()).collect();
    add_op(rt, hole_record(HOLE_A, params));
    let report = clean_regen(rt).await;
    published(&report, "three holes");

    // Promote hole 2's top rim exactly the way a viewport pick does.
    let rim_key = circles(wm, plate)
        .await
        .into_iter()
        .find(|(c, r, _)| c.approx_eq(&top(-10.0, 50.0), 1e-6) && (r - RIM).abs() < 1e-6)
        .map(|(_, _, key)| key)
        .expect("hole 2 top rim");
    let rim_at = top(-10.0 + SQRT_2, 50.0 + SQRT_2);
    let promoted = rt
        .promote_selection(
            SnapshotId(report.snapshot_id),
            plate,
            vec![(TopoKey::new(rim_key.as_str()), Some(anchor_at(rim_at)))],
        )
        .await
        .expect("promote hole 2 rim");
    let edge = ElementId::new(&promoted[0].element_id);
    add_op(rt, chamfer_record(plate, &edge, rim_at));
    let snap = clean_regen(rt).await;
    assert_eq!(
        published(&snap, "chamfer")
            .repair_summary
            .needs_repair_count,
        0
    );

    // The frozen intent names the producer record and hole 2's point id.
    assert_eq!(
        stored_origin_slot(rt),
        serde_json::json!({
            "recordId": RecordId(Uuid::from_u128(HOLE_A)).to_string(),
            "slot": ids[2].to_string(),
        })
    );
    assert_eq!(
        chamfered_rims(wm, plate, &[10.0, 30.0, 50.0]).await,
        [false, false, true]
    );

    (plate, ids)
}

/// The frozen `intent.descriptor.originSlot` of the stored Chamfer's edge ref.
fn stored_origin_slot(rt: &DocumentRuntime) -> serde_json::Value {
    let Some(Operation::Known(KnownOperation::Chamfer(chamfer))) = rt
        .operation_record(RecordId(Uuid::from_u128(CHAMFER)))
        .map(|r| r.op)
    else {
        panic!("stored chamfer");
    };
    chamfer.edges[0]
        .intent
        .as_ref()
        .expect("stamped intent")
        .descriptor["originSlot"]
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chamfer_on_one_hole_never_moves_to_a_sibling() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (plate, ids) = seed_rim_chamfer(&mut rt, &wm).await;

    // (a) move points[0]: the chamfer stays on hole 2.
    assert_eq!(
        place(&mut rt, &[(ids[0], 15.0), (ids[1], 30.0), (ids[2], 50.0)]).await,
        0
    );
    assert_eq!(
        chamfered_rims(&wm, plate, &[15.0, 30.0, 50.0]).await,
        [false, false, true]
    );
    // (b) remove points[1]: hole 2 is now index 1 and keeps the chamfer.
    assert_eq!(place(&mut rt, &[(ids[0], 15.0), (ids[2], 50.0)]).await, 0);
    assert_eq!(
        chamfered_rims(&wm, plate, &[15.0, 50.0]).await,
        [false, true]
    );
    // (d) a small move of hole 2: the chamfer follows its own hole.
    assert_eq!(place(&mut rt, &[(ids[0], 15.0), (ids[2], 47.0)]).await, 0);
    assert_eq!(
        chamfered_rims(&wm, plate, &[15.0, 47.0]).await,
        [false, true]
    );
    // (c) remove hole 2: NeedsRepair — never the surviving sibling.
    assert!(
        place(&mut rt, &[(ids[0], 15.0)]).await >= 1,
        "orphaned ref needs repair"
    );
    assert_eq!(feature_row(&rt, CHAMFER).status, FeatureStatus::NeedsRepair);
    let found = circles(&wm, plate).await;
    assert!(
        !found
            .iter()
            .any(|(_, r, _)| (r - (RIM + CHAMFER_LEG)).abs() < 1e-6),
        "no rim anywhere was chamfered: {found:?}"
    );
    assert!(
        has_circle(&found, top(-10.0, 15.0), RIM),
        "the sibling rim is untouched"
    );
    wm.shutdown().await;
}

/// Identity after save → fresh worker → reopen: the rebuilt topology ledger
/// plus `rebind_persisted_elements` (ResolveRefs with `originSlot`) must keep
/// the Chamfer on hole 2, and a later point edit must still never hand it to
/// a sibling.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slotted_chamfer_keeps_its_hole_after_reopen_on_a_fresh_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let doc = tmp.path().join("slotted-chamfer.onecad");
    let (plate, ids, slot) = {
        let wm = spawn_worker(bin.clone()).await;
        let mut rt = runtime_over(&wm);
        let (plate, ids) = seed_rim_chamfer(&mut rt, &wm).await;
        let slot = stored_origin_slot(&rt);
        rt.save(&doc, save_meta()).expect("save");
        wm.shutdown().await;
        (plate, ids, slot)
    };

    let wm = spawn_worker(bin).await;
    let mut rt = open_over(&wm, &doc);
    let snap = clean_regen(&mut rt).await;
    assert_eq!(
        published(&snap, "reopen").repair_summary.needs_repair_count,
        0
    );
    assert_eq!(feature_row(&rt, CHAMFER).status, FeatureStatus::Ok);
    assert_eq!(stored_origin_slot(&rt), slot, "originSlot frozen verbatim");
    assert_eq!(
        chamfered_rims(&wm, plate, &[10.0, 30.0, 50.0]).await,
        [false, false, true],
        "reopen: the chamfer is on hole 2"
    );
    // A point edit on the reopened document: still hole 2, never a sibling.
    assert_eq!(
        place(&mut rt, &[(ids[0], 15.0), (ids[1], 30.0), (ids[2], 50.0)]).await,
        0
    );
    assert_eq!(
        chamfered_rims(&wm, plate, &[15.0, 30.0, 50.0]).await,
        [false, false, true],
        "reopen + edit: the chamfer stays on hole 2"
    );
    assert_eq!(
        stored_origin_slot(&rt),
        slot,
        "originSlot unchanged by the edit"
    );
    wm.shutdown().await;
}
