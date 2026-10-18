// Viewport shaders. The wide-line vertex expansion (near-plane cut, screen-space quad) and the
// key/fill/spec studio shading are adapted from SolidCraft `assets/shaders/viewport.wgsl`
// (MIT OR Apache-2.0); see ATTRIBUTION.md. Analytic line AA and the hemisphere ambient are ours.

struct U {
    vp: mat4x4<f32>,
    eye: vec4<f32>,     // xyz eye position
    screen: vec4<f32>,  // viewport w, h (physical px), grid fade radius, 1 = encode sRGB
    target_: vec4<f32>, // xyz orbit target
};
@group(0) @binding(0) var<uniform> u: U;

struct Body { color: vec4<f32> };
@group(1) @binding(0) var<uniform> body: Body;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(c / 12.92, pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), c > vec3<f32>(0.04045));
}
fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(c * 12.92, 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c > vec3<f32>(0.0031308));
}
// Lighting runs in linear light; a non-sRGB target (egui's usual Bgra8Unorm) gets encoded here.
fn out_color(linear: vec3<f32>, a: f32) -> vec4<f32> {
    if (u.screen.w > 0.5) { return vec4<f32>(to_srgb(linear), a); }
    return vec4<f32>(linear, a);
}

struct TOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) n: vec3<f32>,
    @location(1) wp: vec3<f32>,
};

@vertex
fn vs_tri(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>) -> TOut {
    var o: TOut;
    o.pos = u.vp * vec4<f32>(p, 1.0);
    o.n = n;
    o.wp = p;
    return o;
}

@fragment
fn fs_tri(i: TOut) -> @location(0) vec4<f32> {
    let v = normalize(u.eye.xyz - i.wp);
    var n = normalize(i.n);
    if (dot(n, v) < 0.0) { n = -n; }
    // Studio rig: a key light riding with the camera (up and to the left), a head-on fill,
    // a sky/ground hemisphere ambient on world Z, and a soft specular lobe.
    let key = normalize(v * 0.8 + vec3<f32>(-0.3, 0.2, 0.6));
    let diff = max(dot(n, key), 0.0);
    let fill = max(dot(n, v), 0.0);
    let hemi = mix(0.55, 1.0, 0.5 + 0.5 * n.z);
    let h = normalize(key + v);
    let spec = pow(max(dot(n, h), 0.0), 40.0) * 0.18;
    let base = to_linear(body.color.rgb);
    let k = 0.30 * hemi + 0.80 * diff + 0.40 * fill;
    return out_color(min(base * k + vec3<f32>(spec), vec3<f32>(1.0)), 1.0);
}

struct LOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) c: vec4<f32>,
    @location(1) side_px: f32,
    @location(2) half_w: f32,
    @location(3) wp: vec3<f32>,
    @location(4) fade: f32,
};

fn near_z(c: vec4<f32>) -> f32 {
    // Reversed depth (1 near): clip z measured the usual way is w - z.
    return c.w - c.z;
}

// One instance = one segment a→b; six vertices make its screen-space quad, widened by a 1 px
// feather on each side for the analytic coverage in fs_line. width < 0 marks a grid line that
// fades out with distance from the orbit target.
@vertex
fn vs_line(@builtin(vertex_index) vi: u32, @location(0) a: vec3<f32>, @location(1) b: vec3<f32>,
           @location(2) c: vec4<f32>, @location(3) width: f32) -> LOut {
    var pa = a;
    var pb = b;
    var ca = u.vp * vec4<f32>(a, 1.0);
    var cb = u.vp * vec4<f32>(b, 1.0);
    let za = near_z(ca);
    let zb = near_z(cb);
    if (za < 0.0 && zb > 0.0) {
        let k = za / (za - zb);
        ca = mix(ca, cb, k);
        pa = mix(a, b, k);
    } else if (zb < 0.0 && za > 0.0) {
        let k = zb / (zb - za);
        cb = mix(cb, ca, k);
        pb = mix(b, a, k);
    }
    let half = u.screen.xy * 0.5;
    let sa = ca.xy / max(ca.w, 1e-6) * half;
    let sb = cb.xy / max(cb.w, 1e-6) * half;
    var dir = sb - sa;
    let len = length(dir);
    if (len < 1e-6) { dir = vec2<f32>(1.0, 0.0); } else { dir = dir / len; }
    let nrm = vec2<f32>(-dir.y, dir.x);
    var ts = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    var ss = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let t = ts[vi];
    let s = ss[vi];
    let w = abs(width);
    let hw = 0.5 * w + 1.0;
    let base = select(ca, cb, t > 0.5);
    let off = nrm * s * hw + dir * (t * 2.0 - 1.0) * 0.5 * w;
    var o: LOut;
    o.pos = vec4<f32>(base.xy + off / half * base.w, base.z, base.w);
    o.c = c;
    o.side_px = s * hw;
    o.half_w = 0.5 * w;
    o.wp = select(pa, pb, t > 0.5);
    o.fade = select(0.0, 1.0, width < 0.0);
    return o;
}

@fragment
fn fs_line(i: LOut) -> @location(0) vec4<f32> {
    var a = clamp(i.half_w + 0.5 - abs(i.side_px), 0.0, 1.0) * i.c.a;
    if (i.fade > 0.5) {
        let d = length(i.wp.xy - u.target_.xy);
        a = a * (1.0 - smoothstep(0.35 * u.screen.z, u.screen.z, d));
    }
    return out_color(to_linear(i.c.rgb), a);
}

// Fullscreen triangle at depth 0, the far plane of reversed depth: egui clears depth to 1.0,
// so the viewport resets its own rect first. Adapted from SolidCraft `crates/gpu/src/view.rs`.
@vertex
fn vs_reset(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}
@fragment
fn fs_reset() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
