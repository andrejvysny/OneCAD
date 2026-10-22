use super::profile::{HEIGHT, WIDTH};
use onecad_app::{api, dto::DocumentProjection};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use std::collections::{BTreeMap, BTreeSet};

pub async fn assert_plate(
    harness: &super::Harness,
    projection: &DocumentProjection,
    depth: f64,
) -> Vec<u8> {
    assert_eq!(
        projection.bodies.len(),
        1,
        "one connected solid body required"
    );
    assert_eq!(projection.features.len(), 2);
    let body = projection.bodies.keys().next().unwrap();
    let wire_body =
        onecad_app::worker::wire::body_id_wire(body.parse::<onecad_core::ids::BodyId>().unwrap());
    let topology = api::query_body_topology(&harness.state, wire_body.clone())
        .await
        .unwrap();
    assert_eq!(topology.solid_count, 1);
    assert_eq!(topology.face_count, 6);
    let mass = api::query_mass_properties(&harness.state, wire_body, None)
        .await
        .unwrap();
    assert_mass(&mass, depth);
    let blob = api::get_mesh(
        &harness.state,
        body.clone(),
        "fine".into(),
        None,
        Some(projection.runtime_session.clone()),
    )
    .await
    .unwrap();
    assert_mesh(&blob, depth);
    let feature = projection
        .features
        .iter()
        .find(|feature| feature.id == super::profile::extrude_id().to_string())
        .unwrap();
    assert_eq!(feature.primary_value, Some(depth));
    blob
}

fn assert_mass(mass: &onecad_app::dto::MassPropertiesDto, depth: f64) {
    near(mass.volume, WIDTH * HEIGHT * depth, 1e-6);
    near(
        mass.surface_area,
        2.0 * (WIDTH * HEIGHT + (WIDTH + HEIGHT) * depth),
        1e-6,
    );
    for (actual, expected) in
        mass.centroid
            .into_iter()
            .zip([-HEIGHT / 2.0, WIDTH / 2.0, depth / 2.0])
    {
        near(actual, expected, 1e-6);
    }
}

pub fn assert_mesh(blob: &[u8], depth: f64) {
    let mesh = MeshView::decode(blob, MeshLimits::default()).unwrap();
    assert_eq!(mesh.header().face_count, 6);
    assert!(mesh.header().triangle_count > 0);
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for index in 0..mesh.header().vertex_count as usize {
        let point = mesh.position(index).unwrap();
        for axis in 0..3 {
            min[axis] = min[axis].min(f64::from(point[axis]));
            max[axis] = max[axis].max(f64::from(point[axis]));
        }
    }
    // Named XY maps local X to world +Y and local Y to world -X (Sketch.h).
    for (axis, expected) in [HEIGHT, WIDTH, depth].into_iter().enumerate() {
        near(min[axis], [-HEIGHT, 0.0, 0.0][axis], 1e-5);
        near(max[axis] - min[axis], expected, 1e-5);
    }
    closed_connected(&mesh);
}

type Point = [i64; 3];
fn key(point: [f32; 3]) -> Point {
    point.map(|value| (f64::from(value) * 100_000.0).round() as i64)
}

fn closed_connected(mesh: &MeshView<'_>) {
    let mut edges = BTreeMap::new();
    let mut adjacent: BTreeMap<Point, BTreeSet<Point>> = BTreeMap::new();
    for triangle in 0..mesh.header().triangle_count as usize {
        let vertices = mesh
            .triangle(triangle)
            .unwrap()
            .map(|index| key(mesh.position(index as usize).unwrap()));
        for side in 0..3 {
            let (a, b) = (vertices[side], vertices[(side + 1) % 3]);
            assert_ne!(a, b, "no collapsed triangle edge");
            let edge = if a < b { (a, b) } else { (b, a) };
            *edges.entry(edge).or_insert(0) += 1;
            adjacent.entry(a).or_default().insert(b);
            adjacent.entry(b).or_default().insert(a);
        }
    }
    assert!(
        edges.values().all(|&count| count == 2),
        "closed triangulated manifold"
    );
    let mut pending = vec![*adjacent.keys().next().unwrap()];
    let mut visited = BTreeSet::new();
    while let Some(point) = pending.pop() {
        if visited.insert(point) {
            pending.extend(adjacent[&point].iter().copied());
        }
    }
    assert_eq!(
        visited.len(),
        adjacent.len(),
        "all triangles belong to one connected shell"
    );
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} within {tolerance}"
    );
}
