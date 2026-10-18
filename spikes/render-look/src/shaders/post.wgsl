// Screen and map passes: contact-shadow occlusion + blur, GTAO + bilateral blur.
// All are full-target triangles; the target is sized by the render pass.

struct POut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> POut {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    var o: POut;
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    return o;
}

@group(1) @binding(0) var depth_in: texture_depth_2d;
@group(1) @binding(1) var tex_in: texture_2d<f32>;

// ---- contact shadow -------------------------------------------------------------------------

// Height map (4x larger) → occlusion: r = tight contact term, g = soft term. Depth stores the
// lowest surface's height over the ground, normalised to the soft range (1: nothing above).
@fragment
fn fs_contact_occ(i: POut) -> @location(0) vec4<f32> {
    let base = vec2<i32>(i.pos.xy) * 4;
    var acc = vec2<f32>(0.0);
    for (var y = 0; y < 4; y = y + 1) {
        for (var x = 0; x < 4; x = x + 1) {
            let h = textureLoad(depth_in, base + vec2<i32>(x, y), 0);
            if (h < 1.0) {
                let soft = 1.0 - h;
                let tight = clamp(1.0 - h * u.contact.z, 0.0, 1.0);
                acc += vec2<f32>(tight * tight * tight, soft * soft);
            }
        }
    }
    return vec4<f32>(acc / 16.0, 0.0, 1.0);
}

fn contact_blur(p: vec2<i32>, axis: vec2<i32>) -> vec4<f32> {
    let st = u.contact.x;
    let ss = u.contact.y;
    let radius = i32(min(ceil(3.0 * ss), 64.0));
    let size = vec2<i32>(textureDimensions(tex_in));
    var acc = vec2<f32>(0.0);
    var wsum = vec2<f32>(0.0);
    for (var k = -radius; k <= radius; k = k + 1) {
        let q = clamp(p + axis * k, vec2<i32>(0), size - 1);
        let s = textureLoad(tex_in, q, 0).rg;
        let x = f32(k);
        let w = vec2<f32>(exp(-x * x / (2.0 * st * st)), exp(-x * x / (2.0 * ss * ss)));
        acc += s * w;
        wsum += w;
    }
    return vec4<f32>(acc / wsum, 0.0, 1.0);
}

@fragment
fn fs_contact_blur_h(i: POut) -> @location(0) vec4<f32> {
    return contact_blur(vec2<i32>(i.pos.xy), vec2<i32>(1, 0));
}

@fragment
fn fs_contact_blur_v(i: POut) -> @location(0) vec4<f32> {
    return contact_blur(vec2<i32>(i.pos.xy), vec2<i32>(0, 1));
}

// ---- GTAO -----------------------------------------------------------------------------------
// Ground-truth AO (Jimenez et al. 2016), structured after Intel's XeGTAO reference (MIT):
// horizon search along SLICES screen directions, cosine-weighted visibility integral against
// the projected normal. View space here follows XeGTAO: x right, y up, z = distance forward.

const SLICES: i32 = 4;
const STEPS: i32 = 6;

fn view_pos(px: vec2<f32>, depth: f32) -> vec3<f32> {
    let dist = u.proj.z / max(depth, 1e-9);
    let uv = px * u.viewport.zw;
    return vec3<f32>((2.0 * uv.x - 1.0) * u.proj.x * dist, (1.0 - 2.0 * uv.y) * u.proj.y * dist, dist);
}

fn fast_acos(x: f32) -> f32 {
    let ax = abs(x);
    var r = (-0.156583 * ax + PI * 0.5) * sqrt(1.0 - ax);
    return select(PI - r, r, x >= 0.0);
}

@fragment
fn fs_gtao(i: POut) -> @location(0) vec4<f32> {
    let pix = vec2<i32>(i.pos.xy);
    let size = vec2<i32>(textureDimensions(depth_in));
    let d0 = textureLoad(depth_in, pix, 0);
    if (d0 <= 0.0) {
        return vec4<f32>(1.0);
    }
    let p = view_pos(i.pos.xy, d0);
    let nrh = textureLoad(tex_in, pix, 0).xyz;
    let n = normalize(vec3<f32>(nrh.x, nrh.y, -nrh.z));
    let v = normalize(-p);
    // World radius → pixels at this distance.
    let px_size = 2.0 * u.proj.y * p.z * u.viewport.w;
    let radius_px = min(u.ao.x / px_size, 256.0);
    if (radius_px < 1.0) {
        return vec4<f32>(1.0);
    }
    let noise_slice = ign(i.pos.xy);
    let noise_step = ign(i.pos.xy + vec2<f32>(37.0, 113.0));
    let min_s = 1.3 / radius_px;
    var visibility = 0.0;
    for (var sl = 0; sl < SLICES; sl = sl + 1) {
        let phi = (f32(sl) + noise_slice) / f32(SLICES) * PI;
        let omega = vec2<f32>(cos(phi), -sin(phi)) * radius_px;
        let dir = vec3<f32>(cos(phi), sin(phi), 0.0);
        let ortho = dir - dot(dir, v) * v;
        let axis = normalize(cross(ortho, v));
        let pn = n - axis * dot(n, axis);
        let pn_len = length(pn);
        let sgn = sign(dot(ortho, pn));
        let cos_n = clamp(dot(pn, v) / max(pn_len, 1e-6), 0.0, 1.0);
        let nang = sgn * fast_acos(cos_n);
        let low0 = cos(nang + PI * 0.5);
        let low1 = cos(nang - PI * 0.5);
        var h0 = low0;
        var h1 = low1;
        for (var st = 0; st < STEPS; st = st + 1) {
            let jit = fract(noise_step + f32(sl + st * STEPS) * 0.6180339887);
            var s = (f32(st) + jit) / f32(STEPS);
            s = s * s + min_s;
            let off = round(omega * s);
            let q0 = clamp(pix + vec2<i32>(off), vec2<i32>(0), size - 1);
            let q1 = clamp(pix - vec2<i32>(off), vec2<i32>(0), size - 1);
            let s0 = view_pos(vec2<f32>(q0) + 0.5, textureLoad(depth_in, q0, 0));
            let s1 = view_pos(vec2<f32>(q1) + 0.5, textureLoad(depth_in, q1, 0));
            let dl0 = s0 - p;
            let dl1 = s1 - p;
            let len0 = length(dl0);
            let len1 = length(dl1);
            let w0 = clamp(len0 * u.ao.y + u.ao.z, 0.0, 1.0);
            let w1 = clamp(len1 * u.ao.y + u.ao.z, 0.0, 1.0);
            h0 = max(h0, mix(low0, dot(dl0 / max(len0, 1e-6), v), w0));
            h1 = max(h1, mix(low1, dot(dl1 / max(len1, 1e-6), v), w1));
        }
        let a0 = nang + clamp(-fast_acos(h1) - nang, -PI * 0.5, PI * 0.5);
        let a1 = nang + clamp(fast_acos(h0) - nang, -PI * 0.5, PI * 0.5);
        let i0 = (cos_n + 2.0 * a0 * sin(nang) - cos(2.0 * a0 - nang)) * 0.25;
        let i1 = (cos_n + 2.0 * a1 * sin(nang) - cos(2.0 * a1 - nang)) * 0.25;
        visibility += mix(pn_len, 1.0, 0.05) * (i0 + i1);
    }
    visibility = pow(clamp(visibility / f32(SLICES), 0.0, 1.0), u.ao.w);
    return vec4<f32>(max(visibility, 0.03), 0.0, 0.0, 1.0);
}

// Depth-aware separable blur of the AO (suppresses the per-pixel slice noise).
fn ao_blur(p: vec2<i32>, axis: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(tex_in));
    let d0 = textureLoad(depth_in, p, 0);
    if (d0 <= 0.0) {
        return vec4<f32>(1.0);
    }
    let z0 = u.proj.z / d0;
    var acc = 0.0;
    var wsum = 0.0;
    for (var k = -4; k <= 4; k = k + 1) {
        let q = clamp(p + axis * k, vec2<i32>(0), size - 1);
        let d = textureLoad(depth_in, q, 0);
        let z = u.proj.z / max(d, 1e-9);
        let x = f32(k);
        let w = exp(-x * x / 8.0) * exp(-abs(z - z0) / (0.01 * z0)) * select(0.0, 1.0, d > 0.0);
        acc += textureLoad(tex_in, q, 0).r * w;
        wsum += w;
    }
    return vec4<f32>(acc / max(wsum, 1e-6), 0.0, 0.0, 1.0);
}

@fragment
fn fs_ao_blur_h(i: POut) -> @location(0) vec4<f32> {
    return ao_blur(vec2<i32>(i.pos.xy), vec2<i32>(1, 0));
}

@fragment
fn fs_ao_blur_v(i: POut) -> @location(0) vec4<f32> {
    return ao_blur(vec2<i32>(i.pos.xy), vec2<i32>(0, 1));
}
