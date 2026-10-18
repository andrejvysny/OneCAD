// Body faces: depth/normal prepass, the quality PBR shade, the baseline (today's three.js
// MeshStandardMaterial lighting) shade, and the contact-shadow height pass.

@group(1) @binding(0) var ao_tex: texture_2d<f32>;
@group(1) @binding(1) var irr_tex: texture_cube<f32>;
@group(1) @binding(2) var spec_tex: texture_cube<f32>;
@group(1) @binding(3) var lut_tex: texture_2d<f32>;
@group(1) @binding(4) var lin: sampler;

const SPEC_MIPS: f32 = 6.0;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) wp: vec3<f32>,
    @location(1) n: vec3<f32>,
};

@vertex
fn vs_body(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>) -> VOut {
    var o: VOut;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    o.wp = p;
    o.n = n;
    return o;
}

// The shading normal: MESH1 normals are outward (smoothed per face); a back-facing fragment
// (seen through an open edge, or a reversed face) is shaded from its other side, like three's
// DoubleSide.
fn shading_normal(i: VOut, front: bool) -> vec3<f32> {
    let n = normalize(i.n);
    return select(-n, n, front);
}

@fragment
fn fs_prepass(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = shading_normal(i, front);
    return vec4<f32>((u.view * vec4<f32>(n, 0.0)).xyz, 1.0);
}

fn fresnel(f0: vec3<f32>, c: f32) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - c, 5.0);
}

fn fresnel_rough(f0: vec3<f32>, c: f32, rough: f32) -> vec3<f32> {
    return f0 + (max(vec3<f32>(1.0 - rough), f0) - f0) * pow(1.0 - c, 5.0);
}

// GGX / Smith height-correlated visibility, Lambert diffuse; `light.w` is three's intensity
// (irradiance at normal incidence), so direct = intensity · N·L · BRDF.
fn direct(light: vec4<f32>, n: vec3<f32>, v: vec3<f32>, diff: vec3<f32>, f0: vec3<f32>, rough: f32) -> vec3<f32> {
    let l = normalize(light.xyz);
    let nl = dot(n, l);
    if (nl <= 0.0 || light.w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = normalize(l + v);
    let nv = max(dot(n, v), 1e-4);
    let nh = max(dot(n, h), 0.0);
    let a = max(rough * rough, 0.002);
    let a2 = a * a;
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * dd * dd);
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    let vis = 0.5 / max(gv + gl, 1e-6);
    let f = fresnel(f0, max(dot(v, h), 0.0));
    return light.w * nl * (diff / PI * (vec3<f32>(1.0) - f) + f * d * vis);
}

// GTAO multi-bounce fit (Jimenez et al. 2016): light albedos lose less to occlusion.
fn ao_multibounce(ao: f32, albedo: vec3<f32>) -> vec3<f32> {
    let a = 2.0404 * albedo - 0.3324;
    let b = -4.7951 * albedo + 0.6417;
    let c = 2.7552 * albedo + 0.6903;
    return max(vec3<f32>(ao), ((ao * a + b) * ao + c) * ao);
}

// Specular occlusion from AO (Lagarde & de Rousiers 2014).
fn spec_occlusion(nv: f32, ao: f32, rough: f32) -> f32 {
    return clamp(pow(nv + ao, exp2(-16.0 * rough - 1.0)) - 1.0 + ao, 0.0, 1.0);
}

@fragment
fn fs_body(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = shading_normal(i, front);
    let v = view_direction(i.wp);
    // Derivatives first, in uniform control flow.
    let ndv = dot(n, v);
    let fw = fwidth(ndv);
    let nv = clamp(ndv, 1e-4, 1.0);
    let rough = u.body.w;
    let metal = u.material.x;
    let f0 = mix(vec3<f32>(u.material.y), u.body.rgb, metal);
    let diff = u.body.rgb * (1.0 - metal);
    var ao = 1.0;
    if (u.flags.y > 0.5) {
        ao = textureLoad(ao_tex, vec2<i32>(i.pos.xy), 0).r;
    }
    let irr = textureSampleLevel(irr_tex, lin, env_dir(n), 0.0).rgb;
    let r = reflect(-v, n);
    let pre = textureSampleLevel(spec_tex, lin, env_dir(r), rough * (SPEC_MIPS - 1.0)).rgb;
    let lut = textureSampleLevel(lut_tex, lin, vec2<f32>(nv, rough), 0.0).rg;
    let fr = fresnel_rough(f0, nv, rough);
    let ibl_d = irr * diff * (vec3<f32>(1.0) - fr) * ao_multibounce(ao, diff);
    let ibl_s = pre * (f0 * lut.x + lut.y) * spec_occlusion(nv, ao, rough);
    var c = (ibl_d + ibl_s) * u.env.z;
    c += direct(u.key, n, v, diff, f0, rough) * mix(1.0, ao, 0.35);
    c = pbr_neutral(c * u.env.w);
    // Silhouettes of curved faces: where the smooth normal turns edge-on, a line about 2 px
    // wide on the visible surface (depth-correct for free). Adapted from SolidCraft
    // `assets/shaders/viewport.wgsl` fs_solid (MIT OR Apache-2.0); see ATTRIBUTION.md.
    if (u.material.z > 0.5 && fw > 1e-6) {
        let a = (1.0 - smoothstep(fw * 1.0, fw * 2.5, abs(ndv))) * u.edge.w;
        c = mix(c, u.edge.rgb, a);
    }
    return vec4<f32>(mix(dither(c, i.pos.xy), u.overlay.rgb, u.overlay.a), 1.0);
}

// Today's look: MeshStandardMaterial(metalness 0, roughness 0.5) under a hemisphere light
// (ground = canvas), a camera-relative key + fill pair and a weak prefiltered environment,
// NeutralToneMapping at exposure 1. No AO, no shadows.
@fragment
fn fs_body_baseline(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = shading_normal(i, front);
    let v = view_direction(i.wp);
    let nv = clamp(dot(n, v), 1e-4, 1.0);
    let rough = u.body.w;
    let f0 = vec3<f32>(u.material.y);
    let diff = u.body.rgb;
    var c = direct(u.key, n, v, diff, f0, rough) + direct(u.fill, n, v, diff, f0, rough);
    let hemi = mix(u.hemi_ground.rgb, u.hemi_sky.rgb, 0.5 * n.z + 0.5) * u.hemi_sky.w;
    c += hemi * diff / PI;
    let irr = textureSampleLevel(irr_tex, lin, env_dir(n), 0.0).rgb;
    let r = reflect(-v, n);
    let pre = textureSampleLevel(spec_tex, lin, env_dir(r), rough * (SPEC_MIPS - 1.0)).rgb;
    let lut = textureSampleLevel(lut_tex, lin, vec2<f32>(nv, rough), 0.0).rg;
    c += (irr * diff + pre * (f0 * lut.x + lut.y)) * u.env.z;
    return vec4<f32>(pbr_neutral(c * u.env.w), 1.0);
}

// Contact shadow: the lowest surface over each ground texel (ortho, looking up).
@vertex
fn vs_height(@location(0) p: vec3<f32>) -> @builtin(position) vec4<f32> {
    return u.shadow_vp * vec4<f32>(p, 1.0);
}

@fragment
fn fs_preview(i: VOut) -> @location(0) vec4<f32> {
    return u.overlay;
}

// Flat alpha overlays follow SolidCraft assets/shaders/viewport.wgsl::shade's unlit
// selection path at 6bde126a5cabd1f5af0c68e6955bd0176fab805a. Keep committed PBR separate.
@fragment
fn fs_highlight(i: VOut) -> @location(0) vec4<f32> {
    return u.overlay;
}
