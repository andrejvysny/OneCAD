// Shared frame uniform and helpers. Prepended to every viewport shader module.
// World is Z-up, right-handed, millimetres. GPU projection uses depth 1 near, 0 far; CPU captures remain unchanged.

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    inverse_projection: mat4x4<f32>,
    shadow_vp: mat4x4<f32>,  // contact-shadow height map: ortho, looking up from under the ground
    proj: vec4<f32>,         // contact height span / soft range, reserved
    eye: vec4<f32>,
    viewport: vec4<f32>,     // width, height, 1/width, 1/height (px)
    env: vec4<f32>,          // cos(yaw), sin(yaw), environment intensity, exposure
    key: vec4<f32>,          // direction toward the light (world), intensity
    fill: vec4<f32>,         // direction toward the light (world), intensity
    hemi_sky: vec4<f32>,     // rgb, intensity
    hemi_ground: vec4<f32>,  // rgb
    body: vec4<f32>,         // linear base colour, roughness
    material: vec4<f32>,     // metallic, dielectric f0, 0, 0
    edge: vec4<f32>,         // linear edge colour, silhouette alpha
    lines: vec4<f32>,        // feature width px, silhouette width px, analytic AA (1) or hard (0), grid width px
    canvas: vec4<f32>,
    grid_minor: vec4<f32>,
    grid_major: vec4<f32>,
    grid: vec4<f32>,         // minor step, fade centre x, fade centre y, extent
    ground: vec4<f32>,       // ground z, minor width px, major width px, shadow strength
    shadow_rect: vec4<f32>,  // min x, min y, 1/size x, 1/size y
    contact: vec4<f32>,      // tight sigma (texels), soft sigma (texels), soft/tight height ratio, tight weight
    ao: vec4<f32>,           // radius (mm), falloff mul, falloff add, final power
    flags: vec4<f32>,        // quality, AO, contact shadow, dither
    overlay: vec4<f32>,
    section: vec4<f32>,
    cap: vec4<f32>,
    display: vec4<f32>, // opacity, SSAA factor, hidden edges, silhouettes
};

@group(0) @binding(0) var<uniform> u: Frame;

const PI: f32 = 3.14159265358979;

// Khronos PBR Neutral tone mapping (the operator three.js calls NeutralToneMapping).
fn pbr_neutral(c_in: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desat = 0.15;
    let x = min(c_in.r, min(c_in.g, c_in.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var c = c_in - offset;
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start) {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (desat * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}

fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// ±half an 8-bit sRGB step of triangular noise, applied in linear space near the value.
fn dither(c: vec3<f32>, p: vec2<f32>) -> vec3<f32> {
    if (u.flags.w < 0.5) {
        return c;
    }
    let n = ign(p) + ign(p + vec2<f32>(17.0, 59.0)) - 1.0;
    let s = pow(max(c, vec3<f32>(1e-4)), vec3<f32>(1.0 / 2.2));
    let s2 = s + vec3<f32>(n / 255.0);
    return pow(max(s2, vec3<f32>(0.0)), vec3<f32>(2.2));
}

// World direction into the studio frame: the environment is authored with the camera at
// azimuth 0 and turned with the camera's yaw (camera-relative like today's light rig), never
// tilted with its pitch. Rotation about world Z only, so the Z-up invariant holds.
fn env_dir(d: vec3<f32>) -> vec3<f32> {
    let c = u.env.x;
    let s = u.env.y;
    return vec3<f32>(c * d.x + s * d.y, -s * d.x + c * d.y, d.z);
}

fn view_direction(p: vec3<f32>) -> vec3<f32> {
    let back = vec3<f32>(u.view[0].z, u.view[1].z, u.view[2].z);
    return select(back, normalize(u.eye.xyz - p), u.eye.w > 0.5);
}

fn sectioned() -> bool { return dot(u.section.xyz, u.section.xyz) > 0.0; }
fn clipped(p: vec3<f32>) -> bool { return sectioned() && dot(u.section.xyz, p) > u.section.w; }
