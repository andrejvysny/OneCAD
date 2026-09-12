use super::super::*;

use onecad_core::document::record::{ChamferParams, ChamferReferenceFace, HoleParams, HoleType};

const HOLE: u128 = 0xF201;
const CHAMFER: u128 = 0xF202;
const HOST_PATTERN: u128 = 0xF203;

fn element_ref(body: BodyId, element: ElementId, kind: ElementKind, at: Vec3) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element,
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

async fn promote_kind(
    rt: &mut DocumentRuntime,
    worker: &WorkerManager,
    snapshot: SnapshotId,
    body: BodyId,
    kind: &str,
    predicate: impl Fn(&onecad_lib::dto::ElementInfoDto) -> bool,
) -> (ElementId, Vec3) {
    let prefix = if kind == "face" { 'f' } else { 'e' };
    for ordinal in 1..=48 {
        let key = format!("{prefix}:{ordinal}");
        let Some(info) = worker
            .query_element_by_topo_key(snapshot, body, &key)
            .await
            .expect("query topology")
        else {
            continue;
        };
        if info.kind != kind || !predicate(&info) {
            continue;
        }
        let at = Vec3::new_unchecked(info.center[0], info.center[1], info.center[2]);
        let result = rt
            .promote_selection(
                snapshot,
                body,
                vec![(
                    TopoKey::new(&key),
                    Some(AnchorIntent {
                        world_point: at,
                        surface_uv: None,
                        local_frame: None,
                        adjacency_hint: None,
                        extra: Default::default(),
                    }),
                )],
            )
            .await
            .expect("promote topology");
        return (ElementId::new(&result[0].element_id), at);
    }
    panic!("no matching {kind}")
}

fn hole_record(body: BodyId, face: ElementId, face_at: Vec3, point: Vec3) -> OperationRecord {
    OperationRecord::new(
        rid(HOLE),
        0,
        "Hole",
        Operation::Known(KnownOperation::Hole(HoleParams {
            target_body: body,
            face: element_ref(body, face, ElementKind::Face, face_at),
            point,
            hole_type: HoleType::Simple,
            diameter: Scalar::new(3.0),
            depth: None,
            cb_diameter: None,
            cb_depth: None,
            cs_diameter: None,
            cs_angle_deg: None,
            thread: None,
            result_policy_version: Some(2),
            extra: Default::default(),
        })),
    )
}

fn chamfer_record(
    body: BodyId,
    edge: ElementId,
    edge_at: Vec3,
    face: ElementId,
    face_at: Vec3,
) -> OperationRecord {
    OperationRecord::new(
        rid(CHAMFER),
        0,
        "Chamfer",
        Operation::Known(KnownOperation::Chamfer(ChamferParams {
            radius: Scalar::new(0.5),
            distance2: Some(Scalar::new(0.4)),
            angle_deg: None,
            edge_ids: vec![edge.clone()],
            edges: vec![element_ref(body, edge.clone(), ElementKind::Edge, edge_at)],
            reference_faces: vec![ChamferReferenceFace {
                edge_id: edge,
                face_id: face.clone(),
            }],
            reference_face_refs: vec![element_ref(body, face, ElementKind::Face, face_at)],
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hole_chamfer_pattern_modifies_one_host_without_children() {
    let Some(binary) = real_worker() else {
        eprintln!("skip: no real worker");
        return;
    };
    let worker = spawn_worker(binary).await;
    let mut rt = runtime(&worker);
    let sketch_id = SketchId(Uuid::from_u128(SKETCH));
    let sketch = rectangle(sketch_id);
    add(&mut rt, sketch_record(&sketch));
    rt.apply(EditCommand::AddSketch { sketch }).unwrap();
    add(&mut rt, extrude_record(sketch_id, 10.0));
    let stock = regen(&mut rt).await;
    published(&stock, "stock");
    assert_clean(&rt, &stock, &[rid(SKETCH), rid(EXTRUDE)]);
    let host = body_of(EXTRUDE);
    let (top, top_at) = promote_kind(
        &mut rt,
        &worker,
        SnapshotId(stock.snapshot_id),
        host,
        "face",
        |info| info.surface_type == 0 && (info.center[2] - 10.0).abs() < 0.1,
    )
    .await;
    let hole_at = Vec3::new_unchecked(top_at.x + 3.0, top_at.y, top_at.z);
    add(&mut rt, hole_record(host, top, top_at, hole_at));
    let holed = regen(&mut rt).await;
    published(&holed, "source hole");
    assert_clean(&rt, &holed, &[rid(SKETCH), rid(EXTRUDE), rid(HOLE)]);
    let hole_snapshot = SnapshotId(holed.snapshot_id);
    let (rim, rim_at) = promote_kind(&mut rt, &worker, hole_snapshot, host, "edge", |info| {
        info.curve_type == 1
            && (info.center[0] - hole_at.x).abs() < 0.1
            && (info.center[1] - hole_at.y).abs() < 0.1
            && (info.center[2] - hole_at.z).abs() < 0.1
    })
    .await;
    let (wall, wall_at) = promote_kind(&mut rt, &worker, hole_snapshot, host, "face", |info| {
        info.surface_type == 1
            && (info.center[0] - hole_at.x).abs() < 0.1
            && (info.center[1] - hole_at.y).abs() < 0.1
    })
    .await;
    add(&mut rt, chamfer_record(host, rim, rim_at, wall, wall_at));
    let source = regen(&mut rt).await;
    let source_snapshot = published(&source, "source chamfer");
    assert_clean(
        &rt,
        &source,
        &[rid(SKETCH), rid(EXTRUDE), rid(HOLE), rid(CHAMFER)],
    );
    assert_eq!(source_snapshot.bodies.len(), 1);
    let (source_volume, _, _, source_faces) = metrics(&mut rt, host).await;

    add(
        &mut rt,
        OperationRecord::new(
            rid(HOST_PATTERN),
            0,
            "FeaturePattern",
            Operation::Known(KnownOperation::FeaturePattern(FeaturePatternParams {
                source_record_ids: vec![rid(HOLE), rid(CHAMFER)],
                layout: FeaturePatternLayout::Circular {
                    axis_origin: Vec3::new_unchecked(top_at.x, top_at.y, 0.0),
                    axis_direction: Vec3::new_unchecked(0.0, 0.0, 1.0),
                    angle_deg: Scalar::new(360.0),
                },
                count: 4,
                semantics_version: 1,
                extra: Default::default(),
            })),
        ),
    );
    let patterned = regen(&mut rt).await;
    let snapshot = published(&patterned, "host feature pattern");
    assert_clean(
        &rt,
        &patterned,
        &[
            rid(SKETCH),
            rid(EXTRUDE),
            rid(HOLE),
            rid(CHAMFER),
            rid(HOST_PATTERN),
        ],
    );
    assert_eq!(snapshot.bodies.len(), 1);
    assert_eq!(snapshot.bodies[0].body, host);
    assert!(rt
        .body_meta(BodyId(split_child_uuid(rid(HOST_PATTERN).0, 0)))
        .is_none());
    let (pattern_volume, _, _, pattern_faces) = metrics(&mut rt, host).await;
    assert!(pattern_volume < source_volume - 50.0);
    assert!(
        pattern_faces > source_faces,
        "pattern adds hole/chamfer topology"
    );
    worker.shutdown().await;
}
