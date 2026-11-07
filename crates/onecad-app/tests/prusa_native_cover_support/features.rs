use super::*;

pub(super) async fn underside_recess(h: &Harness) {
    let data: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-derivatives/prusa/cover-native/recess-recipe.json"
    ))
    .unwrap();
    let before = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    let profile = create_profile(h, &data, 60).await;
    apply_prism(
        h,
        62,
        profile,
        BooleanMode::Cut,
        data["extrusionMm"].as_f64().unwrap(),
    )
    .await;
    let after = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    let expected = data["outerAreaMm2SameKernel"].as_f64().unwrap();
    assert!(
        (before - after - expected).abs() < 1e-4,
        "underside recess removed {} vs {expected}",
        before - after
    );
    eprintln!(
        "incomplete perimeter+tab+underside recess volume {after}; removed {} mm³",
        before - after
    );
}

pub(super) async fn hinge_cut(h: &Harness, extraction: &serde_json::Value) {
    let cylinder = &extraction["faces"][2];
    let cone = &extraction["faces"][0];
    let origin = &cylinder["origin"];
    let x = origin[0].as_f64().unwrap();
    let base_y = origin[1].as_f64().unwrap();
    let z = origin[2].as_f64().unwrap();
    let radius = cylinder["radius"].as_f64().unwrap();
    let transition = cylinder["vRange"][1].as_f64().unwrap();
    let top = 4.0;
    let slope = cone["semiAngleRadians"].as_f64().unwrap().tan();
    let mouth = radius + (top - transition) * slope;
    let points = [
        [0., 0.9],
        [radius, 0.9],
        [radius, transition],
        [mouth, top],
        [mouth + 0.1 * slope, top + 0.1],
        [0., top + 0.1],
    ];
    let profile = profile_from_points(
        h,
        70,
        &points,
        frame([x, base_y, z], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]),
    )
    .await;
    let axis = EntityId(id(70 * 100 + 50 + points.len() as u128 - 1));
    let params = RevolveParams {
        profile: Some(profile),
        angle_deg: Scalar::new(360.0),
        axis: Some(AxisRef::SketchLine {
            sketch: SketchId(id(70)),
            line: axis,
            extra: Default::default(),
        }),
        boolean_mode: BooleanMode::Cut,
        target_body: Some(BodyId(id(3))),
        extra: Default::default(),
    };
    let before = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    add(h, 72, Operation::Known(KnownOperation::Revolve(params))).await;
    let after = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    let expected = std::f64::consts::PI
        * (radius.powi(2) * (transition - 1.0)
            + (top - transition) * (radius * radius + radius * mouth + mouth * mouth) / 3.0);
    assert!(
        (before - after - expected).abs() < 1e-4,
        "hinge removed {} vs {expected}",
        before - after
    );
    eprintln!(
        "incomplete cover with hinge: {after} mm³; hinge removed {} mm³",
        before - after
    );
}

fn circle_sketch(n: u128, center: [f64; 2], radius: f64) -> Sketch {
    let mut sketch =
        Sketch::on_world_plane(SketchId(id(n)), "Cover fastener circle", WorldPlane::XY);
    let point = EntityId(id(n * 100));
    sketch
        .add_entity(SketchEntity::point(
            point,
            Vec2::new(center[0], center[1]).unwrap(),
            false,
            false,
        ))
        .unwrap();
    sketch
        .add_entity(SketchEntity::circle(EntityId(id(n * 100 + 1)), point, radius, false).unwrap())
        .unwrap();
    sketch
}

fn stripe_sketch(n: u128, center: [f64; 2], radius: f64, half_width: f64) -> Sketch {
    let mut sketch = Sketch::on_world_plane(
        SketchId(id(n)),
        "Cover fastener clipped circle",
        WorldPlane::XY,
    );
    let height = (radius * radius - half_width * half_width).sqrt();
    let points = [
        [center[0] - half_width, center[1] - height],
        [center[0] - half_width, center[1] + height],
        [center[0] + half_width, center[1] + height],
        [center[0] + half_width, center[1] - height],
        center,
    ];
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
    for (i, (a, b)) in [(0, 1), (2, 3)].iter().enumerate() {
        sketch
            .add_entity(SketchEntity::line(
                EntityId(id(n * 100 + 10 + i as u128)),
                EntityId(id(n * 100 + *a)),
                EntityId(id(n * 100 + *b)),
                false,
            ))
            .unwrap();
    }
    let theta = (half_width / radius).acos();
    for (i, (start, end)) in [
        (theta, std::f64::consts::PI - theta),
        (std::f64::consts::PI + theta, std::f64::consts::TAU - theta),
    ]
    .iter()
    .enumerate()
    {
        sketch
            .add_entity(
                SketchEntity::arc(
                    EntityId(id(n * 100 + 20 + i as u128)),
                    EntityId(id(n * 100 + 4)),
                    radius,
                    *start,
                    *end,
                    false,
                )
                .unwrap(),
            )
            .unwrap();
    }
    sketch
}

async fn source_sketch_profile(h: &Harness, n: u128, sketch: &Sketch, y: f64) -> SketchRegionRef {
    let worker = h.state.live_worker().unwrap();
    worker.sketch_upsert(sketch).await.unwrap();
    let regions = worker.sketch_regions(&sketch.id.to_string()).await.unwrap();
    assert_eq!(
        regions.regions.len(),
        1,
        "fastener cut profile must yield one region"
    );
    let profile = SketchRegionRef {
        sketch: sketch.id,
        region: RegionId::new(&regions.regions[0].region_id),
        region_identity_version: Some(regions.region_identity_version),
        region_anchor: None,
        extra: Default::default(),
    };
    let (_, entities, constraints) = sketch_wire(sketch);
    let plane = frame([0., y, 0.], [1., 0., 0.], [0., 0., 1.], [0., -1., 0.]);
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

pub(super) async fn fastener_site(
    h: &Harness,
    extraction: &serde_json::Value,
    outer_face: usize,
    inner_face: usize,
    n: u128,
) {
    let outer = &extraction["faces"][outer_face];
    let inner = &extraction["faces"][inner_face];
    let center = [
        outer["origin"][0].as_f64().unwrap(),
        outer["origin"][2].as_f64().unwrap(),
    ];
    let top_y = outer["origin"][1].as_f64().unwrap();
    let outer_radius = outer["radius"].as_f64().unwrap();
    let inner_radius = inner["radius"].as_f64().unwrap();
    let stages = [
        (circle_sketch(n, center, outer_radius), top_y, 2.3),
        (
            stripe_sketch(n + 10, center, outer_radius, inner_radius),
            top_y - 2.3,
            0.35,
        ),
        (
            cut_polygon(
                n + 20,
                &[
                    [center[0] - inner_radius, center[1] - inner_radius],
                    [center[0] + inner_radius, center[1] - inner_radius],
                    [center[0] + inner_radius, center[1] + inner_radius],
                    [center[0] - inner_radius, center[1] + inner_radius],
                ],
            ),
            top_y - 2.65,
            0.35,
        ),
        (
            circle_sketch(n + 30, center, inner_radius),
            top_y - 3.0,
            1.0,
        ),
    ];
    for (offset, (sketch, y, depth)) in stages.into_iter().enumerate() {
        let stage = n + offset as u128 * 10;
        eprintln!("fastener source face {outer_face}, stage {offset}");
        let profile = source_sketch_profile(h, stage, &sketch, y).await;
        apply_prism(h, stage + 2, profile, BooleanMode::Cut, depth).await;
    }
    let volume = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    eprintln!("cover after fastener face {outer_face}: {volume} mm³");
}
