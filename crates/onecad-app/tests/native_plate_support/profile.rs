//! Synthetic acceptance data, not approved mounting-plate product dimensions.
use super::Harness;
use onecad_app::{
    api,
    dto::{FinishSketchDto, SketchSolveStatus},
};
use onecad_core::{
    document::{
        record::{BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation},
        refs::SketchRegionRef,
        variables::Scalar,
    },
    edit::SketchEditOp,
    ids::{ConstraintId, EntityId, RecordId, RegionId, SketchId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane},
};
use uuid::Uuid;

pub const WIDTH: f64 = 80.0;
pub const HEIGHT: f64 = 50.0;
pub const FIRST_DEPTH: f64 = 6.0;
pub const EDITED_DEPTH: f64 = 8.0;
pub const SKETCH: u128 = 0x27300;
pub const EXTRUDE: u128 = 0x27301;
pub fn sketch_id() -> SketchId {
    SketchId(Uuid::from_u128(SKETCH))
}
async fn upsert_rectangle(harness: &Harness) {
    let rectangle = rectangle();
    let ops = rectangle
        .entities()
        .iter()
        .map(|entity| SketchEditOp::AddEntity {
            entity: entity.clone(),
        })
        .chain(
            rectangle
                .constraints()
                .iter()
                .map(|constraint| SketchEditOp::AddConstraint {
                    constraint: constraint.clone(),
                }),
        )
        .collect();
    let solve = api::sketch_upsert(
        &harness.state,
        harness.host.clone(),
        sketch_id().to_string(),
        ops,
    )
    .await
    .unwrap();
    assert_eq!(solve.status, SketchSolveStatus::FullyConstrained);
    assert_eq!(solve.dof, 0);
    assert!(solve.conflicting.is_empty());
}

pub fn extrude_id() -> RecordId {
    RecordId(Uuid::from_u128(EXTRUDE))
}
fn entity(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(0x273100 + n))
}
fn constraint(n: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(0x273200 + n))
}

pub fn rectangle() -> Sketch {
    let mut sketch = Sketch::on_world_plane(
        sketch_id(),
        "Synthetic constrained 80 x 50 mm plate profile",
        WorldPlane::XY,
    );
    let corners = [[0.0, 0.0], [WIDTH, 0.0], [WIDTH, HEIGHT], [0.0, HEIGHT]];
    for side in 0..4usize {
        for endpoint in 0..2usize {
            let point = corners[(side + endpoint) % 4];
            sketch
                .add_entity(SketchEntity::point(
                    entity((side * 2 + endpoint) as u128),
                    Vec2::new_unchecked(point[0], point[1]),
                    false,
                    false,
                ))
                .unwrap();
        }
        sketch
            .add_entity(SketchEntity::line(
                entity(16 + side as u128),
                entity((side * 2) as u128),
                entity((side * 2 + 1) as u128),
                false,
            ))
            .unwrap();
    }
    add_side_constraints(&mut sketch);
    add_dimensions(&mut sketch);
    sketch
}

fn add_side_constraints(sketch: &mut Sketch) {
    for side in 0..4u128 {
        sketch
            .add_constraint(Constraint::Coincident {
                id: constraint(side),
                point1: entity(side * 2 + 1),
                point2: entity(((side + 1) % 4) * 2),
                point1_position: CurvePosition::Arbitrary,
                point2_position: CurvePosition::Arbitrary,
            })
            .unwrap();
        let constraint = if side.is_multiple_of(2) {
            Constraint::Horizontal {
                id: constraint(4 + side),
                line: entity(16 + side),
            }
        } else {
            Constraint::Vertical {
                id: constraint(4 + side),
                line: entity(16 + side),
            }
        };
        sketch.add_constraint(constraint).unwrap();
    }
}

fn add_dimensions(sketch: &mut Sketch) {
    sketch
        .add_constraint(Constraint::Fixed {
            id: constraint(8),
            point: entity(0),
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new_unchecked(0.0, 0.0),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::HorizontalDistance {
            id: constraint(9),
            point1: entity(0),
            point1_position: CurvePosition::Arbitrary,
            point2: entity(1),
            point2_position: CurvePosition::Arbitrary,
            value: Scalar::new(WIDTH),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::VerticalDistance {
            id: constraint(10),
            point1: entity(2),
            point1_position: CurvePosition::Arbitrary,
            point2: entity(3),
            point2_position: CurvePosition::Arbitrary,
            value: Scalar::new(HEIGHT),
        })
        .unwrap();
}

pub async fn create(harness: &Harness) -> FinishSketchDto {
    let empty = Sketch::on_world_plane(
        sketch_id(),
        "Synthetic constrained 80 x 50 mm plate profile",
        WorldPlane::XY,
    );
    api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        onecad_core::edit::EditCommand::AddSketch { sketch: empty },
    )
    .await
    .unwrap();
    api::enter_sketch(
        &harness.state,
        harness.host.clone(),
        sketch_id().to_string(),
        Some(true),
    )
    .await
    .unwrap();
    upsert_rectangle(harness).await;
    let finish = api::finish_sketch(
        &harness.state,
        harness.host.clone(),
        sketch_id().to_string(),
    )
    .await
    .unwrap();
    assert_eq!(finish.regions.len(), 1);
    assert!(finish.regions[0].holes.is_empty());
    let fill = finish.regions[0]
        .preview_triangles
        .as_ref()
        .expect("native region selection requires supported fill geometry");
    assert!(!fill.positions.is_empty() && !fill.indices.is_empty());
    assert_eq!(fill.holes_subtracted, 0);
    let projection = harness.projection().await;
    harness.settled(&projection).await;
    finish
}

pub fn extrude(profile: &FinishSketchDto, depth: f64) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch: sketch_id(),
            region: RegionId::new(&profile.regions[0].region_id),
            region_identity_version: Some(profile.region_identity_version),
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(depth),
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
    }))
}
