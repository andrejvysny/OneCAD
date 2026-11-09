//! Native editable source-derived cutter. No imported geometry.
mod geometry;
mod negative;
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
    pub fn new(h: &'a Harness, next: u128) -> Self {
        Self { h, next }
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
    async fn apply_report(
        &mut self,
        name: &str,
        operation: Operation,
    ) -> (RecordId, onecad_app::dto::RegenFinished) {
        let id = RecordId(self.id());
        eprintln!("clip native step {id}: {name}");
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
        let report = self.h.settled(&result.projection).await;
        self.h
            .manual_save(&id.to_string())
            .await
            .expect("manual save after accepted operation");
        (id, report)
    }
    async fn apply(&mut self, name: &str, operation: Operation) -> RecordId {
        self.apply_report(name, operation).await.0
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
    async fn boolean_results(
        &mut self,
        kind: BooleanOp,
        target: BodyId,
        tools: Vec<BodyId>,
        keep: bool,
    ) -> Vec<BodyId> {
        assert!(!tools.is_empty());
        let params = BooleanParams::new(kind, target, tools.clone(), keep);
        let (id, report) = self
            .apply_report(
                "Editable analytic Boolean",
                Operation::Known(KnownOperation::Boolean(params)),
            )
            .await;
        let projection = self.h.projection().await;
        let affected = report
            .affected_bodies
            .get(&id.to_string())
            .expect("Boolean affected bodies");
        let bodies: Vec<BodyId> = affected
            .iter()
            .map(|s| {
                assert!(
                    projection.bodies.contains_key(s),
                    "reported body absent: {s}"
                );
                s.parse().unwrap()
            })
            .collect();
        assert!(!bodies.is_empty());
        if keep {
            for tool in tools {
                assert!(projection.bodies.contains_key(&tool.to_string()));
            }
        }
        bodies
    }
    async fn boolean(&mut self, kind: BooleanOp, target: BodyId, tools: Vec<BodyId>) {
        let results = self.boolean_results(kind, target, tools, false).await;
        assert_eq!(
            results,
            vec![target],
            "unexpected split in prerequisite operation"
        );
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

async fn capture_retainer(h: &Harness, index: usize, bodies: &[BodyId], out: &std::path::Path) {
    use std::io::Write;
    let mut masses = Vec::new();
    for body in bodies {
        let query = api::query_mass_properties(
            &h.state,
            onecad_app::worker::wire::body_id_wire(*body),
            None,
        );
        let mass = tokio::time::timeout(std::time::Duration::from_secs(10), query)
            .await
            .unwrap()
            .unwrap();
        masses.push(serde_json::json!({"bodyId":body.to_string(),"mass":mass}));
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(out.join("retainers.jsonl"))
        .unwrap();
    serde_json::to_writer(
        &mut file,
        &serde_json::json!({"retainerIndex":index,"bodies":masses}),
    )
    .unwrap();
    writeln!(file).unwrap();
    file.flush().unwrap();
}

pub async fn apply_negative(h: &Harness, original: BodyId, next: u128, out: &std::path::Path) {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out.join("retainers.jsonl"))
        .unwrap();
    let mut builder = Builder::new(h, next);
    let (sharp, bounds) = negative::sharp(&mut builder).await;
    let pipes = pipes::build_negative_eight(&mut builder).await;
    let mut retained = Vec::new();
    for index in 0..9 {
        if index == 1 {
            continue;
        }
        let protection = if index == 0 {
            Some(pipes[0])
        } else if !matches!(index, 3 | 6 | 8) {
            Some(negative::protection(&mut builder, bounds, index - 2).await)
        } else {
            None
        };
        let copy = builder
            .created(
                "Retain negative-side original material",
                Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
                    targets: vec![original],
                    translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
                    rotate: Default::default(),
                    copy: true,
                    extra: Default::default(),
                })),
            )
            .await;
        // Distribute B intersect (slab minus primitive) to avoid the failing
        // sphere-complement intersection without changing the retained set.
        let results = if matches!(index, 3 | 6 | 8) {
            negative::retain_piece(&mut builder, copy, bounds, index - 2, sharp, original, out)
                .await
        } else {
            builder
                .boolean_results(BooleanOp::Intersect, copy, vec![protection.unwrap()], false)
                .await
        };
        capture_retainer(h, index, &results, out).await;
        retained.extend(results);
    }
    let mut difference = builder
        .boolean_results(BooleanOp::Cut, original, vec![sharp], false)
        .await;
    let target = difference.remove(0);
    retained.extend(difference);
    let results = builder
        .boolean_results(BooleanOp::Union, target, retained, false)
        .await;
    eprintln!("final restoration bodies: {results:?}");
}
