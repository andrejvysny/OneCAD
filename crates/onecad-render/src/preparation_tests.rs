use super::*;

#[cfg(test)]
mod budget_tests {
    use super::*;
    use onecad_scene::{SceneBody, SceneLimits};
    use std::{path::PathBuf, sync::Arc};

    #[test]
    fn nema_preparation_fits_host_budget_without_worst_case_reservations() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/render/nema17");
        let mut paths: Vec<_> = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "mesh1")
            })
            .collect();
        paths.sort();
        let bodies = paths
            .into_iter()
            .map(|path| {
                SceneBody::new(
                    BodyId::new(),
                    1,
                    Arc::from(std::fs::read(path).unwrap()),
                    onecad_scene::MeshLimits::default(),
                )
                .unwrap()
            })
            .collect();
        let scene = SceneSnapshot::new(
            SceneIdentity {
                session: 1,
                scene: 1,
            },
            bodies,
            SceneLimits::default(),
        )
        .unwrap();
        assert_eq!(scene.bodies().len(), 67);
        assert_eq!(
            scene
                .bodies()
                .iter()
                .map(|body| u64::from(body.mesh().unwrap().header().triangle_count))
                .sum::<u64>(),
            381_524
        );
        let prepared = PreparedScene::new(
            &scene,
            PreparationBudget {
                limits: RenderLimits {
                    max_cpu_prepared_bytes: 64 << 20,
                    ..Default::default()
                },
                resident: MemoryEstimate::default(),
                max_buffer: u64::MAX,
            },
        )
        .unwrap();
        eprintln!("NEMA preparation charge: {} bytes", prepared.cpu_bytes());
        assert!(prepared.cpu_bytes() <= 64 << 20);
        let insufficient = PreparationBudget {
            limits: RenderLimits {
                max_cpu_prepared_bytes: prepared.cpu_bytes() - 1,
                ..Default::default()
            },
            resident: MemoryEstimate::default(),
            max_buffer: u64::MAX,
        };
        assert!(matches!(
            PreparedScene::new(&scene, insufficient),
            Err(RenderError::Budget("CPU preparation"))
        ));
        for (_, body) in prepared.bodies {
            assert_eq!(
                body.silhouette_len_and_capacity().0,
                body.silhouette_len_and_capacity().1
            );
        }
    }
}

#[cfg(test)]
mod reuse_tests {
    use super::*;
    use onecad_scene::{SceneBody, SceneLimits};
    use std::sync::Arc;

    #[test]
    #[ignore = "requires a real GPU; run explicitly with --ignored --nocapture"]
    fn publication_revision_reuses_identical_geometry_buffers() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: crate::required_device_features(adapter.features()),
            ..Default::default()
        }))
        .unwrap();
        let mut renderer = Renderer::new(&device, &queue, [0, 0], RenderLimits::default()).unwrap();
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        assert!(renderer.poll_idle().unwrap());
        let bytes: Arc<[u8]> = Arc::from(
            include_bytes!(
                "../../../fixtures/render/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
            )
            .as_slice(),
        );
        let id = BodyId::new();
        for revision in [1, 2] {
            let body = SceneBody::new(
                id,
                revision,
                bytes.clone(),
                onecad_scene::MeshLimits::default(),
            )
            .unwrap();
            let scene = SceneSnapshot::new(
                SceneIdentity {
                    session: 1,
                    scene: revision,
                },
                vec![body],
                SceneLimits::default(),
            )
            .unwrap();
            let previous = renderer
                .committed
                .first()
                .map(|body| body.positions.clone());
            renderer.replace_scene(&scene).unwrap();
            assert_eq!(renderer.committed[0].mesh_revision, revision);
            if let Some(previous) = previous {
                assert_eq!(renderer.committed[0].positions, previous);
            }
        }
        let committed_uniform = renderer.committed[0].uniform.clone();
        let body = SceneBody::new(id, 3, bytes, onecad_scene::MeshLimits::default()).unwrap();
        let base = renderer.scene_identity().unwrap();
        let scene = SceneSnapshot::new(base, vec![body], SceneLimits::default()).unwrap();
        renderer
            .replace_preview(
                crate::PreviewIdentity {
                    base,
                    generation: 1,
                },
                &scene,
            )
            .unwrap();
        assert_ne!(renderer.preview[0].uniform, committed_uniform);
    }
}

#[test]
fn cache_reuses_cpu_arrays_across_publication_revisions_and_preserves_cache_on_refusal() {
    let bytes: Arc<[u8]> = Arc::from(
        include_bytes!(
            "../../../fixtures/render/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
        )
        .as_slice(),
    );
    let id = BodyId::new();
    let budget = PreparationBudget {
        limits: RenderLimits::default(),
        resident: MemoryEstimate::default(),
        max_buffer: u64::MAX,
    };
    let mut cache = PreparationCache::default();
    let mut previous = None;
    for revision in [1, 2] {
        let body = SceneBody::new(
            id,
            revision,
            bytes.clone(),
            onecad_scene::MeshLimits::default(),
        )
        .unwrap();
        let scene = SceneSnapshot::new(
            SceneIdentity {
                session: 1,
                scene: revision,
            },
            vec![body],
            onecad_scene::SceneLimits::default(),
        )
        .unwrap();
        let prepared = PreparedScene::new_cached(&scene, budget, &mut cache).unwrap();
        assert_eq!(prepared.bodies[0].1.mesh_revision, revision);
        if let Some(previous) = previous {
            assert!(Arc::ptr_eq(&previous, &prepared.bodies[0].1.geometry));
        }
        previous = Some(prepared.bodies[0].1.geometry.clone());
        let insufficient = budget.limit_cpu_prepared_bytes(cache.cpu_bytes() - 1);
        assert!(matches!(
            PreparedScene::new_cached(&scene, insufficient, &mut cache),
            Err(RenderError::Budget("CPU preparation"))
        ));
        assert!(Arc::ptr_eq(
            previous.as_ref().unwrap(),
            &cache.geometries[0]
        ));
    }
}
