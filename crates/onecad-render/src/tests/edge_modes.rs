use super::{
    quality_acceptance::{idle, render, section_options},
    *,
};
use onecad_protocol::mesh::EdgeClass;

fn classified(name: &str) -> SceneSnapshot {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/render/edge-policy")
        .join(format!("{name}.mesh1"));
    let body = SceneBody::new(
        BodyId::new(),
        1,
        Arc::from(std::fs::read(path).unwrap()),
        onecad_scene::MeshLimits::default(),
    )
    .unwrap();
    SceneSnapshot::new(
        SceneIdentity {
            session: 41,
            scene: 1,
        },
        vec![body],
        SceneLimits::default(),
    )
    .unwrap()
}

fn view(scene: &SceneSnapshot, projection: Projection) -> ViewSnapshot {
    let viewport = Viewport {
        width: 256.0,
        height: 192.0,
        scale_factor: 1.25,
    };
    let mut camera = Camera {
        projection,
        ..Default::default()
    };
    let fit = onecad_scene::fit_camera(scene.bounds().unwrap(), camera, viewport, None).unwrap();
    camera.target = fit.target;
    camera.offset = camera.offset.normalize() * fit.distance;
    ViewSnapshot::new(scene.identity(), 1, camera, viewport).unwrap()
}

fn changed(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(a, b)| a.abs_diff(*b) > 2))
        .count()
}

#[test]
fn checked_worker_fixtures_include_classification_and_adjacency() {
    for name in [
        "box",
        "cylinder",
        "sphere",
        "filleted_tangent_block",
        "near_tangent",
        "small_crease",
        "non_manifold",
        "open_boundary",
        "reversed_box",
        "transformed_box",
    ] {
        let scene = classified(name);
        let mesh = scene.bodies()[0].mesh().unwrap();
        assert!(mesh.header().section(14).is_some());
        assert!(mesh.header().section(15).is_some());
        for edge in 0..mesh.header().edge_count as usize {
            assert_ne!(
                mesh.edge_class(edge),
                EdgeClass::Unknown,
                "{name}, edge {edge}"
            );
            assert!(!mesh.edge_incident_faces(edge).is_empty());
        }
    }
}

#[test]
#[ignore = "requires real Metal hardware; absent GPU fails qualification"]
fn headless_classified_edges_and_mode_combinations() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("real Metal GPU required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_device_features(adapter.features()),
        ..Default::default()
    }))
    .unwrap();
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut renderer = Renderer::new(&device, &queue, [320, 240], RenderLimits::default()).unwrap();
    idle(&device, &mut renderer);
    verify_edge_filters(&mut renderer);
    let scene = classified("box");
    renderer.replace_scene(&scene).unwrap();
    verify_wire_interactions(&mut renderer, &device, &queue, &scene);
    verify_combinations(&mut renderer, &device, &queue, &scene);
    idle(&device, &mut renderer);
    assert!(pollster::block_on(errors.pop()).is_none());
}

fn verify_edge_filters(renderer: &mut Renderer) {
    for name in [
        "cylinder",
        "sphere",
        "filleted_tangent_block",
        "near_tangent",
        "small_crease",
    ] {
        let scene = classified(name);
        renderer.replace_scene(&scene).unwrap();
        let mesh = scene.bodies()[0].mesh().unwrap();
        let mut expected = 0;
        for edge in 0..mesh.header().edge_count as usize {
            if matches!(
                mesh.edge_class(edge),
                EdgeClass::Unknown
                    | EdgeClass::Boundary
                    | EdgeClass::Sharp
                    | EdgeClass::NonManifold
            ) {
                expected += mesh.edge_range(edge).unwrap().count.saturating_sub(1);
            }
        }
        assert_eq!(
            renderer.committed[0].feature_segment_count, expected,
            "{name}"
        );
        if name == "small_crease" {
            assert_eq!(expected, renderer.committed[0].segment_count);
        } else {
            assert!(
                expected < renderer.committed[0].segment_count,
                "{name}: filtered joins/seams"
            );
        }
    }
}

fn verify_wire_interactions(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
) {
    let view = view(scene, Projection::Orthographic);
    renderer
        .set_options(RenderOptions {
            display_mode: DisplayMode::Wireframe,
            ..Default::default()
        })
        .unwrap();
    let wire = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "box-wire",
    );
    let body = &scene.bodies()[0];
    let mesh = body.mesh().unwrap();
    assert_all_box_wires_visible(&mesh, &view, &wire);
    let targets: Vec<_> = (0..mesh.header().face_count as usize)
        .map(|face| HighlightTarget {
            scene: scene.identity(),
            body_id: body.body_id(),
            mesh_revision: body.mesh_revision(),
            kind: HighlightKind::Face,
            topology_id: mesh.face_id(face).unwrap(),
        })
        .collect();
    renderer
        .replace_highlights(
            PreparedHighlights::new(
                scene,
                &targets,
                targets.first().copied(),
                renderer.preparation_budget(),
            )
            .unwrap(),
        )
        .unwrap();
    let selected = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "wire-face-boundaries",
    );
    assert!(changed(&wire, &selected) > 100);
    assert!(
        changed(&wire, &selected) < 6000,
        "wire face selection must not fill faces"
    );
    renderer.clear_highlights().unwrap();
    renderer
        .replace_preview(
            PreviewIdentity {
                base: scene.identity(),
                generation: 1,
            },
            scene,
        )
        .unwrap();
    let preview = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "wire-preview",
    );
    assert!(
        changed(&wire, &preview) < 6000,
        "wire preview must not fill faces"
    );
    renderer.clear_preview().unwrap();
    renderer
        .set_options(RenderOptions {
            display_mode: DisplayMode::Shaded,
            ..Default::default()
        })
        .unwrap();
    let shaded = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "box-shaded",
    );
    renderer.set_options(RenderOptions::default()).unwrap();
    let edges = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "box-shaded-edges",
    );
    let line_pixels = changed(&shaded, &edges);
    assert!(
        (100..6000).contains(&line_pixels),
        "base edge visibility: {line_pixels}"
    );
    assert!(
        changed(&wire, &shaded) > 10000,
        "model needs substantial face coverage"
    );
}

fn verify_combinations(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
) {
    let geometry = renderer.committed[0].positions.clone();
    let body = &scene.bodies()[0];
    let mesh = body.mesh().unwrap();
    let target = HighlightTarget {
        scene: scene.identity(),
        body_id: body.body_id(),
        mesh_revision: body.mesh_revision(),
        kind: HighlightKind::Face,
        topology_id: mesh.face_id(0).unwrap(),
    };
    let cut = [0.0, 0.0, 1.0, 10.0];
    for mode in [
        DisplayMode::Wireframe,
        DisplayMode::Shaded,
        DisplayMode::ShadedEdges,
        DisplayMode::Xray,
    ] {
        for ssaa in [1.0, 2.0] {
            section_options(
                renderer,
                scene,
                RenderOptions {
                    display_mode: mode,
                    ssaa,
                    section: cut,
                    ..Default::default()
                },
            );
            renderer
                .replace_highlights(
                    PreparedHighlights::new(
                        scene,
                        &[target],
                        Some(target),
                        renderer.preparation_budget(),
                    )
                    .unwrap(),
                )
                .unwrap();
            renderer
                .replace_preview(
                    PreviewIdentity {
                        base: scene.identity(),
                        generation: renderer.latest_preview_generation.unwrap_or(0) + 1,
                    },
                    scene,
                )
                .unwrap();
            for projection in [Projection::Perspective, Projection::Orthographic] {
                let view = view(scene, projection);
                for theme in [Theme::Dark, Theme::Light] {
                    let name =
                        format!("mode-section-preview-{mode:?}-{projection:?}-{theme:?}-{ssaa}");
                    let pixels = render(
                        renderer,
                        device,
                        queue,
                        &view,
                        RenderStyle {
                            theme,
                            ..Default::default()
                        },
                        &name,
                    );
                    assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
                }
            }
            renderer.clear_preview().unwrap();
            renderer.clear_highlights().unwrap();
            assert_eq!(geometry, renderer.committed[0].positions);
        }
    }
    renderer.set_options(RenderOptions::default()).unwrap();
}

fn assert_all_box_wires_visible(
    mesh: &onecad_protocol::mesh::MeshView<'_>,
    view: &ViewSnapshot,
    pixels: &[u8],
) {
    for edge in 0..mesh.header().edge_count as usize {
        let span = mesh.edge_range(edge).unwrap();
        let a = glam::Vec3::from_array(mesh.edge_point(span.first as usize).unwrap()).as_dvec3();
        let b = glam::Vec3::from_array(
            mesh.edge_point((span.first + span.count - 1) as usize)
                .unwrap(),
        )
        .as_dvec3();
        let point = view.project((a + b) * 0.5).unwrap().logical * view.viewport().scale_factor;
        let x = point.x.round() as usize;
        let y = point.y.round() as usize;
        let brightness = (y.saturating_sub(2)..=(y + 2).min(239))
            .flat_map(|y| (x.saturating_sub(2)..=(x + 2).min(319)).map(move |x| (y * 320 + x) * 4))
            .map(|i| *pixels[i..i + 3].iter().min().unwrap())
            .max()
            .unwrap();
        assert!(
            brightness > 130,
            "topology edge {edge} missing, including rear wires: {brightness}"
        );
    }
}
