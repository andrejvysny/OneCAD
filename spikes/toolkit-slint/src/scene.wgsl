// Viewport shader for the Slint toolkit spike.
// `shade` and the instanced wide-line vertex shader `vs_line` are adapted from SolidCraft
// assets/shaders/viewport.wgsl (MIT OR Apache-2.0); see ATTRIBUTION.md.

struct U {
    vp: mat4x4<f32>,
    eye: vec4<f32>,     // xyz = eye position, w = 1 perspective, 0 orthographic
    back: vec4<f32>,    // xyz = toward the eye (orthographic view direction)
    screen: vec4<f32>,  // physical width, height, edge width px, grid width px
    grid: vec4<f32>,    // fade centre xy, fade start radius, fade end radius
    pal: array<vec4<f32>, 8>, // 0 edge, 1 selected, 2 grid minor, 3 grid major, 4 axis x, 5 axis y, 6 sky, 7 ground
};

@group(0) @binding(0) var<uniform> u: U;
// Per body: rgb = linear fill, a = 0 hidden, 1 visible, 2 selected.
@group(0) @binding(1) var<storage, read> bodies: array<vec4<f32>>;

struct TOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) n: vec3<f32>,
    @location(1) wp: vec3<f32>,
    @location(2) @interpolate(flat) body: u32,
};

@vertex
fn vs_tri(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) body: u32) -> TOut {
    var o: TOut;
    o.pos = u.vp * vec4<f32>(p, 1.0);
    if (bodies[body].a < 0.5) {
        o.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0); // outside the clip volume: hidden body
    }
    o.n = n;
    o.wp = p;
    o.body = body;
    return o;
}

fn view_dir(wp: vec3<f32>) -> vec3<f32> {
    if (u.eye.w > 0.5) {
        return normalize(u.eye.xyz - wp);
    }
    return normalize(u.back.xyz);
}

// Studio-ish Lambert: a key light offset from the eye, an eye fill, a sky/ground hemisphere
// and a small Blinn highlight.
fn shade(base: vec3<f32>, n_in: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    var n = normalize(n_in);
    if (dot(n, v) < 0.0) { n = -n; }
    let key = normalize(v * 0.8 + vec3<f32>(-0.3, 0.2, 0.6));
    let diff = max(dot(n, key), 0.0);
    let fill = max(dot(n, v), 0.0);
    let h = normalize(key + v);
    let spec = pow(max(dot(n, h), 0.0), 40.0) * 0.18;
    let hemi = mix(u.pal[7].rgb, u.pal[6].rgb, n.z * 0.5 + 0.5);
    let k = 0.35 + 0.75 * diff + 0.45 * fill;
    return min(base * (k + hemi) + vec3<f32>(spec), vec3<f32>(1.0));
}

@fragment
fn fs_tri(i: TOut) -> @location(0) vec4<f32> {
    let v = view_dir(i.wp);
    let ndv = dot(normalize(i.n), v);
    let fw = fwidth(ndv);
    let b = bodies[i.body];
    var base = b.rgb;
    if (b.a > 1.5) {
        base = mix(base, u.pal[1].rgb, 0.45);
    }
    var c = shade(base, i.n, v);
    // Silhouette where a curved surface turns away from the eye (about 1.5 px).
    if (fw > 1e-6) {
        let a = 1.0 - smoothstep(fw * 0.8, fw * 2.0, abs(ndv));
        c = mix(c, u.pal[0].rgb, a);
    }
    return vec4<f32>(c, 1.0);
}

struct LOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) c: vec4<f32>,
    @location(1) wp: vec3<f32>,
    @location(2) @interpolate(flat) kind: u32,
};

// kind: 0 body edge, 1 grid minor, 2 grid major, 3 axis x, 4 axis y.
@vertex
fn vs_line(@builtin(vertex_index) vi: u32, @location(0) a: vec3<f32>, @location(1) b: vec3<f32>,
           @location(2) body: u32, @location(3) kind: u32) -> LOut {
    var o: LOut;
    var color = u.pal[0];
    var w = u.screen.z;
    if (kind == 0u) {
        let f = bodies[body].a;
        if (f < 0.5) {
            o.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
            return o;
        }
        if (f > 1.5) { color = u.pal[1]; w = w * 1.4; }
    } else {
        color = u.pal[kind + 1u];
        w = u.screen.w;
        if (kind >= 3u) { w = u.screen.z * 1.3; }
    }
    var pa = a;
    var pb = b;
    var ca = u.vp * vec4<f32>(a, 1.0);
    var cb = u.vp * vec4<f32>(b, 1.0);
    // Cut a segment that crosses the near plane at the plane (reversed depth: near is z = w).
    let za = ca.w - ca.z;
    let zb = cb.w - cb.z;
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
    let base = select(ca, cb, t > 0.5);
    let off = nrm * s * w * 0.5 + dir * (t * 2.0 - 1.0) * w * 0.5;
    o.pos = vec4<f32>(base.xy + off / half * base.w, base.z, base.w);
    // Pull lines slightly toward the eye so edges win over the faces they bound.
    if (u.eye.w > 0.5) {
        o.pos.z = o.pos.z * 1.0006;
    } else {
        o.pos.z = o.pos.z + 0.0004 * o.pos.w;
    }
    o.c = color;
    o.wp = select(pa, pb, t > 0.5);
    o.kind = kind;
    return o;
}

@fragment
fn fs_line(i: LOut) -> @location(0) vec4<f32> {
    var c = i.c;
    if (i.kind == 1u || i.kind == 2u) {
        let d = length(i.wp.xy - u.grid.xy);
        c.a = c.a * (1.0 - smoothstep(u.grid.z, u.grid.w, d));
    }
    return c;
}
