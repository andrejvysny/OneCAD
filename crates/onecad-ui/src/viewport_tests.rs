use super::*;

#[test]
fn physical_target_rounding_suspends_subpixel_dimensions() {
    let viewport = Viewport {
        width: 0.2,
        height: 600.0,
        scale_factor: 1.25,
    };
    assert_eq!(physical_size(viewport), [0, 750]);
    assert_eq!(
        physical_size(Viewport {
            width: 800.0,
            ..viewport
        }),
        [1000, 750]
    );
}

#[test]
fn drawable_capture_retains_rendered_camera_until_next_submission() {
    let scene = onecad_scene::SceneIdentity {
        session: 1,
        scene: 1,
    };
    let viewport = Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 1.25,
    };
    let camera = onecad_scene::Camera::default();
    let view = ViewSnapshot::new(scene, 1, camera, viewport).unwrap();
    let mut pending = camera;
    pending.roll_radians = 0.5;
    let drawable = drawable_capture(Some(view), scene, viewport).unwrap();
    assert_eq!(drawable.camera(), camera);
    assert_ne!(drawable.camera(), pending);
    assert_eq!(drawable.identity(), view.identity());
}

#[test]
fn stale_scene_resize_and_monitor_scale_reject_drawable_actions() {
    let scene = onecad_scene::SceneIdentity {
        session: 1,
        scene: 1,
    };
    let viewport = Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 1.25,
    };
    let view = ViewSnapshot::new(scene, 1, onecad_scene::Camera::default(), viewport).unwrap();
    assert!(
        drawable_capture(
            Some(view),
            onecad_scene::SceneIdentity {
                session: 2,
                ..scene
            },
            viewport
        )
        .is_none()
    );
    for next in [
        Viewport {
            width: 801.0,
            ..viewport
        },
        Viewport {
            scale_factor: 2.0,
            ..viewport
        },
    ] {
        assert!(drawable_capture(Some(view), scene, next).is_none());
    }
    assert!(drawable_capture(None, scene, viewport).is_none());
}
