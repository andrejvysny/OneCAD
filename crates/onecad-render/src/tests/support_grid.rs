use super::{
    quality_acceptance::{fixture_view, idle, render},
    *,
};

fn supports() -> [GridFrame; 2] {
    [
        GridFrame {
            origin: DVec3::new(10.0, -5.0, 12.0),
            x_axis: DVec3::Y,
            y_axis: DVec3::Z,
        },
        GridFrame {
            origin: DVec3::new(-7.0, 12.0, 18.0),
            x_axis: DVec3::Y,
            y_axis: DVec3::new(-0.8, 0.0, 0.6),
        },
    ]
}

#[test]
fn support_grid_and_visibility_preserve_world_contact_parameters() {
    let scene = fixture_scene(SceneIdentity {
        session: 67,
        scene: 1,
    });
    let view = fixture_view(&scene, Projection::Orthographic, 1.0);
    let baseline = crate::frame::uniform(
        &view,
        scene.bounds(),
        RenderStyle::default(),
        BodyStyle::default(),
        false,
    )
    .unwrap();
    for support_grid in supports().map(Some).into_iter().chain([None]) {
        for grid_visible in [false, true] {
            let style = RenderStyle {
                support_grid,
                grid_visible,
                ..Default::default()
            };
            let frame =
                crate::frame::uniform(&view, scene.bounds(), style, BodyStyle::default(), false)
                    .unwrap();
            assert_eq!(frame.shadow_vp, baseline.shadow_vp);
            assert_eq!(frame.shadow_rect, baseline.shadow_rect);
            assert_eq!(frame.contact, baseline.contact);
            assert_eq!(frame.ground, baseline.ground);
            if !grid_visible {
                assert_eq!(frame.support_origin[3], -1.0);
            }
        }
    }
}

#[test]
#[ignore = "requires real Metal hardware; missing hardware fails qualification"]
fn headless_vertical_and_oblique_support_grids_preserve_modeling_ground() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("real Metal adapter required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_device_features(adapter.features()),
        ..Default::default()
    }))
    .unwrap();
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let scene = fixture_scene(SceneIdentity {
        session: 67,
        scene: 1,
    });
    let view = fixture_view(&scene, Projection::Orthographic, 1.0);
    let mut renderer = Renderer::new(&device, &queue, [320, 240], RenderLimits::default()).unwrap();
    idle(&device, &mut renderer);
    renderer.replace_scene(&scene).unwrap();
    for theme in [Theme::Light, Theme::Dark] {
        let style = RenderStyle {
            theme,
            ..Default::default()
        };
        let baseline = render(
            &mut renderer,
            &device,
            &queue,
            &view,
            style,
            &format!("grid-modeling-{theme:?}"),
        );
        for (index, support) in supports().into_iter().enumerate() {
            let support_style = RenderStyle {
                support_grid: Some(support),
                ..style
            };
            let pixels = render(
                &mut renderer,
                &device,
                &queue,
                &view,
                support_style,
                &format!("grid-support-{index}-{theme:?}"),
            );
            let changed = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(baseline.as_chunks::<4>().0.iter())
                .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(a, b)| a.abs_diff(*b) > 2))
                .count();
            assert!(
                changed > 100,
                "support grid must visibly change captured frame"
            );
        }
        let restored = render(
            &mut renderer,
            &device,
            &queue,
            &view,
            style,
            &format!("grid-restored-{theme:?}"),
        );
        assert_eq!(
            restored, baseline,
            "hover cancellation must restore ordinary ground byte-exactly"
        );
    }
    idle(&device, &mut renderer);
    assert!(pollster::block_on(errors.pop()).is_none());
}
