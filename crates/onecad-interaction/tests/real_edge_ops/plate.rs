//! The reference plate: 60 × 40 × 4 mm with one Ø6 mm hole cut through it.
//! Synthetic acceptance data built through the shared application services.
use super::host::Harness;
use onecad_app::{
    api,
    dto::{DocumentProjection, FinishSketchDto, SketchSolveStatus},
};
use onecad_core::{
    document::{record::*, refs::SketchRegionRef, variables::Scalar},
    edit::{EditCommand, SketchEditOp},
    ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane},
};
use std::f64::consts::PI;
use uuid::Uuid;

pub const WIDTH: f64 = 60.0;
pub const HEIGHT: f64 = 40.0;
pub const THICKNESS: f64 = 4.0;
pub const HOLE_RADIUS: f64 = 3.0;

/// 9600 − 36π mm³.
pub fn volume() -> f64 {
    volume_at(THICKNESS)
}

/// The plate with its through hole at `thickness`.
pub fn volume_at(thickness: f64) -> f64 {
    WIDTH * HEIGHT * thickness - PI * HOLE_RADIUS * HOLE_RADIUS * thickness
}

/// World (x, y) of sketch point (u, v). The XY sketch frame is non-standard
/// (u → world +Y, v → world −X), so the plate lies at x ∈ [−40, 0], y ∈ [0, 60].
pub fn world(u: f64, v: f64) -> [f64; 2] {
    let plane = WorldPlane::XY.plane();
    let at = |axis: usize| {
        let [o, x, y] = [plane.origin, plane.x_axis, plane.y_axis].map(|p| [p.x, p.y, p.z][axis]);
        o + u * x + v * y
    };
    [at(0), at(1)]
}

fn uuid(n: u128) -> Uuid {
    Uuid::from_u128(0x4600_0000 + n)
}
fn entity(n: u128) -> EntityId {
    EntityId(uuid(0x100 + n))
}
fn constraint(n: u128) -> ConstraintId {
    ConstraintId(uuid(0x200 + n))
}
pub fn plate_sketch() -> SketchId {
    SketchId(uuid(1))
}
pub fn hole_sketch() -> SketchId {
    SketchId(uuid(2))
}
pub fn plate_record() -> RecordId {
    RecordId(uuid(3))
}
pub fn hole_record() -> RecordId {
    RecordId(uuid(4))
}

const ARBITRARY: CurvePosition = CurvePosition::Arbitrary;

fn rectangle() -> Vec<SketchEditOp> {
    let corners = [[0.0, 0.0], [WIDTH, 0.0], [WIDTH, HEIGHT], [0.0, HEIGHT]];
    let (mut entities, mut constraints) = (Vec::new(), Vec::new());
    for side in 0..4u128 {
        for end in 0..2u128 {
            let [x, y] = corners[((side + end) % 4) as usize];
            let at = Vec2::new_unchecked(x, y);
            entities.push(SketchEntity::point(
                entity(side * 2 + end),
                at,
                false,
                false,
            ));
        }
        let line = entity(16 + side);
        entities.push(SketchEntity::line(
            line,
            entity(side * 2),
            entity(side * 2 + 1),
            false,
        ));
        constraints.push(Constraint::Coincident {
            id: constraint(side),
            point1: entity(side * 2 + 1),
            point2: entity(((side + 1) % 4) * 2),
            point1_position: ARBITRARY,
            point2_position: ARBITRARY,
        });
        constraints.push(if side % 2 == 0 {
            Constraint::Horizontal {
                id: constraint(4 + side),
                line,
            }
        } else {
            Constraint::Vertical {
                id: constraint(4 + side),
                line,
            }
        });
    }
    constraints.extend(rectangle_dimensions());
    edit_ops(entities, constraints)
}

fn rectangle_dimensions() -> [Constraint; 3] {
    [
        Constraint::Fixed {
            id: constraint(8),
            point: entity(0),
            point_position: ARBITRARY,
            at: Vec2::new_unchecked(0.0, 0.0),
        },
        Constraint::HorizontalDistance {
            id: constraint(9),
            point1: entity(0),
            point1_position: ARBITRARY,
            point2: entity(1),
            point2_position: ARBITRARY,
            value: Scalar::new(WIDTH),
        },
        Constraint::VerticalDistance {
            id: constraint(10),
            point1: entity(2),
            point1_position: ARBITRARY,
            point2: entity(3),
            point2_position: ARBITRARY,
            value: Scalar::new(HEIGHT),
        },
    ]
}

fn hole() -> Vec<SketchEditOp> {
    let (center, circle) = (entity(30), entity(31));
    let at = Vec2::new_unchecked(WIDTH / 2.0, HEIGHT / 2.0);
    let entities = vec![
        SketchEntity::point(center, at, false, false),
        SketchEntity::circle(circle, center, HOLE_RADIUS, false).unwrap(),
    ];
    let constraints = vec![
        Constraint::Fixed {
            id: constraint(30),
            point: center,
            point_position: ARBITRARY,
            at,
        },
        Constraint::Diameter {
            id: constraint(31),
            entity: circle,
            value: Scalar::new(2.0 * HOLE_RADIUS),
        },
    ];
    edit_ops(entities, constraints)
}

fn edit_ops(entities: Vec<SketchEntity>, constraints: Vec<Constraint>) -> Vec<SketchEditOp> {
    let entities = entities
        .into_iter()
        .map(|entity| SketchEditOp::AddEntity { entity });
    let constraints = constraints
        .into_iter()
        .map(|constraint| SketchEditOp::AddConstraint { constraint });
    entities.chain(constraints).collect()
}

/// Authors, solves and finishes one fully constrained XY sketch.
async fn sketch(harness: &Harness, id: SketchId, ops: Vec<SketchEditOp>) -> FinishSketchDto {
    let empty = Sketch::on_world_plane(id, "Reference plate profile", WorldPlane::XY);
    let (state, host) = (&harness.state, harness.host.clone());
    api::apply_edit_command(
        state,
        host.clone(),
        EditCommand::AddSketch { sketch: empty },
    )
    .await
    .unwrap();
    api::enter_sketch(state, host.clone(), id.to_string(), Some(true))
        .await
        .unwrap();
    let solve = api::sketch_upsert(state, host.clone(), id.to_string(), ops)
        .await
        .unwrap();
    assert_eq!(solve.status, SketchSolveStatus::FullyConstrained);
    let finish = api::finish_sketch(state, host, id.to_string())
        .await
        .unwrap();
    assert_eq!(finish.regions.len(), 1, "{finish:?}");
    harness.settled(&harness.projection().await).await;
    finish
}

fn extrude(sketch: SketchId, profile: &FinishSketchDto, cut: Option<BodyId>) -> Operation {
    let (mode, boolean_mode) = match cut {
        Some(_) => (ExtrudeMode::ThroughAll, BooleanMode::Cut),
        None => (ExtrudeMode::Blind, BooleanMode::NewBody),
    };
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(&profile.regions[0].region_id),
            region_identity_version: Some(profile.region_identity_version),
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(THICKNESS),
        draft_angle_deg: Scalar::new(0.0),
        mode,
        boolean_mode,
        target_body: cut,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }))
}

async fn add(harness: &Harness, id: RecordId, name: &str, op: Operation) -> DocumentProjection {
    let step = harness.projection().await.applied_ops as u32;
    let record = OperationRecord::new(id, step, name, op);
    let response = api::apply_operation(
        &harness.state,
        harness.host.clone(),
        vec![EditCommand::AddOperation {
            record,
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    harness.settled(&response.projection).await
}

/// Builds the solid plate before drilling; useful for hollowing workflows.
pub async fn build_solid(harness: &Harness) -> (BodyId, DocumentProjection) {
    let profile = sketch(harness, plate_sketch(), rectangle()).await;
    let plate = extrude(plate_sketch(), &profile, None);
    let projection = add(harness, plate_record(), "Plate", plate).await;
    assert_eq!(projection.bodies.len(), 1);
    let body: BodyId = projection.bodies.keys().next().unwrap().parse().unwrap();
    (body, projection)
}

/// Builds the plate and cuts the hole; returns the one body and the head.
pub async fn build(harness: &Harness) -> (BodyId, DocumentProjection) {
    let (body, _) = build_solid(harness).await;
    let circle = sketch(harness, hole_sketch(), hole()).await;
    let cut = extrude(hole_sketch(), &circle, Some(body));
    let projection = add(harness, hole_record(), "Through hole", cut).await;
    assert_eq!(
        projection.bodies.len(),
        1,
        "the cut modifies the plate in place"
    );
    (body, projection)
}
