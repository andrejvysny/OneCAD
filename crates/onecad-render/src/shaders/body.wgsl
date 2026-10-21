// Shared depth/normal, studio shading, transparency, and contact height entry points.

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

@vertex
fn vs_mask(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>) -> VOut {
    var o: VOut;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    o.wp = p;
    o.n = n;
    // Wall crossings beyond near/far still cancel winding; only the reconstructed
    // cap and ordinary geometry use the captured depth clip range.
    o.pos.z = 0.5 * o.pos.w;
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
    if clipped(i.wp) { discard; }
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

fn shade_body(i: VOut, front: bool) -> vec4<f32> {
    let n = shading_normal(i, front);
    let v = view_direction(i.wp);
    // Derivatives first, in uniform control flow.
    let ndv = dot(n, v);
    if clipped(i.wp) { discard; }
    // Retained backfaces provide depth proxies; the cap colors the section once.
    if sectioned() && !front && u.display.w < 0.5 { return vec4<f32>(0.0); }
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
    return vec4<f32>(mix(dither(c, i.pos.xy), u.overlay.rgb, u.overlay.a), u.display.x);
}

@fragment
fn fs_body(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return shade_body(i, front);
}

struct Transparency {
    @location(0) accumulation: vec4<f32>,
    @location(1) revealage: f32,
};

// Constant bounded weight: each triangle contributes at most 1/16 to fp16.
// At the 500,000 triangle admission ceiling the sum remains below 31,251.
@fragment
fn fs_xray(i: VOut, @builtin(front_facing) front: bool) -> Transparency {
    let color = shade_body(i, front);
    let alpha = u.display.x;
    var out: Transparency;
    out.accumulation = vec4<f32>(clamp(color.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * alpha, alpha) * 0.0625;
    out.revealage = alpha;
    return out;
}

// Contact shadow: the lowest surface over each ground texel (ortho, looking up).
@vertex
fn vs_height(@location(0) p: vec3<f32>) -> VOut {
    var o: VOut;
    o.pos = u.shadow_vp * vec4<f32>(p, 1.0);
    o.wp = p;
    o.n = vec3<f32>(0.0, 0.0, -1.0);
    return o;
}

@fragment fn fs_height(i: VOut) { if clipped(i.wp) { discard; } }


@fragment
fn fs_preview(i: VOut) -> @location(0) vec4<f32> {
    if clipped(i.wp) { discard; }
    // Ghost faces stay translucent but shade by facing ratio so the preview reads as 3D.
    let facing = abs(dot(normalize(i.n), view_direction(i.wp)));
    return vec4<f32>(u.overlay.rgb * (0.7 + 0.3 * facing), u.overlay.a);
}

// Flat alpha overlays follow SolidCraft assets/shaders/viewport.wgsl::shade's unlit
// selection path at 6bde126a5cabd1f5af0c68e6955bd0176fab805a. Keep committed PBR separate.
@fragment
fn fs_highlight(i: VOut) -> @location(0) vec4<f32> {
    if clipped(i.wp) { discard; }
    return u.overlay;
}

fn cap_color(i: VOut) -> vec4<f32> {
    let hatch = fract((i.pos.x + i.pos.y) / (9.0 * u.display.y));
    let c = mix(u.body.rgb, u.cap.rgb, 0.65);
    return vec4<f32>(select(c, c * 0.6, hatch < 0.22), u.display.x * u.cap.a);
}

struct CutOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@vertex fn vs_cap(@builtin(vertex_index) vi: u32) -> VOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var o: VOut;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.wp = vec3<f32>(0.0);
    o.n = vec3<f32>(0.0);
    return o;
}

fn section_depth(pixel: vec2<f32>) -> f32 {
    let uv = pixel * u.viewport.zw;
    let near_view = u.inverse_projection * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0);
    let rotation = mat3x3<f32>(u.view[0].xyz, u.view[1].xyz, u.view[2].xyz);
    let near = u.eye.xyz + transpose(rotation) * (near_view.xyz / near_view.w);
    let back = vec3<f32>(u.view[0].z, u.view[1].z, u.view[2].z);
    let direction = select(-back, near - u.eye.xyz, u.eye.w > 0.5);
    let denominator = dot(u.section.xyz, direction);
    if abs(denominator) < 1e-12 { return -1.0; }
    let distance = (u.section.w - dot(u.section.xyz, near)) / denominator;
    if distance < 0.0 { return -1.0; }
    let clip = u.view_proj * vec4<f32>(near + direction * distance, 1.0);
    if clip.w <= 0.0 { return -1.0; }
    return clip.z / clip.w;
}

@fragment fn fs_cap(i: VOut) -> CutOut {
    let d = section_depth(i.pos.xy);
    if !(d >= 0.0 && d <= 1.0) { discard; }
    var o: CutOut;
    o.color = cap_color(i);
    // Keep a cap coincident with the ground ahead of the later grid draw.
    o.depth = min(d + 1e-7, 1.0);
    return o;
}
@fragment fn fs_pre_cap(i: VOut) -> CutOut {
    let d = section_depth(i.pos.xy);
    if !(d >= 0.0 && d <= 1.0) { discard; }
    var o: CutOut;
    o.color = vec4<f32>((u.view * vec4<f32>(normalize(u.section.xyz), 0.0)).xyz, 1.0);
    o.depth = d;
    return o;
}

@fragment fn fs_height_cap(i: VOut) -> @builtin(frag_depth) f32 {
    if u.section.z >= 0.0 { discard; }
    let uv = i.pos.xy / vec2<f32>(1024.0);
    let xy = u.shadow_rect.xy + vec2<f32>(uv.x, 1.0 - uv.y) / u.shadow_rect.zw;
    let z = (u.section.w - dot(u.section.xy, xy)) / u.section.z;
    let clip = u.shadow_vp * vec4<f32>(xy, z, 1.0);
    let depth = clip.z / clip.w;
    if !(depth >= 0.0 && depth <= 1.0) { discard; }
    return depth;
}
@fragment fn fs_depth(i: VOut) -> @location(0) vec4<f32> {
    if clipped(i.wp) { discard; }
    return vec4<f32>(0.0);
}

@fragment fn fs_xray_mask(i: VOut) -> Transparency {
    if clipped(i.wp) { discard; }
    var out: Transparency;
    out.accumulation = vec4<f32>(0.0);
    out.revealage = 0.0;
    return out;
}
@fragment fn fs_xray_cap(i: VOut) -> Transparency {
    let depth = section_depth(i.pos.xy);
    if !(depth >= 0.0 && depth <= 1.0) { discard; }
    let color = cap_color(i);
    var out: Transparency;
    out.accumulation = vec4<f32>(clamp(color.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * color.a, color.a) * 0.0625;
    out.revealage = color.a;
    return out;
}
@fragment fn fs_cap_depth(i: VOut) -> CutOut {
    let depth = section_depth(i.pos.xy);
    if !(depth >= 0.0 && depth <= 1.0) { discard; }
    var out: CutOut;
    out.color = vec4<f32>(0.0);
    out.depth = depth;
    return out;
}

@fragment fn fs_clear_mask() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
@fragment fn fs_preview_cap(i: VOut) -> CutOut {
    let depth = section_depth(i.pos.xy);
    if !(depth >= 0.0 && depth <= 1.0) { discard; }
    var out: CutOut;
    out.color = u.overlay;
    out.depth = depth;
    return out;
}
