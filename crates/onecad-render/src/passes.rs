use crate::{mesh::GpuBody, targets::Targets, Renderer};
fn color(
    view: &wgpu::TextureView,
    clear: wgpu::Color,
) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(clear),
            store: wgpu::StoreOp::Store,
        },
    })
}
fn depth(
    view: &wgpu::TextureView,
    clear: f32,
) -> Option<wgpu::RenderPassDepthStencilAttachment<'_>> {
    Some(wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(clear),
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: None,
    })
}
fn begin<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &str,
    colors: &[Option<wgpu::RenderPassColorAttachment<'_>>],
    depth_stencil_attachment: Option<wgpu::RenderPassDepthStencilAttachment<'_>>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: colors,
        depth_stencil_attachment,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}
fn geometry(pass: &mut wgpu::RenderPass<'_>, body: &GpuBody) {
    pass.set_bind_group(0, &body.frame_bind, &[]);
    pass.set_vertex_buffer(0, body.positions.slice(..));
    pass.set_vertex_buffer(1, body.normals.slice(..));
    pass.set_index_buffer(body.indices.slice(..), wgpu::IndexFormat::Uint32);
    pass.draw_indexed(0..body.index_count, 0, 0..1);
}
impl Renderer {
    pub(crate) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &Targets,
        clear: [f32; 4],
    ) {
        self.height_pass(encoder, target);
        for (view, pipeline, bind) in [
            (&target.occ, &self.pipelines.contact_occ, 0),
            (&target.occ_tmp, &self.pipelines.contact_h, 1),
            (&target.occ_final, &self.pipelines.contact_v, 2),
        ] {
            self.fullscreen(encoder, view, pipeline, &target.post_binds[bind]);
        }
        {
            let mut pass = begin(
                encoder,
                "depth/normal",
                &[color(&target.pre_normal, wgpu::Color::TRANSPARENT)],
                depth(&target.pre_depth, 1.0),
            );
            pass.set_pipeline(&self.pipelines.prepass);
            pass.set_bind_group(1, &target.body_bind, &[]);
            for body in self.committed.iter() {
                geometry(&mut pass, body);
            }
        }
        for (view, pipeline, bind) in [
            (&target.ao_raw, &self.pipelines.gtao, 3),
            (&target.ao_tmp, &self.pipelines.ao_h, 4),
            (&target.ao_final, &self.pipelines.ao_v, 5),
        ] {
            self.fullscreen(encoder, view, pipeline, &target.post_binds[bind]);
        }
        self.main_pass(encoder, target, clear);
    }
    fn height_pass(&self, encoder: &mut wgpu::CommandEncoder, target: &Targets) {
        let mut pass = begin(encoder, "contact height", &[], depth(&target.height, 1.0));
        pass.set_pipeline(&self.pipelines.height);
        pass.set_bind_group(1, &target.body_bind, &[]);
        for body in self.committed.iter() {
            pass.set_bind_group(0, &body.frame_bind, &[]);
            pass.set_vertex_buffer(0, body.positions.slice(..));
            pass.set_index_buffer(body.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..body.index_count, 0, 0..1);
        }
    }
    fn fullscreen(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        bind: &wgpu::BindGroup,
    ) {
        let mut pass = begin(
            encoder,
            "postprocess",
            &[color(view, wgpu::Color::WHITE)],
            None,
        );
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.frame_bind, &[]);
        pass.set_bind_group(1, bind, &[]);
        pass.draw(0..3, 0..1);
    }
    fn main_pass(&self, encoder: &mut wgpu::CommandEncoder, target: &Targets, clear: [f32; 4]) {
        let colors = [Some(wgpu::RenderPassColorAttachment {
            view: &target.color,
            depth_slice: None,
            resolve_target: Some(&target.output_view),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: f64::from(clear[0]),
                    g: f64::from(clear[1]),
                    b: f64::from(clear[2]),
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Discard,
            },
        })];
        let mut pass = begin(encoder, "CAD quality", &colors, depth(&target.depth, 1.0));
        pass.set_pipeline(&self.pipelines.body);
        pass.set_bind_group(1, &target.body_bind, &[]);
        for body in self.committed.iter() {
            geometry(&mut pass, body);
        }
        pass.set_pipeline(&self.pipelines.ground);
        pass.set_bind_group(0, &self.frame_bind, &[]);
        pass.set_bind_group(1, &target.ground_bind, &[]);
        pass.draw(0..6, 0..1);
        pass.set_bind_group(1, &target.body_bind, &[]);
        self.draw_highlights(&mut pass, crate::HighlightKind::Face);
        pass.set_pipeline(&self.pipelines.silhouette);
        for body in self.committed.iter() {
            pass.set_bind_group(0, &body.frame_bind, &[]);
            pass.set_vertex_buffer(0, body.silhouettes.slice(..));
            pass.draw(0..4, 0..body.silhouette_count);
        }
        pass.set_pipeline(&self.pipelines.edge);
        for body in self.committed.iter() {
            pass.set_bind_group(0, &body.frame_bind, &[]);
            pass.set_vertex_buffer(0, body.segments.slice(..));
            pass.draw(0..4, 0..body.segment_count);
        }
        pass.set_pipeline(&self.pipelines.preview);
        pass.set_bind_group(1, &target.body_bind, &[]);
        for body in self.preview.iter() {
            geometry(&mut pass, body);
        }
        self.draw_highlights(&mut pass, crate::HighlightKind::Edge);
    }
    fn draw_highlights(&self, pass: &mut wgpu::RenderPass<'_>, kind: crate::HighlightKind) {
        let Some(highlights) = &self.highlights else {
            return;
        };
        pass.set_pipeline(match kind {
            crate::HighlightKind::Face => &self.pipelines.highlight_face,
            crate::HighlightKind::Edge => &self.pipelines.highlight_edge,
        });
        for draw in highlights.draws.iter().filter(|draw| draw.kind == kind) {
            let Some(body) = self.committed.get(draw.body_index) else {
                continue;
            };
            let uniform = match draw.tone {
                crate::highlights::HighlightTone::Selected => 0,
                crate::highlights::HighlightTone::Hover => 1,
            };
            pass.set_bind_group(0, &highlights.binds[uniform], &[]);
            if kind == crate::HighlightKind::Face {
                pass.set_vertex_buffer(0, body.positions.slice(..));
                pass.set_vertex_buffer(1, body.normals.slice(..));
                pass.set_index_buffer(body.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(draw.first..draw.first + draw.count, 0, 0..1);
            } else {
                pass.set_vertex_buffer(0, body.segments.slice(..));
                pass.draw(0..4, draw.first..draw.first + draw.count);
            }
        }
    }
}
