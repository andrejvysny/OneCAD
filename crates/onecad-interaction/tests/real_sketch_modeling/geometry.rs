use super::{extrude_id, host::Harness};
use onecad_app::{api, dto::DocumentProjection};
use onecad_scene::{MeshLimits, MeshView};

pub(super) async fn assert_solid(harness: &Harness, projection: &DocumentProjection, depth: f64) {
    assert_eq!(projection.bodies.len(), 1);
    let body = projection.bodies.keys().next().unwrap();
    assert_eq!(body, &extrude_id().to_string());
    let wire = onecad_app::worker::wire::body_id_wire(body.parse().unwrap());
    let topology = api::query_body_topology(&harness.state, wire.clone())
        .await
        .unwrap();
    assert_eq!((topology.solid_count, topology.face_count), (1, 6));
    let mass = api::query_mass_properties(&harness.state, wire, None)
        .await
        .unwrap();
    assert!((mass.volume - 30.0 * 20.0 * depth).abs() < 1e-6);
    let blob = api::get_mesh(
        &harness.state,
        body.clone(),
        "fine".into(),
        None,
        Some(projection.runtime_session.clone()),
    )
    .await
    .unwrap();
    let mesh = MeshView::decode(&blob, MeshLimits::default()).unwrap();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for index in 0..mesh.header().vertex_count as usize {
        let point = mesh.position(index).unwrap();
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    for axis in 0..3 {
        assert!((min[axis] - [-20.0, 0.0, 0.0][axis]).abs() < 1e-5);
        assert!((max[axis] - [0.0, 30.0, depth as f32][axis]).abs() < 1e-5);
    }
}
