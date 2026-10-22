//! The representative modeling round shared by the in-process tests and the
//! sidecar comparison: a fully constrained 40 × 20 rectangle on XY, solved on
//! the PlaneGCS lane, extruded 25 mm through the app's own `DocumentRuntime`,
//! tessellated (MESH1) and measured (exact volume 20 000 mm³). Same records
//! as `tests/m2_gate.rs`.

use std::sync::Arc;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};
use onecad_protocol::mesh::validate_mesh_blob;

use crate::document_runtime::DocumentRuntime;
use crate::worker::wire::sketch_wire;
use crate::worker::{ElementQuery, MeshProvider, SolverEngine, WorkerManager};

pub(super) const RECT_W: f64 = 40.0;
pub(super) const RECT_H: f64 = 20.0;
pub(super) const DEPTH: f64 = 25.0;
pub(super) const PLATE_VOLUME: f64 = RECT_W * RECT_H * DEPTH;

/// What one round observed — compared across backends.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PlateOutcome {
    pub dof: u32,
    pub regions: usize,
    pub faces: u32,
    pub triangles: u32,
    pub mesh_bytes: usize,
    pub bbox_dims: [f32; 3],
    pub volume: f64,
}

pub(super) async fn plate_round(wm: &WorkerManager) -> PlateOutcome {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    let sketch = rectangle();
    let sid = sketch.id;
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Plate", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    let solved = rt
        .sketch_upsert(sid, edit_ops(&sketch))
        .await
        .expect("sketch_upsert");
    let finished = rt.finish_sketch(sid).await.expect("finish_sketch");
    let region = finished
        .regions
        .first()
        .expect("a closed region")
        .region_id
        .clone();
    for record in [
        sketch_record(&sketch),
        extrude_record(sid, RegionId::new(region), finished.region_identity_version),
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
    assert_eq!(
        report.changed.len(),
        1,
        "one extruded body: {:?}",
        report.outcome
    );
    let body = report.changed[0].0;
    let mesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch the plate mesh");
    let view = validate_mesh_blob(&mesh).expect("MESH1 validates");
    let mass = wm
        .query_mass_properties(body, crate::worker::wire::body_id_wire(body))
        .await
        .expect("QueryMassProperties");
    let dims = std::array::from_fn(|i| view.bbox_max[i] - view.bbox_min[i]);
    PlateOutcome {
        dof: solved.dof,
        regions: finished.regions.len(),
        faces: view.face_count,
        triangles: view.triangle_count,
        mesh_bytes: mesh.len(),
        bbox_dims: dims,
        volume: mass.volume,
    }
}

/// The exact analytic plate, within OCCT's GProp precision.
pub(super) fn assert_plate(outcome: &PlateOutcome) {
    assert_eq!(outcome.dof, 0, "fully constrained rectangle: {outcome:?}");
    assert_eq!(outcome.regions, 1, "{outcome:?}");
    assert_eq!(outcome.faces, 6, "a box has six faces: {outcome:?}");
    assert!(
        outcome.triangles >= 12 && outcome.mesh_bytes > 0,
        "nonempty MESH1: {outcome:?}"
    );
    assert!(
        (outcome.volume - PLATE_VOLUME).abs() <= PLATE_VOLUME * 1e-9,
        "plate volume {} != {PLATE_VOLUME}",
        outcome.volume
    );
}

fn entity(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}

fn rectangle() -> Sketch {
    let c = |n: u128| ConstraintId(Uuid::from_u128(n));
    let mut sk = Sketch::on_world_plane(SketchId(Uuid::from_u128(0x5c)), "Plate", WorldPlane::XY);
    let corners = [(0.0, 0.0), (RECT_W, 0.0), (RECT_W, RECT_H), (0.0, RECT_H)];
    for side in 0..4u128 {
        let (start, end) = (corners[side as usize], corners[(side as usize + 1) % 4]);
        let (ps, pe) = (entity(0x100 + 0x10 * side), entity(0x101 + 0x10 * side));
        for (id, (x, y)) in [(ps, start), (pe, end)] {
            sk.add_entity(SketchEntity::point(
                id,
                Vec2::new_unchecked(x, y),
                false,
                false,
            ))
            .unwrap();
        }
        sk.add_entity(SketchEntity::line(entity(0x200 + side), ps, pe, false))
            .unwrap();
    }
    for side in 0..4u128 {
        let next = (side + 1) % 4;
        sk.add_constraint(Constraint::Coincident {
            id: c(1 + side),
            point1: entity(0x101 + 0x10 * side),
            point2: entity(0x100 + 0x10 * next),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    }
    sk.add_constraint(Constraint::Horizontal {
        id: c(5),
        line: entity(0x200),
    })
    .unwrap();
    sk.add_constraint(Constraint::Horizontal {
        id: c(6),
        line: entity(0x202),
    })
    .unwrap();
    sk.add_constraint(Constraint::Vertical {
        id: c(7),
        line: entity(0x201),
    })
    .unwrap();
    sk.add_constraint(Constraint::Vertical {
        id: c(8),
        line: entity(0x203),
    })
    .unwrap();
    pin_and_dimension(&mut sk);
    sk
}

/// One fixed corner + width + height: dof 0.
fn pin_and_dimension(sk: &mut Sketch) {
    let c = |n: u128| ConstraintId(Uuid::from_u128(n));
    let at = CurvePosition::Arbitrary;
    sk.add_constraint(Constraint::Fixed {
        id: c(9),
        point: entity(0x100),
        point_position: at,
        at: Vec2::new_unchecked(0.0, 0.0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: entity(0x100),
        point1_position: at,
        point2: entity(0x101),
        point2_position: at,
        value: Scalar::new(RECT_W),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: entity(0x110),
        point1_position: at,
        point2: entity(0x111),
        point2_position: at,
        value: Scalar::new(RECT_H),
    })
    .unwrap();
}

fn edit_ops(sk: &Sketch) -> Vec<SketchEditOp> {
    let entities = sk
        .entities()
        .iter()
        .map(|e| SketchEditOp::AddEntity { entity: e.clone() });
    let constraints = sk
        .constraints()
        .iter()
        .map(|c| SketchEditOp::AddConstraint {
            constraint: c.clone(),
        });
    entities.chain(constraints).collect()
}

fn sketch_record(sk: &Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    let params = SketchOpParams {
        sketch: sk.id,
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
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(0x5c00)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

fn extrude_record(sketch: SketchId, region: RegionId, version: u32) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region,
            region_identity_version: Some(version),
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(DEPTH),
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
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(0xe0)),
        1,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}
