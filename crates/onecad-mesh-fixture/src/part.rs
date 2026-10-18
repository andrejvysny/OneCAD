//! The parametric fixture part: constrained 40x30 rectangle on XY -> blind extrude
//! 10 -> fillet r=3 on one vertical edge. Record builders mirror
//! `crates/onecad-app/tests/step_import.rs` and `crates/onecad-app/tests/fillet_blend_class.rs`.

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::ids::{BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_app::worker::wire::sketch_wire;

pub const WIDTH: f64 = 40.0;
pub const HEIGHT: f64 = 30.0;
pub const DEPTH: f64 = 10.0;
pub const FILLET_R: f64 = 3.0;

pub const SKETCH_REC: u128 = 0xF100;
pub const EXTRUDE_REC: u128 = 0xF101;
pub const FILLET_REC: u128 = 0xF102;
const SKETCH_ID: u128 = 0xF0;

/// The extrude's NewBody id (deterministic: `BodyId(extrude record uuid)`).
pub fn body() -> BodyId {
    BodyId(Uuid::from_u128(EXTRUDE_REC))
}

fn plane_ref(sk: &Sketch) -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: sk.plane.origin,
        x_axis: sk.plane.x_axis,
        y_axis: sk.plane.y_axis,
        normal: sk.plane.normal,
        extra: Default::default(),
    }
}

fn add_point(sk: &mut Sketch, id: EntityId, x: f64, y: f64) {
    sk.add_entity(SketchEntity::point(
        id,
        Vec2::new_unchecked(x, y),
        false,
        false,
    ))
    .expect("add point");
}

/// Fully constrained rectangle with its corner fixed at the origin.
fn rect_sketch() -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(0x1000 + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(0x1040 + n));
    let (w, h) = (WIDTH, HEIGHT);
    let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
    let mut sk =
        Sketch::on_world_plane(SketchId(Uuid::from_u128(SKETCH_ID)), "Rect", WorldPlane::XY);
    for i in 0..4usize {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        add_point(&mut sk, e(2 * i as u128), a.0, a.1);
        add_point(&mut sk, e(2 * i as u128 + 1), b.0, b.1);
        sk.add_entity(SketchEntity::line(
            e(0x10 + i as u128),
            e(2 * i as u128),
            e(2 * i as u128 + 1),
            false,
        ))
        .expect("add line");
    }
    for i in 0..4u128 {
        let (p1, p2) = (e(2 * i + 1), e((2 * i + 2) % 8));
        sk.add_constraint(Constraint::Coincident {
            id: c(1 + i),
            point1: p1,
            point2: p2,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .expect("coincident");
    }
    add_shape_constraints(&mut sk, &e, &c);
    sk
}

fn add_shape_constraints(
    sk: &mut Sketch,
    e: &dyn Fn(u128) -> EntityId,
    c: &dyn Fn(u128) -> ConstraintId,
) {
    let any = CurvePosition::Arbitrary;
    let list = [
        Constraint::Horizontal {
            id: c(5),
            line: e(0x10),
        },
        Constraint::Horizontal {
            id: c(6),
            line: e(0x12),
        },
        Constraint::Vertical {
            id: c(7),
            line: e(0x11),
        },
        Constraint::Vertical {
            id: c(8),
            line: e(0x13),
        },
        Constraint::Fixed {
            id: c(9),
            point: e(0),
            point_position: any,
            at: Vec2::new_unchecked(0.0, 0.0),
        },
        Constraint::HorizontalDistance {
            id: c(10),
            point1: e(0),
            point1_position: any,
            point2: e(1),
            point2_position: any,
            value: Scalar::new(WIDTH),
        },
        Constraint::VerticalDistance {
            id: c(11),
            point1: e(2),
            point1_position: any,
            point2: e(3),
            point2_position: any,
            value: Scalar::new(HEIGHT),
        },
    ];
    for k in list {
        sk.add_constraint(k).expect("shape constraint");
    }
}

pub fn sketch_record() -> (OperationRecord, SketchId) {
    let sk = rect_sketch();
    let (_plane, entities, constraints) = sketch_wire(&sk);
    let params = SketchOpParams {
        sketch: sk.id,
        plane: plane_ref(&sk),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    };
    let rec = OperationRecord::new(
        RecordId(Uuid::from_u128(SKETCH_REC)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    );
    (rec, sk.id)
}

pub fn extrude_record(sketch: SketchId) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""),
            region_identity_version: None,
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
        RecordId(Uuid::from_u128(EXTRUDE_REC)),
        1,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

pub fn anchor(world: Vec3) -> AnchorIntent {
    AnchorIntent {
        world_point: world,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    }
}

pub fn fillet_record(edge: &ElementId, world: Vec3) -> OperationRecord {
    let edge_ref = ElementRef {
        primary: Some(PrimaryRef {
            body: body(),
            element: edge.clone(),
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(anchor(world)),
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(FILLET_REC)),
        2,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(FILLET_R),
            edge_ids: vec![edge.clone()],
            edges: vec![edge_ref],
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    )
}
