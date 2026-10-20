//! Independent analytic checks complement regeneration's publication validation.
use super::*;
use onecad_scene::{MeshLimits, MeshView};
use std::{collections::BTreeSet, f64::consts::PI};

struct Shape {
    area: f64,
    perimeter: f64,
    centroid: Vec2,
    lower: Vec2,
    upper: Vec2,
}

fn expected(sketch: &Sketch, name: &str) -> Shape {
    if name.starts_with("slot") {
        let (a, r, _, _) = arc(sketch, ids(1000).entities[8]);
        let (b, _, _, _) = arc(sketch, ids(1000).entities[9]);
        let length = geom::dist(a, b);
        if name == "slot-dimensioned" {
            near(length, 30.0, 1e-7);
            near(r, 6.0, 1e-7);
            near(a[0], 30.0, 1e-7);
            near(a[1], 0.0, 1e-7);
            near(b[0], 0.0, 1e-7);
            near(b[1], 0.0, 1e-7);
        }
        Shape {
            area: 2.0 * r * length + PI * r * r,
            perimeter: 2.0 * length + 2.0 * PI * r,
            centroid: xy((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0),
            lower: xy(a[0].min(b[0]) - r, a[1].min(b[1]) - r),
            upper: xy(a[0].max(b[0]) + r, a[1].max(b[1]) + r),
        }
    } else {
        assert_chain(sketch);
        let area = 400.0 + 50.0 * PI;
        Shape {
            area,
            perimeter: 60.0 + 10.0 * PI,
            centroid: xy(
                (4000.0 + 50.0 * PI * (20.0 + 40.0 / (3.0 * PI))) / area,
                10.0,
            ),
            lower: xy(0.0, 0.0),
            upper: xy(30.0, 20.0),
        }
    }
}

pub fn assert_profile(result: &FinishSketchDto, sketch: &Sketch) {
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.regions.len(), 1);
    let region = &result.regions[0];
    assert!(region.holes.is_empty());
    let expected: BTreeSet<_> = sketch
        .entities()
        .iter()
        .filter_map(|entity| match entity {
            SketchEntity::Line {
                id,
                construction: false,
                ..
            }
            | SketchEntity::Arc {
                id,
                construction: false,
                ..
            } => Some(id.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(expected.len(), 4);
    assert_eq!(
        region.outer_loop.iter().cloned().collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(region.outer_loop.len(), 4);
    for constraint in sketch.constraints() {
        if let Constraint::Coincident {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } = constraint
        {
            assert!(
                geom::dist(
                    geom::endpoint(sketch, *point1, *point1_position),
                    geom::endpoint(sketch, *point2, *point2_position)
                ) < 1e-6
            );
        }
    }
}

pub async fn assert_solid(
    harness: &Harness,
    projection: &DocumentProjection,
    sketch: &Sketch,
    name: &str,
) {
    assert_eq!(projection.bodies.len(), 1, "one connected body");
    assert_eq!(projection.features.len(), 2);
    let extrusion = projection
        .features
        .iter()
        .find(|feature| feature.id == extrude_id().to_string())
        .unwrap();
    assert_eq!(extrusion.primary_value, Some(4.0));
    let body = projection.bodies.keys().next().unwrap();
    let wire = onecad_app::worker::wire::body_id_wire(body.parse().unwrap());
    let topology = api::query_body_topology(&harness.state, wire.clone())
        .await
        .unwrap();
    assert_eq!(topology.body_id, wire);
    assert_eq!(topology.solid_count, 1);
    assert_eq!(topology.face_count, 6);
    let mass = api::query_mass_properties(&harness.state, wire.clone(), None)
        .await
        .unwrap();
    assert_eq!(mass.body_id, wire);
    let shape = expected(sketch, name);
    assert_mass(&mass, sketch, &shape);
    let blob = api::get_mesh(
        &harness.state,
        body.clone(),
        "fine".into(),
        None,
        Some(projection.runtime_session.clone()),
    )
    .await
    .unwrap();
    assert_bounds(&blob, sketch, shape);
}

fn assert_bounds(blob: &[u8], sketch: &Sketch, shape: Shape) {
    let mesh = MeshView::decode(
        blob,
        MeshLimits {
            max_triangles: 500_000,
            ..MeshLimits::default()
        },
    )
    .unwrap();
    assert_eq!(mesh.header().face_count, 6);
    assert!(mesh.header().triangle_count > 0);
    let mut lower = [f64::INFINITY; 3];
    let mut upper = [f64::NEG_INFINITY; 3];
    for index in 0..mesh.header().vertex_count as usize {
        for (axis, coordinate) in mesh.position(index).unwrap().into_iter().enumerate() {
            lower[axis] = lower[axis].min(f64::from(coordinate));
            upper[axis] = upper[axis].max(f64::from(coordinate));
        }
    }
    let plane = sketch.effective_plane();
    let a = plane.to_world(shape.lower);
    let b = plane.to_world(shape.upper);
    let first = [a.x, a.y, a.z];
    let last = [b.x, b.y, b.z + 4.0];
    for axis in 0..3 {
        // Curved tessellation need not place a vertex exactly at every analytic extremum.
        near(lower[axis], first[axis].min(last[axis]), 0.01);
        near(upper[axis], first[axis].max(last[axis]), 0.01);
        // OCCT BRep bounds include the 1e-4 modeling tolerance plus f32 rounding.
        near(
            f64::from(mesh.header().bbox_min[axis]),
            first[axis].min(last[axis]),
            2e-4,
        );
        near(
            f64::from(mesh.header().bbox_max[axis]),
            first[axis].max(last[axis]),
            2e-4,
        );
    }
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} within {tolerance}"
    );
}

fn assert_mass(mass: &onecad_app::dto::MassPropertiesDto, sketch: &Sketch, shape: &Shape) {
    near(mass.volume, shape.area * 4.0, 1e-5);
    near(
        mass.surface_area,
        2.0 * shape.area + shape.perimeter * 4.0,
        1e-5,
    );
    let plane = sketch.effective_plane();
    let center = plane.to_world(shape.centroid);
    for (actual, expected) in mass.centroid.into_iter().zip([
        center.x + plane.normal.x * 2.0,
        center.y + plane.normal.y * 2.0,
        center.z + plane.normal.z * 2.0,
    ]) {
        near(actual, expected, 1e-6);
    }
}
