//! Typed FeaturePattern records through DocumentRuntime and the real OCCT worker.

mod feature_pattern_integration {
    mod host_chain;
    mod independent;
    pub(super) mod persistence;
    mod revolve;
    mod shared_host;
}

use feature_pattern_integration::persistence::{document_json, open_runtime, save_meta};

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use onecad_core::document::body::split_child_uuid;
use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, FeaturePatternLayout, FeaturePatternParams,
    FilletParams, KnownOperation, Operation, OperationRecord, PlaneKind, RevolveParams,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, AxisRef, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{
    BodyId, ElementId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};
use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::dto::FeatureStatus;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};
use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};
use uuid::Uuid;

const SKETCH: u128 = 0xF100;
const EXTRUDE: u128 = 0xF101;
const FILLET: u128 = 0xF102;
const FILLET_2: u128 = 0xF103;
const PATTERN: u128 = 0xF104;
const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;

fn rid(value: u128) -> RecordId {
    RecordId(Uuid::from_u128(value))
}

fn body_of(value: u128) -> BodyId {
    BodyId(Uuid::from_u128(value))
}

fn real_worker() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&value);
        assert!(path.is_file(), "ONECAD_WORKER_PATH={value:?} is invalid");
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert_ne!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref(),
        Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker resolved"
    );
    None
}

async fn spawn_worker(path: PathBuf) -> WorkerManager {
    let worker = WorkerManager::spawn(SupervisorConfig::production(path));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    worker
}

fn runtime(worker: &WorkerManager) -> DocumentRuntime {
    let geometry: Arc<dyn GeometryEngine> = Arc::new(worker.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(worker.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(worker.clone());
    DocumentRuntime::new_blank(geometry, meshes, solver)
}

fn add(rt: &mut DocumentRuntime, record: OperationRecord) {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("add operation");
}

async fn regen(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

async fn regen_from(rt: &mut DocumentRuntime, from: usize) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from }, CancelToken::new())
        .await
}

async fn clean_regen(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(
        RegenRequest::RevertToEnd { from: 0 },
        CancelToken::new(),
    )
    .await
}

fn published<'a>(report: &'a RegenReport, label: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(snapshot) => snapshot,
        outcome => panic!("{label}: expected Published, got {outcome:?}"),
    }
}

fn assert_clean(rt: &DocumentRuntime, report: &RegenReport, records: &[RecordId]) {
    assert!(
        report.needs_repair.is_empty(),
        "repairs: {:?}",
        report.needs_repair
    );
    assert!(
        report.failed_steps.is_empty(),
        "failures: {:?}",
        report.failed_steps
    );
    for record in records {
        let row = rt
            .projection()
            .features
            .into_iter()
            .find(|feature| feature.id == record.to_string())
            .unwrap_or_else(|| panic!("missing feature row {record}"));
        assert_eq!(
            row.status,
            FeatureStatus::Ok,
            "{}: {:?}",
            row.op_type,
            row.diagnostics
        );
    }
}

fn row_status(rt: &DocumentRuntime, record: RecordId) -> FeatureStatus {
    rt.projection()
        .features
        .into_iter()
        .find(|feature| feature.id == record.to_string())
        .unwrap_or_else(|| panic!("missing feature row {record}"))
        .status
}

fn rectangle(id: SketchId) -> Sketch {
    rectangle_sized(id, 20.0, 12.0)
}

fn rectangle_sized(id: SketchId, width: f64, height: f64) -> Sketch {
    let mut sketch = Sketch::on_world_plane(id, "Pattern profile", WorldPlane::XY);
    let points = [
        (0x10, 0.0, 0.0),
        (0x11, width, 0.0),
        (0x12, width, height),
        (0x13, 0.0, height),
    ];
    for (id, x, y) in points {
        sketch
            .add_entity(SketchEntity::point(
                EntityId(Uuid::from_u128(id)),
                Vec2::new_unchecked(x, y),
                false,
                false,
            ))
            .unwrap();
    }
    for (index, (start, end)) in [(0x10, 0x11), (0x11, 0x12), (0x12, 0x13), (0x13, 0x10)]
        .into_iter()
        .enumerate()
    {
        sketch
            .add_entity(SketchEntity::line(
                EntityId(Uuid::from_u128(0x20 + index as u128)),
                EntityId(Uuid::from_u128(start)),
                EntityId(Uuid::from_u128(end)),
                false,
            ))
            .unwrap();
    }
    for (id, x, y) in [(0x14, -8.0, -4.0), (0x15, -8.0, 16.0)] {
        sketch
            .add_entity(SketchEntity::point(
                EntityId(Uuid::from_u128(id)),
                Vec2::new_unchecked(x, y),
                false,
                true,
            ))
            .unwrap();
    }
    sketch
        .add_entity(SketchEntity::line(
            EntityId(Uuid::from_u128(0x30)),
            EntityId(Uuid::from_u128(0x14)),
            EntityId(Uuid::from_u128(0x15)),
            true,
        ))
        .unwrap();
    sketch
}

fn asymmetric_triangle(id: SketchId) -> Sketch {
    let mut sketch = Sketch::on_world_plane(id, "Pattern triangle", WorldPlane::XY);
    for (id, x, y) in [(0x40, 0.0, 0.0), (0x41, 17.0, 0.0), (0x42, 3.0, 11.0)] {
        sketch
            .add_entity(SketchEntity::point(
                EntityId(Uuid::from_u128(id)),
                Vec2::new_unchecked(x, y),
                false,
                false,
            ))
            .unwrap();
    }
    for (index, (start, end)) in [(0x40, 0x41), (0x41, 0x42), (0x42, 0x40)]
        .into_iter()
        .enumerate()
    {
        sketch
            .add_entity(SketchEntity::line(
                EntityId(Uuid::from_u128(0x50 + index as u128)),
                EntityId(Uuid::from_u128(start)),
                EntityId(Uuid::from_u128(end)),
                false,
            ))
            .unwrap();
    }
    sketch
}

fn sketch_record(sketch: &Sketch) -> OperationRecord {
    let (_, entities, constraints) = sketch_wire(sketch);
    OperationRecord::new(
        rid(SKETCH),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sketch.id,
            plane: SketchPlaneRef {
                kind: PlaneKind::Xy,
                origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
                x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
                y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
                normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
                extra: Default::default(),
            },
            entities: entities.as_array().cloned().unwrap_or_default(),
            constraints: constraints.as_array().cloned().unwrap_or_default(),
            host_face: None,
            extra: Default::default(),
        })),
    )
}

fn extrude_record(sketch: SketchId, distance: f64) -> OperationRecord {
    OperationRecord::new(
        rid(EXTRUDE),
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
            distance: Scalar::new(distance),
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

fn anchored_ref(body: BodyId, element: ElementId, anchor: Vec3) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element,
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: anchor,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    }
}

async fn promote_straight_edge(
    rt: &mut DocumentRuntime,
    worker: &WorkerManager,
    snapshot: SnapshotId,
    body: BodyId,
) -> (ElementId, Vec3) {
    for ordinal in 1..=24 {
        let key = format!("e:{ordinal}");
        let Some(info) = worker
            .query_element_by_topo_key(snapshot, body, &key)
            .await
            .expect("query edge")
        else {
            continue;
        };
        if info.kind != "edge" || info.curve_type != 0 {
            continue;
        }
        let anchor = Vec3::new_unchecked(info.center[0], info.center[1], info.center[2]);
        let promoted = rt
            .promote_selection(
                snapshot,
                body,
                vec![(
                    TopoKey::new(&key),
                    Some(AnchorIntent {
                        world_point: anchor,
                        surface_uv: None,
                        local_frame: None,
                        adjacency_hint: None,
                        extra: Default::default(),
                    }),
                )],
            )
            .await
            .expect("promote straight edge");
        return (ElementId::new(&promoted[0].element_id), anchor);
    }
    panic!("box has no straight edge")
}

async fn promote_straight_edge_above(
    rt: &mut DocumentRuntime,
    worker: &WorkerManager,
    snapshot: SnapshotId,
    body: BodyId,
    minimum_z: f64,
) -> (ElementId, Vec3) {
    for ordinal in 1..=48 {
        let key = format!("e:{ordinal}");
        let Some(info) = worker
            .query_element_by_topo_key(snapshot, body, &key)
            .await
            .expect("query edge")
        else {
            continue;
        };
        if info.kind != "edge"
            || info.curve_type != 0
            || info.center[2] <= minimum_z
            || info.magnitude <= 6.0
        {
            continue;
        }
        let anchor = Vec3::new_unchecked(info.center[0], info.center[1], info.center[2]);
        let promoted = rt
            .promote_selection(
                snapshot,
                body,
                vec![(
                    TopoKey::new(&key),
                    Some(AnchorIntent {
                        world_point: anchor,
                        surface_uv: None,
                        local_frame: None,
                        adjacency_hint: None,
                        extra: Default::default(),
                    }),
                )],
            )
            .await
            .expect("promote produced straight edge");
        return (ElementId::new(&promoted[0].element_id), anchor);
    }
    panic!("body has no produced straight edge above {minimum_z}")
}

fn fillet_record(
    record: u128,
    body: BodyId,
    edge: ElementId,
    anchor: Vec3,
    radius: f64,
) -> OperationRecord {
    OperationRecord::new(
        rid(record),
        0,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(radius),
            edge_ids: vec![edge.clone()],
            edges: vec![anchored_ref(body, edge, anchor)],
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    )
}

fn pattern_record() -> OperationRecord {
    OperationRecord::new(
        rid(PATTERN),
        0,
        "FeaturePattern",
        Operation::Known(KnownOperation::FeaturePattern(FeaturePatternParams {
            source_record_ids: vec![rid(SKETCH), rid(EXTRUDE), rid(FILLET), rid(FILLET_2)],
            layout: FeaturePatternLayout::Linear {
                direction: Vec3::new_unchecked(1.0, 0.0, 0.0),
                spacing: Scalar::new(40.0),
            },
            count: 3,
            semantics_version: 1,
            extra: Default::default(),
        })),
    )
}

fn vertex(blob: &[u8], base: usize, index: usize) -> [f64; 3] {
    let offset = base + index * 12;
    [
        f32_le(blob, offset) as f64,
        f32_le(blob, offset + 4) as f64,
        f32_le(blob, offset + 8) as f64,
    ]
}

fn mesh_volume(view: &MeshHeaderView, blob: &[u8]) -> f64 {
    let positions = view.section(SEC_POSITIONS).expect("positions");
    let indices = view.section(SEC_INDICES).expect("indices");
    let mut volume6 = 0.0;
    for triangle in 0..view.triangle_count as usize {
        let offset = indices.offset as usize + triangle * 12;
        let a = vertex(
            blob,
            positions.offset as usize,
            u32_le(blob, offset) as usize,
        );
        let b = vertex(
            blob,
            positions.offset as usize,
            u32_le(blob, offset + 4) as usize,
        );
        let c = vertex(
            blob,
            positions.offset as usize,
            u32_le(blob, offset + 8) as usize,
        );
        volume6 += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    (volume6 / 6.0).abs()
}

async fn metrics(rt: &mut DocumentRuntime, body: BodyId) -> (f64, [f32; 3], [f32; 3], u32) {
    let blob = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .unwrap_or_else(|| panic!("mesh {body} unavailable"));
    let view = validate_mesh_blob(&blob).expect("valid MESH1");
    (
        mesh_volume(&view, &blob),
        view.bbox_min,
        view.bbox_max,
        view.face_count,
    )
}
