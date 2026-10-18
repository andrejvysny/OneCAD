@group(1) @binding(0) var accumulation: texture_2d<f32>;
@group(1) @binding(1) var revealage: texture_2d<f32>;

@vertex fn vs_full(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment fn fs_composite(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(p.xy);
    let sum = textureLoad(accumulation, pixel, 0);
    let alpha = 1.0 - textureLoad(revealage, pixel, 0).r;
    return vec4<f32>(sum.rgb / max(sum.a, 1e-8), alpha);
}
