use onecad_core::{
    document::variables::Scalar,
    ids::{ConstraintId, EntityId, SketchId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchAttachment, SketchEntity, WorldPlane},
};
use uuid::Uuid;

pub fn entity(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}
fn constraint(n: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(n))
}
fn xy(x: f64, y: f64) -> Vec2 {
    Vec2::new_unchecked(x, y)
}

pub fn sketch(control: bool) -> Sketch {
    let mut sketch = Sketch::new(
        SketchId(Uuid::from_u128(0x305)),
        "ONECAD-305",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    sketch
        .add_entity(SketchEntity::point(entity(1000), xy(0., 0.), false, false))
        .unwrap();
    sketch
        .add_entity(
            SketchEntity::arc(
                entity(1001),
                entity(1000),
                10.,
                0.,
                std::f64::consts::FRAC_PI_2,
                false,
            )
            .unwrap(),
        )
        .unwrap();
    if control {
        add_control(&mut sketch);
    }
    sketch
}

pub fn radius(sketch: &mut Sketch) {
    sketch
        .add_constraint(Constraint::Radius {
            id: constraint(1008),
            entity: entity(1001),
            value: Scalar::new(10.),
        })
        .unwrap();
}

fn add_control(sketch: &mut Sketch) {
    for (id, at) in [(1002, xy(10., 0.)), (1003, xy(0., 10.))] {
        sketch
            .add_entity(SketchEntity::point(entity(id), at, true, false))
            .unwrap();
    }
    for (id, end) in [(1004, 1002), (1005, 1003)] {
        sketch
            .add_entity(SketchEntity::line(
                entity(id),
                entity(1000),
                entity(end),
                true,
            ))
            .unwrap();
    }
    sketch
        .add_constraint(Constraint::Fixed {
            id: constraint(1010),
            point: entity(1000),
            point_position: CurvePosition::Arbitrary,
            at: xy(0., 0.),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Horizontal {
            id: constraint(1011),
            line: entity(1004),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Vertical {
            id: constraint(1012),
            line: entity(1005),
        })
        .unwrap();
    for (id, point, role) in [
        (1013, 1002, CurvePosition::Start),
        (1014, 1003, CurvePosition::End),
    ] {
        sketch
            .add_constraint(Constraint::Coincident {
                id: constraint(id),
                point1: entity(point),
                point2: entity(1001),
                point1_position: CurvePosition::Arbitrary,
                point2_position: role,
            })
            .unwrap();
    }
}
