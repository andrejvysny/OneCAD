@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var lin: sampler;
struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_full(@builtin(vertex_index) vi: u32) -> Out {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}
// sRGB texture sampling decodes to linear; attachment encodes once for egui sampling.
@fragment fn fs_compose(i: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(image, lin, i.uv, 0.0).rgb, 1.0);
}
