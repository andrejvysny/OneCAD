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
    checkpoint: Option<std::path::PathBuf>,
}
impl<'a> Builder<'a> {
    pub fn new(h: &'a Harness) -> Self {
        Self {
            h,
            next: 1,
            checkpoint: None,
        }
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
        self.h.settled(&result.projection).await;
        if let Some(path) = &self.checkpoint {
            assert!(
                api::save_document(
                    &self.h.state,
                    self.h.host.clone(),
                    Some(path.to_string_lossy().into_owned()),
                    None,
                )
                .await
                .unwrap()
                .clean
            );
        }
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

async fn capture_inputs(h: &Harness, sharp: BodyId, protections: &[BodyId]) {
    let Some(directory) = std::env::var_os("ONECAD_CLIP_CAPTURE_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    capture_inputs_at(h, sharp, protections, &directory).await;
}

async fn capture_inputs_at(
    h: &Harness,
    sharp: BodyId,
    protections: &[BodyId],
    directory: &std::path::Path,
) {
    std::fs::create_dir(directory).unwrap();
    let worker = h.state.live_worker().unwrap();
    let snapshot = api::preview_context(&h.state).await.unwrap().snapshot_id;
    let mut rows = Vec::new();
    for (index, body) in std::iter::once(sharp)
        .chain(protections.iter().copied())
        .enumerate()
    {
        let path = directory.join(format!("input-{index}.brep"));
        let result = worker
            .export_geometry(
                path.to_str().unwrap(),
                &[body],
                "brep",
                false,
                Some(snapshot),
            )
            .await
            .unwrap();
        assert_eq!(result.solid_count, 1);
        rows.push(
            serde_json::json!({"body":body.to_string(),"path":path,"bytes":result.bytes_written}),
        );
    }
    std::fs::write(
        directory.join("inputs.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

pub async fn build_cutter(h: &Harness) -> BodyId {
    let mut builder = Builder::new(h);
    let (sharp, bounds) = solids::sharp(&mut builder).await;
    let mut protections = pipes::build(&mut builder).await;
    protections.extend(solids::protections(&mut builder, bounds).await);
    assert_eq!(protections.len(), 9);
    capture_inputs(h, sharp, &protections).await;
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

pub async fn cut_and_restore_original_material(
    h: &Harness,
    original: BodyId,
    checkpoint: &std::path::Path,
) {
    let mut builder = Builder::new(h);
    builder.checkpoint = Some(checkpoint.to_owned());
    let (sharp, bounds) = solids::sharp(&mut builder).await;
    let distributed = std::env::var_os("ONECAD_FRAMEWORK_DISTRIBUTED_RETENTION").is_some();
    let hybrid = std::env::var_os("ONECAD_FRAMEWORK_HYBRID_RETENTION").is_some();
    let eight = std::env::var_os("ONECAD_FRAMEWORK_EIGHT_RETENTIONS").is_some();
    assert!(
        !eight || hybrid,
        "eight-retention diagnostic requires the hybrid recipe"
    );
    if eight {
        eprintln!("Private eight-retention candidate: omission is explicit, not certified redundant or source-equivalent");
    }
    let mut protections = pipes::build(&mut builder).await;
    if !distributed && !hybrid {
        protections.extend(solids::protections(&mut builder, bounds).await);
    }
    let mut retained = Vec::new();
    for index in 0..9 {
        if eight && index == 1 {
            continue;
        }
        let protection = if hybrid && index >= 2 && index != 3 && index != 6 {
            Some(solids::protection(&mut builder, bounds, index - 2).await)
        } else {
            protections.get(index).copied()
        };
        if index < 2 && std::env::var_os("ONECAD_FRAMEWORK_RESTORE_UNCLIPPED_PIPES").is_some() {
            eprintln!("private candidate: unclipped pipe {index}; containment remains unqualified");
            retained.push(protection.unwrap());
            continue;
        }
        let copy = builder
            .created(
                "Retain original material for clip blend",
                Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
                    targets: vec![original],
                    translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
                    rotate: Default::default(),
                    copy: true,
                    extra: Default::default(),
                })),
            )
            .await;
        if index >= 2 && (distributed || (hybrid && (index == 3 || index == 6))) {
            solids::retain_piece(&mut builder, copy, bounds, index - 2).await;
            retained.push(copy);
            continue;
        }
        let protection = protection.unwrap();
        if index == 0 {
            if let Some(directory) = std::env::var_os("ONECAD_FRAMEWORK_MATERIAL_CAPTURE_DIR") {
                capture_inputs_at(h, copy, &[protection], std::path::Path::new(&directory)).await;
            }
        }
        builder
            .boolean(BooleanOp::Intersect, copy, vec![protection])
            .await;
        retained.push(copy);
    }
    // B - (S - P) = (B - S) union (B intersect P).
    builder.boolean(BooleanOp::Cut, original, vec![sharp]).await;
    assert_eq!(retained.len(), if eight { 8 } else { 9 });
    builder.boolean(BooleanOp::Union, original, retained).await;
    assert_eq!(h.projection().await.bodies.len(), 1);
    if std::env::var_os("ONECAD_FRAMEWORK_NEGATIVE_CLIP").is_some() {
        assert!(
            eight && hybrid,
            "negative diagnostic requires the eight-retention hybrid"
        );
        let output = checkpoint
            .parent()
            .expect("checkpoint directory")
            .join("positive-live");
        assert!(!output.exists(), "positive live capture already exists");
        capture_inputs_at(h, original, &[], &output).await;
        apply_negative_diagnostic(&mut builder, original).await;
    }
}

async fn apply_negative_diagnostic(builder: &mut Builder<'_>, original: BodyId) {
    let (sharp, bounds) = negative::sharp(builder).await;
    if let Some(path) = std::env::var_os("ONECAD_FRAMEWORK_SHARP_CAPTURE_DIR") {
        let directory = std::path::PathBuf::from(path);
        assert!(
            !directory.exists(),
            "sharp capture directory already exists"
        );
        capture_inputs_at(builder.h, original, &[sharp], &directory).await;
    }
    let pipes = pipes::build_negative_eight(builder).await;
    let mut retained = Vec::new();
    for index in 0..9 {
        if index == 1 {
            continue;
        }
        let protection = if index == 0 {
            Some(pipes[0])
        } else if index != 3 && index != 6 {
            Some(negative::protection(builder, bounds, index - 2).await)
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
        if index == 3 || index == 6 {
            negative::retain_piece(builder, copy, bounds, index - 2).await;
        } else {
            builder
                .boolean(BooleanOp::Intersect, copy, vec![protection.unwrap()])
                .await;
        }
        retained.push(copy);
    }
    builder.boolean(BooleanOp::Cut, original, vec![sharp]).await;
    builder.boolean(BooleanOp::Union, original, retained).await;
    assert_eq!(builder.h.projection().await.bodies.len(), 1);
}
