use super::*;

#[test]
#[ignore = "requires real Metal hardware; absent GPU fails qualification"]
fn retina_target_replacement_reclaims_only_after_fences() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .expect("real GPU required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_device_features(adapter.features()),
        ..Default::default()
    }))
    .unwrap();
    let mut renderer =
        Renderer::new(&device, &queue, [2560, 1496], RenderLimits::default()).unwrap();
    quality_acceptance::idle(&device, &mut renderer);
    assert!(matches!(
        renderer.resize([2562, 1498]),
        Err(RenderError::Budget(_))
    ));
    assert_eq!(renderer.size(), [2560, 1496]);
    assert!(renderer.output_view().is_some());
    for size in [[2562, 1498], [2560, 1496], [2600, 1520], [2400, 1400]] {
        renderer.resize_reclaiming(size).unwrap();
        assert_eq!(renderer.size(), size);
        assert!(renderer.output_view().is_some());
        assert!(renderer.memory_estimate().peak_total_bytes <= 768 << 20);
    }
    assert!(renderer.resize_reclaiming([4097, 2000]).is_err());
    assert_eq!(renderer.size(), [2400, 1400]);
    renderer.resize_reclaiming([1280, 748]).unwrap();
    renderer.resize([0, 0]).unwrap();
    assert!(renderer.output_view().is_none());
    renderer.resize([640, 400]).unwrap();
    for ssaa in [2.0, 1.0, 2.0, 1.0] {
        renderer
            .set_options_reclaiming(RenderOptions {
                ssaa,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(renderer.options().ssaa, ssaa);
    }
    assert!(RenderError::Allocation("out of memory".into()).recoverable_target_refusal());
    assert!(!RenderError::Device("lost".into()).recoverable_target_refusal());
}
