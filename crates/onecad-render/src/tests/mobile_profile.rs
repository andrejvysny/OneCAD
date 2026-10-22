//! The Mobile profile on real Metal: its targets match the estimate, Xray targets exist only
//! while Xray is shown, supersampling is refused and the body still renders without AO.
use super::{
    quality_acceptance::{idle, render},
    *,
};

fn device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .expect("real Metal GPU required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_device_features(adapter.features()),
        ..Default::default()
    }))
    .unwrap()
}

fn sphere_view(scene: &SceneSnapshot) -> ViewSnapshot {
    ViewSnapshot::new(
        scene.identity(),
        1,
        Camera {
            target: DVec3::ZERO,
            offset: DVec3::new(0.0, -100.0, 70.0),
            projection: Projection::Orthographic,
            ..Default::default()
        },
        Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        },
    )
    .unwrap()
}

fn mode(display_mode: DisplayMode) -> RenderOptions {
    RenderOptions {
        display_mode,
        ..Default::default()
    }
}

fn expected(options: RenderOptions, profile: RenderProfile) -> u64 {
    limits::quality_target_bytes([320, 240], options, profile).unwrap()
}

fn pixel(image: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * 320 + x) * 4;
    [image[i], image[i + 1], image[i + 2]]
}

#[test]
#[ignore = "requires real Metal hardware; missing hardware fails required qualification"]
fn mobile_profile_allocates_what_it_estimates_and_xray_targets_lazily() {
    let (device, queue) = device();
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let scene = SceneSnapshot::new(
        SceneIdentity {
            session: 40,
            scene: 1,
        },
        vec![super::analytic_fixture::sphere_body([0.0; 3], 35.0)],
        SceneLimits::default(),
    )
    .unwrap();
    let view = sphere_view(&scene);
    let mobile = RenderProfile::Mobile;
    let mut renderer =
        Renderer::with_profile(&device, &queue, [320, 240], RenderLimits::default(), mobile)
            .unwrap();
    idle(&device, &mut renderer);
    assert_eq!(renderer.profile(), mobile);
    let estimate = renderer.memory_estimate();
    assert_eq!(estimate.ibl_bytes, limits::ibl_bytes(mobile));
    assert_eq!(
        estimate.target_bytes,
        expected(RenderOptions::default(), mobile)
    );
    renderer.replace_scene(&scene).unwrap();
    let shaded = render(
        &mut renderer,
        &device,
        &queue,
        &view,
        RenderStyle::default(),
        "mobile-shaded",
    );
    assert_ne!(
        pixel(&shaded, 160, 120),
        pixel(&shaded, 2, 2),
        "the sphere renders without ambient occlusion"
    );

    renderer.set_options(mode(DisplayMode::Xray)).unwrap();
    let xray_bytes = expected(mode(DisplayMode::Xray), mobile);
    assert_eq!(renderer.memory_estimate().target_bytes, xray_bytes);
    assert!(xray_bytes > estimate.target_bytes);
    let xray = render(
        &mut renderer,
        &device,
        &queue,
        &view,
        RenderStyle::default(),
        "mobile-xray",
    );
    assert_ne!(pixel(&xray, 160, 120), pixel(&shaded, 160, 120));
    renderer.set_options(mode(DisplayMode::Shaded)).unwrap();
    assert_eq!(
        renderer.memory_estimate().target_bytes,
        estimate.target_bytes,
        "leaving Xray releases its targets"
    );

    let supersampled = RenderOptions {
        ssaa: 2.0,
        ..mode(DisplayMode::Shaded)
    };
    assert!(matches!(
        renderer.set_options(supersampled),
        Err(RenderError::Budget(_))
    ));
    assert_eq!(renderer.options().ssaa, 1.0);
    idle(&device, &mut renderer);
    assert!(pollster::block_on(errors.pop()).is_none());
}

#[test]
#[ignore = "requires real Metal hardware; missing hardware fails required qualification"]
fn desktop_profile_keeps_eager_xray_targets() {
    let (device, queue) = device();
    let mut renderer = Renderer::new(&device, &queue, [320, 240], RenderLimits::default()).unwrap();
    idle(&device, &mut renderer);
    assert_eq!(renderer.profile(), RenderProfile::Desktop);
    let before = renderer.memory_estimate().target_bytes;
    assert_eq!(before, limits::target_bytes([320, 240]).unwrap());
    renderer.set_options(mode(DisplayMode::Xray)).unwrap();
    assert_eq!(renderer.memory_estimate().target_bytes, before);
    assert_eq!(
        renderer.memory_estimate().ibl_bytes,
        limits::ibl_bytes(RenderProfile::Desktop)
    );
}
