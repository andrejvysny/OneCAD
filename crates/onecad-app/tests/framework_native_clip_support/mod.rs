//! Native editable source-derived cutter. No imported geometry.
mod geometry;
mod pipes;
mod solids;

use crate::host::Harness;
use onecad_app::{
    api,
    worker::{wire::sketch_wire, SolverEngine},
};
use onecad_core::{
    document::{record::*, refs::*, variables::Scalar},
    edit::EditCommand,
    ids::{BodyId, EntityId, RecordId, RegionId, SketchId},
    math::{Vec2, Vec3},
    sketch::{Sketch, SketchEntity, WorldPlane},
};
use uuid::Uuid;

pub struct Builder<'a> {
    pub h: &'a Harness,
    next: u128,
}
impl<'a> Builder<'a> {
    pub fn new(h: &'a Harness) -> Self {
        Self { h, next: 1 }
    }
    fn id(&mut self) -> Uuid {
        let id = Uuid::from_u128(0xfabc_0000 + self.next);
        self.next += 1;
        id
    }
    fn entity(&mut self) -> EntityId {
        EntityId(self.id())
    }
    fn sketch_id(&mut self) -> SketchId {
        SketchId(self.id())
    }
    async fn apply(&mut self, name: &str, operation: Operation) -> RecordId {
        let id = RecordId(self.id());
        let result = api::apply_operation(
            &self.h.state,
            self.h.host.clone(),
            vec![EditCommand::AddOperation {
                record: OperationRecord::new(id, 0, name, operation),
                at_cursor: true,
            }],
            None,
        )
        .await
        .unwrap();
        self.h.settled(&result.projection).await;
        id
    }
    async fn created(&mut self, name: &str, operation: Operation) -> BodyId {
        let before = self.h.projection().await.bodies;
        self.apply(name, operation).await;
        let after = self.h.projection().await.bodies;
        let created: Vec<_> = after
            .keys()
            .filter(|id| !before.contains_key(*id))
            .collect();
        assert_eq!(created.len(), 1, "{name}: expected exactly one new body");
        created[0].parse().unwrap()
    }
    async fn sketch(&mut self, sketch: Sketch, frame: SketchPlaneRef) -> SketchRegionRef {
        let worker = self.h.state.live_worker().unwrap();
        worker.sketch_upsert(&sketch).await.unwrap();
        let regions = worker.sketch_regions(&sketch.id.to_string()).await.unwrap();
        assert_eq!(regions.regions.len(), 1);
        let region = SketchRegionRef {
            sketch: sketch.id,
            region: RegionId::new(&regions.regions[0].region_id),
            region_identity_version: Some(regions.region_identity_version),
            region_anchor: None,
            extra: Default::default(),
        };
        let (_, entities, constraints) = sketch_wire(&sketch);
        let params = SketchOpParams {
            sketch: sketch.id,
            plane: frame,
            entities: entities.as_array().unwrap().clone(),
            constraints: constraints.as_array().unwrap().clone(),
            host_face: None,
            extra: Default::default(),
        };
        self.apply(
            "Editable analytic sketch",
            Operation::Known(KnownOperation::Sketch(params)),
        )
        .await;
        region
    }
    fn empty_sketch(&mut self) -> Sketch {
        Sketch::on_world_plane(self.sketch_id(), "Editable clip profile", WorldPlane::XY)
    }
    fn polygon(&mut self, coords: &[[f64; 2]]) -> Sketch {
        let mut sketch = self.empty_sketch();
        let points: Vec<_> = coords
            .iter()
            .map(|p| {
                let id = self.entity();
                sketch
                    .add_entity(SketchEntity::point(
                        id,
                        Vec2::new(p[0], p[1]).unwrap(),
                        false,
                        false,
                    ))
                    .unwrap();
                id
            })
            .collect();
        for i in 0..points.len() {
            let id = self.entity();
            sketch
                .add_entity(SketchEntity::line(
                    id,
                    points[i],
                    points[(i + 1) % points.len()],
                    false,
                ))
                .unwrap();
        }
        sketch
    }
    fn circle(&mut self, radius: f64) -> Sketch {
        let mut s = self.empty_sketch();
        let center = self.entity();
        let curve = self.entity();
        s.add_entity(SketchEntity::point(
            center,
            Vec2::new(0., 0.).unwrap(),
            false,
            false,
        ))
        .unwrap();
        s.add_entity(SketchEntity::circle(curve, center, radius, false).unwrap())
            .unwrap();
        s
    }
    async fn boolean(&mut self, kind: BooleanOp, target: BodyId, tools: Vec<BodyId>) {
        assert!(!tools.is_empty());
        let mut params = BooleanParams::single(kind, target, tools[0]);
        params.tools = tools;
        self.apply(
            "Editable analytic Boolean",
            Operation::Known(KnownOperation::Boolean(params)),
        )
        .await;
    }
}
fn vector(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2]).unwrap()
}
fn frame(o: [f64; 3], x: [f64; 3], y: [f64; 3], n: [f64; 3]) -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Custom,
        origin: vector(o),
        x_axis: vector(x),
        y_axis: vector(y),
        normal: vector(n),
        extra: Default::default(),
    }
}
fn extrusion(profile: SketchRegionRef, distance: f64) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(profile),
        distance: Scalar::new(distance),
        draft_angle_deg: Scalar::new(0.),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.),
        target_face2: None,
        extra: Default::default(),
    }))
}

pub async fn build_cutter(h: &Harness) -> BodyId {
    let mut builder = Builder::new(h);
    let (sharp, bounds) = solids::sharp(&mut builder).await;
    let mut protections = pipes::build(&mut builder).await;
    protections.extend(solids::protections(&mut builder, bounds).await);
    assert_eq!(protections.len(), 9);
    builder.boolean(BooleanOp::Cut, sharp, protections).await;
    let projection = h.projection().await;
    assert_eq!(
        projection.bodies.len(),
        1,
        "all protection bodies must be consumed"
    );
    assert!(projection.bodies.contains_key(&sharp.to_string()));
    sharp
}
