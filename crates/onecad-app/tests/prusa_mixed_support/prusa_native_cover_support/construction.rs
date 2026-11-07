use super::*;

fn outer_sketch(data: &serde_json::Value, n: u128) -> Sketch {
    let mut sketch =
        Sketch::on_world_plane(SketchId(id(n)), "Cover source contour", WorldPlane::XY);
    for (i, curve) in data["curves"].as_array().unwrap().iter().enumerate() {
        let base = n * 100 + i as u128 * 3;
        let first = EntityId(id(base));
        let second = EntityId(id(base + 1));
        let entity = EntityId(id(base + 2));
        match curve["kind"].as_str().unwrap() {
            "line" => {
                sketch
                    .add_entity(SketchEntity::point(
                        first,
                        point(&curve["start"]),
                        false,
                        false,
                    ))
                    .unwrap();
                sketch
                    .add_entity(SketchEntity::point(
                        second,
                        point(&curve["end"]),
                        false,
                        false,
                    ))
                    .unwrap();
                sketch
                    .add_entity(SketchEntity::line(entity, first, second, false))
                    .unwrap();
            }
            "arc" => {
                sketch
                    .add_entity(SketchEntity::point(
                        first,
                        point(&curve["center"]),
                        false,
                        false,
                    ))
                    .unwrap();
                sketch
                    .add_entity(
                        SketchEntity::arc(
                            entity,
                            first,
                            curve["radius"].as_f64().unwrap(),
                            curve["startAngle"].as_f64().unwrap(),
                            curve["endAngle"].as_f64().unwrap(),
                            false,
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
            kind => panic!("unsupported cover contour curve {kind}"),
        }
    }
    sketch
}

pub(super) async fn create_profile(
    h: &Harness,
    data: &serde_json::Value,
    n: u128,
) -> SketchRegionRef {
    let sketch = outer_sketch(data, n);
    let worker = h.state.live_worker().unwrap();
    worker.sketch_upsert(&sketch).await.unwrap();
    let regions = worker.sketch_regions(&sketch.id.to_string()).await.unwrap();
    assert_eq!(
        regions.regions.len(),
        1,
        "outer contour must have one region"
    );
    let profile = SketchRegionRef {
        sketch: sketch.id,
        region: RegionId::new(&regions.regions[0].region_id),
        region_identity_version: Some(regions.region_identity_version),
        region_anchor: None,
        extra: Default::default(),
    };
    let (_, entities, constraints) = sketch_wire(&sketch);
    let plane = SketchPlaneRef {
        kind: PlaneKind::Custom,
        origin: Vec3::new(0.0, data["planeOrigin"][1].as_f64().unwrap(), 0.0).unwrap(),
        x_axis: Vec3::new(1.0, 0.0, 0.0).unwrap(),
        y_axis: Vec3::new(0.0, 0.0, 1.0).unwrap(),
        normal: Vec3::new(0.0, -1.0, 0.0).unwrap(),
        extra: Default::default(),
    };
    add(
        h,
        n + 1,
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sketch.id,
            plane,
            entities: entities.as_array().unwrap().clone(),
            constraints: constraints.as_array().unwrap().clone(),
            host_face: None,
            extra: Default::default(),
        })),
    )
    .await;
    profile
}

fn bevel_sketch(face: &serde_json::Value) -> Sketch {
    let edges = &face["wires"][0];
    let high = &edges[0]["start"];
    let low = &edges[0]["end"];
    let points = [
        [high[0].as_f64().unwrap(), high[1].as_f64().unwrap()],
        [low[0].as_f64().unwrap(), low[1].as_f64().unwrap()],
        [low[0].as_f64().unwrap(), high[1].as_f64().unwrap()],
    ];
    let mut sketch = Sketch::on_world_plane(SketchId(id(10)), "Cover left bevel", WorldPlane::XY);
    for (i, p) in points.iter().enumerate() {
        sketch
            .add_entity(SketchEntity::point(
                EntityId(id(1000 + i as u128)),
                Vec2::new(p[0], p[1]).unwrap(),
                false,
                false,
            ))
            .unwrap();
    }
    for i in 0..3 {
        sketch
            .add_entity(SketchEntity::line(
                EntityId(id(1010 + i as u128)),
                EntityId(id(1000 + i as u128)),
                EntityId(id(1000 + ((i + 1) % 3) as u128)),
                false,
            ))
            .unwrap();
    }
    sketch
}

pub(super) async fn bevel_profile(h: &Harness, face: &serde_json::Value) -> SketchRegionRef {
    let sketch = bevel_sketch(face);
    let worker = h.state.live_worker().unwrap();
    worker.sketch_upsert(&sketch).await.unwrap();
    let regions = worker.sketch_regions(&sketch.id.to_string()).await.unwrap();
    assert_eq!(
        regions.regions.len(),
        1,
        "bevel triangle must have one region"
    );
    let profile = SketchRegionRef {
        sketch: sketch.id,
        region: RegionId::new(&regions.regions[0].region_id),
        region_identity_version: Some(regions.region_identity_version),
        region_anchor: None,
        extra: Default::default(),
    };
    let z = face["wires"][0][0]["start"][2].as_f64().unwrap();
    let plane = SketchPlaneRef {
        kind: PlaneKind::Custom,
        origin: Vec3::new(0., 0., z).unwrap(),
        x_axis: Vec3::new(1., 0., 0.).unwrap(),
        y_axis: Vec3::new(0., 1., 0.).unwrap(),
        normal: Vec3::new(0., 0., 1.).unwrap(),
        extra: Default::default(),
    };
    let (_, entities, constraints) = sketch_wire(&sketch);
    add(
        h,
        11,
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sketch.id,
            plane,
            entities: entities.as_array().unwrap().clone(),
            constraints: constraints.as_array().unwrap().clone(),
            host_face: None,
            extra: Default::default(),
        })),
    )
    .await;
    profile
}

pub(super) fn cut_polygon(n: u128, points: &[[f64; 2]]) -> Sketch {
    let mut sketch =
        Sketch::on_world_plane(SketchId(id(n)), "Cover analytic profile", WorldPlane::XY);
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

pub(super) fn frame(
    origin: [f64; 3],
    x: [f64; 3],
    y: [f64; 3],
    normal: [f64; 3],
) -> SketchPlaneRef {
    let v = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]).unwrap();
    SketchPlaneRef {
        kind: PlaneKind::Custom,
        origin: v(origin),
        x_axis: v(x),
        y_axis: v(y),
        normal: v(normal),
        extra: Default::default(),
    }
}

pub(super) async fn profile_from_points(
    h: &Harness,
    n: u128,
    points: &[[f64; 2]],
    plane: SketchPlaneRef,
) -> SketchRegionRef {
    let sketch = cut_polygon(n, points);
    let worker = h.state.live_worker().unwrap();
    worker.sketch_upsert(&sketch).await.unwrap();
    let regions = worker.sketch_regions(&sketch.id.to_string()).await.unwrap();
    assert_eq!(
        regions.regions.len(),
        1,
        "source-derived profile must yield one region"
    );
    let profile = SketchRegionRef {
        sketch: sketch.id,
        region: RegionId::new(&regions.regions[0].region_id),
        region_identity_version: Some(regions.region_identity_version),
        region_anchor: None,
        extra: Default::default(),
    };
    let (_, entities, constraints) = sketch_wire(&sketch);
    add(
        h,
        n + 1,
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sketch.id,
            plane,
            entities: entities.as_array().unwrap().clone(),
            constraints: constraints.as_array().unwrap().clone(),
            host_face: None,
            extra: Default::default(),
        })),
    )
    .await;
    profile
}

pub(super) async fn apply_prism(
    h: &Harness,
    n: u128,
    profile: SketchRegionRef,
    mode: BooleanMode,
    distance: f64,
) {
    let mut op = match stock(profile) {
        Operation::Known(KnownOperation::Extrude(params)) => params,
        _ => unreachable!(),
    };
    op.distance = Scalar::new(distance);
    op.boolean_mode = mode;
    op.target_body = Some(BodyId(id(3)));
    add(h, n, Operation::Known(KnownOperation::Extrude(op))).await;
}

async fn side_bevel(h: &Harness, face: &serde_json::Value, n: u128, z: f64, length: f64) {
    let edge = &face["wires"][0][0];
    let a = &edge["start"];
    let b = &edge["end"];
    let p = |v: &serde_json::Value| [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()];
    let (a, b) = (p(a), p(b));
    let (high, low) = if a[1] > b[1] { (a, b) } else { (b, a) };
    let points = [high, low, [low[0], high[1]]];
    let plane = frame([0., 0., z], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]);
    let profile = profile_from_points(h, n, &points, plane).await;
    apply_prism(h, n + 2, profile, BooleanMode::Cut, length).await;
}

async fn top_bevel(h: &Harness, face: &serde_json::Value) {
    let edge = &face["wires"][0][0];
    let low = &edge["start"];
    let high = &edge["end"];
    let x = low[0].as_f64().unwrap();
    let yl = low[1].as_f64().unwrap();
    let zl = low[2].as_f64().unwrap();
    let yh = high[1].as_f64().unwrap();
    let zh = high[2].as_f64().unwrap();
    let profile = profile_from_points(
        h,
        40,
        &[[yl, zl], [yh, zh], [yh, zl]],
        frame([x, 0., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 0.]),
    )
    .await;
    apply_prism(h, 42, profile, BooleanMode::Cut, 49.0).await;
}

async fn append_tab(h: &Harness, extraction: &serde_json::Value) {
    let face = &extraction["faces"][45];
    let bounds = face["wires"][0].as_array().unwrap();
    let points: Vec<[f64; 2]> = bounds
        .iter()
        .map(|edge| {
            [
                edge["start"][0].as_f64().unwrap(),
                edge["start"][2].as_f64().unwrap(),
            ]
        })
        .collect();
    assert_eq!(points.len(), 4);
    let y = extraction["faces"][46]["origin"][1].as_f64().unwrap();
    let profile = profile_from_points(
        h,
        50,
        &points,
        frame([0., y, 0.], [1., 0., 0.], [0., 0., 1.], [0., -1., 0.]),
    )
    .await;
    apply_prism(h, 52, profile, BooleanMode::Add, 4.5).await;
}

pub(super) async fn remaining_perimeter_and_tab(h: &Harness, extraction: &serde_json::Value) {
    let faces = &extraction["faces"];
    let z = faces[5]["wires"][0][2]["start"][2].as_f64().unwrap();
    side_bevel(h, &faces[5], 20, z, 10.0).await;
    let z = faces[7]["wires"][0][1]["end"][2].as_f64().unwrap();
    side_bevel(h, &faces[7], 30, z, 9.0).await;
    top_bevel(h, &faces[6]).await;
    let before = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    append_tab(h, extraction).await;
    let after = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    assert!(
        (after - before - 63.0).abs() < 1e-4,
        "tab volume {}",
        after - before
    );
    eprintln!(
        "incomplete perimeter+tab volume {after}; tab contributes {} mm³",
        after - before
    );
}
