use super::*;

pub(super) fn outer_sketch(data: &serde_json::Value) -> Sketch {
    let mut sketch = Sketch::on_world_plane(SketchId(id(1)), "Lever outer profile", WorldPlane::XY);
    for (i, curve) in data["curves"].as_array().unwrap().iter().enumerate() {
        let base = 100 + i as u128 * 3;
        let center = EntityId(id(base));
        let other = EntityId(id(base + 1));
        let entity = EntityId(id(base + 2));
        match curve["kind"].as_str().unwrap() {
            "line" => {
                add_source_point(&mut sketch, center, &curve["start"]);
                add_source_point(&mut sketch, other, &curve["end"]);
                sketch
                    .add_entity(SketchEntity::line(entity, center, other, false))
                    .unwrap();
            }
            "arc" => {
                add_source_point(&mut sketch, center, &curve["center"]);
                sketch
                    .add_entity(
                        SketchEntity::arc(
                            entity,
                            center,
                            curve["radius"].as_f64().unwrap(),
                            curve["startAngle"].as_f64().unwrap(),
                            curve["endAngle"].as_f64().unwrap(),
                            false,
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
            other => panic!("unsupported outer curve {other}"),
        }
    }
    sketch
}

fn add_source_point(sketch: &mut Sketch, id: EntityId, value: &serde_json::Value) {
    sketch
        .add_entity(SketchEntity::point(id, point(value), false, false))
        .unwrap();
}

pub(super) async fn add(h: &Harness, n: u128, op: Operation) {
    let response = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(RecordId(id(n)), 0, "Source-derived lever", op),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&response.projection).await;
}

fn polygon_sketch(n: u128, points: &[[f64; 2]]) -> Sketch {
    let mut sketch = Sketch::on_world_plane(SketchId(id(n)), "Lever cut profile", WorldPlane::XY);
    for (i, p) in points.iter().enumerate() {
        sketch
            .add_entity(SketchEntity::point(
                EntityId(id(n * 100 + i as u128)),
                Vec2::new(p[0], p[1]).unwrap(),
                false,
                false,
            ))
            .unwrap();
    }
    for i in 0..points.len() {
        sketch
            .add_entity(SketchEntity::line(
                EntityId(id(n * 100 + 50 + i as u128)),
                EntityId(id(n * 100 + i as u128)),
                EntityId(id(n * 100 + ((i + 1) % points.len()) as u128)),
                false,
            ))
            .unwrap();
    }
    sketch
}

pub(super) fn plane(
    origin: [f64; 3],
    x: [f64; 3],
    y: [f64; 3],
    normal: [f64; 3],
) -> SketchPlaneRef {
    let v = |a: [f64; 3]| Vec3::new(a[0], a[1], a[2]).unwrap();
    SketchPlaneRef {
        kind: PlaneKind::Custom,
        origin: v(origin),
        x_axis: v(x),
        y_axis: v(y),
        normal: v(normal),
        extra: Default::default(),
    }
}

async fn cut_profile(
    h: &Harness,
    sketch: &Sketch,
    frame: SketchPlaneRef,
    record: u128,
) -> SketchRegionRef {
    let worker = h.state.live_worker().unwrap();
    worker.sketch_upsert(sketch).await.unwrap();
    let regions = worker.sketch_regions(&sketch.id.to_string()).await.unwrap();
    assert_eq!(regions.regions.len(), 1, "cut contour must have one region");
    let profile = SketchRegionRef {
        sketch: sketch.id,
        region: RegionId::new(&regions.regions[0].region_id),
        region_identity_version: Some(regions.region_identity_version),
        region_anchor: None,
        extra: Default::default(),
    };
    let (_, entities, constraints) = sketch_wire(sketch);
    add(
        h,
        record,
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sketch.id,
            plane: frame,
            entities: entities.as_array().unwrap().clone(),
            constraints: constraints.as_array().unwrap().clone(),
            host_face: None,
            extra: Default::default(),
        })),
    )
    .await;
    profile
}

async fn cut_extrude(h: &Harness, profile: SketchRegionRef, record: u128, distance: f64) {
    let params = ExtrudeParams {
        profile: Some(profile),
        distance: Scalar::new(distance),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::Cut,
        target_body: Some(BodyId(id(3))),
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    };
    add(h, record, Operation::Known(KnownOperation::Extrude(params))).await;
}

pub(super) async fn apply_remaining_features(h: &Harness, data: &serde_json::Value) {
    let slot = data["slot"]["cornersXZ"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()])
        .collect::<Vec<_>>();
    let sketch = polygon_sketch(10, &slot);
    let frame = plane(
        [0.0, 4.00000005960464, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, -1.0, 0.0],
    );
    let profile = cut_profile(h, &sketch, frame, 11).await;
    cut_extrude(h, profile, 12, 8.0).await;
    cut_hinge(h, data).await;
    cut_transverse(h, data).await;
}

async fn cut_hinge(h: &Harness, data: &serde_json::Value) {
    let hinge = &data["hinge"];
    let radius = hinge["radius"].as_f64().unwrap();
    let transition = hinge["transitionRadius"].as_f64().unwrap();
    let mouth = hinge["mouthRadius"].as_f64().unwrap();
    let middle = hinge["yMid"].as_f64().unwrap();
    let half = hinge["transitionHalfLength"].as_f64().unwrap();
    let extension = mouth + 0.1 * 15_f64.to_radians().tan();
    let polygon = [
        [0., -4.1],
        [extension, -4.1],
        [mouth, -4.0],
        [transition, -half],
        [radius, -2.0],
        [radius, 2.0],
        [transition, half],
        [mouth, 4.0],
        [extension, 4.1],
        [0., 4.1],
    ];
    let sketch = polygon_sketch(20, &polygon);
    let axis = EntityId(id(20 * 100 + 50 + polygon.len() as u128 - 1));
    let c = &hinge["centerXZ"];
    let frame = plane(
        [c[0].as_f64().unwrap(), middle, c[1].as_f64().unwrap()],
        [1., 0., 0.],
        [0., 1., 0.],
        [0., 0., 1.],
    );
    let profile = cut_profile(h, &sketch, frame, 21).await;
    let revolve = RevolveParams {
        profile: Some(profile),
        angle_deg: Scalar::new(360.0),
        axis: Some(AxisRef::SketchLine {
            sketch: sketch.id,
            line: axis,
            extra: Default::default(),
        }),
        boolean_mode: BooleanMode::Cut,
        target_body: Some(BodyId(id(3))),
        extra: Default::default(),
    };
    add(h, 22, Operation::Known(KnownOperation::Revolve(revolve))).await;
}

async fn cut_transverse(h: &Harness, data: &serde_json::Value) {
    let t = &data["transverse"];
    let a = &t["direction"];
    let d = [
        a[0].as_f64().unwrap(),
        a[1].as_f64().unwrap(),
        a[2].as_f64().unwrap(),
    ];
    let o = &t["origin"];
    let origin = [
        o[0].as_f64().unwrap() - 2.0 * d[0],
        o[1].as_f64().unwrap(),
        o[2].as_f64().unwrap() - 2.0 * d[2],
    ];
    let mut sketch = Sketch::on_world_plane(SketchId(id(30)), "Transverse bore", WorldPlane::XY);
    let center = EntityId(id(3000));
    sketch
        .add_entity(SketchEntity::point(
            center,
            Vec2::new(0., 0.).unwrap(),
            false,
            false,
        ))
        .unwrap();
    sketch
        .add_entity(
            SketchEntity::circle(
                EntityId(id(3001)),
                center,
                t["radius"].as_f64().unwrap(),
                false,
            )
            .unwrap(),
        )
        .unwrap();
    let frame = plane(origin, [0., 1., 0.], [-d[2], 0., d[0]], d);
    let profile = cut_profile(h, &sketch, frame, 31).await;
    cut_extrude(h, profile, 32, 9.0).await;
}
