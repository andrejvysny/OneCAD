// Procedural studio environment and its split-sum prefiltering (Karis 2013): radiance cube with
// mips, GGX-prefiltered specular cube (filtered importance sampling), diffuse irradiance cube and
// the BRDF integration LUT. Runs once at start-up; nothing here depends on the camera, because
// the viewport turns the lookup direction with the camera yaw (env_dir) instead.
//
// The draw's instance index carries the cube face (and, for the prefilter, the mip):
// k = mip * 6 + face.

const SAMPLES: u32 = 512u;
const PI: f32 = 3.14159265358979;

struct FOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) k: u32,
};

@vertex
fn vs_face(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> FOut {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    var o: FOut;
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(x, -y); // texel space: v grows downward
    o.k = ii;
    return o;
}

// Cube face direction for texel coordinates uv in [-1, 1] (WebGPU/Vulkan/D3D face table).
fn cube_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let u = uv.x;
    let v = uv.y;
    var d: vec3<f32>;
    switch face {
        case 0u: { d = vec3<f32>(1.0, -v, -u); }
        case 1u: { d = vec3<f32>(-1.0, -v, u); }
        case 2u: { d = vec3<f32>(u, 1.0, v); }
        case 3u: { d = vec3<f32>(u, -1.0, -v); }
        case 4u: { d = vec3<f32>(u, -v, 1.0); }
        default: { d = vec3<f32>(-u, -v, -1.0); }
    }
    return normalize(d);
}

// A soft rectangular light panel centred on azimuth `az`, elevation `el` (degrees, studio
// frame: the camera sits at azimuth 0), half extents `hw` x `hh` and edge softness `soft`.
fn panel(d: vec3<f32>, az: f32, el: f32, hw: f32, hh: f32, soft: f32) -> f32 {
    let a = radians(az);
    let e = radians(el);
    let c = vec3<f32>(cos(e) * cos(a), cos(e) * sin(a), sin(e));
    let t = normalize(vec3<f32>(-sin(a), cos(a), 0.0));
    let b = cross(c, t);
    let fwd = dot(d, c);
    if (fwd <= 0.0) {
        return 0.0;
    }
    let x = degrees(atan2(dot(d, t), fwd));
    let y = degrees(atan2(dot(d, b), fwd));
    return smoothstep(hw + soft, hw - soft, abs(x)) * smoothstep(hh + soft, hh - soft, abs(y));
}

// The studio: a light-grey cyclorama (brighter overhead, darker floor), a large overhead key
// softbox over the camera's left shoulder, a tall fill strip on the right, a wide rim strip
// behind the part and a soft top light. Neutral white throughout: colour comes from tokens.
fn studio(d: vec3<f32>) -> vec3<f32> {
    let z = d.z;
    var l = mix(0.30, 0.55, smoothstep(-0.05, 0.10, z));
    l = mix(l, 0.80, smoothstep(0.10, 0.95, z));
    l = select(l, mix(0.30, 0.16, smoothstep(0.0, -0.6, z)), z < -0.05);
    l += 7.0 * panel(d, 40.0, 52.0, 24.0, 15.0, 5.0);
    l += 2.2 * panel(d, -80.0, 18.0, 9.0, 26.0, 4.0);
    l += 3.0 * panel(d, 180.0, 22.0, 45.0, 6.0, 3.0);
    l += 2.0 * smoothstep(0.88, 0.97, z);
    return vec3<f32>(l);
}

@fragment
fn fs_env(i: FOut) -> @location(0) vec4<f32> {
    return vec4<f32>(studio(cube_dir(i.k % 6u, i.uv)), 1.0);
}

@group(0) @binding(0) var src: texture_cube<f32>;
@group(0) @binding(1) var smp: sampler;

// Mip m from mip m-1 (`src` is that single mip): the bilinear tap at the parent texel's centre
// is the mean of its four children.
@fragment
fn fs_down(i: FOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(src, smp, cube_dir(i.k % 6u, i.uv), 0.0).rgb, 1.0);
}

fn hammersley(i: u32, n: u32) -> vec2<f32> {
    var b = i;
    b = (b << 16u) | (b >> 16u);
    b = ((b & 0x55555555u) << 1u) | ((b & 0xAAAAAAAAu) >> 1u);
    b = ((b & 0x33333333u) << 2u) | ((b & 0xCCCCCCCCu) >> 2u);
    b = ((b & 0x0F0F0F0Fu) << 4u) | ((b & 0xF0F0F0F0u) >> 4u);
    b = ((b & 0x00FF00FFu) << 8u) | ((b & 0xFF00FF00u) >> 8u);
    return vec2<f32>(f32(i) / f32(n), f32(b) * 2.3283064365386963e-10);
}

fn basis(n: vec3<f32>) -> mat3x3<f32> {
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < 0.999);
    let t = normalize(cross(up, n));
    return mat3x3<f32>(t, cross(n, t), n);
}

fn ggx_h(xi: vec2<f32>, a: f32) -> vec3<f32> {
    let phi = 2.0 * PI * xi.x;
    let ct = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    let st = sqrt(1.0 - ct * ct);
    return vec3<f32>(st * cos(phi), st * sin(phi), ct);
}

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let f = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * f * f);
}

// Specular prefilter, N = V = R. Roughness = mip / (mips - 1).
@fragment
fn fs_prefilter(i: FOut) -> @location(0) vec4<f32> {
    let face = i.k % 6u;
    let mip = i.k / 6u;
    let n = cube_dir(face, i.uv);
    let mips = f32(textureNumLevels(src));
    let rough = f32(mip) / 5.0;
    let src_size = f32(textureDimensions(src).x);
    if (mip == 0u) {
        return vec4<f32>(textureSampleLevel(src, smp, n, 1.0).rgb, 1.0);
    }
    let a = rough * rough;
    let tbn = basis(n);
    let sa_texel = 4.0 * PI / (6.0 * src_size * src_size);
    var acc = vec3<f32>(0.0);
    var w = 0.0;
    for (var s = 0u; s < SAMPLES; s = s + 1u) {
        let h = tbn * ggx_h(hammersley(s, SAMPLES), a);
        let l = 2.0 * dot(n, h) * h - n;
        let nl = dot(n, l);
        if (nl > 0.0) {
            let nh = max(dot(n, h), 0.0);
            let pdf = d_ggx(nh, a) * 0.25 + 1e-4;
            let sa_sample = 1.0 / (f32(SAMPLES) * pdf);
            let lod = clamp(0.5 * log2(sa_sample / sa_texel) + 1.0, 0.0, mips - 1.0);
            acc += textureSampleLevel(src, smp, l, lod).rgb * nl;
            w += nl;
        }
    }
    return vec4<f32>(acc / max(w, 1e-4), 1.0);
}

// Diffuse: (1/π)∫ L cos dω, so the shader multiplies by albedo only.
@fragment
fn fs_irradiance(i: FOut) -> @location(0) vec4<f32> {
    let n = cube_dir(i.k % 6u, i.uv);
    let tbn = basis(n);
    let lod = max(f32(textureNumLevels(src)) - 5.0, 0.0);
    var acc = vec3<f32>(0.0);
    var count = 0.0;
    let steps = 64u;
    for (var p = 0u; p < steps; p = p + 1u) {
        let phi = (f32(p) + 0.5) / f32(steps) * 2.0 * PI;
        for (var t = 0u; t < steps / 4u; t = t + 1u) {
            let theta = (f32(t) + 0.5) / f32(steps / 4u) * 0.5 * PI;
            let local = vec3<f32>(sin(theta) * cos(phi), sin(theta) * sin(phi), cos(theta));
            acc += textureSampleLevel(src, smp, tbn * local, lod).rgb * cos(theta) * sin(theta);
            count += 1.0;
        }
    }
    return vec4<f32>(PI * acc / count, 1.0);
}

// Split-sum BRDF LUT: x = N·V, y = roughness → (scale, bias) on F0.
@fragment
fn fs_lut(i: FOut) -> @location(0) vec4<f32> {
    let nv = max((i.uv.x + 1.0) * 0.5, 1e-3);
    let rough = (i.uv.y + 1.0) * 0.5; // texcoord y (top = 0) is roughness
    let a = rough * rough;
    let k = a * 0.5;
    let v = vec3<f32>(sqrt(1.0 - nv * nv), 0.0, nv);
    var acc = vec2<f32>(0.0);
    for (var s = 0u; s < SAMPLES; s = s + 1u) {
        let h = ggx_h(hammersley(s, SAMPLES), a);
        let l = 2.0 * dot(v, h) * h - v;
        let nl = max(l.z, 0.0);
        let nh = max(h.z, 0.0);
        let vh = max(dot(v, h), 0.0);
        if (nl > 0.0) {
            let g = (nv / (nv * (1.0 - k) + k)) * (nl / (nl * (1.0 - k) + k));
            let g_vis = g * vh / max(nh * nv, 1e-6);
            let fc = pow(1.0 - vh, 5.0);
            acc += vec2<f32>((1.0 - fc) * g_vis, fc * g_vis);
        }
    }
    return vec4<f32>(acc / f32(SAMPLES), 0.0, 1.0);
}
