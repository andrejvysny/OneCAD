// The ground: one quad on the world XY plane at Z = 0 (today's grid plane), carrying the
// contact shadow and an analytic grid (today's 1/2/5/10 step ladder, minor + major lines,
// fading out with distance from the target). Depth-tested against the bodies, writes no depth.

@group(1) @binding(0) var shadow_tex: texture_2d<f32>;
@group(1) @binding(1) var lin: sampler;

struct GOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) wp: vec3<f32>,
    @location(1) @interpolate(flat) support: u32,
};

@vertex
fn vs_ground(@builtin(vertex_index) vi: u32) -> GOut {
    var xs = array<f32, 6>(-1.0, 1.0, 1.0, -1.0, 1.0, -1.0);
    var ys = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let r = u.grid.w;
    let index = vi % 6u;
    var p = vec3<f32>(u.grid.y + xs[index] * r, u.grid.z + ys[index] * r, u.ground.x);
    if (vi >= 6u) {
        p = u.support_origin.xyz + u.support_x.xyz * (u.support_x.w + xs[index] * r)
            + u.support_y.xyz * (u.support_y.w + ys[index] * r);
    }
    var o: GOut;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    o.wp = p;
    o.support = vi / 6u;
    return o;
}

// Coverage of grid lines `width` px wide at spacing `step` (analytic, per axis, then max).
fn grid_cov(g: vec2<f32>, step: f32, width: f32) -> vec2<f32> {
    let c = g / step;
    let fw = max(fwidth(c), vec2<f32>(1e-6));
    let d = abs(fract(c + 0.5) - 0.5) / fw;
    let hw = width * 0.5;
    let cov = clamp(min(d + 0.5, vec2<f32>(hw)) - max(d - 0.5, vec2<f32>(-hw)), vec2<f32>(0.0), vec2<f32>(1.0));
    // x: coverage, y: cells per pixel (for the density fade)
    return vec2<f32>(max(cov.x, cov.y), max(fw.x, fw.y));
}

// Minor lines share the major lines' radial fade; they leave early only where cells shrink
// below ~2 px and would alias into a flat tone.
fn minor_lod(cells_per_pixel: f32) -> f32 {
    return 1.0 - smoothstep(0.4, 0.7, cells_per_pixel);
}

@fragment
fn fs_ground(i: GOut) -> @location(0) vec4<f32> {
    if (i.support == 1u) {
        if (u.support_origin.w < 0.5) { discard; }
        let relative = i.wp - u.support_origin.xyz;
        let local = vec2<f32>(dot(relative, u.support_x.xyz), dot(relative, u.support_y.xyz));
        let minor = grid_cov(local, u.grid.x, u.ground.y);
        let major = grid_cov(local, u.grid.x * 10.0, u.ground.z);
        let radius = length(local - vec2<f32>(u.support_x.w, u.support_y.w)) / u.grid.w;
        let fade = pow(clamp(1.0 - radius, 0.0, 1.0), 2.0);
        let coverage = max(minor.x * minor_lod(minor.y), major.x) * fade;
        return vec4<f32>(mix(u.grid_minor.rgb, u.grid_major.rgb, major.x), coverage);
    }
    let g = i.wp.xy;
    let minor = grid_cov(g, u.grid.x, u.ground.y);
    let major = grid_cov(g, u.grid.x * 10.0, u.ground.z);
    let r = length(g - u.grid.yz) / u.grid.w;
    let fade = pow(clamp(1.0 - r, 0.0, 1.0), 2.0);
    let lod = minor_lod(minor.y);
    var c = u.canvas.rgb;
    if (u.support_origin.w == 0.0) {
        c = mix(c, u.grid_minor.rgb, minor.x * fade * lod);
        c = mix(c, u.grid_major.rgb, major.x * fade);
    }
    if (u.flags.z > 0.5) {
        let uv = vec2<f32>((g.x - u.shadow_rect.x) * u.shadow_rect.z, 1.0 - (g.y - u.shadow_rect.y) * u.shadow_rect.w);
        let s = textureSampleLevel(shadow_tex, lin, uv, 0.0).rg;
        let occ = 1.0 - (1.0 - u.contact.w * s.r) * (1.0 - (1.0 - u.contact.w) * s.g);
        c *= 1.0 - u.ground.w * occ;
    }
    return vec4<f32>(dither(c, i.pos.xy), 1.0);
}
