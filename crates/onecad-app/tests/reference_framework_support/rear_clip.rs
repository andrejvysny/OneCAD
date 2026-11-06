use super::*;

pub(crate) async fn rear_clip_cut(h: &Harness, body: BodyId, recipe: &Value, side: usize, n: u128) {
    let sign = if side == 0 { -1.0 } else { 1.0 };
    let sections =
        [13.2 * sign, 16.0 * sign].map(|x| (x, super::rear_clip_sections(recipe, side, x)));
    let triangle = sections.clone().map(|(x, (triangle, _))| (x, triangle));
    let limiter = sections.map(|(x, (_, limiter))| (x, limiter));
    let triangle = new_loft(h, triangle, n).await;
    let limiter = new_loft(h, limiter, n + 3).await;
    boolean(h, triangle, limiter, BooleanOp::Intersect, n + 6).await;
    boolean(h, body, triangle, BooleanOp::Cut, n + 7).await;
}

pub(crate) async fn rear_gap_cut(h: &Harness, body: BodyId, recipe: &Value, side: usize, n: u128) {
    let sign = if side == 0 { -1.0 } else { 1.0 };
    let xs = [13.2 * sign, 16.0 * sign];
    let mut tools = Vec::new();
    for index in 0..3 {
        let sections = xs.map(|x| (x, super::rear_gap_sections(recipe, side, x)[index].clone()));
        tools.push(new_loft(h, sections, n + 3 * index as u128).await);
    }
    boolean(h, tools[0], tools[1], BooleanOp::Intersect, n + 9).await;
    boolean(h, tools[0], tools[2], BooleanOp::Intersect, n + 10).await;
    boolean(h, body, tools[0], BooleanOp::Cut, n + 11).await;
}

pub(crate) async fn inboard_clip_cut(
    h: &Harness,
    body: BodyId,
    recipe: &Value,
    side: usize,
    n: u128,
) {
    let sign = if side == 0 { -1.0 } else { 1.0 };
    let sections =
        [12.0 * sign, 13.2 * sign].map(|x| (x, super::rear_clip_sections(recipe, side, x)));
    let triangle = sections.clone().map(|(x, (triangle, _))| (x, triangle));
    let limiter = sections.clone().map(|(x, (_, limiter))| (x, limiter));
    let triangle = new_loft(h, triangle, n).await;
    let limiter = new_loft(h, limiter, n + 3).await;
    boolean(h, triangle, limiter, BooleanOp::Intersect, n + 6).await;
    super::fillet_inboard_tool(h, triangle, &sections, n + 7).await;
    boolean(h, body, triangle, BooleanOp::Cut, n + 8).await;
}

async fn new_loft(h: &Harness, sections: [(f64, Vec<[f64; 2]>); 2], n: u128) -> BodyId {
    let before = h.projection().await.bodies;
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
    apply(h, n + 2, op).await;
    let after = h.projection().await.bodies;
    let new: Vec<_> = after
        .keys()
        .filter(|id| !before.contains_key(*id))
        .collect();
    assert_eq!(new.len(), 1);
    new[0].parse().unwrap()
}

async fn boolean(h: &Harness, target: BodyId, tool: BodyId, kind: BooleanOp, n: u128) {
    let op = Operation::Known(KnownOperation::Boolean(BooleanParams::single(
        kind, target, tool,
    )));
    apply(h, n, op).await;
}

async fn apply(h: &Harness, n: u128, op: Operation) {
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(record(n), 1, "Framework rear clip tool", op),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
}
