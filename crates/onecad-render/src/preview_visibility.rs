//! Preview visibility never changes the retained committed scene or topology.
use crate::{mesh::GpuBody, BodyId, PreviewIdentity, RenderError, Renderer};

impl Renderer {
    /// Hide replaced or removed head bodies only for this accepted preview.
    pub fn suppress_preview_bodies(
        &mut self,
        identity: PreviewIdentity,
        bodies: &[BodyId],
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        if self.preview_identity != Some(identity) {
            return Err(RenderError::CaptureMismatch);
        }
        for body in &mut self.committed {
            let suppressed = bodies.contains(&body.id);
            self.contact_dirty |= body.preview_suppressed != suppressed;
            body.preview_suppressed = suppressed;
        }
        // Draw passes filter head highlights; retaining buffers restores them on Cancel.
        Ok(())
    }

    pub(crate) fn reset_preview_suppression(&mut self) {
        for body in &mut self.committed {
            self.contact_dirty |= body.preview_suppressed;
            body.preview_suppressed = false;
        }
    }

    pub(crate) fn visible_committed(&self) -> impl Iterator<Item = &GpuBody> {
        self.committed
            .iter()
            .filter(|body| !body.preview_suppressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        HighlightKind, HighlightTarget, PreparedHighlights, RenderLimits, SceneIdentity,
        SceneSnapshot,
    };
    use onecad_scene::{MeshLimits, SceneBody, SceneLimits};
    use std::sync::Arc;

    #[test]
    #[ignore = "requires real GPU; run explicitly with ONECAD_REQUIRE_GPU=1"]
    fn replacing_and_cancelling_preview_restore_only_transient_suppression() {
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
        let base = SceneIdentity {
            session: 1,
            scene: 1,
        };
        let ids = [BodyId::new(), BodyId::new()];
        let scene = scene(base, &ids);
        renderer.replace_scene(&scene).unwrap();
        select_faces_and_edges(&mut renderer, &scene);
        let highlights = renderer.highlights.as_ref().unwrap().uniforms.clone();
        let first = PreviewIdentity {
            base,
            generation: 1,
        };
        renderer.replace_preview(first, &scene).unwrap();
        renderer.suppress_preview_bodies(first, &ids[..1]).unwrap();
        assert_eq!(renderer.highlights.as_ref().unwrap().uniforms, highlights);
        assert_eq!(
            renderer
                .highlights
                .as_ref()
                .unwrap()
                .draws
                .iter()
                .filter(|draw| draw.body_id == ids[1])
                .count(),
            2,
            "unmodified body retains selected face and edge highlights"
        );
        assert_eq!(
            renderer
                .visible_committed()
                .map(|body| body.id)
                .collect::<Vec<_>>(),
            ids[1..]
        );
        assert!(renderer
            .suppress_preview_bodies(
                PreviewIdentity {
                    generation: 2,
                    ..first
                },
                &ids
            )
            .is_err());
        assert_eq!(renderer.visible_committed().count(), 1);
        renderer.clear_preview().unwrap();
        assert_eq!(renderer.visible_committed().count(), 2);
        assert_eq!(
            renderer.highlights.as_ref().unwrap().uniforms,
            highlights,
            "Cancel restores face and edge highlights without pointer motion or replacement"
        );
        assert_eq!(renderer.highlights.as_ref().unwrap().draws.len(), 4);
        let second = PreviewIdentity {
            generation: 2,
            ..first
        };
        renderer.replace_preview(second, &scene).unwrap();
        renderer.suppress_preview_bodies(second, &ids).unwrap();
        assert_eq!(renderer.visible_committed().count(), 0);
        renderer
            .replace_preview(
                PreviewIdentity {
                    generation: 3,
                    ..first
                },
                &scene,
            )
            .unwrap();
        assert_eq!(renderer.visible_committed().count(), 2);
        renderer
            .suppress_preview_bodies(
                PreviewIdentity {
                    generation: 3,
                    ..first
                },
                &ids,
            )
            .unwrap();
        renderer.replace_scene(&scene).unwrap();
        assert_eq!(renderer.visible_committed().count(), 2);
    }

    fn scene(identity: SceneIdentity, ids: &[BodyId]) -> SceneSnapshot {
        let mesh = include_bytes!(
            "../../../fixtures/render/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
        );
        let bodies = ids
            .iter()
            .map(|id| {
                SceneBody::new(*id, 1, Arc::from(mesh.as_slice()), MeshLimits::default()).unwrap()
            })
            .collect();
        SceneSnapshot::new(identity, bodies, SceneLimits::default()).unwrap()
    }

    fn select_faces_and_edges(renderer: &mut Renderer, scene: &SceneSnapshot) {
        let meshes: Vec<_> = scene
            .bodies()
            .iter()
            .map(|body| body.mesh().unwrap())
            .collect();
        let targets: Vec<_> = scene
            .bodies()
            .iter()
            .zip(&meshes)
            .flat_map(|(body, mesh)| {
                [
                    HighlightTarget {
                        scene: scene.identity(),
                        body_id: body.body_id(),
                        mesh_revision: 1,
                        kind: HighlightKind::Face,
                        topology_id: mesh.face_id(0).unwrap(),
                    },
                    HighlightTarget {
                        scene: scene.identity(),
                        body_id: body.body_id(),
                        mesh_revision: 1,
                        kind: HighlightKind::Edge,
                        topology_id: mesh.edge_id(0).unwrap(),
                    },
                ]
            })
            .collect();
        let selected =
            PreparedHighlights::new(scene, &targets, None, renderer.preparation_budget()).unwrap();
        renderer.replace_highlights(selected).unwrap();
    }
}
