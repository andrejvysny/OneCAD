use crate::gpu::depth_state;

/// Count retained surface crossings independently of visible wall depth. Complete
/// unrelated solids cancel; solids crossing the cut plane leave a nonzero winding.
pub(super) fn state(
    format: wgpu::TextureFormat,
    compare: wgpu::CompareFunction,
    mask: bool,
) -> Option<wgpu::DepthStencilState> {
    use wgpu::StencilOperation::{DecrementWrap, IncrementWrap, Keep};
    let face = |operation| wgpu::StencilFaceState {
        compare: if mask {
            wgpu::CompareFunction::Always
        } else {
            wgpu::CompareFunction::NotEqual
        },
        fail_op: Keep,
        depth_fail_op: Keep,
        pass_op: operation,
    };
    let mut state = depth_state(compare, !mask, wgpu::DepthBiasState::default());
    if let Some(state) = &mut state {
        state.format = format;
        state.stencil = wgpu::StencilState {
            front: face(if mask { DecrementWrap } else { Keep }),
            back: face(if mask { IncrementWrap } else { Keep }),
            read_mask: 0xff,
            write_mask: if mask { 0xff } else { 0 },
        };
    }
    state
}

pub(super) fn height_depth(format: wgpu::TextureFormat) -> Option<wgpu::DepthStencilState> {
    let mut state = depth_state(
        wgpu::CompareFunction::LessEqual,
        true,
        wgpu::DepthBiasState::default(),
    );
    if let Some(state) = &mut state {
        state.format = format;
    }
    state
}
