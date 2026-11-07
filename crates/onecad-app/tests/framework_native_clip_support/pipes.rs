//! Open-arc pipes retain source-derived frames and typed sweep parameters.
use super::*;
use serde_json::Value;

fn v3(v: &Value) -> [f64; 3] {
    [
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    ]
}

async fn pipe(b: &mut Builder<'_>, data: &Value) -> BodyId {
    let center = v3(&data["centerMm"]);
    let major = v3(&data["majorUnit"]);
    let minor = geometry::unit(v3(&data["minorUnit"]));
    let normal = v3(&data["normalUnit"]);
    let a = data["semiaxesMm"][0].as_f64().unwrap();
    let c = data["semiaxesMm"][1].as_f64().unwrap();
    let start = data["pathRangeRadians"][0].as_f64().unwrap();
    let end = data["pathRangeRadians"][1].as_f64().unwrap();
    let mut path = b.empty_sketch();
    let point = b.entity();
    let ellipse = b.entity();
    path.add_entity(SketchEntity::point(
        point,
        Vec2::new(0., 0.).unwrap(),
        false,
        false,
    ))
    .unwrap();
    path.add_entity(SketchEntity::ellipse(ellipse, point, a, c, 0., false).unwrap())
        .unwrap();
    let path_id = path.id;
    b.sketch(path, frame(center, major, minor, normal)).await;
    let section = b.circle(0.3);
    let profile_data = &data["profile"];
    let profile = b
        .sketch(
            section,
            frame(
                v3(&profile_data["centerMm"]),
                v3(&profile_data["xAxisUnit"]),
                v3(&profile_data["yAxisUnit"]),
                v3(&profile_data["normalUnit"]),
            ),
        )
        .await;
    b.created(
        "Editable source-intent open sweep",
        Operation::Known(KnownOperation::Sweep(SweepParams {
            profile: Some(profile),
            path_sketch: Some(path_id),
            path_edge: None,
            path_range: Some(SweepPathRange {
                start_radians: start,
                end_radians: end,
            }),
            boolean_mode: BooleanMode::NewBody,
            extra: Default::default(),
        })),
    )
    .await
}

pub async fn build(b: &mut Builder<'_>) -> Vec<BodyId> {
    let data: Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-histories/framework-open-arc-pipes.json"
    ))
    .unwrap();
    assert_eq!(
        data["sourceSha256"],
        "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09"
    );
    let pipes = data["pipes"].as_array().unwrap();
    assert_eq!(pipes.len(), 2);
    let mut bodies = Vec::new();
    for entry in pipes {
        bodies.push(pipe(b, entry).await);
    }
    bodies
}
