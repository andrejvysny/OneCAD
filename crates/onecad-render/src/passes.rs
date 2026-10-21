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
    stencil: bool,
) -> Option<wgpu::RenderPassDepthStencilAttachment<'_>> {
    Some(wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(clear),
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: stencil.then_some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(0),
            store: wgpu::StoreOp::Store,
        }),
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
        let studio = self.options.display_mode.studio();
        if studio && self.contact_dirty {
            self.height_pass(encoder, target);
            for (view, pipeline, bind) in [
                (&target.occ, &self.pipelines.contact_occ, 0),
                (&target.occ_tmp, &self.pipelines.contact_h, 1),
                (&target.occ_final, &self.pipelines.contact_v, 2),
            ] {
                self.fullscreen(encoder, view, pipeline, &target.post_binds[bind]);
            }
        }
        if studio {
            let mut pass = begin(
                encoder,
                "depth/normal",
                &[color(&target.pre_normal, wgpu::Color::TRANSPARENT)],
                depth(&target.pre_depth, 0.0, true),
            );
            pass.set_stencil_reference(0);
            pass.set_pipeline(&self.pipelines.prepass);
            pass.set_bind_group(1, &target.body_bind, &[]);
            for body in self.committed.iter() {
                geometry(&mut pass, body);
            }
            if self.options.section[..3].iter().any(|v| *v != 0.0) {
                self.section_mask(&mut pass, &self.pipelines.pre_cap_mask, false);
                pass.set_pipeline(&self.pipelines.pre_cap);
                pass.set_bind_group(0, &self.frame_bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        if studio {
            for (view, pipeline, bind) in [
                (&target.ao_raw, &self.pipelines.gtao, 3),
                (&target.ao_tmp, &self.pipelines.ao_h, 4),
                (&target.ao_final, &self.pipelines.ao_v, 5),
            ] {
                self.fullscreen(encoder, view, pipeline, &target.post_binds[bind]);
            }
        }
        if self.options.display_mode == crate::DisplayMode::Xray {
            self.xray_pass(encoder, target);
        }
        self.main_pass(encoder, target, clear);
        if self.options.ssaa != 1.0 {
            let mut pass = begin(
                encoder,
                "CAD downsample",
                &[color(&target.output_attachment, wgpu::Color::BLACK)],
                None,
            );
            pass.set_pipeline(&self.pipelines.compose);
            pass.set_bind_group(0, &target.compose, &[]);
            pass.draw(0..3, 0..1);
        }
    }
    fn height_pass(&self, encoder: &mut wgpu::CommandEncoder, target: &Targets) {
        let mut pass = begin(
            encoder,
            "contact height",
            &[],
            depth(&target.height, 1.0, true),
        );
        pass.set_stencil_reference(0);
        pass.set_pipeline(&self.pipelines.height);
        pass.set_bind_group(1, &target.body_bind, &[]);
        for body in self.committed.iter() {
            pass.set_bind_group(0, &body.frame_bind, &[]);
            pass.set_vertex_buffer(0, body.positions.slice(..));
            pass.set_index_buffer(body.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..body.index_count, 0, 0..1);
        }
        if self.options.section[2] < 0.0 {
            self.section_mask(&mut pass, &self.pipelines.height_cap_mask, true);
            pass.set_pipeline(&self.pipelines.height_cap);
            pass.set_bind_group(0, &self.frame_bind, &[]);
            pass.draw(0..3, 0..1);
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
            resolve_target: Some(&target.resolved),
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
        let mut pass = begin(
            encoder,
            "CAD quality",
            &colors,
            depth(&target.depth, 0.0, true),
        );
        pass.set_stencil_reference(0);
        let wireframe = self.options.display_mode == crate::DisplayMode::Wireframe;
        let xray = self.options.display_mode == crate::DisplayMode::Xray;
        if xray || wireframe {
            self.draw_ground(&mut pass, target);
        }
        pass.set_bind_group(1, &target.body_bind, &[]);
        if !wireframe {
            pass.set_pipeline(if xray {
                &self.pipelines.body_depth
            } else {
                &self.pipelines.body
            });
            for body in &self.committed {
                geometry(&mut pass, body);
            }
        }
        if xray {
            pass.set_pipeline(&self.pipelines.oit_compose);
            pass.set_bind_group(0, &self.frame_bind, &[]);
            pass.set_bind_group(1, &target.oit_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        pass.set_bind_group(1, &target.body_bind, &[]);
        if !wireframe && self.options.section[..3].iter().any(|v| *v != 0.0) {
            self.section_mask(&mut pass, &self.pipelines.cap_mask, false);
            pass.set_pipeline(if xray {
                &self.pipelines.cap_depth
            } else {
                &self.pipelines.cap
            });
            pass.set_bind_group(0, &self.frame_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        if self.options.display_mode.studio() {
            self.draw_ground(&mut pass, target);
        }
        pass.set_bind_group(1, &target.body_bind, &[]);
        if !wireframe {
            self.draw_highlights(&mut pass, crate::HighlightKind::Face);
        }
        if xray {
            pass.set_pipeline(&self.pipelines.hidden);
            for body in self.line_bodies() {
                pass.set_bind_group(0, &body.frame_bind, &[]);
                pass.set_vertex_buffer(
                    0,
                    if wireframe {
                        body.segments.slice(..)
                    } else {
                        body.feature_segments.slice(..)
                    },
                );
                pass.draw(
                    0..4,
                    0..if wireframe {
                        body.segment_count
                    } else {
                        body.feature_segment_count
                    },
                );
            }
        }
        if xray {
            pass.set_pipeline(&self.pipelines.hidden_silhouette);
            for body in self.line_bodies() {
                pass.set_bind_group(0, &body.frame_bind, &[]);
                pass.set_vertex_buffer(0, body.silhouettes.slice(..));
                pass.draw(0..4, 0..body.silhouette_count);
            }
        }
        if self.options.display_mode != crate::DisplayMode::Shaded {
            pass.set_pipeline(&self.pipelines.silhouette);
            for body in self.line_bodies() {
                pass.set_bind_group(0, &body.frame_bind, &[]);
                pass.set_vertex_buffer(0, body.silhouettes.slice(..));
                pass.draw(0..4, 0..body.silhouette_count);
            }
        }
        if self.options.display_mode != crate::DisplayMode::Shaded {
            pass.set_pipeline(&self.pipelines.edge);
            for body in self.line_bodies() {
                pass.set_bind_group(0, &body.frame_bind, &[]);
                pass.set_vertex_buffer(
                    0,
                    if wireframe {
                        body.segments.slice(..)
                    } else {
                        body.feature_segments.slice(..)
                    },
                );
                pass.draw(
                    0..4,
                    0..if wireframe {
                        body.segment_count
                    } else {
                        body.feature_segment_count
                    },
                );
            }
        }
        self.draw_section_contours(&mut pass);
        pass.set_pipeline(&self.pipelines.preview);
        pass.set_bind_group(1, &target.body_bind, &[]);
        if wireframe {
            pass.set_pipeline(&self.pipelines.edge);
            for body in &self.preview {
                pass.set_bind_group(0, &body.frame_bind, &[]);
                pass.set_vertex_buffer(0, body.segments.slice(..));
                pass.draw(0..4, 0..body.segment_count);
            }
            pass.set_pipeline(&self.pipelines.silhouette);
            for body in &self.preview {
                pass.set_bind_group(0, &body.frame_bind, &[]);
                pass.set_vertex_buffer(0, body.silhouettes.slice(..));
                pass.draw(0..4, 0..body.silhouette_count);
            }
        } else {
            for body in &self.preview {
                geometry(&mut pass, body);
            }
        }
        if !wireframe {
            self.draw_preview_cap(&mut pass);
        }
        self.draw_highlights(&mut pass, crate::HighlightKind::Edge);
        if wireframe {
            if let Some(highlights) = &self.highlights {
                pass.set_pipeline(&self.pipelines.highlight_edge);
                pass.set_vertex_buffer(0, highlights.boundaries.slice(..));
                for draw in &highlights.boundary_draws {
                    let tone = usize::from(draw.tone == crate::highlights::HighlightTone::Hover);
                    pass.set_bind_group(0, &highlights.binds[tone], &[]);
                    pass.draw(0..4, draw.first..draw.first + draw.count);
                }
            }
        }
    }
    fn line_bodies(&self) -> impl Iterator<Item = &GpuBody> {
        // Selection is an interaction overlay: its line color must not depend on publication order.
        self.committed
            .iter()
            .filter(|body| !body.style.selected)
            .chain(self.committed.iter().filter(|body| body.style.selected))
    }
    fn draw_preview_cap(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(preview) = self.preview.first() else {
            return;
        };
        if self.options.section[..3].iter().all(|v| *v == 0.0) {
            return;
        }
        pass.set_pipeline(&self.pipelines.preview_cap_clear);
        pass.set_bind_group(0, &self.frame_bind, &[]);
        pass.draw(0..3, 0..1);
        pass.set_pipeline(&self.pipelines.cap_mask);
        for body in &self.preview {
            geometry(pass, body);
        }
        pass.set_pipeline(&self.pipelines.preview_cap);
        pass.set_bind_group(0, &preview.frame_bind, &[]);
        pass.draw(0..3, 0..1);
    }
    fn xray_pass(&self, encoder: &mut wgpu::CommandEncoder, target: &Targets) {
        let mut pass = begin(
            encoder,
            "Xray all surface layers",
            &[
                color(&target.oit_accum, wgpu::Color::TRANSPARENT),
                color(&target.oit_reveal, wgpu::Color::WHITE),
            ],
            depth(&target.pre_depth, 0.0, true),
        );
        pass.set_pipeline(&self.pipelines.oit);
        pass.set_bind_group(1, &target.body_bind, &[]);
        for body in &self.committed {
            geometry(&mut pass, body);
        }
        if self.options.section[..3].iter().any(|v| *v != 0.0) {
            self.section_mask(&mut pass, &self.pipelines.oit_mask, false);
            pass.set_pipeline(&self.pipelines.oit_cap);
            pass.set_bind_group(0, &self.frame_bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
    fn draw_ground(&self, pass: &mut wgpu::RenderPass<'_>, target: &Targets) {
        pass.set_pipeline(&self.pipelines.ground);
        pass.set_bind_group(0, &self.frame_bind, &[]);
        pass.set_bind_group(1, &target.ground_bind, &[]);
        pass.draw(0..12, 0..1);
    }
    fn section_mask(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        height: bool,
    ) {
        pass.set_pipeline(pipeline);
        for body in &self.committed {
            if height {
                pass.set_bind_group(0, &body.frame_bind, &[]);
                pass.set_vertex_buffer(0, body.positions.slice(..));
                pass.set_index_buffer(body.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..body.index_count, 0, 0..1);
            } else {
                geometry(pass, body);
            }
        }
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
