//! The pass ladder, recorded into one command encoder.

use super::{EdgeExp, Parts, Renderer};
use crate::timing::Timer;

fn color<'a>(
    view: &'a wgpu::TextureView,
    resolve: Option<&'a wgpu::TextureView>,
    clear: wgpu::Color,
    store: bool,
) -> Option<wgpu::RenderPassColorAttachment<'a>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: resolve,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(clear),
            store: if store { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard },
        },
    })
}

fn depth(view: &wgpu::TextureView, clear: f32, store: bool) -> Option<wgpu::RenderPassDepthStencilAttachment<'_>> {
    Some(wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(clear),
            store: if store { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard },
        }),
        stencil_ops: None,
    })
}

fn begin<'e>(
    enc: &'e mut wgpu::CommandEncoder,
    timer: &mut Option<&mut Timer>,
    label: &'static str,
    colors: &[Option<wgpu::RenderPassColorAttachment<'_>>],
    ds: Option<wgpu::RenderPassDepthStencilAttachment<'_>>,
) -> wgpu::RenderPass<'e> {
    let ts = timer.as_mut().and_then(|t| t.pass(label));
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: colors,
        depth_stencil_attachment: ds,
        timestamp_writes: ts,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// One pass of the ladder. Every pass is idempotent, so it can be repeated for timing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pass {
    ContactHeight,
    ContactOcc,
    ContactBlurH,
    ContactBlurV,
    Prepass,
    SilCull,
    Gtao,
    AoBlurH,
    AoBlurV,
    Main,
}

impl Pass {
    pub const CONTACT: [Pass; 4] = [Pass::ContactHeight, Pass::ContactOcc, Pass::ContactBlurH, Pass::ContactBlurV];
    pub const AO: [Pass; 4] = [Pass::Prepass, Pass::Gtao, Pass::AoBlurH, Pass::AoBlurV];

    pub fn label(self) -> &'static str {
        match self {
            Pass::ContactHeight => "contact height",
            Pass::ContactOcc => "contact occlusion",
            Pass::ContactBlurH => "contact blur h",
            Pass::ContactBlurV => "contact blur v",
            Pass::Prepass => "depth/normal prepass",
            Pass::SilCull => "silhouette compaction",
            Pass::Gtao => "gtao",
            Pass::AoBlurH => "ao blur h",
            Pass::AoBlurV => "ao blur v",
            Pass::Main => "main (msaa 4x)",
        }
    }
}

impl Renderer {
    fn fullscreen(
        &self,
        enc: &mut wgpu::CommandEncoder,
        timer: &mut Option<&mut Timer>,
        label: &'static str,
        target: &wgpu::TextureView,
        pipe: &wgpu::RenderPipeline,
        bind: usize,
    ) {
        let mut pass = begin(enc, timer, label, &[color(target, None, wgpu::Color::WHITE, true)], None);
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &self.frame_bind, &[]);
        pass.set_bind_group(1, &self.post_binds[bind], &[]);
        pass.draw(0..3, 0..1);
    }

    fn draw_bodies(&self, pass: &mut wgpu::RenderPass<'_>, pipe: &wgpu::RenderPipeline) {
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &self.frame_bind, &[]);
        pass.set_bind_group(1, &self.body_bind, &[]);
        pass.set_vertex_buffer(0, self.positions.slice(..));
        pass.set_vertex_buffer(1, self.normals.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    /// Record one pass. The contact passes are camera-independent (rebuilt only when the
    /// geometry changes); the rest run per frame. `quality` false draws today's baseline look.
    pub fn encode_pass(
        &self,
        enc: &mut wgpu::CommandEncoder,
        timer: &mut Option<&mut Timer>,
        which: Pass,
        quality: bool,
        parts: Parts,
        clear: [f32; 4],
    ) {
        let label = which.label();
        match which {
            Pass::ContactHeight => {
                let mut pass = begin(enc, timer, label, &[], depth(&self.height, 1.0, true));
                pass.set_pipeline(&self.pipes.height);
                pass.set_bind_group(0, &self.frame_bind, &[]);
                pass.set_bind_group(1, &self.body_bind, &[]);
                pass.set_vertex_buffer(0, self.positions.slice(..));
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
            Pass::ContactOcc => self.fullscreen(enc, timer, label, &self.occ, &self.pipes.contact_occ, 0),
            Pass::ContactBlurH => self.fullscreen(enc, timer, label, &self.occ_tmp, &self.pipes.contact_h, 1),
            Pass::ContactBlurV => self.fullscreen(enc, timer, label, &self.occ_final, &self.pipes.contact_v, 2),
            Pass::Prepass => {
                let colors = [color(&self.pre_normal, None, wgpu::Color::TRANSPARENT, true)];
                let mut pass = begin(enc, timer, label, &colors, depth(&self.pre_depth, 0.0, true));
                self.draw_bodies(&mut pass, &self.pipes.prepass);
            }
            Pass::Gtao => self.fullscreen(enc, timer, label, &self.ao_raw, &self.pipes.gtao, 3),
            Pass::AoBlurH => self.fullscreen(enc, timer, label, &self.ao_tmp, &self.pipes.ao_h, 4),
            Pass::AoBlurV => self.fullscreen(enc, timer, label, &self.ao_final, &self.pipes.ao_v, 5),
            Pass::SilCull => {
                // Reset the indirect instance count, then append this eye's silhouettes.
                enc.clear_buffer(&self.sil_args, 4, Some(4));
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(label), timestamp_writes: None });
                pass.set_pipeline(&self.pipes.sil_cull);
                pass.set_bind_group(0, &self.frame_bind, &[]);
                pass.set_bind_group(1, &self.sil_bind, &[]);
                pass.dispatch_workgroups(self.sil_count.div_ceil(64).max(1), 1, 1);
            }
            Pass::Main => self.encode_main(enc, timer, quality, parts, clear),
        }
    }

    fn encode_main(&self, enc: &mut wgpu::CommandEncoder, timer: &mut Option<&mut Timer>, quality: bool, parts: Parts, clear: [f32; 4]) {
        let c = wgpu::Color { r: clear[0] as f64, g: clear[1] as f64, b: clear[2] as f64, a: 1.0 };
        let colors = [color(&self.msaa_color, Some(&self.output_view), c, false)];
        let mut pass = begin(enc, timer, Pass::Main.label(), &colors, depth(&self.msaa_depth, 0.0, false));
        if parts.bodies {
            self.draw_bodies(&mut pass, if quality { &self.pipes.body } else { &self.pipes.body_base });
        }
        pass.set_bind_group(0, &self.frame_bind, &[]);
        if parts.ground {
            if quality {
                pass.set_pipeline(&self.pipes.ground);
                pass.set_bind_group(1, &self.ground_bind, &[]);
                pass.draw(0..6, 0..1);
            } else if let Some((grid, n)) = &self.grid {
                pass.set_pipeline(if parts.a2c { &self.pipes.grid_a2c } else { &self.pipes.grid_base });
                pass.set_vertex_buffer(0, grid.slice(..));
                pass.draw(0..4, 0..*n);
            }
        }
        if parts.silhouettes && quality {
            if parts.sil_vertex_cull {
                pass.set_pipeline(if parts.a2c { &self.pipes.sil_a2c } else { &self.pipes.sil });
                pass.set_vertex_buffer(0, self.sils.slice(..));
                pass.draw(0..4, 0..self.sil_count);
            } else {
                pass.set_pipeline(if parts.a2c { &self.pipes.sil_seg_a2c } else { &self.pipes.sil_seg });
                pass.set_vertex_buffer(0, self.sil_out.slice(..));
                pass.draw_indirect(&self.sil_args, 0);
            }
        }
        if parts.edges {
            let pipe = match parts.edge_exp {
                EdgeExp::Opaque => &self.pipes.edge_opaque,
                EdgeExp::Discard => &self.pipes.edge_discard,
                EdgeExp::CulledAfterVs => &self.pipes.edge_culled,
                EdgeExp::None if parts.a2c => &self.pipes.edge_a2c,
                EdgeExp::None => &self.pipes.edge,
            };
            pass.set_pipeline(pipe);
            pass.set_vertex_buffer(0, self.segments.slice(..));
            pass.draw(0..4, 0..self.segment_count);
        }
    }
}

/// The passes a frame runs: the contact map (when the geometry changed), the AO chain
/// (quality), then the main pass.
pub fn frame_passes(quality: bool, contact: bool, parts: Parts) -> Vec<Pass> {
    let mut v = Vec::new();
    if quality && contact {
        v.extend(Pass::CONTACT);
    }
    if quality {
        v.extend(Pass::AO);
        if parts.silhouettes && !parts.sil_vertex_cull {
            v.push(Pass::SilCull);
        }
    }
    v.push(Pass::Main);
    v
}

pub fn encode(
    r: &Renderer,
    enc: &mut wgpu::CommandEncoder,
    timer: Option<&mut Timer>,
    quality: bool,
    contact: bool,
    parts: Parts,
    clear: [f32; 4],
) {
    let mut timer = timer;
    for p in frame_passes(quality, contact, parts) {
        r.encode_pass(enc, &mut timer, p, quality, parts, clear);
    }
    if let Some(t) = timer {
        t.finish(enc);
    }
}
