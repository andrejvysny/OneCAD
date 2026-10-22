//! The Mac-authored reference plate for the simulator round trip: a fully
//! constrained 60 x 40 mm profile with one Ø6 mm hole, extruded 4 mm, built
//! headlessly through the shared services and saved through `save_document`.
//! Synthetic acceptance geometry, not product dimensions.
use super::host::Host;
use onecad_app::{
    api,
    dto::{FinishSketchDto, SketchSolveStatus},
};
use onecad_core::{
    document::{
        record::OperationRecord,
        record::{BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation},
        refs::SketchRegionRef,
        variables::Scalar,
    },
    edit::{EditCommand, SketchEditOp},
    ids::{ConstraintId, EntityId, RecordId, RegionId, SketchId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane},
};
use std::path::Path;
use uuid::Uuid;

pub const WIDTH: f64 = 60.0;
pub const HEIGHT: f64 = 40.0;
pub const THICKNESS: f64 = 4.0;
pub const HOLE_DIAMETER: f64 = 6.0;
const SKETCH: u128 = 0x1_0518_0000;
const EXTRUDE: u128 = 0x1_0518_0001;

fn sketch_id() -> SketchId {
    SketchId(Uuid::from_u128(SKETCH))
}
fn entity(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(SKETCH + 0x100 + n))
}
fn constraint(n: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(SKETCH + 0x200 + n))
}

fn point(sketch: &mut Sketch, id: EntityId, at: [f64; 2]) {
    sketch
        .add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(at[0], at[1]),
            false,
            false,
        ))
        .unwrap();
}

fn rectangle(sketch: &mut Sketch) {
    let corners = [[0.0, 0.0], [WIDTH, 0.0], [WIDTH, HEIGHT], [0.0, HEIGHT]];
    for side in 0..4u128 {
        point(sketch, entity(side * 2), corners[side as usize]);
        point(
            sketch,
            entity(side * 2 + 1),
            corners[(side as usize + 1) % 4],
        );
        let line = SketchEntity::line(
            entity(16 + side),
            entity(side * 2),
            entity(side * 2 + 1),
            false,
        );
        sketch.add_entity(line).unwrap();
    }
    // Every corner point exists before a constraint names one.
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
        let axis = if side % 2 == 0 {
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
        sketch.add_constraint(axis).unwrap();
    }
}

fn fix(sketch: &mut Sketch, id: ConstraintId, point: EntityId, at: [f64; 2]) {
    sketch
        .add_constraint(Constraint::Fixed {
            id,
            point,
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new_unchecked(at[0], at[1]),
        })
        .unwrap();
}

fn dimensions(sketch: &mut Sketch) {
    fix(sketch, constraint(8), entity(0), [0.0, 0.0]);
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

fn hole(sketch: &mut Sketch) {
    point(sketch, entity(30), [WIDTH / 2.0, HEIGHT / 2.0]);
    let circle = SketchEntity::circle(entity(31), entity(30), HOLE_DIAMETER / 2.0, false).unwrap();
    sketch.add_entity(circle).unwrap();
    fix(
        sketch,
        constraint(11),
        entity(30),
        [WIDTH / 2.0, HEIGHT / 2.0],
    );
    sketch
        .add_constraint(Constraint::Diameter {
            id: constraint(12),
            entity: entity(31),
            value: Scalar::new(HOLE_DIAMETER),
        })
        .unwrap();
}

pub fn profile() -> Sketch {
    let mut sketch = Sketch::on_world_plane(sketch_id(), "iPad acceptance plate", WorldPlane::XY);
    rectangle(&mut sketch);
    dimensions(&mut sketch);
    hole(&mut sketch);
    sketch
}

fn upsert_ops(sketch: &Sketch) -> Vec<SketchEditOp> {
    let entities = sketch.entities().iter().cloned();
    let constraints = sketch.constraints().iter().cloned();
    entities
        .map(|entity| SketchEditOp::AddEntity { entity })
        .chain(constraints.map(|constraint| SketchEditOp::AddConstraint { constraint }))
        .collect()
}

async fn finished_profile(host: &Host) -> FinishSketchDto {
    let empty = Sketch::on_world_plane(sketch_id(), "iPad acceptance plate", WorldPlane::XY);
    api::apply_edit_command(
        &host.state,
        host.host.clone(),
        EditCommand::AddSketch { sketch: empty },
    )
    .await
    .unwrap();
    let id = sketch_id().to_string();
    api::enter_sketch(&host.state, host.host.clone(), id.clone(), Some(true))
        .await
        .unwrap();
    let solve = api::sketch_upsert(
        &host.state,
        host.host.clone(),
        id.clone(),
        upsert_ops(&profile()),
    )
    .await
    .unwrap();
    assert_eq!(
        solve.status,
        SketchSolveStatus::FullyConstrained,
        "{solve:?}"
    );
    assert_eq!(solve.dof, 0);
    api::finish_sketch(&host.state, host.host.clone(), id)
        .await
        .unwrap()
}

fn extrude(profile: &FinishSketchDto) -> OperationRecord {
    let region = profile
        .regions
        .iter()
        .find(|region| region.holes.len() == 1)
        .expect("the plate region has the Ø6 hole as its one inner loop");
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch: sketch_id(),
            region: RegionId::new(&region.region_id),
            region_identity_version: Some(profile.region_identity_version),
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(THICKNESS),
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
    let operation = Operation::Known(KnownOperation::Extrude(params));
    OperationRecord::new(RecordId(Uuid::from_u128(EXTRUDE)), 1, "Plate", operation)
}

/// Builds the plate in a fresh host under `scratch` and saves it to `target`.
pub async fn author(target: &Path, scratch: &Path) {
    let host = Host::start(scratch);
    api::new_document(&host.state, host.host.clone())
        .await
        .unwrap();
    let worker = host.state.live_worker().expect("real worker adopted");
    assert!(worker.wait_ready(std::time::Duration::from_secs(15)).await);
    let profile = finished_profile(&host).await;
    let sketched = host.projection().await;
    host.settle(&sketched).await;
    let command = EditCommand::AddOperation {
        record: extrude(&profile),
        at_cursor: true,
    };
    let effects = api::OperationEffects {
        hide_sketch_ids: vec![sketch_id().to_string()],
    };
    let applied =
        api::apply_operation(&host.state, host.host.clone(), vec![command], Some(effects))
            .await
            .unwrap();
    let report = host.settle(&applied.projection).await;
    assert_eq!(report.outcome, "published", "{report:?}");
    let saved = api::save_document(
        &host.state,
        host.host.clone(),
        Some(target.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean, "save persists the current revision");
    host.close().await;
}
