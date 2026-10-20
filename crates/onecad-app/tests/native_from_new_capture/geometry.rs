use super::*;
use onecad_app::dto::DocumentProjection;
use onecad_core::{math::Vec2, sketch::Sketch};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use std::f64::consts::PI;

pub async fn solid(
    harness: &Harness,
    projection: &DocumentProjection,
    sketch: &Sketch,
    body: &str,
    width: f64,
) {
    let wire = onecad_app::worker::wire::body_id_wire(body.parse().unwrap());
    let topology = api::query_body_topology(&harness.state, wire.clone())
        .await
        .unwrap();
    assert_eq!(topology.body_id, wire);
    assert_eq!((topology.solid_count, topology.face_count), (1, 8));
    let mass = api::query_mass_properties(&harness.state, wire.clone(), None)
        .await
        .unwrap();
    assert_eq!(mass.body_id, wire);
    let area = width * 50.0 - 18.0 * PI;
    near(mass.volume, 4.0 * area);
    near(
        mass.surface_area,
        2.0 * area + 4.0 * (2.0 * (width + 50.0) + 12.0 * PI),
    );
    let plane = sketch.effective_plane();
    let center = plane.to_world(Vec2::new_unchecked(width / 2.0, 25.0));
    for (actual, expected) in mass.centroid.into_iter().zip([
        center.x + 2.0 * plane.normal.x,
        center.y + 2.0 * plane.normal.y,
        center.z + 2.0 * plane.normal.z,
    ]) {
        near(actual, expected);
    }
    let blob = api::get_mesh(
        &harness.state,
        body.into(),
        "fine".into(),
        None,
        Some(projection.runtime_session.clone()),
    )
    .await
    .unwrap();
    bounds(&blob, sketch, width);
}

fn bounds(blob: &[u8], sketch: &Sketch, width: f64) {
    let mesh = MeshView::decode(
        blob,
        MeshLimits {
            max_triangles: 500_000,
            ..MeshLimits::default()
        },
    )
    .unwrap();
    assert_eq!(mesh.header().face_count, 8);
    assert!(mesh.header().triangle_count > 0);
    let mut lower = [f64::INFINITY; 3];
    let mut upper = [f64::NEG_INFINITY; 3];
    for index in 0..mesh.header().vertex_count as usize {
        for (axis, value) in mesh.position(index).unwrap().into_iter().enumerate() {
            lower[axis] = lower[axis].min(f64::from(value));
            upper[axis] = upper[axis].max(f64::from(value));
        }
    }
    let plane = sketch.effective_plane();
    let mut expected_lower = [f64::INFINITY; 3];
    let mut expected_upper = [f64::NEG_INFINITY; 3];
    for u in [0.0, width] {
        for v in [0.0, 50.0] {
            for z in [0.0, 4.0] {
                let point = plane.to_world(Vec2::new_unchecked(u, v));
                for (axis, value) in [
                    point.x + z * plane.normal.x,
                    point.y + z * plane.normal.y,
                    point.z + z * plane.normal.z,
                ]
                .into_iter()
                .enumerate()
                {
                    expected_lower[axis] = expected_lower[axis].min(value);
                    expected_upper[axis] = expected_upper[axis].max(value);
                }
            }
        }
    }
    for axis in 0..3 {
        near(lower[axis], expected_lower[axis]);
        near(upper[axis], expected_upper[axis]);
        assert!(f64::from(mesh.header().bbox_min[axis]) <= lower[axis]);
        assert!(f64::from(mesh.header().bbox_max[axis]) >= upper[axis]);
        // OCCT header bounds include the publication modeling tolerance.
        assert!((f64::from(mesh.header().bbox_min[axis]) - expected_lower[axis]).abs() < 2e-4);
        assert!((f64::from(mesh.header().bbox_max[axis]) - expected_upper[axis]).abs() < 2e-4);
    }
}
