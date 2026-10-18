// Instanced screen-space wide lines. Each segment instance expands to a quad around its
// projected endpoints (round caps, so polylines join without gaps); the fragment computes its
// distance to the segment and covers the pixel analytically (box-filtered band), or hard for the
// baseline and the MSAA-only variant (like three's LineMaterial: the quad is the line, the
// rasteriser's per-sample coverage anti-aliases its sides, and only the round caps are cut at the
// pixel centre). Lines are depth-tested against the faces, which are pushed back by their own
// slope (FACE_BIAS), and never write depth.
//
// Quad expansion and near-plane clipping adapted from SolidCraft `assets/shaders/viewport.wgsl`
// `vs_line` (MIT OR Apache-2.0); see ATTRIBUTION.md.

struct LOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) seg: vec4<f32>,   // endpoints in framebuffer px (y down)
    @location(1) @interpolate(flat) style: vec4<f32>, // linear rgb, half width px
    @location(2) @interpolate(flat) alpha: f32,
};

fn culled() -> LOut {
    var o: LOut;
    o.pos = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
    o.seg = vec4<f32>(0.0);
    o.style = vec4<f32>(0.0);
    o.alpha = 0.0;
    return o;
}

fn expand(vi: u32, a: vec3<f32>, b: vec3<f32>, width: f32, rgb: vec3<f32>, alpha: f32) -> LOut {
    var ca = u.view_proj * vec4<f32>(a, 1.0);
    var cb = u.view_proj * vec4<f32>(b, 1.0);
    // Clip against the captured finite near/far planes before screen-space expansion.
    for (var plane = 0; plane < 2; plane = plane + 1) {
        let za = select(ca.z, ca.w - ca.z, plane == 1);
        let zb = select(cb.z, cb.w - cb.z, plane == 1);
        if (za < 0.0 && zb < 0.0) { return culled(); }
        if (za < 0.0) { ca = mix(ca, cb, za / (za - zb)); }
        else if (zb < 0.0) { cb = mix(cb, ca, zb / (zb - za)); }
    }
    let half_vp = u.viewport.xy * 0.5;
    let sa = ca.xy / ca.w * half_vp;
    let sb = cb.xy / cb.w * half_vp;
    var dir = sb - sa;
    let len = length(dir);
    dir = select(vec2<f32>(1.0, 0.0), dir / len, len > 1e-4);
    let nrm = vec2<f32>(-dir.y, dir.x);
    // One pixel of margin for the analytic falloff.
    let h = width * 0.5 + select(0.0, 1.0, u.lines.z > 0.5);
    // Triangle strip corners: (t, side) = (0,-1) (0,1) (1,-1) (1,1).
    let t = f32(vi >> 1u);
    let s = f32(vi & 1u) * 2.0 - 1.0;
    let base = select(ca, cb, t > 0.5);
    let off = nrm * s * h + dir * (t * 2.0 - 1.0) * h;
    var o: LOut;
    o.pos = vec4<f32>(base.xy + off / half_vp * base.w, base.z, base.w);
    let fa = vec2<f32>(sa.x + half_vp.x, half_vp.y - sa.y);
    let fb = vec2<f32>(sb.x + half_vp.x, half_vp.y - sb.y);
    o.seg = vec4<f32>(fa, fb);
    o.style = vec4<f32>(rgb, width * 0.5);
    o.alpha = alpha;
    return o;
}

// Topological edges (MESH1 EDGE_POSITIONS polylines, one instance per segment).
@vertex
fn vs_edge(@builtin(vertex_index) vi: u32, @location(0) a: vec3<f32>, @location(1) b: vec3<f32>) -> LOut {
    return expand(vi, a, b, u.lines.x, u.edge.rgb, 1.0);
}

// Silhouette candidates: an interior edge of a curved face is drawn only where one of its two
// triangles faces the eye and the other faces away (the terminator of the tessellation).
@vertex
fn vs_sil(
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) n0: vec3<f32>,
    @location(3) n1: vec3<f32>,
) -> LOut {
    let e = view_direction((a + b) * 0.5);
    if (u.material.z > 0.5) { return culled(); }
    if (dot(n0, e) * dot(n1, e) > 0.0) {
        return culled();
    }
    return expand(vi, a, b, u.lines.y, u.edge.rgb, u.edge.w);
}

// Silhouette segments compacted by sil_cull.wgsl (drawn indirect).
@vertex
fn vs_silseg(@builtin(vertex_index) vi: u32, @location(0) a: vec3<f32>, @location(1) b: vec3<f32>) -> LOut {
    return expand(vi, a, b, u.lines.y, u.edge.rgb, u.edge.w);
}

// Baseline grid: today's GridPlane LineSegments (per-line faded colour, 1 px).
@vertex
fn vs_grid(@builtin(vertex_index) vi: u32, @location(0) a: vec3<f32>, @location(1) b: vec3<f32>, @location(2) c: vec4<f32>) -> LOut {
    return expand(vi, a, b, u.lines.w, c.rgb, 1.0);
}

fn line_color(i: LOut) -> vec4<f32> {
    let p = i.pos.xy;
    let a = i.seg.xy;
    let ab = i.seg.zw - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-8), 0.0, 1.0);
    let d = length(p - (a + ab * t));
    let hw = i.style.w;
    // No `discard`: a shader that may discard defers the depth test, so hidden edges behind the
    // bodies would all be shaded. Uncovered pixels output alpha 0 instead (blended: no change;
    // alpha-to-coverage: an empty sample mask).
    var cov: f32;
    if (u.lines.z > 0.5) {
        // Overlap of the 1 px pixel footprint [d-½, d+½] with the band [-hw, hw].
        cov = max(0.0, min(d + 0.5, hw) - max(d - 0.5, -hw));
    } else {
        // Hard: the quad is exactly the band, so inside the segment's span every covered sample
        // keeps full alpha and MSAA alone shapes the sides. Only the cap regions (beyond either
        // endpoint) are cut to the round cap, tested once at the pixel centre as three does.
        let t_raw = dot(p - a, ab) / max(dot(ab, ab), 1e-8);
        let in_span = dot(ab, ab) > 1e-8 && t_raw >= 0.0 && t_raw <= 1.0;
        cov = select(select(0.0, 1.0, d <= hw), 1.0, in_span);
    }
    return vec4<f32>(i.style.rgb, cov * i.alpha);
}

@fragment
fn fs_line(i: LOut) -> @location(0) vec4<f32> {
    return line_color(i);
}

// Timing experiment only (bench): the same coverage, but uncovered fragments are discarded.
@fragment
fn fs_line_discard(i: LOut) -> @location(0) vec4<f32> {
    let c = line_color(i);
    if (c.a <= 0.0) {
        discard;
    }
    return c;
}

// Timing experiment only (bench): the full edge vertex work, then the quad is culled. The
// always-true test reads every output so the compiler cannot skip the expansion.
@vertex
fn vs_edge_culled(@builtin(vertex_index) vi: u32, @location(0) a: vec3<f32>, @location(1) b: vec3<f32>) -> LOut {
    let o = expand(vi, a, b, u.lines.x, u.edge.rgb, 1.0);
    let probe = abs(o.pos.x) + abs(o.pos.y) + abs(o.pos.z) + abs(o.pos.w) + abs(o.seg.x) + abs(o.seg.w) + abs(o.style.w);
    if (probe < 3.0e38) {
        return culled();
    }
    return o;
}
