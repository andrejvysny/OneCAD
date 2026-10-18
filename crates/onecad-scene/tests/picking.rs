mod support;
use onecad_scene::*;
use support::*;
const FACE: [[f32; 3]; 3] = [
    [-100.0, 0.0, -100.0],
    [100.0, 0.0, -100.0],
    [0.0, 0.0, 100.0],
];
const CENTER: DVec2 = DVec2::new(400.0, 300.0);

#[test]
fn ray_triangle_resolves_snapshot_scoped_face_identity() {
    let s = scene(&FACE, &[[0, 1, 2]], &[]);
    let v = view(Projection::Perspective, 1.0);
    let hit = pick(&s, &v, v.identity(), CENTER).unwrap().unwrap();
    assert_eq!(hit.kind, PickKind::Face);
    assert_eq!(hit.topology_id, "f:1");
    assert_eq!(hit.mesh_revision, 3);
    assert_eq!(hit.capture, v.identity());
    assert!((hit.world_position - DVec3::ZERO).length() < 1e-12);
    assert!((hit.ray_distance - 10.0).abs() < 1e-12);
}

#[test]
fn edge_aperture_is_eight_logical_pixels_at_both_scales_and_projections() {
    let edge = vec![[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
    let s = scene(&[], &[], &[edge]);
    for projection in [Projection::Perspective, Projection::Orthographic] {
        for scale in [1.0, 2.0] {
            let v = view(projection, scale);
            for (pixels, expected) in [(7.99, true), (8.01, false)] {
                assert_eq!(
                    pick(&s, &v, v.identity(), CENTER + DVec2::Y * pixels)
                        .unwrap()
                        .is_some(),
                    expected
                );
            }
        }
    }
}

#[test]
fn nearest_face_occludes_edges_only_beyond_two_pixel_depth_slack() {
    for depth in [2.0, 10.0, 100.0] {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let v = view(projection, 2.0);
            let face: Vec<_> = FACE
                .iter()
                .map(|p| [p[0], (depth - 10.0) as f32, p[2]])
                .collect();
            let pixel = v.world_per_pixel(depth);
            for (behind, expected) in [(1.9, PickKind::Edge), (2.1, PickKind::Face)] {
                let y = (depth - 10.0 + behind * pixel) as f32;
                let s = scene(&face, &[[0, 1, 2]], &[vec![[-1.0, y, 0.0], [1.0, y, 0.0]]]);
                assert_eq!(
                    pick(&s, &v, v.identity(), CENTER).unwrap().unwrap().kind,
                    expected,
                    "{depth} {projection:?} {behind}"
                );
            }
        }
    }
}

#[test]
fn perspective_edge_anchor_uses_reciprocal_depth_interpolation() {
    let v = view(Projection::Perspective, 1.0);
    let s = scene(&[], &[], &[vec![[-0.5, -8.0, 0.0], [2.0, -2.0, 0.0]]]);
    let hit = pick(&s, &v, v.identity(), CENTER).unwrap().unwrap();
    assert!(hit.world_position.x.abs() < 1e-12);
    assert!((hit.depth - 3.2).abs() < 1e-12);
}

#[test]
fn near_and_far_clipping_preserve_crossing_segments_reject_invisible_edges() {
    let v = view(Projection::Perspective, 1.0);
    let s = scene(&[], &[], &[vec![[-1.0, -9.95, 0.0], [1.0, -8.0, 0.0]]]);
    let hit = pick(&s, &v, v.identity(), CENTER).unwrap().unwrap();
    assert!(hit.world_position.x.abs() < 1e-9);
    assert!((hit.depth - 1.025).abs() < 1e-6);
    for edge in [
        vec![[-1.0, -11.0, 0.0], [1.0, -10.5, 0.0]],
        vec![[-1.0, 100_001.0, 0.0], [1.0, 100_002.0, 0.0]],
    ] {
        let s = scene(&[], &[], &[edge]);
        assert!(pick(&s, &v, v.identity(), CENTER).unwrap().is_none());
    }
    let c = Camera {
        far: 20.0,
        ..v.camera()
    };
    let v = ViewSnapshot::new(identity(), 11, c, v.viewport()).unwrap();
    let s = scene(&[], &[], &[vec![[-1.0, 0.0, 0.0], [1.0, 20.0, 0.0]]]);
    assert!(pick(&s, &v, v.identity(), CENTER).unwrap().is_some());
}

#[test]
fn mismatch_in_any_capture_component_is_explicit() {
    let s = scene(&FACE, &[[0, 1, 2]], &[]);
    let v = view(Projection::Perspective, 1.0);
    let other_scale = view(Projection::Perspective, 2.0);
    let other_projection = view(Projection::Orthographic, 1.0);
    let other_view = ViewSnapshot::new(identity(), 12, v.camera(), v.viewport()).unwrap();
    let other_camera = ViewSnapshot::new(
        identity(),
        11,
        Camera {
            near: 0.2,
            ..v.camera()
        },
        v.viewport(),
    )
    .unwrap();
    let other_session = ViewSnapshot::new(
        SceneIdentity {
            session: 8,
            ..identity()
        },
        11,
        v.camera(),
        v.viewport(),
    )
    .unwrap();
    for capture in [
        other_scale.identity(),
        other_projection.identity(),
        other_view.identity(),
        other_camera.identity(),
        other_session.identity(),
    ] {
        assert!(matches!(
            pick(&s, &v, capture, CENTER),
            Err(SceneError::CaptureMismatch)
        ));
    }
    assert!(matches!(
        pick(&s, &other_session, other_session.identity(), CENTER),
        Err(SceneError::CaptureMismatch)
    ));
}

#[test]
fn faces_outside_clip_planes_never_pick() {
    let v = view(Projection::Perspective, 1.0);
    for y in [-9.95, 100_001.0] {
        let vertices: Vec<_> = FACE.iter().map(|p| [p[0], y, p[2]]).collect();
        let s = scene(&vertices, &[[0, 1, 2]], &[]);
        assert!(pick(&s, &v, v.identity(), CENTER).unwrap().is_none());
    }
}

#[test]
fn front_body_occludes_edges_on_other_bodies_independent_of_body_order() {
    let v = view(Projection::Perspective, 1.0);
    for reversed in [false, true] {
        let front = body(&FACE, &[[0, 1, 2]], &[]);
        let hidden = body(&[], &[], &[vec![[-1.0, 1.0, 0.0], [1.0, 1.0, 0.0]]]);
        let mut bodies = vec![front, hidden];
        if reversed {
            bodies.reverse();
        }
        let s = SceneSnapshot::new(identity(), bodies, SceneLimits::default()).unwrap();
        assert_eq!(
            pick(&s, &v, v.identity(), CENTER).unwrap().unwrap().kind,
            PickKind::Face
        );
    }
}

#[test]
fn equal_depth_hits_use_stable_body_identity_not_insertion_order() {
    let lower: BodyId = "00000000-0000-0000-0000-000000000001".parse().unwrap();
    let higher: BodyId = "00000000-0000-0000-0000-000000000002".parse().unwrap();
    let bytes: std::sync::Arc<[u8]> = std::sync::Arc::from(blob(&FACE, &[[0, 1, 2]], &[]));
    let v = view(Projection::Perspective, 1.0);
    for order in [[lower, higher], [higher, lower]] {
        let bodies = order
            .into_iter()
            .map(|id| SceneBody::new(id, 1, bytes.clone(), MeshLimits::default()).unwrap())
            .collect();
        let s = SceneSnapshot::new(identity(), bodies, SceneLimits::default()).unwrap();
        assert_eq!(
            pick(&s, &v, v.identity(), CENTER).unwrap().unwrap().body_id,
            lower
        );
    }
}

#[test]
fn edge_projected_to_point_uses_nearest_endpoint_in_either_order() {
    let v = view(Projection::Perspective, 1.0);
    for edge in [
        vec![[0.0, 10.0, 0.0], [0.0, -9.0, 0.0]],
        vec![[0.0, -9.0, 0.0], [0.0, 10.0, 0.0]],
    ] {
        let s = scene(&FACE, &[[0, 1, 2]], &[edge]);
        let hit = pick(&s, &v, v.identity(), CENTER).unwrap().unwrap();
        assert_eq!(hit.kind, PickKind::Edge);
        assert!((hit.depth - 1.0).abs() < 1e-9);
    }
}
