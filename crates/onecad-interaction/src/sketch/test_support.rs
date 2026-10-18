// Compiled only by the sketch transition integration test.
use onecad_app::dto::{FinishSketchDto, SketchSessionDto, SketchSolveStatus, SketchUpsertDto};
use onecad_core::{
    edit::SketchEditOp,
    ids::{ConstraintId, EntityId, SketchId, SnapshotId},
    math::Vec2,
    sketch::{Sketch, SketchPlane, WorldPlane},
};
use onecad_interaction::{sketch::*, DocumentContext};
use onecad_scene::{Camera, DVec3, Projection, SceneIdentity, ViewSnapshot, Viewport};
use uuid::Uuid;

pub fn context() -> DocumentContext {
    DocumentContext {
        document_id: "sketch-doc".into(),
        runtime_session: "runtime-1".into(),
        revision: 4,
        snapshot_id: SnapshotId(8),
        scene: SceneIdentity {
            session: 1,
            scene: 1,
        },
    }
}
pub fn empty(plane: SketchPlane) -> Sketch {
    let mut sketch = Sketch::on_world_plane(SketchId(Uuid::from_u128(1)), "Sketch", WorldPlane::XY);
    sketch.plane = plane;
    sketch
}
pub fn enter_dto(id: SketchId) -> SketchSessionDto {
    SketchSessionDto {
        sketch_id: id.to_string(),
        plane: Default::default(),
        entities: Default::default(),
        constraints: Default::default(),
        dof: 0,
        status: SketchSolveStatus::UnderConstrained,
        truth: Default::default(),
        solve_current: true,
        conflicting: Vec::new(),
        entity_states: Default::default(),
        projections: Default::default(),
    }
}
pub fn solve_dto(id: SketchId) -> SketchUpsertDto {
    SketchUpsertDto {
        sketch_id: id.to_string(),
        sketch_revision: 1,
        dof: 0,
        status: SketchSolveStatus::UnderConstrained,
        truth: Default::default(),
        conflicting: Vec::new(),
        solved_positions: Default::default(),
        solved_curves: Default::default(),
        entity_states: Default::default(),
    }
}
pub fn active(plane: SketchPlane, tool: SketchTool) -> SketchInteraction {
    let mut state = SketchInteraction::default();
    state.dispatch(SketchAction::ContextChanged(Some(context())));
    let sketch = empty(plane);
    let effects = state.dispatch(SketchAction::Begin {
        sketch: Box::new(sketch.clone()),
        create: false,
        context: context(),
    });
    let SketchEffect::EnterSketch { tag, .. } = &effects[0] else {
        panic!()
    };
    state.dispatch(SketchAction::Entered {
        tag: tag.clone(),
        snapshot: SketchSnapshot {
            document: context(),
            sketch: Box::new(sketch.clone()),
        },
        solve: Box::new(enter_dto(sketch.id)),
    });
    state.dispatch(SketchAction::SetTool(tool));
    state
}
pub fn view(state: &SketchInteraction) -> ViewSnapshot {
    let session = state.session.as_ref().unwrap();
    let p = session.plane;
    ViewSnapshot::new(
        session.document.scene,
        1,
        Camera {
            target: DVec3::new(p.origin.x, p.origin.y, p.origin.z),
            offset: DVec3::new(p.normal.x, p.normal.y, p.normal.z).normalize() * 100.0,
            projection: Projection::Orthographic,
            ..Camera::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 2.0,
        },
    )
    .unwrap()
}
pub fn point(state: &SketchInteraction, x: f64, y: f64) -> CapturedSketchPoint {
    let session = state.session.as_ref().unwrap();
    let view = view(state);
    let world = session.plane.to_world(Vec2::new(x, y).unwrap());
    let screen = view
        .project(DVec3::new(world.x, world.y, world.z))
        .unwrap()
        .logical;
    capture_sketch_pointer(session, &view, screen).unwrap()
}
pub fn ids(seed: u128) -> PrimitiveIds {
    PrimitiveIds {
        entities: std::array::from_fn(|i| EntityId(Uuid::from_u128(seed + i as u128))),
        constraints: std::array::from_fn(|i| ConstraintId(Uuid::from_u128(seed + 100 + i as u128))),
    }
}
pub fn click(state: &mut SketchInteraction, x: f64, y: f64, seed: u128) -> Vec<SketchEffect> {
    let point = point(state, x, y);
    state.dispatch(SketchAction::Click {
        point,
        ids: ids(seed),
    })
}
pub fn upsert(effects: Vec<SketchEffect>) -> (SketchTag, Vec<SketchEditOp>) {
    assert_eq!(effects.len(), 1);
    let SketchEffect::Upsert { tag, ops } = effects.into_iter().next().unwrap() else {
        panic!()
    };
    (tag, ops)
}
pub fn applied(state: &SketchInteraction, ops: &[SketchEditOp]) -> SketchSnapshot {
    let mut sketch = state.session.as_ref().unwrap().authoritative.clone();
    for op in ops {
        match op {
            SketchEditOp::AddEntity { entity } => sketch.add_entity(entity.clone()).unwrap(),
            SketchEditOp::AddConstraint { constraint } => {
                sketch.add_constraint(constraint.clone()).unwrap()
            }
            _ => panic!(),
        }
    }
    SketchSnapshot {
        document: state.context.clone().unwrap(),
        sketch: Box::new(sketch),
    }
}
pub fn complete(
    state: &mut SketchInteraction,
    tag: SketchTag,
    snapshot: SketchSnapshot,
) -> Vec<SketchEffect> {
    state.dispatch(SketchAction::BatchAccepted { tag: tag.clone() });
    let solve = Box::new(solve_dto(snapshot.sketch.id));
    state.dispatch(SketchAction::Solved {
        tag,
        snapshot,
        solve,
    })
}
pub fn primitive(
    state: &mut SketchInteraction,
    x: f64,
    y: f64,
    seed: u128,
) -> (SketchTag, SketchSnapshot) {
    let (tag, ops) = upsert(click(state, x, y, seed));
    let snapshot = applied(state, &ops);
    (tag, snapshot)
}
pub fn closed(state: &mut SketchInteraction) {
    let effects = state.dispatch(SketchAction::Finish);
    let SketchEffect::FinishSketch { tag, .. } = &effects[0] else {
        panic!()
    };
    state.dispatch(SketchAction::Finished {
        tag: tag.clone(),
        result: Box::new(FinishSketchDto {
            region_identity_version: 2,
            regions: Vec::new(),
            diagnostics: Vec::new(),
        }),
    });
}
