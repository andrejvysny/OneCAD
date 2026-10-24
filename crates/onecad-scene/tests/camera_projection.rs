mod support;
use onecad_scene::*;
use support::*;

#[test]
fn captured_projection_matches_uncached_matrix_and_pick_in_both_projections() {
    let scene = scene(
        &[[-4.0, 0.0, -4.0], [4.0, 0.0, -4.0], [0.0, 0.0, 4.0]],
        &[[0, 1, 2]],
        &[],
    );
    for projection in [Projection::Perspective, Projection::Orthographic] {
        for roll_radians in [0.0, 0.73, std::f64::consts::FRAC_PI_2] {
            for scale_factor in [0.75, 2.0] {
                let viewport = Viewport {
                    width: 801.25,
                    height: 603.75,
                    scale_factor,
                };
                let camera = Camera {
                    roll_radians,
                    ..camera(projection)
                };
                let capture = ViewSnapshot::new(identity(), 13, camera, viewport).unwrap();
                let uncached = capture.projection_matrix() * capture.view_matrix();
                assert_eq!(capture.view_projection(), uncached);
                let copied = capture;
                assert_eq!(copied.identity(), capture.identity());
                assert_eq!(copied.view_projection(), uncached);
                for point in [DVec3::ZERO, DVec3::new(0.5, 0.0, -0.5)] {
                    assert_projected_pick(&scene, &capture, uncached, point);
                }
            }
        }
    }
}

fn assert_projected_pick(scene: &SceneSnapshot, view: &ViewSnapshot, matrix: DMat4, point: DVec3) {
    let clip = matrix * point.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    let viewport = view.viewport();
    let logical = DVec2::new(
        (ndc.x + 1.0) * viewport.width * 0.5,
        (1.0 - ndc.y) * viewport.height * 0.5,
    );
    assert_eq!(view.project(point).unwrap().logical, logical);
    for picker in [pick, pick_brute_force] {
        let hit = picker(scene, view, view.identity(), logical)
            .unwrap()
            .unwrap();
        assert_eq!(hit.topology_id, "f:1");
        assert_eq!(hit.capture, view.identity());
        assert!((hit.world_position - point).length() < 1e-9);
    }
}

#[test]
fn new_camera_or_viewport_captures_recompute_projection_without_changing_identity_rules() {
    let a = view(Projection::Perspective, 1.0);
    for (camera, viewport) in [
        (
            Camera {
                roll_radians: 0.4,
                ..a.camera()
            },
            a.viewport(),
        ),
        (
            Camera {
                target: DVec3::X,
                ..a.camera()
            },
            a.viewport(),
        ),
        (
            a.camera(),
            Viewport {
                width: 900.0,
                ..a.viewport()
            },
        ),
    ] {
        let b = ViewSnapshot::new(identity(), a.identity().view, camera, viewport).unwrap();
        assert_ne!(a.identity(), b.identity());
        assert_ne!(a.view_projection(), b.view_projection());
        assert_eq!(b.view_projection(), b.projection_matrix() * b.view_matrix());
    }
}
