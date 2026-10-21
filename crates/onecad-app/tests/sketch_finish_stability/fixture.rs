use super::*;
use onecad_core::{
    document::{
        record::{
            BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
        },
        refs::SketchRegionRef,
        variables::Scalar,
    },
    ids::{ConstraintId, EntityId, RecordId, RegionId},
    math::{Vec2, Vec3},
    sketch::{plane_from_point_normal, Constraint, CurvePosition, SketchEntity, WorldPlane},
};

pub fn id() -> SketchId {
    SketchId(Uuid::from_u128(0x307))
}

fn entity(value: u128) -> EntityId {
    EntityId(Uuid::from_u128(0x307000 + value))
}

pub fn radius_id() -> ConstraintId {
    ConstraintId(Uuid::from_u128(0x307901))
}

pub fn frames() -> Vec<(&'static str, SketchPlane)> {
    vec![
        ("xy", SketchPlane::xy()),
        ("xz", SketchPlane::xz()),
        ("yz", SketchPlane::yz()),
        (
            "translated-oblique",
            plane_from_point_normal(
                Vec3::new_unchecked(31.0, -17.0, 43.0),
                Vec3::new_unchecked(1.0, 2.0, 3.0),
            ),
        ),
    ]
}

pub fn blank(plane: SketchPlane) -> Sketch {
    let world = if plane == SketchPlane::xz() {
        WorldPlane::XZ
    } else if plane == SketchPlane::yz() {
        WorldPlane::YZ
    } else {
        WorldPlane::XY
    };
    let mut sketch = Sketch::on_world_plane(id(), "Reconstructed Finish regression", world);
    // This fixture isolates a stored frame, not planar-face admission or re-seating.
    sketch.plane = plane;
    sketch
}

fn point(sketch: &mut Sketch, number: u128, at: [f64; 2]) {
    let at = Vec2::new(at[0], at[1]).unwrap();
    sketch
        .add_entity(SketchEntity::point(entity(number), at, false, false))
        .unwrap();
    sketch
        .add_constraint(Constraint::Fixed {
            id: ConstraintId(Uuid::from_u128(0x307800 + number)),
            point: entity(number),
            point_position: CurvePosition::Arbitrary,
            at,
        })
        .unwrap();
}

fn polygon(sketch: &mut Sketch, base: u128, vertices: &[[f64; 2]]) {
    for (index, &at) in vertices.iter().enumerate() {
        point(sketch, base + index as u128, at);
    }
    for index in 0..vertices.len() {
        sketch
            .add_entity(SketchEntity::line(
                entity(base + 20 + index as u128),
                entity(base + index as u128),
                entity(base + ((index + 1) % vertices.len()) as u128),
                false,
            ))
            .unwrap();
    }
}

pub fn profile(plane: SketchPlane) -> Sketch {
    let mut sketch = blank(plane);
    polygon(
        &mut sketch,
        0,
        &[[0., 0.], [40., 0.], [40., 20.], [0., 20.]],
    );
    polygon(
        &mut sketch,
        40,
        &[[60., 0.], [80., 0.], [85., 10.], [70., 25.], [55., 10.]],
    );
    for (center, circle, at, radius) in [(80, 81, [20., 10.], 5.), (90, 91, [100., 10.], 4.)] {
        point(&mut sketch, center, at);
        sketch
            .add_entity(
                SketchEntity::circle(entity(circle), entity(center), radius, false).unwrap(),
            )
            .unwrap();
        sketch
            .add_constraint(Constraint::Radius {
                id: if circle == 81 {
                    radius_id()
                } else {
                    ConstraintId(Uuid::from_u128(0x307902))
                },
                entity: entity(circle),
                value: Scalar::with_expr(radius, format!("{radius}mm")),
            })
            .unwrap();
    }
    sketch
}

pub fn operations(sketch: &Sketch) -> Vec<SketchEditOp> {
    sketch
        .entities()
        .iter()
        .cloned()
        .map(|entity| SketchEditOp::AddEntity { entity })
        .chain(
            sketch
                .constraints()
                .iter()
                .cloned()
                .map(|constraint| SketchEditOp::AddConstraint { constraint }),
        )
        .collect()
}

pub fn extrude(region: &SketchRegionDto, version: u32, index: usize) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(0x307a00 + index as u128)),
        0,
        "Finish regression cell",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch: id(),
                region: RegionId::new(&region.region_id),
                region_identity_version: Some(version),
                region_anchor: None,
                extra: Default::default(),
            }),
            distance: Scalar::new(6.0),
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

pub fn expected(region: &SketchRegionDto, inner_radius: f64) -> (f64, u32) {
    let pi = std::f64::consts::PI;
    if !region.holes.is_empty() {
        (800.0 - pi * inner_radius.powi(2), 7)
    } else if region.outer_loop == [entity(81).to_string()] {
        (pi * inner_radius.powi(2), 3)
    } else if region.outer_loop == [entity(91).to_string()] {
        (pi * 16.0, 3)
    } else {
        assert_eq!(region.outer_loop.len(), 5);
        (475.0, 7)
    }
}
