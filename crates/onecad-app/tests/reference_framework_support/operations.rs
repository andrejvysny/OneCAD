use super::*;

pub(crate) async fn add_sketch(
    h: &Harness,
    n: u128,
    wires: &Value,
    top: Option<f64>,
) -> onecad_app::dto::FinishSketchDto {
    let sid = sketch(n);
    let mut s = Sketch::on_world_plane(sid, "Framework enclosure profile", WorldPlane::XY);
    if let Some(height) = top {
        let datum = DatumPlaneId(Uuid::from_u128(0xfac0200 + n));
        api::apply_edit_command(
            &h.state,
            h.host.clone(),
            EditCommand::AddDatumPlane {
                datum: DatumPlane::offset_from_plane(datum, "Profile plane", "XY", height),
            },
        )
        .await
        .unwrap();
        s.attachment = SketchAttachment::Datum { datum };
    }
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddSketch { sketch: s },
    )
    .await
    .unwrap();
    api::enter_sketch(&h.state, h.host.clone(), sid.to_string(), Some(true))
        .await
        .unwrap();
    api::sketch_upsert(
        &h.state,
        h.host.clone(),
        sid.to_string(),
        super::entities(wires),
    )
    .await
    .unwrap();
    let finish = api::finish_sketch(&h.state, h.host.clone(), sid.to_string())
        .await
        .unwrap();
    h.settled(&h.projection().await).await;
    finish
}

pub(crate) async fn extrusion(
    h: &Harness,
    n: u128,
    profile: &onecad_app::dto::FinishSketchDto,
    depth: f64,
    target: Option<BodyId>,
) {
    let mut operation = native_plate_support::profile::extrude(profile, depth);
    let Operation::Known(KnownOperation::Extrude(params)) = &mut operation else {
        unreachable!()
    };
    params.profile.as_mut().unwrap().sketch = sketch(n);
    params.profile.as_mut().unwrap().region = RegionId::new(&profile.regions[0].region_id);
    if n == 8 {
        let angle =
            std::env::var("ONECAD_FRAMEWORK_UNDERSIDE_DRAFT").unwrap_or_else(|_| "-1".into());
        params.draft_angle_deg = Scalar::new(angle.parse().unwrap());
    }
    if let Some(body) = target {
        params.boolean_mode = BooleanMode::Cut;
        params.target_body = Some(body);
    }
    let result = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(record(n), 1, "Framework stock or pocket", operation),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&result.projection).await;
}

pub(crate) async fn ruled_ramp(h: &Harness, recipe: &Value, body: BodyId) {
    let lower = add_sketch(h, 80, &recipe["rearPocketFloorFace"]["wires"], Some(1.2)).await;
    let source = recipe["interiorFloorFace"]["wires"][0].as_array().unwrap();
    let mut outline = source[8..=12].to_vec();
    outline.push(json!({"kind":"line", "start":[-9.0,-30.5], "end":[9.0,-30.5]}));
    let upper = add_sketch(h, 81, &json!([outline]), Some(1.5)).await;
    let section = |n, finish: &onecad_app::dto::FinishSketchDto| SketchRegionRef {
        sketch: sketch(n),
        region: RegionId::new(&finish.regions[0].region_id),
        region_identity_version: Some(finish.region_identity_version),
        region_anchor: None,
        extra: Default::default(),
    };
    let loft = Operation::Known(KnownOperation::Loft(LoftParams {
        profiles: vec![section(80, &lower), section(81, &upper)],
        is_solid: true,
        is_ruled: true,
        boolean_mode: BooleanMode::NewBody,
        extra: Default::default(),
    }));
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(record(90), 1, "Framework ramp tool", loft),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
    let projection = h.projection().await;
    assert_eq!(projection.bodies.len(), 2);
    let tool = projection
        .bodies
        .keys()
        .map(|id| id.parse::<BodyId>().unwrap())
        .find(|id| *id != body)
        .unwrap();
    let cut = Operation::Known(KnownOperation::Boolean(BooleanParams::single(
        BooleanOp::Cut,
        body,
        tool,
    )));
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(record(91), 1, "Framework ramp cut", cut),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
}

pub(crate) async fn loft_clip_cut(
    h: &Harness,
    body: BodyId,
    recipe: &Value,
    side: usize,
    upper: bool,
    n: u128,
) {
    eprintln!("clip tool side={side} upper={upper}: begin");
    let inner = if side == 0 { -13.2 } else { 13.2 };
    let outer = if side == 0 { -16.0 } else { 16.0 };
    let sections = [inner, outer].map(|x| (x, super::clip_section(recipe, side, x, upper)));
    let rounding = std::env::var_os("ONECAD_FRAMEWORK_ROUND_CLIP_TOOLS").is_some();
    let rounded = rounding.then(|| super::clip_unfillet_recipe(recipe));
    loft_clip_section_cut_inner(h, body, sections, n, rounded.as_ref().map(|r| (r, side))).await;
}

pub(crate) async fn loft_clip_section_cut(
    h: &Harness,
    body: BodyId,
    sections: [(f64, [[f64; 2]; 4]); 2],
    n: u128,
) {
    loft_clip_section_cut_inner(h, body, sections, n, None).await;
}

async fn loft_clip_section_cut_inner(
    h: &Harness,
    body: BodyId,
    sections: [(f64, [[f64; 2]; 4]); 2],
    n: u128,
    rounded: Option<(&Value, usize)>,
) {
    let mut profiles = Vec::new();
    for (index, (offset, points)) in sections.into_iter().enumerate() {
        let sketch_number = n + index as u128;
        let finish = super::slot_profile(h, sketch_number, offset, WorldPlane::XZ, &points).await;
        assert_eq!(finish.regions.len(), 1);
        profiles.push(SketchRegionRef {
            sketch: sketch(sketch_number),
            region: RegionId::new(&finish.regions[0].region_id),
            region_identity_version: Some(finish.region_identity_version),
            region_anchor: None,
            extra: Default::default(),
        });
    }
    let op = Operation::Known(KnownOperation::Loft(LoftParams {
        profiles,
        is_solid: true,
        is_ruled: true,
        boolean_mode: BooleanMode::NewBody,
        extra: Default::default(),
    }));
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(record(n + 2), 1, "Framework unfilleted clip tool", op),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
    eprintln!("clip tool {n}: loft published");
    let projection = h.projection().await;
    assert_eq!(projection.bodies.len(), 2);
    let tool = projection
        .bodies
        .keys()
        .map(|id| id.parse::<BodyId>().unwrap())
        .find(|id| *id != body)
        .unwrap();
    if let Some((recipe, side)) = rounded {
        super::fillet_clip_edges(h, tool, recipe, side, n + 4, 3).await;
    }
    let cut = Operation::Known(KnownOperation::Boolean(BooleanParams::single(
        BooleanOp::Cut,
        body,
        tool,
    )));
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(record(n + 3), 1, "Framework unfilleted clip cut", cut),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
    eprintln!("clip tool {n}: Boolean published");
}

pub(crate) async fn cut_connector_end(h: &Harness, body: BodyId, n: u128, x: f64) {
    let (finish, axis) = super::connector_end_profile(h, n, x).await;
    assert_eq!(finish.regions.len(), 1);
    let fused = std::env::var_os("ONECAD_FRAMEWORK_ENDS_FUSED").is_some();
    let params = RevolveParams {
        profile: Some(SketchRegionRef {
            sketch: sketch(n),
            region: RegionId::new(&finish.regions[0].region_id),
            region_identity_version: Some(finish.region_identity_version),
            region_anchor: None,
            extra: Default::default(),
        }),
        angle_deg: Scalar::new(
            if std::env::var_os("ONECAD_FRAMEWORK_ENDS_FULL").is_some() {
                360.0
            } else {
                180.0 * x.signum()
            },
        ),
        axis: Some(AxisRef::SketchLine {
            sketch: sketch(n),
            line: axis,
            extra: Default::default(),
        }),
        boolean_mode: if fused {
            BooleanMode::Cut
        } else {
            BooleanMode::NewBody
        },
        target_body: fused.then_some(body),
        extra: Default::default(),
    };
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                record(n),
                1,
                "Framework connector end flare",
                Operation::Known(KnownOperation::Revolve(params)),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
    if fused {
        return;
    }
    let projection = h.projection().await;
    assert_eq!(projection.bodies.len(), 2);
    let tool = projection
        .bodies
        .keys()
        .map(|id| id.parse::<BodyId>().unwrap())
        .find(|id| *id != body)
        .unwrap();
    let cut = Operation::Known(KnownOperation::Boolean(BooleanParams::single(
        BooleanOp::Cut,
        body,
        tool,
    )));
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(record(n + 1000), 1, "Framework connector end cut", cut),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
}

pub(crate) async fn mesh_signature(h: &Harness, body: BodyId) -> ([u32; 6], u32, u32) {
    use onecad_protocol::mesh::{MeshLimits, MeshView};
    let projection = h.projection().await;
    let blob = api::get_mesh(
        &h.state,
        body.to_string(),
        "fine".into(),
        None,
        Some(projection.runtime_session),
    )
    .await
    .unwrap();
    let mesh = MeshView::decode(&blob, MeshLimits::default()).unwrap();
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for i in 0..mesh.header().vertex_count as usize {
        let p = mesh.position(i).unwrap();
        for axis in 0..3 {
            low[axis] = low[axis].min(p[axis]);
            high[axis] = high[axis].max(p[axis]);
        }
    }
    let bounds = [
        low[0].to_bits(),
        low[1].to_bits(),
        low[2].to_bits(),
        high[0].to_bits(),
        high[1].to_bits(),
        high[2].to_bits(),
    ];
    (
        bounds,
        mesh.header().vertex_count,
        mesh.header().triangle_count,
    )
}
